//! Dab rasterisation and stroke compositing.
//!
//! A stroke paints into a sparse, tiled *stroke buffer* (coverage plus, with Color Dynamics, a
//! premultiplied colour), exactly like a private stroke layer: dabs build up with flow up to their
//! opacity ceiling, stroke-level masks (Dual Brush, Texture) apply when the buffer is composited,
//! and the whole buffer composites onto the pre-stroke pixels at the stroke opacity. Because the
//! buffer is sparse and compositing always starts from the pre-stroke pixels, strokes can be fed in
//! chunks and re-composited incrementally for interactive painting (see [`StrokeRenderer`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};

use crate::brush::{BrushSettings, MaskMode, Pattern, TipShape};
use crate::dynamics::DabGenerator;
use crate::retouch::{Footprint, alpha_index, over_native};
use crate::rng::hash2;
use crate::tile::{Mips, PatternImage};
use crate::{Dab, StrokePoint, tip_reach};

/// Salt of the 8-bit dither's per-pixel noise.
const DITHER_SALT: u64 = 0x0000_6469_7468_6572;

/// Before a stroke's result is quantised to 8 bits, offset every channel of the pixel by the same
/// deterministic noise of under half a level, so fractional values round up or down in proportion
/// (a soft, low-flow stroke's fine gradient survives as grain instead of bands). A value already on
/// an 8-bit level still rounds back to itself, and because the noise depends only on the pixel's
/// position, the live preview, the commit and a replay all agree.
#[inline]
fn dither8(px: &mut [f32], x: i32, y: i32) {
    let d = (hash2(x, y, DITHER_SALT) - 0.5) / 255.0;
    for v in px.iter_mut() {
        *v += d;
    }
}

/// Edge of a stroke-buffer tile.
pub const COV_TILE: i32 = 64;
/// Computed tips with a smaller radius (in pixels) are rasterised by area, not at pixel centres.
const SMALL_TIP_RADIUS: f32 = 3.0;
/// Sub-pixel sample offsets (a 4×4 grid) for the area coverage of small tips.
const SUBPIXEL: [f32; 4] = [-0.375, -0.125, 0.125, 0.375];
const NOISE_SALT: u64 = 0x006E_6F69_7365;

#[inline]
/// Where an aliased (Pencil) dab of `diameter` pixels centred near (`x`, `y`) lands on the pixel
/// grid: the centre of the pixel holding the point for odd diameters, the nearest pixel corner
/// for even ones. Its footprint is then a whole-pixel block around that point, the same square
/// the Pencil cursor shows ([`grid_square`]).
pub fn grid_center(x: f64, y: f64, diameter: f32) -> (f32, f32) {
    let n = if diameter.is_finite() { diameter.round().max(1.0) as i64 } else { 1 };
    let snap = |v: f64| if n % 2 == 1 { v.floor() + 0.5 } else { v.round() };
    (snap(x) as f32, snap(y) as f32)
}

/// The whole-pixel square `[x0, y0, x1, y1]` an aliased dab of `diameter` pixels at (`x`, `y`)
/// can touch (see [`grid_center`]): the Pencil's cursor.
pub fn grid_square(x: f64, y: f64, diameter: f32) -> [f64; 4] {
    let n = if diameter.is_finite() { f64::from(diameter.round().clamp(1.0, 100_000.0)) } else { 1.0 };
    let (cx, cy) = grid_center(x, y, n as f32);
    let h = n / 2.0;
    [f64::from(cx) - h, f64::from(cy) - h, f64::from(cx) + h, f64::from(cy) + h]
}

/// Wet Edges (measured against Photoshop 2026): a curve on the stroke's coverage, after Texture
/// and the Dual Brush and before opacity. Up to half coverage the paint gains half again; above
/// that it eases back to 59 % (150/255) where the stroke is solid. A soft edge rings, a hard
/// stroke goes see-through with only its antialiased rim darker, and a dual or texture mask's
/// own edges ring too.
#[inline]
pub fn wet_edges(m: f32) -> f32 {
    if !m.is_finite() {
        return 0.0;
    }
    let m = m.clamp(0.0, 1.0);
    if m <= 0.5 { 1.5 * m } else { 0.75 - 0.65 * (m - 0.5) * (m - 0.5) }
}

/// Photoshop's Overlay of coverage `v` with a mask value `t`.
#[inline]
fn overlay(v: f32, t: f32) -> f32 {
    if v < 0.5 { 2.0 * v * t } else { 1.0 - 2.0 * (1.0 - v) * (1.0 - t) }
}

/// How far a Height-mode dab at full depth and coverage reaches into the pattern, in pattern
/// ranges (measured against Photoshop, #3340).
const HEIGHT_REACH: f32 = 12.0;

/// The share of the depth Color Dodge and Color Burn use (Photoshop 2026, 2026-10-17: fitted
/// over single dabs at several flows and depths, Texture and Dual Brush alike; 248/255 cuts the
/// error 2–10× against full depth, and 246/255 or 250/255 are worse).
const DIVIDE_DEPTH: f32 = 248.0 / 255.0;

/// Combine brush coverage `v` with a Texture pattern value `t` at `depth` (Brush Settings ›
/// Texture). Every mode maps `v = 0` to 0 (a mask never adds paint where the brush has none).
/// Measured against Photoshop with single dabs at several flows and depths (#3340, and
/// 2026-10-17 for Hard Mix and Linear Burn):
/// - Subtract takes `depth × t` away, so a white pattern removes the most paint.
/// - Color Dodge and Color Burn move the pattern toward their neutral colour as depth falls, and
///   never reach the end of it: they act at 248/255 of the depth (measured 2026-10-17), so at
///   full depth an empty pattern or dual tip still leaves paint above ~97 % (98 % keeps 73/255,
///   99 % keeps 146/255) instead of wiping everything short of solid.
/// - Linear Burn takes `depth × (1 − t)` away.
/// - Hard Mix is a steep ramp, `4v − 3 + 3·depth·t`: full paint stays, half paint survives
///   only on the pattern's light half, and lower depths need lighter pattern.
/// - Height treats the pattern as a height field the brush presses into: the paint reaches
///   `12 × depth × v` and fills whatever lies below that, so a low depth paints only the dark
///   valleys, from about 17 % the whole pattern is under the paint, and a brighter pattern
///   takes paint away.
/// - Linear Height is Height, but where the reach stays below the top of the pattern the paint
///   fades with the pattern instead of being cut off.
pub fn texture_combine(mode: MaskMode, v: f32, t: f32, d: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let r = match mode {
        MaskMode::Multiply => v * (1.0 - d + d * t),
        MaskMode::Subtract => v - d * t,
        MaskMode::Darken => v.min(1.0 - d + d * t),
        MaskMode::Overlay => v + (overlay(v, t) - v) * d,
        MaskMode::ColorDodge => v / (1.0 - d * DIVIDE_DEPTH * t).max(1e-3),
        MaskMode::ColorBurn => {
            let d = d * DIVIDE_DEPTH;
            1.0 - (1.0 - v) / (1.0 - d + d * t).max(1e-3)
        }
        MaskMode::LinearBurn => v - d * (1.0 - t),
        MaskMode::HardMix => 4.0 * v - 3.0 + 3.0 * d * t,
        // Photoshop steps the depth in 1/255ths here, which shows at the lowest depths.
        MaskMode::Height => HEIGHT_REACH * (d * 255.0).round() / 255.0 * v - t,
        MaskMode::LinearHeight => {
            let reach = HEIGHT_REACH * (d * 255.0).round() / 255.0 * v;
            if reach < 1.0 { reach * (1.0 - t) } else { reach - t }
        }
    };
    if r.is_finite() { r.clamp(0.0, 1.0) } else { 0.0 }
}

/// Combine the stroke's coverage `v` with the Dual Brush's coverage `t` (Brush Settings › Dual
/// Brush, measured against Photoshop 2026). The modes act like Texture at full depth, with the
/// dual stroke as the pattern: Multiply, Darken and Linear Burn keep paint only under the dual
/// tip, while Overlay, Color Dodge, Color Burn and Hard Mix keep a solid stroke whole and only
/// work on its partial paint. Linear Height is Overlay with the dual tip inverted.
pub fn dual_combine(mode: MaskMode, v: f32, t: f32) -> f32 {
    match mode {
        MaskMode::LinearHeight if v > 0.0 => {
            let r = overlay(v, 1.0 - t);
            if r.is_finite() { r.clamp(0.0, 1.0) } else { 0.0 }
        }
        _ => texture_combine(mode, v, t, 1.0),
    }
}

/// A stroke-ready brush: the settings plus prepared tip mips and texture.
#[derive(Clone, Debug)]
pub struct BrushContext {
    pub brush: BrushSettings,
    tip: Option<Arc<Mips>>,
    dual_tip: Option<Arc<Mips>>,
    texture: Option<Arc<PatternImage>>,
}

