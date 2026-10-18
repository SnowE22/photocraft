//! Marching ants of a big outline, drawn from a texture (user report: a large or complicated
//! selection made every frame slow, where Photoshop stays smooth).
//!
//! A speckled selection seen whole has a boundary segment at almost every screen pixel. As a mesh
//! ([`crate::canvas::marching_ants_lines`]) that was tens of millions of vertices built, tessellated
//! and uploaded every frame: ten times a second for the march, and on every pointer move between.
//! Here the outline is rasterized once per view into a mask of the canvas's screen pixels, coloured
//! when the dashes march, and drawn as one image; every other frame reuses the texture. The pixels
//! match the mesh's: one pixel wide, white with black dashes four pixels long phased along x + y.
//! Small outlines keep the mesh, which also antialiases slanted (rotated) lines.

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};

use crate::canvas::ViewXform;
use crate::outline::Segment;

/// Outlines with more segments than this are drawn from a texture.
pub const MIN_SEGMENTS: usize = 20_000;
/// A canvas wider or taller than this many pixels keeps the mesh (no texture that size).
const MAX_SIDE: usize = 8192;
/// The dash and gap length, in pixels (as the mesh's).
const DASH: i64 = 4;

/// One view's ants texture and what it was made for.
struct Raster {
    /// The outline (its allocation) and the view the mask was rasterized for.
    key: u64,
    /// The mask's size in pixels and its top-left corner in window pixels (dashes are phased by
    /// window position, so they line up with the mesh-drawn ants of other views).
    size: [usize; 2],
    origin: [i64; 2],
    /// 1 for a pixel on the outline, row by row.
    mask: Vec<u8>,
    /// The march step the texture was coloured for.
    phase: Option<i64>,
    texture: Option<TextureHandle>,
}

type Shared = Arc<Mutex<Option<Raster>>>;

fn cache_id(view: egui::Id) -> egui::Id {
    view.with("ants-raster")
}

/// Draw `segs` (document space, placed on screen by `xf`) as marching ants from a texture kept
/// for `view`. False when the canvas is too big for one (the caller then draws the mesh).
pub fn draw(ctx: &egui::Context, painter: &egui::Painter, view: egui::Id, xf: &ViewXform, segs: &Arc<Vec<Segment>>, time: f64) -> bool {
    let area = xf.rect.intersect(painter.clip_rect());
    let ppp = ctx.pixels_per_point();
    if !area.is_positive() || !area.is_finite() || !ppp.is_finite() || ppp <= 0.0 {
        return true;
    }
    let size = [(area.width() * ppp).round() as usize, (area.height() * ppp).round() as usize];
    if size[0] == 0 || size[1] == 0 {
        return true;
    }
    if size[0] > MAX_SIDE || size[1] > MAX_SIDE {
        return false;
    }
    let origin = [(area.min.x * ppp).round() as i64, (area.min.y * ppp).round() as i64];
    let key = view_key(xf, segs, area, ppp);
    let shared = ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(cache_id(view)).clone());
    let mut slot = shared.lock().unwrap_or_else(PoisonError::into_inner);
    let fresh = slot.as_ref().is_some_and(|r| r.key == key && r.size == size && r.origin == origin);
    if !fresh {
        let mask = rasterize(xf, segs, area.min, ppp, size);
        let texture = slot.take().and_then(|r| r.texture);
        *slot = Some(Raster { key, size, origin, mask, phase: None, texture });
    }
    let Some(r) = slot.as_mut() else { return true };
    let phase = march_step(time);
    if r.phase != Some(phase) || r.texture.is_none() {
        let image = colour(&r.mask, r.size, r.origin, phase);
        match r.texture.as_mut() {
            Some(t) => t.set(image, TextureOptions::NEAREST),
            None => r.texture = Some(ctx.load_texture("pc-ants-raster", image, TextureOptions::NEAREST)),
        }
        r.phase = Some(phase);
    }
    if let Some(t) = &r.texture {
        // The texture covers exactly the pixels it was made for, so it maps one to one.
        let rect = Rect::from_min_size(pos2(r.origin[0] as f32 / ppp, r.origin[1] as f32 / ppp), egui::vec2(r.size[0] as f32 / ppp, r.size[1] as f32 / ppp));
        painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    true
}

