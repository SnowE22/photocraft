//! Window › Timelapse: the panel of File › Timelapse (`photocraft_engine::timelapse_cmds`), after
//! the Photoshop Timelapse Recorder's panel. Every action runs a `file.timelapse.*` command, so the
//! control channel and the menus drive the same thing; the panel only shows the state.
//!
//! Each frame, [`frame`] lets the recorder follow edits and capture (`Session::timelapse_tick`)
//! and asks for a repaint when the next frame is due, so capturing works without input.

use std::sync::Arc;

use egui::{RichText, vec2};
use photocraft_doc::DocId;
use photocraft_engine::jobs::{JobEvent, JobOutcome};
use photocraft_engine::timelapse_cmds::{self as tc, ExportSettings, Frame};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The Window menu item.
pub const WINDOW_ID: &str = "window.timelapse";

/// Panel state (not saved).
#[derive(Default)]
pub struct TimelapseUi {
    pub open: bool,
    /// The export box, with the options being chosen (`None`: closed).
    export: Option<ExportSettings>,
    preview: bool,
    /// The preview texture: document, preview revision, image.
    texture: Option<(DocId, u64, egui::TextureHandle)>,
    /// The last exported file (Play / Show in Folder).
    last_export: Option<String>,
    /// The Library section is open (it lists the recordings only then).
    library_open: bool,
    /// `file.timelapse.list` and when it was read (`egui` time): the Library re-reads it every
    /// few seconds while open, and after each change.
    library: Option<(f64, Value)>,
    /// A Library deletion waiting for the user's yes.
    confirm: Option<Confirm>,
}

/// A deletion to confirm in the Library.
#[derive(Clone)]
struct Confirm {
    command: &'static str,
    id: String,
    name: String,
    text: String,
    /// "Delete frames" or "Delete folder".
    frames: bool,
}

/// The Library re-reads the recordings folder this often while open (seconds).
const LIBRARY_REFRESH: f64 = 3.0;

/// Menu checkmark for Window › Timelapse.
pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    (id == WINDOW_ID).then_some(app.timelapse.open)
}

/// Ids this module answers itself.
pub fn handles(id: &str) -> bool {
    id == WINDOW_ID
}

/// Window › Timelapse toggles the panel. File › Timelapse › Export Timelapse… without a path opens
/// the panel's export box; Start Recording asks for the recordings folder the first time.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        WINDOW_ID => {
            app.timelapse.open = !app.timelapse.open;
            Some(Ok(json!({"open": app.timelapse.open})))
        }
        tc::EXPORT if params.get("path").is_none() => {
            app.timelapse.open = true;
            app.timelapse.export = Some(app.session.timelapse_settings().export);
            Some(Ok(json!({"open": true})))
        }
        tc::RECORD if app.session.timelapse_settings().folder.is_empty() => {
            app.timelapse.open = true;
            Some(choose_folder(app, true))
        }
        _ => None,
    }
}

/// Ask for the recordings folder (then start recording, when `record`).
fn choose_folder(app: &mut PhotocraftApp, record: bool) -> Result<Value, String> {
    app.ui.status = tl!("Choose a folder for your timelapse recordings").into();
    app.ui.status_error = false;
    app.ask_folder(move |app, folder| {
        let v = app.run(tc::SETTINGS, json!({"folder": folder}))?;
        if record { app.run(tc::RECORD, json!({})) } else { Ok(v) }
    })
}

/// Ask where the video (or the frames) go, then export.
fn choose_output(app: &mut PhotocraftApp, o: ExportSettings) -> Result<Value, String> {
    let mut params = serde_json::to_value(&o).unwrap_or_else(|_| json!({}));
    if let Some(m) = params.as_object_mut()
        && o.length_sec.is_none()
    {
        m.insert("lengthSec".into(), json!("auto"));
    }
    let run = move |app: &mut PhotocraftApp, path: String| {
        if let Some(m) = params.as_object_mut() {
            m.insert("path".into(), json!(path));
        }
        app.run(tc::EXPORT, params)
    };
    if o.encoder == "frames" {
        app.ask_folder(run)
    } else {
        let name = app.session.active().map_or_else(|| "Untitled".to_string(), |d| d.doc.name.rsplit_once('.').map_or(d.doc.name.clone(), |(a, _)| a.to_string()));
        app.pick_save(&format!("{name} timelapse.mp4"), run)
    }
}

