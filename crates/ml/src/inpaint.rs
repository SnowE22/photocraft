//! A document region with a hole, to a fixed-size inpainting model and back.
//!
//! The hole is split into groups of nearby pixels (its connected parts, merged when they come
//! within [`GAP`] pixels of each other), and each group gets its own square crop: the group's
//! extent plus [`CONTEXT`] of it on every side (at least [`MIN_MARGIN`] pixels), and never less
//! than the model's side, so a hole up to about 256 pixels across is filled at full resolution.
//! A larger crop is scaled down to the model's side (area averaging) and the fill scaled back up
//! (bilinear), which softens large fills. Only the hole's pixels change, and groups are filled one
//! after another, each seeing the fills before it.
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

/// Context around a group, as a fraction of its larger side, on every side.
pub const CONTEXT: f64 = 0.3;

/// Least context around a group, in pixels.
pub const MIN_MARGIN: u32 = 128;

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
pub type Model<'a> = dyn FnMut(&[f32], &[f32]) -> Result<Vec<f32>, MlError> + 'a;

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

/// The square crop the model sees for `group`: its extent plus context, at least [`SIDE`],
/// centred on it, moved inside the canvas and cut to it.
pub fn crop(group: Rect, canvas: Rect) -> Rect {
    let e = group.width().max(group.height());
    let margin = ((f64::from(e) * CONTEXT).ceil() as u32).max(MIN_MARGIN);
    let side = i64::from(e.saturating_add(margin.saturating_mul(2)).max(SIDE as u32));
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

/// How `len` source samples map onto [`SIDE`] targets: for each source sample, the targets it
/// feeds and with what weight (each target's weights sum to 1). Area averaging when shrinking,
/// bilinear when enlarging, pixel centres aligned.
fn weights(len: usize) -> Vec<Vec<(usize, f32)>> {
    let mut by_source = vec![Vec::new(); len];
    if len == 0 {
        return by_source;
    }
    let scale = len as f64 / SIDE as f64;
    for t in 0..SIDE {
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
struct Input {
    rgb: Vec<f32>,
    mask: Vec<f32>,
}

/// One window pixel's index, if `(x, y)` is in the window.
fn window_index(window: Rect, x: i32, y: i32) -> Option<usize> {
    if !window.contains(x, y) {
        return None;
    }
    let w = window.width() as usize;
    Some((y - window.y0) as usize * w + (x - window.x0) as usize)
}

/// Read `crop` in bands and shrink or enlarge it to the model's side. Pixels filled by earlier
/// groups come from `out`; hole pixels not filled yet are masked.
fn sample(req: &Request, crop: Rect, out: &[f32], filled: &[bool], read: &mut dyn FnMut(Rect) -> Vec<f32>, ctl: &Interrupt) -> Result<Input, MlError> {
    let (cw, chh) = (crop.width() as usize, crop.height() as usize);
    let ch = req.ch;
    if cw == 0 || chh == 0 || ch == 0 {
        return Err(MlError::BadInput("nothing to read around the hole".into()));
    }
    let (wx, wy) = (weights(cw), weights(chh));
    let n = SIDE * SIDE;
    // Planes: r, g, b, hole.
    let mut acc = vec![0f32; 4 * n];
    let mut row = vec![[0f32; 4]; cw];
    let mut hrow = vec![[0f32; 4]; SIDE];
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
                let done = wi.is_some_and(|k| filled.get(k).copied().unwrap_or(false));
                let hole = !done && wi.is_some_and(|k| req.hole.get(k).copied().unwrap_or(false));
                let px = match wi {
                    Some(k) if done => out.get(k * ch..(k + 1) * ch).unwrap_or(px),
                    _ => px,
                };
                if !hole && req.alpha.is_some_and(|a| px.get(a).is_some_and(|v| *v < OPAQUE)) {
                    return Err(MlError::Unsuitable("the area around the stroke has transparent pixels".into()));
                }
                let c = |k: usize| {
                    let v = px.get(k).copied().unwrap_or(0.0);
                    if req.linear { encode(v) } else { v.clamp(0.0, 1.0) }
                };
                *slot = if hole {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    match req.colours {
                        Colours::Rgb => [c(0), c(1), c(2), 0.0],
                        Colours::Gray => [c(0), c(0), c(0), 0.0],
                    }
                };
            }
            // Horizontal, then into the target rows this source row feeds.
            for v in hrow.iter_mut() {
                *v = [0.0; 4];
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
                        if let Some(a) = acc.get_mut(p * n + t * SIDE + x) {
                            *a += w * s;
                        }
                    }
                }
            }
        }
        y = band.y1;
    }
    let (rgb, rest) = acc.split_at(3 * n);
    // A cell any hole pixel reaches is filled; its colour is zeroed as the model expects.
    let mask: Vec<f32> = rest.iter().map(|m| if *m > 1e-6 { 1.0 } else { 0.0 }).collect();
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
    Ok(Input { rgb, mask })
}