fn prepare_pattern(b: &BrushSettings) -> Option<Arc<PatternImage>> {
    let t = &b.texture;
    if !t.enabled {
        return None;
    }
    let mut img = match &t.pattern {
        Pattern::Procedural { style, size, seed } => crate::procedural::pattern(*style, *size, *seed),
        p => match p.bitmap() {
            Some(g) if g.is_valid() => PatternImage { width: g.width as usize, height: g.height as usize, data: g.to_f32() },
            _ => return None,
        },
    };
    for v in &mut img.data {
        *v = t.level(*v);
    }
    Some(Arc::new(img))
}

/// A dab's pixel offset → tip-space map as a matrix: `u = m[0]·dx + m[1]·dy`,
/// `v = m[2]·dx + m[3]·dy` (to y-up, Brush Projection, rotate by −angle, flip), the linear map
/// `rasterize` applies to each pixel.
fn tip_matrix(d: &Dab, dual: bool) -> [f32; 4] {
    let (sn, cs) = d.angle.sin_cos();
    let (fx, fy) = (if d.flip_x { -1.0 } else { 1.0 }, if d.flip_y { -1.0 } else { 1.0 });
    let proj = (!dual && d.proj_scale < 0.999).then(|| {
        let (ps, pc) = d.proj_angle.sin_cos();
        (pc, ps, 1.0 / d.proj_scale.max(0.05) - 1.0)
    });
    let to_tip = |dx: f32, dy: f32| {
        let (mut ux, mut uy) = (dx, -dy);
        if let Some((ax, ay, k)) = proj {
            let t = (ux * ax + uy * ay) * k;
            ux += t * ax;
            uy += t * ay;
        }
        ((ux * cs + uy * sn) * fx, (-ux * sn + uy * cs) * fy)
    };
    let ((m0, m2), (m1, m3)) = (to_tip(1.0, 0.0), to_tip(0.0, 1.0));
    [m0, m1, m2, m3]
}

/// The pixel rectangle `reach` (plus 1 px of anti-aliasing slack) around a dab's centre. The casts
/// saturate and the arithmetic saturates, so a far or non-finite centre or reach from a direct
/// caller gives a (possibly empty) rectangle instead of an `i32` overflow (#977).
pub(crate) fn rect_around(d: &Dab, reach: f32) -> Rect {
    let rr = (reach.ceil() as i32).saturating_add(1);
    let (cx, cy) = (d.center.x.floor() as i32, d.center.y.floor() as i32);
    Rect::new(cx.saturating_sub(rr), cy.saturating_sub(rr), cx.saturating_add(rr).saturating_add(1), cy.saturating_add(rr).saturating_add(1))
}

impl BrushContext {
    pub fn new(brush: &BrushSettings) -> Self {
        let brush = brush.bounded_for_render();
        let mips = |t: &TipShape| match t.bitmap() {
            Some(g) if g.is_valid() => Some(Arc::new(Mips::new(g))),
            _ => None,
        };
        Self {
            tip: mips(&brush.tip),
            dual_tip: if brush.dual_brush.enabled { mips(&brush.dual_brush.tip) } else { None },
            texture: prepare_pattern(&brush),
            brush,
        }
    }

