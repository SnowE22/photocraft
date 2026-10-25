//! Everyday edits on a real document, through the command path the app uses (`Session::execute`,
//! without the canvas refresh): selecting layers, layer properties, a brush stroke, undo/redo,
//! duplicate, fill, selections, a blur. Each is run several times; the median is shown.
//!
//! ```sh
//! cargo run --release -p photocraft-engine --example bench_everyday -- file.psd [reps]
//! ```

use std::time::Instant;

use photocraft_engine::Session;
use serde_json::{Value, json};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn main() {
    let path = std::env::args().nth(1).expect("give a file");
    let reps: usize = std::env::args().nth(2).and_then(|r| r.parse().ok()).unwrap_or(5);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let t = Instant::now();
    let doc = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}")).document;
    println!("open: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let mut s = Session::new();
    s.add_document(doc, None);
    let doc = s.active().unwrap().doc.clone();
    let pixel_layers: Vec<u64> = doc.walk().iter().filter(|(_, _, l)| l.surface().is_some()).map(|(_, _, l)| l.id.0).collect();
    println!("{} × {}, {} layers ({} pixel)", doc.size.width, doc.size.height, doc.walk().len(), pixel_layers.len());
    let mid = pixel_layers[pixel_layers.len() / 2];
    let (w, h) = (doc.size.width as f64, doc.size.height as f64);
    let stroke: Vec<Value> = (0..200).map(|i| json!([w * 0.2 + i as f64 * 3.0, h * 0.3 + (i as f64 * 0.1).sin() * 80.0, 0.8])).collect();
    let mut run = |label: &str, f: &mut dyn FnMut(&mut Session, usize)| {
        let mut times = Vec::new();
        for i in 0..reps {
            let t = Instant::now();
            f(&mut s, i);
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        println!("  {label:<40} {:8.1} ms  (max {:.1})", median(times.clone()), times.iter().copied().fold(0.0, f64::max));
    };
    let ex = |s: &mut Session, id: &str, p: Value| {
        s.execute(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    run("select a layer", &mut |s, i| ex(s, "layer.select", json!({"layer": pixel_layers[i % pixel_layers.len()]})));
    run("opacity of a layer", &mut |s, i| ex(s, "layer.setProps", json!({"layer": mid, "opacity": 0.5 + i as f64 * 0.05})));
    run("hide / show a layer", &mut |s, i| ex(s, "layer.setProps", json!({"layer": mid, "visible": i % 2 == 1})));
    run("blend mode of a layer", &mut |s, i| ex(s, "layer.setProps", json!({"layer": mid, "blend": if i % 2 == 0 { "multiply" } else { "normal" }})));
    run("new layer", &mut |s, _| ex(s, "layer.new.layer", json!({})));
    run("brush stroke (600 px)", &mut |s, _| ex(s, "paint.stroke", json!({"points": stroke})));
    run("undo", &mut |s, _| ex(s, "edit.undo", json!({})));
    run("redo", &mut |s, _| ex(s, "edit.redo", json!({})));
    run("select rect + fill + deselect", &mut |s, _| {
        ex(s, "select.rect", json!({"x": 100, "y": 100, "width": 400, "height": 300}));
        ex(s, "edit.fill", json!({"color": "#3366cc"}));
        ex(s, "select.deselect", json!({}));
    });
    run("duplicate a layer", &mut |s, _| {
        ex(s, "layer.select", json!({"layer": mid}));
        ex(s, "layer.duplicate", json!({}));
    });
    run("delete layer", &mut |s, _| ex(s, "layer.delete", json!({})));
    run("select all + deselect", &mut |s, _| {
        ex(s, "select.all", json!({}));
        ex(s, "select.deselect", json!({}));
    });
    // A pixel layer that is still there after the deletes.
    let live = |s: &Session| s.active().unwrap().doc.walk().iter().filter(|(_, _, l)| l.surface().is_some()).map(|(_, _, l)| l.id.0).nth(3).unwrap();
    run("move a layer 10 px", &mut |s, _| {
        let id = live(s);
        ex(s, "layer.translate", json!({"layer": id, "dx": 10, "dy": 0}));
    });
    run("gaussian blur 4 px on a layer", &mut |s, _| {
        let id = live(s);
        ex(s, "layer.select", json!({"layer": id}));
        ex(s, "filter.blur.gaussianBlur", json!({"radius": 4}));
    });
    run("fill tool click (all layers)", &mut |s, _| ex(s, "paint.fill", json!({"x": w * 0.5, "y": h * 0.5, "reference": "all", "closeGap": 3})));
    run("full composite (compose::render)", &mut |s, _| {
        let doc = s.active().unwrap().doc.clone();
        let _ = photocraft_compose::render(&doc, doc.bounds());
    });
}
