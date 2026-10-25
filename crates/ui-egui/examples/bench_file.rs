//! Interactive edits on a real file with the app's GPU canvas: opening, the first and a warm full
//! refresh, Move tool drag frames, Enclose and Fill, a brush stroke, undo, hiding a layer and
//! saving. Each edit is timed with the canvas refresh the app does after it; the median of `reps`
//! runs is shown.
//!
//! ```sh
//! cargo run --release -p photocraft-ui-egui --example bench_file -- file.psd [reps]
//! ```

use std::time::Instant;

use eframe::egui_wgpu::RenderState;
use photocraft_doc::{Document, LayerId};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::gpu_canvas::GpuCanvas;
use serde_json::{Value, json};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn reach(layers: &[photocraft_doc::Layer]) -> i32 {
    layers
        .iter()
        .map(|l| {
            let own = if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
            let kids = match &l.content {
                photocraft_doc::LayerContent::Group(g) => reach(&g.children),
                _ => 0,
            };
            own + kids
        })
        .max()
        .unwrap_or(0)
}

struct Canvas {
    g: GpuCanvas,
    rs: RenderState,
}

impl Canvas {
    fn show(&self, doc: &Document, damage: Option<Rect>) -> f64 {
        let t = Instant::now();
        self.g.refresh(doc.id.0, doc, damage, None);
        let _ = self.rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None });
        ms(t)
    }
    /// The refresh after a command: its damage grown by the effect reach.
    fn after(&self, s: &Session) -> f64 {
        let Some(st) = s.active() else { return 0.0 };
        let damage = st.last_damage.map(|r| r.inflate(reach(&st.doc.layers)).intersect(&st.doc.bounds()));
        if damage.is_some_and(|d| d.is_empty()) {
            return 0.0;
        }
        self.show(&st.doc, damage)
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("give a file");
    let reps: usize = std::env::args().nth(2).and_then(|r| r.parse().ok()).unwrap_or(7);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let t = Instant::now();
    let doc = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}")).document;
    println!("open (import)                              {:8.1} ms", ms(t));
    let mut s = Session::new();
    s.open_document(doc, Some(path.clone()));
    let setup = photocraft_ui_egui::gpu_canvas::wgpu_setup();
    let rs = egui_kittest::wgpu::create_render_state(setup, Default::default());
    eprintln!("adapter: {}", rs.adapter.get_info().name);
    let c = Canvas { g: GpuCanvas::new(&rs), rs };
    // A tiny document first: what setting up the compositor costs, apart from this file.
    let mut tiny = Session::new();
    tiny.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    println!("first refresh of a 64 × 64 document        {:8.1} ms", c.show(&tiny.active().unwrap().doc, None));
    let doc = s.active().unwrap().doc.clone();
    println!("{} × {}, {} layers", doc.size.width, doc.size.height, doc.walk().len());
    let t = Instant::now();
    let st = c.g.refresh(doc.id.0, &doc, None, None);
    let _ = c.rs.device.poll(eframe::wgpu::PollType::Wait { submission_index: None, timeout: None });
    println!("first full refresh                         {:8.1} ms  {st:?}", ms(t));
    println!("warm full refresh                          {:8.1} ms", c.show(&doc, None));

    let row = |label: &str, times: Vec<f64>| {
        let max = times.iter().copied().fold(0.0, f64::max);
        println!("{label:<42} {:8.1} ms  (max {max:.1})", median(times));
    };
    let ex = |s: &mut Session, id: &str, p: Value| -> Value { s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}")) };

    // The largest visible pixel layer, and the one in the middle of the stack.
    let canvas = doc.bounds();
    let mut pixel: Vec<(i64, LayerId)> = doc
        .walk()
        .iter()
        .filter(|(path, _, l)| doc.shown_at(path) && matches!(l.content, photocraft_doc::LayerContent::Raster(_)))
        .map(|(_, _, l)| {
            let b = photocraft_compose::change_bounds(l, canvas).unwrap_or(canvas);
            (i64::from(b.width()) * i64::from(b.height()), l.id)
        })
        .collect();
    pixel.sort();
    let biggest = pixel.last().map(|p| p.1).expect("a pixel layer");
    let small = pixel.get(pixel.len() / 3).map(|p| p.1).expect("a pixel layer");

    for (label, id) in [("biggest layer", biggest), ("a smaller layer", small)] {
        let doc = s.active().unwrap().doc.clone();
        let all = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
        let was = photocraft_compose::change_bounds(doc.layer(id).unwrap(), all).unwrap_or(canvas);
        let (mut shift, mut frame) = (Vec::new(), Vec::new());
        let mut prev = 0;
        for i in 0..reps * 3 {
            let d = 3 * i as i32 + 1;
            let t = Instant::now();
            let shown = photocraft_engine::layer_multi_cmds::moved(&doc, &[id], d, d / 2).unwrap();
            shift.push(ms(t));
            let damage = was.translate(prev, prev / 2).union(&was.translate(d, d / 2)).inflate(1).intersect(&canvas);
            frame.push(shift.last().copied().unwrap_or(0.0) + c.show(&shown, Some(damage)));
            prev = d;
        }
        c.show(&doc, None);
        let (mut clone, mut bounds, mut tr) = (Vec::new(), Vec::new(), Vec::new());
        if let Some(s) = doc.layer(id).and_then(|l| l.surface()) {
            for i in 0..reps {
                let t = Instant::now();
                std::hint::black_box(doc.as_ref().clone());
                clone.push(ms(t));
                let t = Instant::now();
                let cb = photocraft_compose::bounds::content_bounds(s);
                bounds.push(ms(t));
                let t = Instant::now();
                std::hint::black_box(s.translated(3 * i as i32 + 1, 2, cb));
                tr.push(ms(t));
            }
            println!("  ({} tiles, {} distinct)", s.tile_count(), s.tiles().map(|(_, t)| std::sync::Arc::as_ptr(t)).collect::<std::collections::HashSet<_>>().len());
        }
        println!("move drag, {label} ({}×{})", was.width(), was.height());
        row("  of which: clone the document", clone);
        row("  of which: content bounds", bounds);
        row("  of which: Surface::translated", tr);
        row("  shift the layer (moved)", shift);
        row("  frame (shift + refresh)", frame);
    }

    let (w, h) = (doc.size.width as f64, doc.size.height as f64);
    ex(&mut s, "layer.select", json!({"layer": small.0}));
    ex(&mut s, "layer.new.layer", json!({}));
    let lasso: Vec<Value> = (0..64)
        .map(|i| {
            let a = i as f64 / 64.0 * std::f64::consts::TAU;
            json!([w * 0.5 + a.cos() * w * 0.3, h * 0.5 + a.sin() * h * 0.3])
        })
        .collect();
    let mut filled = 0;
    let mut timed = |s: &mut Session, label: &str, p: Value| {
        let mut v = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            let r = ex(s, "paint.fill", p.clone());
            v.push(ms(t) + c.after(s));
            if r["filled"] == true {
                filled += 1;
                ex(s, "edit.undo", json!({}));
                c.after(s);
            }
        }
        row(label, v);
    };
    for reference in ["layer", "all"] {
        timed(&mut s, &format!("Enclose and Fill, reference {reference}"), json!({"lasso": lasso, "reference": reference, "closeGap": 4, "color": "#cc3366"}));
    }
    timed(&mut s, "Fill click, all layers, gap 4", json!({"x": w * 0.5, "y": h * 0.5, "reference": "all", "closeGap": 4, "color": "#3366cc"}));
    println!("  ({filled} of them filled something)");

    let stroke: Vec<Value> = (0..200).map(|i| json!([w * 0.2 + i as f64 * 3.0, h * 0.3 + (i as f64 * 0.1).sin() * 80.0, 0.8])).collect();
    let (mut sv, mut uv) = (Vec::new(), Vec::new());
    for _ in 0..reps {
        let t = Instant::now();
        ex(&mut s, "paint.stroke", json!({"points": stroke}));
        sv.push(ms(t) + c.after(&s));
        let t = Instant::now();
        ex(&mut s, "edit.undo", json!({}));
        uv.push(ms(t) + c.after(&s));
    }
    row("brush stroke 600 px + refresh", sv);
    row("undo it + refresh", uv);

    let mut v = Vec::new();
    for i in 0..reps {
        let t = Instant::now();
        ex(&mut s, "layer.setProps", json!({"layer": biggest.0, "visible": !i.is_multiple_of(2)}));
        v.push(ms(t) + c.after(&s));
    }
    ex(&mut s, "layer.setProps", json!({"layer": biggest.0, "visible": true}));
    row("hide / show the biggest layer + refresh", v);

    let mut v = Vec::new();
    for i in 0..reps {
        let t = Instant::now();
        ex(&mut s, "layer.translate", json!({"layer": biggest.0, "dx": if i.is_multiple_of(2) { 9 } else { -9 }, "dy": 4}));
        v.push(ms(t) + c.after(&s));
    }
    row("layer.translate the biggest + refresh", v);

    let dir = std::env::temp_dir().join("photocraft-bench-file");
    let _ = std::fs::create_dir_all(&dir);
    for ext in ["psd", "pcraft"] {
        let out = dir.join(format!("copy.{ext}"));
        let mut v = Vec::new();
        for _ in 0..reps.min(3) {
            let t = Instant::now();
            ex(&mut s, "file.saveACopy", json!({"path": out.to_string_lossy()}));
            v.push(ms(t));
        }
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        row(&format!("save a copy .{ext} ({} MB)", size >> 20), v);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
