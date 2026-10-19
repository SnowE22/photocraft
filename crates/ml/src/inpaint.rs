//! A document region with a hole, to a fixed-size inpainting model and back.
//!
//! The hole is split into groups of nearby pixels (its connected parts, merged when they come
//! within [`GAP`] pixels of each other), and each group gets its own square crop around it. A
//! group up to the model's side less [`FULL_RES_MARGIN`] on each side (384 pixels for LaMa) is
//! filled at full resolution; a larger one gets [`SCALED_CONTEXT`] of its size as context and is
//! scaled down to the model (area averaging), filled, scaled back (bicubic), then filled again at
//! full resolution in tiles, ring by ring from the hole's edge inwards ([`fill_with`]), so it
//! keeps the sharpness of its surroundings (user report: large removals came out blurry). Only
//! the hole's pixels change, and groups are filled one after another, each seeing the fills
//! before it.
//!
//! Each fill is corrected before it is used (`correct`): the model's drift in tone, measured on
//! the known pixels at the hole's edge, is taken out smoothly across the fill, and a crop whose
//! surroundings are neutral (a black-and-white manga page, even in an RGB document) gets a
//! neutral fill.
//!
//! The model sees colour only, as sRGB-encoded values from 0 to 1 (32-bit documents hold linear
//! light and are encoded on the way in and decoded on the way out). It needs opaque surroundings:
//! a crop with transparent pixels around the hole is [`MlError::Unsuitable`], and the caller uses
//! another fill.

use photocraft_geom::Rect;
use photocraft_raster::Interrupt;

use crate::MlError;

/// The model's input side (LaMa's export is fixed at 512 × 512).
pub const SIDE: usize = 512;

/// Parts of the hole closer than this (pixels) are filled together.
pub const GAP: i32 = 32;

/// Context around a group a crop gives, as a fraction of its larger side, before it would have
/// to be shrunk (a crop is never smaller than the model).
pub const CONTEXT: f64 = 0.3;

/// Rows read at a time, so a large crop is never held at full resolution.
const BAND: i32 = 64;

/// More separate parts than this are filled as one group.
const MAX_GROUPS: usize = 64;

/// Alpha at or above this counts as opaque.
const OPAQUE: f32 = 0.999;

/// How the region's channels give the model its RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colours {
    /// Red, green and blue are channels 0, 1 and 2.
    Rgb,
    /// One grey channel (0), given to the model as R = G = B and taken back as their mean.
    Gray,
}

impl Colours {
    fn channels(self) -> usize {
        match self {
            Colours::Rgb => 3,
            Colours::Gray => 1,
        }
    }
}

/// The inpainting network as [`fill`] calls it: planar RGB (`SIDE × SIDE`, 0–1, zero in the hole)
/// and its mask (1 = fill) in, planar RGB (0–1) out.
///
/// The third argument is a hint for the masked pixels (planar RGB on the same grid), when the fill
/// redoes an earlier one: a model that can start from it (diffusion) keeps its shapes; one that
/// can't (LaMa) ignores it.
pub type Model<'a> = dyn FnMut(&[f32], &[f32], Option<&[f32]>) -> Result<Vec<f32>, MlError> + 'a;

/// What [`fill`] works on.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    /// Where the source can be read.
    pub canvas: Rect,
    /// The region the hole and the result cover (inside the canvas).
    pub window: Rect,
    /// One per window pixel: true where it is to be filled.
    pub hole: &'a [bool],
    /// The window's pixels, `ch` samples each; the result starts as a copy.
    pub pixels: &'a [f32],
    /// Samples per pixel.
    pub ch: usize,
    pub colours: Colours,
    /// The alpha sample, if there is one.
    pub alpha: Option<usize>,
    /// Samples are linear light (32-bit documents).
    pub linear: bool,
    /// Transparent pixels around the hole are unknown to the model (it fills them, they stay as
    /// they are), instead of making the request [`MlError::Unsuitable`]: outpainting, where the
    /// canvas around a picture is empty.
    pub transparent_unknown: bool,
}

/// The groups of hole pixels filled together, as bounding boxes in canvas coordinates.
pub fn groups(window: Rect, hole: &[bool]) -> Vec<Rect> {
    let (w, h) = (window.width() as usize, window.height() as usize);
    if w.checked_mul(h) != Some(hole.len()) {
        return Vec::new();
    }
    let mut seen = vec![false; hole.len()];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    for start in 0..hole.len() {
        if !hole.get(start).copied().unwrap_or(false) || seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            for (dx, dy) in [(-1isize, -1isize), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (Some(nx), Some(ny)) = (x.checked_add_signed(dx), y.checked_add_signed(dy)) else { continue };
                if nx >= w || ny >= h {
                    continue;
                }
                let j = ny * w + nx;
                if hole.get(j).copied().unwrap_or(false) && seen.get(j).is_some_and(|s| !*s) {
                    if let Some(s) = seen.get_mut(j) {
                        *s = true;
                    }
                    stack.push(j);
                }
            }
        }
        let at = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
        boxes.push(Rect::new(
            window.x0.saturating_add(at(x0)),
            window.y0.saturating_add(at(y0)),
            window.x0.saturating_add(at(x1)),
            window.y0.saturating_add(at(y1)),
        ));
    }
    merge(boxes)
}

/// Boxes that come within [`GAP`] of each other, as one; too many become one box.
fn merge(mut boxes: Vec<Rect>) -> Vec<Rect> {
    if boxes.len() > MAX_GROUPS {
        return boxes.into_iter().reduce(|a, b| a.union(&b)).into_iter().collect();
    }
    loop {
        let mut pair = None;
        'find: for (i, a) in boxes.iter().enumerate() {
            for (j, b) in boxes.iter().enumerate().skip(i + 1) {
                if !a.inflate(GAP).intersect(b).is_empty() {
                    pair = Some((i, j));
                    break 'find;
                }
            }
        }
        let Some((i, j)) = pair else { return boxes };
        let b = boxes.swap_remove(j);
        if let Some(a) = boxes.get_mut(i) {
            *a = a.union(&b);
        }
    }
}

/// Context around a group whose crop has to be shrunk, as a fraction of its larger side: less
/// than [`CONTEXT`], so a big hole is shrunk less.
pub const SCALED_CONTEXT: f64 = 0.15;

/// Least context around a group, in pixels: a hole up to `side - 2 × FULL_RES_MARGIN` pixels
/// across is filled 1:1.
pub const FULL_RES_MARGIN: u32 = 64;

/// How [`fill_with`] uses its model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// The model's square input side, in pixels.
    pub side: usize,
    /// When a group's crop had to be shrunk, redo its fill at full resolution in tiles, from the
    /// hole's edge inwards ([`fill_with`]).
    pub refine: bool,
    /// Most model runs one fill may take; the refinement stops there (the shrunk fill stays).
    pub max_runs: usize,
    /// Correct the model's drift in tone at the hole's edge and keep neutral areas neutral.
    pub correct: bool,
    /// Refine in rings from the edge inwards (a model that can't use a hint); else all at once,
    /// each tile hinted with the shrunk fill.
    pub rings: bool,
}

/// LaMa's: 512 square, refined, corrected.
pub const LAMA: Options = Options { side: SIDE, refine: true, max_runs: 48, correct: true, rings: true };

