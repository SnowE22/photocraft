//! Paint one straight stroke with a brush preset from a preset store on disk and write its
//! coverage as an 8-bit PGM (black paint on transparency, so the value is the alpha). Used to put
//! PhotoCraft's stroke next to Photoshop's for the same preset (Photoshop: stroke a path with the
//! Brush tool, Simulate Pressure off, which paints at full pressure).
//!
//! ```text
//! cargo run --release -p photocraft-engine --example preset_stroke -- <store dir> <preset name> <out.pgm>
//!     [--size PX] [--canvas WxH] [--line X0,X1,Y] [--seed N] [--dual-mode colorBurn] [--tex-mode height]
//!     [--plain] [--spacing FRACTION] [--off dual,texture,shape,scatter,transfer,wet] [--show-dual PX]
//! ```
//!
//! `--size` sets the diameter the way picking a preset that doesn't capture its size does (the
//! Dual Brush tip scales with it); without it the preset paints at its stored size.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use photocraft_color::PixelFormat;
use photocraft_engine::paint::{MaskMode, Pattern, StrokePoint, TipShape};
use photocraft_engine::preset_store::open_dir;
use photocraft_raster::Surface;

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn nums(s: &str, sep: char) -> Vec<f64> {
    s.split(sep).filter_map(|v| v.trim().parse().ok()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [store, name, out] = [0, 1, 2].map(|i| args.get(i).cloned().expect("usage: <store dir> <preset name> <out.pgm> [options]"));
    let opened = open_dir(&store);
    let preset = opened.presets.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no preset named {name:?} in {store}"));
    let mut b = preset.brush.clone();
    let st = &opened.store;
    if let TipShape::Stored(r) = &mut b.tip {
        r.full = Some(st.load_tile(r).unwrap());
    }
    if let TipShape::Stored(r) = &mut b.dual_brush.tip {
        r.full = Some(st.load_tile(r).unwrap());
    }
    if let Pattern::Stored(r) = &mut b.texture.pattern {
        r.full = Some(st.load_tile(r).unwrap());
    }
    if let Some(size) = arg(&args, "--size").and_then(|v| v.parse::<f32>().ok()) {
        let old = b.size;
        b.size = size;
        b.scale_dual_with_size(old);
    }
    let mode = |m: &str| serde_json::from_value::<MaskMode>(serde_json::Value::String(m.to_string())).expect("a mask mode such as colorBurn");
    if let Some(m) = arg(&args, "--dual-mode") {
        b.dual_brush.mode = mode(m);
    }
    if let Some(m) = arg(&args, "--tex-mode") {
        b.texture.mode = mode(m);
    }
    // `--plain`: the bare tip (every section off), to compare single dabs.
    if args.iter().any(|a| a == "--plain") {
        b.shape_dynamics.enabled = false;
        b.scattering.enabled = false;
        b.texture.enabled = false;
        b.dual_brush.enabled = false;
        b.color_dynamics.enabled = false;
        b.transfer.enabled = false;
        b.wet_edges = false;
        b.noise = false;
    }
    // `--off dual,texture,shape,scatter,transfer,wet`: switch those sections off.
    let off = arg(&args, "--off").unwrap_or("");
    b.dual_brush.enabled &= !off.contains("dual");
    b.texture.enabled &= !off.contains("texture");
    b.shape_dynamics.enabled &= !off.contains("shape");
    b.scattering.enabled &= !off.contains("scatter");
    b.transfer.enabled &= !off.contains("transfer");
    b.wet_edges &= !off.contains("wet");
    // `--show-dual D`: a hard round D px primary at 1 % under the Dual Brush in Multiply, so the
    // stroke shows exactly where the dual tip lands.
    if let Some(d) = arg(&args, "--show-dual").and_then(|v| v.parse::<f32>().ok()) {
        let dual_size = b.dual_brush.size;
        (b.tip, b.size, b.hardness, b.spacing, b.roundness, b.angle) = (TipShape::Round, d, 1.0, 0.01, 1.0, 0.0);
        b.dual_brush.size = dual_size;
        b.dual_brush.mode = MaskMode::Multiply;
        b.shape_dynamics.enabled = false;
        b.scattering.enabled = false;
        b.texture.enabled = false;
        b.transfer.enabled = false;
        b.wet_edges = false;
        b.flow = 1.0;
        b.opacity = 1.0;
    }
    if let Some(sp) = arg(&args, "--spacing").and_then(|v| v.parse::<f32>().ok()) {
        b.spacing = sp;
    }
    if let TipShape::Stored(r) = &b.tip {
        eprintln!("tip {}x{}", r.width, r.height);
    }
    b.seed = arg(&args, "--seed").and_then(|v| v.parse().ok()).unwrap_or(1);
    b.color = [0.0, 0.0, 0.0, 1.0];
    let canvas = nums(arg(&args, "--canvas").unwrap_or("1000x400"), 'x');
    let (w, h) = (canvas[0] as i32, canvas[1] as i32);
    let line = nums(arg(&args, "--line").unwrap_or("200,800,200"), ',');
    let (x0, x1, y) = (line[0], line[1], line[2]);
    let mut pts = Vec::new();
    let n = ((x1 - x0).abs() / 2.0).ceil().max(1.0) as usize;
    for i in 0..=n {
        let mut p = StrokePoint::new(x0 + (x1 - x0) * i as f64 / n as f64, y, 1.0);
        p.time = i as f64 * 2.0;
        pts.push(p);
    }
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.fill_rect(photocraft_geom::Rect::new(0, 0, w, h), &[1.0, 1.0, 1.0, 0.0]);
    photocraft_engine::paint::render::render_stroke(&mut s, &b, &pts, None, false, 1.0);
    let mut pgm = format!("P5\n{w} {h}\n255\n").into_bytes();
    for yy in 0..h {
        for xx in 0..w {
            pgm.push((s.rgba(xx, yy)[3].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    std::fs::write(&out, pgm).unwrap();
    eprintln!(
        "{name}: {} px, dual {} ({:?}, {:.1} px, spacing {:.2}), texture {} ({:?})",
        b.size, b.dual_brush.enabled, b.dual_brush.mode, b.dual_brush.size, b.dual_brush.spacing, b.texture.enabled, b.texture.mode
    );
}