/// A finished export: say where the video went. `true` when the event was this module's.
pub fn on_job_event(app: &mut PhotocraftApp, e: &JobEvent) -> bool {
    if e.command != tc::EXPORT {
        return false;
    }
    match &e.outcome {
        JobOutcome::Done(v) => {
            let file = v.get("file").and_then(Value::as_str).unwrap_or_default().to_string();
            let secs = v.get("seconds").and_then(Value::as_f64).unwrap_or(0.0).round() as u64;
            let name = std::path::Path::new(&file).file_name().map_or_else(|| file.clone(), |n| n.to_string_lossy().into_owned());
            app.ui.status = crate::i18n::fmt(tl!("Saved {name} ({seconds} s timelapse)"), &[("name", &name), ("seconds", &secs.to_string())]);
            if let Some(w) = v.get("warning").and_then(Value::as_str) {
                app.ui.status = format!("{} ({w})", app.ui.status);
            }
            app.ui.status_error = false;
            app.timelapse.last_export = Some(file);
            app.timelapse.export = None;
        }
        JobOutcome::Failed(err) => crate::notices::error(app, format!("{}: {err}", tl!("Export Timelapse"))),
        JobOutcome::Cancelled => {
            app.ui.status = tl!("Timelapse export cancelled").into();
            app.ui.status_error = false;
        }
    }
    true
}

/// Once per frame: follow edits and capture, pass on messages, draw the panel.
pub fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let input = ctx.input(|i| i.pointer.any_down() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. } | egui::Event::Text(_))));
    if let Some(ms) = app.session.timelapse_tick(input) {
        ctx.request_repaint_after(std::time::Duration::from_millis(ms.clamp(16.0, 60_000.0) as u64));
    }
    for m in app.session.timelapse_messages() {
        app.ui.status = m;
        app.ui.status_error = false;
    }
    if app.timelapse.open {
        window(app, ctx);
    }
}

/// "1 h 05 min" style work time.
fn work_text(ms: u64) -> String {
    let min = ms / 60_000;
    let (h, m) = (min / 60, min % 60);
    if h > 0 {
        format!("{h} h {m:02} min")
    } else if min > 0 {
        format!("{m} min")
    } else {
        format!("{} s", ms / 1000)
    }
}

fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{mb:.1} MB") }
}

/// The preview texture of `doc`, refreshed when its newest frame changed.
fn preview(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) -> Option<egui::TextureHandle> {
    let (rev, path, live) = app.session.timelapse.preview(doc)?;
    if let Some((d, r, tex)) = &app.timelapse.texture
        && *d == doc
        && *r == rev
    {
        return Some(tex.clone());
    }
    let frame: Arc<Frame> = match live {
        Some(f) => f,
        None => {
            let (w, h, rgb) = tc::export::decode_frame(&path).ok()?;
            Arc::new(Frame { width: w as u32, height: h as u32, rgb })
        }
    };
    let img = egui::ColorImage::from_rgb([frame.width as usize, frame.height as usize], &frame.rgb);
    let tex = ctx.load_texture("timelapse-preview", img, egui::TextureOptions::LINEAR);
    app.timelapse.texture = Some((doc, rev, tex.clone()));
    Some(tex)
}

const LENGTHS: [(Option<u32>, &str); 7] =
    [(None, "Auto"), (Some(10), "10 seconds"), (Some(20), "20 seconds"), (Some(30), "30 seconds"), (Some(60), "1 minute"), (Some(120), "2 minutes"), (Some(300), "5 minutes")];