/// LaMa's Quality mode (the Remove tool's Quality: High): the same model at 1024 square, so a
/// hole up to 896 pixels across is filled 1:1 instead of shrunk to 512, and a larger one is shrunk
/// half as much; refined and corrected like [`LAMA`]. About four times the work per run, but far
/// fewer runs for a large hole. Measured on a public-domain photo (RTX 5070 Laptop, DirectML): a
/// 700 × 420 hole kept 61 % of its surroundings' sharpness instead of 43 %, in 1.3 s instead of
/// 1.8 s. Averaging each run with a run on the mirrored crop was tried and blurred texture.
pub const LAMA_QUALITY: Options = Options { side: 1024, refine: true, max_runs: 24, correct: true, rings: true };

/// Stable Diffusion's: 512 square, refined in one hinted pass, corrected.
pub const DIFFUSION: Options = Options { side: 512, refine: true, max_runs: 24, correct: true, rings: false };

/// The square crop the model sees for `group`: its extent plus context, at least [`SIDE`],
/// centred on it, moved inside the canvas and cut to it. A group up to `SIDE - 2 ×
/// FULL_RES_MARGIN` across gets a full-resolution crop; a larger one [`SCALED_CONTEXT`] of
/// context (at least [`FULL_RES_MARGIN`]), shrunk to the model.
pub fn crop(group: Rect, canvas: Rect) -> Rect {
    crop_for(group, canvas, SIDE)
}

/// [`crop`] for a model of `side` pixels.
pub fn crop_for(group: Rect, canvas: Rect, side: usize) -> Rect {
    let e = group.width().max(group.height());
    let model = u32::try_from(side).unwrap_or(u32::MAX);
    let full = e.saturating_add(FULL_RES_MARGIN.saturating_mul(2)) <= model;
    let side = if full {
        i64::from(model)
    } else {
        let margin = ((f64::from(e) * SCALED_CONTEXT).ceil() as u32).max(FULL_RES_MARGIN);
        i64::from(e.saturating_add(margin.saturating_mul(2)).max(model))
    };
    let place = |lo: i32, hi: i32, c0: i32, c1: i32| -> (i32, i32) {
        let (c0, c1) = (i64::from(c0), i64::from(c1));
        let mut a = (i64::from(lo) + i64::from(hi)) / 2 - side / 2;
        if a + side > c1 {
            a = c1 - side;
        }
        a = a.max(c0);
        let b = (a + side).min(c1);
        (i32::try_from(a).unwrap_or(i32::MIN), i32::try_from(b).unwrap_or(i32::MAX))
    };
    let (x0, x1) = place(group.x0, group.x1, canvas.x0, canvas.x1);
    let (y0, y1) = place(group.y0, group.y1, canvas.y0, canvas.y1);
    Rect::new(x0, y0, x1, y1)
}

/// How `len` source samples map onto `side` targets: for each source sample, the targets it
/// feeds and with what weight (each target's weights sum to 1). Area averaging when shrinking,
/// bilinear when enlarging, pixel centres aligned.
fn weights(len: usize, side: usize) -> Vec<Vec<(usize, f32)>> {
    let mut by_source = vec![Vec::new(); len];
    if len == 0 || side == 0 {
        return by_source;
    }
    let scale = len as f64 / side as f64;
    for t in 0..side {
        if scale >= 1.0 {
            let (a, b) = (t as f64 * scale, (t + 1) as f64 * scale);
            let mut s = a.floor() as usize;
            while (s as f64) < b && s < len {
                let overlap = (b.min(s as f64 + 1.0) - a.max(s as f64)).max(0.0);
                if overlap > 0.0
                    && let Some(v) = by_source.get_mut(s)
                {
                    v.push((t, (overlap / scale) as f32));
                }
                s += 1;
            }
        } else {
            let u = ((t as f64 + 0.5) * scale - 0.5).clamp(0.0, (len - 1) as f64);
            let s0 = u.floor() as usize;
            let f = u - s0 as f64;
            let s1 = (s0 + 1).min(len - 1);
            if let Some(v) = by_source.get_mut(s0) {
                v.push((t, (1.0 - f) as f32));
            }
            if f > 0.0
                && let Some(v) = by_source.get_mut(s1)
            {
                v.push((t, f as f32));
            }
        }
    }
    by_source
}

/// The sRGB curve, for the linear light of 32-bit documents.
fn encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn decode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Model-space input for one crop: planar RGB (0–1, zero in the hole) and the mask (1 = fill).
pub(crate) struct Input {
    pub(crate) side: usize,
    pub(crate) rgb: Vec<f32>,
    pub(crate) mask: Vec<f32>,
    /// The masked pixels' values so far (planar RGB), when some are redone ([`Px::Redo`]).
    pub(crate) hint: Option<Vec<f32>>,
}

/// One window pixel's index, if `(x, y)` is in the window.
fn window_index(window: Rect, x: i32, y: i32) -> Option<usize> {
    if !window.contains(x, y) {
        return None;
    }
    let w = window.width() as usize;
    Some((y - window.y0) as usize * w + (x - window.x0) as usize)
}

/// Where a crop pixel's value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Px {
    /// The source as read.
    Source,
    /// The result so far (filled earlier).
    Out,
    /// Unknown: the model fills it.
    Masked,
    /// Filled earlier and filled again: masked, its value so far the model's hint.
    Redo,
}

