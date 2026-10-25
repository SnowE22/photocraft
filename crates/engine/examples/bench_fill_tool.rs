//! Times the Fill tool (`paint.fill`) on a line-art page, as a Clip Studio style colouring
//! session does: many clicks in a row on a colour layer under a reference line layer.
//!
//! ```sh
//! cargo run --release -p photocraft-engine --example bench_fill_tool -- [width height]
//! ```

use std::time::Instant;

use photocraft_engine::Session;
use photocraft_geom::Rect;
use serde_json::json;

fn main() {
    let args: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (w, h) = (args.first().copied().unwrap_or(6000), args.get(1).copied().unwrap_or(4000));
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Lines"})).unwrap();
    // Line art: a grid of 3 px lines every 150 px, with a 4 px gap in every other segment.
    s.edit("lines", |doc, active| {
        let id = active.ok_or(photocraft_engine::EngineError::NoDocument)?;
        let surf = doc.layer_mut(id).and_then(|l| l.surface_mut()).ok_or(photocraft_engine::EngineError::NoDocument)?;
        let black = [0.0, 0.0, 0.0, 1.0];
        let (w, h) = (w as i32, h as i32);
        let mut k = 0;
        for x in (75..w).step_by(150) {
            for y in (0..h).step_by(150) {
                k += 1;
                let gap = if k % 2 == 0 { 4 } else { 0 };
                surf.fill_rect(Rect::from_xywh(x, y, 3, (150 - gap) as u32), &black);
            }
        }
        for y in (75..h).step_by(150) {
            for x in (0..w).step_by(150) {
                k += 1;
                let gap = if k % 3 == 0 { 4 } else { 0 };
                surf.fill_rect(Rect::from_xywh(x, y, (150 - gap) as u32, 3), &black);
            }
        }
        Ok(())
    })
    .unwrap();
    let lines = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setReference", json!({"layer": lines.0, "on": true})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Colour"})).unwrap();
    println!("{w}×{h}: grid every 150 px, gaps of 4 px");
    {
        let doc = s.active().unwrap().doc.clone();
        let area = doc.bounds();
        let t = Instant::now();
        let buf = photocraft_compose::render(&doc, area);
        let t1 = t.elapsed().as_secs_f64() * 1000.0;
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let img: Vec<[u8; 4]> = buf.px.iter().map(|px| px.map(q)).collect();
        let t2 = t.elapsed().as_secs_f64() * 1000.0;
        drop(buf);
        let mut o = photocraft_algo::fill::FillOptions::default();
        for (gap, scale) in [(0.0, 0.0), (5.0, 0.0), (5.0, 2.0)] {
            o.close_gap = gap;
            o.scale = scale;
            let t = Instant::now();
            let r = photocraft_algo::fill::click_region(&img, area, (170, 170), &o);
            println!("algo click gap {gap} scale {scale}: {:6.0} ms ({:?})", t.elapsed().as_secs_f64() * 1000.0, r.map(|r| r.bbox));
        }
        println!("composite {t1:6.0} ms, to u8 {:6.0} ms", t2 - t1);
    }
    for (label, params) in [
        ("click, reference layer, gap 0", json!({"reference": "reference", "closeGap": 0})),
        ("click, reference layer, gap 5", json!({"reference": "reference", "closeGap": 5})),
        ("click, reference layer, gap 5, scale 2", json!({"reference": "reference", "closeGap": 5, "areaScaling": 2})),
        ("click, all layers, gap 5", json!({"reference": "all", "closeGap": 5})),
        ("click, own layer, gap 5", json!({"reference": "layer", "closeGap": 5})),
    ] {
        let mut times = Vec::new();
        for i in 0..4 {
            let mut p = params.clone();
            p["x"] = json!(150 * (i + 1) + 20);
            p["y"] = json!(150 * (i + 1) + 20);
            let t = Instant::now();
            let r = s.execute("paint.fill", p).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(r["filled"], true, "{label}: {r}");
            if i == 0 {
                println!("  bounds {}", r["bounds"]);
            }
        }
        println!("{label:<44} {}", times.iter().map(|t| format!("{t:7.0} ms")).collect::<Vec<_>>().join(""));
    }
    let t = Instant::now();
    let r = s.execute("paint.fill", json!({"reference": "reference", "closeGap": 5, "lasso": [[0, 0], [1000, 0], [1000, 1000], [0, 1000]]})).unwrap();
    println!("{:<44} {:7.0} ms ({})", "enclose and fill (lasso)", t.elapsed().as_secs_f64() * 1000.0, r["filled"]);
}