    /// Texture value at a document pixel (bilinear, tiled).
    #[inline]
    pub fn texture_at(&self, x: i32, y: i32) -> f32 {
        let Some(p) = &self.texture else { return 1.0 };
        let s = self.brush.texture.scale.max(0.01);
        if s == 1.0 {
            // Pixel centres land exactly on texels at 100 %.
            return p.data[y.rem_euclid(p.height as i32) as usize * p.width + x.rem_euclid(p.width as i32) as usize];
        }
        let (fx, fy) = ((x as f32 + 0.5) / s - 0.5, (y as f32 + 0.5) / s - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let a = p.sample_wrap(x0, y0) + (p.sample_wrap(x0 + 1.0, y0) - p.sample_wrap(x0, y0)) * tx;
        let b = p.sample_wrap(x0, y0 + 1.0) + (p.sample_wrap(x0 + 1.0, y0 + 1.0) - p.sample_wrap(x0, y0 + 1.0)) * tx;
        a + (b - a) * ty
    }

    /// Bounding rectangle of a dab (rotated sampled tips need the corner reach).
    pub fn dab_rect(&self, d: &Dab, dual: bool) -> Rect {
        let sampled = if dual { self.dual_tip.is_some() } else { self.tip.is_some() };
        let hardness = if dual { self.brush.dual_brush.hardness } else { self.brush.hardness };
        // A computed soft tip paints past its nominal radius (see `tip_shape`).
        let aliased = self.brush.aliased && !dual;
        let reach = if sampled {
            d.radius * std::f32::consts::SQRT_2
        } else if aliased {
            d.radius
        } else {
            crate::tip_reach(d.radius, hardness).max(d.radius)
        };
        rect_around(d, reach)
    }

    /// [`dab_rect`](Self::dab_rect) cut down to the dab's footprint: a sampled tip's bitmap or a
    /// computed tip's ellipse at the dab's angle and roundness, where the square around the full
    /// radius (room for any angle) is mostly empty. Every pixel the dab paints over `dab_rect` is
    /// inside it.
    pub fn paint_rect(&self, d: &Dab, dual: bool) -> Rect {
        let full = self.dab_rect(d, dual);
        let b = &self.brush;
        let aliased = b.aliased && !dual;
        let mips = if dual { self.dual_tip.as_deref() } else { self.tip.as_deref() };
        let hardness = if dual { b.dual_brush.hardness } else { b.hardness };
        let r = d.radius;
        let ro = d.roundness.max(0.5 / r).min(1.0);
        let rm = (r * ro).max(0.5);
        if aliased || !(r.is_finite() && ro.is_finite() && ro > 0.0 && d.angle.is_finite()) {
            return full;
        }
        // Half extents of the footprint in tip space (u along the tip's width, v along its
        // height), with the rasteriser's own cut-offs and a little room for rounding.
        let (hu, hv) = match mips {
            Some(m) => {
                let (w0, h0, _) = &m.levels[0];
                let (tw, th) = (*w0 as f32, *h0 as f32);
                let inv_scale = tw.max(th) / (2.0 * r);
                // `−1 ≤ px ≤ tw + 1` with `px = u·inv_scale + tw/2`, likewise `py` with `v / ro`.
                ((tw / 2.0 + 1.0) / inv_scale, (th / 2.0 + 1.0) * ro / inv_scale)
            }
            None => {
                let reach = crate::tip_reach(rm, hardness).max(rm);
                // A tiny tip takes area samples a little further out (`small_reach2`).
                let rr = reach + if rm < SMALL_TIP_RADIUS { 0.75 } else { 0.5 };
                (rr / ro, rr)
            }
        };
        // Back to pixel offsets through the inverse of the pixel → tip-space map (rotation, flips
        // and Brush Projection, which skews a footprint that isn't round).
        let [a, bb, c, dd] = tip_matrix(d, dual);
        let det = a * dd - bb * c;
        if !(det.is_finite() && det.abs() > 1e-12) {
            return full;
        }
        let inv = [dd / det, -bb / det, -c / det, a / det];
        let (hx, hy) = match mips {
            Some(_) => (inv[0].abs() * hu + inv[1].abs() * hv, inv[2].abs() * hu + inv[3].abs() * hv),
            None => ((inv[0] * hu).hypot(inv[1] * hv), (inv[2] * hu).hypot(inv[3] * hv)),
        };
        if !(hx.is_finite() && hy.is_finite()) {
            return full;
        }
        let (cx, cy) = (d.center.x.floor() as i32, d.center.y.floor() as i32);
        let (rx, ry) = ((hx.ceil() as i32).saturating_add(2), (hy.ceil() as i32).saturating_add(2));
        Rect::new(cx.saturating_sub(rx), cy.saturating_sub(ry), cx.saturating_add(rx).saturating_add(1), cy.saturating_add(ry).saturating_add(1)).intersect(&full)
    }

    /// Rasterise a dab over `rect` into `out` (tip shape × noise × per-tip texture × flow). Wet
    /// Edges and the stroke-level masks act on a stroke's accumulated coverage
    /// ([`stroke_mask`](Self::stroke_mask)), never on one dab.
    pub fn rasterize(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>) {
        let (w, h) = (rect.width() as usize, rect.height() as usize);
        out.clear();
        out.resize(w * h, 0.0);
        let b = &self.brush;
        let (hardness, mips) = if dual { (b.dual_brush.hardness, self.dual_tip.as_deref()) } else { (b.hardness, self.tip.as_deref()) };
        let aliased = b.aliased && !dual;
        let noise = b.noise && !dual;
        let tex_tip = !dual && b.texture.enabled && b.texture.each_tip && self.texture.is_some();
        let (cx, cy) = if aliased { grid_center(d.center.x, d.center.y, 2.0 * d.radius) } else { (d.center.x as f32, d.center.y as f32) };
        let (sn, cs) = d.angle.sin_cos();
        let (fx, fy) = (if d.flip_x { -1.0 } else { 1.0 }, if d.flip_y { -1.0 } else { 1.0 });
        // Brush Projection: stretch the sampling coordinates along the tilt direction, which
        // foreshortens the tip there by `proj_scale`.
        let proj = (!dual && d.proj_scale < 0.999).then(|| {
            let (ps, pc) = d.proj_angle.sin_cos();
            (pc, ps, 1.0 / d.proj_scale.max(0.05) - 1.0)
        });
        let r = d.radius;
        let ro = d.roundness.max(0.5 / r).min(1.0);
        let rm = (r * ro).max(0.5);
        let reach = if aliased || mips.is_some() { rm } else { crate::tip_reach(rm, hardness).max(rm) };
        let reach2 = (reach + 0.5) * (reach + 0.5);
        // Sampled-tip mapping.
        let (level, inv_scale, tw, th) = match mips {
            Some(m) => {
                let (w0, h0, _) = &m.levels[0];
                let big = *w0.max(h0) as f32;
                (m.level_for(2.0 * r * ro.max(0.25)), big / (2.0 * r), *w0 as f32, *h0 as f32)
            }
            None => (0, 1.0, 1.0, 1.0),
        };
        // A pixel offset from the dab centre in tip space: to y-up, project, rotate by -angle, flip.
        let to_tip = |dx: f32, dy: f32| {
            let (mut ux, mut uy) = (dx, -dy);
            if let Some((ax, ay, k)) = proj {
                let t = (ux * ax + uy * ay) * k;
                ux += t * ax;
                uy += t * ay;
            }
            ((ux * cs + uy * sn) * fx, (-ux * sn + uy * cs) * fy)
        };
        // A tip a few pixels across takes each pixel's covered area (a 4×4 grid inside it):
        // sampling only the pixel centre made a tiny dab's total coverage swing with its sub-pixel
        // position, and thin lines beaded.
        let small = mips.is_none() && !aliased && rm < SMALL_TIP_RADIUS;
        let small_reach2 = (reach + 0.75) * (reach + 0.75);
        // Keep ordinary computed tips separate from the feature loop, so their dense rows
        // can be vectorised without the per-pixel sampled-mask branches.
        if !(mips.is_some() || small || (aliased && b.square) || noise || tex_tip)
            && proj.is_none()
            && d.angle == 0.0
            && d.radius.is_finite()
            && d.roundness.is_finite()
            && rm.is_finite()
            && cx.is_finite()
            && cy.is_finite()
            && hardness.is_finite()
            && d.alpha.is_finite()
            && w > 0
        {
            rasterize_round(d, rect, out, hardness, aliased);
            return;
        }
        // The round tip's falloff, worked out once for the dab (`dab_coverage` per pixel did it
        // for every pixel).
        let (r0, s) = crate::tip_shape(rm, hardness);
        let cut = crate::tip_reach(rm, hardness) + 0.5;
        // The columns of each row the tip can reach (a large dab's square has a lot of empty
        // corner); tiny and Block tips take the whole row.
        let span = match mips {
            _ if aliased && b.square => None,
            None if small => None,
            None => Some(Span::Ellipse { ro, r: reach + 0.5 }),
            Some(_) => Some(Span::Rect { inv_scale, ro, tw, th }),
        };
        // `to_tip` is linear: its matrix, for the spans.
        let ((m0, m2), (m1, m3)) = (to_tip(1.0, 0.0), to_tip(0.0, 1.0));
        let map = [m0, m1, m2, m3];
        let dx0 = rect.x0 as f32 + 0.5 - cx;
        let slack = span_slack(cx, cy, rect);
        // A plain sampled tip (most imported brushes): a loop of its own, without the features'
        // per-pixel branches and divisions.
        if let (Some(m), Some(sp)) = (mips, span)
            && !(aliased || noise || tex_tip)
            && map.iter().chain(&[cx, cy, inv_scale, ro, tw, th, d.alpha]).all(|v| v.is_finite())
            && ro > 0.0
        {
            let tip = SampledTip { mips: m, level, map, inv_scale, ro, tw, th, x0: rect.x0, cx, alpha: d.alpha };
            for (yy, row) in out.chunks_exact_mut(w.max(1)).enumerate().take(h) {
                let dy = (rect.y0 + yy as i32) as f32 + 0.5 - cy;
                let (from, to) = sp.columns(map, dx0, dy, slack, w);
                tip.row(from, to, dy, row);
            }
            return;
        }
        for yy in 0..h {
            let y = rect.y0 + yy as i32;
            let dy = y as f32 + 0.5 - cy;
            let (from, to) = span.map_or((0, w), |sp| sp.columns(map, dx0, dy, slack, w));
            for xx in from..to {
                let x = rect.x0 + xx as i32;
                let dx = x as f32 + 0.5 - cx;
                let (u, v) = to_tip(dx, dy);
                let mut val = match mips {
                    // Block: the whole-pixel square, hard (the Eraser's Block mode).
                    _ if aliased && b.square => {
                        if dx.abs() >= r || dy.abs() >= r {
                            continue;
                        }
                        1.0
                    }
                    None if small => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= small_reach2 {
                            continue;
                        }
                        let mut sum = 0.0;
                        for sy in SUBPIXEL {
                            for sx in SUBPIXEL {
                                let (su, sv) = to_tip(dx + sx, dy + sy);
                                let sd = ((su * ro).powi(2) + sv * sv).sqrt();
                                // Each sample takes the tip's own edge, so the covered area
                                // changes smoothly with the dab's position.
                                sum += crate::falloff_shaped(sd, r0, s);
                            }
                        }
                        sum / (SUBPIXEL.len() * SUBPIXEL.len()) as f32
                    }
                    None => {
                        let d2 = (u * ro).powi(2) + v * v;
                        if d2 >= reach2 {
                            continue;
                        }
                        let dd = d2.sqrt();
                        // `dab_coverage(dd, rm, hardness)`, its shape worked out above.
                        if aliased {
                            if dd <= rm { 1.0 } else { 0.0 }
                        } else if dd.is_nan() || dd >= cut {
                            0.0
                        } else {
                            crate::falloff_shaped(dd, r0, s)
                        }
                    }
                    Some(m) => {
                        let (px, py) = (u * inv_scale + tw / 2.0, -(v / ro) * inv_scale + th / 2.0);
                        if px < -1.0 || py < -1.0 || px > tw + 1.0 || py > th + 1.0 {
                            continue;
                        }
                        let s = m.sample_clamped(level, px / tw, py / th);
                        if aliased { if s >= 0.5 { 1.0 } else { 0.0 } } else { s }
                    }
                };
                if val <= 0.0 {
                    continue;
                }
                if noise && val < 1.0 {
                    let n = hash2(x, y, NOISE_SALT);
                    let hard = if n < val { 1.0 } else { 0.0 };
                    val += (hard - val) * 0.7;
                }
                if tex_tip {
                    val = texture_combine(b.texture.mode, val, self.texture_at(x, y), d.depth);
                }
                out[yy * w + xx] = val * d.alpha;
            }
        }
    }

    /// One dab's dense footprint (for the sequential retouch tools).
    pub fn footprint(&self, d: &Dab, index: usize) -> Footprint {
        let rect = self.dab_rect(d, false);
        let mut cov = Vec::new();
        self.rasterize(d, false, rect, &mut cov);
        Footprint { dab: *d, index, rect, cov }
    }

