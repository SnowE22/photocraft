//! Nano Banana 2.1 (`gemini-nano-banana-2.1`, Google's Gemini image model) as a removal model:
//! paid, online, and much better than the local models at knowing what lies behind something.
//!
//! The hole goes to Google the way people use the model in Photoshop, but without the manual
//! work: an area around the hole ([`Area::Near`], the default: about twice the hole, at most
//! [`NEAR_MAX`] px across unless the hole needs more) or the whole page ([`Area::Page`]; on a
//! page larger than [`MAX_SEND`], a wide area around the hole), with the hole shown either as a
//! separate mask image ([`Mark::Mask`]) or painted one solid colour ([`Mark::Paint`], the default,
//! [`MARKERS`]: red, or on a colour picture whichever of the markers it has least of), and a
//! prompt asking for it to be removed and what is behind it drawn in the surrounding style. The
//! model draws its whole output image at about the same level of detail, so the less of the page
//! it is sent, the sharper the fill: a 1200 × 1800 page sent whole came back at 2K with its fine
//! hatching soft; the area near the hole comes back at about twice the page's resolution. The
//! answer is scaled back to the crop and only the hole's pixels are taken from it, shifted to the
//! surrounding colours (the model's output drifts a little in tone), so nothing outside the hole
//! changes. Each part of the hole far from the others is one request ([`crate::inpaint::groups`]),
//! each billed by Google: about [`PRICE_1K`] (a crop up to [`MAX_1K`] px) or [`PRICE_2K`] (larger
//! crops, sent at most [`MAX_SEND`] px across).
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
/// Crops up to this long side ask for the 1K output (at least about 1.6 times their size),
/// larger ones for 2K.
pub const MAX_1K: u32 = 640;
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
/// Smallest crop side worth sending where the canvas allows, with [`Area::Page`] on a page larger
/// than [`MAX_SEND`]: the model needs to see what is around the hole to continue it.
const MIN_CONTEXT: u32 = 1024;
/// [`Area::Near`]: the smallest crop side where the canvas allows.
pub const NEAR_MIN: u32 = 768;
/// [`Area::Near`]: the largest crop side, unless the hole itself needs more (the 2K output still
/// draws that at about 1.6 times).
pub const NEAR_MAX: u32 = 1280;
/// [`Area::Near`]: the least context kept on each side of a hole too large for [`NEAR_MAX`].
const NEAR_MARGIN: u32 = 64;
/// More separate parts than this go in one request (the parts' bounding box).
const MAX_REQUESTS: usize = 4;
/// Pixels around the hole that [`Crop::align`] leaves out (the model may redraw them).
const RING: usize = 24;
/// Least correlation between the answer's detail and the page's away from the hole: below it the
/// model changed the picture's framing (zoomed in, shifted) beyond what [`Crop::align`] corrects,
/// and its fill wouldn't meet the page.
const MIN_MATCH: f64 = 0.5;
/// The largest shift the colour match applies (0–1 sRGB).
const MAX_SHIFT: f32 = 0.25;
/// The range of the contrast match's gain.
const MIN_GAIN: f64 = 0.8;
const MAX_GAIN: f64 = 1.6;
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

/// The colours the hole can be painted in the image sent, and their names in the prompt: the
/// first one the picture around the hole has least of (red, unless the picture is red).
pub(crate) const MARKERS: [([f32; 3], &str); 4] = [
    ([1.0, 0.0, 0.0], "solid pure red (#FF0000)"),
    ([0.0, 1.0, 0.0], "solid pure green (#00FF00)"),
    ([0.0, 0.0, 1.0], "solid pure blue (#0000FF)"),
    ([1.0, 0.0, 1.0], "solid pure magenta (#FF00FF)"),
];

/// What both prompts ask of the fill: continue only what reaches the hole, keep blank paper blank.
/// Without it the model made up a scene: asked to remove a title box at the foot of a manga page
/// (legs over blank paper), it drew a marsh of cattails, water and bushes into the box and
/// redrew the paper around it as ground (2026-10-23).
const RESTRAINT: &str = "Continue only what reaches the edge of that area from outside it: the lines, shapes, \
hatching, screentones and colours that run into it, in exactly the same style, scale and lighting. Where the picture \
next to the area is blank paper or a flat tone, keep the matching part of the area blank or flat too. Do not invent \
scenery, ground, plants, water, objects, people, text or speech bubbles that are not already there.";

