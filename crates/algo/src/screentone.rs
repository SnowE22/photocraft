//! Procedural screentone generator (GenTonik engine) for manga, comics and graphic arts.
//!
//! Features:
//! * 12 pattern types: dots, lines, crosshatch, noise, gaussian_noise, stipple,
//!   hexgrid, checker, concentric, stars, hearts, triangles.
//! * 4 dot shapes: circle, square, diamond, hexagon.
//! * Corner roundness (0.0 = sharp polygon, 1.0 = fully rounded).
//! * Orbital satellite system (1–4 satellites, angle, distance, shape, stretch toward parent).
//! * Gradient modulation (linear, radial) with midpoint curve bias.
//! * Subpixel antialiasing or 1-bit binary threshold (manga print mode).
//! * Layer-luminance halftone modulation (manga shading) or pure procedural sheet fill.
//! * Depth-agnostic, selection-aware and tile-invariant.

use photocraft_geom::Rect;
use photocraft_raster::Surface;
pub use photocraft_tone::{DotShape, PatternType, ScreentoneBackend, ScreentoneGradientType, ScreentoneMode, ScreentoneSpec};

use crate::Ctx;
use crate::fxutil::{luma, rgba, set_rgba, xy};
use crate::image::Image;

/// Built-in preset descriptor for manga screentone styles.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreentonePreset {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub spec: ScreentoneSpec,
}