/// Read `crop` in bands and shrink or enlarge it to the model's `side`. `state(k)` says, for
/// window pixel `k`, where its value comes from (pixels outside the window are the source's).
fn sample(
    req: &Request,
    crop: Rect,
    side: usize,
    out: &[f32],
    state: &dyn Fn(usize) -> Px,
    read: &mut dyn FnMut(Rect) -> Vec<f32>,
    ctl: &Interrupt,
) -> Result<Input, MlError> {
    let (cw, chh) = (crop.width() as usize, crop.height() as usize);
    let ch = req.ch;
    if cw == 0 || chh == 0 || ch == 0 || side == 0 {
        return Err(MlError::BadInput("nothing to read around the hole".into()));
    }
    let (wx, wy) = (weights(cw, side), weights(chh, side));
    let n = side * side;
    // Planes: r, g, b, hole, then the hint's r, g, b.
    let mut acc = vec![0f32; 7 * n];
    let mut row = vec![[0f32; 7]; cw];
    let mut hrow = vec![[0f32; 7]; side];
    let mut redone = false;
    let mut y = crop.y0;
    while y < crop.y1 {
        ctl.check()?;
        let band = Rect::new(crop.x0, y, crop.x1, y.saturating_add(BAND).min(crop.y1));
        let data = read(band);
        if Some(data.len()) != (band.width() as usize).checked_mul(band.height() as usize).and_then(|p| p.checked_mul(ch)) {
            return Err(MlError::BadInput("the source returned the wrong number of samples".into()));
        }
        for (r, line) in data.chunks_exact(cw * ch).enumerate() {
            let yy = band.y0 + r as i32;
            for (i, (px, slot)) in line.chunks_exact(ch).zip(row.iter_mut()).enumerate() {
                let xx = crop.x0 + i as i32;
                let wi = window_index(req.window, xx, yy);
                let mut st = wi.map_or(Px::Source, state);
                let clear = req.alpha.is_some_and(|a| {
                    let v = match wi {
                        Some(k) if matches!(st, Px::Out | Px::Redo) => out.get(k * ch + a).copied(),
                        _ => px.get(a).copied(),
                    };
                    v.is_some_and(|v| v < OPAQUE)
                });
                if clear && req.transparent_unknown && matches!(st, Px::Source | Px::Out) {
                    st = Px::Masked;
                }
                let hole = matches!(st, Px::Masked | Px::Redo);
                redone |= st == Px::Redo;
                let px = match wi {
                    Some(k) if matches!(st, Px::Out | Px::Redo) => out.get(k * ch..(k + 1) * ch).unwrap_or(px),
                    _ => px,
                };
                if !hole && req.alpha.is_some_and(|a| px.get(a).is_some_and(|v| *v < OPAQUE)) {
                    return Err(MlError::Unsuitable("the area around the stroke has transparent pixels".into()));
                }
                let c = |k: usize| {
                    let v = px.get(k).copied().unwrap_or(0.0);
                    if req.linear { encode(v) } else { v.clamp(0.0, 1.0) }
                };
                let rgb = match req.colours {
                    Colours::Rgb => [c(0), c(1), c(2)],
                    Colours::Gray => [c(0), c(0), c(0)],
                };
                *slot = match st {
                    Px::Masked => [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
                    Px::Redo => [0.0, 0.0, 0.0, 1.0, rgb[0], rgb[1], rgb[2]],
                    Px::Source | Px::Out => [rgb[0], rgb[1], rgb[2], 0.0, rgb[0], rgb[1], rgb[2]],
                };
            }
            // Horizontal, then into the target rows this source row feeds.
            for v in hrow.iter_mut() {
                *v = [0.0; 7];
            }
            for (s, taps) in wx.iter().enumerate() {
                let Some(src) = row.get(s) else { continue };
                for &(t, w) in taps {
                    if let Some(dst) = hrow.get_mut(t) {
                        for (d, s) in dst.iter_mut().zip(src) {
                            *d += w * s;
                        }
                    }
                }
            }
            let Some(taps) = wy.get((yy - crop.y0) as usize) else { continue };
            for &(t, w) in taps {
                for (x, v) in hrow.iter().enumerate() {
                    for (p, s) in v.iter().enumerate() {
                        if let Some(a) = acc.get_mut(p * n + t * side + x) {
                            *a += w * s;
                        }
                    }
                }
            }
        }
        y = band.y1;
    }
    let (rgb, rest) = acc.split_at(3 * n);
    let (holes, hint) = rest.split_at(n);
    // A cell any hole pixel reaches is filled; its colour is zeroed as the model expects.
    let mask: Vec<f32> = holes.iter().map(|m| if *m > 1e-6 { 1.0 } else { 0.0 }).collect();
    let hint = redone.then(|| hint.to_vec());
    let mut rgb = rgb.to_vec();
    for (p, m) in mask.iter().enumerate() {
        if *m > 0.0 {
            for c in 0..3 {
                if let Some(v) = rgb.get_mut(c * n + p) {
                    *v = 0.0;
                }
            }
        }
    }
    Ok(Input { side, rgb, mask, hint })
}

/// A planar `side × side` channel at model coordinates `(u, v)`: Catmull-Rom bicubic, kept
/// within the four nearest samples so edges don't ring (a line on a manga page would get a halo),
/// or bilinear where `sharp` is off.
fn resample(plane: &[f32], side: usize, u: f64, v: f64, sharp: bool) -> f32 {
    if side == 0 {
        return 0.0;
    }
    let max = (side - 1) as f64;
    let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
    let (x0, y0) = (u.floor() as isize, v.floor() as isize);
    let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
    let last = side as isize - 1;
    let at = |x: isize, y: isize| plane.get(y.clamp(0, last) as usize * side + x.clamp(0, last) as usize).copied().unwrap_or(0.0);
    let near = [at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1)];
    if !sharp {
        let top = near[0] * (1.0 - fx) + near[1] * fx;
        let bottom = near[2] * (1.0 - fx) + near[3] * fx;
        return top * (1.0 - fy) + bottom * fy;
    }
    let cr = |t: f32| {
        let (t2, t3) = (t * t, t * t * t);
        [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
    };
    let (wx, wy) = (cr(fx), cr(fy));
    let mut sum = 0.0;
    for (j, wyj) in wy.iter().enumerate() {
        let mut r = 0.0;
        for (i, wxi) in wx.iter().enumerate() {
            r += wxi * at(x0 + i as isize - 1, y0 + j as isize - 1);
        }
        sum += wyj * r;
    }
    let (lo, hi) = near.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    sum.clamp(lo, hi)
}

/// Known pixels within this many model pixels of the hole measure the model's drift in tone.
const RING: usize = 6;
/// The drift is averaged over this radius along the ring, so the fill gets the ring's tone, not
/// its texture.
const DRIFT_BLUR: usize = 4;
/// Known pixels at most this far from grey (0–1) count as neutral.
const NEUTRAL: f32 = 2.5 / 255.0;
/// A crop whose known pixels are at least this neutral gets a neutral fill.
const NEUTRAL_SHARE: f64 = 0.995;

/// Make the model's fill `pred` meet what is around it (planar RGB on the input's grid): its drift
/// from the known pixels near the hole (the model doesn't keep tones exactly), smoothed along the
/// edge and spread over the hole; and on a neutral (black-and-white) crop a neutral fill, where
/// the model may tint it.
pub(crate) fn correct(input: &Input, pred: &mut [f32]) {
    let side = input.side;
    let n = side * side;
    if pred.len() != 3 * n || input.rgb.len() != 3 * n || input.mask.len() != n || n == 0 {
        return;
    }
    let masked = |p: usize| input.mask.get(p).is_some_and(|m| *m > 0.0);
    // The ring: known pixels near the hole.
    let hole: Vec<f32> = (0..n).map(|p| if masked(p) { 1.0 } else { 0.0 }).collect();
    let near = box_blur(&hole, side, side, RING);
    let ring: Vec<f32> = (0..n).map(|p| if !masked(p) && near.get(p).is_some_and(|v| *v > 0.0) { 1.0 } else { 0.0 }).collect();
    if ring.iter().any(|w| *w > 0.0) {
        let wsum = box_blur(&ring, side, side, DRIFT_BLUR);
        let mut drift = Vec::with_capacity(3);
        for c in 0..3 {
            let d: Vec<f32> = (0..n)
                .map(|p| {
                    let r = ring.get(p).copied().unwrap_or(0.0);
                    r * (input.rgb.get(c * n + p).copied().unwrap_or(0.0) - pred.get(c * n + p).copied().unwrap_or(0.0))
                })
                .collect();
            let blurred = box_blur(&d, side, side, DRIFT_BLUR);
            drift.push(blurred.iter().zip(&wsum).map(|(d, w)| if *w > 1e-6 { d / w } else { 0.0 }).collect::<Vec<f32>>());
        }
        let spread = push_pull(&drift, &ring, side, side);
        for p in (0..n).filter(|p| masked(*p)) {
            for (c, plane) in spread.iter().enumerate() {
                if let (Some(v), Some(d)) = (pred.get_mut(c * n + p), plane.get(p)) {
                    *v = (*v + d).clamp(0.0, 1.0);
                }
            }
        }
    }
    // Neutral surroundings: a neutral fill.
    let (mut known, mut grey) = (0u64, 0u64);
    let g =
        |p: usize| [input.rgb.get(p).copied().unwrap_or(0.0), input.rgb.get(n + p).copied().unwrap_or(0.0), input.rgb.get(2 * n + p).copied().unwrap_or(0.0)];
    for p in (0..n).filter(|p| !masked(*p)) {
        let px = g(p);
        known += 1;
        if (px[0] - px[1]).abs() <= NEUTRAL && (px[1] - px[2]).abs() <= NEUTRAL && (px[0] - px[2]).abs() <= NEUTRAL {
            grey += 1;
        }
    }
    if known > 0 && grey as f64 >= known as f64 * NEUTRAL_SHARE {
        for p in (0..n).filter(|p| masked(*p)) {
            let m = (pred.get(p).copied().unwrap_or(0.0) + pred.get(n + p).copied().unwrap_or(0.0) + pred.get(2 * n + p).copied().unwrap_or(0.0)) / 3.0;
            for c in 0..3 {
                if let Some(v) = pred.get_mut(c * n + p) {
                    *v = m;
                }
            }
        }
    }
}

/// Mean over a `(2r + 1)²` box of a `w × h` plane (the box counts the part inside as it is and
/// the rest as zero).
fn box_blur(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], along_x: bool| -> Vec<f32> {
        let mut out = vec![0f32; src.len()];
        let (len, lines) = if along_x { (w, h) } else { (h, w) };
        let idx = |line: usize, i: usize| if along_x { line * w + i } else { i * w + line };
        let mut pre = vec![0f64; len + 1];
        for line in 0..lines {
            for i in 0..len {
                pre[i + 1] = pre[i] + f64::from(src.get(idx(line, i)).copied().unwrap_or(0.0));
            }
            for i in 0..len {
                let (a, b) = (i.saturating_sub(r), (i + r + 1).min(len));
                if let Some(o) = out.get_mut(idx(line, i)) {
                    *o = ((pre[b] - pre[a]) / (2 * r + 1) as f64) as f32;
                }
            }
        }
        out
    };
    let x = pass(v, true);
    pass(&x, false)
}

