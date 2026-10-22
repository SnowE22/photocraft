//! Nano Banana 2.1 (`gemini-nano-banana-2.1`, Google's Gemini image model) as a removal model:
//! paid, online, and much better than the local models at knowing what lies behind something.
//!
//! The hole goes to Google the way people use the model in Photoshop, but without the manual
//! work: a crop around it with the hole painted solid red, and a prompt asking for the red to be
//! removed and what is behind it drawn in the surrounding style. The answer is scaled back to the
//! crop and only the hole's pixels are taken from it, shifted to the surrounding colours (the
//! model's output drifts a little in tone), so nothing outside the hole changes. Each part of the
//! hole far from the others is one request ([`crate::inpaint::groups`]), each billed by Google:
//! about [`PRICE_1K`] (a crop up to [`MAX_1K`] px) or [`PRICE_2K`] (larger crops, sent at most
//! [`MAX_SEND`] px across).
//!
//! The request goes to the Interactions API with `store: false`, so Google doesn't keep it (it
//! keeps interactions for 55 days otherwise). The API key is the user's own: the one saved with
//! [`set_key`] (the Windows Credential Manager, the macOS Keychain) or else the `GEMINI_API_KEY`
//! environment variable. It is sent only in the request's `x-goog-api-key` header and never
//! logged or put in an error. Cancelling stops waiting for the answer at once; a request already
//! sent may still be billed.
//!
//! Endpoint, fields and prices from ai.google.dev (image generation, Interactions API, pricing),
//! read 2026-10-22.

use photocraft_geom::Rect;
use photocraft_raster::Interrupt;
use serde_json::{Value, json};

use crate::MlError;
use crate::inpaint::{Colours, Request, encode, groups, mirror, put, window_index};

/// The model id.
pub const MODEL: &str = "gemini-nano-banana-2.1";
/// What the options bar and messages call it.
pub const NAME: &str = "Nano Banana 2.1";
/// The environment variable read when no key is saved (Google's own name for it).
pub const ENV_KEY: &str = "GEMINI_API_KEY";
/// Google's standard price of one 1K output image (USD).
pub const PRICE_1K: f64 = 0.0336;
/// Google's standard price of one 2K output image (USD).
pub const PRICE_2K: f64 = 0.0504;
/// Crops up to this long side ask for the 1K output, larger ones for 2K.
pub const MAX_1K: u32 = 1100;
/// Longest side sent: larger crops are scaled down to it (the 2K output's side).
pub const MAX_SEND: u32 = 2048;

#[cfg(not(target_arch = "wasm32"))]
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";
/// Longest wait for an answer.
#[cfg(not(target_arch = "wasm32"))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
/// Largest answer read (a 2K PNG in base64 is a few MB).
#[cfg(not(target_arch = "wasm32"))]
const MAX_ANSWER: u64 = 64 << 20;
/// Smallest crop side worth sending where the canvas allows: the model needs to see what is
/// around the hole to continue it.
const MIN_CONTEXT: u32 = 512;
/// More separate parts than this go in one request (the parts' bounding box).
const MAX_REQUESTS: usize = 4;
/// Pixels around the hole whose colours the answer is matched to.
const RING: usize = 12;
/// The largest shift the colour match applies (0–1 sRGB).
const MAX_SHIFT: f32 = 0.25;
/// Largest answer image accepted, in pixels.
const MAX_ANSWER_PIXELS: u64 = 64 << 20;

/// The aspect ratios Nano Banana 2.1 makes (its 1K/2K/4K size table).
const RATIOS: [(&str, u32, u32); 14] = [
    ("1:1", 1, 1),
    ("2:3", 2, 3),
    ("3:2", 3, 2),
    ("3:4", 3, 4),
    ("4:3", 4, 3),
    ("4:5", 4, 5),
    ("5:4", 5, 4),
    ("9:16", 9, 16),
    ("16:9", 16, 9),
    ("21:9", 21, 9),
    ("1:4", 1, 4),
    ("4:1", 4, 1),
    ("1:8", 1, 8),
    ("8:1", 8, 1),
];