/// "12 frames" in the current language.
fn frames_text(n: u64) -> String {
    crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "{n} frame", "{n} frames"), &[])
}

/// The Library: every recording in the folder with its frames, size and work time, whether it
/// was exported, and buttons to open its folder or delete it after a yes (the plug-in's Library
/// tab). Deleting is refused while a recording records.
fn library_section(ui: &mut egui::Ui, t: &Tokens, library: &Value, confirm: &mut Option<Confirm>, ask: &mut Option<Ask>) {
    let folder = library.get("folder").and_then(Value::as_str).unwrap_or_default();
    let recs = library.get("recordings").and_then(Value::as_array).cloned().unwrap_or_default();
    ui.horizontal(|ui| {
        let total: u64 = recs.iter().filter_map(|r| r.get("bytes").and_then(Value::as_u64)).sum();
        let count = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), recs.len() as u64, "{n} recording", "{n} recordings"), &[]);
        ui.label(RichText::new(format!("{} · {count}", size_text(total))).color(t.text_dim).size(11.0));
        if crate::widgets::secondary_button(ui, tl!("Refresh"), 0.0).clicked() {
            *ask = Some(Ask::RefreshLibrary);
        }
    });
    if recs.is_empty() {
        ui.weak(if folder.is_empty() { tl!("Recordings will be listed here once you've chosen a folder.") } else { tl!("No recordings yet.") });
        return;
    }
    if let Some(c) = confirm.clone() {
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.danger)).corner_radius(t.radius).inner_margin(6.0).show(ui, |ui| {
            ui.label(RichText::new(&c.text).color(t.text));
            ui.horizontal(|ui| {
                if crate::widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
                    *confirm = None;
                }
                let label = if c.frames { tl!("Delete frames") } else { tl!("Delete folder") };
                if ui.add(egui::Button::new(RichText::new(label).color(t.primary_text)).fill(t.danger)).clicked() {
                    *ask = Some(Ask::Confirm(c.clone()));
                }
            });
        });
        ui.add_space(4.0);
    }
    egui::ScrollArea::vertical().id_salt("timelapse-library-list").max_height(260.0).show(ui, |ui| {
        for r in &recs {
            library_row(ui, t, r, confirm, ask);
        }
    });
}

fn library_row(ui: &mut egui::Ui, t: &Tokens, r: &Value, confirm: &mut Option<Confirm>, ask: &mut Option<Ask>) {
    let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let num = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
    let flag = |k: &str| r.get(k).and_then(Value::as_bool).unwrap_or(false);
    let (id, name, dir) = (text("id"), text("name"), text("dir"));
    let (frames, all, bytes, work, since) = (num("frames"), num("allFrames"), num("bytes"), num("workMs"), num("framesSinceExport"));
    let (recording, exported) = (flag("recording"), flag("exported"));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        crate::widgets::elided_label(ui, &name, 170.0).on_hover_text(&dir);
        if recording {
            ui.label(RichText::new(tl!("REC")).color(t.danger).size(10.0).strong());
        }
        if exported {
            ui.label(RichText::new(tl!("EXPORTED")).color(t.accent_text).size(10.0).strong());
        } else if all > 0 {
            ui.label(RichText::new(tl!("NOT EXPORTED")).color(t.warning).size(10.0).strong());
        }
    });
    let work_label = crate::i18n::fmt(tl!("{time} work"), &[("time", &work_text(work))]);
    ui.label(RichText::new(format!("{} · {} · {work_label}", frames_text(frames), size_text(bytes))).color(t.text_dim).size(11.0));
    if exported && since > 0 && all > 0 {
        ui.label(RichText::new(tl!("Recorded more since the last export")).color(t.text_faint).size(11.0));
    }
    ui.horizontal(|ui| {
        if crate::widgets::secondary_button(ui, tl!("Open folder"), 0.0).clicked() {
            *ask = Some(Ask::Open(dir.clone()));
        }
        ui.add_enabled_ui(!recording, |ui| {
            let label = if all > 0 { tl!("Delete frames") } else { tl!("Delete folder") };
            let b = crate::widgets::secondary_button(ui, label, 0.0).on_disabled_hover_text(tl!("Pause the recording first"));
            if !b.clicked() {
                return;
            }
            let text = if all == 0 {
                crate::i18n::fmt(
                    tl!("Delete “{name}” and its folder? It's removed from the library, along with its work time. Videos you exported stay where you saved them. This can't be undone."),
                    &[("name", &name)],
                )
            } else {
                let what = format!("{} ({})", frames_text(all), size_text(bytes));
                if exported {
                    let mut s = crate::i18n::fmt(tl!("Delete the {frames} of “{name}”?"), &[("name", &name), ("frames", &what)]);
                    if since > 0 {
                        s = format!("{s} {}", tl!("You've recorded more since the last export, and that part isn't in the video."));
                    }
                    format!("{s} {}", tl!("Your videos and the work time are kept. This can't be undone."))
                } else {
                    crate::i18n::fmt(
                        tl!("“{name}” hasn't been exported yet. If you delete its {frames}, you can't make a video of it anymore. This can't be undone."),
                        &[("name", &name), ("frames", &what)],
                    )
                }
            };
            let command = if all > 0 { tc::DELETE_FRAMES } else { tc::DELETE_RECORDING };
            *confirm = Some(Confirm { command, id: id.clone(), name: name.clone(), text, frames: all > 0 });
        });
    });
}

