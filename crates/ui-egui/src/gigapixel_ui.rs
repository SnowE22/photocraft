//! The shell's half of File › Automate › Topaz Gigapixel AI… (`photocraft_engine::gigapixel_cmds`).
//!
//! The command sends the flattened image to Gigapixel's app and returns where it put it. While
//! Gigapixel has it, a strip at the bottom of the window says so and offers Cancel, and twice a
//! second the folder is looked at. When the upscaled export appears (Gigapixel writes it next to
//! the image, as its default export settings do), it is applied (`topazGigapixel.apply`): the
//! document resized to it and the image on top as "Topaz Gigapixel layer", as Photoshop's
//! plug-in leaves it. Then the folder is removed. Editing goes on meanwhile.

use photocraft_engine::gigapixel_cmds::{APPLY_ID, Pending};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A round trip in progress.
pub struct Waiting {
    pending: Pending,
    /// When to look at the folder next (`egui` input time, seconds).
    next_look: f64,
}

/// Seconds between looks at Gigapixel's export folder.
const LOOK_EVERY: f64 = 0.5;

/// After `file.automate.topazGigapixel` ran: wait for the export it set up, if it set one up.
pub fn started(app: &mut PhotocraftApp, r: &Value) {
    let Some(pending) = Pending::from_result(r) else { return };
    if let Some(old) = app.gigapixel.take() {
        old.pending.clean_up();
    }
    app.gigapixel = Some(Waiting { pending, next_look: 0.0 });
    app.ui.status_error = false;
    app.ui.status = tl!("Opened in Topaz Gigapixel AI: export the image there to bring it back").to_string();
}

/// Stop waiting; Gigapixel's window stays open, its export is no longer picked up.
pub fn cancel(app: &mut PhotocraftApp) {
    if let Some(w) = app.gigapixel.take() {
        w.pending.clean_up();
        app.ui.status_error = false;
        app.ui.status = tl!("Stopped waiting for Topaz Gigapixel AI").to_string();
    }
}

/// Is a round trip waiting (the strip is showing)?
pub fn waiting(app: &PhotocraftApp) -> bool {
    app.gigapixel.is_some()
}

/// Once per frame: look for the export, apply it when it is there, and show the strip.
pub fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(w) = app.gigapixel.as_mut() else { return };
    let now = ctx.input(|i| i.time);
    if now >= w.next_look {
        w.next_look = now + LOOK_EVERY;
        if let Some(path) = w.pending.poll() {
            let document = w.pending.document;
            arrived(app, &path.to_string_lossy(), document);
            return;
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(LOOK_EVERY));
    let mut cancelled = false;
    egui::Area::new(egui::Id::new("gigapixel-waiting")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tl!("Waiting for Topaz Gigapixel AI: export the upscaled image there to bring it back."));
                if ui.button(tl!("Cancel")).clicked() {
                    cancelled = true;
                }
            });
        });
    });
    if cancelled {
        cancel(app);
    }
}

/// The export is there: apply it to the document it came from.
fn arrived(app: &mut PhotocraftApp, path: &str, document: u64) {
    let Some(w) = app.gigapixel.take() else { return };
    match app.run(APPLY_ID, json!({"result": path, "document": document})) {
        Ok(r) => {
            let (wd, ht) = (r["width"].as_u64().unwrap_or(0).to_string(), r["height"].as_u64().unwrap_or(0).to_string());
            let scale = format!("{:.2}", r["scale"].as_f64().unwrap_or(1.0)).trim_end_matches('0').trim_end_matches('.').to_string();
            app.ui.status_error = false;
            app.ui.status = crate::i18n::fmt(tl!("Topaz Gigapixel AI: upscaled {scale}× to {w} × {h} px (new layer)"), &[("scale", &scale), ("w", &wd), ("h", &ht)]);
        }
        Err(e) => {
            app.ui.status_error = true;
            app.ui.status = crate::i18n::fmt(tl!("Topaz Gigapixel AI's image could not be applied: {error}"), &[("error", &e)]);
        }
    }
    w.pending.clean_up();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A started round trip shows the strip; the export in the folder is applied to its document
    /// on the next look, the folder goes, and the status says what arrived.
    #[test]
    fn an_export_in_the_folder_is_applied() {
        let dir = std::env::temp_dir().join("PhotoCraft").join("Gigapixel").join(format!("ui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("photo.tif");
        std::fs::write(&input, b"tiff").unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
        let doc = app.session.active().unwrap().doc.id.0;
        started(&mut app, &json!({"waiting": true, "document": doc, "folder": dir.to_string_lossy(), "input": input.to_string_lossy()}));
        assert!(waiting(&app));
        // Gigapixel exports a 2× PNG next to the image.
        let mut other = photocraft_engine::Session::new();
        other.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        other.execute("file.saveACopy", json!({"path": dir.join("photo-gigapixel-std-2x.png").to_string_lossy(), "layers": false})).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let time = 10.0 + app.gigapixel.as_ref().map_or(0.0, |w| w.next_look);
            let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| frame(&mut app, ui.ctx()));
            out.textures_delta.clear();
        }
        assert!(!waiting(&app), "applied");
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.size.width, st.doc.size.height), (40, 20));
        assert_eq!(st.doc.layers.last().unwrap().name, photocraft_engine::gigapixel_cmds::LAYER_NAME);
        assert!(app.ui.status.contains("2×") && app.ui.status.contains("40 × 20"), "{}", app.ui.status);
        assert!(!dir.exists(), "the folder is removed");
    }

    #[test]
    fn cancel_stops_waiting_and_removes_the_folder() {
        let dir = std::env::temp_dir().join("PhotoCraft").join("Gigapixel").join(format!("ui-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        started(&mut app, &json!({"waiting": true, "document": 1, "folder": dir.to_string_lossy(), "input": dir.join("x.tif").to_string_lossy()}));
        assert!(waiting(&app));
        cancel(&mut app);
        assert!(!waiting(&app));
        assert!(!dir.exists());
        // Results that set up nothing start nothing.
        started(&mut app, &json!({"layer": 3}));
        assert!(!waiting(&app));
    }
}