/// The prompt for one request with the hole painted in `marker` (its name, [`MARKERS`]); `extra`
/// is what the user asked to be there instead.
pub(crate) fn prompt(extra: &str, marker: &str) -> String {
    let base = format!(
        "Remove the area painted in {marker} from this image and fill it with what would naturally be behind it. \
{RESTRAINT} Keep the rest of the image exactly as it is, at the same framing, and leave none of the marker colour."
    );
    let extra: String = extra.trim().chars().take(500).collect();
    if extra.is_empty() { base } else { format!("{base} What should be there instead: {extra}") }
}

/// How much of the page the model is sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Area {
    /// About twice the hole, [`NEAR_MIN`]–[`NEAR_MAX`] px across: sharper.
    #[default]
    Near,
    /// The whole page (on one larger than [`MAX_SEND`], a wide area around the hole): more to go
    /// on, softer.
    Page,
}

/// How a removal asks the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub mark: Mark,
    pub area: Area,
}

/// How the hole is shown to the model. Painted, it keeps the picture's framing and what is around
/// the hole: on a title box at the foot of a manga page it continued the legs over blank paper
/// twice, aligned to 0.97, where with a mask it drew a marsh into the box once and zoomed the
/// whole picture in once (a 0.02 match, refused); on lashes it kept the eye a mask erased
/// (2026-10-23). On another page's title the mask continued the hatching a little more naturally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    /// Painted in a marker colour in the one image sent ([`MARKERS`]): the Photoshop routine.
    #[default]
    Paint,
    /// The picture as it is, and a second image, the mask: white where the hole is.
    Mask,
}

/// The prompt for one request with the hole shown as a mask (the second image).
pub(crate) fn mask_prompt(extra: &str) -> String {
    let base = format!(
        "The second image is a mask for the first: remove whatever is in the white area of the mask from the first image \
and fill that area with what would naturally be behind it. {RESTRAINT} Return only the edited first image, at the same \
framing, with every pixel outside the white area unchanged."
    );
    let extra: String = extra.trim().chars().take(500).collect();
    if extra.is_empty() { base } else { format!("{base} What should be there instead: {extra}") }
}