/// The hole's colour in the image sent.
const RED: [f32; 3] = [1.0, 0.0, 0.0];

const PROMPT: &str = "Remove everything painted in solid pure red (#FF0000) in this image. Redraw what would naturally be \
behind it, continuing the surrounding lines, shapes, textures, patterns, screentones and colours seamlessly, in exactly \
the same style, scale and lighting. Leave everything outside the red area unchanged. Do not add new objects, people, text \
or speech, and leave no red.";

/// The prompt for one request; `extra` is what the user asked to be there instead.
pub(crate) fn prompt(extra: &str) -> String {
    let extra: String = extra.trim().chars().take(500).collect();
    if extra.is_empty() { PROMPT.to_string() } else { format!("{PROMPT} What should be there instead: {extra}") }
}

// --- The API key -------------------------------------------------------------------------------

/// Where the key in use comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// Saved with [`set_key`] in the system's credential store.
    Saved,
    /// The `GEMINI_API_KEY` environment variable.
    Environment,
}

/// The saved key as last read or written (`None`: not looked up yet).
static SAVED: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

fn saved_key() -> Option<String> {
    let mut slot = SAVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(store::read());
    }
    slot.clone().flatten()
}

fn env_key() -> Option<String> {
    std::env::var(ENV_KEY).ok().map(|k| k.trim().to_string()).filter(|k| valid_key(k).is_ok())
}

/// The key a request uses: the saved one, else the environment's.
fn key() -> Option<String> {
    saved_key().or_else(env_key)
}

/// Where the key comes from, `None` when there is none.
pub fn key_source() -> Option<KeySource> {
    if saved_key().is_some() {
        Some(KeySource::Saved)
    } else if env_key().is_some() {
        Some(KeySource::Environment)
    } else {
        None
    }
}

/// Whether the system has a credential store [`set_key`] can save in.
pub fn can_save_key() -> bool {
    store::AVAILABLE
}

/// Is `key` shaped like an API key? Printable ASCII without spaces, at most 256 characters (says
/// why not, without repeating the key).
pub(crate) fn valid_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("the API key is empty".into());
    }
    if key.len() > 256 {
        return Err("that is too long for an API key".into());
    }
    if !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("an API key has only letters, digits and punctuation, no spaces".into());
    }
    Ok(())
}

/// Save the API key in the system's credential store (`None` deletes it). The key is checked
/// for shape only; Google says whether it works on the first removal.
pub fn set_key(key: Option<&str>) -> Result<(), String> {
    let key = key.map(str::trim);
    if let Some(k) = key {
        valid_key(k)?;
    }
    store::write(key)?;
    *SAVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key.map(str::to_string));
    Ok(())
}

/// The credential store: the Windows Credential Manager or the macOS Keychain.
#[cfg(any(windows, target_os = "macos"))]
mod store {
    pub(super) const AVAILABLE: bool = true;
    const SERVICE: &str = "PhotoCraft";
    const USER: &str = "Google Gemini API key";

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, USER).map_err(|e| format!("the system's credential store isn't available ({e})"))
    }

    pub(super) fn read() -> Option<String> {
        let key = entry().ok()?.get_password().ok()?;
        let key = key.trim().to_string();
        super::valid_key(&key).is_ok().then_some(key)
    }

    pub(super) fn write(key: Option<&str>) -> Result<(), String> {
        let e = entry()?;
        match key {
            Some(k) => e.set_password(k).map_err(|e| format!("couldn't save the API key ({e})")),
            None => match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("couldn't delete the API key ({e})")),
            },
        }
    }
}

/// No credential store here: the key comes from the environment.
#[cfg(not(any(windows, target_os = "macos")))]
mod store {
    pub(super) const AVAILABLE: bool = false;

    pub(super) fn read() -> Option<String> {
        None
    }