/// Bilinear sample of a planar `SIDE × SIDE` channel at model coordinates `(u, v)`.
fn bilinear(plane: &[f32], u: f64, v: f64) -> f32 {
    let max = (SIDE - 1) as f64;
    let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(SIDE - 1), (y0 + 1).min(SIDE - 1));
    let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
    let at = |x: usize, y: usize| plane.get(y * SIDE + x).copied().unwrap_or(0.0);
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Fill the request's hole: `read(rect)` gives the source's pixels over `rect` (inside the
/// canvas, `ch` samples each); `model(rgb, mask)` runs the model on planar RGB (`SIDE × SIDE`,
/// 0–1) and its mask (1 = fill) and gives back planar RGB (0–1). Returns the window's pixels with
/// the hole filled.
pub fn fill(req: &Request, read: &mut dyn FnMut(Rect) -> Vec<f32>, model: &mut Model<'_>, ctl: &Interrupt) -> Result<Vec<f32>, MlError> {
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
    let mut out = req.pixels.to_vec();
    let mut filled = vec![false; pixels];
    let groups = groups(req.window, req.hole);
    let n = SIDE * SIDE;
    for (gi, group) in groups.iter().enumerate() {
        ctl.check()?;
        let crop = crop(*group, req.canvas);
        let input = sample(req, crop, &out, &filled, read, ctl)?;
        let pred = model(&input.rgb, &input.mask)?;
        if pred.len() != 3 * n {
            return Err(MlError::Inference(format!("the model gave {} values, not {}", pred.len(), 3 * n)));
        }
        // Model space: the fill in the mask, the input elsewhere, so the hole's edge meets its
        // surroundings when scaled back up.
        let mut merged = input.rgb;
        for (p, m) in input.mask.iter().enumerate() {
            if *m > 0.0 {
                for c in 0..3 {
                    if let (Some(d), Some(s)) = (merged.get_mut(c * n + p), pred.get(c * n + p)) {
                        *d = s.clamp(0.0, 1.0);
                    }
                }
            }
        }
        let planes: Vec<&[f32]> = merged.chunks_exact(n).collect();
        let (sx, sy) = (SIDE as f64 / f64::from(crop.width().max(1)), SIDE as f64 / f64::from(crop.height().max(1)));
        let area = group.intersect(&req.window);
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                let Some(k) = window_index(req.window, x, y) else { continue };
                if !req.hole.get(k).copied().unwrap_or(false) || filled.get(k).copied().unwrap_or(true) {
                    continue;
                }
                let u = (f64::from(x - crop.x0) + 0.5) * sx - 0.5;
                let v = (f64::from(y - crop.y0) + 0.5) * sy - 0.5;
                let rgb: Vec<f32> = planes.iter().map(|p| bilinear(p, u, v)).collect();
                let back = |v: f32| if req.linear { decode(v) } else { v };
                let Some(px) = out.get_mut(k * req.ch..(k + 1) * req.ch) else { continue };
                match req.colours {
                    Colours::Rgb => {
                        for (d, s) in px.iter_mut().zip(&rgb) {
                            *d = back(*s);
                        }
                    }
                    Colours::Gray => {
                        if let Some(d) = px.first_mut() {
                            *d = back(rgb.iter().sum::<f32>() / 3.0);
                        }
                    }
                }
                if let Some(a) = req.alpha.and_then(|a| px.get_mut(a)) {
                    *a = 1.0;
                }
                if let Some(f) = filled.get_mut(k) {
                    *f = true;
                }
            }
        }
        ctl.progress((gi + 1) as f32 / groups.len() as f32);
    }
    Ok(out)
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
        let big = Rect::new(1000, 1000, 2000, 1500);
        let c = crop(big, canvas);
        assert_eq!((c.width(), c.height()), (1600, 1600));
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
            let w = weights(len);
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
        let w = weights(SIDE);
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
    fn mean_model(rgb: &[f32], mask: &[f32]) -> Result<Vec<f32>, MlError> {
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
        let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch, colours: Colours::Rgb, alpha: Some(3), linear: false };
        let mut reads = Vec::new();
        let mut calls = 0;
        let out = fill(
            &req,
            &mut |r| {
                reads.push(r);
                gradient(r, ch)
            },
            &mut |rgb, m| {
                calls += 1;
                // Every masked cell is zeroed; the mask covers the hole's 40 × 20 at 1:1.
                assert_eq!(m.iter().filter(|v| **v > 0.0).count(), 800);
                mean_model(rgb, m)
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
        let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch: 4, colours: Colours::Rgb, alpha: Some(3), linear: false };
        let clear = |r: Rect| vec![0.0f32; area(r) * 4];
        let e = fill(&req, &mut |r| clear(r), &mut mean_model, &Interrupt::NONE).unwrap_err();
        assert!(matches!(e, MlError::Unsuitable(_)), "{e}");
        // A source that returns the wrong amount, a model that returns the wrong amount.
        let short = fill(&req, &mut |_| vec![1.0; 3], &mut mean_model, &Interrupt::NONE).unwrap_err();
        assert!(matches!(short, MlError::BadInput(_)));
        let opaque = |r: Rect| vec![1.0f32; area(r) * 4];
        let wrong = fill(&req, &mut |r| opaque(r), &mut |_, _| Ok(vec![0.0; 5]), &Interrupt::NONE).unwrap_err();
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
            let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch: 1, colours: Colours::Gray, alpha: None, linear };
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
        let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false };
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
        // The crop is 1600 square; it is read 64 rows at a time.
        assert_eq!(largest, 1600 * 64);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-4));
    }

    #[test]
    fn far_apart_strokes_are_filled_one_after_another() {
        let canvas = Rect::new(0, 0, 3000, 1000);
        let window = Rect::new(0, 0, 3000, 1000);
        let pixels = vec![0.3f32; 3000 * 1000 * 3];
        let hole = mask(3000, 1000, &[(100, 100), (2900, 900)]);
        let req = Request { canvas, window, hole: &hole, pixels: &pixels, ch: 3, colours: Colours::Rgb, alpha: None, linear: false };
        let mut calls = 0;
        let last = std::sync::Mutex::new(0.0f32);
        let report = |p: f32| *last.lock().unwrap() = p;
        let never = || false;
        let ctl = Interrupt::new(&never, &report);
        let out = fill(
            &req,
            &mut |r| vec![0.3f32; area(r) * 3],
            &mut |rgb, m| {
                calls += 1;
                mean_model(rgb, m)
            },
            &ctl,
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(*last.lock().unwrap(), 1.0);
        assert!((out[(100 * 3000 + 100) * 3] - 0.3).abs() < 1e-5, "{:?}", &out[(100 * 3000 + 99) * 3..(100 * 3000 + 102) * 3]);
        assert!((out[(900 * 3000 + 2900) * 3] - 0.3).abs() < 1e-5);
    }
}
