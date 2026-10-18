//! Marching ants of a big outline, drawn from a texture (user report: a large or complicated
//! selection made every frame slow, where Photoshop stays smooth).
//!
//! A speckled selection seen whole has a boundary segment at almost every screen pixel. As a mesh
//! ([`crate::canvas::marching_ants_lines`]) that was tens of millions of vertices built, tessellated
//! and uploaded every frame: ten times a second for the march, and on every pointer move between.
//! Here the outline is rasterized into a mask of device pixels, coloured when the dashes march,
//! and drawn as one image; every other frame reuses the texture. The pixels match the mesh's: one
//! pixel wide, white with black dashes four pixels long phased along x + y. Small outlines keep
//! the mesh, which also antialiases slanted (rotated) lines.
//!
//! The mask is anchored to the document, not to the window. Panning only moves where the view
//! puts the document's origin (zoom, rotation, flip, pixel aspect and display scale stay), so a
//! mask made at one pan serves the next moved by whole device pixels: a pan costs a quad, not a
//! rasterization of every segment. The mask covers the canvas shown plus `MARGIN` on each side
//! (clipped to the outline). It is made again for a new outline (the canvas re-traces it as the
//! view crosses its 256-step quanta), another zoom, rotation, flip, aspect or display scale, a pan
//! past the margin, and once a pan by a fraction of a pixel comes to rest (the moved image is then
//! up to a pixel from where rasterizing puts the outline).

use std::sync::{Arc, Mutex, PoisonError, Weak};

use egui::{Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};

use crate::canvas::ViewXform;
use crate::outline::Segment;

/// Outlines with more segments than this are drawn from a texture.
pub const MIN_SEGMENTS: usize = 20_000;
/// A canvas wider or taller than this many device pixels keeps the mesh (no texture that size).
const MAX_SIDE: usize = 8192;
/// The dash and gap length, in pixels (as the mesh's).
const DASH: i64 = 4;
/// Device pixels rasterized past each side of the canvas shown, so a pan moves the texture until
/// it has gone this far (the canvas re-traces a zoomed-out outline every 256 to 512).
const MARGIN: i64 = 512;
/// Seconds a view rests after a pan by a fraction of a pixel before its ants are rasterized
/// again exactly where it puts them.
const SETTLE: f64 = 0.2;

/// Whole device pixels `x0..x1` × `y0..y1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Px {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Px {
    const fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The device pixels any part of `r` (points) falls on at `ppp` device pixels per point.
    fn covering(r: Rect, ppp: f32) -> Self {
        let f = |v: f32| f64::from(v) * f64::from(ppp);
        Self::new(f(r.min.x).floor() as i64, f(r.min.y).floor() as i64, f(r.max.x).ceil() as i64, f(r.max.y).ceil() as i64)
    }

    fn w(self) -> i64 {
        self.x1.saturating_sub(self.x0).max(0)
    }

    fn h(self) -> i64 {
        self.y1.saturating_sub(self.y0).max(0)
    }

    fn is_empty(self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    fn intersect(self, o: Self) -> Self {
        Self::new(self.x0.max(o.x0), self.y0.max(o.y0), self.x1.min(o.x1), self.y1.min(o.y1))
    }

    /// Every pixel of `o` is in `self` (an empty `o` is in anything).
    fn contains(self, o: Self) -> bool {
        o.is_empty() || (self.x0 <= o.x0 && self.y0 <= o.y0 && o.x1 <= self.x1 && o.y1 <= self.y1)
    }

    fn expand(self, mx: i64, my: i64) -> Self {
        Self::new(self.x0.saturating_sub(mx), self.y0.saturating_sub(my), self.x1.saturating_add(mx), self.y1.saturating_add(my))
    }

    fn shift(self, d: [i64; 2]) -> Self {
        Self::new(self.x0.saturating_add(d[0]), self.y0.saturating_add(d[1]), self.x1.saturating_add(d[0]), self.y1.saturating_add(d[1]))
    }

    /// The pixels of `self` outside `o`, as up to four bands (none empty).
    fn minus(self, o: Self) -> Vec<Self> {
        let i = self.intersect(o);
        let parts = if i.is_empty() {
            vec![self]
        } else {
            vec![
                Self::new(self.x0, self.y0, self.x1, i.y0),
                Self::new(self.x0, i.y1, self.x1, self.y1),
                Self::new(self.x0, i.y0, i.x0, i.y1),
                Self::new(i.x1, i.y0, self.x1, i.y1),
            ]
        };
        parts.into_iter().filter(|p| !p.is_empty()).collect()
    }
}