    /// The stroke-level masks on accumulated coverage `c` at a pixel (`dual`: the Dual Brush
    /// stroke's coverage there), in Photoshop's order (measured 2026-10-17): Texture (not per
    /// tip), then the Dual Brush, then Wet Edges. Opacity and the selection come after.
    #[inline]
    pub fn stroke_mask(&self, c: f32, dual: Option<f32>, x: i32, y: i32) -> f32 {
        let b = &self.brush;
        let mut m = c;
        if self.texture.is_some() && !b.texture.each_tip {
            m = texture_combine(b.texture.mode, m, self.texture_at(x, y), b.texture.depth.clamp(0.0, 1.0));
        }
        if let Some(dv) = dual {
            m = dual_combine(b.dual_brush.mode, m, dv);
        }
        if b.wet_edges {
            m = wet_edges(m);
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Sparse stroke buffer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CovTile {
    cov: Vec<f32>,
    /// Premultiplied native colour channels (`nc` per pixel), empty without per-dab colour.
    col: Vec<f32>,
}

/// Pixels of one coverage tile.
const TILE_PIXELS: usize = (COV_TILE * COV_TILE) as usize;

/// Dab area (pixels) from which a batch of dabs is rasterised on several threads.
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_AREA: u64 = 128 * 128;

/// Dirty tiles from which a composite runs on several threads.
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_TILES: usize = 4;

/// The pixels of coverage tile `key`.
fn tile_rect((tx, ty): (i32, i32)) -> Rect {
    Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE)
}

/// Is any value of `vals` (one per pixel of `vrect`, in rows) over `tr` (inside `vrect`) positive?
fn any_paint(tr: Rect, vrect: Rect, vals: &[f32]) -> bool {
    let w = vrect.width() as usize;
    (tr.y0..tr.y1).any(|y| {
        let row = (y - vrect.y0) as usize * w;
        vals.get(row + (tr.x0 - vrect.x0) as usize..row + (tr.x1 - vrect.x0) as usize).is_some_and(|r| r.iter().any(|&v| v > 0.0))
    })
}

impl CovTile {
    /// An empty tile, with per-pixel colour for `nc` channels when given.
    fn new(nc: Option<usize>) -> Self {
        Self { cov: vec![0.0; TILE_PIXELS], col: nc.map_or_else(Vec::new, |nc| vec![0.0; TILE_PIXELS * nc]) }
    }

    /// Make room for colour (`nc` channels) before paint lands.
    fn prepare(&mut self, nc: Option<usize>) {
        if let Some(nc) = nc
            && self.col.is_empty()
        {
            self.col = vec![0.0; TILE_PIXELS * nc];
        }
    }

    /// Accumulate the values of `vals` (one per pixel of `vrect`, in rows) over `tr` (inside both
    /// `vrect` and this tile, `key`): see [`CoverageMap::accumulate`].
    #[allow(clippy::too_many_arguments)]
    fn accumulate(&mut self, key: (i32, i32), tr: Rect, vrect: Rect, vals: &[f32], ceil: f32, color: Option<&[f32]>, nc: usize) {
        let (tx, ty) = key;
        let w = vrect.width() as usize;
        for y in tr.y0..tr.y1 {
            let row = (y - vrect.y0) as usize * w;
            let trow = ((y - ty * COV_TILE) * COV_TILE) as usize;
            for x in tr.x0..tr.x1 {
                let vi = row + (x - vrect.x0) as usize;
                let Some(&v) = vals.get(vi) else { continue };
                if v <= 0.0 {
                    continue;
                }
                let ti = trow + (x - tx * COV_TILE) as usize;
                let Some(&c) = self.cov.get(ti) else { continue };
                let nv = if ceil > c { c + v * (ceil - c) } else { c };
                if nv <= c {
                    continue;
                }
                if let (Some(col), Some(p)) = (color, self.col.get_mut(ti * nc..(ti + 1) * nc)) {
                    let k = if c < 1.0 { (nv - c) / (1.0 - c) } else { 0.0 };
                    for (p, col) in p.iter_mut().zip(col) {
                        *p = col * k + *p * (1.0 - k);
                    }
                }
                self.cov[ti] = nv;
            }
        }
    }
}

/// A sparse tiled coverage (+ optional colour) buffer.
#[derive(Clone, Debug, Default)]
pub struct CoverageMap {
    tiles: HashMap<(i32, i32), CovTile>,
    nc: usize,
    bounds: Rect,
    dirty: HashSet<(i32, i32)>,
}

impl CoverageMap {
    pub fn new(color_channels: usize) -> Self {
        Self { nc: color_channels, ..Default::default() }
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        let (tx, ty) = (x.div_euclid(COV_TILE), y.div_euclid(COV_TILE));
        self.tiles.get(&(tx, ty)).map_or(0.0, |t| t.cov[((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize])
    }

    /// Accumulate dab values over `rect`: flow builds up towards the dab's opacity ceiling
    /// (`c ← c + v·(ceil − c)`). `color` = native colour for per-dab colour.
    pub fn accumulate(&mut self, rect: Rect, vals: &[f32], ceil: f32, color: Option<&[f32]>) {
        if rect.is_empty() {
            return;
        }
        self.bounds = self.bounds.union(&rect);
        let nc = self.nc;
        let (t0x, t0y) = (rect.x0.div_euclid(COV_TILE), rect.y0.div_euclid(COV_TILE));
        let (t1x, t1y) = ((rect.x1 - 1).div_euclid(COV_TILE), (rect.y1 - 1).div_euclid(COV_TILE));
        let want_col = color.is_some() && nc > 0;
        for ty in t0y..=t1y {
            for tx in t0x..=t1x {
                let tr = tile_rect((tx, ty)).intersect(&rect);
                // Skip tiles this dab doesn't touch.
                if tr.is_empty() || !any_paint(tr, rect, vals) {
                    continue;
                }
                let tile = self.tiles.entry((tx, ty)).or_insert_with(|| CovTile::new(want_col.then_some(nc)));
                tile.prepare(want_col.then_some(nc));
                self.dirty.insert((tx, ty));
                tile.accumulate((tx, ty), tr, rect, vals, ceil, color.filter(|_| want_col), nc);
            }
        }
    }

    /// Rasterise `dabs` with `ctx` and accumulate them in order, as `rasterize` and
    /// [`accumulate`](Self::accumulate) would one dab at a time (the same bits), but tile by tile:
    /// each tile takes the dabs over it in order, so the tiles of a large brush go in parallel.
    /// `rects` are the dabs' rectangles, `ceil` their opacity ceilings, `colors` their native
    /// colours (per-dab colour).
    fn apply_dabs(&mut self, ctx: &BrushContext, dabs: &[Dab], rects: &[Rect], ceil: &[f32], colors: Option<&[[f32; 8]]>, dual: bool) {
        let nc = self.nc;
        let want_col = colors.is_some() && nc > 0;
        // (tile, dab) pairs, in dab order within each tile.
        let mut pairs: Vec<((i32, i32), u32)> = Vec::new();
        let mut area = 0u64;
        for (i, r) in rects.iter().enumerate().take(dabs.len()) {
            if r.is_empty() {
                continue;
            }
            self.bounds = self.bounds.union(r);
            area = area.saturating_add(u64::from(r.width()) * u64::from(r.height()));
            for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
                for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                    pairs.push(((tx, ty), i as u32));
                }
            }
        }
        if pairs.is_empty() {
            return;
        }
        pairs.sort_unstable();
        struct Job {
            key: (i32, i32),
            tile: CovTile,
            created: bool,
            dabs: std::ops::Range<usize>,
            painted: bool,
        }
        let mut jobs = Vec::new();
        let mut start = 0;
        while let Some(&(key, _)) = pairs.get(start) {
            let end = start + pairs[start..].iter().take_while(|p| p.0 == key).count();
            let (tile, created) = match self.tiles.remove(&key) {
                Some(t) => (t, false),
                None => (CovTile::new(want_col.then_some(nc)), true),
            };
            jobs.push(Job { key, tile, created, dabs: start..end, painted: false });
            start = end;
        }
        let run = |job: &mut Job, vals: &mut Vec<f32>| {
            let whole = tile_rect(job.key);
            for &(_, i) in &pairs[job.dabs.clone()] {
                let i = i as usize;
                let (Some(d), Some(r)) = (dabs.get(i), rects.get(i)) else { continue };
                let tr = whole.intersect(r);
                if tr.is_empty() {
                    continue;
                }
                ctx.rasterize(d, dual, tr, vals);
                if !any_paint(tr, tr, vals) {
                    continue;
                }
                job.tile.prepare(want_col.then_some(nc));
                job.painted = true;
                let color = colors.and_then(|c| c.get(i)).filter(|_| want_col).map(|c| &c[..nc.min(8)]);
                job.tile.accumulate(job.key, tr, tr, vals, ceil.get(i).copied().unwrap_or(1.0), color, nc);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if area >= PARALLEL_AREA && jobs.len() > 1 {
            use rayon::prelude::*;
            jobs.par_iter_mut().for_each_init(Vec::new, |vals, job| run(job, vals));
        } else {
            let mut vals = Vec::new();
            jobs.iter_mut().for_each(|job| run(job, &mut vals));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = area;
            let mut vals = Vec::new();
            jobs.iter_mut().for_each(|job| run(job, &mut vals));
        }
        for job in jobs {
            if job.painted {
                self.dirty.insert(job.key);
            }
            if job.painted || !job.created {
                self.tiles.insert(job.key, job.tile);
            }
        }
    }

    fn take_dirty(&mut self) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = self.dirty.drain().collect();
        v.sort_unstable();
        v
    }

    /// The finished coverage over the tiles `keys`: `tail`'s copy of a tile where finishing the
    /// stroke changes it (see [`StrokeRenderer::tail_preview`]), else this map's own.
    fn finished_subset(&self, tail: Option<&Self>, keys: &HashSet<(i32, i32)>) -> Self {
        let tile = |k: &(i32, i32)| tail.and_then(|t| t.tiles.get(k)).or_else(|| self.tiles.get(k)).map(|t| (*k, t.clone()));
        Self { tiles: keys.iter().filter_map(tile).collect(), nc: self.nc, bounds: tail.map_or(self.bounds, |t| t.bounds), dirty: HashSet::new() }
    }

    /// Union two passes of the same brush stroke without applying opacity twice where they
    /// overlap. The stronger coverage wins, including its per-dab colour when present.
    fn union_max(&mut self, other: &Self) {
        self.bounds = self.bounds.union(&other.bounds);
        for (&key, source) in &other.tiles {
            let Some(target) = self.tiles.get_mut(&key) else {
                self.tiles.insert(key, source.clone());
                self.dirty.insert(key);
                continue;
            };
            for (index, (dst, src)) in target.cov.iter_mut().zip(&source.cov).enumerate() {
                if *src > *dst {
                    *dst = *src;
                    let start = index.saturating_mul(self.nc);
                    let end = start.saturating_add(self.nc);
                    if let (Some(dst_color), Some(src_color)) = (target.col.get_mut(start..end), source.col.get(start..end)) {
                        dst_color.copy_from_slice(src_color);
                    }
                }
            }
            self.dirty.insert(key);
        }
    }
}

// ---------------------------------------------------------------------------
// Stroke renderer
// ---------------------------------------------------------------------------

/// What [`StrokeRenderer::composite_union_live`] drew last: the coverage tiles of the passes'
/// smoothing tails (redrawn without them next time) and the merged bounds.
#[derive(Clone, Debug, Default)]
pub struct UnionLive {
    tail_keys: HashSet<(i32, i32)>,
    bounds: Rect,
}

impl UnionLive {
    /// The merged bounds of the finished passes, as last drawn.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
}

/// Incremental stroke rendering: feed points, composite dirty tiles from the pre-stroke pixels.
#[derive(Clone, Debug)]
pub struct StrokeRenderer {
    pub ctx: BrushContext,
    generator: DabGenerator,
    cov: CoverageMap,
    dual: Option<CoverageMap>,
    fmt: Option<PixelFormat>,
    per_dab_color: bool,
    dabs_done: usize,
    dab_buf: Vec<Dab>,
    dual_buf: Vec<Dab>,
    all_dabs: Option<Vec<Dab>>,
}

impl StrokeRenderer {
    /// Composite the union of this stroke and a mirrored pass as one stroke. This keeps
    /// overlapping dabs on a symmetry axis under one opacity ceiling at every bit depth.
    pub fn composite_union(&self, other: &Self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool) -> Rect {
        self.composite_union_many(std::iter::once(other), pre, target, selection, lock_transparency)
    }

    /// Combine all symmetry passes before applying brush opacity and selection once.
    pub fn composite_union_many<'a>(
        &self,
        others: impl IntoIterator<Item = &'a Self>,
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
    ) -> Rect {
        let mut merged = self.clone();
        for other in others {
            merged.cov.union_max(&other.cov);
            if let (Some(to), Some(from)) = (&mut merged.dual, &other.dual) {
                to.union_max(from);
            }
        }
        merged.composite(pre, target, selection, lock_transparency, true)
    }

    /// Coverage tiles (stroke and Dual Brush) changed since the last call or composite, into
    /// `keys`; forgets them.
    fn drain_dirty_into(&mut self, keys: &mut HashSet<(i32, i32)>) {
        keys.extend(self.cov.dirty.drain());
        if let Some(d) = self.dual.as_mut() {
            keys.extend(d.dirty.drain());
        }
    }

    /// Every coverage tile (stroke and Dual Brush), into `keys`.
    fn keys_into(&self, keys: &mut HashSet<(i32, i32)>) {
        keys.extend(self.cov.tiles.keys());
        if let Some(d) = &self.dual {
            keys.extend(d.tiles.keys());
        }
    }

    /// Live preview of a stroke painted as several passes (the stroke, then its symmetry
    /// mirrors): leaves `target` as finishing every pass now and compositing them with
    /// [`composite_union_many`](Self::composite_union_many) over `pre` would, but redraws only the
    /// coverage tiles that changed since the previous call (new dabs, and the smoothing tail now
    /// and before). `full` redraws every tile: on the first call, or when a pass joins. Returns
    /// the damaged rectangle.
    pub fn composite_union_live(
        passes: &mut [&mut Self],
        live: &mut UnionLive,
        full: bool,
        pre: &Surface,
        target: &mut Surface,
        selection: Option<&Surface>,
        lock_transparency: bool,
    ) -> Rect {
        let mut keys = std::mem::take(&mut live.tail_keys);
        for p in passes.iter_mut() {
            p.drain_dirty_into(&mut keys);
            if full {
                p.keys_into(&mut keys);
            }
        }
        let tails: Vec<_> = passes.iter().map(|p| p.tail_preview()).collect();
        let mut tail_keys = HashSet::new();
        for t in tails.iter().flatten() {
            t.keys_into(&mut tail_keys);
        }
        keys.extend(&tail_keys);
        let mut merged: Option<Self> = None;
        for (p, t) in passes.iter().zip(&tails) {
            let cov = p.cov.finished_subset(t.as_ref().map(|t| &t.cov), &keys);
            let dual = p.dual.as_ref().map(|d| d.finished_subset(t.as_ref().and_then(|t| t.dual.as_ref()), &keys));
            match merged.as_mut() {
                None => {
                    merged = Some(Self {
                        ctx: p.ctx.clone(),
                        generator: p.generator.clone(),
                        cov,
                        dual,
                        fmt: p.fmt,
                        per_dab_color: p.per_dab_color,
                        dabs_done: p.dabs_done,
                        dab_buf: Vec::new(),
                        dual_buf: Vec::new(),
                        all_dabs: None,
                    })
                }
                Some(m) => {
                    m.cov.union_max(&cov);
                    if let (Some(to), Some(from)) = (&mut m.dual, &dual) {
                        to.union_max(from);
                    }
                }
            }
        }
        let Some(mut merged) = merged else { return Rect::EMPTY };
        let (before, after) = (live.bounds, merged.bounds());
        // Back to the pre-stroke pixels where the previous call drew and nothing covers now.
        let mut keys: Vec<_> = keys.into_iter().collect();
        keys.sort_unstable();
        let mut dmg = Rect::EMPTY;
        let mut region = Vec::new();
        for &(tx, ty) in &keys {
            let tile = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE);
            let old = tile.intersect(&before);
            if !old.is_empty() {
                pre.read_region_into(old, &mut region);
                target.write_region(old, &region);
                dmg = dmg.union(&old);
            }
            if merged.cov.tiles.contains_key(&(tx, ty)) {
                dmg = dmg.union(&tile.intersect(&after));
            }
        }
        merged.composite(pre, target, selection, lock_transparency, true);
        live.tail_keys = tail_keys;
        live.bounds = after;
        dmg
    }
    /// `fmt` = target pixel format (needed for per-dab colour); `zoom` for smoothing.
    pub fn new(brush: &BrushSettings, fmt: Option<PixelFormat>, zoom: f32) -> Self {
        let per_dab_color = brush.color_dynamics.enabled && !brush.erase && fmt.is_some();
        let nc = if per_dab_color { fmt.map_or(0, |f| f.mode.color_channels()) } else { 0 };
        Self {
            ctx: BrushContext::new(brush),
            generator: DabGenerator::new(brush, zoom),
            cov: CoverageMap::new(nc),
            dual: brush.dual_brush.enabled.then(|| CoverageMap::new(0)),
            fmt,
            per_dab_color,
            dabs_done: 0,
            dab_buf: Vec::new(),
            dual_buf: Vec::new(),
            all_dabs: None,
        }
    }

    /// Keep a copy of every primary dab (for tests and sequential tools).
    pub fn record_dabs(mut self) -> Self {
        self.all_dabs = Some(Vec::new());
        self
    }
    pub fn dabs(&self) -> &[Dab] {
        self.all_dabs.as_deref().unwrap_or(&[])
    }
    pub fn dab_count(&self) -> usize {
        self.dabs_done
    }
    /// Union of the dab rectangles so far.
    pub fn bounds(&self) -> Rect {
        self.cov.bounds
    }

    fn raster_pending(&mut self) {
        let mut dabs = std::mem::take(&mut self.dab_buf);
        let mut duals = std::mem::take(&mut self.dual_buf);
        // Colours convert here, on the calling thread (a CMYK document's space is thread-local).
        let colors: Option<Vec<[f32; 8]>> = match (self.per_dab_color, self.fmt) {
            (true, Some(f)) => Some(
                dabs.iter()
                    .map(|d| {
                        let mut native = [0.0f32; 8];
                        from_rgba_into(&f, d.color, &mut native);
                        native
                    })
                    .collect(),
            ),
            _ => None,
        };
        // Bounds track the full dab rectangle (even fully transparent parts), like the damage.
        let rects: Vec<Rect> = dabs.iter().map(|d| self.ctx.paint_rect(d, false)).collect();
        let ceil: Vec<f32> = dabs.iter().map(|d| d.opacity).collect();
        self.cov.apply_dabs(&self.ctx, &dabs, &rects, &ceil, colors.as_deref(), false);
        if let Some(dm) = self.dual.as_mut() {
            let rects: Vec<Rect> = duals.iter().map(|d| self.ctx.paint_rect(d, true)).collect();
            dm.apply_dabs(&self.ctx, &duals, &rects, &vec![1.0; duals.len()], None, true);
        }
        self.dabs_done += dabs.len();
        if let Some(all) = self.all_dabs.as_mut() {
            all.extend_from_slice(&dabs);
        }
        dabs.clear();
        duals.clear();
        self.dab_buf = dabs;
        self.dual_buf = duals;
    }

    /// See [`DabGenerator::wants_time`].
    pub fn wants_time(&self) -> bool {
        self.generator.wants_time()
    }

    /// The string between the brush and the pointer while smoothing lags (see
    /// [`Smoother::string`](crate::dynamics::Smoother::string)).
    pub fn smoothing_string(&self) -> Option<([f64; 2], [f64; 2])> {
        self.generator.smoothing_string()
    }

    /// Feed input points (any chunking gives the same result).
    pub fn push(&mut self, pts: &[StrokePoint]) {
        self.generator.push(pts, &mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// End of stroke (smoothing catch-up, a lone first point).
    pub fn finish(&mut self) {
        self.generator.finish(&mut self.dab_buf, &mut self.dual_buf);
        self.raster_pending();
    }

    /// The primary dabs finishing the stroke now would add, without finishing it: for live
    /// previews of sequential-dab tools, which run each dab themselves (see [`tail_preview`](Self::tail_preview)).
    pub fn tail_dabs(&self) -> Vec<Dab> {
        let mut generator = self.generator.clone();
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        generator.finish(&mut dabs, &mut duals);
        dabs
    }

    /// What finishing the stroke now would add (the smoothing catch-up tail to the last point, or
    /// a lone first dab), for live previews: a renderer holding copies of the coverage tiles the
    /// tail touches with the tail rendered in, so its `composite` draws exactly those tiles as
    /// [`finish`](Self::finish) would leave them. `None` when finishing adds nothing.
    pub fn tail_preview(&self) -> Option<StrokeRenderer> {
        let mut generator = self.generator.clone();
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        generator.finish(&mut dabs, &mut duals);
        if dabs.is_empty() && duals.is_empty() {
            return None;
        }
        let mut keys = HashSet::new();
        for r in dabs.iter().map(|d| self.ctx.dab_rect(d, false)).chain(duals.iter().map(|d| self.ctx.dab_rect(d, true))) {
            if r.is_empty() {
                continue;
            }
            for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
                for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                    keys.insert((tx, ty));
                }
            }
        }
        let subset = |m: &CoverageMap| CoverageMap {
            tiles: keys.iter().filter_map(|k| m.tiles.get(k).map(|t| (*k, t.clone()))).collect(),
            nc: m.nc,
            bounds: m.bounds,
            dirty: HashSet::new(),
        };
        let mut t = StrokeRenderer {
            ctx: self.ctx.clone(),
            generator,
            cov: subset(&self.cov),
            dual: self.dual.as_ref().map(subset),
            fmt: self.fmt,
            per_dab_color: self.per_dab_color,
            dabs_done: self.dabs_done,
            dab_buf: dabs,
            dual_buf: duals,
            all_dabs: None,
        };
        t.raster_pending();
        // Tiles whose coverage the tail doesn't change still composite the same: redraw them all.
        t.cov.dirty.extend(t.cov.tiles.keys().copied());
        Some(t)
    }

    /// Mark the coverage tiles over `r` for the next `composite` (e.g. to redraw over a preview).
    pub fn mark_dirty(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        for ty in r.y0.div_euclid(COV_TILE)..=(r.y1 - 1).div_euclid(COV_TILE) {
            for tx in r.x0.div_euclid(COV_TILE)..=(r.x1 - 1).div_euclid(COV_TILE) {
                if self.cov.tiles.contains_key(&(tx, ty)) {
                    self.cov.dirty.insert((tx, ty));
                }
            }
        }
    }

    /// The coverage tiles touched since the last call (or `composite`), as one rectangle clipped to
    /// the stroke bounds, and forget them: what a live preview that composites its own paint has
    /// to redraw.
    pub fn take_dirty_rect(&mut self) -> Rect {
        let r = self
            .cov
            .take_dirty()
            .into_iter()
            .fold(Rect::EMPTY, |acc, (tx, ty)| acc.union(&Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE)));
        if let Some(d) = self.dual.as_mut() {
            d.dirty.clear();
        }
        r.intersect(&self.bounds())
    }