/// Spread the values of `planes` from where `weight` is positive over the rest of the `w × h`
/// grid, smoothly (push-pull: averaged down a pyramid, then each level fills what it lacks from
/// the coarser one, bilinearly).
pub(crate) fn push_pull(planes: &[Vec<f32>], weight: &[f32], w: usize, h: usize) -> Vec<Vec<f32>> {
    let k = planes.len();
    type Level = (usize, usize, Vec<Vec<f32>>, Vec<f32>);
    let sums: Vec<Vec<f32>> = planes.iter().map(|p| p.iter().zip(weight).map(|(v, w)| v * w).collect()).collect();
    let mut levels: Vec<Level> = vec![(w, h, sums, weight.to_vec())];
    while let Some((lw, lh, ls, lwt)) = levels.last() {
        if (*lw <= 1 && *lh <= 1) || levels.len() > 40 {
            break;
        }
        let (lw, lh) = (*lw, *lh);
        let (nw, nh) = (lw.div_ceil(2), lh.div_ceil(2));
        let mut ns = vec![vec![0f32; nw * nh]; k];
        let mut nwt = vec![0f32; nw * nh];
        for y in 0..lh {
            for x in 0..lw {
                let (i, j) = (y * lw + x, (y / 2) * nw + x / 2);
                if let (Some(d), Some(s)) = (nwt.get_mut(j), lwt.get(i)) {
                    *d += s;
                }
                for c in 0..k {
                    if let (Some(d), Some(s)) = (ns.get_mut(c).and_then(|p| p.get_mut(j)), ls.get(c).and_then(|p| p.get(i))) {
                        *d += s;
                    }
                }
            }
        }
        levels.push((nw, nh, ns, nwt));
    }
    // Normalised values per level, from the top down.
    let mut coarse: Option<(usize, usize, Vec<Vec<f32>>)> = None;
    for (lw, lh, ls, lwt) in levels.into_iter().rev() {
        let mut vals = vec![vec![0f32; lw * lh]; k];
        for y in 0..lh {
            for x in 0..lw {
                let i = y * lw + x;
                let wt = lwt.get(i).copied().unwrap_or(0.0);
                let a = wt.clamp(0.0, 1.0);
                for c in 0..k {
                    let own = if wt > 1e-6 { ls.get(c).and_then(|p| p.get(i)).copied().unwrap_or(0.0) / wt } else { 0.0 };
                    let up = match &coarse {
                        Some((cw, chh, cv)) => {
                            // This pixel's centre on the coarser grid.
                            let u = ((x as f64 + 0.5) / 2.0 - 0.5).clamp(0.0, (*cw - 1) as f64);
                            let v = ((y as f64 + 0.5) / 2.0 - 0.5).clamp(0.0, (*chh - 1) as f64);
                            let (x0, y0) = (u.floor() as usize, v.floor() as usize);
                            let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(chh - 1));
                            let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
                            let p = cv.get(c);
                            let g = |xx: usize, yy: usize| p.and_then(|p| p.get(yy * cw + xx)).copied().unwrap_or(0.0);
                            (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy) + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
                        }
                        None => own,
                    };
                    if let Some(v) = vals.get_mut(c).and_then(|p| p.get_mut(i)) {
                        *v = own * a + up * (1.0 - a);
                    }
                }
            }
        }
        coarse = Some((lw, lh, vals));
    }
    coarse.map(|(_, _, v)| v).unwrap_or_default()
}

/// Write model colour `rgb` (sRGB-encoded 0–1) into window pixel `k` of `out`.
fn put(req: &Request, out: &mut [f32], k: usize, rgb: [f32; 3]) {
    let back = |v: f32| if req.linear { decode(v) } else { v.clamp(0.0, 1.0) };
    let Some(px) = out.get_mut(k * req.ch..(k + 1) * req.ch) else { return };
    match req.colours {
        Colours::Rgb => {
            for (d, s) in px.iter_mut().zip(rgb) {
                *d = back(s);
            }
        }
        Colours::Gray => {
            if let Some(d) = px.first_mut() {
                *d = back((rgb[0] + rgb[1] + rgb[2]) / 3.0);
            }
        }
    }
    if let Some(a) = req.alpha.and_then(|a| px.get_mut(a)) {
        *a = 1.0;
    }
}

/// Run the model on `input`, check its answer and correct it: the fill in the mask, the input
/// elsewhere (planar, on the input's grid).
fn run_model(model: &mut Model<'_>, input: &Input, opts: &Options) -> Result<Vec<f32>, MlError> {
    let n = input.side * input.side;
    let mut pred = model(&input.rgb, &input.mask, input.hint.as_deref())?;
    if pred.len() != 3 * n {
        return Err(MlError::Inference(format!("the model gave {} values, not {}", pred.len(), 3 * n)));
    }
    for v in pred.iter_mut() {
        *v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    }
    if opts.correct {
        correct(input, &mut pred);
    }
    let mut merged = input.rgb.clone();
    for (p, m) in input.mask.iter().enumerate() {
        if *m > 0.0 {
            for c in 0..3 {
                if let (Some(d), Some(s)) = (merged.get_mut(c * n + p), pred.get(c * n + p)) {
                    *d = *s;
                }
            }
        }
    }
    Ok(merged)
}

