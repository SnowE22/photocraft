//! ⌘-click on the Channels panel (`select.loadSelection`): how long loading the composite's
//! luminosity or a colour channel takes, and how much of it is the composite render.
//!
//! ```sh
//! cargo run --release -p photocraft-engine --example bench_load_selection -- [file.psd]
//! ```
//!
//! Without a file it builds a 2400 × 3000 document with 12 filled layers and two adjustments.

use std::time::Instant;

use photocraft_engine::Session;
use serde_json::json;

fn main() {
    let mut s = Session::new();
    match std::env::args().nth(1) {
        Some(path) => {
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let t = Instant::now();
            let doc = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}")).document;
            println!("open {path}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
            s.add_document(doc, None);
        }
        None => {
            s.execute("file.new", json!({"width": 2400, "height": 3000, "resolution": 300, "background": "white"})).unwrap();
            for i in 0..12 {
                s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
                s.execute("select.rect", json!({"x": 100 + i * 150, "y": 100 + i * 200, "width": 900, "height": 1200})).unwrap();
                s.execute("edit.fill", json!({"color": format!("#{:02x}{:02x}{:02x}", (i * 37) % 256, (i * 91) % 256, (i * 53) % 256)})).unwrap();
                let _ = s.execute("select.deselect", json!({}));
                if i % 4 == 3 {
                    s.execute("layer.setProps", json!({"blend": "Multiply", "opacity": 0.8})).unwrap();
                }
            }
            s.execute("layer.newAdjustmentLayer.curves", json!({"points": [[0, 0], [128, 160], [255, 255]]})).unwrap();
            s.execute("layer.newAdjustmentLayer.hueSaturation", json!({"hue": 20, "saturation": 10})).unwrap();
        }
    }
    let doc = s.active().unwrap().doc.clone();
    println!("{} × {}, {} layers", doc.size.width, doc.size.height, doc.walk().len());
    for _ in 0..2 {
        let t = Instant::now();
        let buf = photocraft_compose::render(&doc, doc.bounds());
        println!("  compose::render                 {:7.0} ms ({} px)", t.elapsed().as_secs_f64() * 1e3, buf.px.len());
    }
    for (label, channel) in [("composite (luminosity)", json!("composite")), ("red", json!({"color": 0})), ("composite again", json!("composite"))] {
        let _ = s.execute("select.deselect", json!({}));
        let t = Instant::now();
        s.execute("select.loadSelection", json!({"channel": channel})).unwrap();
        println!("  load {label:26} {:7.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
    // An edit in between: the next load must see it.
    let _ = s.execute("select.deselect", json!({}));
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 300, "height": 300})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = s.execute("select.deselect", json!({}));
    let t = Instant::now();
    s.execute("select.loadSelection", json!({"channel": "composite"})).unwrap();
    println!("  load after an edit              {:7.0} ms", t.elapsed().as_secs_f64() * 1e3);
}