    /// Final coverage over `r`, one value per pixel in rows (as [`dense_coverage`](Self::dense_coverage)
    /// over its bounds).
    pub fn coverage_in(&self, r: Rect) -> Vec<f32> {
        (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| self.coverage_at(x, y)).collect()
    }

    /// Final stroke coverage at a pixel (stroke-level masks applied, before opacity/selection).
    pub fn coverage_at(&self, x: i32, y: i32) -> f32 {
        let c = self.cov.get(x, y);
        if c <= 0.0 {
            return 0.0;
        }
        self.ctx.stroke_mask(c, self.dual.as_ref().map(|d| d.get(x, y)), x, y)
    }

    /// Dense final coverage over the stroke bounds.
    pub fn dense_coverage(&self) -> (Rect, Vec<f32>) {
        let b = self.bounds();
        let w = b.width() as usize;
        let mut out = vec![0.0f32; w * b.height() as usize];
        for (&(tx, ty), t) in &self.cov.tiles {
            let tr = Rect::new(tx * COV_TILE, ty * COV_TILE, (tx + 1) * COV_TILE, (ty + 1) * COV_TILE).intersect(&b);
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let c = t.cov[ti];
                    if c > 0.0 {
                        out[(y - b.y0) as usize * w + (x - b.x0) as usize] =
                            self.ctx.stroke_mask(c, self.dual.as_ref().map(|d| d.get(x, y)), x, y);
                    }
                }
            }
        }
        (b, out)
    }

    /// Composite the stroke buffer onto `target`, reading original pixels from `pre` (the
    /// pre-stroke surface; pass a clone of the target taken before the first `push`). With
    /// `all = false` only tiles touched since the last composite are processed (interactive use).
    /// Returns the damaged rectangle.
    pub fn composite(&mut self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool, all: bool) -> Rect {
        self.composite_tiles(pre, target, selection, lock_transparency, all, true)
    }

    /// [`composite`](Self::composite), on several threads when `parallel` and there are enough
    /// tiles (the same pixels either way).
    fn composite_tiles(&mut self, pre: &Surface, target: &mut Surface, selection: Option<&Surface>, lock_transparency: bool, all: bool, parallel: bool) -> Rect {
        let keys: Vec<(i32, i32)> = if all {
            self.cov.dirty.clear();
            let mut k: Vec<_> = self.cov.tiles.keys().copied().collect();
            k.sort_unstable();
            k
        } else {
            self.cov.take_dirty()
        };
        if let Some(d) = self.dual.as_mut() {
            d.dirty.clear();
        }
        let bounds = self.bounds();
        let fmt = target.format();
        let n = fmt.channels();
        let a_idx = alpha_index(&fmt);
        let nc = fmt.mode.color_channels();
        let b = &self.ctx.brush;
        let opacity = b.opacity.clamp(0.0, 1.0);
        let mut src = [0.0f32; 8];
        from_rgba_into(&fmt, b.color, &mut src);
        // A colour picked as CMYK paints its inks as they are in a CMYK target (#4344).
        if let (ColorMode::Cmyk, Some((rgb, inks))) = (fmt.mode, b.color_cmyk)
            && rgb == [b.color[0], b.color[1], b.color[2]]
        {
            src[..4].copy_from_slice(&inks);
        }
        if let Some(a) = a_idx {
            src[a] = b.color[3];
        }
        // 8-bit targets: the pixels the stroke changes are dithered as they are quantised, so
        // soft, low-flow strokes don't band (see `dither8`). Aliased strokes (the Pencil, the
        // Eraser's Pencil and Block modes) paint every pixel by the same amount, so they have no
        // gradient to band and are not dithered: a Pencil line at partial opacity is one flat
        // colour, as in Photoshop, not grain that a tolerance-0 Magic Wand or Paint Bucket trips on.
        let dither = fmt.sample == SampleType::U8 && !b.aliased;
        let (cov, dual, ctx, per_dab_color) = (&self.cov, self.dual.as_ref(), &self.ctx, self.per_dab_color);
        // Part `part` of one coverage tile onto the pre-stroke pixels: the region it changes
        // (`part` within the stroke bounds) and its new pixels.
        let tile_pixels = |(tx, ty): (i32, i32), part: Rect, region: &mut Vec<f32>| -> Option<Rect> {
            let tile = cov.tiles.get(&(tx, ty))?;
            let tr = tile_rect((tx, ty)).intersect(&bounds).intersect(&part);
            if tr.is_empty() {
                return None;
            }
            let mut src = src;
            pre.read_region_into(tr, region);
            let sel = selection.map(|s| (s.channels(), s.read_region(tr)));
            // The Dual Brush tile once, not a lookup per pixel (no tile: no dual paint there).
            let dual_tile = dual.map(|d| d.tiles.get(&(tx, ty)));
            let w = tr.width() as usize;
            for y in tr.y0..tr.y1 {
                for x in tr.x0..tr.x1 {
                    let ti = ((y - ty * COV_TILE) * COV_TILE + (x - tx * COV_TILE)) as usize;
                    let c = tile.cov[ti];
                    if c <= 0.0 {
                        continue;
                    }
                    let i = (y - tr.y0) as usize * w + (x - tr.x0) as usize;
                    let dv = dual_tile.map(|t| t.and_then(|t| t.cov.get(ti).copied()).unwrap_or(0.0));
                    let m = ctx.stroke_mask(c, dv, x, y);
                    let s = sel.as_ref().map_or(1.0, |(sc, v)| v[i * sc]);
                    let mut k = (m * opacity * s).min(1.0);
                    if k <= 0.0 {
                        continue;
                    }
                    if b.mode == BlendMode::Dissolve {
                        k = if hash2(x, y, b.seed) < k { 1.0 } else { 0.0 };
                        if k == 0.0 {
                            continue;
                        }
                    }
                    let px = &mut region[i * n..(i + 1) * n];
                    if b.erase {
                        if let (Some(a), false) = (a_idx, lock_transparency) {
                            px[a] *= 1.0 - k;
                        }
                        if dither {
                            dither8(px, x, y);
                        }
                        continue;
                    }
                    if lock_transparency && a_idx.is_some_and(|a| px[a] <= 0.0) {
                        continue;
                    }
                    if per_dab_color && !tile.col.is_empty() {
                        let p = &tile.col[ti * nc..(ti + 1) * nc];
                        for ch in 0..nc {
                            src[ch] = p[ch] / c;
                        }
                    }
                    if matches!(b.mode, BlendMode::Normal | BlendMode::Dissolve) {
                        over_native(&fmt, px, &src[..n], k, lock_transparency);
                    } else {
                        let d = to_rgba(&fmt, px);
                        let sr = to_rgba(&fmt, &src[..n]);
                        let mut o = photocraft_color::blend::composite(b.mode, d, sr, k);
                        if lock_transparency {
                            o[3] = d[3];
                        }
                        let mut enc = [0.0f32; 8];
                        from_rgba_into(&fmt, o, &mut enc);
                        px.copy_from_slice(&enc[..n]);
                    }
                    if dither {
                        dither8(px, x, y);
                    }
                }
            }
            Some(tr)
        };
        let mut dmg = Rect::EMPTY;
        // A large brush dirties hundreds of tiles per frame: the layer tiles under them come out
        // of the layer and are worked on every core, each with the coverage tiles over it (the
        // colour conversions under the calling thread's CMYK space), then go back.
        #[cfg(not(target_arch = "wasm32"))]
        if parallel && keys.len() >= PARALLEL_TILES {
            use rayon::prelude::*;
            let cmyk = photocraft_color::convert::active_cmyk_space();
            let mut over: std::collections::BTreeMap<photocraft_geom::TileCoord, Vec<(i32, i32)>> = std::collections::BTreeMap::new();
            for &key in &keys {
                if cov.tiles.contains_key(&key) {
                    for tc in tile_rect(key).intersect(&bounds).tiles() {
                        over.entry(tc).or_default().push(key);
                    }
                }
            }
            let mut work: Vec<_> = over.into_iter().filter_map(|(tc, ks)| target.take_tiles(tc.rect()).into_iter().next().map(|(c, t)| (c, t, ks))).collect();
            let changed: Vec<Rect> = work
                .par_iter_mut()
                .map(|(tc, tile, ks)| {
                    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                        let tile = std::sync::Arc::make_mut(tile);
                        let mut region = Vec::new();
                        let mut changed = Rect::EMPTY;
                        for &key in ks.iter() {
                            if let Some(tr) = tile_pixels(key, tc.rect(), &mut region) {
                                tile.write_region(&fmt, *tc, tr, &region, tr);
                                changed = changed.union(&tr);
                            }
                        }
                        changed
                    })
                })
                .collect();
            target.put_tiles(work.into_iter().map(|(c, t, _)| (c, t)));
            let dmg = changed.iter().fold(dmg, |a, r| a.union(r));
            return if all { bounds } else { dmg };
        }
        let _ = parallel;
        let mut region = Vec::new();
        for key in keys {
            if let Some(tr) = tile_pixels(key, tile_rect(key), &mut region) {
                target.write_region(tr, &region);
                dmg = dmg.union(&tr);
            }
        }
        if all { bounds } else { dmg }
    }
}