/// Fill the request's hole with LaMa's [`Options`]: see [`fill_with`].
pub fn fill(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, model: &mut Model<'_>, ctl: &Interrupt) -> Result<Vec<f32>, MlError> {
    fill_with(req, read, model, ctl, &LAMA)
}

/// Fill the request's hole: `read(rect)` gives the source's pixels over `rect` (inside the
/// canvas, `ch` samples each); `model(rgb, mask)` runs the model on planar RGB (`side × side`,
/// 0–1) and its mask (1 = fill) and gives back planar RGB (0–1). Returns the window's pixels with
/// the hole filled.
///
/// A group whose crop is larger than the model is filled shrunk, then (with `refine`) filled
/// again at full resolution: in rings [`REFINE_BAND`] pixels deep from the hole's edge inwards,
/// each ring in model-sized tiles that see the real pixels outside, the rings already redone and
/// the shrunk fill further in, so detail continues sharply into the hole.
pub fn fill_with(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, model: &mut Model<'_>, ctl: &Interrupt, opts: &Options) -> Result<Vec<f32>, MlError> {
    let (w, h) = (req.window.width() as usize, req.window.height() as usize);
    let pixels = w.checked_mul(h).ok_or_else(|| MlError::BadInput("the region is too large".into()))?;
    if req.window.is_empty() || !req.canvas.contains_rect(&req.window) {
        return Err(MlError::BadInput("the region isn't on the canvas".into()));
    }
    if req.hole.len() != pixels || Some(req.pixels.len()) != pixels.checked_mul(req.ch) {
        return Err(MlError::BadInput("the hole and the pixels don't match the region".into()));
    }
    if req.ch < req.colours.channels() || req.alpha.is_some_and(|a| a >= req.ch || a < req.colours.channels()) {
        return Err(MlError::BadInput("the pixels don't have the colour channels the model needs".into()));
    }
    let side = opts.side.max(8);
    let n = side * side;
    let mut out = req.pixels.to_vec();
    let mut filled = vec![false; pixels];
    let groups = groups(req.window, req.hole);
    let mut runs = 0usize;
    for (gi, group) in groups.iter().enumerate() {
        ctl.check()?;
        let crop = crop_for(*group, req.canvas, side);
        let state = |k: usize| {
            if filled.get(k).copied().unwrap_or(false) {
                Px::Out
            } else if req.hole.get(k).copied().unwrap_or(false) {
                Px::Masked
            } else {
                Px::Source
            }
        };
        let input = sample(req, crop, side, &out, &state, read, ctl)?;
        let merged = run_model(model, &input, opts)?;
        runs += 1;
        let planes: Vec<&[f32]> = merged.chunks_exact(n).collect();
        let (sx, sy) = (side as f64 / f64::from(crop.width().max(1)), side as f64 / f64::from(crop.height().max(1)));
        let scaled = sx < 1.0 - 1e-9 || sy < 1.0 - 1e-9;
        let area = group.intersect(&req.window);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let Some(k) = window_index(req.window, x, y) else { continue };
                if !req.hole.get(k).copied().unwrap_or(false) || filled.get(k).copied().unwrap_or(true) {
                    continue;
                }
                let u = (f64::from(x - crop.x0) + 0.5) * sx - 0.5;
                let v = (f64::from(y - crop.y0) + 0.5) * sy - 0.5;
                let c = |i: usize| planes.get(i).map_or(0.0, |p| resample(p, side, u, v, true));
                put(req, &mut out, k, [c(0), c(1), c(2)]);
                if let Some(f) = filled.get_mut(k) {
                    *f = true;
                }
            }
        }
        if scaled && opts.refine {
            refine(req, area, side, &mut out, &filled, read, model, ctl, opts, &mut runs)?;
        }
        ctl.progress((gi + 1) as f32 / groups.len() as f32);
    }
    Ok(out)
}

/// The depth of each refinement ring, in pixels.
pub const REFINE_BAND: u32 = 128;

/// How far a refinement tile reaches past the cell it redoes, on each side.
const TILE_CONTEXT: usize = 64;

/// Redo the shrunk fill of the hole pixels in `area` at full resolution (see [`fill_with`]).
#[allow(clippy::too_many_arguments)]
fn refine(
    req: &Request,
    area: Rect,
    side: usize,
    out: &mut [f32],
    filled: &[bool],
    read: &mut dyn FnMut(Rect) -> Vec<f32>,
    model: &mut Model<'_>,
    ctl: &Interrupt,
    opts: &Options,
    runs: &mut usize,
) -> Result<(), MlError> {
    let (aw, ah) = (area.width() as usize, area.height() as usize);
    if aw == 0 || ah == 0 || side <= 2 * TILE_CONTEXT {
        return Ok(());
    }
    let ww = req.window.width() as usize;
    let at = |x: usize, y: usize| window_index(req.window, area.x0 + x as i32, area.y0 + y as i32);
    let dist = depth(aw, ah, |x, y| at(x, y).is_some_and(|k| req.hole.get(k).copied().unwrap_or(false)));
    let deepest = dist.iter().copied().filter(|d| *d < UNREACHED).max().unwrap_or(0);
    let band = REFINE_BAND.max(1);
    let rings = if opts.rings { deepest / band + 1 } else { 1 };
    let cell = side - 2 * TILE_CONTEXT;
    let mut redone = vec![false; aw * ah];
    for ring in 0..rings {
        let (lo, hi) = (ring * band, if ring + 1 == rings { u32::MAX } else { (ring + 1) * band });
        let in_ring = |x: usize, y: usize| dist.get(y * aw + x).is_some_and(|d| *d > 0 && *d < UNREACHED && *d >= lo && *d < hi);
        for ty in (0..ah).step_by(cell) {
            for tx in (0..aw).step_by(cell) {
                let (cx1, cy1) = ((tx + cell).min(aw), (ty + cell).min(ah));
                if !(ty..cy1).any(|y| (tx..cx1).any(|x| in_ring(x, y) && !redone[y * aw + x])) {
                    continue;
                }
                if *runs >= opts.max_runs {
                    return Ok(());
                }
                ctl.check()?;
                // The tile: the cell plus context, moved inside the canvas.
                let place = |lo: i32, c0: i32, c1: i32| (lo - TILE_CONTEXT as i32).min(c1 - side as i32).max(c0);
                let (x0, y0) = (place(area.x0 + tx as i32, req.canvas.x0, req.canvas.x1), place(area.y0 + ty as i32, req.canvas.y0, req.canvas.y1));
                let tile = Rect::new(x0, y0, (x0 + side as i32).min(req.canvas.x1), (y0 + side as i32).min(req.canvas.y1));
                if tile.width() as usize != side || tile.height() as usize != side {
                    return Ok(());
                }
                let state = |k: usize| {
                    let (x, y) = ((k % ww) as i32 + req.window.x0 - area.x0, (k / ww) as i32 + req.window.y0 - area.y0);
                    let here =
                        x >= 0 && y >= 0 && (x as usize) < aw && (y as usize) < ah && in_ring(x as usize, y as usize) && !redone[y as usize * aw + x as usize];
                    let unfilled = req.hole.get(k).copied().unwrap_or(false) && !filled.get(k).copied().unwrap_or(false);
                    if unfilled {
                        Px::Masked
                    } else if here {
                        Px::Redo
                    } else if filled.get(k).copied().unwrap_or(false) {
                        Px::Out
                    } else {
                        Px::Source
                    }
                };
                let input = sample(req, tile, side, out, &state, read, ctl)?;
                let merged = run_model(model, &input, opts)?;
                *runs += 1;
                for y in ty..cy1 {
                    for x in tx..cx1 {
                        if !in_ring(x, y) || redone[y * aw + x] {
                            continue;
                        }
                        let Some(k) = at(x, y) else { continue };
                        let (u, v) = ((area.x0 + x as i32 - tile.x0) as usize, (area.y0 + y as i32 - tile.y0) as usize);
                        let p = v * side + u;
                        let c = |i: usize| merged.get(i * side * side + p).copied().unwrap_or(0.0);
                        put(req, out, k, [c(0), c(1), c(2)]);
                        redone[y * aw + x] = true;
                    }
                }
            }
        }
    }
    Ok(())
}