    pub(super) fn write(_: Option<&str>) -> Result<(), String> {
        Err(format!("this system has no credential store PhotoCraft can use: set the {} environment variable instead", super::ENV_KEY))
    }
}

/// Whether a removal can use the model: a key is set (the web build has no network access to
/// Google's API).
pub fn availability() -> Result<(), MlError> {
    if cfg!(target_arch = "wasm32") {
        return Err(MlError::Unsupported);
    }
    match key() {
        Some(_) => Ok(()),
        None => Err(MlError::NotConfigured(format!("no Google API key for {NAME} (add one in the Remove tool's options, or set {ENV_KEY})"))),
    }
}

/// What one removal costs at most, in USD: one image per part of the hole.
pub fn price(sides: &[u32]) -> f64 {
    sides.iter().map(|s| if *s <= MAX_1K { PRICE_1K } else { PRICE_2K }).sum()
}

// --- Geometry ----------------------------------------------------------------------------------

/// The crop sent for a part of the hole with bounding box `part`: the part and context on every
/// side (half its size, at least 96 px), grown to [`MIN_CONTEXT`] across where the canvas allows,
/// kept inside the canvas (shifted rather than cut where it can be).
pub(crate) fn context(part: Rect, canvas: Rect) -> Rect {
    let side = part.width().max(part.height());
    let margin = i32::try_from((side / 2).max(96)).unwrap_or(i32::MAX);
    let r = part.inflate(margin);
    // One axis: at least `min` long, then moved inside `lo..hi` (or cut to it when longer).
    let fit = |a: i32, b: i32, lo: i32, hi: i32| -> (i32, i32) {
        let min = i64::from(MIN_CONTEXT);
        let (mut a, mut b) = (i64::from(a), i64::from(b));
        if b - a < min {
            let d = min - (b - a);
            a -= d / 2;
            b += d - d / 2;
        }
        let (lo, hi) = (i64::from(lo), i64::from(hi));
        if b - a >= hi - lo {
            return (lo as i32, hi as i32);
        }
        if a < lo {
            b += lo - a;
            a = lo;
        }
        if b > hi {
            a -= b - hi;
            b = hi;
        }
        (a as i32, b as i32)
    };
    let (x0, x1) = fit(r.x0, r.x1, canvas.x0, canvas.x1);
    let (y0, y1) = fit(r.y0, r.y1, canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// The aspect ratio closest to `w`:`h` that the model makes, and the smallest frame of that ratio
/// holding `w` × `h` (the crop is centred in it, mirrored out to its edges).
pub(crate) fn frame(w: u32, h: u32) -> (&'static str, u32, u32) {
    let (w, h) = (w.max(1), h.max(1));
    let a = (f64::from(w) / f64::from(h)).ln();
    let off = |(_, rw, rh): &(&str, u32, u32)| ((f64::from(*rw) / f64::from(*rh)).ln() - a).abs();
    let (name, rw, rh) = RATIOS.iter().copied().min_by(|x, y| off(x).total_cmp(&off(y))).unwrap_or(("1:1", 1, 1));
    // Width from the height, or the height from the width, whichever holds the crop.
    let wide = (u64::from(h) * u64::from(rw)).div_ceil(u64::from(rh));
    if wide >= u64::from(w) {
        (name, u32::try_from(wide).unwrap_or(u32::MAX), h)
    } else {
        (name, w, u32::try_from((u64::from(w) * u64::from(rh)).div_ceil(u64::from(rw))).unwrap_or(u32::MAX))
    }
}

/// The size sent for a `w` × `h` frame: scaled down to [`MAX_SEND`] across when larger.
pub(crate) fn send_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_SEND {
        return (w.max(1), h.max(1));
    }
    let s = f64::from(MAX_SEND) / f64::from(long);
    ((f64::from(w) * s).round().max(1.0) as u32, (f64::from(h) * s).round().max(1.0) as u32)
}

/// The output size asked for an image sent `w` × `h`.
pub(crate) fn image_size(w: u32, h: u32) -> &'static str {
    if w.max(h) <= MAX_1K { "1K" } else { "2K" }
}