/// What the panel asks for once it's drawn (the panel can't borrow the app mutably while drawing).
enum Ask {
    Run(&'static str, Value),
    Settings(Value),
    ChooseFolder,
    ChooseFfmpeg,
    Export(ExportSettings),
    Open(String),
    CloseExport,
    /// Run a Library deletion the user confirmed.
    Confirm(Confirm),
    RefreshLibrary,
}

fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let t = Tokens::get(ctx);
    let settings = app.session.timelapse_settings();
    let doc = app.session.active().map(|d| (d.doc.id, d.doc.name.clone()));
    let status = app.session.timelapse_status();
    let rec = status.get("recording").filter(|v| !v.is_null()).cloned();
    let num = |k: &str| rec.as_ref().and_then(|r| r.get(k)).and_then(Value::as_u64).unwrap_or(0);
    let recording = rec.as_ref().and_then(|r| r["recording"].as_bool()).unwrap_or(false);
    let idle = rec.as_ref().and_then(|r| r["idle"].as_bool()).unwrap_or(false);
    let (frames, work, bytes, since_export) = (num("frames"), num("workMs"), num("bytes"), num("framesSinceExport"));
    let exported = num("exports") > 0;
    let tex = if app.timelapse.preview { doc.as_ref().and_then(|(id, _)| preview(app, ctx, *id)) } else { None };
    let (selected, refs) = app
        .session
        .active()
        .map(|st| {
            let tag = settings.reference_tag.to_lowercase();
            let ids = st.selected_layers();
            let refs = ids.iter().filter(|id| st.doc.layer(**id).is_some_and(|l| l.name.to_lowercase().contains(&tag))).count();
            (ids.len(), refs)
        })
        .unwrap_or((0, 0));