/// Where a view puts document points, in device pixels: `lin` (zoom, rotation, flip, pixel
/// aspect and display scale; row-major 2 × 2), then `offset`, where the document's origin lands
/// (the pan). Panning changes only the offset.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mapping {
    lin: [f64; 4],
    offset: [f64; 2],
}

impl Mapping {
    /// `xf`'s mapping at `ppp` device pixels per point (as [`ViewXform::to_screen`] × `ppp`, with
    /// the pan in f64); `None` when it is not finite.
    fn of(xf: &ViewXform, ppp: f32) -> Option<Self> {
        let p = f64::from(ppp);
        let (ex, ey) = (xf.map_vec(vec2(1.0, 0.0)), xf.map_vec(vec2(0.0, 1.0)));
        let lin = [f64::from(ex.x) * p, f64::from(ey.x) * p, f64::from(ex.y) * p, f64::from(ey.y) * p];
        let (cx, cy) = (f64::from(xf.center[0]), f64::from(xf.center[1]));
        let mid = xf.rect.center();
        let offset = [f64::from(mid.x) * p - lin[0] * cx - lin[1] * cy, f64::from(mid.y) * p - lin[2] * cx - lin[3] * cy];
        lin.iter().chain(&offset).all(|v| v.is_finite()).then_some(Self { lin, offset })
    }

    fn apply(&self, x: f64, y: f64) -> [f64; 2] {
        [self.lin[0] * x + self.lin[1] * y + self.offset[0], self.lin[2] * x + self.lin[3] * y + self.offset[1]]
    }

    /// The device pixels an outline within `b` (document pixels `[x0, y0, x1, y1]`, inclusive)
    /// can mark: a segment's ends round to their nearest pixel, plus one on each side for float
    /// error.
    fn reach(&self, b: [i32; 4]) -> Px {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])] {
            let [px, py] = self.apply(f64::from(x), f64::from(y));
            (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
        }
        Px::new(
            (x0.floor() as i64).saturating_sub(1),
            (y0.floor() as i64).saturating_sub(1),
            (x1.ceil() as i64).saturating_add(2),
            (y1.ceil() as i64).saturating_add(2),
        )
    }
}

/// One view's ants texture and what it was made for.
#[derive(Default)]
struct Cache {
    /// The outline (held weakly: no other outline can take its allocation while this compares
    /// addresses) and its bounds in document pixels (`None`: no segments).
    outline: Weak<Vec<Segment>>,
    bounds: Option<[i32; 4]>,
    /// That outline's mask, once made.
    raster: Option<Raster>,
    texture: Option<TextureHandle>,
}

/// A mask of the outline and the view mapping it was rasterized at.
struct Raster {
    map: Mapping,
    /// The mask's pixels, as device pixels at `map` (the window's pixels when it was made).
    rect: Px,
    /// 1 for a pixel on the outline, row by row.
    mask: Vec<u8>,
    /// Whether the texture holds this mask, the march step it is coloured for and the part (in
    /// mask pixels) coloured at that step.
    uploaded: bool,
    phase: i64,
    fresh: Px,
    /// The offset of the last frame and when it last changed, to tell a view at rest.
    seen: [f64; 2],
    moved: f64,
}