/// The marker the context pixels (interleaved sRGB `rgb`, where `hole` is false) have least of:
/// pixels within 0.3 of a marker in every channel count as it.
pub(crate) fn pick_marker(rgb: &[f32], hole: &[bool]) -> usize {
    let mut counts = [0usize; MARKERS.len()];
    for (px, h) in rgb.chunks(3).zip(hole) {
        if *h || px.len() < 3 {
            continue;
        }
        for (count, (m, _)) in counts.iter_mut().zip(MARKERS) {
            if px.iter().zip(m).all(|(a, b)| (a - b).abs() < 0.3) {
                *count += 1;
            }
        }
    }
    // The first of the least used, so a picture without any of them gets red.
    let least = counts.iter().copied().min().unwrap_or(0);
    counts.iter().position(|c| *c == least).unwrap_or(0)
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

/// The crop sent for a part of the hole with bounding box `part`, kept inside the canvas (shifted
/// rather than cut where it can be) and scaled down to [`MAX_SEND`] when larger.
/// - [`Area::Near`]: the part with half its longer side around it (at least 192 px), cut to
///   [`NEAR_MAX`] across (keeping [`NEAR_MARGIN`] around a part too large for that), then grown
///   to [`NEAR_MIN`] across.
/// - [`Area::Page`]: the whole canvas when it fits in [`MAX_SEND`] (sent as it is); on a larger
///   canvas the part with context on every side (its own size, at least 384 px), grown to
///   [`MIN_CONTEXT`] across.
pub(crate) fn context(part: Rect, canvas: Rect, area: Area) -> Rect {
    let side = part.width().max(part.height());
    let (r, least) = match area {
        Area::Page if canvas.width().max(canvas.height()) <= MAX_SEND => return canvas,
        Area::Page => (part.inflate(i32::try_from(side.max(384)).unwrap_or(i32::MAX)), MIN_CONTEXT),
        Area::Near => {
            let wanted = (side / 2).max(192);
            let room = NEAR_MAX.saturating_sub(side) / 2;
            let margin = wanted.min(room.max(NEAR_MARGIN));
            (part.inflate(i32::try_from(margin).unwrap_or(i32::MAX)), NEAR_MIN)
        }
    };
    // One axis: at least `least` long, then moved inside `lo..hi` (or cut to it when longer).
    let fit = |a: i32, b: i32, lo: i32, hi: i32| -> (i32, i32) {
        let min = i64::from(least);
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
    /// The hole's colour in the image sent ([`MARKERS`]).
    pub(crate) marker: usize,
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
pub(crate) fn crop(req: &Request, part: Rect, area: Area, read: &mut dyn FnMut(Rect) -> Vec<f32>) -> Result<Crop, MlError> {
    let rect = context(part, req.canvas, area);
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
    let marker = pick_marker(&rgb, &hole);
    Ok(Crop { rect, frame, ratio, send: send_size(fw, fh), rgb, hole, marker })
}

impl Crop {
    /// The image sent: the frame (the crop centred, mirrored beyond it) with the hole red, at
    /// the size sent, as interleaved sRGB.
    pub(crate) fn image(&self) -> Vec<f32> {
        self.framed(|k, hole| if hole { MARKERS.get(self.marker).map_or([1.0, 0.0, 0.0], |m| m.0) } else { self.px(k) })
    }

    /// The image sent with [`Mark::Mask`]: the frame as it is.
    pub(crate) fn picture(&self) -> Vec<f32> {
        self.framed(|k, _| self.px(k))
    }

    /// The mask sent with [`Mark::Mask`]: white where the hole is, black elsewhere.
    pub(crate) fn mask(&self) -> Vec<f32> {
        self.framed(|_, hole| if hole { [1.0; 3] } else { [0.0; 3] })
    }

    fn px(&self, k: usize) -> [f32; 3] {
        [0, 1, 2].map(|c| self.rgb.get(k * 3 + c).copied().unwrap_or(0.0))
    }

    /// The frame (the crop centred, mirrored beyond it) at the size sent, each pixel `colour(crop
    /// pixel, in the hole)`.
    fn framed(&self, colour: impl Fn(usize, bool) -> [f32; 3]) -> Vec<f32> {
        let (cw, ch) = (self.rect.width() as usize, self.rect.height() as usize);
        let (fw, fh) = (self.frame.width() as usize, self.frame.height() as usize);
        let mut out = Vec::with_capacity(fw * fh * 3);
        for fy in 0..fh as i32 {
            for fx in 0..fw as i32 {
                let x = mirror(self.frame.x0 + fx, self.rect.x0, self.rect.x1) - self.rect.x0;
                let y = mirror(self.frame.y0 + fy, self.rect.y0, self.rect.y1) - self.rect.y0;
                let k = (y as usize).min(ch.saturating_sub(1)) * cw + (x as usize).min(cw.saturating_sub(1));
                out.extend_from_slice(&colour(k, self.hole.get(k).copied().unwrap_or(false)));
            }
        }
        let (sw, sh) = self.send;
        resize(&out, fw, fh, sw as usize, sh as usize)
    }

    /// The answer (`aw` × `ah`, interleaved sRGB) for the crop's pixels: placed over the frame,
    /// cut to the crop, and its black and white points matched to the original's outside the
    /// hole ([`levels`]); where it was found to lie, and how well it matches there (the
    /// correlation of their detail away from the hole, 1 when it can't be judged).
    pub(crate) fn fitted_placed(&self, answer: &[f32], aw: usize, ah: usize) -> (Vec<f32>, Placement, f64) {
        let (cw, ch) = (self.rect.width() as usize, self.rect.height() as usize);
        let fw = self.frame.width() as usize;
        let (framed, placed, matched) = self.framed_answer(answer, aw, ah);
        let (ox, oy) = ((self.rect.x0 - self.frame.x0) as usize, (self.rect.y0 - self.frame.y0) as usize);
        let mut out = Vec::with_capacity(cw * ch * 3);
        for y in 0..ch {
            let start = ((y + oy) * fw + ox) * 3;
            match framed.get(start..start + cw * 3) {
                Some(row) => out.extend_from_slice(row),
                None => out.extend(std::iter::repeat_n(0.0, cw * 3)),
            }
        }
        // Every few pixels outside the hole (at most about 200,000), original and answer.
        let step = ((cw * ch) / 200_000).max(1);
        let outside: Vec<usize> = (0..cw * ch).step_by(step).filter(|k| !self.hole.get(*k).copied().unwrap_or(true)).collect();
        if !outside.is_empty() {
            let fit = [0, 1, 2].map(|c| {
                let at = |v: &[f32], k: usize| f64::from(v.get(k * 3 + c).copied().unwrap_or(0.0));
                let (o, a): (Vec<f64>, Vec<f64>) = outside.iter().map(|&k| (at(&self.rgb, k), at(&out, k))).unzip();
                levels(o, a)
            });
            for px in out.as_chunks_mut::<3>().0 {
                for (v, (from, gain, to)) in px.iter_mut().zip(fit) {
                    *v = (to + (*v - from) * gain).clamp(0.0, 1.0);
                }
            }
        }
        (out, placed, matched)
    }

    /// The answer over the frame (interleaved sRGB, the frame's size). The model keeps the
    /// picture's proportions: it scales it evenly to cover its output and cuts what overhangs (a
    /// 750 × 500 frame came back 1264 × 848: scaled 1.696, 4 px cut from each side), so stretching
    /// the answer to the frame put it up to 2–3 px off at the sides. The answer is scaled back
    /// evenly, then placed where it matches the frame's own pixels best ([`Self::align`]).
    fn framed_answer(&self, answer: &[f32], aw: usize, ah: usize) -> (Vec<f32>, Placement, f64) {
        let (fw, fh) = (self.frame.width() as usize, self.frame.height() as usize);
        if aw == 0 || ah == 0 || fw == 0 || fh == 0 {
            return (vec![0.0; fw * fh * 3], Placement::IDENTITY, 0.0);
        }
        let cover = (aw as f64 / fw as f64).max(ah as f64 / fh as f64);
        let (rw, rh) = (((aw as f64 / cover).round() as usize).max(1), ((ah as f64 / cover).round() as usize).max(1));
        let small = resize(answer, aw, ah, rw, rh);
        let (placed, matched) = self.align(&small, rw, rh);
        (place(&small, rw, rh, fw, fh, placed), placed, matched)
    }

    /// Where the answer (scaled to about the frame's size, `rw` × `rh`) lies over the frame:
    /// the scale and shift (within [`ALIGN_SCALE`] and [`ALIGN_SHIFT`] px) at which it best
    /// matches the crop's own pixels away from the hole, both blurred so the model's redrawn
    /// texture counts less than where things are, and their tone taken out; and the correlation
    /// of the two there (1 when there is too little to judge).
    fn align(&self, small: &[f32], rw: usize, rh: usize) -> (Placement, f64) {
        let (cw, ch) = (self.rect.width() as usize, self.rect.height() as usize);
        let (fw, fh) = (self.frame.width() as usize, self.frame.height() as usize);
        let grey = |rgb: &[f32]| -> Vec<f32> { rgb.chunks(3).map(|p| p.iter().sum::<f32>() / 3.0).collect() };
        // Lightly blurred, less a wide blur: where things are, not the tone (the answer comes back
        // a little lighter or darker, which on a gradient would read as a shift).
        let detail = |g: Vec<f32>, w: usize, h: usize| -> Vec<f32> {
            let (fine, wide) = (blur(&g, w, h, 2), blur(&g, w, h, 10));
            fine.iter().zip(&wide).map(|(a, b)| a - b).collect()
        };
        let theirs = detail(grey(small), rw, rh);
        let ours = detail(grey(&self.rgb), cw, ch);
        let far = dilate(&self.hole, cw, ch, RING);
        let (ox, oy) = ((self.rect.x0 - self.frame.x0) as f64, (self.rect.y0 - self.frame.y0) as f64);
        // Every few pixels of the crop away from the hole: frame coordinates and value.
        let step = ((cw * ch) as f64 / 40_000.0).sqrt().ceil().max(1.0) as usize;
        let samples: Vec<(f64, f64, f32)> = (0..ch)
            .step_by(step)
            .flat_map(|y| (0..cw).step_by(step).map(move |x| (x, y)))
            .filter(|&(x, y)| !far.get(y * cw + x).copied().unwrap_or(true))
            .map(|(x, y)| (ox + x as f64, oy + y as f64, ours.get(y * cw + x).copied().unwrap_or(0.0)))
            .collect();
        if samples.len() < 64 {
            return (Placement::IDENTITY, 1.0);
        }
        let error = |p: Placement| -> f64 {
            samples
                .iter()
                .map(|&(x, y, v)| {
                    let (u, w) = p.map(x, y, fw, fh, rw, rh);
                    let d = f64::from(v - bilinear(&theirs, rw, rh, u, w));
                    d * d
                })
                .sum()
        };
        let mut best = (error(Placement::IDENTITY), Placement::IDENTITY);
        let search = |best: &mut (f64, Placement), scales: &[f64], shifts: &[f64], around: Placement| {
            for &ds in scales {
                for &dx in shifts {
                    for &dy in shifts {
                        let p = Placement { scale: around.scale + ds, dx: around.dx + dx, dy: around.dy + dy };
                        let e = error(p);
                        if e < best.0 {
                            *best = (e, p);
                        }
                    }
                }
            }
        };
        let coarse_scales: Vec<f64> = (-3..=3).map(|k| f64::from(k) * ALIGN_SCALE / 3.0).collect();
        let coarse_shifts: Vec<f64> = (-ALIGN_SHIFT..=ALIGN_SHIFT).map(f64::from).collect();
        search(&mut best, &coarse_scales, &coarse_shifts, Placement::IDENTITY);
        let fine_scales: Vec<f64> = (-4..=4).map(|k| f64::from(k) * ALIGN_SCALE / 12.0).collect();
        let fine_shifts: Vec<f64> = (-4..=4).map(|k| f64::from(k) * 0.25).collect();
        let around = best.1;
        search(&mut best, &fine_scales, &fine_shifts, around);
        let (mut so, mut st, mut c) = (0.0f64, 0.0f64, 0.0f64);
        for &(x, y, v) in &samples {
            let (u, w) = best.1.map(x, y, fw, fh, rw, rh);
            let (a, b) = (f64::from(v), f64::from(bilinear(&theirs, rw, rh, u, w)));
            so += a * a;
            st += b * b;
            c += a * b;
        }
        let matched = if so > 1e-9 && st > 1e-9 { c / (so * st).sqrt() } else { 1.0 };
        (best.1, matched)
    }
}

/// Where an answer lies over its frame: frame pixel `(x, y)` is answer pixel
/// `((x + ½ − fw/2) · scale + rw/2 + dx − ½, …)` of the answer scaled to cover the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub scale: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Placement {
    /// Centred, at the scale that covers the frame.
    pub const IDENTITY: Placement = Placement { scale: 1.0, dx: 0.0, dy: 0.0 };

    fn map(self, x: f64, y: f64, fw: usize, fh: usize, rw: usize, rh: usize) -> (f64, f64) {
        ((x + 0.5 - fw as f64 / 2.0) * self.scale + rw as f64 / 2.0 + self.dx - 0.5, (y + 0.5 - fh as f64 / 2.0) * self.scale + rh as f64 / 2.0 + self.dy - 0.5)
    }
}

/// How far the answer's scale is searched either side of covering the frame.
const ALIGN_SCALE: f64 = 0.015;
/// How far its shift is searched, in frame pixels.
const ALIGN_SHIFT: i32 = 6;

/// How the answer's values (`a`) are matched to the original's (`o`), both sampled outside the
/// hole: `(from, gain, to)`, the answer's value `v` becoming `to + (v − from) · gain`. Its black
/// and white points (1st and 99th percentiles) are mapped to the original's: on paper and ink they
/// hold even where the model redrew what is around the hole, which threw a match of the ring's
/// averages (a 245 grey paper came back from a marsh drawn around a title box, 2026-10-23). With
/// too little range to measure (a flat area), the medians are matched. The gain stays within
/// [`MIN_GAIN`]–[`MAX_GAIN`], the shift within [`MAX_SHIFT`].
pub(crate) fn levels(mut o: Vec<f64>, mut a: Vec<f64>) -> (f32, f32, f32) {
    if o.is_empty() || a.is_empty() {
        return (0.0, 1.0, 0.0);
    }
    o.sort_by(f64::total_cmp);
    a.sort_by(f64::total_cmp);
    let at = |v: &[f64], q: f64| v.get(((v.len() - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
    let (o1, o99, a1, a99) = (at(&o, 0.01), at(&o, 0.99), at(&a, 0.01), at(&a, 0.99));
    let shift = |from: f64, to: f64| (from, to.clamp(from - f64::from(MAX_SHIFT), from + f64::from(MAX_SHIFT)));
    let (from, gain, to) = if a99 - a1 > 0.1 && o99 - o1 > 0.1 {
        let gain = ((o99 - o1) / (a99 - a1)).clamp(MIN_GAIN, MAX_GAIN);
        // About the answer's middle, so the gain cap leaves both ends equally off.
        let (from, to) = shift((a1 + a99) / 2.0, (o1 + o99) / 2.0);
        (from, gain, to)
    } else {
        let (from, to) = shift(at(&a, 0.5), at(&o, 0.5));
        (from, 1.0, to)
    };
    (from as f32, gain as f32, to as f32)
}

/// One channel `v` (`w` × `h`) at `(u, w)`, bilinear, the edge repeated beyond it.
fn bilinear(v: &[f32], w: usize, h: usize, x: f64, y: f64) -> f32 {
    if w == 0 || h == 0 {
        return 0.0;
    }
    let (x, y) = (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64));
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let at = |x: usize, y: usize| v.get(y * w + x).copied().unwrap_or(0.0);
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Interleaved RGB `small` (`rw` × `rh`) placed over a `fw` × `fh` frame by `p`.
fn place(small: &[f32], rw: usize, rh: usize, fw: usize, fh: usize, p: Placement) -> Vec<f32> {
    let planes: Vec<Vec<f32>> = (0..3).map(|c| small.iter().skip(c).step_by(3).copied().collect()).collect();
    let mut out = Vec::with_capacity(fw * fh * 3);
    for y in 0..fh {
        for x in 0..fw {
            let (u, v) = p.map(x as f64, y as f64, fw, fh, rw, rh);
            out.extend(planes.iter().map(|plane| bilinear(plane, rw, rh, u, v)));
        }
    }
    out
}

/// A box blur of radius `r` (`w` × `h`, one channel), by prefix sums along each axis.
fn blur(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], n: usize, m: usize, at: &dyn Fn(usize, usize) -> usize| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        let mut prefix = vec![0.0f64; m + 1];
        for line in 0..n {
            for i in 0..m {
                prefix[i + 1] = prefix[i] + f64::from(src.get(at(line, i)).copied().unwrap_or(0.0));
            }
            for i in 0..m {
                let (a, b) = (i.saturating_sub(r), (i + r + 1).min(m));
                if let Some(o) = out.get_mut(at(line, i)) {
                    *o = ((prefix[b] - prefix[a]) / (b - a) as f64) as f32;
                }
            }
        }
        out
    };
    let rows = pass(v, h, w, &|y, x| y * w + x);
    pass(&rows, w, h, &|x, y| y * w + x)
}

/// The request's JSON body for images (PNG, base64) of ratio `ratio` sent `send` large.
pub(crate) fn body(pngs_base64: &[String], prompt: &str, ratio: &str, send: (u32, u32)) -> String {
    let mut input = vec![json!({"type": "text", "text": prompt})];
    input.extend(pngs_base64.iter().map(|data| json!({"type": "image", "mime_type": "image/png", "data": data})));
    json!({
        "model": MODEL,
        // Google keeps stored interactions for 55 days; a removal has nothing to come back to.
        "store": false,
        "input": input,
        // No `mime_type`: whatever Google sends (PNG or JPEG) decodes.
        "response_format": {"type": "image", "aspect_ratio": ratio, "image_size": image_size(send.0, send.1)},
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
pub fn fill(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, extra: &str, opts: Options, ctl: &Interrupt) -> Result<Vec<f32>, MlError> {
    let Some(key) = key() else {
        return Err(availability().err().unwrap_or(MlError::Unsupported));
    };
    fill_via(req, read, extra, opts, ctl, &mut |body, ctl| post(&key, body, ctl))
}

/// [`fill`], also writing the image sent to the file `sent` and Google's image to
/// `<sent>.answer`: for the evaluation example (`examples/nano_banana_eval.rs`).
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub fn eval_fill(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, extra: &str, ctl: &Interrupt, sent: &str) -> Result<Vec<Piece>, MlError> {
    use base64::Engine as _;
    let Some(key) = key() else {
        return Err(availability().err().unwrap_or(MlError::Unsupported));
    };
    // `PHOTOCRAFT_EVAL_MARK=mask` sends the picture and a mask instead of painting the hole.
    let mark = if std::env::var("PHOTOCRAFT_EVAL_MARK").is_ok_and(|m| m == "mask") { Mark::Mask } else { Mark::Paint };
    // `PHOTOCRAFT_EVAL_AREA=page` sends the whole page instead of the area near the hole.
    let area = if std::env::var("PHOTOCRAFT_EVAL_AREA").is_ok_and(|a| a == "page") { Area::Page } else { Area::Near };
    pieces_via(req, read, extra, Options { mark, area }, ctl, &mut |body, ctl| {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if let Some(Ok(png)) = v.pointer("/input/1/data").and_then(Value::as_str).map(|d| base64::engine::general_purpose::STANDARD.decode(d)) {
            let _ = std::fs::write(sent, png);
        }
        // `PHOTOCRAFT_EVAL_SIZE` (1K, 2K, 4K) overrides the output size asked for.
        let body = match std::env::var("PHOTOCRAFT_EVAL_SIZE") {
            Ok(size) => {
                let mut v = v.clone();
                v["response_format"]["image_size"] = Value::String(size);
                v.to_string()
            }
            Err(_) => body.to_string(),
        };
        // `PHOTOCRAFT_EVAL_ANSWER`: an earlier answer's image file instead of asking Google (free).
        let answer = match std::env::var("PHOTOCRAFT_EVAL_ANSWER") {
            Ok(path) => {
                let bytes = std::fs::read(&path).map_err(|e| MlError::BadInput(format!("{path}: {e}")))?;
                let data = base64::engine::general_purpose::STANDARD.encode(bytes);
                json!({"steps": [{"type": "model_output", "content": [{"type": "image", "data": data}]}]}).to_string()
            }
            Err(_) => post(&key, &body, ctl)?,
        };
        if let Ok(bytes) = answer_image(&answer) {
            let _ = std::fs::write(format!("{sent}.answer"), bytes);
        }
        Ok(answer)
    })
}

/// One area the model drew: the crop on the canvas, the model's picture of all of it (interleaved
/// sRGB, aligned and colour-matched to the page) and which of its pixels are the hole.
#[derive(Clone, Debug)]
pub struct Piece {
    pub rect: Rect,
    pub rgb: Vec<f32>,
    pub hole: Vec<bool>,
}

/// The model's whole picture of each area it drew for `req`, not only the hole (the Remove tool
/// keeps it on a layer masked to the hole, so more of it can be shown).
pub fn generate(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, extra: &str, opts: Options, ctl: &Interrupt) -> Result<Vec<Piece>, MlError> {
    let Some(key) = key() else {
        return Err(availability().err().unwrap_or(MlError::Unsupported));
    };
    pieces_via(req, read, extra, opts, ctl, &mut |body, ctl| post(&key, body, ctl))
}

/// A piece's pixels as the request's samples (`req.ch` per pixel: its colours, linear light in a
/// 32-bit document, opaque alpha).
pub fn samples(req: &Request, piece: &Piece) -> Vec<f32> {
    let n = piece.rect.width() as usize * piece.rect.height() as usize;
    let mut out = vec![0.0f32; n * req.ch];
    for k in 0..n {
        put(req, &mut out, k, [0, 1, 2].map(|c| piece.rgb.get(k * 3 + c).copied().unwrap_or(0.0)));
    }
    out
}

/// [`fill`] through `send`: the window with the hole taken from the pieces.
pub(crate) fn fill_via(
    req: &Request,
    read: &mut dyn FnMut(Rect) -> Vec<f32>,
    extra: &str,
    opts: Options,
    ctl: &Interrupt,
    send: &mut Transport<'_>,
) -> Result<Vec<f32>, MlError> {
    let pieces = pieces_via(req, read, extra, opts, ctl, send)?;
    let mut out = req.pixels.to_vec();
    for piece in &pieces {
        let cw = piece.rect.width() as usize;
        for (k, h) in piece.hole.iter().enumerate() {
            if !*h {
                continue;
            }
            let (x, y) = (piece.rect.x0 + (k % cw) as i32, piece.rect.y0 + (k / cw) as i32);
            let Some(j) = window_index(req.window, x, y) else { continue };
            put(req, &mut out, j, [0, 1, 2].map(|i| piece.rgb.get(k * 3 + i).copied().unwrap_or(0.0)));
        }
    }
    Ok(out)
}

/// The pieces of [`generate`] through `send`.
pub(crate) fn pieces_via(
    req: &Request,
    read: &mut dyn FnMut(Rect) -> Vec<f32>,
    extra: &str,
    opts: Options,
    ctl: &Interrupt,
    send: &mut Transport<'_>,
) -> Result<Vec<Piece>, MlError> {
    let (w, h) = (req.window.width() as usize, req.window.height() as usize);
    if req.hole.len() != w * h || req.pixels.len() != w * h * req.ch {
        return Err(MlError::BadInput("the hole or pixels don't match the window".into()));
    }
    let parts = merged(groups(req.window, req.hole), req.canvas, opts.area);
    let mark = opts.mark;
    let mut pieces = Vec::with_capacity(parts.len());
    let count = parts.len().max(1) as f32;
    for (g, part) in parts.iter().enumerate() {
        ctl.check()?;
        let c = crop(req, *part, opts.area, read)?;
        let (sw, sh) = c.send;
        let png = |rgb: &[f32]| -> Result<String, MlError> {
            use base64::Engine as _;
            let img = photocraft_codecs::Image::from_normalized(sw, sh, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, rgb)
                .map_err(|e| MlError::BadInput(format!("couldn't make the image to send ({e})")))?;
            let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default())
                .map_err(|e| MlError::BadInput(format!("couldn't make the image to send ({e})")))?;
            Ok(base64::engine::general_purpose::STANDARD.encode(&png))
        };
        let marker = MARKERS.get(c.marker).map_or("solid pure red (#FF0000)", |m| m.1);
        let (images, text, shown) = match mark {
            Mark::Paint => (vec![png(&c.image())?], prompt(extra, marker), marker),
            Mark::Mask => (vec![png(&c.picture())?, png(&c.mask())?], mask_prompt(extra), "a mask"),
        };
        let progress = move |f: f32| ((g as f32 + f) / count).clamp(0.0, 1.0);
        let report = |f: f32| ctl.progress(progress(f));
        let cancelled = || ctl.cancelled();
        let span = Interrupt::new(&cancelled, &report);
        let started = std::time::Instant::now();
        let answered = send(&body(&images, &text, c.ratio, c.send), &span).and_then(|answer| decode_answer(&answer_image(&answer)?)).inspect_err(|e| {
            if *e != MlError::Cancelled {
                log::warn!("[ml] {NAME}: {e}")
            }
        });
        let (rgb, aw, ah) = answered?;
        let elapsed = started.elapsed().as_secs_f32();
        ctl.check()?;
        let (fitted, placed, matched) = c.fitted_placed(&rgb, aw, ah);
        log::info!(
            "[ml] {NAME}: sent {sw} x {sh} ({}, {}, {shown}), answered {aw} x {ah} in {elapsed:.1} s, placed at scale {:.4}, shift {:.2}, {:.2}, match {matched:.2}",
            c.ratio,
            image_size(sw, sh),
            placed.scale,
            placed.dx,
            placed.dy
        );
        if matched < MIN_MATCH {
            return Err(MlError::Inference(format!(
                "Google's picture doesn't line up with the page (it changed the framing; {:.0} % match), so its fill wouldn't fit: try again (each try is billed)",
                matched.max(0.0) * 100.0
            )));
        }
        pieces.push(Piece { rect: c.rect, rgb: fitted, hole: c.hole });
        ctl.progress(progress(1.0));
    }
    Ok(pieces)
}

/// The parts of the hole sent one request each: parts whose crops overlap become one (else the
/// later answer would cover the earlier one's surroundings, drawn and paid twice); all of them
/// when the whole page goes (each would send the same page) or when there are more than
/// [`MAX_REQUESTS`].
pub(crate) fn merged(mut parts: Vec<Rect>, canvas: Rect, area: Area) -> Vec<Rect> {
    let whole = area == Area::Page && canvas.width().max(canvas.height()) <= MAX_SEND;
    loop {
        if (whole || parts.len() > MAX_REQUESTS)
            && let Some(all) = parts.iter().copied().reduce(|a, b| a.union(&b))
        {
            return vec![all];
        }
        let overlap = parts.iter().enumerate().find_map(|(i, a)| {
            let ca = context(*a, canvas, area);
            parts.iter().enumerate().skip(i + 1).find(|(_, b)| !ca.intersect(&context(**b, canvas, area)).is_empty()).map(|(j, _)| (i, j))
        });
        let Some((i, j)) = overlap else { return parts };
        let b = parts.remove(j);
        if let Some(a) = parts.get_mut(i) {
            *a = a.union(&b);
        }
    }
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

/// Ask Google about the model with the key in use (free: nothing is generated): its answer's
/// status and the start of its body, or why it couldn't be asked. For checking a setup.
#[cfg(not(target_arch = "wasm32"))]
pub fn check() -> Result<String, MlError> {
    let key = key().ok_or_else(|| availability().err().unwrap_or(MlError::Unsupported))?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into();
    let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{MODEL}");
    let mut answer = agent.get(&url).header("x-goog-api-key", &key).call().map_err(|e| MlError::Inference(format!("couldn't reach Google ({e})")))?;
    let status = answer.status().as_u16();
    let text = answer.body_mut().with_config().limit(1 << 20).read_to_string().unwrap_or_default();
    Ok(format!("{status}: {}", clip(text.trim(), 600)))
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
