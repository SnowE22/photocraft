//! Live-stroke throughput of large brushes: a fast stroke fed frame by frame, as the canvas feeds
//! it, timing each frame's `push` (dabs) and `composite` (the dirty tiles onto the layer).
//!
//! ```sh
//! cargo run --release -p photocraft-paint --example stroke_bench -- [brush.json] [--frames N]
//! ```
//!
//! Without a brush file it runs a few synthetic large brushes. A frame over ~16 ms means the
//! stroke falls behind the pointer.

use std::time::Instant;

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_paint::{BrushSettings, GrayTile, StrokePoint, StrokeRenderer, TipShape};
use photocraft_raster::Surface;

fn blob_tip(w: u32, h: u32) -> GrayTile {
    GrayTile::from_fn(w, h, |x, y| {
        let (u, v) = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, (y as f32 + 0.5) / h as f32 * 2.0 - 1.0);
        let r = (u * u + v * v).sqrt();
        let n = ((x * 7919 + y * 104_729) % 97) as f32 / 97.0;
        ((1.0 - r) * 2.0).clamp(0.0, 1.0) * (0.6 + 0.4 * n)
    })
}

fn run(name: &str, brush: &BrushSettings, frames: usize) {
    let (w, h) = (4000, 3000);
    let mut target = Surface::new(PixelFormat::RGBA8);
    target.fill_rect(Rect::new(0, 0, w, h), &[1.0, 1.0, 1.0, 0.0]);
    let pre = target.clone();
    let mut r = StrokeRenderer::new(brush, Some(target.format()), 1.0);
    // A fast stroke: 1500 px/s sampled at 120 Hz, two samples per 60 Hz frame, pressure varying.
    let speed = 1500.0 / 120.0;
    let mut t = 0.0f64;
    let mut pt = |i: usize| {
        t += 1000.0 / 120.0;
        let s = i as f64 * speed;
        let x = 500.0 + s % 3000.0;
        let y = 1500.0 + 800.0 * (s / 400.0).sin();
        let mut p = StrokePoint::new(x, y, (0.5 + 0.5 * (s / 300.0).sin().abs()) as f32);
        p.time = t;
        p
    };
    let mut push_ms = Vec::new();
    let mut comp_ms = Vec::new();
    let mut dabs = 0;
    for f in 0..frames {
        let pts = [pt(2 * f), pt(2 * f + 1)];
        let t0 = Instant::now();
        r.push(&pts);
        let t1 = Instant::now();
        r.composite(&pre, &mut target, None, false, false);
        let t2 = Instant::now();
        push_ms.push((t1 - t0).as_secs_f64() * 1000.0);
        comp_ms.push((t2 - t1).as_secs_f64() * 1000.0);
        dabs = r.dab_count();
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let sum: f64 = v.iter().sum();
        (v[v.len() / 2], v[(v.len() * 95 / 100).min(v.len() - 1)], sum)
    };
    let (p50, p95, sp) = stat(&mut push_ms);
    let (c50, c95, sc) = stat(&mut comp_ms);
    let real = frames as f64 * 1000.0 / 60.0;
    println!(
        "{name:<44} push p50 {p50:7.2} p95 {p95:7.2} | composite p50 {c50:7.2} p95 {c95:7.2} ms/frame | {dabs} dabs | total {:.0} ms for {:.0} ms of stroke ({:.1}x)",
        sp + sc,
        real,
        (sp + sc) / real
    );
}

/// Single-thread cost of rasterising one large dab of each kind, in ns per pixel of its rectangle.
fn raster_cost() {
    use photocraft_geom::Point;
    use photocraft_paint::{BrushContext, Dab};
    let tip = TipShape::Sampled(blob_tip(338, 294));
    for (name, brush, angle) in [
        ("round soft", BrushSettings { hardness: 0.0, ..Default::default() }, 0.0),
        ("round soft, rotated", BrushSettings { hardness: 0.0, ..Default::default() }, 0.6),
        ("sampled", BrushSettings { tip: tip.clone(), ..Default::default() }, 0.0),
        ("sampled, rotated", BrushSettings { tip, ..Default::default() }, 0.6),
    ] {
        let ctx = BrushContext::new(&brush);
        let mut d = Dab::round(Point::new(2000.3, 1500.7), 750.0, 0.5);
        d.angle = angle;
        let r = ctx.paint_rect(&d, false);
        let mut out = Vec::new();
        let n = 6;
        let t = Instant::now();
        for _ in 0..n {
            ctx.rasterize(&d, false, r, &mut out);
        }
        let px = (r.width() as f64) * (r.height() as f64) * n as f64;
        println!("{name:<24} {:>6.2} ns/px over {}x{}", t.elapsed().as_secs_f64() * 1e9 / px, r.width(), r.height());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--raster") {
        raster_cost();
        return;
    }
    let frames = args.iter().position(|a| a == "--frames").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(120);
    if let Some(path) = args.iter().find(|a| a.ends_with(".json")) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        match serde_json::from_str::<BrushSettings>(&text) {
            Ok(mut b) => {
                b.seed = 7;
                run(&format!("{path} ({} px)", b.size), &b, frames);
                let mut plain = b.clone();
                plain.dual_brush.enabled = false;
                plain.texture.enabled = false;
                run("  without dual brush and texture", &plain, frames);
                let mut nodyn = plain.clone();
                nodyn.shape_dynamics.enabled = false;
                run("  … and without shape dynamics", &nodyn, frames);
            }
            Err(e) => eprintln!("{path}: {e}"),
        }
        return;
    }
    let base = BrushSettings { pressure_size: false, seed: 7, ..Default::default() };
    run("round soft 1000 px, 10 %", &BrushSettings { size: 1000.0, hardness: 0.0, spacing: 0.1, ..base.clone() }, frames);
    run("round hard 300 px, 25 %", &BrushSettings { size: 300.0, hardness: 1.0, spacing: 0.25, ..base.clone() }, frames);
    let tip = TipShape::Sampled(blob_tip(338, 294));
    run("sampled 500 px, 3 %", &BrushSettings { size: 500.0, spacing: 0.03, tip: tip.clone(), ..base.clone() }, frames);
    let mut rotated = BrushSettings { size: 500.0, spacing: 0.03, tip: tip.clone(), ..base.clone() };
    rotated.shape_dynamics.enabled = true;
    rotated.shape_dynamics.angle.control = photocraft_paint::Control::Direction;
    run("sampled 500 px, 3 %, angle follows direction", &rotated, frames);
    run("sampled 1500 px, 1 %", &BrushSettings { size: 1500.0, spacing: 0.01, tip, ..base }, frames / 4);
}