/// Interleaved RGB `src` (`sw` × `sh`) resampled to `dw` × `dh`: a tent filter as wide as the
/// scale when shrinking (an area average), bilinear when enlarging.
pub(crate) fn resize(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * 3 {
        return vec![0.0; dw * dh * 3];
    }
    if (sw, sh) == (dw, dh) {
        return src[..sw * sh * 3].to_vec();
    }
    // Per output index: the source indices and weights along one axis.
    let taps = |from: usize, to: usize| -> Vec<Vec<(usize, f32)>> {
        let scale = from as f64 / to as f64;
        let support = scale.max(1.0);
        (0..to)
            .map(|o| {
                let c = (o as f64 + 0.5) * scale - 0.5;
                let (lo, hi) = ((c - support).floor() as i64, (c + support).ceil() as i64);
                let mut t: Vec<(usize, f32)> = (lo..=hi)
                    .filter_map(|i| {
                        let w = 1.0 - ((i as f64 - c).abs() / support);
                        (w > 0.0).then(|| (i.clamp(0, from as i64 - 1) as usize, w as f32))
                    })
                    .collect();
                let sum: f32 = t.iter().map(|(_, w)| w).sum();
                if sum > 0.0 {
                    for (_, w) in &mut t {
                        *w /= sum;
                    }
                } else {
                    t = vec![((c.round().max(0.0) as usize).min(from - 1), 1.0)];
                }
                t
            })
            .collect()
    };
    let (tx, ty) = (taps(sw, dw), taps(sh, dh));
    let mut rows = vec![0.0f32; dw * sh * 3];
    for y in 0..sh {
        for (x, t) in tx.iter().enumerate() {
            let mut acc = [0.0f32; 3];
            for &(i, w) in t {
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += src.get((y * sw + i) * 3 + c).copied().unwrap_or(0.0) * w;
                }
            }
            if let Some(d) = rows.get_mut((y * dw + x) * 3..(y * dw + x) * 3 + 3) {
                d.copy_from_slice(&acc);
            }
        }
    }
    let mut out = vec![0.0f32; dw * dh * 3];
    for (y, t) in ty.iter().enumerate() {
        for x in 0..dw {
            let mut acc = [0.0f32; 3];
            for &(i, w) in t {
                for (c, a) in acc.iter_mut().enumerate() {
                    *a += rows.get((i * dw + x) * 3 + c).copied().unwrap_or(0.0) * w;
                }
            }
            if let Some(d) = out.get_mut((y * dw + x) * 3..(y * dw + x) * 3 + 3) {
                d.copy_from_slice(&acc);
            }
        }
    }
    out
}

/// `mask` (`w` × `h`) grown by `r` pixels (a square neighbourhood, by prefix sums).
fn dilate(mask: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let grow = |get: &dyn Fn(usize, usize) -> bool, n: usize, m: usize| -> Vec<bool> {
        // `n` lines of `m` samples; a sample is set when any within `r` along its line is.
        let mut out = vec![false; n * m];
        let mut prefix = vec![0u32; m + 1];
        for line in 0..n {
            for i in 0..m {
                prefix[i + 1] = prefix[i] + u32::from(get(line, i));
            }
            for i in 0..m {
                let (a, b) = (i.saturating_sub(r), (i + r + 1).min(m));
                out[line * m + i] = prefix[b] > prefix[a];
            }
        }
        out
    };
    let rows = grow(&|y, x| mask.get(y * w + x).copied().unwrap_or(false), h, w);
    let cols = grow(&|x, y| rows.get(y * w + x).copied().unwrap_or(false), w, h);
    let mut out = vec![false; w * h];
    for x in 0..w {
        for y in 0..h {
            out[y * w + x] = cols[x * h + y];
        }
    }
    out
}