/// A pixel [`depth`] couldn't reach (no known pixel anywhere).
const UNREACHED: u32 = u32::MAX / 4;

/// How deep each pixel of a `w × h` grid lies inside the hole (`inside(x, y)`), in pixels from
/// the nearest pixel that isn't (chamfer distance; 0 outside; past the grid is outside).
fn depth(w: usize, h: usize, inside: impl Fn(usize, usize) -> bool) -> Vec<u32> {
    let mut d = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            d[y * w + x] = if inside(x, y) { UNREACHED * 3 } else { 0 };
        }
    }
    let get = |d: &[u32], x: isize, y: isize| if x < 0 || y < 0 || x as usize >= w || y as usize >= h { 0 } else { d[y as usize * w + x as usize] };
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let v = d[y * w + x];
            if v != 0 {
                d[y * w + x] = v.min(get(&d, xi - 1, yi) + 3).min(get(&d, xi, yi - 1) + 3).min(get(&d, xi - 1, yi - 1) + 4).min(get(&d, xi + 1, yi - 1) + 4);
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let (xi, yi) = (x as isize, y as isize);
            let v = d[y * w + x];
            if v != 0 {
                d[y * w + x] = v.min(get(&d, xi + 1, yi) + 3).min(get(&d, xi, yi + 1) + 3).min(get(&d, xi + 1, yi + 1) + 4).min(get(&d, xi - 1, yi + 1) + 4);
            }
        }
    }
    // Chamfer units (3 a step) to pixels, at least 1 inside.
    for v in &mut d {
        if *v != 0 {
            *v = (*v / 3).max(1);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: Rect) -> usize {
        r.width() as usize * r.height() as usize
    }

    fn mask(w: usize, h: usize, on: &[(usize, usize)]) -> Vec<bool> {
        let mut m = vec![false; w * h];
        for &(x, y) in on {
            m[y * w + x] = true;
        }
        m
    }

    #[test]
    fn nearby_parts_are_one_group_and_far_ones_two() {
        let win = Rect::new(100, 50, 400, 150);
        // Two blobs 10 px apart, a third 200 px away.
        let m = mask(300, 100, &[(10, 10), (11, 10), (25, 12), (230, 80)]);
        let g = groups(win, &m);
        assert_eq!(g.len(), 2, "{g:?}");
        assert!(g.contains(&Rect::new(110, 60, 126, 63)));
        assert!(g.contains(&Rect::new(330, 130, 331, 131)));
        // Diagonal neighbours belong together.
        let d = groups(Rect::new(0, 0, 3, 3), &mask(3, 3, &[(0, 0), (1, 1), (2, 2)]));
        assert_eq!(d, [Rect::new(0, 0, 3, 3)]);
        // A mismatched mask gives nothing rather than a panic.
        assert!(groups(win, &[true; 3]).is_empty());
    }

    #[test]
    fn a_small_hole_gets_a_full_resolution_square_and_a_large_one_context() {
        let canvas = Rect::new(0, 0, 4000, 3000);
        let small = crop(Rect::new(1000, 1000, 1040, 1020), canvas);
        assert_eq!((small.width(), small.height()), (512, 512));
        assert!(small.contains_rect(&Rect::new(1000, 1000, 1040, 1020)));
        // Up to 384 across fits 1:1 with 64 px of context.
        let fits = crop(Rect::new(1000, 1000, 1384, 1200), canvas);
        assert_eq!((fits.width(), fits.height()), (512, 512));
        let big = Rect::new(1000, 1000, 2000, 1500);
        let c = crop(big, canvas);
        assert_eq!((c.width(), c.height()), (1300, 1300));
        assert!(c.contains_rect(&big));
        // At the canvas edge the square moves inside; on a small canvas it is the canvas.
        let corner = crop(Rect::new(0, 0, 30, 30), canvas);
        assert_eq!(corner, Rect::new(0, 0, 512, 512));
        let tiny = Rect::new(0, 0, 300, 200);
        assert_eq!(crop(Rect::new(10, 10, 20, 20), tiny), tiny);
    }

    #[test]
    fn weights_cover_every_target_once() {
        for len in [1, 7, 300, 512, 513, 1600, 5000] {
            let w = weights(len, SIDE);
            let mut sum = vec![0f64; SIDE];
            for taps in &w {
                for &(t, v) in taps {
                    sum[t] += f64::from(v);
                }
            }
            for (t, s) in sum.iter().enumerate() {
                assert!((s - 1.0).abs() < 1e-4, "len {len} target {t}: {s}");
            }
        }
        // 1:1 is the identity.
        let w = weights(SIDE, SIDE);
        assert!(w.iter().enumerate().all(|(s, taps)| taps.as_slice() == [(s, 1.0)]));
    }

    fn gradient(rect: Rect, ch: usize) -> Vec<f32> {
        let mut v = Vec::new();
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                for c in 0..ch {
                    v.push(if c == 3 { 1.0 } else { ((x + 2 * y + 50 * c as i32) % 1000) as f32 / 1000.0 });
                }
            }
        }
        v
    }

    /// The model stand-in: fills the mask with the mean colour of what it can see.
    fn mean_model(rgb: &[f32], mask: &[f32], _hint: Option<&[f32]>) -> Result<Vec<f32>, MlError> {
        let n = SIDE * SIDE;
        let mut out = rgb.to_vec();
        for c in 0..3 {
            let (mut s, mut k) = (0.0f64, 0.0f64);
            for p in 0..n {
                if mask[p] == 0.0 {
                    s += f64::from(rgb[c * n + p]);
                    k += 1.0;
                }
            }
            for p in 0..n {
                if mask[p] > 0.0 {
                    out[c * n + p] = (s / k) as f32;
                }
            }
        }
        Ok(out)
    }

    #[test]
    fn only_the_hole_changes_and_the_model_sees_its_surroundings() {
        let canvas = Rect::new(0, 0, 900, 700);
        let window = Rect::new(300, 200, 400, 300);
        let ch = 4;
        let pixels = gradient(window, ch);
        let mut hole = vec![false; 100 * 100];
        for y in 40..60 {
            for x in 30..70 {
                hole[y * 100 + x] = true;
            }
        }
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch, colours: Colours::Rgb, alpha: Some(3), linear: false, transparent_unknown: false };
        let mut reads = Vec::new();
        let mut calls = 0;
        let out = fill(
            &req,
            &mut |r| {
                reads.push(r);
                gradient(r, ch)
            },
            &mut |rgb, m, _| {
                calls += 1;
                // Every masked cell is zeroed; the mask covers the hole's 40 × 20 at 1:1.
                assert_eq!(m.iter().filter(|v| **v > 0.0).count(), 800);
                mean_model(rgb, m, None)
            },
            &Interrupt::NONE,
        )
        .unwrap();
        assert_eq!(calls, 1);
        // The crop is 512 square around the hole, read in bands inside the canvas.
        assert!(reads.iter().all(|r| canvas.contains_rect(r) && r.width() == 512));
        assert_eq!(reads.iter().map(|r| r.height()).sum::<u32>(), 512);
        for (k, m) in hole.iter().enumerate() {
            let (a, b) = (&pixels[k * ch..(k + 1) * ch], &out[k * ch..(k + 1) * ch]);
            if *m {
                assert_eq!(b[3], 1.0);
                // The mean of a gradient: mid-grey-ish, the same in every hole pixel.
                assert!((b[0] - out[(45 * 100 + 50) * ch]).abs() < 1e-5);
            } else {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn transparent_surroundings_and_bad_requests_are_errors() {
        let canvas = Rect::new(0, 0, 600, 600);
        let window = Rect::new(0, 0, 10, 10);
        let pixels = vec![0.5f32; 100 * 4];
        let hole = mask(10, 10, &[(5, 5)]);
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 4, colours: Colours::Rgb, alpha: Some(3), linear: false, transparent_unknown: false };
        let clear = |r: Rect| vec![0.0f32; area(r) * 4];
        let e = fill(&req, &mut |r| clear(r), &mut mean_model, &Interrupt::NONE).unwrap_err();
        assert!(matches!(e, MlError::Unsuitable(_)), "{e}");
        // A source that returns the wrong amount, a model that returns the wrong amount.
        let short = fill(&req, &mut |_| vec![1.0; 3], &mut mean_model, &Interrupt::NONE).unwrap_err();
        assert!(matches!(short, MlError::BadInput(_)));
        let opaque = |r: Rect| vec![1.0f32; area(r) * 4];
        let wrong = fill(&req, &mut |r| opaque(r), &mut |_, _, _| Ok(vec![0.0; 5]), &Interrupt::NONE).unwrap_err();
        assert!(matches!(wrong, MlError::Inference(_)));
        // Mismatched sizes, a window off the canvas, too few channels.
        for bad in [
            Request { hole: &hole[..5], ..req },
            Request { window: Rect::new(595, 0, 605, 10), ..req },
            Request { ch: 2, pixels: &pixels[..200], alpha: None, ..req },
            Request { alpha: Some(7), ..req },
            Request { window: Rect::EMPTY, hole: &[], pixels: &[], ..req },
        ] {
            assert!(matches!(fill(&bad, &mut |r| opaque(r), &mut mean_model, &Interrupt::NONE), Err(MlError::BadInput(_))));
        }
        // Cancelled before the model runs.
        let cancel = || true;
        let ctl = Interrupt::cancel_only(&cancel);
        assert_eq!(fill(&req, &mut |r| opaque(r), &mut mean_model, &ctl).unwrap_err(), MlError::Cancelled);
    }

    #[test]
    fn grey_and_linear_documents_round_trip_through_the_model() {
        let canvas = Rect::new(0, 0, 512, 512);
        let window = Rect::new(100, 100, 120, 120);
        let pixels = vec![0.2f32; 400];
        let hole = mask(20, 20, &[(10, 10)]);
        let flat = |r: Rect| vec![0.2f32; area(r)];
        // The model returns its input's colour: a flat grey stays the same grey, linear or not.
        for linear in [false, true] {
            let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch: 1, colours: Colours::Gray, alpha: None, linear, transparent_unknown: false };
            let out = fill(&req, &mut |r| flat(r), &mut mean_model, &Interrupt::NONE).unwrap();
            assert!((out[10 * 20 + 10] - 0.2).abs() < 1e-4, "linear {linear}: {:?}", &out[205..215]);
        }
        assert!((decode(encode(0.18)) - 0.18).abs() < 1e-5);
    }

    #[test]
    fn a_large_hole_is_shrunk_to_the_model_and_scaled_back() {
        let canvas = Rect::new(0, 0, 3000, 2000);
        let window = Rect::new(500, 500, 1700, 1300);
        let (w, h) = (1200usize, 800usize);
        let pixels = vec![0.5f32; w * h * 3];
        let mut hole = vec![false; w * h];
        for y in 100..700 {
            for x in 100..1100 {
                hole[y * w + x] = true;
            }
        }
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
        let mut largest = 0;
        let out = fill(
            &req,
            &mut |r| {
                largest = largest.max(area(r));
                vec![0.5f32; area(r) * 3]
            },
            &mut mean_model,
            &Interrupt::NONE,
        )
        .unwrap();
        // The crop is 1300 square; it is read 64 rows at a time.
        assert_eq!(largest, 1300 * 64);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-4));
    }

    #[test]
    fn far_apart_strokes_are_filled_one_after_another() {
        let canvas = Rect::new(0, 0, 3000, 1000);
        let window = Rect::new(0, 0, 3000, 1000);
        let pixels = vec![0.3f32; 3000 * 1000 * 3];
        let hole = mask(3000, 1000, &[(100, 100), (2900, 900)]);
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
        let mut calls = 0;
        let last = std::sync::Mutex::new(0.0f32);
        let report = |p: f32| *last.lock().unwrap() = p;
        let never = || false;
        let ctl = Interrupt::new(&never, &report);
        let out = fill(
            &req,
            &mut |r| vec![0.3f32; area(r) * 3],
            &mut |rgb, m, _| {
                calls += 1;
                mean_model(rgb, m, None)
            },
            &ctl,
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(*last.lock().unwrap(), 1.0);
        assert!((out[(100 * 3000 + 100) * 3] - 0.3).abs() < 1e-5, "{:?}", &out[(100 * 3000 + 99) * 3..(100 * 3000 + 102) * 3]);
        assert!((out[(900 * 3000 + 2900) * 3] - 0.3).abs() < 1e-5);
    }

    /// A model that fills the hole with a pixel checkerboard on its own grid: shrunk, the pattern
    /// is lost; redone at full resolution, every hole pixel alternates with its neighbour.
    #[test]
    fn a_shrunk_fill_is_redone_at_full_resolution_ring_by_ring() {
        let canvas = Rect::new(0, 0, 2000, 1600);
        let window = canvas;
        let (w, h) = (2000usize, 1600usize);
        let pixels = vec![0.5f32; w * h * 3];
        let mut hole = vec![false; w * h];
        // 700 × 500: shrunk (crop 910), 250 deep: two rings.
        for y in 500..1000 {
            for x in 600..1300 {
                hole[y * w + x] = true;
            }
        }
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
        let checker = |rgb: &[f32], m: &[f32]| -> Result<Vec<f32>, MlError> {
            let mut out = rgb.to_vec();
            let n = SIDE * SIDE;
            for p in 0..n {
                if m[p] > 0.0 {
                    let v = if (p % SIDE + p / SIDE).is_multiple_of(2) { 0.2 } else { 0.8 };
                    for c in 0..3 {
                        out[c * n + p] = v;
                    }
                }
            }
            Ok(out)
        };
        let mut runs = 0;
        let opts = Options { correct: false, ..LAMA };
        let out = fill_with(
            &req,
            &mut |r| vec![0.5f32; area(r) * 3],
            &mut |rgb, m, _| {
                runs += 1;
                checker(rgb, m)
            },
            &Interrupt::NONE,
            &opts,
        )
        .unwrap();
        assert!(runs > 1 && runs <= LAMA.max_runs, "{runs} runs");
        let at = |x: usize, y: usize| out[(y * w + x) * 3];
        for (x, y) in [(601, 501), (950, 750), (1297, 998), (700, 900)] {
            let (a, b) = (at(x, y), at(x + 1, y));
            assert!((a - b).abs() > 0.5, "({x}, {y}): {a} then {b}, not full-resolution");
        }
        // Outside the hole nothing changed.
        assert_eq!(at(599, 750), 0.5);
        // Without refining, the shrunk checkerboard comes back blurred.
        let out = fill_with(&req, &mut |r| vec![0.5f32; area(r) * 3], &mut |rgb, m, _| checker(rgb, m), &Interrupt::NONE, &Options { refine: false, ..opts })
            .unwrap();
        let (a, b) = (out[(750 * w + 950) * 3], out[(750 * w + 951) * 3]);
        assert!((a - b).abs() < 0.5, "{a} {b}");
        // The run budget stops the refinement, not the fill.
        let mut capped = 0;
        let out = fill_with(
            &req,
            &mut |r| vec![0.5f32; area(r) * 3],
            &mut |rgb, m, _| {
                capped += 1;
                checker(rgb, m)
            },
            &Interrupt::NONE,
            &Options { max_runs: 3, ..opts },
        )
        .unwrap();
        assert_eq!(capped, 3);
        assert!(hole.iter().zip(out.as_chunks::<3>().0).all(|(m, px)| !*m || px[0] > 0.0));
    }

    /// The model's drift in tone at the hole's edge is taken out, and neutral surroundings get a
    /// neutral fill.
    #[test]
    fn the_fill_is_corrected_to_its_surroundings() {
        let canvas = Rect::new(0, 0, 600, 600);
        let window = Rect::new(200, 200, 300, 300);
        let pixels = vec![0.9f32; 100 * 100 * 3];
        let mut hole = vec![false; 100 * 100];
        for y in 30..70 {
            for x in 30..70 {
                hole[y * 100 + x] = true;
            }
        }
        let req =
            Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false, transparent_unknown: false };
        // A model whose answer is off by (-0.08, -0.05, +0.02) everywhere.
        let drifting = |rgb: &[f32], m: &[f32]| -> Result<Vec<f32>, MlError> {
            let n = SIDE * SIDE;
            let mut out = mean_model(rgb, m, None)?;
            for (c, d) in [-0.08f32, -0.05, 0.02].into_iter().enumerate() {
                for p in 0..n {
                    out[c * n + p] += d;
                }
            }
            Ok(out)
        };
        let out = fill(&req, &mut |r| vec![0.9f32; area(r) * 3], &mut |rgb, m, _| drifting(rgb, m), &Interrupt::NONE).unwrap();
        let mid = &out[(50 * 100 + 50) * 3..(50 * 100 + 50) * 3 + 3];
        assert!(mid.iter().all(|v| (v - 0.9).abs() < 0.01), "{mid:?}");
        // Uncorrected it is off.
        let raw = fill_with(&req, &mut |r| vec![0.9f32; area(r) * 3], &mut |rgb, m, _| drifting(rgb, m), &Interrupt::NONE, &Options { correct: false, ..LAMA })
            .unwrap();
        assert!((raw[(50 * 100 + 50) * 3] - 0.82).abs() < 0.01);
        // Grey surroundings and a model that tints: a grey fill.
        let tinting = |rgb: &[f32], m: &[f32]| -> Result<Vec<f32>, MlError> {
            let n = SIDE * SIDE;
            let mut out = rgb.to_vec();
            for p in 0..n {
                if m[p] > 0.0 {
                    let x = (p % SIDE) as f32 / SIDE as f32;
                    (out[p], out[n + p], out[2 * n + p]) = (x, x * 0.5, 0.3);
                }
            }
            Ok(out)
        };
        let src = |r: Rect| -> Vec<f32> {
            let mut v = Vec::new();
            for _y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let g = x.rem_euclid(100) as f32 / 100.0;
                    v.extend([g, g, g]);
                }
            }
            v
        };
        let grey = src(window);
        let req = Request { pixels: &grey, ..req };
        let out = fill(&req, &mut |r| src(r), &mut |rgb, m, _| tinting(rgb, m), &Interrupt::NONE).unwrap();
        for (k, m) in hole.iter().enumerate() {
            if *m {
                let px = &out[k * 3..k * 3 + 3];
                assert!((px[0] - px[1]).abs() < 1e-6 && (px[1] - px[2]).abs() < 1e-6, "{px:?}");
            }
        }
    }

    #[test]
    fn bicubic_resampling_is_exact_on_samples_and_never_overshoots() {
        let side = 16;
        // A hard edge: 0, then 1.
        let plane: Vec<f32> = (0..side * side).map(|p| if p % side < 8 { 0.0 } else { 1.0 }).collect();
        assert_eq!(resample(&plane, side, 3.0, 4.0, true), 0.0);
        assert_eq!(resample(&plane, side, 9.0, 4.0, true), 1.0);
        for i in 0..100 {
            let u = 5.0 + f64::from(i) * 0.05;
            let v = resample(&plane, side, u, 7.3, true);
            assert!((0.0..=1.0).contains(&v), "{u}: {v}");
        }
        // A ramp: bicubic is exact on it, as bilinear is.
        let ramp: Vec<f32> = (0..side * side).map(|p| (p % side) as f32 / 15.0).collect();
        assert!((resample(&ramp, side, 6.4, 3.0, true) - 6.4 / 15.0).abs() < 1e-5);
        assert_eq!(resample(&[], 0, 1.0, 1.0, true), 0.0);
    }

    #[test]
    fn push_pull_spreads_known_values_smoothly() {
        let (w, h) = (64, 64);
        let mut weight = vec![0f32; w * h];
        let mut vals = vec![0f32; w * h];
        for y in 0..h {
            weight[y * w] = 1.0;
            vals[y * w] = 1.0;
            weight[y * w + w - 1] = 1.0;
        }
        let out = push_pull(&[vals], &weight, w, h);
        let row = &out[0][32 * w..33 * w];
        assert!((row[0] - 1.0).abs() < 1e-6 && row[w - 1].abs() < 1e-6);
        // In between it goes from one to the other, never outside.
        assert!(row.iter().all(|v| (-1e-6..=1.0 + 1e-6).contains(v)), "{row:?}");
        assert!(row[20] > row[44], "{row:?}");
        assert!(push_pull(&[], &[], 0, 0).is_empty());
    }

    #[test]
    fn depth_counts_pixels_from_the_hole_edge() {
        let d = depth(9, 9, |x, y| (2..7).contains(&x) && (2..7).contains(&y));
        assert_eq!(&d[4 * 9..5 * 9], [0, 0, 1, 2, 3, 2, 1, 0, 0]);
        // A hole filling the grid is measured from past the grid.
        let all = depth(3, 3, |_, _| true);
        assert_eq!(all[4], 2);
    }
}