/// Collection of classic manga screentone presets.
pub fn built_in_presets() -> Vec<ScreentonePreset> {
    vec![
        ScreentonePreset {
            id: "classic-10",
            name: "Deleter 10% Dot",
            category: "Classic Dots",
            spec: ScreentoneSpec {
                dot_size: 3.0,
                spacing_x: 25.0,
                spacing_y: 25.0,
                density: 0.1,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "classic-30",
            name: "Deleter 30% Dot",
            category: "Classic Dots",
            spec: ScreentoneSpec {
                dot_size: 5.5,
                spacing_x: 22.0,
                spacing_y: 22.0,
                density: 0.3,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "classic-60",
            name: "Deleter 60% Dot",
            category: "Classic Dots",
            spec: ScreentoneSpec {
                dot_size: 8.0,
                spacing_x: 20.0,
                spacing_y: 20.0,
                density: 0.6,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "square-dots",
            name: "Square Dots 45°",
            category: "Classic Dots",
            spec: ScreentoneSpec {
                dot_shape: DotShape::Square,
                dot_size: 6.0,
                spacing_x: 22.0,
                spacing_y: 22.0,
                density: 0.4,
                angle: 45.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "diamond-dots",
            name: "Diamond Dots",
            category: "Classic Dots",
            spec: ScreentoneSpec {
                dot_shape: DotShape::Diamond,
                dot_size: 7.0,
                spacing_x: 22.0,
                spacing_y: 22.0,
                density: 0.4,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "lines-45",
            name: "Parallel Lines 45°",
            category: "Lines & Hatching",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Lines,
                spacing_y: 12.0,
                line_width: 1.5,
                angle: 45.0,
                density: 0.5,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "crosshatch",
            name: "Crosshatch 45°/90°",
            category: "Lines & Hatching",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Crosshatch,
                spacing_y: 15.0,
                line_width: 1.0,
                angle: 45.0,
                cross_angle: 90.0,
                density: 0.5,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "fine-lines",
            name: "Fine Lines 0°",
            category: "Lines & Hatching",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Lines,
                spacing_y: 8.0,
                line_width: 0.8,
                angle: 0.0,
                density: 0.3,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "gauze",
            name: "Gauze / Sand Noise",
            category: "Texture",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Noise,
                dot_size: 2.0,
                spacing_x: 6.0,
                spacing_y: 6.0,
                density: 0.3,
                jitter_pos: 30.0,
                jitter_size: 50.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "heavy-noise",
            name: "Heavy Noise",
            category: "Texture",
            spec: ScreentoneSpec {
                pattern_type: PatternType::GaussianNoise,
                dot_size: 2.0,
                spacing_x: 4.0,
                spacing_y: 4.0,
                density: 0.5,
                jitter_pos: 50.0,
                jitter_size: 60.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "stipple",
            name: "Stipple Pointillism",
            category: "Texture",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Stipple,
                dot_size: 2.0,
                spacing_x: 8.0,
                spacing_y: 8.0,
                density: 0.4,
                jitter_pos: 40.0,
                jitter_size: 40.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "hex-grid",
            name: "Hex Grid Honeycomb",
            category: "Geometric",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Hexgrid,
                dot_shape: DotShape::Hexagon,
                dot_size: 8.0,
                spacing_x: 20.0,
                spacing_y: 20.0,
                density: 0.5,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "checker",
            name: "Checkerboard",
            category: "Geometric",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Checker,
                spacing_x: 20.0,
                spacing_y: 20.0,
                density: 0.5,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "triangles",
            name: "Triangles",
            category: "Geometric",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Triangles,
                dot_size: 10.0,
                spacing_x: 30.0,
                spacing_y: 30.0,
                density: 0.4,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "concentric",
            name: "Concentric Rings",
            category: "Effects",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Concentric,
                spacing_x: 20.0,
                spacing_y: 20.0,
                line_width: 1.5,
                density: 0.5,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "stars",
            name: "Shojo Stars",
            category: "Fun & Shojo",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Stars,
                dot_size: 8.0,
                spacing_x: 40.0,
                spacing_y: 40.0,
                density: 0.4,
                jitter_pos: 20.0,
                jitter_size: 20.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "hearts",
            name: "Romantic Hearts",
            category: "Fun & Shojo",
            spec: ScreentoneSpec {
                pattern_type: PatternType::Hearts,
                dot_size: 8.0,
                spacing_x: 35.0,
                spacing_y: 35.0,
                density: 0.3,
                jitter_pos: 15.0,
                jitter_size: 15.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "radial-fade",
            name: "Radial Gradient Fade",
            category: "Gradient Effects",
            spec: ScreentoneSpec {
                dot_size: 6.0,
                spacing_x: 18.0,
                spacing_y: 18.0,
                grad_type: ScreentoneGradientType::Radial,
                grad_size_start: 1.5,
                grad_size_end: 0.0,
                ..Default::default()
            },
        },
        ScreentonePreset {
            id: "planets",
            name: "Planets & Satellites",
            category: "Original",
            spec: ScreentoneSpec {
                dot_size: 12.0,
                spacing_x: 70.0,
                spacing_y: 60.0,
                density: 0.5,
                satellite_enabled: true,
                satellite_size: 5.0,
                satellite_distance: 22.0,
                satellite_count: 4,
                row_offset: 0.5,
                ..Default::default()
            },
        },
    ]
}

/// The document-space point the lattice anchors to for `spec` over `bounds`.
///
/// The renderer phases its lattice from the centre of `bounds`; adding the spec's rotated phase
/// gives the same anchor in document coordinates. A stored screentone layer keeps this point, so
/// growing the canvas or rendering only the visible tile does not slide the pattern.
pub fn origin_doc(spec: &ScreentoneSpec, bounds: Rect) -> (f32, f32) {
    let cx = bounds.x0 as f32 + bounds.width().max(1) as f32 * 0.5;
    let cy = bounds.y0 as f32 + bounds.height().max(1) as f32 * 0.5;
    let (sin, cos) = spec.rot_canvas.to_radians().sin_cos();
    (cx + spec.origin_x * cos - spec.origin_y * sin, cy + spec.origin_x * sin + spec.origin_y * cos)
}

/// A copy of `spec` whose lattice phase is the document-space `origin` over `bounds`.
///
/// This is the inverse of [`origin_doc`]. `use_source_luminance` is preserved so callers
/// passing a source surface can shade by it; [`render_screentone_to_surface`] turns it off
/// automatically when the target surface carries no source pixels.
pub fn sheet_spec(spec: &ScreentoneSpec, bounds: Rect, origin: (f32, f32)) -> ScreentoneSpec {
    let cx = bounds.x0 as f32 + bounds.width().max(1) as f32 * 0.5;
    let cy = bounds.y0 as f32 + bounds.height().max(1) as f32 * 0.5;
    let (dx, dy) = (origin.0 - cx, origin.1 - cy);
    let (sin, cos) = spec.rot_canvas.to_radians().sin_cos();
    ScreentoneSpec {
        origin_x: dx * cos + dy * sin,
        origin_y: -dx * sin + dy * cos,
        ..spec.clone()
    }
}

/// Render a procedural sheet into `surface`, anchored at the document-space `origin`.
///
/// The whole `bounds` frame is rendered (a layer mask shapes the visible area), and the function
/// returns the document-space area written. When `spec.use_source_luminance` is `true` and
/// `surface` already holds source pixels, the generated dots are modulated by that source's
/// darkness before `surface` is replaced with the rendered sheet; an empty `surface` renders a
/// uniform procedural sheet.
pub fn render_screentone_to_surface(surface: &mut Surface, bounds: Rect, spec: &ScreentoneSpec, origin: (f32, f32)) -> Rect {
    let mut s = sheet_spec(spec, bounds, origin);
    if surface.content_bounds().is_empty() || s.mode == ScreentoneMode::Sheet {
        s.use_source_luminance = false;
    }
    let params = crate::FilterParams::Screentone { spec: s };
    let content = surface.content_bounds().union(&bounds);
    let area = crate::output_area(&params, content, bounds, None).intersect(&bounds);
    if area.is_empty() {
        return area;
    }
    let rendered = crate::apply_in(surface, &params, area, bounds, None, content);
    *surface = Surface::new(surface.format());
    let px = rendered.read_region(area);
    surface.write_region(area, &px);
    surface.prune();
    area
}

// ────────────────────────────────────────────────────────────
// Math & Distance Field Helpers
// ────────────────────────────────────────────────────────────

/// Euclidean distance, bit-equivalent to WGSL `length(vec2f(a, b))`.
///
/// WGSL defines `length(v) = sqrt(dot(v, v))`. For a `vec2f(a, b)` that
/// is `sqrt(a*a + b*b)`. Rust's `f32::hypot(a, b)` is permitted to use
/// a different (overflow-avoiding) algorithm and diverges from this
/// form by 1-2 ULP.
///
/// Every distance in the screentone hot path goes through this
/// helper so the CPU oracle and the WGSL kernel produce identical
/// ULPs.
#[inline]
fn sdist2(a: f32, b: f32) -> f32 {
    (a * a + b * b).sqrt()
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn mid_bias_curve(t: f32, midpoint: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    if midpoint <= 0.001 || midpoint >= 0.999 {
        return t;
    }
    let exponent = 0.5f32.ln() / midpoint.clamp(0.001, 0.999).ln();
    if exponent.is_finite() {
        t.powf(exponent).clamp(0.0, 1.0)
    } else {
        t
    }
}

/// Stable integer grid hash returning [0.0, 1.0).
fn hash_coord(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263) ^ (seed as i32).wrapping_mul(1274126177)) & 0x7fffffff;
    h = ((h ^ (h >> 13)).wrapping_mul(1274126177)) & 0x7fffffff;
    h = (h ^ (h >> 16)) & 0x7fffffff;
    (h as f32) / 2147483647.0
}

/// Box-Muller normal random pair from cell coordinate.
fn hash_gaussian(ix: i32, iy: i32, seed: u32) -> (f32, f32) {
    let u1 = hash_coord(ix, iy, seed).clamp(1e-6, 1.0);
    let u2 = hash_coord(ix.wrapping_add(1000), iy.wrapping_add(1000), seed);
    let r = (-2.0 * u1.ln()).sqrt();
    let theta = 2.0 * std::f32::consts::PI * u2;
    (r * theta.cos(), r * theta.sin())
}

/// Compute gradient factor in [0.0, 1.0] at relative position `(x, y)` from center.
fn compute_grad_factor(x: f32, y: f32, width: f32, height: f32, spec: &ScreentoneSpec) -> f32 {
    if spec.grad_type == ScreentoneGradientType::None {
        return 0.5;
    }
    let rad = spec.grad_angle.to_radians();
    let (dir_y, dir_x) = rad.sin_cos();
    let raw_grad = match spec.grad_type {
        ScreentoneGradientType::Linear => {
            let proj = x * dir_x + y * dir_y;
            proj / (width * 0.8).max(1.0) + 0.5
        }
        ScreentoneGradientType::Radial => {
            let dist = sdist2(x, y);
            dist / (width.min(height) * 0.6).max(1.0)
        }
        ScreentoneGradientType::None => 0.5,
    };
    let mut g = raw_grad.clamp(0.0, 1.0);
    if spec.grad_reverse {
        g = 1.0 - g;
    }
    mid_bias_curve(g, spec.grad_midpoint)
}

/// Distance from point `p` to line segment `a -> b`.
#[inline]
fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let pax = p.0 - a.0;
    let pay = p.1 - a.1;
    let bax = b.0 - a.0;
    let bay = b.1 - a.1;
    let len_sq = bax * bax + bay * bay;
    if len_sq < 1e-8 {
        return sdist2(pax, pay);
    }
    let h = ((pax * bax + pay * bay) / len_sq).clamp(0.0, 1.0);
    let dx = pax - bax * h;
    let dy = pay - bay * h;
    sdist2(dx, dy)
}

/// Signed distance field to a 2D box with rounded corners.
#[inline]
fn sdf_box_rounded(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let r = radius.clamp(0.0, half_w.min(half_h));
    let qx = px.abs() - (half_w - r);
    let qy = py.abs() - (half_h - r);
    sdist2(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Signed distance field for Square dot shape.
#[inline]
fn sdf_square(px: f32, py: f32, size: f32, roundness: f32) -> f32 {
    if roundness >= 0.95 {
        return sdist2(px, py) - size;
    }
    let s = size * 0.8;
    let r = roundness.clamp(0.0, 1.0) * s * 0.5;
    sdf_box_rounded(px, py, s, s, r)
}

/// Signed distance field for Diamond dot shape (rotated rhombus).
#[inline]
fn sdf_diamond(px: f32, py: f32, size: f32, roundness: f32) -> f32 {
    if roundness >= 0.95 {
        return sdist2(px, py) - size;
    }
    let u = (px + py) * std::f32::consts::FRAC_1_SQRT_2;
    let v = (-px + py) * std::f32::consts::FRAC_1_SQRT_2;
    let s = size * std::f32::consts::FRAC_1_SQRT_2;
    let r = roundness.clamp(0.0, 1.0) * s * 0.5;
    sdf_box_rounded(u, v, s, s, r)
}

/// Signed distance field for a general polygon with ray-casting winding check.
fn sdf_polygon(p: (f32, f32), pts: &[[f32; 2]], roundness: f32) -> f32 {
    let n = pts.len();
    if n < 3 {
        return f32::MAX;
    }
    let mut min_d = f32::MAX;
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let vi = (pts[i][0], pts[i][1]);
        let vj = (pts[j][0], pts[j][1]);
        if ((vi.1 > p.1) != (vj.1 > p.1)) && (p.0 < (vj.0 - vi.0) * (p.1 - vi.1) / (vj.1 - vi.1).abs().max(1e-6) + vi.0) {
            inside = !inside;
        }
        let seg_d = dist_to_segment(p, vj, vi);
        if seg_d < min_d {
            min_d = seg_d;
        }
        j = i;
    }
    let signed_d = if inside { -min_d } else { min_d };
    let r = roundness.clamp(0.0, 1.0);
    if r > 0.01 {
        let corner_rad = min_d.min(2.0) * 0.5 * r;
        signed_d - corner_rad
    } else {
        signed_d
    }
}

/// Polygon vertices for Hexagon.
fn hexagon_vertices(size: f32) -> [[f32; 2]; 6] {
    let mut pts = [[0.0f32; 2]; 6];
    for (i, p) in pts.iter_mut().enumerate() {
        let a = (std::f32::consts::PI / 3.0) * i as f32 - std::f32::consts::FRAC_PI_6;
        p[0] = a.cos() * size;
        p[1] = a.sin() * size;
    }
    pts
}

/// Polygon vertices for 5-pointed Star.
fn star_vertices(size: f32) -> [[f32; 2]; 10] {
    let mut pts = [[0.0f32; 2]; 10];
    for (i, p) in pts.iter_mut().enumerate() {
        let r = if i % 2 == 0 { size } else { size * 0.4 };
        let a = (std::f32::consts::PI / 5.0) * i as f32 - std::f32::consts::FRAC_PI_2;
        p[0] = a.cos() * r;
        p[1] = a.sin() * r;
    }
    pts
}

/// Polygon vertices for Equilateral Triangle.
fn triangle_vertices(size: f32) -> [[f32; 2]; 3] {
    [[0.0, -size], [size * 0.8660254, size * 0.5], [-size * 0.8660254, size * 0.5]]
}

/// Polygon vertices for Heart shape curve (28 sample points).
fn heart_vertices(size: f32) -> [[f32; 2]; 28] {
    let mut pts = [[0.0f32; 2]; 28];
    let sc = size / 17.0;
    for (i, p) in pts.iter_mut().enumerate() {
        let t = (i as f32 / 28.0) * std::f32::consts::PI * 2.0;
        let sin_t = t.sin();
        let x = sc * 16.0 * sin_t * sin_t * sin_t;
        let y = -sc * (13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos());
        p[0] = x;
        p[1] = y;
    }
    pts
}

/// Per-pixel shape transform (stretch, corner roundness, pattern rotation) shared by all SDF
/// evaluations, so the helpers stay within the crate's argument limit.
#[derive(Clone, Copy, Debug)]
struct ShapeXform {
    stretch_x: f32,
    stretch_y: f32,
    roundness: f32,
    rot_pattern_rad: f32,
}

/// Evaluates Signed Distance to a specific shape at `(px, py)` with stretch and rotation.
#[inline]
fn eval_shape_sdf(px: f32, py: f32, shape: DotShape, size: f32, xf: ShapeXform) -> f32 {
    if size <= 0.05 {
        return 1e6;
    }
    if (shape == DotShape::Circle || xf.roundness >= 0.95)
        && (xf.stretch_x - 1.0).abs() <= 1e-4
        && (xf.stretch_y - 1.0).abs() <= 1e-4
    {
        return sdist2(px, py) - size;
    }
    let (sin_p, cos_p) = xf.rot_pattern_rad.sin_cos();
    let rx = px * cos_p + py * sin_p;
    let ry = -px * sin_p + py * cos_p;
    let sx = rx / xf.stretch_x.max(0.01);
    let sy = ry / xf.stretch_y.max(0.01);

    if shape == DotShape::Circle || xf.roundness >= 0.95 {
        return sdist2(sx, sy) - size;
    }
    match shape {
        DotShape::Circle => sdist2(sx, sy) - size,
        DotShape::Square => sdf_square(sx, sy, size, xf.roundness),
        DotShape::Diamond => sdf_diamond(sx, sy, size, xf.roundness),
        DotShape::Hexagon => {
            let verts = hexagon_vertices(size);
            sdf_polygon((sx, sy), &verts, xf.roundness)
        }
    }
}

/// Evaluates special pattern shape SDF (star, heart, triangle).
fn eval_special_shape_sdf(px: f32, py: f32, pattern: PatternType, size: f32, xf: ShapeXform) -> f32 {
    if size <= 0.05 {
        return 1e6;
    }
    let (sin_p, cos_p) = xf.rot_pattern_rad.sin_cos();
    let rx = px * cos_p + py * sin_p;
    let ry = -px * sin_p + py * cos_p;
    let sx = rx / xf.stretch_x.max(0.01);
    let sy = ry / xf.stretch_y.max(0.01);

    match pattern {
        PatternType::Stars => {
            let verts = star_vertices(size);
            sdf_polygon((sx, sy), &verts, xf.roundness)
        }
        PatternType::Hearts => {
            let verts = heart_vertices(size);
            sdf_polygon((sx, sy), &verts, xf.roundness)
        }
        PatternType::Triangles => {
            let verts = triangle_vertices(size);
            sdf_polygon((sx, sy), &verts, xf.roundness)
        }
        _ => sdist2(sx, sy) - size,
    }
}

/// Convert Signed Distance to coverage in [0.0, 1.0].
#[inline]
fn coverage_from_sdf(dist: f32, smoothness: f32) -> f32 {
    if smoothness < 0.05 {
        if dist <= 0.0 { 1.0 } else { 0.0 }
    } else {
        ((0.5 - dist) / smoothness).clamp(0.0, 1.0)
    }
}

// ────────────────────────────────────────────────────────────
// Screentone Kernel Execution
// ────────────────────────────────────────────────────────────

/// Core screentone rendering kernel over tile `out`.
pub fn screentone(src: &Image, out: Rect, ctx: &Ctx, spec: &ScreentoneSpec) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    let b = ctx.bounds;
    let (bw, bh) = (b.width().max(1) as f32, b.height().max(1) as f32);
    let (cx, cy) = (b.x0 as f32 + bw * 0.5, b.y0 as f32 + bh * 0.5);

    let rot_canvas_rad = spec.rot_canvas.to_radians();
    let (sin_canvas, cos_canvas) = rot_canvas_rad.sin_cos();
    let rot_angle_rad = spec.angle.to_radians();
    let (sin_ang, cos_ang) = rot_angle_rad.sin_cos();
    let rot_pattern_rad = spec.rot_pattern.to_radians();
    let rot_sat_rad = spec.satellite_angle.to_radians();
    let (sin_sat, cos_sat) = rot_sat_rad.sin_cos();

    let spacing_x = spec.spacing_x.clamp(1.0, 2000.0);
    let spacing_y = spec.spacing_y.clamp(1.0, 2000.0);

    let fg = spec.foreground;
    let bg = spec.background;

    let use_shading = spec.use_source_luminance && spec.mode == ScreentoneMode::Shading && !src.rect.is_empty();
    let brightness = spec.brightness.clamp(-1.0, 1.0);
    let contrast = spec.contrast.clamp(0.0, 4.0);
    let white_thresh = spec.white_threshold.clamp(0.0, 1.0);
    let black_thresh = spec.black_threshold.clamp(0.0, white_thresh);
    let gamma = spec.tone_gamma.clamp(0.01, 10.0);
    let line_thresh = spec.line_threshold.clamp(0.0, 1.0);
    let line_thick = spec.line_thickness.clamp(0.5, 5.0);

    // Raw luminance at integer pixel `(ix, iy)` composited over white paper.
    let raw_paper_luma = |ix: i32, iy: i32| -> f32 {
        if src.rect.is_empty() {
            return 0.5;
        }
        let cx = ix.clamp(src.rect.x0, src.rect.x1.saturating_sub(1));
        let cy = iy.clamp(src.rect.y0, src.rect.y1.saturating_sub(1));
        if src.rect.contains(cx, cy) {
            let px = src.px(cx, cy);
            let a = if ctx.alpha { px.last().copied().unwrap_or(1.0).clamp(0.0, 1.0) } else { 1.0 };
            if a <= 1e-3 {
                return 1.0;
            }
            let raw_luma = luma(ctx, px).clamp(0.0, 1.0);
            (raw_luma * a + (1.0 - a)).clamp(0.0, 1.0)
        } else {
            1.0
        }
    };

    let luma_to_ink = |luma_eff: f32| -> f32 {
        let g = (luma_eff + brightness).clamp(0.0, 1.0);
        let g = g.powf(gamma);
        let luma_adj = ((g - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
        if luma_adj >= white_thresh {
            0.0
        } else if luma_adj <= black_thresh {
            1.0
        } else {
            ((white_thresh - luma_adj) / (white_thresh - black_thresh).max(1e-5)).clamp(0.0, 1.0)
        }
    };

    // Precompute neighborhood grids over `work_rect` when shading is active so line-art
    // detection, dilation, and morphological tone de-lining run once per pixel in O(1).
    let max_sp = spacing_x.max(spacing_y).clamp(1.0, 256.0);
    let reach = (max_sp * 1.5).ceil() as i32 + 16;
    let work_rect = if use_shading {
        out.inflate(reach).intersect(&src.rect)
    } else {
        Rect::new(0, 0, 0, 0)
    };
    let ww = work_rect.width().max(0) as usize;
    let wh = work_rect.height().max(0) as usize;

    let mut luma_grid = Vec::new();
    let mut line_edge_grid = Vec::new();
    let mut preserved_grid = Vec::new();
    let mut near_line_grid = Vec::new();
    let mut ambient_ink_grid = Vec::new();

    const NEAR_2PX: u8 = 4;

    if use_shading && ww > 0 && wh > 0 {
        luma_grid.resize(ww * wh, 1.0f32);
        for wy in 0..wh {
            let iy = work_rect.y0 + wy as i32;
            let row_off = wy * ww;
            for wx in 0..ww {
                let ix = work_rect.x0 + wx as i32;
                luma_grid[row_off + wx] = raw_paper_luma(ix, iy);
            }
        }

        let grid_luma = |ix: i32, iy: i32| -> f32 {
            let cx = ix.clamp(work_rect.x0, work_rect.x1 - 1) - work_rect.x0;
            let cy = iy.clamp(work_rect.y0, work_rect.y1 - 1) - work_rect.y0;
            luma_grid[cy as usize * ww + cx as usize]
        };

        if spec.preserve_lines {
            line_edge_grid.resize(ww * wh, 0u8);
            preserved_grid.resize(ww * wh, 0u8);
            near_line_grid.resize(ww * wh, 0u8);

            let thresh = line_thresh.clamp(0.02, 0.72);
            const AXES_4: [(i32, i32); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

            // 1. Detect dark ink line cores and 4-axis monotonic 1D valleys (catching both thick
            //    black contours and thin 1..3px diagonal/cardinal hatch strokes while ignoring
            //    anti-aliased outer fringes, flat tones, and un-outlined shadow edges).
            for wy in 0..wh {
                let iy = work_rect.y0 + wy as i32;
                let row_off = wy * ww;
                for wx in 0..ww {
                    let ix = work_rect.x0 + wx as i32;
                    let l_c = luma_grid[row_off + wx];
                    if l_c >= 0.72 {
                        continue;
                    }
                    let l_l = grid_luma(ix - 1, iy);
                    let l_r = grid_luma(ix + 1, iy);
                    let l_u = grid_luma(ix, iy - 1);
                    let l_d = grid_luma(ix, iy + 1);
                    let l_ul = grid_luma(ix - 1, iy - 1);
                    let l_ur = grid_luma(ix + 1, iy - 1);
                    let l_dl = grid_luma(ix - 1, iy + 1);
                    let l_dr = grid_luma(ix + 1, iy + 1);

                    let nb_max = l_l
                        .max(l_r)
                        .max(l_u)
                        .max(l_d)
                        .max(l_ul)
                        .max(l_ur)
                        .max(l_dl)
                        .max(l_dr)
                        .max(grid_luma(ix - 2, iy))
                        .max(grid_luma(ix + 2, iy))
                        .max(grid_luma(ix, iy - 2))
                        .max(grid_luma(ix, iy + 2))
                        .max(grid_luma(ix - 2, iy - 2))
                        .max(grid_luma(ix + 2, iy - 2))
                        .max(grid_luma(ix - 2, iy + 2))
                        .max(grid_luma(ix + 2, iy + 2));

                    let local_dip = (nb_max - l_c).max(0.0);
                    let contrast_nb = (l_l - l_c)
                        .max(l_r - l_c)
                        .max(l_u - l_c)
                        .max(l_d - l_c)
                        .max(l_ul - l_c)
                        .max(l_ur - l_c)
                        .max(l_dl - l_c)
                        .max(l_dr - l_c)
                        .max(0.0);
                    let edge_mag = sdist2(l_r - l_l, l_d - l_u)
                        .max(sdist2(l_dr - l_ul, l_ur - l_dl))
                        .max(contrast_nb);

                    let mut max_valley = 0.0f32;
                    for &(dx, dy) in &AXES_4 {
                        let mut l_minus = l_c;
                        for s in 1..=3 {
                            let v = grid_luma(ix - dx * s, iy - dy * s);
                            if v < l_c - 0.08 {
                                break;
                            }
                            if v > l_minus {
                                l_minus = v;
                            }
                        }
                        let mut l_plus = l_c;
                        for s in 1..=3 {
                            let v = grid_luma(ix + dx * s, iy + dy * s);
                            if v < l_c - 0.08 {
                                break;
                            }
                            if v > l_plus {
                                l_plus = v;
                            }
                        }
                        let valley = (l_minus.min(l_plus) - l_c).max(0.0);
                        if valley > max_valley {
                            max_valley = valley;
                        }
                    }

                    let is_edge = (l_c <= thresh && (edge_mag > 0.04 || local_dip > 0.05 || l_c <= thresh * 0.85))
                        || (l_c < 0.65 && max_valley > (0.14 - thresh * 0.20).max(0.06))
                        || (l_c < 0.62 && edge_mag > (0.22 - thresh * 0.35).max(0.06) && local_dip > 0.08 && max_valley > 0.03);
                    if is_edge {
                        line_edge_grid[row_off + wx] = 1;
                    }
                }
            }

            // 2. Dilate detected line edges by `line_thickness` radius (0.5..=5.0 px).
            let r_px = (line_thick - 0.5).max(0.0);
            let rad = r_px.ceil() as i32;
            if rad <= 0 {
                preserved_grid.copy_from_slice(&line_edge_grid);
            } else {
                let r_sq = r_px * r_px + 0.25;
                for wy in 0..wh {
                    let iy = wy as i32;
                    let row_off = wy * ww;
                    for wx in 0..ww {
                        let ix = wx as i32;
                        let mut hit = false;
                        'dilate: for dy in -rad..=rad {
                            let ny = iy + dy;
                            if ny < 0 || ny >= wh as i32 {
                                continue;
                            }
                            for dx in -rad..=rad {
                                let nx = ix + dx;
                                if nx < 0 || nx >= ww as i32 {
                                    continue;
                                }
                                if (dx * dx + dy * dy) as f32 <= r_sq
                                    && line_edge_grid[ny as usize * ww + nx as usize] != 0
                                {
                                    hit = true;
                                    break 'dilate;
                                }
                            }
                        }
                        if hit {
                            preserved_grid[row_off + wx] = 1;
                        }
                    }
                }
            }

            // 3. Build proximity bitmask (`NEAR_2PX`) around core lines (1..=2 px radius).
            for wy in 0..wh {
                let iy = wy as i32;
                let row_off = wy * ww;
                for wx in 0..ww {
                    let ix = wx as i32;
                    let mut mask = 0u8;
                    'near: for dy in -2i32..=2 {
                        let ny = iy + dy;
                        if ny < 0 || ny >= wh as i32 {
                            continue;
                        }
                        for dx in -2i32..=2 {
                            if dx * dx + dy * dy > 4 {
                                continue;
                            }
                            let nx = ix + dx;
                            if nx < 0 || nx >= ww as i32 {
                                continue;
                            }
                            let n_idx = ny as usize * ww + nx as usize;
                            if line_edge_grid[n_idx] != 0 || preserved_grid[n_idx] != 0 {
                                mask |= NEAR_2PX;
                                break 'near;
                            }
                        }
                    }
                    near_line_grid[row_off + wx] = mask;
                }
            }
        }

        // 4. Reconstruct the de-lined ambient tone map (`ambient_ink_grid`).
        //    - Any pixel more than 2px from a dark line core (`!NEAR_2PX`) uses its raw luminance
        //      untouched, so white outlines/halos never bleed into adjacent tones.
        //    - Pixels within 1..2px of a dark line core (`NEAR_2PX`) step 1..2px directly away
        //      from the adjacent line core (`(-dx.signum(), -dy.signum())`) to sample the true
        //      underlying tone outside the line's anti-aliased ramp.
        ambient_ink_grid.resize(ww * wh, 0.0f32);
        const AXES_4: [(i32, i32); 4] = [(1, 0), (0, 1), (1, 1), (1, -1)];

        let is_core_idx = |wx: i32, wy: i32| -> bool {
            let cx = wx.clamp(0, ww as i32 - 1) as usize;
            let cy = wy.clamp(0, wh as i32 - 1) as usize;
            let idx = cy * ww + cx;
            line_edge_grid[idx] != 0 || preserved_grid[idx] != 0
        };

        for wy in 0..wh {
            let row_off = wy * ww;
            for wx in 0..ww {
                let idx = row_off + wx;
                let l0 = luma_grid[idx];
                if !spec.preserve_lines || (near_line_grid[idx] & NEAR_2PX) == 0 {
                    ambient_ink_grid[idx] = luma_to_ink(l0);
                    continue;
                }

                let ix = wx as i32;
                let iy = wy as i32;
                let mut best_luma = l0;

                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        if (dx == 0 && dy == 0) || dx * dx + dy * dy > 4 {
                            continue;
                        }
                        if is_core_idx(ix + dx, iy + dy) {
                            let ux = -dx.signum();
                            let uy = -dy.signum();
                            let nx1 = (ix + ux).clamp(0, ww as i32 - 1);
                            let ny1 = (iy + uy).clamp(0, wh as i32 - 1);
                            if !is_core_idx(nx1, ny1) {
                                let l1 = luma_grid[ny1 as usize * ww + nx1 as usize];
                                if l1 > best_luma {
                                    best_luma = l1;
                                }
                                let nx2 = (ix + ux * 2).clamp(0, ww as i32 - 1);
                                let ny2 = (iy + uy * 2).clamp(0, wh as i32 - 1);
                                if !is_core_idx(nx2, ny2) {
                                    let l2 = luma_grid[ny2 as usize * ww + nx2 as usize];
                                    if l2 > best_luma {
                                        best_luma = l2;
                                    }
                                }
                            }
                        }
                    }
                }

                // If `(ix, iy)` is sandwiched in a narrow 1..3px slit between two dark lines
                // (lines within 1..3px on both opposite sides), compensate for overlapping AA.
                let mut in_tight_slit = false;
                for &(ax, ay) in &AXES_4 {
                    let neg_hit = (1..=3).any(|s| is_core_idx(ix - ax * s, iy - ay * s));
                    let pos_hit = (1..=3).any(|s| is_core_idx(ix + ax * s, iy + ay * s));
                    if neg_hit && pos_hit {
                        in_tight_slit = true;
                        break;
                    }
                }
                if in_tight_slit && best_luma >= 0.50 {
                    best_luma = (best_luma + 0.28).min(1.0);
                }

                ambient_ink_grid[idx] = luma_to_ink(best_luma);
            }
        }
    }

    let grid_idx = |ix: i32, iy: i32| -> usize {
        let cx = (ix.clamp(work_rect.x0, work_rect.x1 - 1) - work_rect.x0) as usize;
        let cy = (iy.clamp(work_rect.y0, work_rect.y1 - 1) - work_rect.y0) as usize;
        cy * ww + cx
    };

    let is_preserved_line = |x: i32, y: i32| -> bool {
        if !spec.preserve_lines || preserved_grid.is_empty() {
            return false;
        }
        preserved_grid[grid_idx(x, y)] != 0
    };

    let sample_ink_at_int = |ix: i32, iy: i32| -> f32 {
        if !ambient_ink_grid.is_empty() {
            ambient_ink_grid[grid_idx(ix, iy)]
        } else {
            luma_to_ink(raw_paper_luma(ix, iy))
        }
    };

    let sample_ink = |doc_x: f32, doc_y: f32| -> f32 {
        sample_ink_at_int(doc_x.floor() as i32, doc_y.floor() as i32)
    };

    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let doc_x = x as f32 + 0.5;
        let doc_y = y as f32 + 0.5;

        // Vector from center in rotated frame
        let rel_x = doc_x - cx;
        let rel_y = doc_y - cy;
        let local_x = rel_x * cos_canvas + rel_y * sin_canvas;
        let local_y = -rel_x * sin_canvas + rel_y * cos_canvas;

        // Gradient factor at this coordinate
        let gf = compute_grad_factor(local_x, local_y, bw, bh, spec);
        let mut mod_size = 1.0f32;
        let mut mod_aspect_x = spec.aspect_x;
        let mut mod_aspect_y = spec.aspect_y;

        if spec.grad_type != ScreentoneGradientType::None {
            mod_size = lerp(spec.grad_size_start, spec.grad_size_end, gf);
            let stretch_mod = lerp(spec.grad_stretch_start, spec.grad_stretch_end, gf);
            mod_aspect_x *= stretch_mod;
            mod_aspect_y *= stretch_mod;
        }

        let cur_fg = if spec.grad_color_trans && spec.grad_type != ScreentoneGradientType::None {
            [lerp(fg[0], bg[0], gf), lerp(fg[1], bg[1], gf), lerp(fg[2], bg[2], gf), fg[3]]
        } else {
            fg
        };

        let xf = ShapeXform {
            stretch_x: mod_aspect_x,
            stretch_y: mod_aspect_y,
            roundness: spec.roundness,
            rot_pattern_rad,
        };

        // A shifted origin phase-moves the whole lattice (cells, dots, satellites, line
        // stripes). The gradient factor and the ink sampling above were already computed from
        // the unshifted coordinates, so the origin only slides the pattern, never its shading.
        let (local_x, local_y) = (local_x - spec.origin_x, local_y - spec.origin_y);

        // Rotate the lattice by `spec.angle` (Grid Rotation) around the phase-shifted origin so
        // Dots, Hexgrid, Checker, Stipple, Stars, Hearts, Triangles, Lines and Crosshatch all
        // rotate smoothly at any angle (e.g. classic 45° Polka Dot).
        let (grid_x, grid_y) = if spec.angle == 0.0 {
            (local_x, local_y)
        } else {
            (local_x * cos_ang + local_y * sin_ang, -local_x * sin_ang + local_y * cos_ang)
        };
        let mut cov = 0.0f32;

        if use_shading && is_preserved_line(x, y) {
            // Preserved line-art pixels emit solid 100% ink directly without evaluating the 3x3
            // halftone lattice.
            cov = 1.0;
        } else {
            let ink = if use_shading { sample_ink(doc_x, doc_y) } else { 1.0 };
            if ink > 1e-5 {
                match spec.pattern_type {
                    PatternType::Lines => {
                        let ly = grid_y;
                        let width = (spec.line_width * mod_size * spec.density * ink).max(0.0);
                        if width > 1e-4 {
                            let sp = spacing_y.max(1.0);
                            let dist = (((ly % sp) + sp) % sp - sp * 0.5).abs() - width * 0.5;
                            cov = coverage_from_sdf(dist, spec.smoothness);
                        }
                    }
                    PatternType::Crosshatch => {
                        let a2_rad = (spec.angle + spec.cross_angle).to_radians();
                        let (sin2, cos2) = a2_rad.sin_cos();
                        let ly1 = grid_y;
                        let ly2 = -local_x * sin2 + local_y * cos2;
                        let width = (spec.line_width * mod_size * spec.density * ink).max(0.0);
                        if width > 1e-4 {
                            let sp = spacing_y.max(1.0);
                            let d1 = (((ly1 % sp) + sp) % sp - sp * 0.5).abs() - width * 0.5;
                            let d2 = (((ly2 % sp) + sp) % sp - sp * 0.5).abs() - width * 0.5;
                            let c1 = coverage_from_sdf(d1, spec.smoothness);
                            let c2 = coverage_from_sdf(d2, spec.smoothness);
                            cov = c1.max(c2);
                        }
                    }
                    PatternType::Concentric => {
                        let r = sdist2(grid_x, grid_y);
                        let width = (spec.line_width * mod_size * spec.density * ink).max(0.0);
                        if width > 1e-4 {
                            let sp = spacing_x.max(1.0);
                            let dist = (((r % sp) + sp) % sp - sp * 0.5).abs() - width * 0.5;
                            cov = coverage_from_sdf(dist, spec.smoothness);
                        }
                    }
                    PatternType::Checker => {
                        let ix = (grid_x / spacing_x).floor() as i32;
                        let iy = (grid_y / spacing_y).floor() as i32;
                        if ((ix.wrapping_add(iy)) & 1) == 0 {
                            let cell_cx = (ix as f32 + 0.5) * spacing_x;
                            let cell_cy = (iy as f32 + 0.5) * spacing_y;
                            let size = (spacing_x * 0.45 * mod_size * spec.density * ink).max(0.0);
                            if size > 1e-4 {
                                let dist = eval_shape_sdf(grid_x - cell_cx, grid_y - cell_cy, DotShape::Square, size, xf);
                                cov = coverage_from_sdf(dist, spec.smoothness);
                            }
                        }
                    }
                    PatternType::Hexgrid => {
                        let hy = spacing_y * 0.8660254;
                        let cy_idx = (grid_y / hy).round() as i32;
                        let size = (spec.dot_size * mod_size * spec.density * ink).max(0.0);
                        for dy in -1..=1 {
                            let iy = cy_idx + dy;
                            let cell_y = iy as f32 * hy;
                            let row_shift = if (iy & 1) != 0 { spacing_x * 0.5 } else { 0.0 };
                            let cx_idx = ((grid_x - row_shift) / spacing_x).round() as i32;
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let cell_x = ix as f32 * spacing_x + row_shift;
                                let dist = eval_shape_sdf(grid_x - cell_x, grid_y - cell_y, DotShape::Hexagon, size, xf);
                                let cell_cov = coverage_from_sdf(dist, spec.smoothness);
                                cov = cov.max(cell_cov);
                            }
                        }
                    }
                    PatternType::Noise => {
                        let seed = spec.seed;
                        let cx_idx = (grid_x / spacing_x).floor() as i32;
                        let cy_idx = (grid_y / spacing_y).floor() as i32;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let iy = cy_idx + dy;
                                let h1 = hash_coord(ix, iy, seed);
                                let h2 = hash_coord(ix.wrapping_add(300), iy.wrapping_add(300), seed);
                                let h3 = hash_coord(ix.wrapping_add(700), iy.wrapping_add(700), seed);
                                if h1 > spec.density {
                                    continue;
                                }
                                let jx = (h2 - 0.5) * spacing_x * (spec.jitter_pos * 0.01 + 0.8);
                                let jy = (h3 - 0.5) * spacing_y * (spec.jitter_pos * 0.01 + 0.8);
                                let dot_cx = (ix as f32 + 0.5) * spacing_x + jx;
                                let dot_cy = (iy as f32 + 0.5) * spacing_y + jy;
                                let sz = (spec.dot_size * (1.0 + (h2 - 0.5) * spec.jitter_size * 0.02) * mod_size * ink).max(0.0);
                                let dist = sdist2(grid_x - dot_cx, grid_y - dot_cy) - sz;
                                cov = cov.max(coverage_from_sdf(dist, spec.smoothness));
                            }
                        }
                    }
                    PatternType::GaussianNoise => {
                        let seed = spec.seed.wrapping_add(137);
                        let cx_idx = (grid_x / spacing_x).floor() as i32;
                        let cy_idx = (grid_y / spacing_y).floor() as i32;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let iy = cy_idx + dy;
                                let h1 = hash_coord(ix, iy, seed);
                                if h1 > spec.density {
                                    continue;
                                }
                                let (gx, gy) = hash_gaussian(ix, iy, seed);
                                let dot_cx = (ix as f32 + 0.5) * spacing_x + gx * spacing_x * 0.4;
                                let dot_cy = (iy as f32 + 0.5) * spacing_y + gy * spacing_y * 0.4;
                                let sz = (spec.dot_size * (0.6 + h1 * 0.8) * mod_size * ink).max(0.0);
                                let dist = sdist2(grid_x - dot_cx, grid_y - dot_cy) - sz;
                                cov = cov.max(coverage_from_sdf(dist, spec.smoothness));
                            }
                        }
                    }
                    PatternType::Stipple => {
                        let seed = spec.seed.wrapping_add(999);
                        let cx_idx = (grid_x / spacing_x).floor() as i32;
                        let cy_idx = (grid_y / spacing_y).floor() as i32;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let iy = cy_idx + dy;
                                let h_keep = hash_coord(ix, iy, seed);
                                if h_keep > spec.density * ink {
                                    continue;
                                }
                                let h_jx = hash_coord(ix.wrapping_add(100), iy, seed);
                                let h_jy = hash_coord(ix.wrapping_add(200), iy, seed);
                                let h_sz = hash_coord(ix.wrapping_add(500), iy, seed);

                                let mut dot_cx = ix as f32 * spacing_x + (h_jx - 0.5) * spec.jitter_pos * 0.005 * spacing_x;
                                let dot_cy = iy as f32 * spacing_y + (h_jy - 0.5) * spec.jitter_pos * 0.005 * spacing_y;
                                if (iy & 1) != 0 {
                                    dot_cx += spacing_x * spec.row_offset;
                                }

                                let sz = (spec.dot_size * (0.5 + h_sz * 0.5) * mod_size).max(0.0);
                                let dist = eval_shape_sdf(grid_x - dot_cx, grid_y - dot_cy, spec.dot_shape, sz, xf);
                                cov = cov.max(coverage_from_sdf(dist, spec.smoothness));
                            }
                        }
                    }
                    PatternType::Stars | PatternType::Hearts | PatternType::Triangles => {
                        let cx_idx = (grid_x / spacing_x).floor() as i32;
                        let cy_idx = (grid_y / spacing_y).floor() as i32;
                        let sz = (spec.dot_size * mod_size * spec.density * ink).max(0.0);
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let iy = cy_idx + dy;
                                let mut dot_cx = ix as f32 * spacing_x;
                                let mut dot_cy = iy as f32 * spacing_y;
                                if (iy & 1) != 0 {
                                    dot_cx += spacing_x * spec.row_offset;
                                }
                                if spec.jitter_pos > 0.0 {
                                    dot_cx += (hash_coord(ix, iy, spec.seed) - 0.5) * spec.jitter_pos * 0.005 * spacing_x;
                                    dot_cy += (hash_coord(ix.wrapping_add(1000), iy, spec.seed) - 0.5) * spec.jitter_pos * 0.005 * spacing_y;
                                }
                                let dist = eval_special_shape_sdf(grid_x - dot_cx, grid_y - dot_cy, spec.pattern_type, sz, xf);
                                cov = cov.max(coverage_from_sdf(dist, spec.smoothness));
                            }
                        }
                    }
                    PatternType::Dots => {
                        // Default Dots + Satellite system (per-pixel ink modulation ensures zero
                        // coarse cell staircases along white outlines and thick lines).
                        let cy_idx = (grid_y / spacing_y).floor() as i32;
                        let merged_size = spec.dot_size + spec.merge_factor * spacing_x * 0.2;
                        let psize = (merged_size * mod_size * spec.density * ink).max(0.0);
                        let ssize = ((spec.satellite_size + spec.merge_factor * spacing_x * 0.4) * mod_size * spec.density * ink).max(0.0);
                        let sdist = spec.satellite_distance * (1.0 - spec.merge_factor * 0.6);

                        for dy in -1..=1 {
                            let iy = cy_idx + dy;
                            let mut row_shift = 0.0f32;
                            if (iy & 1) != 0 {
                                row_shift = spacing_x * spec.row_offset;
                            }
                            let cx_idx = ((grid_x - row_shift) / spacing_x).floor() as i32;
                            for dx in -1..=1 {
                                let ix = cx_idx + dx;
                                let mut dot_cx = ix as f32 * spacing_x + row_shift;
                                let mut dot_cy = iy as f32 * spacing_y;

                                if spec.jitter_pos > 0.0 {
                                    dot_cx += (hash_coord(ix, iy, spec.seed) - 0.5) * spec.jitter_pos * 0.005 * spacing_x;
                                    dot_cy += (hash_coord(ix.wrapping_add(1000), iy, spec.seed) - 0.5) * spec.jitter_pos * 0.005 * spacing_y;
                                }

                                let mut final_size = psize;
                                if spec.jitter_size > 0.0 {
                                    let jval = hash_coord(ix.wrapping_add(5000), iy.wrapping_add(5000), spec.seed);
                                    final_size *= (1.0 + (jval - 0.5) * spec.jitter_size * 0.02).max(0.0);
                                }

                                // Main dot
                                let dist_main = eval_shape_sdf(grid_x - dot_cx, grid_y - dot_cy, spec.dot_shape, final_size, xf);
                                cov = cov.max(coverage_from_sdf(dist_main, spec.smoothness));

                                // Satellites
                                if spec.satellite_enabled && ssize > 0.05 {
                                    let sat_offsets: [(f32, f32); 4] = [
                                        (0.0, -sdist),
                                        (0.0, sdist),
                                        (sdist, 0.0),
                                        (-sdist, 0.0),
                                    ];
                                    let sat_count = (spec.satellite_count as usize).min(4);
                                    for &(ox, oy) in sat_offsets.iter().take(sat_count) {
                                        let rsx = ox * cos_sat - oy * sin_sat;
                                        let rsy = ox * sin_sat + oy * cos_sat;
                                        let sat_x = dot_cx + rsx;
                                        let sat_y = dot_cy + rsy;

                                        let mut spx = grid_x - sat_x;
                                        let mut spy = grid_y - sat_y;

                                        if spec.satellite_stretch > 0.0 && (rsx != 0.0 || rsy != 0.0) {
                                            let stretch_dir = (-rsy).atan2(-rsx);
                                            let (s_sin, s_cos) = stretch_dir.sin_cos();
                                            let rot_sx = spx * s_cos + spy * s_sin;
                                            let rot_sy = -spx * s_sin + spy * s_cos;
                                            let scaled_sx = rot_sx / (1.0 + spec.satellite_stretch).max(0.01);
                                            spx = scaled_sx * s_cos - rot_sy * s_sin;
                                            spy = scaled_sx * s_sin + rot_sy * s_cos;
                                        }

                                        let dist_sat = eval_shape_sdf(spx, spy, spec.satellite_dot_shape, ssize, xf);
                                        cov = cov.max(coverage_from_sdf(dist_sat, spec.smoothness));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        cov = cov.clamp(0.0, 1.0);

        // Blend pattern output:
        let out_color = if bg[3] <= 0.001 {
            if ctx.alpha {
                // Transparent background on an alpha surface: emit only the screentone ink so
                // halftone sheets and smart filters do not leave the unmodulated source behind
                // between the dots.
                let alpha = cov * cur_fg[3];
                if alpha <= 1e-6 {
                    [0.0, 0.0, 0.0, 0.0]
                } else {
                    [cur_fg[0], cur_fg[1], cur_fg[2], alpha]
                }
            } else {
                // Opaque surface without an alpha channel: blend pattern foreground over base.
                let base_c = rgba(ctx, px);
                let alpha = cov * cur_fg[3];
                [
                    lerp(base_c[0], cur_fg[0], alpha),
                    lerp(base_c[1], cur_fg[1], alpha),
                    lerp(base_c[2], cur_fg[2], alpha),
                    base_c[3].max(alpha),
                ]
            }
        } else {
            // Opaque or specified background: blend between background and foreground
            [
                lerp(bg[0], cur_fg[0], cov),
                lerp(bg[1], cur_fg[1], cov),
                lerp(bg[2], cur_fg[2], cov),
                lerp(bg[3], cur_fg[3], cov),
            ]
        };

        set_rgba(ctx, px, out_color);
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;

    fn make_test_image(w: i32, h: i32, value: f32) -> Image {
        let rect = Rect::new(0, 0, w, h);
        Image { rect, ch: 3, data: vec![value; (w * h * 3) as usize] }
    }

    #[test]
    fn test_all_12_patterns_render_safely() {
        let bounds = Rect::new(0, 0, 64, 64);
        let img = make_test_image(64, 64, 0.5);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };

        let patterns = [
            PatternType::Dots,
            PatternType::Lines,
            PatternType::Crosshatch,
            PatternType::Noise,
            PatternType::GaussianNoise,
            PatternType::Stipple,
            PatternType::Hexgrid,
            PatternType::Checker,
            PatternType::Concentric,
            PatternType::Stars,
            PatternType::Hearts,
            PatternType::Triangles,
        ];

        for pattern in patterns {
            let spec = ScreentoneSpec {
                pattern_type: pattern,
                dot_size: 4.0,
                spacing_x: 12.0,
                spacing_y: 12.0,
                density: 0.5,
                use_source_luminance: false,
                ..Default::default()
            };
            let out_px = screentone(&img, bounds, &ctx, &spec);
            assert_eq!(out_px.len(), 64 * 64 * 3);
            for (idx, val) in out_px.iter().enumerate() {
                assert!(val.is_finite(), "Pattern {pattern:?} produced non-finite sample at {idx}");
                assert!(*val >= -0.01 && *val <= 1.01, "Pattern {pattern:?} sample {val} out of bounds at {idx}");
            }
        }
    }

    #[test]
    fn test_dot_shapes_and_roundness() {
        let bounds = Rect::new(0, 0, 32, 32);
        let img = make_test_image(32, 32, 0.0);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };

        let shapes = [DotShape::Circle, DotShape::Square, DotShape::Diamond, DotShape::Hexagon];
        for shape in shapes {
            for roundness in [0.0, 0.3, 0.7, 1.0] {
                let spec = ScreentoneSpec {
                    pattern_type: PatternType::Dots,
                    dot_shape: shape,
                    roundness,
                    dot_size: 5.0,
                    spacing_x: 16.0,
                    spacing_y: 16.0,
                    ..Default::default()
                };
                let out_px = screentone(&img, bounds, &ctx, &spec);
                assert_eq!(out_px.len(), 32 * 32 * 3);
            }
        }
    }

    #[test]
    fn test_satellites_orbit() {
        let bounds = Rect::new(0, 0, 48, 48);
        let img = make_test_image(48, 48, 0.5);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };

        let spec = ScreentoneSpec {
            pattern_type: PatternType::Dots,
            dot_size: 6.0,
            spacing_x: 24.0,
            spacing_y: 24.0,
            satellite_enabled: true,
            satellite_size: 3.0,
            satellite_distance: 8.0,
            satellite_count: 4,
            satellite_stretch: 0.5,
            satellite_angle: 30.0,
            ..Default::default()
        };
        let out_px = screentone(&img, bounds, &ctx, &spec);
        assert_eq!(out_px.len(), 48 * 48 * 3);
        assert!(out_px.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn test_gradients_linear_and_radial() {
        let bounds = Rect::new(0, 0, 40, 40);
        let img = make_test_image(40, 40, 0.5);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };

        for grad in [ScreentoneGradientType::Linear, ScreentoneGradientType::Radial] {
            let spec = ScreentoneSpec {
                grad_type: grad,
                grad_angle: 45.0,
                grad_size_start: 2.0,
                grad_size_end: 0.1,
                grad_color_trans: true,
                ..Default::default()
            };
            let out_px = screentone(&img, bounds, &ctx, &spec);
            assert_eq!(out_px.len(), 40 * 40 * 3);
            assert!(out_px.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn test_tile_invariance() {
        let bounds = Rect::new(0, 0, 64, 64);
        let img = make_test_image(64, 64, 0.4);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };

        let spec = ScreentoneSpec {
            pattern_type: PatternType::Dots,
            dot_size: 5.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            density: 0.5,
            use_source_luminance: false,
            ..Default::default()
        };

        // Render whole tile
        let whole = screentone(&img, bounds, &ctx, &spec);

        // Render in 4 sub-tiles of 32x32
        let tiles = [
            (screentone(&img, Rect::new(0, 0, 32, 32), &ctx, &spec), 0, 0),
            (screentone(&img, Rect::new(32, 0, 64, 32), &ctx, &spec), 32, 0),
            (screentone(&img, Rect::new(0, 32, 32, 64), &ctx, &spec), 0, 32),
            (screentone(&img, Rect::new(32, 32, 64, 64), &ctx, &spec), 32, 32),
        ];

        for (tile, ox, oy) in tiles {
            for y in 0..32 {
                for x in 0..32 {
                    for c in 0..3 {
                        let whole_idx = (((y + oy) * 64 + (x + ox)) * 3 + c) as usize;
                        let tile_idx = ((y * 32 + x) * 3 + c) as usize;
                        assert!(
                            (whole[whole_idx] - tile[tile_idx]).abs() < 1e-5,
                            "Mismatch between whole and tiled output at ({}, {}) channel {c}",
                            x + ox,
                            y + oy
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_presets_validity() {
        let presets = built_in_presets();
        assert_eq!(presets.len(), 19);
        for p in presets {
            assert!(!p.name.is_empty());
            assert!(!p.category.is_empty());
            assert!(p.spec.spacing_x > 0.0);
            assert!(p.spec.spacing_y > 0.0);
        }
    }

    #[test]
    fn test_origin_shifts_the_lattice_phase() {
        let bounds = Rect::new(0, 0, 96, 48);
        let img = make_test_image(96, 48, 0.5);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };
        let base = ScreentoneSpec {
            pattern_type: PatternType::Dots,
            dot_size: 5.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            use_source_luminance: false,
            ..Default::default()
        };
        let plain = screentone(&img, bounds, &ctx, &base);

        // A period-16 lattice queried through `local - origin` sees exactly the same cells a
        // plain render computes 16 px earlier, so an origin of (16, 0) slides the sheet a full
        // tile: the shifted output at x equals the plain output at x - 16 (dyadic f32 math,
        // bit-identical).
        let shifted_x = screentone(&img, bounds, &ctx, &ScreentoneSpec { origin_x: 16.0, ..base.clone() });
        for y in 0..48 {
            for x in 16..96 {
                for c in 0..3 {
                    assert_eq!(shifted_x[(y * 96 + x) * 3 + c], plain[(y * 96 + (x - 16)) * 3 + c], "origin x shift at {x}, {y} ch {c}");
                }
            }
        }

        let shifted_y = screentone(&img, bounds, &ctx, &ScreentoneSpec { origin_y: 16.0, ..base.clone() });
        // Row parity comes from the shifted lattice, so the y shift is a full-row translation
        // too (spacing 16 keeps odd rows identified with plain rows).
        for y in 16..48 {
            for x in 0..96 {
                for c in 0..3 {
                    assert_eq!(shifted_y[(y * 96 + x) * 3 + c], plain[((y - 16) * 96 + x) * 3 + c], "origin y shift at {x}, {y} ch {c}");
                }
            }
        }

        // The satellite ring travels with its dot: satellites on and the same equality holds.
        let sat = ScreentoneSpec { satellite_enabled: true, satellite_size: 3.0, satellite_distance: 7.0, ..base.clone() };
        let plain_sat = screentone(&img, bounds, &ctx, &sat);
        let shifted_sat = screentone(&img, bounds, &ctx, &ScreentoneSpec { origin_x: 16.0, ..sat });
        for y in 0..48 {
            for x in 16..96 {
                for c in 0..3 {
                    assert_eq!(shifted_sat[(y * 96 + x) * 3 + c], plain_sat[(y * 96 + (x - 16)) * 3 + c], "satellite origin shift at {x}, {y} ch {c}");
                }
            }
        }

        // An untouched origin renders exactly as before the field existed.
        let unchanged = screentone(&img, bounds, &ctx, &base);
        assert_eq!(unchanged, plain);
    }

    #[test]
    fn document_origin_round_trips_across_render_frames() {
        let spec = ScreentoneSpec {
            pattern_type: PatternType::Dots,
            dot_size: 5.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            rot_canvas: 27.0,
            origin_x: 5.0,
            origin_y: -3.0,
            use_source_luminance: false,
            ..Default::default()
        };
        let sheet = Rect::new(8, 6, 72, 54);
        let origin = origin_doc(&spec, sheet);
        // Rendering the same document anchor over a different frame recovers that same anchor:
        // the frame's centre is removed before the tone's rotation is undone.
        let wider = Rect::new(0, 0, 160, 120);
        let rerendered = sheet_spec(&spec, wider, origin);
        let back = origin_doc(&rerendered, wider);
        assert!((back.0 - origin.0).abs() < 0.001, "{back:?} vs {origin:?}");
        assert!((back.1 - origin.1).abs() < 0.001, "{back:?} vs {origin:?}");
        // The phase itself is frame-relative, so the two specs differ while anchoring the same point.
        assert_ne!((rerendered.origin_x, rerendered.origin_y), (spec.origin_x, spec.origin_y));
    }

    #[test]
    fn test_manga_shading_thresholds_beta_fill_and_satellites() {
        let bounds = Rect::new(0, 0, 96, 32);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };
        let mut data = vec![0.0f32; (96 * 32 * 4) as usize];
        for y in 0..32usize {
            for x in 0..96usize {
                let idx = (y * 96 + x) * 4;
                if x < 32 {
                    // Deep shadow below black_threshold (0.04 <= 0.08): beta fill
                    data[idx..idx + 4].copy_from_slice(&[0.04, 0.04, 0.04, 1.0]);
                } else if x < 64 {
                    // Midtone (0.45 in 0.08..0.85): halftone dots + satellites
                    data[idx..idx + 4].copy_from_slice(&[0.45, 0.45, 0.45, 1.0]);
                } else {
                    // Highlight paper above white_threshold (0.92 >= 0.85): clean paper
                    data[idx..idx + 4].copy_from_slice(&[0.92, 0.92, 0.92, 1.0]);
                }
            }
        }
        let img = Image { rect: bounds, ch: 4, data };
        let spec = ScreentoneSpec {
            mode: ScreentoneMode::Shading,
            use_source_luminance: true,
            white_threshold: 0.85,
            black_threshold: 0.08,
            tone_gamma: 1.0,
            pattern_type: PatternType::Dots,
            dot_size: 5.0,
            spacing_x: 10.0,
            spacing_y: 10.0,
            satellite_enabled: true,
            satellite_size: 3.0,
            satellite_distance: 4.0,
            satellite_count: 4,
            jitter_pos: 5.0,
            jitter_size: 5.0,
            ..Default::default()
        };
        let out_px = screentone(&img, bounds, &ctx, &spec);
        let out_slice = out_px.as_slice();
        for y in 0..32usize {
            for x in 72..96usize {
                let a = out_px[(y * 96 + x) * 4 + 3];
                assert_eq!(a, 0.0, "highlight paper at ({x}, {y}) must be 0.0, got {a}");
            }
        }
        let deep_ink: f32 = (0..32usize).flat_map(|y| (0..24usize).map(move |x| out_slice[(y * 96 + x) * 4 + 3])).sum();
        let mid_ink: f32 = (0..32usize).flat_map(|y| (38..58usize).map(move |x| out_slice[(y * 96 + x) * 4 + 3])).sum();
        assert!(deep_ink > mid_ink, "deep shadow region (ink=1.0) renders higher ink coverage ({deep_ink}) than midtone ({mid_ink}) without binary flat clipping");
        assert!(mid_ink > 20.0, "midtone region renders partial halftone + satellites (got {mid_ink})");

        // In Sheet mode, the same image renders uniform pattern even over the 0.92 highlight zone.
        let sheet_spec = ScreentoneSpec { mode: ScreentoneMode::Sheet, use_source_luminance: false, ..spec };
        let sheet_px = screentone(&img, bounds, &ctx, &sheet_spec);
        let sheet_slice = sheet_px.as_slice();
        let sheet_right_ink: f32 = (0..32usize).flat_map(|y| (72..96usize).map(move |x| sheet_slice[(y * 96 + x) * 4 + 3])).sum();
        assert!(sheet_right_ink > 20.0, "Sheet mode renders uniform dots + satellites across white paper (got {sheet_right_ink})");
    }

    #[test]
    fn test_brightness_contrast_and_preserve_lines_with_thickness() {
        let bounds = Rect::new(0, 0, 64, 32);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };
        let mut data = vec![1.0f32; (64 * 32 * 4) as usize];
        // Draw a 1px vertical pure black line at x = 32 (luma = 0.0 <= black_threshold = 0.08).
        // With preserve_lines = false, there is no forced cov=1.0 per-pixel clip: it is rendered
        // purely as smooth halftone dots sized by sample_ink.
        // With preserve_lines = true, the line-art detector triggers and preserves it as solid ink.
        for y in 0..32usize {
            let idx = (y * 64 + 32) * 4;
            data[idx..idx + 4].copy_from_slice(&[0.0, 0.0, 0.0, 1.0]);
        }
        let img = Image { rect: bounds, ch: 4, data };
        let base_spec = ScreentoneSpec {
            mode: ScreentoneMode::Shading,
            use_source_luminance: true,
            dot_size: 2.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            white_threshold: 0.85,
            black_threshold: 0.08,
            preserve_lines: false,
            line_threshold: 0.30,
            line_thickness: 1.0,
            ..Default::default()
        };
        let without_lines = screentone(&img, bounds, &ctx, &base_spec);
        let line_ink_off: f32 = (0..32usize).map(|y| without_lines[(y * 64 + 32) * 4 + 3]).sum();
        assert!(line_ink_off < 28.0, "without preserve_lines, the 0.20 line is broken by the 16px dot grid (got {line_ink_off})");

        let with_lines = screentone(&img, bounds, &ctx, &ScreentoneSpec { preserve_lines: true, ..base_spec.clone() });
        let line_ink_on: f32 = (0..32usize).map(|y| with_lines[(y * 64 + 32) * 4 + 3]).sum();
        assert!((line_ink_on - 32.0).abs() < 1e-4, "with preserve_lines, every pixel of the vertical line at x=32 is 1.0 solid ink");
        let bleed_ink: f32 = (0..32usize).map(|y| with_lines[(y * 64 + 30) * 4 + 3] + with_lines[(y * 64 + 34) * 4 + 3]).sum();
        assert_eq!(bleed_ink, 0.0, "zero dot bleeding or pimples adjacent to preserved line");

        // Increasing `line_thickness` to 2.5 dilates the preserved line into x=31 and x=33 as well.
        let thick_lines = screentone(&img, bounds, &ctx, &ScreentoneSpec { preserve_lines: true, line_thickness: 2.5, ..base_spec.clone() });
        let neighbor_ink: f32 = (0..32usize).map(|y| thick_lines[(y * 64 + 31) * 4 + 3] + thick_lines[(y * 64 + 33) * 4 + 3]).sum();
        assert!((neighbor_ink - 64.0).abs() < 1e-4, "line_thickness=2.5 dilates the preserved line to adjacent columns");
    }

    #[test]
    fn test_grid_rotation_rotates_dot_lattice() {
        let bounds = Rect::new(0, 0, 64, 64);
        let img = make_test_image(64, 64, 0.5);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: false };
        let spec_0 = ScreentoneSpec {
            pattern_type: PatternType::Dots,
            dot_size: 4.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            angle: 0.0,
            use_source_luminance: false,
            ..Default::default()
        };
        let spec_45 = ScreentoneSpec { angle: 45.0, ..spec_0.clone() };
        let px_0 = screentone(&img, bounds, &ctx, &spec_0);
        let px_45 = screentone(&img, bounds, &ctx, &spec_45);
        assert_ne!(px_0, px_45, "changing angle to 45° must rotate the dot grid");
    }

    #[test]
    fn test_thick_line_in_gray_tone_has_no_staircase_halo_and_white_line_has_no_pimples() {
        let bounds = Rect::new(0, 0, 64, 64);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };

        let spec = ScreentoneSpec {
            mode: ScreentoneMode::Shading,
            use_source_luminance: true,
            pattern_type: PatternType::Dots,
            dot_size: 8.0,
            spacing_x: 10.0,
            spacing_y: 10.0,
            density: 0.7,
            white_threshold: 0.85,
            black_threshold: 0.08,
            preserve_lines: true,
            line_threshold: 0.24,
            line_thickness: 1.0,
            ..Default::default()
        };

        // 1. Pure 50% gray tone reference (no line) vs 50% gray tone with a thick 8px black line
        //    (x = 28..36) and 2px anti-aliased fringes (x = 26..28 and x = 36..38).
        let mut gray_ref_data = vec![0.0f32; 64 * 64 * 4];
        for px in gray_ref_data.chunks_exact_mut(4) {
            px.copy_from_slice(&[0.50, 0.50, 0.50, 1.0]);
        }
        let gray_ref_img = Image { rect: bounds, ch: 4, data: gray_ref_data.clone() };
        let gray_ref_out = screentone(&gray_ref_img, bounds, &ctx, &spec);

        let mut gray_line_data = gray_ref_data;
        for y in 0..64usize {
            for x in 26..38usize {
                let v = if (28..36).contains(&x) {
                    0.02
                } else if x == 27 || x == 36 {
                    0.28
                } else {
                    0.40
                };
                let idx = (y * 64 + x) * 4;
                gray_line_data[idx..idx + 4].copy_from_slice(&[v, v, v, 1.0]);
            }
        }
        let gray_line_img = Image { rect: bounds, ch: 4, data: gray_line_data };
        let gray_line_out = screentone(&gray_line_img, bounds, &ctx, &spec);

        // Verify the thick black line itself is 100% solid ink, and the pixels immediately
        // outside the line (x = 22..28 and x = 36..42) match the un-lined 50% gray reference
        // with zero white staircase halo!
        for y in 4..60usize {
            for x in 28..36usize {
                let a = gray_line_out[(y * 64 + x) * 4 + 3];
                assert!((a - 1.0).abs() < 1e-4, "thick line interior at ({x}, {y}) must be 1.0, got {a}");
            }
            for x in (22..28usize).chain(36..42usize) {
                let a_line = gray_line_out[(y * 64 + x) * 4 + 3];
                let a_ref = gray_ref_out[(y * 64 + x) * 4 + 3];
                assert!(
                    (a_line - a_ref).abs() < 1e-4,
                    "white staircase halo or fringe distortion at ({x}, {y}): got {a_line}, expected {a_ref}"
                );
            }
        }

        // 2. Thick 8px black line on white paper (1.0) with 2px anti-aliased gray fringes (0.45, 0.72):
        //    must produce ZERO halftone pimples outside the preserved line core.
        let mut white_line_data = vec![1.0f32; 64 * 64 * 4];
        for y in 0..64usize {
            for x in 26..38usize {
                let v = if (28..36).contains(&x) {
                    0.02
                } else if x == 27 || x == 36 {
                    0.45
                } else {
                    0.72
                };
                let idx = (y * 64 + x) * 4;
                white_line_data[idx..idx + 4].copy_from_slice(&[v, v, v, 1.0]);
            }
        }
        let white_line_img = Image { rect: bounds, ch: 4, data: white_line_data };
        let white_line_out = screentone(&white_line_img, bounds, &ctx, &spec);
        for y in 4..60usize {
            for x in (20..28usize).chain(36..44usize) {
                let a = white_line_out[(y * 64 + x) * 4 + 3];
                assert_eq!(a, 0.0, "anti-aliased fringe on white paper at ({x}, {y}) spawned a pimple (alpha={a})");
            }
        }
    }

    #[test]
    fn test_fine_hatching_on_white_paper_has_no_pimples() {
        let bounds = Rect::new(0, 0, 64, 64);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };
        let mut data = vec![1.0f32; 64 * 64 * 4];

        // Draw fine parallel hatching (strokes at x = 20, 24, 28, 32, 36) where the 3px gaps
        // between strokes are darkened by overlapping anti-aliasing to 0.62..0.74 (< white_threshold 0.85).
        for y in 12..52usize {
            for x in 19..=37usize {
                let dist_to_stroke = [20isize, 24, 28, 32, 36]
                    .iter()
                    .map(|&sx| (x as isize - sx).abs())
                    .min()
                    .unwrap_or(10);
                let v = match dist_to_stroke {
                    0 => 0.12, // hatch stroke core
                    1 => 0.62, // 1px anti-aliased fringe
                    _ => 0.74, // midpoint of narrow gap between strokes
                };
                let idx = (y * 64 + x) * 4;
                data[idx..idx + 4].copy_from_slice(&[v, v, v, 1.0]);
            }
        }

        let img = Image { rect: bounds, ch: 4, data };
        let spec = ScreentoneSpec {
            mode: ScreentoneMode::Shading,
            use_source_luminance: true,
            pattern_type: PatternType::Dots,
            dot_size: 7.0,
            spacing_x: 10.0,
            spacing_y: 10.0,
            density: 0.6,
            white_threshold: 0.85,
            black_threshold: 0.08,
            preserve_lines: true,
            line_threshold: 0.24,
            line_thickness: 1.0,
            ..Default::default()
        };
        let out_px = screentone(&img, bounds, &ctx, &spec);

        for y in 16..48usize {
            // Every hatch stroke core is preserved as 1.0 solid ink.
            for &sx in &[20usize, 24, 28, 32, 36] {
                let a = out_px[(y * 64 + sx) * 4 + 3];
                assert!((a - 1.0).abs() < 1e-4, "hatch stroke at ({sx}, {y}) must be preserved as 1.0, got {a}");
            }
            // Every narrow slit between the hatch strokes is clean paper (0.0), with zero pimples.
            for sx in [21usize, 22, 23, 25, 26, 27, 29, 30, 31, 33, 34, 35] {
                let a = out_px[(y * 64 + sx) * 4 + 3];
                assert_eq!(a, 0.0, "pimple detected inside fine hatching gap at ({sx}, {y}): alpha={a}");
            }
        }
    }

    #[test]
    fn test_white_outline_against_coarse_screentone_has_no_staircase() {
        let bounds = Rect::new(0, 0, 96, 96);
        let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };

        // Simulate a manga white outline (halo) separating a black hair line from a dark gray background
        // rendered with coarse LPI dots (spacing = 24.0, dot_size = 14.0, ~14 LPI):
        // x in 0..16: black hair line (0.04)
        // x in 16..24: white hair outline (1.0)
        // x in 24..96: dark gray background tone (0.30)
        let mut data = vec![0.30f32; 96 * 96 * 4];
        for i in 0..(96 * 96) {
            data[i * 4 + 3] = 1.0;
        }
        for y in 0..96usize {
            for x in 0..16usize {
                let idx = (y * 96 + x) * 4;
                data[idx..idx + 4].copy_from_slice(&[0.04, 0.04, 0.04, 1.0]);
            }
            for x in 16..24usize {
                let idx = (y * 96 + x) * 4;
                data[idx..idx + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
            }
        }
        let img = Image { rect: bounds, ch: 4, data };

        let mut ref_data = vec![0.30f32; 96 * 96 * 4];
        for i in 0..(96 * 96) {
            ref_data[i * 4 + 3] = 1.0;
        }
        let ref_img = Image { rect: bounds, ch: 4, data: ref_data };

        let spec = ScreentoneSpec {
            mode: ScreentoneMode::Shading,
            use_source_luminance: true,
            pattern_type: PatternType::Dots,
            dot_size: 14.0,
            spacing_x: 24.0,
            spacing_y: 24.0,
            angle: 45.0,
            density: 0.65,
            white_threshold: 0.85,
            black_threshold: 0.08,
            preserve_lines: true,
            line_threshold: 0.24,
            line_thickness: 1.0,
            ..Default::default()
        };

        let out_px = screentone(&img, bounds, &ctx, &spec);
        let ref_px = screentone(&ref_img, bounds, &ctx, &spec);

        for y in 4..92usize {
            // Inside the white outline (x in 16..24), output must be clean white paper (0.0).
            for x in 16..24usize {
                let a = out_px[(y * 96 + x) * 4 + 3];
                assert_eq!(a, 0.0, "white outline at ({x}, {y}) should be 0.0, got {a}");
            }
            // Immediately outside the white outline (x in 24..92), coarse dots must match the pure dark tone
            // reference with ZERO missing dots whose centers happened to fall in x=16..24!
            for x in 24..92usize {
                let a = out_px[(y * 96 + x) * 4 + 3];
                let a_ref = ref_px[(y * 96 + x) * 4 + 3];
                assert!(
                    (a - a_ref).abs() < 1e-4,
                    "coarse dot staircase / missing dot near white outline at ({x}, {y}): got {a}, expected {a_ref}"
                );
            }
        }
    }

    #[test]
    fn step7_sdist2_matches_wgsl_length_semantics() {
        let mut tested_pairs = 0usize;
        let mut hypot_ulp_divergences = 0usize;

        for iy in -128..=128 {
            for ix in -128..=128 {
                let a = (ix as f32) * 0.371_093_75;
                let b = (iy as f32) * 0.613_281_25;

                let wgsl_length = (a * a + b * b).sqrt();
                let cpu_sdist2 = sdist2(a, b);
                let std_hypot = a.hypot(b);

                // 1. Exact bit-level identity between sdist2(a, b) and WGSL length(vec2f(a, b))
                assert_eq!(
                    cpu_sdist2.to_bits(),
                    wgsl_length.to_bits(),
                    "sdist2({a}, {b}) must match WGSL sqrt(a*a + b*b) to 0 ULP"
                );

                // 2. Track 1-2 ULP divergences against f32::hypot while proving tight numerical tolerance
                let ulp_diff = cpu_sdist2.to_bits().abs_diff(std_hypot.to_bits());
                if ulp_diff > 0 {
                    hypot_ulp_divergences += 1;
                }
                assert!(
                    ulp_diff <= 2,
                    "sdist2({a}, {b}) vs hypot({a}, {b}) exceeded 2 ULP: {ulp_diff} ULP"
                );

                let abs_err = (cpu_sdist2 - std_hypot).abs();
                let rel_tol = 2.0 * f32::EPSILON * std_hypot.max(1.0);
                assert!(
                    abs_err <= rel_tol,
                    "Relative error {abs_err} exceeded tolerance {rel_tol} at ({a}, {b})"
                );

                tested_pairs += 1;
            }
        }

        assert!(tested_pairs > 65_000);
        assert!(
            hypot_ulp_divergences > 0,
            "Expected f32::hypot to exhibit 1-2 ULP scaling divergences across test grid"
        );
    }
}