// --- One request -------------------------------------------------------------------------------

/// What goes to Google for one part of the hole, and how its answer maps back.
pub(crate) struct Crop {
    /// The crop on the canvas.
    pub(crate) rect: Rect,
    /// The frame of a supported ratio around it (canvas coordinates; may reach past the canvas).
    pub(crate) frame: Rect,
    pub(crate) ratio: &'static str,
    /// The size sent.
    pub(crate) send: (u32, u32),
    /// The crop's sRGB pixels (interleaved RGB), and which are hole.
    pub(crate) rgb: Vec<f32>,
    pub(crate) hole: Vec<bool>,
}

/// Pixel `px` of the request as the model sees it: sRGB (0–1) over white where not opaque, and
/// its alpha.
fn model_rgb(req: &Request, px: &[f32]) -> ([f32; 3], f32) {
    let enc = |v: f32| if req.linear { encode(v) } else { v.clamp(0.0, 1.0) };
    let rgb = match req.colours {
        Colours::Rgb => [px.first(), px.get(1), px.get(2)].map(|v| enc(v.copied().unwrap_or(0.0))),
        Colours::Gray => [enc(px.first().copied().unwrap_or(0.0)); 3],
    };
    let a = req.alpha.and_then(|a| px.get(a)).copied().unwrap_or(1.0).clamp(0.0, 1.0);
    (rgb.map(|v| v * a + (1.0 - a)), a)
}

/// The crop for the part of the hole in `part`, read from the source: the hole red, the frame
/// beyond the crop mirrored. `MlError::Unsuitable` when transparent pixels surround the hole and
/// the request doesn't allow it.
pub(crate) fn crop(req: &Request, part: Rect, read: &mut dyn FnMut(Rect) -> Vec<f32>) -> Result<Crop, MlError> {
    let rect = context(part, req.canvas);
    if rect.is_empty() {
        return Err(MlError::BadInput("the hole is outside the canvas".into()));
    }
    let (cw, ch) = (rect.width(), rect.height());
    let (ratio, fw, fh) = frame(cw, ch);
    let fx0 = rect.x0.saturating_sub(i32::try_from((fw - cw) / 2).unwrap_or(0));
    let fy0 = rect.y0.saturating_sub(i32::try_from((fh - ch) / 2).unwrap_or(0));
    let frame = Rect::from_xywh(fx0, fy0, fw, fh);
    let src = read(rect);
    let n = req.ch;
    if src.len() != cw as usize * ch as usize * n {
        return Err(MlError::BadInput("the source read doesn't match the crop".into()));
    }
    let mut rgb = Vec::with_capacity(cw as usize * ch as usize * 3);
    let mut hole = Vec::with_capacity(cw as usize * ch as usize);
    let mut see_through = 0usize;
    for (i, px) in src.chunks_exact(n).enumerate() {
        let (x, y) = (rect.x0 + (i % cw as usize) as i32, rect.y0 + (i / cw as usize) as i32);
        let h = window_index(req.window, x, y).and_then(|k| req.hole.get(k)).copied().unwrap_or(false);
        let (c, a) = model_rgb(req, px);
        if !h && a < 0.5 {
            see_through += 1;
        }
        rgb.extend_from_slice(&c);
        hole.push(h);
    }
    if see_through > 0 && !req.transparent_unknown {
        return Err(MlError::Unsuitable(format!(
            "{NAME} needs opaque pixels around what it removes: turn on Sample All Layers, or remove on a layer without transparency"
        )));
    }
    Ok(Crop { rect, frame, ratio, send: send_size(fw, fh), rgb, hole })
}

