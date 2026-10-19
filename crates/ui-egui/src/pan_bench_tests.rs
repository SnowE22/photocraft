//! Whole-app frame timings while panning with a ⌘-clicked composite selection on a real file
//! (headless, no GPU: the CPU side of each frame, every panel included):
//! `PHOTOCRAFT_BENCH_PSD=<file> cargo test --release -p photocraft-ui-egui --lib whole_app_pan -- --ignored --nocapture`.

use super::*;
use serde_json::json;

#[test]
#[ignore = "benchmark on a real file"]
fn whole_app_pan_with_a_loaded_composite_selection() {
    let Ok(path) = std::env::var("PHOTOCRAFT_BENCH_PSD") else { return };
    let bytes = std::fs::read(&path).expect("read");
    let doc = photocraft_io::import(&path, &bytes).expect("import").document;
    let (w, h) = (f32::from(u16::try_from(doc.size.width).unwrap_or(u16::MAX)), f32::from(u16::try_from(doc.size.height).unwrap_or(u16::MAX)));
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_pixels_per_point(1.0)
        .with_max_steps(8)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut s = photocraft_engine::Session::new();
            s.add_document(doc, None);
            PhotocraftApp::new(s, Services::default())
        });
    harness.run_steps(10);
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
    let pan = |harness: &mut egui_kittest::Harness<'static, PhotocraftApp>, label: &str, zoom: f32, step: f32| {
        let mut times = Vec::new();
        let mut spans = std::collections::BTreeMap::<&str, f64>::new();
        for i in 0..60 {
            let d = i as f32 * step;
            {
                let app = harness.state_mut();
                if let Some(v) = app.ui.views.get_mut(0) {
                    v.zoom = zoom;
                    v.center = [w / 2.0 + d, h / 2.0 + d / 2.0];
                }
                app.perf.spans.clear();
            }
            let t = std::time::Instant::now();
            harness.step();
            times.push(ms(t));
            for (k, v) in &harness.state().perf.spans {
                *spans.entry(k).or_default() += v;
            }
        }
        // At rest: an off-pixel pan rasterizes the ants again once the view is still.
        let mut rest = 0.0f64;
        for _ in 0..30 {
            let t = std::time::Instant::now();
            harness.step();
            rest = rest.max(ms(t));
        }
        times.sort_by(f64::total_cmp);
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        let big: Vec<_> = spans.into_iter().filter(|(_, v)| *v >= 5.0).map(|(k, v)| format!("{k} {v:.0}")).collect();
        println!("{label:38} avg {avg:5.1} ms  p50 {:5.1}  max {:5.1}  at rest max {rest:5.1}  spans>5ms(total) {big:?}", times[30], times[59]);
    };
    pan(&mut harness, "layers panel, no selection, fit", 0.3, 3.0);
    pan(&mut harness, "layers panel, no selection, 100 %", 1.0, 3.0);
    {
        let ctx = harness.ctx.clone();
        crate::shortcut_dispatch::dispatch(harness.state_mut(), &ctx, "window.panel.channels");
    }
    harness.run_steps(5);
    pan(&mut harness, "channels panel, no selection, fit", 0.3, 3.0);
    let t = std::time::Instant::now();
    harness.state_mut().run("select.loadSelection", json!({"channel": "composite"})).expect("load");
    println!("load composite: {:.0} ms", ms(t));
    let t = std::time::Instant::now();
    harness.step();
    println!("first frame after: {:.0} ms", ms(t));
    harness.run_steps(3);
    pan(&mut harness, "channels panel, selection, fit", 0.3, 3.0);
    pan(&mut harness, "channels panel, selection, 100 %", 1.0, 3.0);
    pan(&mut harness, "channels panel, selection, 100 % fast", 1.0, 20.0);
    pan(&mut harness, "channels panel, selection, 66.7 %", 0.667, 4.0);
    pan(&mut harness, "channels panel, selection, 200 %", 2.0, 2.0);
    {
        let ctx = harness.ctx.clone();
        crate::shortcut_dispatch::dispatch(harness.state_mut(), &ctx, "window.panel.layers");
    }
    harness.run_steps(5);
    pan(&mut harness, "layers panel, selection, fit", 0.3, 3.0);
    pan(&mut harness, "layers panel, selection, 100 %", 1.0, 3.0);
}