impl Raster {
    /// The whole device pixels to move this mask by to show the outline at `map`, when it serves:
    /// the same view but for the pan, covering `need` (device pixels at `map`), and not a
    /// fractional pan come to rest. `None`: rasterize again.
    fn shift_for(&mut self, map: &Mapping, need: Px, time: f64) -> Option<[i64; 2]> {
        if self.map.lin != map.lin {
            return None;
        }
        if self.seen != map.offset {
            self.seen = map.offset;
            self.moved = time;
        }
        let d = [map.offset[0] - self.map.offset[0], map.offset[1] - self.map.offset[1]];
        let shift = [d[0].round() as i64, d[1].round() as i64];
        let fraction = (d[0] - shift[0] as f64).abs().max((d[1] - shift[1] as f64).abs());
        if fraction > 1e-3 && time - self.moved >= SETTLE {
            return None;
        }
        self.rect.contains(need.shift([shift[0].saturating_neg(), shift[1].saturating_neg()])).then_some(shift)
    }
}

type Shared = Arc<Mutex<Cache>>;

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
    let shown = Px::covering(area, ppp);
    let side = ctx.input(|i| i.max_texture_side).min(MAX_SIDE) as i64;
    if shown.w() > side || shown.h() > side {
        return false;
    }
    let Some(map) = Mapping::of(xf, ppp) else { return true };
    let shared = ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(cache_id(view)).clone());
    let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
    let cache = &mut *guard;
    if !std::ptr::eq(cache.outline.as_ptr(), Arc::as_ptr(segs)) {
        cache.outline = Arc::downgrade(segs);
        cache.bounds = bounds(segs);
        cache.raster = None;
    }
    let Some(b) = cache.bounds else { return true };
    let reach = map.reach(b);
    let need = shown.intersect(reach);
    if need.is_empty() {
        return true;
    }
    let shift = match cache.raster.as_mut().and_then(|r| r.shift_for(&map, need, time)) {
        Some(shift) => shift,
        None => {
            let rect = need.expand(MARGIN.min((side - need.w()) / 2).max(0), MARGIN.min((side - need.h()) / 2).max(0)).intersect(reach);
            let mask = rasterize(&map, segs, rect);
            cache.raster = Some(Raster { map, rect, mask, uploaded: false, phase: 0, fresh: Px::default(), seen: map.offset, moved: time });
            [0, 0]
        }
    };
    let Some(r) = cache.raster.as_mut() else { return true };
    let size = [r.rect.w() as usize, r.rect.h() as usize];
    let whole = Px::new(0, 0, r.rect.w(), r.rect.h());
    let origin = [r.rect.x0, r.rect.y0];
    // The canvas shown, in mask pixels.
    let on_canvas = need.shift([shift[0].saturating_add(origin[0]).saturating_neg(), shift[1].saturating_add(origin[1]).saturating_neg()]).intersect(whole);
    let phase = march_step(time);
    match cache.texture.as_mut() {
        Some(t) if r.uploaded => {
            // The march recolours the part shown; a pan colours what it brings into view.
            let parts = if r.phase != phase { vec![on_canvas] } else { on_canvas.minus(r.fresh) };
            for p in &parts {
                t.set_partial([p.x0 as usize, p.y0 as usize], colour(&r.mask, size[0], *p, origin, phase), TextureOptions::NEAREST);
            }
            if !parts.is_empty() {
                (r.phase, r.fresh) = (phase, on_canvas);
            }
        }
        t => {
            let image = colour(&r.mask, size[0], whole, origin, phase);
            match t {
                Some(t) => t.set(image, TextureOptions::NEAREST),
                None => cache.texture = Some(ctx.load_texture("pc-ants-raster", image, TextureOptions::NEAREST)),
            }
            (r.uploaded, r.phase, r.fresh) = (true, phase, whole);
        }
    }
    if let Some(t) = &cache.texture {
        // One texture pixel per device pixel, on whole device pixels.
        let min = pos2(origin[0].saturating_add(shift[0]) as f32 / ppp, origin[1].saturating_add(shift[1]) as f32 / ppp);
        let rect = Rect::from_min_size(min, vec2(size[0] as f32 / ppp, size[1] as f32 / ppp));
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

/// The bounds of `segs`' ends, `[x0, y0, x1, y1]` inclusive; `None` for no segments.
fn bounds(segs: &[Segment]) -> Option<[i32; 4]> {
    let mut ends = segs.iter().flat_map(|(a, b)| [*a, *b]);
    let first = ends.next()?;
    Some(ends.fold([first[0], first[1], first[0], first[1]], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]))
}

