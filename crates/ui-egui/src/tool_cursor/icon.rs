//! Tool icons as OS cursor bitmaps: Photoshop's standard tool pointers (the Move tool's arrow and
//! its ⌥ duplicate arrows, the lasso, the wand, the crop corners, the pipette, the bucket, the
//! pens). Each is rendered once from the embedded icon SVG, white with a dark outline (or dark
//! with a light one, as Photoshop's arrow), so it moves with the hardware pointer: a painted icon
//! trails the pointer by the app's render and present latency (#1343), and the plain OS arrow
//! made the Move tool, its ⌥ duplicate and many other tools look alike.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use egui::{CustomCursorImage, Vec2, vec2};

/// The largest bitmap side, physical pixels: tool pointers are small.
const MAX_SIDE: usize = 256;

/// One icon of a cursor, in logical pixels from the hotspot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Part {
    pub name: &'static str,
    /// Where the icon's box (its 24-unit grid) has its top-left corner.
    pub at: Vec2,
    pub size: f32,
    /// Dark with a light outline (Photoshop's arrow) instead of light with a dark one.
    pub dark: bool,
    /// Fill the icon's outline shapes (the arrow), not only stroke them.
    pub filled: bool,
}

impl Part {
    /// `name` at `size` with the point `hot` of its box (a fraction, 0..1) on the hotspot.
    pub(crate) fn at_hotspot(name: &'static str, hot: Vec2, size: f32) -> Self {
        Part { name, at: -hot * size, size, dark: false, filled: false }
    }
}

/// The Move tool's black arrow, its tip on the hotspot.
pub(crate) fn arrow(size: f32, dark: bool, offset: Vec2) -> Part {
    // The tip of the icon's arrowhead is at (4.4, 4.4) of its 24-unit box.
    Part { name: "mouse-pointer-2", at: offset - vec2(4.4, 4.4) / 24.0 * size, size, dark, filled: true }
}

type Key = (Vec<(&'static str, [u32; 4], bool, bool)>, u32);

fn cache() -> &'static Mutex<HashMap<Key, Option<CustomCursorImage>>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Option<CustomCursorImage>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The bitmap for `parts` (drawn in order, later ones on top) at `scale` physical pixels per
/// logical pixel, kept for the next ask. `None` when an icon is missing or it can't be drawn.
pub(crate) fn image(parts: &[Part], scale: f32) -> Option<CustomCursorImage> {
    if !scale.is_finite() || scale <= 0.0 || parts.is_empty() {
        return None;
    }
    let key: Key = (parts.iter().map(|p| (p.name, [p.at.x.to_bits(), p.at.y.to_bits(), p.size.to_bits(), 0], p.dark, p.filled)).collect(), scale.to_bits());
    let mut map = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(found) = map.get(&key) {
        return found.clone();
    }
    // A bounded set: tool pointers at a few UI scales.
    if map.len() > 256 {
        map.clear();
    }
    let made = render(parts, scale);
    map.insert(key, made.clone());
    made
}

/// The icon's coverage (0..1) over a `side` × `side` box of physical pixels.
fn coverage(part: &Part, side: usize) -> Option<Vec<f32>> {
    let mut svg = String::from_utf8(crate::icons::svg_bytes(part.name)?.to_vec()).ok()?;
    if part.filled {
        svg = svg.replace("fill=\"none\"", "fill=\"#ffffff\"");
    }
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &resvg::usvg::Options::default()).ok()?;
    let side_u32 = u32::try_from(side).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side_u32, side_u32)?;
    let k = side as f32 / tree.size().width().max(1.0);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    Some(pixmap.data().as_chunks::<4>().0.iter().map(|p| f32::from(p[3]) / 255.0).collect())
}

/// The most of `cov` within `r` pixels of each pixel: the outline around the icon.
fn dilate(cov: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let reach = r.ceil() as i64;
    let mut out = vec![0.0f32; cov.len()];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut m = 0.0f32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let d = ((dx * dx + dy * dy) as f32).sqrt();
                    if d > r + 0.5 {
                        continue;
                    }
                    let (sx, sy) = (x + dx, y + dy);
                    if sx < 0 || sy < 0 || sx >= w as i64 || sy >= h as i64 {
                        continue;
                    }
                    let v = cov.get(sy as usize * w + sx as usize).copied().unwrap_or(0.0) * (r + 0.5 - d).clamp(0.0, 1.0);
                    m = m.max(v);
                }
            }
            if let Some(o) = out.get_mut(y as usize * w + x as usize) {
                *o = m;
            }
        }
    }
    out
}

