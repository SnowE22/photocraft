//! Frozen scalar rasterizer used only as a pixel oracle for the computed-tip fast path.
use super::*;
use crate::tile::GrayTile;
use crate::{dab_coverage, tip_falloff};
use photocraft_geom::Point;

impl BrushContext {
    fn rasterize_reference(&self, d: &Dab, dual: bool, rect: Rect, out: &mut Vec<f32>) {
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
        for yy in 0..h {
            let y = rect.y0 + yy as i32;
            let dy = y as f32 + 0.5 - cy;
            for xx in 0..w {
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
                                sum += tip_falloff(sd, rm, hardness);
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
                        if aliased { if dd <= rm { 1.0 } else { 0.0 } } else { dab_coverage(dd, rm, hardness) }
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
}

fn compare(ctx: &BrushContext, dab: &Dab, dual: bool, rect: Rect) {
    compare_within(ctx, dab, dual, rect, 0.0);
}

/// Does `rasterize` take its plain sampled-tip loop for this dab? That loop multiplies where
/// the scalar one divides, so it agrees to float rounding, not bit for bit.
fn sampled_loop(ctx: &BrushContext, dual: bool) -> bool {
    let b = &ctx.brush;
    let tip = if dual { ctx.dual_tip.is_some() } else { ctx.tip.is_some() };
    let tex_tip = !dual && b.texture.enabled && b.texture.each_tip && ctx.texture.is_some();
    let feature = !dual && (b.aliased || b.noise || tex_tip);
    tip && !feature
}

/// [`compare`] allowing `tol` of difference per value (0: the same bits).
fn compare_within(ctx: &BrushContext, dab: &Dab, dual: bool, rect: Rect, tol: f32) {
    // Reused, dirty buffers also check clearing and resizing.
    let (mut actual, mut expected) = (vec![0.75; 19], vec![0.75; 19]);
    ctx.rasterize(dab, dual, rect, &mut actual);
    ctx.rasterize_reference(dab, dual, rect, &mut expected);
    assert_eq!(actual.len(), expected.len());
    let mismatch = actual.iter().zip(&expected).enumerate().find(|(_, (a, b))| if tol > 0.0 { (*a - *b).abs() > tol || a.is_nan() != b.is_nan() } else { a.to_bits() != b.to_bits() });
    assert!(mismatch.is_none(), "coverage mismatch {mismatch:?}; {dab:?}; dual={dual}, rect={rect:?}");
}

#[test]
fn computed_tips_match_scalar_bits() {
    for aliased in [false, true] {
        for hardness in [-0.5, 0.0, 0.1, 0.5, 0.9999, 1.0, 1.5] {
            let ctx = BrushContext::new(&BrushSettings { aliased, hardness, ..Default::default() });
            for radius in [0.5, 1.0, 2.99, 3.0, 3.01, 15.0, 31.5] {
                for roundness in [0.0, 0.1, 0.4, 1.0] {
                    for alpha in [-0.5, -0.0, 0.0, 0.37, 1.0] {
                        let mut d = Dab::round(Point::new(-31.25, 63.75), radius, alpha);
                        d.roundness = roundness;
                        d.flip_x = true;
                        d.flip_y = true;
                        compare(&ctx, &d, false, ctx.dab_rect(&d, false));
                    }
                }
            }
        }
    }
}

#[test]
fn large_corner_dabs_keep_off_canvas_coverage() {
    for (hardness, aliased) in [(0.0, false), (1.0, false), (1.0, true)] {
        let ctx = BrushContext::new(&BrushSettings { hardness, aliased, ..Default::default() });
        let d = Dab::round(Point::new(-1050.25, -1050.75), 1500.0, 0.73);
        let rect = ctx.dab_rect(&d, false);
        compare(&ctx, &d, false, rect);
        let mut coverage = Vec::new();
        ctx.rasterize(&d, false, rect, &mut coverage);
        // The layer is an infinite plane: paint outside the visible document must survive.
        assert!(coverage.iter().enumerate().any(|(i, v)| *v > 0.0 && rect.x0 + ((i % rect.width() as usize) as i32) < 0));
        compare(&ctx, &d, false, Rect::new(0, 0, 32, 32));
        compare(&ctx, &d, false, Rect::new(0, 0, 0, 0));
    }
}

#[test]
fn feature_and_exceptional_dabs_match_scalar_bits() {
    let tip = GrayTile::from_fn(8, 5, |x, y| ((x + y) % 7) as f32 / 6.0);
    for feature in 0..7 {
        let mut b = BrushSettings { hardness: 0.25, ..Default::default() };
        match feature {
            0 => b.noise = true,
            1 => b.wet_edges = true,
            2 => {
                b.aliased = true;
                b.square = true;
            }
            3 => b.tip = TipShape::Sampled(tip.clone()),
            4 => {
                b.texture.enabled = true;
                b.texture.each_tip = true;
                b.texture.pattern = Pattern::Tile(tip.clone());
            }
            5 => b.dual_brush.tip = TipShape::Sampled(tip.clone()),
            _ => {}
        }
        let ctx = BrushContext::new(&b);
        for dual in [false, true] {
            for (angle, proj_scale) in [(0.0, 1.0), (0.7, 1.0), (0.0, 0.6)] {
                let mut d = Dab::round(Point::new(63.5, -0.25), 15.0, 0.47);
                d.angle = angle;
                d.roundness = 0.4;
                d.proj_scale = proj_scale;
                let tol = if sampled_loop(&ctx, dual) { 2e-4 } else { 0.0 };
                compare_within(&ctx, &d, dual, ctx.dab_rect(&d, dual), tol);
            }
        }
    }
    let ctx = BrushContext::new(&BrushSettings::default());
    for value in [f32::NAN, f32::from_bits(0x7f80_0123), f32::INFINITY, f32::NEG_INFINITY] {
        for field in 0..4 {
            let mut d = Dab::round(Point::new(0.0, 0.0), 15.0, 1.0);
            match field {
                0 => d.alpha = value,
                1 => d.angle = value,
                2 => d.radius = value,
                _ => d.roundness = value,
            }
            compare(&ctx, &d, false, Rect::new(-4, -4, 4, 4));
        }
        let ctx = BrushContext::new(&BrushSettings { hardness: value, ..Default::default() });
        compare(&ctx, &Dab::round(Point::new(0.0, 0.0), 15.0, 1.0), false, Rect::new(-16, -16, 16, 16));
    }
}

#[test]
fn computed_tips_match_at_pixel_and_float_boundaries() {
    for aliased in [false, true] {
        for center in [Point::new(0.0, -0.0), Point::new(0.5, 0.5), Point::new(255.9999, -256.0001), Point::new(16_777_216.25, -16_777_216.75)] {
            for radius in [3.0, 3.5, 15.25, 113.75] {
                for hardness in [0.0, 0.6, 1.0] {
                    let ctx = BrushContext::new(&BrushSettings { hardness, aliased, ..Default::default() });
                    let mut d = Dab::round(center, radius, 0.137);
                    d.roundness = 0.375;
                    compare(&ctx, &d, false, ctx.dab_rect(&d, false));
                }
            }
        }
    }
}

/// A soft blob with an empty border, `w × h`, with some texture inside.
fn blob(w: u32, h: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| {
        let (u, v) = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, (y as f32 + 0.5) / h as f32 * 2.0 - 1.0);
        let r = (u * u + v * v).sqrt();
        ((1.0 - r) * 1.6).clamp(0.0, 1.0) * (0.55 + 0.45 * ((x * 7 + y * 13) % 11) as f32 / 10.0)
    })
}

/// Every pixel a dab paints over its full rectangle lies inside its tight one.
fn assert_inside_paint_rect(ctx: &BrushContext, d: &Dab, dual: bool) {
    let (full, tight) = (ctx.dab_rect(d, dual), ctx.paint_rect(d, dual));
    let mut out = Vec::new();
    ctx.rasterize(d, dual, full, &mut out);
    let w = full.width().max(1) as usize;
    for (i, v) in out.iter().enumerate() {
        let (x, y) = (full.x0 + (i % w) as i32, full.y0 + (i / w) as i32);
        assert!(*v == 0.0 || tight.contains(x, y), "({x}, {y}) = {v} is outside {tight:?}; {d:?}");
    }
}

#[test]
fn large_rotated_tips_match_scalar_and_stay_in_their_rects() {
    let tips = [blob(338, 294), blob(40, 12), blob(7, 300), blob(1, 1), GrayTile::from_fn(8, 5, |x, y| ((x + y) % 7) as f32 / 6.0)];
    for (t, tip) in tips.iter().enumerate() {
        for sampled in [true, false] {
            let b = if sampled {
                BrushSettings { tip: TipShape::Sampled(tip.clone()), ..Default::default() }
            } else {
                BrushSettings { hardness: 0.3 * t as f32, ..Default::default() }
            };
            let ctx = BrushContext::new(&b);
            // The edge-heavy tile only upscaled: a downscaled tip's edge texels can make a pixel
            // on the bitmap's border differ by more than rounding.
            let radii: &[f32] = if t == 4 { &[3.5, 40.0] } else { &[2.0, 40.0, 250.0, 700.0] };
            for &radius in radii {
                for angle in [0.0, 0.3, 1.2, 3.0, -2.0] {
                    for roundness in [1.0, 0.6, 0.25] {
                        for (proj_scale, flip) in [(1.0, false), (0.5, true)] {
                            let mut d = Dab::round(Point::new(-123.37, 4567.81), radius, 0.8);
                            (d.angle, d.roundness, d.proj_scale, d.proj_angle, d.flip_x, d.flip_y) = (angle, roundness, proj_scale, 0.9, flip, !flip);
                            let tol = if sampled_loop(&ctx, false) { 2e-4 } else { 0.0 };
                            compare_within(&ctx, &d, false, ctx.dab_rect(&d, false), tol);
                            assert_inside_paint_rect(&ctx, &d, false);
                        }
                    }
                }
            }
        }
    }
}

/// The stroke renderer's coverage, built tile by tile (in parallel for large dabs), is exactly
/// what rasterising and accumulating its dabs one at a time gives.
#[test]
fn tile_parallel_strokes_match_dab_by_dab_accumulation() {
    use crate::{StrokePoint, dynamics::DabGenerator};
    let tip = TipShape::Sampled(blob(120, 90));
    let mut brushes = Vec::new();
    let mut b = BrushSettings { size: 300.0, spacing: 0.05, tip: tip.clone(), seed: 3, ..Default::default() };
    b.shape_dynamics.enabled = true;
    b.shape_dynamics.angle.control = crate::Control::Direction;
    b.shape_dynamics.size.jitter = 0.5;
    b.transfer.enabled = true;
    b.transfer.opacity.jitter = 0.6;
    brushes.push(b.clone());
    let mut dual = b.clone();
    dual.dual_brush = crate::DualBrush { enabled: true, tip: TipShape::Sampled(blob(40, 30)), size: 120.0, spacing: 0.1, scatter: 1.0, count: 3, ..Default::default() };
    brushes.push(dual);
    let mut wet = BrushSettings { size: 260.0, hardness: 0.2, spacing: 0.08, wet_edges: true, ..Default::default() };
    wet.shape_dynamics.enabled = true;
    wet.shape_dynamics.roundness.jitter = 0.7;
    wet.shape_dynamics.angle.jitter = 1.0;
    brushes.push(wet);
    let mut colour = BrushSettings { size: 220.0, spacing: 0.1, tip, ..Default::default() };
    colour.color_dynamics.enabled = true;
    colour.color_dynamics.hue_jitter = 0.5;
    brushes.push(colour);
    let points: Vec<StrokePoint> =
        (0..40).map(|i| StrokePoint::new(100.0 + 23.0 * i as f64, 300.0 + 120.0 * (i as f64 / 6.0).sin(), 0.4 + 0.015 * i as f32)).collect();
    for brush in brushes {
        let fmt = photocraft_color::PixelFormat::RGBA8;
        let mut r = StrokeRenderer::new(&brush, Some(fmt), 1.0);
        for chunk in points.chunks(3) {
            r.push(chunk);
        }
        r.finish();
        // The same dabs, one at a time.
        let mut g = DabGenerator::new(&r.ctx.brush, 1.0);
        let (mut dabs, mut duals) = (Vec::new(), Vec::new());
        g.push(&points, &mut dabs, &mut duals);
        g.finish(&mut dabs, &mut duals);
        let ctx = &r.ctx;
        let mut cov = CoverageMap::new(r.cov.nc);
        let mut vals = Vec::new();
        for d in &dabs {
            let rect = ctx.dab_rect(d, false);
            ctx.rasterize(d, false, rect, &mut vals);
            let mut native = [0.0f32; 8];
            let col = if r.per_dab_color {
                photocraft_raster::from_rgba_into(&fmt, d.color, &mut native);
                Some(&native[..fmt.mode.color_channels()])
            } else {
                None
            };
            cov.accumulate(rect, &vals, d.opacity, col);
        }
        let mut dual_cov = CoverageMap::new(0);
        for d in &duals {
            let rect = ctx.dab_rect(d, true);
            ctx.rasterize(d, true, rect, &mut vals);
            dual_cov.accumulate(rect, &vals, 1.0, None);
        }
        let same = |a: &CoverageMap, b: &CoverageMap, what: &str| {
            let mut ka: Vec<_> = a.tiles.keys().collect();
            let mut kb: Vec<_> = b.tiles.keys().collect();
            ka.sort();
            kb.sort();
            assert_eq!(ka, kb, "{what}: tiles");
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            for (k, ta) in &a.tiles {
                let tb = &b.tiles[k];
                assert_eq!(bits(&ta.cov), bits(&tb.cov), "{what}: coverage of tile {k:?}");
                assert_eq!(bits(&ta.col), bits(&tb.col), "{what}: colour of tile {k:?}");
            }
            // The tight bounds hold all the paint and are never wider than the full ones.
            assert_eq!(a.bounds.union(&b.bounds), b.bounds, "{what}: bounds {:?} vs {:?}", a.bounds, b.bounds);
            for (k, t) in &a.tiles {
                for (i, c) in t.cov.iter().enumerate() {
                    let (x, y) = (k.0 * COV_TILE + (i as i32 % COV_TILE), k.1 * COV_TILE + (i as i32 / COV_TILE));
                    assert!(*c == 0.0 || a.bounds.contains(x, y), "{what}: paint at ({x}, {y}) outside {:?}", a.bounds);
                }
            }
        };
        same(&r.cov, &cov, "stroke");
        if let Some(d) = &r.dual {
            same(d, &dual_cov, "dual");
        }
    }
}

#[test]
fn parallel_composite_matches_single_threaded() {
    use crate::StrokePoint;
    let mut b = BrushSettings {
        size: 400.0,
        spacing: 0.04,
        tip: TipShape::Sampled(blob(200, 150)),
        mode: photocraft_color::BlendMode::Multiply,
        opacity: 0.8,
        seed: 9,
        ..Default::default()
    };
    b.dual_brush = crate::DualBrush { enabled: true, size: 90.0, spacing: 0.2, scatter: 0.5, count: 2, ..Default::default() };
    b.texture.enabled = true;
    b.texture.mode = crate::MaskMode::Height;
    b.texture.depth = 0.3;
    b.color_dynamics.enabled = true;
    b.color_dynamics.brightness_jitter = 0.4;
    for fmt in [photocraft_color::PixelFormat::RGBA8, photocraft_color::PixelFormat::RGBA16, photocraft_color::PixelFormat::CMYKA8] {
        let mut base = Surface::new(fmt);
        base.fill_rect(Rect::new(0, 0, 900, 700), &photocraft_raster::from_rgba(&fmt, [0.9, 0.6, 0.3, 1.0]));
        base.fill_rect(Rect::new(200, 0, 300, 700), &photocraft_raster::from_rgba(&fmt, [0.2, 0.4, 0.8, 0.0]));
        let mut sel = Surface::new(photocraft_color::PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 640, 700), &[0.7]);
        let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0);
        let points: Vec<StrokePoint> = (0..20).map(|i| StrokePoint::new(80.0 + 40.0 * i as f64, 350.0 + 90.0 * (i as f64 / 3.0).cos(), 1.0)).collect();
        for chunk in points.chunks(4) {
            r.push(chunk);
            for lock in [false, true] {
                let (mut one, mut many) = (r.clone(), r.clone());
                let (mut t1, mut t2) = (base.clone(), base.clone());
                let d1 = one.composite_tiles(&base, &mut t1, Some(&sel), lock, false, false);
                let d2 = many.composite_tiles(&base, &mut t2, Some(&sel), lock, false, true);
                assert_eq!(d1, d2, "{fmt:?}: damage");
                let area = Rect::new(-200, -200, 1200, 1000);
                assert_eq!(t1.read_region(area), t2.read_region(area), "{fmt:?}: pixels (lock {lock})");
            }
            r.composite(&base, &mut base.clone(), None, false, false);
        }
    }
}
