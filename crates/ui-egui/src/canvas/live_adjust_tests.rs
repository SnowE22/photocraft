//! Dragging an adjustment layer's control on a document above the proxy threshold (a user
//! report: 2400 × 3000 with 31 layers lagged at 220 ms a frame). The GPU tests skip when there
//! is no adapter.

use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

/// The app on a real device with background jobs, or `None` without a GPU adapter.
fn gpu_app() -> Option<egui_kittest::Harness<'static, PhotocraftApp>> {
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app.background_jobs = true;
            app
        })
    }));
    let Ok(harness) = built else {
        eprintln!("skipping: no GPU adapter");
        return None;
    };
    harness.state().gpu.is_some().then_some(harness)
}

/// The live parameters of `layer` with Hue set to `hue`.
fn hue_params(app: &PhotocraftApp, layer: LayerId, hue: f64) -> serde_json::Value {
    let st = app.session.active().expect("document");
    let LayerContent::Adjustment(a) = &st.doc.layer(layer).expect("layer").content else { panic!("not an adjustment") };
    let mut v = photocraft_engine::adjust_params::to_params(a);
    v["hue"] = json!(hue);
    v
}

/// The wgpu compositor draws the full-size live preview; the downsampled CPU proxy (200 ms a
/// frame on that file, against 5 ms on the GPU) is only for documents the CPU composites.
#[test]
fn a_live_adjustment_on_a_large_document_composites_on_the_gpu_not_the_cpu_proxy() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = gpu_app() else { return };
    let app = h.state_mut();
    app.run("file.new", json!({"width": 2600, "height": 2200, "background": "#336699"})).expect("new");
    let layer = LayerId(app.run("layer.newAdjustmentLayer.hueSaturation", json!({})).expect("adjustment")["layer"].as_u64().expect("id"));
    let doc = app.session.active().expect("document").doc.clone();
    assert!(crate::proxy::preview_factor(&doc, crate::proxy::reduced_previews(app)) >= 2, "large enough for the proxy");
    h.run_steps(3);
    for hue in [40.0, 80.0, 120.0] {
        let p = hue_params(h.state(), layer, hue);
        h.state_mut().live_adjust = Some((layer, p));
        h.step();
        let app = h.state();
        assert!(app.proxy.is_none() && app.proxy_uploaded.is_none(), "no CPU proxy was built (hue {hue})");
        assert_eq!(app.perf.last_refresh, "gpu-full", "the canvas composited the full-size preview on the GPU (hue {hue})");
    }
}