fn render(parts: &[Part], scale: f32) -> Option<CustomCursorImage> {
    let pad = scale.ceil() + 2.0;
    // The bitmap covers every part and the hotspot, with room for the outline.
    let (mut x0, mut y0, mut x1, mut y1) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in parts {
        if !(p.at.x.is_finite() && p.at.y.is_finite() && p.size.is_finite() && p.size > 0.0) {
            return None;
        }
        x0 = x0.min(p.at.x * scale);
        y0 = y0.min(p.at.y * scale);
        x1 = x1.max((p.at.x + p.size) * scale);
        y1 = y1.max((p.at.y + p.size) * scale);
    }
    let (ox, oy) = ((-x0 + pad).ceil(), (-y0 + pad).ceil());
    let (w, h) = ((ox + x1 + pad).ceil() as usize, (oy + y1 + pad).ceil() as usize);
    if w > MAX_SIDE || h > MAX_SIDE || w == 0 || h == 0 {
        return None;
    }
    // Premultiplied RGBA, composited part by part.
    let mut px = vec![[0.0f32; 4]; w * h];
    let over = |dst: &mut [f32; 4], c: f32, a: f32| {
        let a = a.clamp(0.0, 1.0);
        for ch in dst.iter_mut().take(3) {
            *ch = c * a + *ch * (1.0 - a);
        }
        dst[3] = a + dst[3] * (1.0 - a);
    };
    for p in parts {
        let side = (p.size * scale).round().max(1.0) as usize;
        let cov = coverage(p, side)?;
        let ring = dilate(&cov, side, side, scale.max(1.0));
        let (fill, line) = if p.dark { (0.0, 1.0) } else { (1.0, 0.0) };
        let (lx, ly) = ((ox + p.at.x * scale).round() as i64, (oy + p.at.y * scale).round() as i64);
        // The outline, then the icon over it.
        for (values, colour, alpha) in [(&ring, line, 0.9), (&cov, fill, 1.0)] {
            for (i, v) in values.iter().enumerate() {
                if *v <= 0.0 {
                    continue;
                }
                let (x, y) = (lx + (i % side) as i64, ly + (i / side) as i64);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                if let Some(d) = px.get_mut(y as usize * w + x as usize) {
                    over(d, colour, v * alpha);
                }
            }
        }
    }
    // Straight RGBA, as winit takes it.
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in &px {
        let a = p[3];
        let c = |v: f32| if a > 0.0 { (v / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        rgba.extend_from_slice(&[c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round().clamp(0.0, 255.0) as u8]);
    }
    let size = [u16::try_from(w).ok()?, u16::try_from(h).ok()?];
    let hotspot = [ox as u16, oy as u16];
    Some(CustomCursorImage { rgba: Arc::from(rgba), size, hotspot })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(img: &CustomCursorImage, x: usize, y: usize) -> u8 {
        img.rgba[(y * img.size[0] as usize + x) * 4 + 3]
    }

    #[test]
    fn an_icon_cursor_puts_its_point_on_the_hotspot() {
        // The pipette's tip (2, 22) of its box on the hotspot: the icon lies up and right of it.
        let img = image(&[Part::at_hotspot("pipette", vec2(2.0, 22.0) / 24.0, 20.0)], 1.0).unwrap();
        let [hx, hy] = img.hotspot.map(usize::from);
        assert!(img.size[0] as usize > hx + 10 && hy > 10, "{:?} {:?}", img.size, img.hotspot);
        // Something is drawn near the tip, nothing far down-left of it.
        let near = (0..5).flat_map(|d| [(hx + d, hy.saturating_sub(d))]).any(|(x, y)| alpha(&img, x, y) > 0);
        assert!(near);
        assert_eq!(alpha(&img, 0, img.size[1] as usize - 1), 0);
    }

    #[test]
    fn the_duplicate_arrow_is_not_the_move_arrow() {
        let moving = image(&[arrow(20.0, true, Vec2::ZERO), Part { name: "move", at: vec2(11.0, 13.0), size: 10.0, dark: true, filled: false }], 1.0).unwrap();
        let dup = image(&[arrow(18.0, false, vec2(5.0, 5.0)), arrow(20.0, true, Vec2::ZERO)], 1.0).unwrap();
        assert_ne!(moving.rgba, dup.rgba);
        // The arrow is dark: its middle has dark, opaque pixels.
        let [hx, hy] = dup.hotspot.map(usize::from);
        let i = ((hy + 6) * dup.size[0] as usize + hx + 6) * 4;
        assert!(dup.rgba[i + 3] > 200 && dup.rgba[i] < 60, "{:?}", &dup.rgba[i..i + 4]);
    }

    #[test]
    fn missing_icons_and_bad_sizes_give_no_cursor() {
        assert!(image(&[Part::at_hotspot("no-such-icon", Vec2::ZERO, 20.0)], 1.0).is_none());
        assert!(image(&[Part::at_hotspot("pipette", Vec2::ZERO, f32::NAN)], 1.0).is_none());
        assert!(image(&[Part::at_hotspot("pipette", Vec2::ZERO, 20.0)], 0.0).is_none());
        assert!(image(&[Part::at_hotspot("pipette", Vec2::ZERO, 900.0)], 1.0).is_none());
    }
}