impl Crop {
    /// The image sent: the frame (the crop centred, mirrored beyond it) with the hole red, at
    /// the size sent, as interleaved sRGB.
    pub(crate) fn image(&self) -> Vec<f32> {
        let (cw, ch) = (self.rect.width() as usize, self.rect.height() as usize);
        let (fw, fh) = (self.frame.width() as usize, self.frame.height() as usize);
        let mut out = Vec::with_capacity(fw * fh * 3);
        for fy in 0..fh as i32 {
            for fx in 0..fw as i32 {
                let x = mirror(self.frame.x0 + fx, self.rect.x0, self.rect.x1) - self.rect.x0;
                let y = mirror(self.frame.y0 + fy, self.rect.y0, self.rect.y1) - self.rect.y0;
                let k = (y as usize).min(ch.saturating_sub(1)) * cw + (x as usize).min(cw.saturating_sub(1));
                let c = if self.hole.get(k).copied().unwrap_or(false) { RED } else { [0, 1, 2].map(|c| self.rgb.get(k * 3 + c).copied().unwrap_or(0.0)) };
                out.extend_from_slice(&c);
            }
        }
        let (sw, sh) = self.send;
        resize(&out, fw, fh, sw as usize, sh as usize)
    }

    /// The answer (`aw` × `ah`, interleaved sRGB) for the crop's pixels: scaled to the frame,
    /// cut to the crop, and its colours shifted to match the original in a ring around the hole.
    pub(crate) fn fitted(&self, answer: &[f32], aw: usize, ah: usize) -> Vec<f32> {
        let (cw, ch) = (self.rect.width() as usize, self.rect.height() as usize);
        let (fw, fh) = (self.frame.width() as usize, self.frame.height() as usize);
        let framed = resize(answer, aw, ah, fw, fh);
        let (ox, oy) = ((self.rect.x0 - self.frame.x0) as usize, (self.rect.y0 - self.frame.y0) as usize);
        let mut out = Vec::with_capacity(cw * ch * 3);
        for y in 0..ch {
            let start = ((y + oy) * fw + ox) * 3;
            match framed.get(start..start + cw * 3) {
                Some(row) => out.extend_from_slice(row),
                None => out.extend(std::iter::repeat_n(0.0, cw * 3)),
            }
        }
        // The ring: just outside the hole, where the answer should equal the original.
        let near = dilate(&self.hole, cw, ch, RING);
        let (mut sum_o, mut sum_a, mut n) = ([0.0f64; 3], [0.0f64; 3], 0u64);
        for (k, (&h, &r)) in self.hole.iter().zip(&near).enumerate() {
            if r && !h {
                for c in 0..3 {
                    sum_o[c] += f64::from(self.rgb.get(k * 3 + c).copied().unwrap_or(0.0));
                    sum_a[c] += f64::from(out.get(k * 3 + c).copied().unwrap_or(0.0));
                }
                n += 1;
            }
        }
        if n > 0 {
            let shift = [0, 1, 2].map(|c| (((sum_o[c] - sum_a[c]) / n as f64) as f32).clamp(-MAX_SHIFT, MAX_SHIFT));
            for px in out.as_chunks_mut::<3>().0 {
                for (v, s) in px.iter_mut().zip(shift) {
                    *v = (*v + s).clamp(0.0, 1.0);
                }
            }
        }
        out
    }
}

/// The request's JSON body for an image (PNG, base64) of ratio `ratio` sent `send` large.
pub(crate) fn body(png_base64: &str, prompt: &str, ratio: &str, send: (u32, u32)) -> String {
    json!({
        "model": MODEL,
        // Google keeps stored interactions for 55 days; a removal has nothing to come back to.
        "store": false,
        "input": [
            {"type": "text", "text": prompt},
            {"type": "image", "mime_type": "image/png", "data": png_base64},
        ],
        "response_format": {"type": "image", "mime_type": "image/png", "aspect_ratio": ratio, "image_size": image_size(send.0, send.1)},
    })
    .to_string()
}

/// At most `n` characters of `s`.
fn clip(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if t.len() < s.len() { format!("{t}…") } else { t }
}