/// Forget `view`'s texture (no outline shown any more), freeing its memory.
pub fn clear(ctx: &egui::Context, view: egui::Id) {
    ctx.data_mut(|d| d.remove::<Shared>(cache_id(view)));
}

/// The march step at `time` (seconds): the mesh moves its dashes ten pixels a second.
fn march_step(time: f64) -> i64 {
    if !time.is_finite() {
        return 0;
    }
    ((time * 10.0).rem_euclid((2 * DASH) as f64)) as i64
}

/// What a mask depends on: the outline (by allocation and length: a new trace is a new `Arc`),
/// the view mapping, the canvas part shown and the display scale.
fn view_key(xf: &ViewXform, segs: &Arc<Vec<Segment>>, area: Rect, ppp: f32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100_0000_01b3);
    mix(Arc::as_ptr(segs) as usize as u64);
    mix(segs.len() as u64);
    for f in [xf.rect.min.x, xf.rect.min.y, xf.rect.max.x, xf.rect.max.y, xf.zoom, xf.center[0], xf.center[1], xf.rotation, xf.aspect] {
        mix(u64::from(f.to_bits()));
    }
    mix(u64::from(xf.flip));
    for f in [area.min.x, area.min.y, area.max.x, area.max.y, ppp] {
        mix(u64::from(f.to_bits()));
    }
    h
}

/// The outline's pixels on a `size` canvas whose top-left is `min` (screen points): each segment
/// as the mesh draws it, one pixel wide from end to end inclusive.
fn rasterize(xf: &ViewXform, segs: &[Segment], min: Pos2, ppp: f32, size: [usize; 2]) -> Vec<u8> {
    let (w, h) = (size[0], size[1]);
    let mut mask = vec![0u8; w.saturating_mul(h)];
    // Pixel centres, so a line on a pixel boundary rounds as the mesh rounds it.
    let bounds = Rect::from_min_max(Pos2::ZERO, pos2(w as f32 - 0.5, h as f32 - 0.5));
    let to_px = |p: [i32; 2]| {
        let s = xf.to_screen(p[0] as f32, p[1] as f32);
        pos2((s.x - min.x) * ppp, (s.y - min.y) * ppp)
    };
    let mut set = |x: i64, y: i64| {
        if x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && let Some(m) = mask.get_mut(y as usize * w + x as usize)
        {
            *m = 1;
        }
    };
    for (a, b) in segs {
        let (pa, pb) = (to_px(*a), to_px(*b));
        if !(pa.is_finite() && pb.is_finite()) {
            continue;
        }
        let Some((pa, pb)) = crate::canvas::clip_line(bounds.expand(1.0), pa, pb) else { continue };
        let (x0, y0, x1, y1) = (pa.x.round() as i64, pa.y.round() as i64, pb.x.round() as i64, pb.y.round() as i64);
        if y0 == y1 {
            for x in x0.min(x1)..=x0.max(x1) {
                set(x, y0);
            }
        } else if x0 == x1 {
            for y in y0.min(y1)..=y0.max(y1) {
                set(x0, y);
            }
        } else {
            // A slanted segment (a rotated view): one pixel per step along the longer axis.
            let n = (x1 - x0).abs().max((y1 - y0).abs());
            for i in 0..=n {
                let t = i as f32 / n as f32;
                set((pa.x + (pb.x - pa.x) * t).round() as i64, (pa.y + (pb.y - pa.y) * t).round() as i64);
            }
        }
    }
    mask
}

