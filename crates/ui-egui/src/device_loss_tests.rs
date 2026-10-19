//! The window's graphics device lost (a driver reset): unsaved work goes to crash recovery, the
//! window closes and PhotoCraft is asked to start again with the saved documents (`gpu_status`).

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::gpu_status::DeviceLossRestart;

/// An app with two documents: "saved" (a file, no changes) and "edited" (unsaved changes), and
/// recording services.
struct Rig {
    app: PhotocraftApp,
    restarts: Arc<Mutex<Vec<DeviceLossRestart>>>,
    recovered: Arc<Mutex<Vec<String>>>,
    quits: Arc<Mutex<u32>>,
    health: photocraft_gpu::DeviceHealth,
}

fn rig(recovery_fails: bool, with_restart: bool) -> Rig {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    if let Some(st) = app.session.active_mut() {
        st.path = Some("C:/pages/saved.psd".into());
        std::sync::Arc::make_mut(&mut st.doc).name = "saved".into();
    }
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    if let Some(st) = app.session.active_mut() {
        std::sync::Arc::make_mut(&mut st.doc).name = "edited".into();
    }
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(app.session.documents().iter().any(|d| d.is_dirty()));
    let (restarts, recovered, quits) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(0)));
    let rec = recovered.clone();
    app.services.autosave = Some(Box::new(move |doc, _rev, _path| {
        if recovery_fails {
            return Err("disk full".into());
        }
        rec.lock().unwrap().push(doc.name.clone());
        Ok(())
    }));
    if with_restart {
        let r = restarts.clone();
        app.services.restart_after_device_loss = Some(Box::new(move |restart| r.lock().unwrap().push(restart)));
    }
    let q = quits.clone();
    app.services.quit = Some(Box::new(move || *q.lock().unwrap() += 1));
    let health = photocraft_gpu::DeviceHealth::new();
    app.watch_window_device(health.clone());
    Rig { app, restarts, recovered, quits, health }
}

#[test]
fn a_lost_window_device_saves_unsaved_work_and_restarts_with_the_saved_files() {
    let mut r = rig(false, true);
    let ctx = egui::Context::default();
    // Healthy: nothing happens.
    crate::gpu_status::check(&mut r.app, &ctx);
    assert!(!crate::gpu_status::window_lost(&r.app));
    r.health.mark(photocraft_gpu::Fault::Lost("DXGI_ERROR_DEVICE_HUNG".into()));
    crate::gpu_status::check(&mut r.app, &ctx);
    assert!(crate::gpu_status::window_lost(&r.app));
    assert_eq!(*r.recovered.lock().unwrap(), ["edited"], "only the unsaved document goes to crash recovery");
    let restarts = r.restarts.lock().unwrap().clone();
    assert_eq!(restarts.len(), 1);
    assert_eq!(restarts[0].reopen, ["C:/pages/saved.psd"], "the saved document is reopened from its file");
    assert!(restarts[0].not_saved.is_empty());
    assert!(restarts[0].reason.contains("DXGI_ERROR_DEVICE_HUNG"), "{}", restarts[0].reason);
    assert!(restarts[0].notice().contains("recovered the unsaved ones"));
    assert_eq!(*r.quits.lock().unwrap(), 1, "the window closes");
    // Later frames do nothing more (no second restart, nothing drawn).
    crate::gpu_status::check(&mut r.app, &ctx);
    assert_eq!(r.restarts.lock().unwrap().len(), 1);
    ctx.graphics_mut(|g| g.entry(egui::LayerId::background()).add(egui::Rect::EVERYTHING, egui::Shape::Noop));
    crate::gpu_status::discard_frame_if_lost(&r.app, &ctx);
    assert!(ctx.graphics(|g| g.get(egui::LayerId::background()).is_none_or(|l| l.is_empty())), "the frame's meshes are dropped");
}

#[test]
fn documents_that_cant_be_recovered_are_named_in_the_restart_notice() {
    let mut r = rig(true, true);
    r.health.mark(photocraft_gpu::Fault::Lost("device removed".into()));
    crate::gpu_status::check(&mut r.app, &egui::Context::default());
    let restarts = r.restarts.lock().unwrap().clone();
    assert_eq!(restarts[0].not_saved.len(), 1);
    assert!(restarts[0].not_saved[0].starts_with("edited: disk full"), "{:?}", restarts[0].not_saved);
    assert!(restarts[0].notice().contains("Could not be saved for recovery: edited: disk full"));
}

#[test]
fn without_a_restart_service_the_window_stops_drawing_but_nothing_crashes() {
    let mut r = rig(false, false);
    // An uncaptured error is not a loss: the window keeps drawing.
    r.health.mark(photocraft_gpu::Fault::Error("validation".into()));
    crate::gpu_status::check(&mut r.app, &egui::Context::default());
    assert!(!crate::gpu_status::window_lost(&r.app));
    let mut r = rig(false, false);
    r.health.mark(photocraft_gpu::Fault::Lost("gone".into()));
    crate::gpu_status::check(&mut r.app, &egui::Context::default());
    assert!(crate::gpu_status::window_lost(&r.app));
    assert_eq!(*r.recovered.lock().unwrap(), ["edited"], "the unsaved work is still saved");
    assert_eq!(*r.quits.lock().unwrap(), 0, "no restart to hand over to: the window stays");
}