    let now = ctx.input(|i| i.time);
    if app.timelapse.library_open && app.timelapse.library.as_ref().is_none_or(|(at, _)| now - at >= LIBRARY_REFRESH) {
        let v = app.session.execute(tc::LIST, json!({})).unwrap_or_else(|_| json!({"folder": "", "recordings": []}));
        app.timelapse.library = Some((now, v));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(LIBRARY_REFRESH));
    }
    let library = app.timelapse.library.as_ref().map(|(_, v)| v.clone()).unwrap_or(Value::Null);
    let mut library_open = app.timelapse.library_open;
    let mut confirm = app.timelapse.confirm.clone();

    let mut export = app.timelapse.export.clone();
    let mut show_preview = app.timelapse.preview;
    let last_export = app.timelapse.last_export.clone();
    let mut ask: Option<Ask> = None;
    let mut close = false;

    crate::analysis_ui::panel_window(app, ctx, "timelapse", tl!("Timelapse"), vec2(-40.0, 120.0), 300.0, |ui| {
        close = crate::analysis_ui::title_row(ui, "Timelapse");
        let mut auto = settings.auto_record;
        if crate::widgets::toggle(ui, &mut auto, tl!("Auto record")).on_hover_text(tl!("Records whatever document you're on, unless you paused it yourself.")).changed() {
            ask = Some(Ask::Settings(json!({"autoRecord": auto})));
        }
        ui.add_space(4.0);
        let Some((_, name)) = &doc else {
            ui.weak(tl!("Open a document to record a timelapse."));
            crate::widgets::hairline(ui);
            let section = egui::CollapsingHeader::new(tl!("Library")).id_salt("timelapse-library").default_open(library_open).show(ui, |ui| library_section(ui, &t, &library, &mut confirm, &mut ask));
            library_open = section.body_returned.is_some();
            return;
        };
        ui.label(RichText::new(name).color(t.text).strong());
        // The record button: Start recording, Recording (click to pause), Paused (click to resume).
        let w = ui.available_width();
        let clicked = if recording {
            let label = if idle { tl!("Recording (idle)") } else { tl!("Recording") };
            crate::widgets::primary_button(ui, &format!("● {label}"), w).on_hover_text(tl!("Click to pause")).clicked()
        } else if rec.is_some() {
            crate::widgets::secondary_button(ui, tl!("Paused: click to resume"), w).clicked()
        } else {
            crate::widgets::primary_button(ui, tl!("Start recording"), w).clicked()
        };
        if clicked {
            ask = Some(if recording { Ask::Run(tc::PAUSE, json!({})) } else { Ask::Run(tc::RECORD, json!({})) });
        }
        if rec.is_some() {
            let frames_text = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), frames, "{n} frame", "{n} frames"), &[]);
            ui.label(RichText::new(format!("{frames_text} · {} · {}", work_text(work), size_text(bytes))).color(t.text_dim).size(11.0));
            if exported && since_export > 0 {
                ui.label(RichText::new(tl!("Recorded more since the last export")).color(t.text_faint).size(11.0));
            }
            crate::widgets::checkbox(ui, &mut show_preview, tl!("Preview"));
            if let Some(tex) = &tex {
                let size = tex.size_vec2();
                let scale = (ui.available_width() / size.x).min(180.0 / size.y).min(1.0);
                ui.image((tex.id(), size * scale));
            }
        }

        crate::widgets::hairline(ui);
        // Reference layers: visible to you, hidden in the timelapse.
        ui.label(RichText::new(tl!("Reference layer")).color(t.text).size(11.0).strong());
        let state = match (selected, refs) {
            (0, _) => tl!("Select a layer").to_string(),
            (1, 1) => tl!("Not recorded: you see it, the timelapse doesn't").to_string(),
            (1, _) => tl!("Recorded").to_string(),
            (n, r) => crate::i18n::fmt(tl!("{count} of {total} not recorded"), &[("count", &r.to_string()), ("total", &n.to_string())]),
        };
        ui.label(RichText::new(state).color(t.text_dim).size(11.0));
        ui.horizontal(|ui| {
            ui.add_enabled_ui(selected > 0, |ui| {
                if crate::widgets::secondary_button(ui, tl!("Normal"), 0.0).clicked() {
                    ask = Some(Ask::Run(tc::REFERENCE, json!({"reference": false})));
                }
                if crate::widgets::secondary_button(ui, tl!("Reference"), 0.0).on_hover_text(crate::i18n::fmt(tl!("Adds {tag} to the layer's name"), &[("tag", &settings.reference_tag)])).clicked() {
                    ask = Some(Ask::Run(tc::REFERENCE, json!({"reference": true})));
                }
            });
        });

        crate::widgets::hairline(ui);
        // Export.
        match &mut export {
            None => {
                ui.add_enabled_ui(frames > 0, |ui| {
                    let label = if exported { tl!("Export again…") } else { tl!("Export video…") };
                    if crate::widgets::secondary_button(ui, label, ui.available_width()).clicked() {
                        export = Some(settings.export.clone());
                    }
                });
                if let Some(file) = &last_export {
                    ui.horizontal(|ui| {
                        if crate::widgets::secondary_button(ui, tl!("Play video"), 0.0).clicked() {
                            ask = Some(Ask::Open(file.clone()));
                        }
                        if crate::widgets::secondary_button(ui, tl!("Show in folder"), 0.0).clicked() {
                            let dir = std::path::Path::new(file).parent().map_or_else(|| file.clone(), |d| d.to_string_lossy().into_owned());
                            ask = Some(Ask::Open(dir));
                        }
                    });
                }
            }
            Some(o) => {
                let mut length = o.length_sec.map(|s| s.round() as u32);
                ui.horizontal(|ui| {
                    ui.label(tl!("Length:"));
                    if crate::widgets::dropdown(ui, "timelapse-length", &mut length, &LENGTHS, 140.0) {
                        o.length_sec = length.map(f64::from);
                    }
                });
                let mut enc = o.encoder.clone();
                ui.horizontal(|ui| {
                    ui.label(tl!("Make:"));
                    if crate::widgets::dropdown(ui, "timelapse-encoder", &mut enc, &[("ffmpeg".to_string(), "MP4 video (ffmpeg)"), ("frames".to_string(), "JPEG frames")], 160.0) {
                        o.encoder = enc;
                    }
                });
                egui::CollapsingHeader::new(tl!("Advanced settings")).id_salt("timelapse-advanced").show(ui, |ui| {
                    let mut intro = o.intro_sec > 0.0;
                    if crate::widgets::checkbox(ui, &mut intro, tl!("Show the finished drawing first")).changed() {
                        o.intro_sec = if intro { 1.0 } else { 0.0 };
                    }
                    crate::widgets::checkbox(ui, &mut o.flash, tl!("Flash before the final image"));
                    let mut hold = o.hold_sec > 0.0;
                    if crate::widgets::checkbox(ui, &mut hold, tl!("Hold the final image")).changed() {
                        o.hold_sec = if hold { 3.0 } else { 0.0 };
                    }
                    crate::widgets::checkbox(ui, &mut o.slow_end, tl!("Slow down at the end"));
                    crate::widgets::checkbox(ui, &mut o.deflicker, tl!("Remove flicker")).on_hover_text(tl!(
                        "Leaves out frames that flash in and out: a layer hidden for a moment, a stroke undone right away."
                    ));
                    crate::widgets::checkbox(ui, &mut o.timer, tl!("Work-time counter"));
                    if o.timer {
                        let mut corner = o.corner.clone();
                        let corners = [
                            ("br".to_string(), "Bottom right"),
                            ("bl".to_string(), "Bottom left"),
                            ("tr".to_string(), "Top right"),
                            ("tl".to_string(), "Top left"),
                        ];
                        if crate::widgets::dropdown(ui, "timelapse-corner", &mut corner, &corners, 140.0) {
                            o.corner = corner;
                        }
                    }
                    let mut bars = o.letterbox.clone();
                    ui.horizontal(|ui| {
                        ui.label(tl!("Bars:")).on_hover_text(tl!("Fills the edges when the canvas size changed during the recording."));
                        let colors = [("black".to_string(), "Black"), ("white".to_string(), "White"), ("gray".to_string(), "Gray")];
                        if crate::widgets::dropdown(ui, "timelapse-bars", &mut bars, &colors, 100.0) {
                            o.letterbox = bars;
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if crate::widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
                        ask = Some(Ask::CloseExport);
                    }
                    if crate::widgets::primary_button(ui, tl!("Export…"), 0.0).clicked() {
                        ask = Some(Ask::Export(o.clone()));
                    }
                });
            }
        }

        crate::widgets::hairline(ui);
        let section = egui::CollapsingHeader::new(tl!("Library")).id_salt("timelapse-library").default_open(library_open).show(ui, |ui| library_section(ui, &t, &library, &mut confirm, &mut ask));
        library_open = section.body_returned.is_some();

        crate::widgets::hairline(ui);
        egui::CollapsingHeader::new(tl!("Settings")).id_salt("timelapse-settings").default_open(settings.folder.is_empty()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl!("Folder:"));
                let shown = if settings.folder.is_empty() { tl!("not chosen yet").to_string() } else { settings.folder.clone() };
                crate::widgets::elided_label(ui, &shown, 150.0).on_hover_text(&settings.folder);
                if crate::widgets::secondary_button(ui, tl!("Choose…"), 0.0).clicked() {
                    ask = Some(Ask::ChooseFolder);
                }
            });
            let mut quality = settings.quality.clone();
            ui.horizontal(|ui| {
                ui.label(tl!("Quality:"));
                let q = [("low".to_string(), "Low"), ("default".to_string(), "Default"), ("high".to_string(), "High")];
                if crate::widgets::dropdown(ui, "timelapse-quality", &mut quality, &q, 110.0) {
                    ask = Some(Ask::Settings(json!({"quality": quality})));
                }
            });
            let mut hide = settings.hide_undone;
            if crate::widgets::checkbox(ui, &mut hide, tl!("Remove frames with undo")).changed() {
                ask = Some(Ask::Settings(json!({"hideUndone": hide})));
            }
            let mut tag = settings.reference_tag.clone();
            ui.horizontal(|ui| {
                ui.label(tl!("Reference tag:"));
                let mut tags: Vec<(String, &str)> = tc::REFERENCE_TAGS.iter().map(|s| (s.to_string(), *s)).collect();
                if !tc::REFERENCE_TAGS.contains(&tag.as_str()) {
                    tags.push((tag.clone(), tag.as_str()));
                }
                let tags: Vec<(String, String)> = tags.into_iter().map(|(a, b)| (a, b.to_string())).collect();
                let options: Vec<(String, &str)> = tags.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
                if crate::widgets::dropdown_literal(ui, "timelapse-tag", &mut tag, &options, 110.0) {
                    ask = Some(Ask::Settings(json!({"referenceTag": tag})));
                }
            });
            let mut idle_sec = settings.idle_sec;
            ui.horizontal(|ui| {
                ui.label(tl!("Work timer stops after:"));
                let opts = [(5u32, "5 seconds"), (10, "10 seconds"), (15, "15 seconds"), (30, "30 seconds"), (60, "1 minute")];
                if crate::widgets::dropdown(ui, "timelapse-idle", &mut idle_sec, &opts, 100.0) {
                    ask = Some(Ask::Settings(json!({"idleSec": idle_sec})));
                }
            });
            ui.horizontal(|ui| {
                ui.label("ffmpeg:");
                let shown = if settings.ffmpeg.is_empty() { tl!("Automatic").to_string() } else { settings.ffmpeg.clone() };
                crate::widgets::elided_label(ui, &shown, 130.0).on_hover_text(tl!("Makes the MP4. Automatic looks next to PhotoCraft, then on PATH."));
                if crate::widgets::secondary_button(ui, tl!("Choose…"), 0.0).clicked() {
                    ask = Some(Ask::ChooseFfmpeg);
                }
                if !settings.ffmpeg.is_empty() && crate::icons::button(ui, "x", 18.0, false, tl!("Automatic")).clicked() {
                    ask = Some(Ask::Settings(json!({"ffmpeg": ""})));
                }
            });
        });
    });

    app.timelapse.preview = show_preview;
    app.timelapse.export = export;
    if library_open && !app.timelapse.library_open {
        app.timelapse.library = None;
    }
    app.timelapse.library_open = library_open;
    app.timelapse.confirm = confirm;
    if close {
        app.timelapse.open = false;
    }
    let r = match ask {
        None => return,
        Some(Ask::CloseExport) => {
            app.timelapse.export = None;
            return;
        }
        Some(Ask::RefreshLibrary) => {
            app.timelapse.library = None;
            return;
        }
        Some(Ask::Confirm(c)) => {
            app.timelapse.confirm = None;
            app.timelapse.library = None;
            app.run(c.command, json!({"recording": c.id})).inspect(|v| {
                app.ui.status = if c.frames {
                    tl!("Frames deleted.").to_string()
                } else if v.get("folderRemoved").and_then(Value::as_bool) == Some(false) {
                    crate::i18n::fmt(tl!("Deleted “{name}”. Its folder holds other files, so it was kept."), &[("name", &c.name)])
                } else {
                    crate::i18n::fmt(tl!("Deleted “{name}”."), &[("name", &c.name)])
                };
                app.ui.status_error = false;
            })
        }
        Some(Ask::Run(id, p)) => app.run(id, p),
        Some(Ask::Settings(p)) => app.run(tc::SETTINGS, p),
        Some(Ask::ChooseFolder) => choose_folder(app, false),
        Some(Ask::ChooseFfmpeg) => {
            use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
            let ext = if cfg!(windows) { Some(vec!["exe".to_string()]) } else { None };
            app.ask_file(FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: ext }, |app, answer| match answer {
                FileDialogAnswer::Paths(p) => {
                    let path = p.into_iter().next().ok_or_else(|| crate::file_dialog::CANCELLED.to_string())?;
                    app.run(tc::SETTINGS, json!({"ffmpeg": path}))
                }
                _ => Err(tl!("Choose the ffmpeg program on this computer").into()),
            })
        }
        Some(Ask::Export(o)) => choose_output(app, o),
        Some(Ask::Open(path)) => match app.services.open_url.as_ref() {
            Some(open) => open(&path).map(|()| Value::Null),
            None => Err(tl!("Opening files isn't available here").into()),
        },
    };
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// The Library lists a paused recording; Delete frames asks first, then deletes them.
    #[test]
    fn the_library_lists_recordings_and_deletes_frames_after_a_yes() {
        let dir = std::env::temp_dir().join(format!("photocraft-timelapse-ui-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute(tc::SETTINGS, json!({"folder": dir.to_string_lossy()})).unwrap();
        s.execute(tc::RECORD, json!({})).unwrap();
        assert_eq!(s.execute(tc::CAPTURE, json!({})).unwrap()["captured"], true);
        s.execute(tc::PAUSE, json!({})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(s, crate::Services::default());
            app.timelapse.open = true;
            app.timelapse.library_open = true;
            app
        });
        h.run_steps(4);
        assert!(h.query_by_label("NOT EXPORTED").is_some(), "tagged");
        assert!(h.query_by_label("Delete folder").is_none(), "it still has frames");
        h.get_by_label("Delete frames").click();
        h.run_steps(3);
        assert!(h.state().timelapse.confirm.is_some(), "asks first");
        assert_eq!(h.state().session.timelapse.track(h.state().session.active().unwrap().doc.id).unwrap().rec.meta.frames.len(), 1, "nothing deleted yet");
        // The confirmation's button comes first (above the list).
        let buttons = h.query_all_by_label("Delete frames").collect::<Vec<_>>();
        assert_eq!(buttons.len(), 2);
        buttons[0].click();
        h.run_steps(4);
        let st = h.state();
        assert!(st.timelapse.confirm.is_none());
        assert_eq!(st.ui.status, "Frames deleted.");
        assert!(st.session.timelapse.track(st.session.active().unwrap().doc.id).unwrap().rec.meta.frames.is_empty());
        assert!(h.query_by_label("Delete folder").is_some(), "now the recording itself can go");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