/// The outline's pixels in `rect` (device pixels at `map`), row by row: each segment as the mesh
/// draws it, one pixel wide from end to end inclusive.
fn rasterize(map: &Mapping, segs: &[Segment], rect: Px) -> Vec<u8> {
    let (w, h) = (rect.w(), rect.h());
    let width = w as usize;
    let mut mask = vec![0u8; width.saturating_mul(h as usize)];
    if mask.is_empty() {
        return mask;
    }
    // Pixel centres, so a line on a pixel boundary rounds as the mesh rounds it.
    let bounds = Rect::from_min_max(Pos2::ZERO, pos2(w as f32 - 0.5, h as f32 - 0.5)).expand(1.0);
    let (ox, oy) = (rect.x0 as f64, rect.y0 as f64);
    let to_px = |p: [i32; 2]| {
        let [x, y] = map.apply(f64::from(p[0]), f64::from(p[1]));
        pos2((x - ox) as f32, (y - oy) as f32)
    };
    let mut set = |x: i64, y: i64| {
        if (0..w).contains(&x)
            && (0..h).contains(&y)
            && let Some(m) = mask.get_mut(y as usize * width + x as usize)
        {
            *m = 1;
        }
    };
    for (a, b) in segs {
        let (pa, pb) = (to_px(*a), to_px(*b));
        if !(pa.is_finite() && pb.is_finite()) {
            continue;
        }
        // Much of a traced region lies off the mask: skip it before clipping.
        if pa.x.max(pb.x) < bounds.min.x || pa.x.min(pb.x) > bounds.max.x || pa.y.max(pb.y) < bounds.min.y || pa.y.min(pb.y) > bounds.max.y {
            continue;
        }
        let Some((pa, pb)) = crate::canvas::clip_line(bounds, pa, pb) else { continue };
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

/// The ants image of `part` (mask pixels) of a mask `width` pixels wide whose top-left is device
/// pixel `origin`, at march step `phase`: outline pixels white, black where (x + y + phase) falls
/// in the first half of each 2 × DASH run, the rest transparent.
fn colour(mask: &[u8], width: usize, part: Px, origin: [i64; 2], phase: i64) -> ColorImage {
    let (w, h) = (part.w() as usize, part.h() as usize);
    let run = (2 * DASH) as usize;
    let mut pixels = Vec::with_capacity(w.saturating_mul(h));
    for y in part.y0..part.y1 {
        let start = (y as usize).saturating_mul(width).saturating_add(part.x0 as usize);
        let row = mask.get(start..start.saturating_add(w)).unwrap_or_default();
        let first = origin[0].saturating_add(part.x0).saturating_add(origin[1]).saturating_add(y).saturating_add(phase).rem_euclid(2 * DASH) as usize;
        pixels.extend(row.iter().enumerate().map(|(i, &m)| match m {
            0 => Color32::TRANSPARENT,
            _ if (first + i) % run < DASH as usize => Color32::BLACK,
            _ => Color32::WHITE,
        }));
        // A row past the mask (never, but keep the image whole) stays clear.
        pixels.resize(pixels.len() + (w - row.len()), Color32::TRANSPARENT);
    }
    ColorImage::new([w, h], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xf(zoom: f32) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(40.0, 30.0)), zoom, center: [20.0, 15.0], flip: false, rotation: 0.0, aspect: 1.0 }
    }

    fn map(xf: &ViewXform, ppp: f32) -> Mapping {
        Mapping::of(xf, ppp).unwrap()
    }

    /// A speckle: the outlines of one-pixel squares scattered over `w` × `h` document pixels.
    fn speckle(w: i32, h: i32) -> Vec<Segment> {
        let mut segs = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if (x * 7 + y * 13 + x * y) % 5 == 0 {
                    segs.extend([([x, y], [x + 1, y]), ([x + 1, y], [x + 1, y + 1]), ([x + 1, y + 1], [x, y + 1]), ([x, y + 1], [x, y])]);
                }
            }
        }
        segs
    }

    /// A square outline at 100 %: its four sides, end to end, and nothing else.
    #[test]
    fn a_square_outline_marks_its_sides() {
        let segs: Vec<Segment> = vec![([5, 5], [10, 5]), ([10, 5], [10, 10]), ([10, 10], [5, 10]), ([5, 10], [5, 5])];
        let m = rasterize(&map(&xf(1.0), 1.0), &segs, Px::new(0, 0, 40, 30));
        let on = |x: usize, y: usize| m[y * 40 + x] == 1;
        for i in 5..=10 {
            assert!(on(i, 5) && on(i, 10) && on(5, i) && on(10, i), "side pixel {i}");
        }
        assert!(!on(7, 7), "the inside stays clear");
        assert_eq!(m.iter().filter(|v| **v == 1).count(), 20, "only the sides");
        // The same square in a mask whose corner is device pixel (4, 3).
        let part = rasterize(&map(&xf(1.0), 1.0), &segs, Px::new(4, 3, 12, 12));
        assert_eq!(part[(5 - 3) * 8 + (5 - 4)], 1);
        assert_eq!(part.iter().filter(|v| **v == 1).count(), 20);
    }

    /// Segments partly or wholly off the canvas, or at hostile coordinates, never panic and mark
    /// only what is on it.
    #[test]
    fn segments_off_the_canvas_are_clipped() {
        let segs: Vec<Segment> = vec![([-1_000_000, 3], [1_000_000, 3]), ([i32::MIN, i32::MIN], [i32::MAX, i32::MAX]), ([100, 100], [200, 100])];
        let m = rasterize(&map(&xf(1.0), 1.0), &segs, Px::new(0, 0, 40, 30));
        assert!((0..40).all(|x| m[3 * 40 + x] == 1), "the long line crosses the canvas");
        assert!(Mapping::of(&xf(f32::NAN), 1.0).is_none(), "no mapping, nothing drawn");
        assert!(rasterize(&map(&xf(1.0), 1.0), &segs, Px::new(0, 0, 0, 0)).is_empty());
        assert!(rasterize(&map(&xf(1.0), 1.0), &segs, Px::new(5, 5, 2, 2)).is_empty(), "an inside-out rect");
        let far = map(&xf(1.0), 1.0).reach([i32::MIN, i32::MIN, i32::MAX, i32::MAX]);
        assert!(far.x0 < far.x1 && far.y0 < far.y1);
        assert_eq!(rasterize(&map(&xf(1e30), 1.0), &segs, Px::new(0, 0, 40, 30)).len(), 1200);
    }

    /// The dashes: four black, four white along x + y, moving one pixel per march step, and only
    /// on outline pixels; a part of the mask is coloured as the whole is there.
    #[test]
    fn dashes_alternate_and_march() {
        let mut mask = vec![1u8; 16];
        mask[3] = 0;
        let img = colour(&mask, 16, Px::new(0, 0, 16, 1), [0, 0], 0);
        let black: Vec<bool> = img.pixels.iter().map(|c| *c == Color32::BLACK).collect();
        assert_eq!(&black[..8], &[true, true, true, false, false, false, false, false]);
        assert_eq!(img.pixels[3], Color32::TRANSPARENT, "not on the outline");
        // One step on, the dashes have moved a pixel: x = 7 turns black.
        let next = colour(&mask, 16, Px::new(0, 0, 16, 1), [0, 0], 1);
        assert_eq!((img.pixels[7], next.pixels[7]), (Color32::WHITE, Color32::BLACK));
        assert_eq!(next.pixels[0], Color32::BLACK);
        // A mask whose corner is device pixel (-5, 2) phases by where its pixels are.
        let moved = colour(&mask, 16, Px::new(0, 0, 16, 1), [-5, 2], 0);
        assert_eq!(moved.pixels[4], img.pixels[1]);
        assert_eq!(march_step(f64::NAN), 0);
        assert!((0..8).contains(&march_step(1234.567)));
        // A 2-D mask: every part matches the whole image there.
        let mask: Vec<u8> = (0..12 * 9).map(|i| u8::from(i % 3 != 0)).collect();
        let whole = colour(&mask, 12, Px::new(0, 0, 12, 9), [7, -3], 5);
        let sub = colour(&mask, 12, Px::new(3, 2, 10, 8), [7, -3], 5);
        assert_eq!(sub.size, [7, 6]);
        for y in 0..6 {
            for x in 0..7 {
                assert_eq!(sub.pixels[y * 7 + x], whole.pixels[(y + 2) * 12 + x + 3], "({x}, {y})");
            }
        }
        // A part reaching past the mask comes back whole, the missing pixels clear.
        let past = colour(&mask, 12, Px::new(10, 7, 14, 11), [0, 0], 0);
        assert_eq!((past.size, past.pixels.len()), ([4, 4], 16));
        assert_eq!(past.pixels[15], Color32::TRANSPARENT);
    }

    /// The device-pixel mapping is the view's (`to_screen` × display scale) for any zoom,
    /// rotation, flip and aspect, and a pan changes only its offset.
    #[test]
    fn the_mapping_is_the_views_and_a_pan_only_moves_it() {
        for (zoom, rotation, flip, aspect, ppp) in
            [(1.0, 0.0, false, 1.0, 1.0), (0.37, 0.0, true, 1.0, 1.25), (3.0, 30.0, false, 2.0, 1.5), (0.5, -90.0, true, 0.5, 2.0)]
        {
            let mut v =
                ViewXform { rect: Rect::from_min_size(pos2(13.0, 7.0), egui::vec2(640.0, 480.0)), zoom, center: [812.5, -40.25], flip, rotation, aspect };
            let m = map(&v, ppp);
            for (x, y) in [(0, 0), (812, -40), (-300, 1200), (5000, 4000)] {
                let s = v.to_screen(x as f32, y as f32) * ppp;
                let [px, py] = m.apply(f64::from(x), f64::from(y));
                let close = |a: f32, b: f64| (f64::from(a) - b).abs() < 1e-3 + b.abs() * 1e-6;
                assert!(close(s.x, px) && close(s.y, py), "{zoom} {rotation}° ({x}, {y}): {s:?} vs {px}, {py}");
            }
            v.center = [v.center[0] + 31.0, v.center[1] - 7.5];
            v.rect = v.rect.translate(egui::vec2(4.0, 0.0));
            let panned = map(&v, ppp);
            assert_eq!(panned.lin, m.lin, "a pan keeps the linear part");
            assert_ne!(panned.offset, m.offset);
        }
    }

    /// The core of it: the mask made before a pan by whole device pixels, moved by that pan, is
    /// the mask made after it, so the ants look as they do rasterized for every frame.
    #[test]
    fn a_mask_moved_by_a_pan_is_the_mask_made_after_it() {
        let segs = speckle(60, 60);
        for (zoom, rotation, flip, aspect, ppp) in
            [(1.0, 0.0, false, 1.0, 1.0), (0.731, 0.0, false, 1.0, 1.0), (2.5, 0.0, true, 1.0, 1.25), (1.3, 0.0, false, 2.0, 1.0), (1.1, 30.0, false, 1.0, 1.0)]
        {
            let v0 = ViewXform { rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(120.0, 90.0)), zoom, center: [30.137, 29.71], flip, rotation, aspect };
            let m0 = map(&v0, ppp);
            // Pan by (17, -9) device pixels: the centre moves by lin⁻¹ · (-17, 9).
            let l = m0.lin;
            let det = l[0] * l[3] - l[1] * l[2];
            let dc = [(l[3] * -17.0 - l[1] * 9.0) / det, (-l[2] * -17.0 + l[0] * 9.0) / det];
            let mut v1 = v0;
            v1.center = [(f64::from(v0.center[0]) + dc[0]) as f32, (f64::from(v0.center[1]) + dc[1]) as f32];
            let m1 = map(&v1, ppp);
            let d = [m1.offset[0] - m0.offset[0], m1.offset[1] - m0.offset[1]];
            assert!((d[0] - 17.0).abs() < 1e-3 && (d[1] + 9.0).abs() < 1e-3, "{d:?}");
            let r0 = Px::new(-40, -30, 200, 160);
            let before = rasterize(&m0, &segs, r0);
            let after = rasterize(&m1, &segs, r0.shift([17, -9]));
            let differ = before.iter().zip(&after).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "zoom {zoom} {rotation}° flip {flip} aspect {aspect}: {differ} pixels differ");
            assert!(before.iter().filter(|v| **v == 1).count() > 500, "the speckle is on the mask");
        }
    }

    #[test]
    fn pixel_rects_cover_intersect_and_subtract() {
        assert_eq!(Px::covering(Rect::from_min_max(pos2(10.2, 3.0), pos2(20.0, 7.6)), 1.0), Px::new(10, 3, 20, 8));
        assert_eq!(Px::covering(Rect::from_min_max(pos2(8.0, 4.0), pos2(16.0, 8.0)), 1.25), Px::new(10, 5, 20, 10));
        let a = Px::new(0, 0, 10, 10);
        assert!(a.contains(Px::new(2, 2, 10, 10)) && !a.contains(Px::new(2, 2, 11, 10)) && a.contains(Px::new(50, 50, 40, 40)));
        assert!(a.intersect(Px::new(10, 0, 20, 10)).is_empty());
        assert_eq!(a.minus(Px::new(-5, -5, 20, 20)), vec![]);
        assert_eq!(a.minus(Px::new(20, 20, 30, 30)), vec![a]);
        // A diagonal pan uncovers an L: every pixel outside counted once.
        let parts = a.minus(Px::new(-3, -2, 7, 8));
        let count = |x: i64, y: i64| parts.iter().filter(|p| p.contains(Px::new(x, y, x + 1, y + 1))).count();
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(count(x, y), usize::from(!(x < 7 && y < 8)), "({x}, {y})");
            }
        }
        assert_eq!(Px::new(i64::MAX - 1, 0, i64::MAX, 1).shift([5, 0]).x1, i64::MAX);
        assert_eq!(bounds(&[([3, -2], [7, -2]), ([-1, 5], [-1, 9])]), Some([-1, -2, 7, 9]));
        assert_eq!(bounds(&[]), None);
    }

    /// Uploads to `ctx`'s textures since the last call: (whole textures, pixels of partial ones).
    fn uploads(ctx: &egui::Context) -> (usize, usize) {
        let mut delta = ctx.tex_manager().write().take_delta();
        let mut out = (0, 0);
        for d in delta.set.values().flatten() {
            match d.pos {
                None => out.0 += 1,
                Some(_) => out.1 += d.image.width() * d.image.height(),
            }
        }
        delta.clear();
        out
    }

    /// Pans by whole pixels move the texture (no new mask, only what comes into view is
    /// coloured); a fractional pan rasterizes again once it rests; so do a new outline, zoom,
    /// rotation, flip, aspect or display scale and a pan past the margin. The march recolours
    /// the part shown.
    #[test]
    fn a_pan_moves_the_texture_and_the_rest_rasterizes_again() {
        let ctx = egui::Context::default();
        let canvas = Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 100.0));
        let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), canvas);
        let segs = Arc::new(speckle(2000, 1400));
        let id = egui::Id::new("ants-test");
        let mut v = ViewXform { rect: canvas, zoom: 1.0, center: [1000.0, 700.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let raster = || {
            let shared = ctx.data_mut(|d| d.get_temp::<Shared>(cache_id(id))).unwrap();
            let c = shared.lock().unwrap();
            c.raster.as_ref().map(|r| (r.rect, r.map))
        };
        uploads(&ctx); // egui's own (the font atlas)
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.0));
        assert_eq!(uploads(&ctx).0, 1, "the first frame uploads the mask");
        let (rect, m0) = raster().unwrap();
        assert_eq!((rect.w(), rect.h()), (200 + 2 * MARGIN, 100 + 2 * MARGIN), "the canvas and the margins");
        // Three whole pixels right: the texture moves.
        v.center[0] -= 3.0;
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.01));
        assert_eq!(uploads(&ctx), (0, 0), "the margins were coloured with the mask");
        assert_eq!(raster().unwrap().1, m0, "the same mask");
        // The march recolours only the canvas shown.
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.15));
        assert_eq!(uploads(&ctx), (0, 200 * 100));
        v.center[1] += 2.0;
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.16));
        assert_eq!(uploads(&ctx), (0, 200 * 2), "the band the pan brought in");
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.17));
        assert_eq!(uploads(&ctx), (0, 0), "nothing new");
        // Half a pixel: moved to a whole pixel while panning, rasterized again at rest.
        v.center[0] -= 0.5;
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.18));
        assert_eq!(uploads(&ctx).0, 0);
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.18 + SETTLE / 2.0));
        assert_eq!(uploads(&ctx).0, 0, "not at rest yet");
        assert!(draw(&ctx, &painter, id, &v, &segs, 0.18 + SETTLE * 1.5));
        assert_eq!(uploads(&ctx).0, 1, "at rest: rasterized where the view puts it");
        let (_, m1) = raster().unwrap();
        assert!((m1.offset[0] - m0.offset[0] - 3.5).abs() < 1e-6);
        // Past the margin.
        v.center[0] += MARGIN as f32 + 10.0;
        assert!(draw(&ctx, &painter, id, &v, &segs, 1.0));
        assert_eq!(uploads(&ctx).0, 1, "a pan past the margin");
        // Zoom, rotation, flip and aspect each make a new mask, which the next frame reuses.
        for change in 0..4 {
            let mut w = v;
            match change {
                0 => w.zoom = 1.5,
                1 => w.rotation = 15.0,
                2 => w.flip = true,
                _ => w.aspect = 0.5,
            }
            assert!(draw(&ctx, &painter, id, &w, &segs, 1.01));
            assert_eq!(uploads(&ctx).0, 1, "change {change}");
            assert!(draw(&ctx, &painter, id, &w, &segs, 1.02));
            assert_eq!(uploads(&ctx).0, 0, "change {change}, then reused");
        }
        let retraced = Arc::new(speckle(2000, 1400));
        assert!(draw(&ctx, &painter, id, &v, &retraced, 1.03));
        assert_eq!(uploads(&ctx).0, 1, "a new outline");
        ctx.input_mut(|i| i.pixels_per_point = 1.25);
        assert!(draw(&ctx, &painter, id, &v, &retraced, 1.04));
        assert_eq!(uploads(&ctx).0, 1, "another display scale");
        // A canvas bigger than a texture can be keeps the mesh; an outline out of view draws nothing.
        ctx.input_mut(|i| i.max_texture_side = 100);
        assert!(!draw(&ctx, &painter, id, &v, &retraced, 1.05));
        ctx.input_mut(|i| i.max_texture_side = 2048);
        v.center = [-5000.0, -5000.0];
        assert!(draw(&ctx, &painter, id, &v, &retraced, 1.06));
        assert_eq!(uploads(&ctx), (0, 0));
        assert!(draw(&ctx, &painter, id, &v, &Arc::new(Vec::new()), 1.07));
        clear(&ctx, id);
        assert!(ctx.data_mut(|d| d.get_temp::<Shared>(cache_id(id))).is_none());
    }
}