/// An error answer (HTTP `status`, JSON body `text`) as a message the user can act on.
pub(crate) fn api_error(status: u16, text: &str) -> MlError {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let message = v.pointer("/error/message").and_then(Value::as_str).map(|m| clip(m.trim(), 300)).unwrap_or_else(|| clip(text.trim(), 200));
    MlError::Inference(match status {
        401 | 403 => format!("Google refused the API key ({status}): {message}"),
        429 => format!("Google's quota or rate limit for {NAME} was reached: {message}"),
        400..=499 => format!("Google refused the request ({status}): {message}"),
        _ => format!("Google's service failed ({status}): {message}"),
    })
}

/// The image of an answer (the last one the model output, as file bytes), or why there is none.
pub(crate) fn answer_image(text: &str) -> Result<Vec<u8>, MlError> {
    use base64::Engine as _;
    let v: Value = serde_json::from_str(text).map_err(|e| MlError::Inference(format!("Google's answer isn't JSON ({e})")))?;
    if v.get("error").is_some() {
        return Err(api_error(500, text));
    }
    let status = v.get("status").and_then(Value::as_str).unwrap_or("completed");
    let (mut image, mut words) = (None, Vec::new());
    for step in v.get("steps").and_then(Value::as_array).into_iter().flatten() {
        if step.get("type").and_then(Value::as_str) != Some("model_output") {
            continue;
        }
        for block in step.get("content").and_then(Value::as_array).into_iter().flatten() {
            match block.get("type").and_then(Value::as_str) {
                Some("image") => image = block.get("data").and_then(Value::as_str).or(image),
                Some("text") => words.extend(block.get("text").and_then(Value::as_str)),
                _ => {}
            }
        }
    }
    let Some(data) = image else {
        let said = words.join(" ");
        return Err(MlError::Inference(if said.trim().is_empty() {
            format!("Google sent no image (the interaction is {status})")
        } else {
            format!("Google sent no image: {}", clip(said.trim(), 300))
        }));
    };
    base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| MlError::Inference(format!("Google's image isn't base64 ({e})")))
}

/// An answer image's bytes decoded to interleaved sRGB, with its size.
fn decode_answer(bytes: &[u8]) -> Result<(Vec<f32>, usize, usize), MlError> {
    let img = photocraft_codecs::decode(bytes).map_err(|e| MlError::Inference(format!("Google's image didn't decode ({e})")))?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > MAX_ANSWER_PIXELS {
        return Err(MlError::Inference(format!("Google's image has an unusable size ({w} × {h})")));
    }
    let rgba = img.to_rgba_f32();
    let rgb: Vec<f32> = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    if rgb.len() != w * h * 3 {
        return Err(MlError::Inference("Google's image didn't decode whole".into()));
    }
    Ok((rgb, w, h))
}

/// Sends one request body, returns the answer's body (the network, or a test's stand-in).
pub(crate) type Transport<'a> = dyn FnMut(&str, &Interrupt) -> Result<String, MlError> + 'a;

/// Fill the hole of `req` with the model (`read` gives the source's pixels), `extra` what the
/// user asked to be there. The window comes back with only the hole changed.
pub fn fill(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, extra: &str, ctl: &Interrupt) -> Result<Vec<f32>, MlError> {
    let Some(key) = key() else {
        return Err(availability().err().unwrap_or(MlError::Unsupported));
    };
    fill_via(req, read, extra, ctl, &mut |body, ctl| post(&key, body, ctl))
}