/// Render a whole stroke onto `target` (one-shot). Returns the damaged rectangle.
pub fn render_stroke(
    target: &mut Surface,
    brush: &BrushSettings,
    points: &[StrokePoint],
    selection: Option<&Surface>,
    lock_transparency: bool,
    zoom: f32,
) -> Rect {
    if points.is_empty() {
        return Rect::EMPTY;
    }
    let pre = target.clone();
    let mut r = StrokeRenderer::new(brush, Some(target.format()), zoom);
    r.push(points);
    r.finish();
    r.composite(&pre, target, selection, lock_transparency, true)
}

// The zero-angle transform only changes signs; squaring gives the same radial distance.
// `out` comes zeroed: only the columns each row's span can reach are written.
#[inline(never)]
fn rasterize_round(d: &Dab, rect: Rect, out: &mut [f32], hardness: f32, aliased: bool) {
    let (cx, cy) = if aliased { grid_center(d.center.x, d.center.y, 2.0 * d.radius) } else { (d.center.x as f32, d.center.y as f32) };
    let ro = d.roundness.max(0.5 / d.radius).min(1.0);
    let rm = (d.radius * ro).max(0.5);
    let w = rect.width() as usize;
    let reach = if aliased { rm } else { tip_reach(rm, hardness).max(rm) };
    let reach2 = (reach + 0.5) * (reach + 0.5);
    let (r0, s) = crate::tip_shape(rm, hardness);
    let slack = span_slack(cx, cy, rect);
    let wide = reach + 0.5 + slack;
    let x0 = rect.x0 as f32 + 0.5 - cx;
    for (yy, row) in out.chunks_exact_mut(w).enumerate() {
        let dy = (rect.y0 + yy as i32) as f32 + 0.5 - cy;
        // `(dx·ro)² + dy² < reach2` only for |dx| < √(reach2 − dy²) / ro: the row's span, widened.
        let left = wide * wide - dy * dy;
        if left < 0.0 {
            continue;
        }
        let half = left.sqrt() / ro + slack;
        let (from, to) = clamp_columns(-half - x0, half - x0, w);
        let Some(row) = row.get_mut(from..to) else { continue };
        if aliased {
            for (xx, dst) in row.iter_mut().enumerate() {
                let dx = (rect.x0 + (from + xx) as i32) as f32 + 0.5 - cx;
                let d2 = (dx * ro).powi(2) + dy * dy;
                *dst = if d2 < reach2 && d2.sqrt() <= rm { d.alpha } else { 0.0 };
            }
        } else {
            for (xx, dst) in row.iter_mut().enumerate() {
                let dx = (rect.x0 + (from + xx) as i32) as f32 + 0.5 - cx;
                let d2 = (dx * ro).powi(2) + dy * dy;
                let val = if d2 >= reach2 { 0.0 } else { crate::falloff_shaped(d2.sqrt(), r0, s) };
                *dst = if val <= 0.0 { 0.0 } else { val * d.alpha };
            }
        }
    }
}