/// The ants image for `mask` at march step `phase`: outline pixels white, black where
/// (x + y + phase) falls in the first half of each 2 × DASH run, the rest transparent.
fn colour(mask: &[u8], size: [usize; 2], origin: [i64; 2], phase: i64) -> ColorImage {
    let w = size[0].max(1);
    let pixels = mask
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            if m == 0 {
                return Color32::TRANSPARENT;
            }
            let (x, y) = ((i % w) as i64 + origin[0], (i / w) as i64 + origin[1]);
            if (x + y + phase).rem_euclid(2 * DASH) < DASH { Color32::BLACK } else { Color32::WHITE }
        })
        .collect();
    ColorImage::new(size, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xf(zoom: f32) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(40.0, 30.0)), zoom, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 }
    }

    /// A square outline at 100 %: its four sides, end to end, and nothing else.
    #[test]
    fn a_square_outline_marks_its_sides() {
        let segs: Vec<Segment> = vec![([5, 5], [10, 5]), ([10, 5], [10, 10]), ([10, 10], [5, 10]), ([5, 10], [5, 5])];
        let m = rasterize(&xf(1.0), &segs, Pos2::ZERO, 1.0, [40, 30]);
        let on = |x: usize, y: usize| m[y * 40 + x] == 1;
        for i in 5..=10 {
            assert!(on(i, 5) && on(i, 10) && on(5, i) && on(10, i), "side pixel {i}");
        }
        assert!(!on(7, 7), "the inside stays clear");
        assert_eq!(m.iter().filter(|v| **v == 1).count(), 20, "only the sides");
    }

    /// Segments partly or wholly off the canvas, or at hostile coordinates, never panic and mark
    /// only what is on it.
    #[test]
    fn segments_off_the_canvas_are_clipped() {
        let segs: Vec<Segment> = vec![([-1_000_000, 3], [1_000_000, 3]), ([i32::MIN, i32::MIN], [i32::MAX, i32::MAX]), ([100, 100], [200, 100])];
        let m = rasterize(&xf(1.0), &segs, Pos2::ZERO, 1.0, [40, 30]);
        assert!((0..40).all(|x| m[3 * 40 + x] == 1), "the long line crosses the canvas");
        assert!(rasterize(&xf(f32::NAN), &segs, Pos2::ZERO, 1.0, [40, 30]).iter().all(|v| *v == 0));
        assert!(rasterize(&xf(1.0), &segs, Pos2::ZERO, 1.0, [0, 0]).is_empty());
    }

    /// The dashes: four black, four white along x + y, moving one pixel per march step, and only
    /// on outline pixels.
    #[test]
    fn dashes_alternate_and_march() {
        let mut mask = vec![0u8; 16];
        mask[..16].fill(1);
        mask[3] = 0;
        let img = colour(&mask, [16, 1], [0, 0], 0);
        let black: Vec<bool> = img.pixels.iter().map(|c| *c == Color32::BLACK).collect();
        assert_eq!(&black[..8], &[true, true, true, false, false, false, false, false]);
        assert_eq!(img.pixels[3], Color32::TRANSPARENT, "not on the outline");
        // One step on, the dashes have moved a pixel: x = 7 turns black.
        let next = colour(&mask, [16, 1], [0, 0], 1);
        assert_eq!((img.pixels[7], next.pixels[7]), (Color32::WHITE, Color32::BLACK));
        assert_eq!(next.pixels[0], Color32::BLACK);
        assert_eq!(march_step(f64::NAN), 0);
        assert!((0..8).contains(&march_step(1234.567)));
    }

    /// The same outline and view reuse the mask; a pan or a new trace makes a new one.
    #[test]
    fn the_key_follows_the_outline_and_the_view() {
        let a = Arc::new(vec![([0, 0], [4, 0])]);
        let b = Arc::new(vec![([0, 0], [4, 0])]);
        let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(40.0, 30.0));
        let k = view_key(&xf(1.0), &a, area, 1.0);
        assert_eq!(k, view_key(&xf(1.0), &a, area, 1.0));
        assert_ne!(k, view_key(&xf(1.0), &b, area, 1.0), "a new trace");
        let mut panned = xf(1.0);
        panned.center[0] += 1.0;
        assert_ne!(k, view_key(&panned, &a, area, 1.0), "a pan");
        assert_ne!(k, view_key(&xf(1.0), &a, area, 1.25), "another display scale");
    }
}