/// [`fill`] through `send`.
pub(crate) fn fill_via(
    req: &Request,
    read: &mut dyn FnMut(Rect) -> Vec<f32>,
    extra: &str,
    ctl: &Interrupt,
    send: &mut Transport<'_>,
) -> Result<Vec<f32>, MlError> {
    let (w, h) = (req.window.width() as usize, req.window.height() as usize);
    if req.hole.len() != w * h || req.pixels.len() != w * h * req.ch {
        return Err(MlError::BadInput("the hole or pixels don't match the window".into()));
    }
    let mut parts = groups(req.window, req.hole);
    if parts.len() > MAX_REQUESTS
        && let Some(all) = parts.iter().copied().reduce(|a, b| a.union(&b))
    {
        parts = vec![all];
    }
    if parts.is_empty() {
        return Ok(req.pixels.to_vec());
    }
    let mut out = req.pixels.to_vec();
    let prompt = prompt(extra);
    let count = parts.len() as f32;
    for (g, part) in parts.iter().enumerate() {
        ctl.check()?;
        let c = crop(req, *part, read)?;
        let image = c.image();
        let (sw, sh) = c.send;
        let img = photocraft_codecs::Image::from_normalized(sw, sh, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, &image)
            .map_err(|e| MlError::BadInput(format!("couldn't make the image to send ({e})")))?;
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default())
            .map_err(|e| MlError::BadInput(format!("couldn't make the image to send ({e})")))?;
        let b64 = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&png)
        };
        let progress = move |f: f32| ((g as f32 + f) / count).clamp(0.0, 1.0);
        let report = |f: f32| ctl.progress(progress(f));
        let cancelled = || ctl.cancelled();
        let span = Interrupt::new(&cancelled, &report);
        let answer = send(&body(&b64, &prompt, c.ratio, c.send), &span)?;
        ctl.check()?;
        let (rgb, aw, ah) = decode_answer(&answer_image(&answer)?)?;
        let fitted = c.fitted(&rgb, aw, ah);
        let cw = c.rect.width() as usize;
        for (k, h) in c.hole.iter().enumerate() {
            if !*h {
                continue;
            }
            let (x, y) = (c.rect.x0 + (k % cw) as i32, c.rect.y0 + (k / cw) as i32);
            let Some(j) = window_index(req.window, x, y) else { continue };
            let rgb = [0, 1, 2].map(|i| fitted.get(k * 3 + i).copied().unwrap_or(0.0));
            put(req, &mut out, j, rgb);
        }
        ctl.progress(progress(1.0));
    }
    Ok(out)
}

/// Send `body` to Google with `key` on a thread, waiting for the answer while checking `ctl`
/// (cancelling stops waiting; the answer, if one comes, is dropped). Progress is an estimate from
/// the time waited (an answer takes ten to thirty seconds).
#[cfg(not(target_arch = "wasm32"))]
fn post(key: &str, body: &str, ctl: &Interrupt) -> Result<String, MlError> {
    use std::sync::mpsc::RecvTimeoutError;
    let (tx, rx) = std::sync::mpsc::channel();
    let (key, body) = (key.to_string(), body.to_string());
    std::thread::Builder::new()
        .name("nano-banana".into())
        .spawn(move || {
            let _ = tx.send(request(&key, &body));
        })
        .map_err(|e| MlError::Inference(format!("couldn't start the request ({e})")))?;
    let start = std::time::Instant::now();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(150)) {
            Ok(answer) => return answer,
            Err(RecvTimeoutError::Timeout) => {
                ctl.check()?;
                let t = start.elapsed().as_secs_f32();
                ctl.progress(0.95 * (1.0 - (-t / 15.0).exp()));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(MlError::Inference("the request stopped without an answer".into())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn post(_: &str, _: &str, _: &Interrupt) -> Result<String, MlError> {
    Err(MlError::Unsupported)
}

/// One HTTPS request to the Interactions API; the answer's body, or the error it describes.
#[cfg(not(target_arch = "wasm32"))]
fn request(key: &str, body: &str) -> Result<String, MlError> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
    let mut answer = agent
        .post(ENDPOINT)
        .header("x-goog-api-key", key)
        .header("content-type", "application/json")
        .send(body)
        .map_err(|e| MlError::Inference(format!("couldn't reach Google ({e})")))?;
    let status = answer.status().as_u16();
    let text = answer
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_string()
        .map_err(|e| MlError::Inference(format!("Google's answer didn't arrive whole ({e})")))?;
    if !(200..300).contains(&status) {
        return Err(api_error(status, &text));
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