/// A plain sampled tip mapped onto the canvas for one dab (see `rasterize`).
struct SampledTip<'a> {
    mips: &'a Mips,
    level: usize,
    /// Pixel offset → tip space: `u = map[0]·dx + map[1]·dy`, `v = map[2]·dx + map[3]·dy`.
    map: [f32; 4],
    inv_scale: f32,
    ro: f32,
    tw: f32,
    th: f32,
    /// The pixel column 0 of a row is, the dab centre's x and the dab's flow.
    x0: i32,
    cx: f32,
    alpha: f32,
}

impl SampledTip<'_> {
    /// Columns `from..to` of a row (`dy` from the dab centre) into `out`: the bitmap sampled
    /// bilinearly at the mip level, times the flow. The arithmetic is the scalar loop's (which
    /// samples through [`Mips::sample_clamped`]) with its divisions turned into
    /// multiplications, so it agrees with it to float rounding.
    #[inline(never)]
    fn row(&self, from: usize, to: usize, dy: f32, out: &mut [f32]) {
        let (x0, cx, alpha) = (self.x0, self.cx, self.alpha);
        let Some((lw, lh, data)) = self.mips.levels.get(self.level) else { return };
        let (lw, lh) = (*lw, *lh);
        let [m0, m1, m2, m3] = self.map;
        let (half_w, half_h) = (self.tw / 2.0, self.th / 2.0);
        let k = self.inv_scale / self.ro;
        // Level texels per level-0 texel.
        let (sx, sy) = (lw as f32 / self.tw, lh as f32 / self.th);
        let (hi_x, hi_y) = (self.tw + 1.0, self.th + 1.0);
        let (u_row, v_row) = (m1 * dy, m3 * dy);
        let Some(out) = out.get_mut(from..to) else { return };
        for (i, dst) in out.iter_mut().enumerate() {
            let dx = (x0 + (from + i) as i32) as f32 + 0.5 - cx;
            let (u, v) = (m0 * dx + u_row, m2 * dx + v_row);
            let (px, py) = (u * self.inv_scale + half_w, -v * k + half_h);
            if px < -1.0 || py < -1.0 || px > hi_x || py > hi_y {
                continue;
            }
            let (fx, fy) = (px * sx - 0.5, py * sy - 0.5);
            let (xf, yf) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - xf, fy - yf);
            let (xi, yi) = (xf as i64, yf as i64);
            let s = if xi >= 0 && yi >= 0 && xi + 1 < lw as i64 && yi + 1 < lh as i64 {
                let j = yi as usize * lw + xi as usize;
                match (data.get(j..j + 2), data.get(j + lw..j + lw + 2)) {
                    (Some(&[p00, p10]), Some(&[p01, p11])) => {
                        let a = p00 + (p10 - p00) * tx;
                        let b = p01 + (p11 - p01) * tx;
                        a + (b - a) * ty
                    }
                    _ => 0.0,
                }
            } else {
                // The bitmap's edge: texels outside it are 0.
                let get = |x: i64, y: i64| if x < 0 || y < 0 || x >= lw as i64 || y >= lh as i64 { 0.0 } else { data.get(y as usize * lw + x as usize).copied().unwrap_or(0.0) };
                let a = get(xi, yi) + (get(xi + 1, yi) - get(xi, yi)) * tx;
                let b = get(xi, yi + 1) + (get(xi + 1, yi + 1) - get(xi, yi + 1)) * tx;
                a + (b - a) * ty
            };
            if s > 0.0 {
                *dst = s * alpha;
            }
        }
    }
}

/// How far (in pixels) a row's span is widened beyond the footprint: two pixels, more far from
/// the origin, where `f32` pixel coordinates lose precision (the per-pixel test still decides).
fn span_slack(cx: f32, cy: f32, rect: Rect) -> f32 {
    let far = cx.abs().max(cy.abs()).max(rect.x0.unsigned_abs().max(rect.x1.unsigned_abs()) as f32).max(rect.y0.unsigned_abs().max(rect.y1.unsigned_abs()) as f32);
    let s = 2.0 + far * 4e-6;
    if s.is_finite() { s } else { f32::INFINITY }
}

/// Columns `[from, to)` of a `w`-wide row for the span `lo..hi` (column offsets, possibly
/// infinite), widened by two columns. A NaN bound takes the whole row.
fn clamp_columns(lo: f32, hi: f32, w: usize) -> (usize, usize) {
    if lo > hi {
        return (0, 0);
    }
    let from = if lo.is_nan() { 0.0 } else { (lo.floor() - 2.0).clamp(0.0, w as f32) };
    let to = if hi.is_nan() { w as f32 } else { (hi.ceil() + 3.0).clamp(0.0, w as f32) };
    let (from, to) = (from as usize, to as usize);
    (from.min(to), to)
}

/// Which columns of a row a dab can paint, from its tip-space footprint. Only an optimisation:
/// every column it gives is still tested pixel by pixel, and it errs wide.
#[derive(Clone, Copy, Debug)]
enum Span {
    /// A computed tip: `(ro·u)² + v² < r²`.
    Ellipse { ro: f32, r: f32 },
    /// A sampled tip's bitmap: `−1 ≤ px ≤ tw + 1` and `−1 ≤ py ≤ th + 1`, where
    /// `px = u·inv_scale + tw/2` and `py = −(v/ro)·inv_scale + th/2`.
    Rect { inv_scale: f32, ro: f32, tw: f32, th: f32 },
}

impl Span {
    /// The columns `[from, to)` of the row `dy` (pixel-centre offset from the dab centre) of a
    /// `w`-wide rectangle whose column 0 is `dx0` from the centre, for the tip-space map `m`
    /// (`u = m[0]·dx + m[1]·dy`, `v = m[2]·dx + m[3]·dy`), widened by `slack` pixels.
    fn columns(self, m: [f32; 4], dx0: f32, dy: f32, slack: f32, w: usize) -> (usize, usize) {
        let finite = m.iter().all(|v| v.is_finite())
            && dx0.is_finite()
            && dy.is_finite()
            && slack.is_finite()
            && match self {
                Span::Ellipse { ro, r } => ro.is_finite() && ro > 0.0 && r.is_finite(),
                Span::Rect { inv_scale, ro, tw, th } => inv_scale.is_finite() && ro.is_finite() && ro > 0.0 && tw.is_finite() && th.is_finite(),
            };
        if !finite {
            // NaN or infinite inputs: the per-pixel test sorts it out (it paints NaN or nothing).
            return (0, w);
        }
        // (u, v) at column t: (u0 + du·t, v0 + dv·t).
        let (u0, du) = (m[0] * dx0 + m[1] * dy, m[0]);
        let (v0, dv) = (m[2] * dx0 + m[3] * dy, m[2]);
        let (lo, hi) = match self {
            Span::Ellipse { ro, r } => {
                let r = r + slack;
                let (a0, a) = (ro * u0, ro * du);
                let q = a * a + dv * dv;
                let b = 2.0 * (a0 * a + v0 * dv);
                let k = a0 * a0 + v0 * v0 - r * r;
                if q.is_nan() || q <= 1e-12 {
                    // The row is (nearly) one tip-space point: all or nothing.
                    return if k < 0.0 || k.is_nan() { (0, w) } else { (0, 0) };
                }
                let disc = b * b - 4.0 * q * k;
                if disc < 0.0 {
                    return (0, 0);
                }
                let sq = disc.sqrt();
                ((-b - sq) / (2.0 * q), (-b + sq) / (2.0 * q))
            }
            Span::Rect { inv_scale, ro, tw, th } => {
                let (p0, pa) = (u0 * inv_scale + tw / 2.0, du * inv_scale);
                let (q0, qa) = (-(v0 / ro) * inv_scale + th / 2.0, -(dv / ro) * inv_scale);
                // `slack` pixels in texels along each axis (a pixel is `inv_scale` texels across,
                // `inv_scale / ro` down).
                let (mp, mq) = (slack * inv_scale.abs() + 1.0, slack * (inv_scale / ro).abs() + 1.0);
                let (Some((a1, b1)), Some((a2, b2))) = (within(p0, pa, -1.0 - mp, tw + 1.0 + mp), within(q0, qa, -1.0 - mq, th + 1.0 + mq)) else {
                    return (0, 0);
                };
                (a1.max(a2), b1.min(b2))
            }
        };
        clamp_columns(lo, hi, w)
    }
}

/// The `t` where `lo ≤ v0 + dv·t ≤ hi`: `None` for none, infinite bounds when `dv` is (nearly)
/// zero and `v0` is in range, or anything is NaN.
fn within(v0: f32, dv: f32, lo: f32, hi: f32) -> Option<(f32, f32)> {
    if v0.is_nan() || dv.is_nan() || lo.is_nan() || hi.is_nan() {
        return Some((f32::NEG_INFINITY, f32::INFINITY));
    }
    if dv.abs() < 1e-12 {
        return (lo <= v0 && v0 <= hi).then_some((f32::NEG_INFINITY, f32::INFINITY));
    }
    let (a, b) = ((lo - v0) / dv, (hi - v0) / dv);
    Some((a.min(b), a.max(b)))
}

#[cfg(test)]
#[path = "render_reference.rs"]
mod reference;
