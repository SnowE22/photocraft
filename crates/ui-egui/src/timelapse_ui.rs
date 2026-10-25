//! Window › Timelapse: the panel of File › Timelapse (`photocraft_engine::timelapse_cmds`), laid
//! out like the Photoshop Timelapse Recorder plug-in's panel it was ported from: Record, Library and
//! Settings tabs, cards with small upper-case headings, the big record button (green while
//! recording), stat tiles, the reference-layer chip with Normal / Reference, and an Export settings
//! view. It is a dock panel of its own (`dock::Group::Timelapse`), so it docks, floats and resizes
//! like any other.
//!
//! Every action runs a `file.timelapse.*` command, so the menus, the control channel and the panel
//! drive the same thing; the panel only shows the state. Each frame, [`frame`] lets the recorder
//! follow edits and capture (`Session::timelapse_tick`) and asks for a repaint when the next frame
//! is due, so capturing works without input and with the panel closed.

use std::sync::Arc;

use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use photocraft_doc::DocId;
use photocraft_engine::jobs::{JobEvent, JobOutcome};
use photocraft_engine::timelapse_cmds::{self as tc, ExportSettings, Frame, Settings};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The Window menu item.
pub const WINDOW_ID: &str = "window.toggle.timelapse";

/// The panel's tabs, and the Export settings view opened from Record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Record,
    Library,
    Settings,
    Export,
}

/// Panel state (not saved).
#[derive(Default)]
pub struct TimelapseUi {
    pub view: View,
    /// The export box under the record button is open.
    export_box: bool,
    /// The newest frame shows under the stats.
    preview: bool,
    /// The preview texture: document, preview revision, image.
    texture: Option<(DocId, u64, egui::TextureHandle)>,
    /// What the last export said, and the file it made (Play video / Show in folder).
    result: Option<(String, Option<String>)>,
    /// The export results are folded away (the fold button beside Export).
    result_hidden: bool,
    /// `file.timelapse.list` and when it was read (`egui` time): the Library re-reads it every
    /// few seconds while it shows, and after each change.
    library: Option<(f64, Value)>,
    /// A Library deletion waiting for the user's yes.
    confirm: Option<Confirm>,
    /// The intro and hold lengths last chosen, kept while the option is off (0 means off in the
    /// engine's settings).
    intro_sec: Option<f64>,
    hold_sec: Option<f64>,
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

/// The Library re-reads the recordings folder this often while it shows (seconds).
const LIBRARY_REFRESH: f64 = 3.0;
/// Corner radius of cards and big buttons (the plug-in's `--r`).
const R: f32 = 8.0;

/// Show the panel (docked where it was, or in the dock).
fn show_panel(app: &mut PhotocraftApp) {
    if crate::panel_docking::command(app, "window.panel.timelapse", &json!({"show": true})).is_none() {
        crate::dock::reveal(app, crate::dock::Group::Timelapse);
    }
}

/// File › Timelapse › Export Timelapse… without a path opens the panel's export box; Start
/// Recording asks for the recordings folder the first time.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    match id {
        tc::EXPORT if params.get("path").is_none() => {
            show_panel(app);
            app.timelapse.view = View::Record;
            app.timelapse.export_box = true;
            Some(Ok(json!({"open": true})))
        }
        tc::RECORD if app.session.timelapse_settings().folder.is_empty() => {
            show_panel(app);
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
        app.timelapse.result = None;
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
    let (message, file) = match &e.outcome {
        JobOutcome::Done(v) => {
            let file = v.get("file").and_then(Value::as_str).unwrap_or_default().to_string();
            let secs = v.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
            let name = std::path::Path::new(&file).file_name().map_or_else(|| file.clone(), |n| n.to_string_lossy().into_owned());
            let mut m = crate::i18n::fmt(tl!("Saved {name} ({seconds} video)."), &[("name", &name), ("seconds", &seconds_text(secs))]);
            if let Some(w) = v.get("warning").and_then(Value::as_str) {
                m = format!("{m} {w}");
            }
            (m, Some(file))
        }
        JobOutcome::Failed(err) => (crate::i18n::fmt(tl!("Export failed: {error}"), &[("error", err)]), None),
        JobOutcome::Cancelled => (tl!("Export cancelled.").to_string(), None),
    };
    app.ui.status.clone_from(&message);
    app.ui.status_error = matches!(e.outcome, JobOutcome::Failed(_));
    app.timelapse.result = Some((message, file));
    app.timelapse.result_hidden = false;
    app.timelapse.export_box = false;
    true
}

/// Once per frame: follow edits and capture, pass on messages.
pub fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let input = ctx.input(|i| i.pointer.any_down() || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. } | egui::Event::Text(_))));
    if let Some(ms) = app.session.timelapse_tick(input) {
        ctx.request_repaint_after(std::time::Duration::from_millis(ms.clamp(16.0, 60_000.0) as u64));
    }
    for m in app.session.timelapse_messages() {
        app.ui.status = m;
        app.ui.status_error = false;
    }
}

// ---- formatting (the plug-in's) ----------------------------------------------------------------

/// "1,234".
fn int_text(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Work time as a clock, "1:05:09".
fn clock_text(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// "820 KB", "4.5 MB", "1.20 GB".
fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1_048_576.0 {
        format!("{} KB", (b / 1024.0).round())
    } else if b < 1_073_741_824.0 {
        let mb = b / 1_048_576.0;
        if mb < 10.0 { format!("{mb:.1} MB") } else { format!("{} MB", mb.round()) }
    } else {
        format!("{:.2} GB", b / 1_073_741_824.0)
    }
}

/// "45 s", "2 min 05 s".
fn seconds_text(s: f64) -> String {
    let s = s.max(0.0).round() as u64;
    if s < 60 { format!("{s} s") } else { format!("{} min {:02} s", s / 60, s % 60) }
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago".
fn ago_text(t: u64) -> String {
    let now = tc::recorder::unix_ms();
    if t == 0 || now < t {
        return tl!("just now").into();
    }
    let m = (now - t) / 60_000;
    let n = |s: &str, v: u64| crate::i18n::fmt(s, &[("n", &v.to_string())]);
    if m < 1 {
        tl!("just now").into()
    } else if m < 60 {
        n(tl!("{n} min ago"), m)
    } else if m < 60 * 24 {
        n(tl!("{n} h ago"), m / 60)
    } else if m < 60 * 48 {
        tl!("yesterday").into()
    } else {
        n(tl!("{n} days ago"), m / 60 / 24)
    }
}

fn frames_text(n: u64) -> String {
    crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "{n} frame", "{n} frames"), &[("n", &int_text(n))])
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

// ---- options (the plug-in's lists) -------------------------------------------------------------

const LENGTHS: [(Option<u32>, &str); 11] = [
    (None, "Auto"),
    (Some(10), "10 seconds"),
    (Some(15), "15 seconds"),
    (Some(20), "20 seconds"),
    (Some(30), "30 seconds"),
    (Some(45), "45 seconds"),
    (Some(60), "1 min"),
    (Some(90), "1.5 min"),
    (Some(120), "2 min"),
    (Some(180), "3 min"),
    (Some(300), "5 min"),
];
/// "Show finished drawing first" lengths (tenths of a second, so they compare exactly).
const INTROS: [(u32, &str); 6] = [(5, "0.5 s"), (10, "1 s"), (15, "1.5 s"), (20, "2 s"), (30, "3 s"), (50, "5 s")];
const HOLDS: [(u32, &str); 7] = [(10, "1 s"), (20, "2 s"), (30, "3 s"), (40, "4 s"), (50, "5 s"), (80, "8 s"), (100, "10 s")];
const IDLES: [(u32, &str); 7] = [(5, "5 seconds"), (10, "10 seconds"), (15, "15 seconds"), (20, "20 seconds"), (30, "30 seconds"), (45, "45 seconds"), (60, "1 minute")];

fn quality_desc(q: &str) -> &'static str {
    match q {
        "low" => tl!("Smaller files and the quickest capturing. Fine for quick sketches and previews."),
        "high" => tl!("Sharper video for big screens. Uses about twice the disk space."),
        _ => tl!("Full-HD video, good for social media. Recommended."),
    }
}

// ---- drawing helpers (the plug-in's CSS, in theme colours) -------------------------------------

/// A card: the plug-in's `.card` (`dark` is its recessed `.card.dark` / `.card.inset`).
fn card<R>(ui: &mut Ui, t: &Tokens, dark: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = egui::Frame::new()
        .fill(if dark { t.field } else { t.card })
        .stroke(Stroke::new(1.0, if dark { t.field_border } else { t.card_border }))
        .corner_radius(R)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui)
        })
        .inner;
    // Cards stack 10 pt apart; a recessed card sits inside another, which spaces it already.
    if !dark {
        ui.add_space(4.0);
    }
    r
}

/// A card's heading: small, bold, upper case, faint.
fn heading(t: &Tokens, text: &str) -> RichText {
    RichText::new(text.to_uppercase()).size(9.5).strong().color(t.text_faint)
}

fn desc(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(text).size(10.0).color(t.text_dim));
}

/// Paint a centred single-line text.
fn centred_text(ui: &Ui, rect: Rect, text: &str, size: f32, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(size), color);
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, color);
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// The plug-in's `.btn`.
    Plain,
    /// `.btn.primary`.
    Primary,
    /// `.btn.live`: recording.
    Live,
    /// `.btn.paused`.
    Paused,
    /// `.btn.cancel`.
    Cancel,
    /// `.btn.danger`.
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    None,
    /// A red dot: Start recording.
    Rec,
    /// A white dot, pulsing: Recording.
    Live,
    /// Two bars: Paused.
    Pause,
    /// A triangle pointing up / down (fold the export results away / show them).
    Up,
    Down,
}

/// A button the width given (or the available width), the plug-in's `.btn` variants.
#[allow(clippy::too_many_arguments)]
fn button(ui: &mut Ui, t: &Tokens, text: &str, look: Look, icon: Icon, width: Option<f32>, height: f32, enabled: bool) -> egui::Response {
    let w = width.unwrap_or_else(|| ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(vec2(w, height), if enabled { Sense::click() } else { Sense::hover() });
    let hovered = enabled && resp.hovered();
    let (fill, fg) = match look {
        Look::Plain => (if hovered { t.hover } else { t.field }, t.text),
        Look::Primary => (t.primary_bg, t.primary_text),
        Look::Live => (t.success, Color32::WHITE),
        Look::Paused => (t.text_faint, t.primary_text),
        Look::Cancel => (t.danger, Color32::WHITE),
        Look::Danger => (t.danger.gamma_multiply(0.25), t.danger),
    };
    let fill = if hovered && look != Look::Plain { fill.gamma_multiply(1.12) } else { fill };
    let alpha = if enabled { 1.0 } else { 0.4 };
    let radius = if height >= 30.0 { R } else { 7.0 };
    ui.painter().rect_filled(rect, radius, fill.gamma_multiply(alpha));
    if look == Look::Plain {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.field_border.gamma_multiply(alpha)), StrokeKind::Inside);
    }
    let size = if height >= 30.0 { 12.0 } else { 11.0 };
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(size), fg);
    let icon_w = match icon {
        Icon::None => 0.0,
        Icon::Up | Icon::Down if text.is_empty() => 10.0,
        _ => 17.0,
    };
    let total = galley.size().x + icon_w;
    let left = rect.center().x - total / 2.0;
    let cy = rect.center().y;
    match icon {
        Icon::None => {}
        Icon::Rec => {
            ui.painter().circle_filled(egui::pos2(left + 5.0, cy), 5.0, t.danger.gamma_multiply(alpha));
        }
        Icon::Live => {
            // The plug-in's white dot dims and brightens while recording.
            let phase = (ui.input(|i| i.time) / 0.6).floor() as i64 % 2 == 0;
            let c = if phase { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.35) };
            ui.painter().circle_filled(egui::pos2(left + 5.0, cy), 5.0, c);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(600));
        }
        Icon::Pause => {
            for dx in [1.5, 7.5] {
                ui.painter().rect_filled(Rect::from_min_size(egui::pos2(left + dx, cy - 5.0), vec2(3.0, 10.0)), 0.0, fg.gamma_multiply(alpha));
            }
        }
        Icon::Up => triangle(ui, egui::pos2(left + 5.0, cy), 'u', fg.gamma_multiply(alpha)),
        Icon::Down => triangle(ui, egui::pos2(left + 5.0, cy), 'd', fg.gamma_multiply(alpha)),
    }
    ui.painter().galley(egui::pos2(left + icon_w, cy - galley.size().y / 2.0), galley, fg.gamma_multiply(alpha));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The plug-in's segmented control (`.tabs`, `.seg`, `.qsel`): one option per cell, the chosen
/// one raised (or filled with `on_fill`). `dot` puts a status dot before the first label. Returns
/// the clicked cell.
#[allow(clippy::too_many_arguments)]
fn segmented(ui: &mut Ui, t: &Tokens, id: &str, items: &[&str], selected: Option<usize>, on_fill: Option<Color32>, dot: Option<Color32>, enabled: bool) -> Option<usize> {
    let h = 28.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 9.0, t.field);
    p.rect_stroke(rect, 9.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    let inner = rect.shrink(3.0);
    let n = items.len().max(1) as f32;
    let cell_w = (inner.width() - 2.0 * (n - 1.0)) / n;
    let mut clicked = None;
    for (i, label) in items.iter().enumerate() {
        let cell = Rect::from_min_size(egui::pos2(inner.left() + i as f32 * (cell_w + 2.0), inner.top()), vec2(cell_w, inner.height()));
        let resp = ui.interact(cell, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        let on = selected == Some(i);
        if on {
            ui.painter().rect_filled(cell, 6.0, on_fill.unwrap_or(t.hover));
        } else if enabled && resp.hovered() {
            ui.painter().rect_filled(cell, 6.0, t.hover.gamma_multiply(0.35));
        }
        let color = if !enabled {
            t.text_faint
        } else if on && on_fill.is_some() {
            Color32::WHITE
        } else if on || resp.hovered() {
            t.text
        } else {
            t.text_dim
        };
        let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), color);
        let dot_w = if i == 0 && dot.is_some() { 12.0 } else { 0.0 };
        let left = cell.center().x - (galley.size().x + dot_w) / 2.0;
        if let (0, Some(c)) = (i, dot) {
            ui.painter().circle_filled(egui::pos2(left + 3.0, cell.center().y), 3.0, c);
        }
        ui.painter().galley(egui::pos2(left + dot_w, cell.center().y - galley.size().y / 2.0), galley, color);
        if resp.clicked() {
            clicked = Some(i);
        }
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, *label));
        let _ = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    clicked
}

/// The plug-in's pill toggle (`.chk`): a small switch and a label in a rounded outline, filled when
/// on. Returns true when clicked.
fn pill(ui: &mut Ui, t: &Tokens, on: bool, label: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), t.text);
    let w = (galley.size().x + 5.0 + 22.0 + 7.0 + 11.0).min(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
    let hovered = resp.hovered();
    let p = ui.painter();
    if on {
        p.rect_filled(rect, 11.0, if hovered { t.accent.gamma_multiply(1.12) } else { t.accent });
    } else {
        p.rect_stroke(rect, 11.0, Stroke::new(1.0, if hovered { t.text_faint } else { t.field_border }), StrokeKind::Inside);
    }
    let sw = Rect::from_min_size(egui::pos2(rect.left() + 5.0, rect.center().y - 6.0), vec2(22.0, 12.0));
    if on {
        p.rect_filled(sw, 6.0, Color32::WHITE);
        p.circle_filled(egui::pos2(sw.right() - 5.0, sw.center().y), 3.0, t.accent);
    } else {
        p.rect_stroke(sw, 6.0, Stroke::new(1.0, t.text_faint), StrokeKind::Inside);
        p.circle_filled(egui::pos2(sw.left() + 5.0, sw.center().y), 3.0, t.text_faint);
    }
    let color = if on { Color32::WHITE } else if hovered { t.text } else { t.text_dim };
    let text_rect = Rect::from_min_max(egui::pos2(sw.right() + 7.0, rect.top()), egui::pos2(rect.right() - 8.0, rect.bottom()));
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), color);
    ui.painter().with_clip_rect(text_rect).galley(egui::pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, label));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The plug-in's settings switch (`.switch`) at the right of a label row.
fn switch_row(ui: &mut Ui, t: &Tokens, on: bool, label: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.label(RichText::new(label).size(11.0).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 14.0), Sense::click());
            let p = ui.painter();
            p.rect_filled(rect, 7.0, if on { t.accent } else { t.field_border });
            let x = if on { rect.right() - 7.0 } else { rect.left() + 7.0 };
            p.circle_filled(egui::pos2(x, rect.center().y), 5.0, if on { Color32::WHITE } else { t.text_dim });
            clicked = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        });
    });
    clicked
}

/// A stat tile: a small upper-case key over a value.
fn stat(ui: &mut Ui, t: &Tokens, width: f32, key: &str, value: &str, idle: bool, mono: bool) {
    egui::Frame::new().fill(t.field).stroke(Stroke::new(1.0, t.field_border)).corner_radius(7.0).inner_margin(egui::Margin::symmetric(8, 7)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width((width - 18.0).max(10.0));
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.add(egui::Label::new(RichText::new(key.to_uppercase()).size(9.0).strong().color(t.text_faint)).truncate());
                if idle {
                    ui.label(RichText::new(tl!("idle")).size(9.0).color(t.warning));
                }
            });
            let v = RichText::new(value).size(if mono { 13.0 } else { 14.0 }).strong().color(t.text);
            ui.add(egui::Label::new(if mono { v.monospace() } else { v }).truncate());
        });
    });
}

/// A small coloured tag (REC, EXPORTED, NOT EXPORTED).
fn tag(ui: &mut Ui, text: &str, fill: Color32, fg: Color32) {
    egui::Frame::new().fill(fill).corner_radius(7.0).inner_margin(egui::Margin::symmetric(7, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(9.0).strong().color(fg));
    });
}

/// A recessed one-line path box (`.path`).
fn path_box(ui: &mut Ui, t: &Tokens, path: &str, empty: &str) {
    egui::Frame::new().fill(t.field).stroke(Stroke::new(1.0, t.field_border)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if path.is_empty() {
            ui.label(RichText::new(empty).size(10.0).color(t.text_dim));
        } else {
            ui.add(egui::Label::new(RichText::new(path).size(10.0).monospace().color(t.text)).wrap());
        }
    });
}

/// A one-line label `width` wide, left-aligned and cut off with "…" when too long.
fn left_label(ui: &mut Ui, width: f32, height: f32, text: RichText) -> egui::Response {
    ui.allocate_ui_with_layout(vec2(width.max(10.0), height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(width.max(10.0));
        ui.add(egui::Label::new(text).truncate())
    })
    .inner
}

/// A small filled triangle pointing right (closed) or down (open), or up, centred on `c`.
fn triangle(ui: &Ui, c: egui::Pos2, dir: char, color: Color32) {
    let r = 3.5;
    let pts = match dir {
        'r' => vec![egui::pos2(c.x - r * 0.6, c.y - r), egui::pos2(c.x + r * 0.8, c.y), egui::pos2(c.x - r * 0.6, c.y + r)],
        'u' => vec![egui::pos2(c.x - r, c.y + r * 0.6), egui::pos2(c.x + r, c.y + r * 0.6), egui::pos2(c.x, c.y - r * 0.8)],
        _ => vec![egui::pos2(c.x - r, c.y - r * 0.6), egui::pos2(c.x + r, c.y - r * 0.6), egui::pos2(c.x, c.y + r * 0.8)],
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// A text link followed by a triangle (the plug-in's "Preview ▸").
fn arrow_link(ui: &mut Ui, t: &Tokens, text: &str, open: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(10.5), t.text_dim);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 16.0), Sense::click());
    let color = if resp.hovered() { t.text } else { t.text_dim };
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0), galley, color);
    triangle(ui, egui::pos2(rect.right() - 4.0, rect.center().y), if open { 'd' } else { 'r' }, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled dropdown filling the width.
fn labelled_dropdown<V: PartialEq + Clone>(ui: &mut Ui, t: &Tokens, id: &str, label: &str, value: &mut V, options: &[(V, &str)]) -> bool {
    ui.label(RichText::new(label).size(10.0).color(t.text_dim));
    crate::widgets::dropdown(ui, id, value, options, ui.available_width())
}

// ---- the panel ---------------------------------------------------------------------------------

/// What the panel asks for once it's drawn.
enum Ask {
    Run(&'static str, Value),
    Settings(Value),
    ChooseFolder,
    ChooseFfmpeg,
    Export(ExportSettings),
    Open(String),
    CancelJob(photocraft_engine::jobs::JobId),
    /// Run a Library deletion the user confirmed.
    Confirm(Confirm),
    RefreshLibrary,
}

/// What the Record view shows about the active document's recording.
struct RecordState {
    doc: Option<String>,
    rec: bool,
    recording: bool,
    idle: bool,
    frames: u64,
    work: u64,
    bytes: u64,
    exported: bool,
    /// The video an export would make: (seconds, frames, width, height, size changed).
    estimate: Option<(f64, usize, u32, u32, bool)>,
    /// Selected layers, how many are reference layers, the name of a single one.
    selected: usize,
    refs: usize,
    layer_name: String,
    /// The running export: id, progress and message.
    job: Option<(photocraft_engine::jobs::JobId, f32, String)>,
}

fn estimate(app: &PhotocraftApp, settings: &Settings, o: &ExportSettings) -> Option<(f64, usize, u32, u32, bool)> {
    let st = app.session.active()?;
    let t = app.session.timelapse.track(st.doc.id)?;
    let frames: Vec<_> = t.rec.meta.usable(settings.hide_undone).collect();
    let last = frames.last()?;
    let p = tc::plan::plan_export(
        frames.len(),
        &tc::plan::PlanOptions { fps: 30.0, length_sec: o.length_sec, intro_sec: o.intro_sec, flash: o.flash, hold_sec: o.hold_sec, slow_end: o.slow_end },
    );
    let even = |v: u32| (v / 2 * 2).max(2);
    let resized = frames.iter().any(|f| f.x != last.x || f.y != last.y);
    Some((p.seconds(), frames.len(), even(last.x), even(last.y), resized))
}

/// The panel (Window › Timelapse), drawn in its dock group or floating window.
pub fn panel(app: &mut PhotocraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let settings = app.session.timelapse_settings();
    let status = app.session.timelapse_status();
    let rec = status.get("recording").filter(|v| !v.is_null()).cloned();
    let num = |k: &str| rec.as_ref().and_then(|r| r.get(k)).and_then(Value::as_u64).unwrap_or(0);
    let flag = |k: &str| rec.as_ref().and_then(|r| r.get(k)).and_then(Value::as_bool).unwrap_or(false);
    let (selected, refs, layer_name) = app
        .session
        .active()
        .map(|st| {
            let tag = settings.reference_tag.to_lowercase();
            let ids = st.selected_layers();
            let refs = ids.iter().filter(|id| st.doc.layer(**id).is_some_and(|l| l.name.to_lowercase().contains(&tag))).count();
            let name = match ids.as_slice() {
                [one] => st.doc.layer(*one).map(|l| tc::without_tag(&l.name, &settings.reference_tag)).unwrap_or_default(),
                _ => String::new(),
            };
            (ids.len(), refs, name)
        })
        .unwrap_or((0, 0, String::new()));
    let job = app.session.jobs().into_iter().find(|j| j.command == tc::EXPORT && j.state == "running").map(|j| (j.id, j.progress, j.message));
    let state = RecordState {
        doc: app.session.active().map(|d| d.doc.name.clone()),
        rec: rec.is_some(),
        recording: flag("recording"),
        idle: flag("idle"),
        frames: num("frames"),
        work: num("workMs"),
        bytes: num("bytes"),
        exported: num("exports") > 0,
        estimate: estimate(app, &settings, &settings.export),
        selected,
        refs,
        layer_name,
        job,
    };
    let tex = match (app.timelapse.preview, app.session.active().map(|d| d.doc.id)) {
        (true, Some(id)) => preview(app, &ctx, id),
        _ => None,
    };
    let now = ctx.input(|i| i.time);
    if app.timelapse.view == View::Library && app.timelapse.library.as_ref().is_none_or(|(at, _)| now - at >= LIBRARY_REFRESH) {
        let v = app.session.execute(tc::LIST, json!({})).unwrap_or(Value::Null);
        app.timelapse.library = Some((now, v));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(LIBRARY_REFRESH));
    }
    let library = app.timelapse.library.as_ref().map(|(_, v)| v.clone()).unwrap_or(Value::Null);

    let mut ask: Option<Ask> = None;
    let mut s = std::mem::take(&mut app.timelapse);
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);

    // The tabs stay at the top; everything else scrolls under them.
    let tabs = [tl!("Record"), tl!("Library"), tl!("Settings")];
    let tab = match s.view {
        View::Record | View::Export => 0,
        View::Library => 1,
        View::Settings => 2,
    };
    let dot = if state.recording { t.success } else { t.warning };
    if let Some(i) = segmented(ui, &t, "timelapse-tabs", &tabs, Some(tab), None, Some(dot), true) {
        s.view = [View::Record, View::Library, View::Settings][i];
        if s.view == View::Library {
            s.library = None;
        }
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("timelapse-panel").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match s.view {
            View::Record => record_view(ui, &t, &state, &settings, &mut s, tex.as_ref(), &mut ask),
            View::Export => export_view(ui, &t, &state, &settings, &mut s, &mut ask),
            View::Library => library_view(ui, &t, &library, &mut s, &mut ask),
            View::Settings => settings_view(ui, &t, &settings, &mut ask),
        }
    });
    confirm_modal(&ctx, &t, &mut s, &mut ask);
    app.timelapse = s;
    apply(app, ask);
}

fn record_view(ui: &mut Ui, t: &Tokens, st: &RecordState, settings: &Settings, s: &mut TimelapseUi, tex: Option<&egui::TextureHandle>, ask: &mut Option<Ask>) {
    // Recording: the record button, the export box and the export results.
    card(ui, t, false, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(22.0);
            ui.label(heading(t, tl!("Recording")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill(ui, t, settings.auto_record, tl!("Auto record")) {
                    *ask = Some(Ask::Settings(json!({"autoRecord": !settings.auto_record})));
                }
            });
        });
        let (label, look, icon, tip) = if st.recording {
            (tl!("Recording"), Look::Live, Icon::Live, tl!("Click to pause"))
        } else if st.rec {
            (tl!("Paused"), Look::Paused, Icon::Pause, tl!("Click to resume recording"))
        } else {
            (tl!("Start recording"), Look::Plain, Icon::Rec, "")
        };
        let b = button(ui, t, label, look, icon, None, 40.0, st.doc.is_some());
        let b = if tip.is_empty() { b } else { b.on_hover_text(tip) };
        if b.clicked() {
            *ask = Some(Ask::Run(if st.recording { tc::PAUSE } else { tc::RECORD }, json!({})));
        }
        if st.doc.is_none() {
            desc(ui, t, tl!("Open a document to record it."));
        } else if !st.rec && settings.folder.is_empty() {
            desc(ui, t, tl!("You'll be asked where to save recordings."));
        }
        // Export video / Export again / Cancel (closes the box).
        let can_export = st.frames > 0 && st.job.is_none();
        if !can_export {
            s.export_box = false;
        }
        ui.horizontal(|ui| {
            let show_arrow = s.result.is_some() && !s.export_box && st.job.is_none();
            let w = ui.available_width() - if show_arrow { 48.0 } else { 0.0 };
            let (label, look) = if s.export_box {
                (tl!("Cancel"), Look::Cancel)
            } else if st.exported {
                (tl!("Export again"), Look::Primary)
            } else {
                (tl!("Export video"), Look::Primary)
            };
            if button(ui, t, label, look, Icon::None, Some(w), 30.0, can_export).clicked() {
                s.export_box = !s.export_box;
            }
            if show_arrow {
                let (icon, tip) = if s.result_hidden { (Icon::Down, tl!("Show the export results")) } else { (Icon::Up, tl!("Hide the export results")) };
                if button(ui, t, "", Look::Plain, icon, Some(42.0), 30.0, true).on_hover_text(tip).clicked() {
                    s.result_hidden = !s.result_hidden;
                }
            }
        });
        if s.export_box {
            card(ui, t, true, |ui| {
                let mut o = settings.export.clone();
                let mut len = o.length_sec.map(|v| v.round() as u32);
                if labelled_dropdown(ui, t, "timelapse-length-quick", tl!("Length"), &mut len, &LENGTHS) {
                    o.length_sec = len.map(f64::from);
                    *ask = Some(Ask::Settings(json!({"export": {"lengthSec": len.map_or(json!("auto"), |v| json!(v))}})));
                }
                if let Some((secs, frames, ..)) = st.estimate {
                    let line = crate::i18n::fmt(tl!("≈ {seconds} video from {frames}."), &[("seconds", &seconds_text(secs)), ("frames", &frames_text(frames as u64))]);
                    desc(ui, t, &line);
                }
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 6.0) / 2.0;
                    if button(ui, t, tl!("Advanced settings"), Look::Plain, Icon::None, Some(w), 28.0, true).clicked() {
                        s.view = View::Export;
                    }
                    if button(ui, t, tl!("Confirm"), Look::Primary, Icon::None, Some(w), 28.0, st.estimate.is_some()).clicked() {
                        *ask = Some(Ask::Export(o.clone()));
                    }
                });
            });
        }
        // A running export: progress and Cancel export.
        if let Some((id, progress, message)) = &st.job {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
            ui.painter().rect_filled(rect, 3.0, t.field);
            let mut bar = rect;
            bar.set_width(rect.width() * progress.clamp(0.0, 1.0));
            ui.painter().rect_filled(bar, 3.0, t.accent);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            desc(ui, t, message);
            if button(ui, t, tl!("Cancel export"), Look::Plain, Icon::None, None, 28.0, true).clicked() {
                *ask = Some(Ask::CancelJob(*id));
            }
        } else if let Some((message, file)) = s.result.clone().filter(|_| !s.result_hidden) {
            desc(ui, t, &message);
            if let Some(file) = file {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 6.0) / 2.0;
                    if button(ui, t, tl!("Play video"), Look::Plain, Icon::None, Some(w), 28.0, true).clicked() {
                        *ask = Some(Ask::Open(file.clone()));
                    }
                    if button(ui, t, tl!("Show in folder"), Look::Plain, Icon::None, Some(w), 28.0, true).clicked() {
                        let dir = std::path::Path::new(&file).parent().map_or_else(|| file.clone(), |d| d.to_string_lossy().into_owned());
                        *ask = Some(Ask::Open(dir));
                    }
                });
            }
        }
    });

    // Stats, the document and the preview.
    card(ui, t, false, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let w = (ui.available_width() - 12.0) / 3.3;
            let dash = "–".to_string();
            stat(ui, t, w, tl!("Frames"), &if st.rec { int_text(st.frames) } else { dash.clone() }, false, false);
            stat(ui, t, w * 1.3, tl!("Timer"), &if st.rec { clock_text(st.work) } else { dash.clone() }, st.idle, true);
            stat(ui, t, w, tl!("On disk"), &if st.rec { size_text(st.bytes) } else { dash }, false, false);
        });
        egui::Frame::new().fill(t.field).stroke(Stroke::new(1.0, t.field_border)).corner_radius(7.0).inner_margin(egui::Margin::symmetric(9, 7)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let name = st.doc.clone().unwrap_or_else(|| tl!("No document open").to_string());
                let link_w = if st.doc.is_some() { 72.0 } else { 0.0 };
                left_label(ui, ui.available_width() - link_w, 16.0, RichText::new(name).size(11.0).strong().color(t.text));
                if st.doc.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let tip = if s.preview { tl!("Hide the live preview") } else { tl!("Show the newest frame") };
                        if arrow_link(ui, t, tl!("Preview"), s.preview).on_hover_text(tip).clicked() {
                            s.preview = !s.preview;
                        }
                    });
                }
            });
        });
        if s.preview && st.doc.is_some() {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 160.0), Sense::hover());
            ui.painter().rect_filled(rect, 7.0, t.canvas);
            ui.painter().rect_stroke(rect, 7.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            match tex {
                Some(tex) => {
                    let size = tex.size_vec2();
                    let k = (rect.width() / size.x).min((rect.height() - 2.0) / size.y).min(1.0);
                    let img = Rect::from_center_size(rect.center(), size * k);
                    ui.painter().image(tex.id(), img, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    let text = if st.recording { tl!("No frames yet — just start drawing") } else { tl!("No frames yet") };
                    centred_text(ui, rect, text, 11.0, t.text_dim);
                }
            }
        }
    });

    // Reference layer: the selected layer, and Normal / Reference.
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Reference layer")));
        let is_ref = st.refs > 0;
        egui::Frame::new()
            .fill(t.field)
            .stroke(Stroke::new(1.0, if is_ref { t.success.gamma_multiply(0.6) } else { t.field_border }))
            .corner_radius(7.0)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (thumb, _) = ui.allocate_exact_size(vec2(22.0, 17.0), Sense::hover());
                    if is_ref {
                        ui.painter().rect_filled(thumb, 3.0, t.success.gamma_multiply(0.25));
                        ui.painter().rect_stroke(thumb, 3.0, Stroke::new(1.0, t.success), StrokeKind::Inside);
                    } else {
                        ui.painter().rect_filled(thumb, 3.0, t.text_dim);
                        ui.painter().rect_stroke(thumb, 3.0, Stroke::new(1.0, t.text_faint), StrokeKind::Inside);
                    }
                    let (name, none) = if st.doc.is_none() {
                        (tl!("No document open").to_string(), true)
                    } else if st.selected == 0 {
                        (tl!("No layer selected").to_string(), true)
                    } else if st.selected > 1 {
                        (crate::i18n::fmt(tl!("{n} layers selected"), &[("n", &st.selected.to_string())]), false)
                    } else {
                        (st.layer_name.clone(), false)
                    };
                    let state = match (st.selected, st.refs) {
                        (0, _) => String::new(),
                        (n, r) if r == n => tl!("Not recorded").to_string(),
                        (n, r) if r > 0 => crate::i18n::fmt(tl!("{count} of {total} not recorded"), &[("count", &r.to_string()), ("total", &n.to_string())]),
                        _ => tl!("Recorded").to_string(),
                    };
                    let state_w = if state.is_empty() { 0.0 } else { 110.0 };
                    let name_text = RichText::new(name).size(11.0).color(if none { t.text_dim } else { t.text });
                    left_label(ui, ui.available_width() - state_w, 17.0, if none { name_text } else { name_text.strong() });
                    if !state.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let c = if is_ref { t.success } else { t.text_dim };
                            ui.label(RichText::new(state).size(10.0).color(c));
                        });
                    }
                });
            });
        let all_ref = st.selected > 0 && st.refs == st.selected;
        let sel = if all_ref {
            Some(1)
        } else if st.selected > 0 && st.refs == 0 {
            Some(0)
        } else {
            None
        };
        let fill = if all_ref { Some(t.success) } else { None };
        if let Some(i) = segmented(ui, t, "timelapse-ref", &[tl!("Normal"), tl!("Reference")], sel, fill, None, st.doc.is_some() && st.selected > 0) {
            *ask = Some(Ask::Run(tc::REFERENCE, json!({"reference": i == 1})));
        }
        desc(ui, t, tl!("A reference layer works like a normal layer, but won't show up in the timelapse."));
    });
}

fn export_view(ui: &mut Ui, t: &Tokens, st: &RecordState, settings: &Settings, s: &mut TimelapseUi, ask: &mut Option<Ask>) {
    let o = &settings.export;
    let set = |v: Value| Some(Ask::Settings(json!({ "export": v })));
    ui.horizontal(|ui| {
        let back = format!("‹ {}", tl!("Back"));
        let galley_w = ui.painter().layout_no_wrap(back.clone(), egui::FontId::proportional(11.0), t.text).size().x;
        if button(ui, t, &back, Look::Plain, Icon::None, Some(galley_w + 22.0), 22.0, true).clicked() {
            s.view = View::Record;
        }
        ui.label(RichText::new(tl!("Export settings")).size(12.0).strong().color(t.text));
    });
    ui.add_space(4.0);
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Video")));
        let mut len = o.length_sec.map(|v| v.round() as u32);
        if labelled_dropdown(ui, t, "timelapse-length", tl!("Length"), &mut len, &LENGTHS) {
            *ask = set(json!({"lengthSec": len.map_or(json!("auto"), |v| json!(v))}));
        }
        ui.columns(2, |cols| {
            let mut enc = o.encoder.clone();
            if labelled_dropdown(&mut cols[0], t, "timelapse-encoder", tl!("Make video with"), &mut enc, &[("ffmpeg".to_string(), "ffmpeg (fast)"), ("frames".to_string(), "JPEG frames")]) {
                *ask = set(json!({"encoder": enc}));
            }
            let mut bars = o.letterbox.clone();
            let colors = [("black".to_string(), "Black"), ("white".to_string(), "White"), ("gray".to_string(), "Dark grey")];
            if labelled_dropdown(&mut cols[1], t, "timelapse-bars", tl!("Bars colour"), &mut bars, &colors) {
                *ask = set(json!({"letterbox": bars}));
            }
        });
        desc(ui, t, tl!("Bars fill the empty space when the canvas size changed during recording."));
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Extras")));
        // An option with its length beside it (the plug-in's `.optrow`).
        let row = |ui: &mut Ui, on: bool, label: &str, extra: &mut dyn FnMut(&mut Ui)| -> bool {
            let mut clicked = false;
            ui.horizontal(|ui| {
                ui.set_min_height(26.0);
                let w = ui.available_width() - 96.0;
                ui.allocate_ui(vec2(w, 24.0), |ui| clicked = pill(ui, t, on, label));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| extra(ui));
            });
            clicked
        };
        let intro_on = o.intro_sec > 0.0;
        let mut intro = ((if intro_on { o.intro_sec } else { s.intro_sec.unwrap_or(1.0) }) * 10.0).round() as u32;
        let mut intro_changed = false;
        if row(ui, intro_on, tl!("Show finished drawing first"), &mut |ui| {
            ui.add_enabled_ui(intro_on, |ui| intro_changed = crate::widgets::dropdown(ui, "timelapse-intro", &mut intro, &INTROS, 88.0));
        }) {
            *ask = set(json!({"introSec": if intro_on { 0.0 } else { f64::from(intro) / 10.0 }}));
        }
        if intro_changed {
            s.intro_sec = Some(f64::from(intro) / 10.0);
            *ask = set(json!({"introSec": f64::from(intro) / 10.0}));
        }
        if pill(ui, t, o.flash, tl!("Flash before the final image")) {
            *ask = set(json!({"flash": !o.flash}));
        }
        let hold_on = o.hold_sec > 0.0;
        let mut hold = ((if hold_on { o.hold_sec } else { s.hold_sec.unwrap_or(3.0) }) * 10.0).round() as u32;
        let mut hold_changed = false;
        if row(ui, hold_on, tl!("Hold final image"), &mut |ui| {
            ui.add_enabled_ui(hold_on, |ui| hold_changed = crate::widgets::dropdown(ui, "timelapse-hold", &mut hold, &HOLDS, 88.0));
        }) {
            *ask = set(json!({"holdSec": if hold_on { 0.0 } else { f64::from(hold) / 10.0 }}));
        }
        if hold_changed {
            s.hold_sec = Some(f64::from(hold) / 10.0);
            *ask = set(json!({"holdSec": f64::from(hold) / 10.0}));
        }
        if pill(ui, t, o.slow_end, tl!("Slow down at the end")) {
            *ask = set(json!({"slowEnd": !o.slow_end}));
        }
        if pill(ui, t, o.deflicker, tl!("Remove flicker")) {
            *ask = set(json!({"deflicker": !o.deflicker}));
        }
        let mut corner = o.corner.clone();
        let mut corner_changed = false;
        if row(ui, o.timer, tl!("Show work-time counter"), &mut |ui| {
            let corners = [("br".to_string(), "Bottom right"), ("bl".to_string(), "Bottom left"), ("tr".to_string(), "Top right"), ("tl".to_string(), "Top left")];
            ui.add_enabled_ui(o.timer, |ui| corner_changed = crate::widgets::dropdown(ui, "timelapse-corner", &mut corner, &corners, 88.0));
        }) {
            *ask = set(json!({"timer": !o.timer}));
        }
        if corner_changed {
            *ask = set(json!({"corner": corner}));
        }
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Result")));
        match st.estimate {
            None => desc(ui, t, tl!("Record something in this document to see how long the video will be.")),
            Some((secs, frames, w, h, resized)) => {
                ui.label(RichText::new(crate::i18n::fmt(tl!("≈ {seconds} video"), &[("seconds", &seconds_text(secs))])).size(13.0).strong().color(t.text));
                desc(ui, t, &format!("{} · {w}×{h}", crate::i18n::fmt(tl!("{n} recorded frames"), &[("n", &int_text(frames as u64))])));
                if o.length_sec.is_none() {
                    desc(ui, t, tl!("Auto shows every frame once (at least 5 s, at most 1 min)."));
                }
                if resized {
                    desc(ui, t, tl!("The canvas size changed during recording; earlier frames are fitted inside the final size."));
                }
                if o.deflicker {
                    desc(ui, t, tl!("Remove flicker leaves out frames that flash in and out (a layer hidden for a moment, a stroke undone right away), so the video can be a bit shorter."));
                }
            }
        }
    });
}

fn library_view(ui: &mut Ui, t: &Tokens, library: &Value, s: &mut TimelapseUi, ask: &mut Option<Ask>) {
    let folder = library.get("folder").and_then(Value::as_str).unwrap_or_default().to_string();
    let recs = library.get("recordings").and_then(Value::as_array).cloned().unwrap_or_default();
    card(ui, t, true, |ui| {
        ui.label(heading(t, tl!("Recordings folder")));
        path_box(ui, t, &folder, tl!("Not chosen yet — see Settings"));
        ui.horizontal(|ui| {
            if !recs.is_empty() {
                let total: u64 = recs.iter().filter_map(|r| r.get("bytes").and_then(Value::as_u64)).sum();
                let count = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), recs.len() as u64, "{n} recording", "{n} recordings"), &[]);
                ui.label(RichText::new(size_text(total)).size(15.0).strong().color(t.text));
                ui.label(RichText::new(format!("· {count}")).size(10.0).color(t.text_dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let w = ui.painter().layout_no_wrap(tl!("Open folder").to_string(), egui::FontId::proportional(11.0), t.text).size().x + 22.0;
                if button(ui, t, tl!("Open folder"), Look::Plain, Icon::None, Some(w), 22.0, !folder.is_empty()).clicked() {
                    *ask = Some(Ask::Open(folder.clone()));
                }
            });
        });
    });
    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        let r = ui.add(egui::Label::new(RichText::new(tl!("Refresh")).size(11.0).strong().color(t.text_dim)).sense(Sense::click()));
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *ask = Some(Ask::RefreshLibrary);
        }
    });
    ui.add_space(4.0);
    if recs.is_empty() {
        let text = if folder.is_empty() { tl!("Recordings will be listed here once you've chosen a folder.") } else { tl!("No recordings yet.") };
        desc(ui, t, text);
        return;
    }
    for r in &recs {
        library_item(ui, t, r, s, ask);
    }
}

fn library_item(ui: &mut Ui, t: &Tokens, r: &Value, s: &mut TimelapseUi, ask: &mut Option<Ask>) {
    let text = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let num = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
    let flag = |k: &str| r.get(k).and_then(Value::as_bool).unwrap_or(false);
    let (id, name, dir) = (text("id"), text("name"), text("dir"));
    let (frames, all, bytes, work, since, recorded) = (num("frames"), num("allFrames"), num("bytes"), num("workMs"), num("framesSinceExport"), num("recorded"));
    let (recording, exported) = (flag("recording"), flag("exported"));
    card(ui, t, false, |ui| {
        ui.horizontal(|ui| {
            let tags_w = if exported || all > 0 { 110.0 } else { 0.0 } + if recording { 40.0 } else { 0.0 };
            left_label(ui, ui.available_width() - tags_w, 18.0, RichText::new(&name).size(12.0).strong().color(t.text)).on_hover_text(&dir);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if exported {
                    tag(ui, tl!("EXPORTED"), t.success.gamma_multiply(0.45), t.text);
                } else if all > 0 {
                    tag(ui, tl!("NOT EXPORTED"), t.warning.gamma_multiply(0.35), t.text);
                }
                if recording {
                    tag(ui, tl!("REC"), t.success, Color32::WHITE);
                }
            });
        });
        let meta = format!(
            "{} · {} · {} · {}",
            frames_text(frames),
            size_text(bytes),
            crate::i18n::fmt(tl!("{time} work"), &[("time", &clock_text(work))]),
            crate::i18n::fmt(tl!("recorded {when}"), &[("when", &ago_text(recorded))])
        );
        desc(ui, t, &meta);
        if exported && since > 0 && all > 0 {
            desc(ui, t, tl!("Recorded more since the last export."));
        }
        ui.horizontal(|ui| {
            let small = |ui: &Ui, label: &str| ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), t.text).size().x + 22.0;
            let delete = if all > 0 { tl!("Delete frames") } else { tl!("Delete folder") };
            let total = small(ui, tl!("Open folder")) + small(ui, delete) + 6.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            if button(ui, t, tl!("Open folder"), Look::Plain, Icon::None, Some(small(ui, tl!("Open folder"))), 22.0, true).clicked() {
                *ask = Some(Ask::Open(dir.clone()));
            }
            let b = button(ui, t, delete, Look::Danger, Icon::None, Some(small(ui, delete)), 22.0, !recording);
            let b = if recording { b.on_hover_text(tl!("Pause the recording first")) } else { b };
            if b.clicked() {
                let text = if all == 0 {
                    crate::i18n::fmt(
                        tl!("Delete “{name}” and its folder? It's removed from the library, along with its work time. Videos you exported stay where you saved them. This can't be undone."),
                        &[("name", &name)],
                    )
                } else {
                    let what = format!("{} ({})", frames_text(all), size_text(bytes));
                    if exported {
                        let mut m = crate::i18n::fmt(tl!("Delete the {frames} of “{name}”?"), &[("name", &name), ("frames", &what)]);
                        if since > 0 {
                            m = format!("{m} {}", tl!("You've recorded more since the last export, and that part isn't in the video."));
                        }
                        format!("{m} {}", tl!("Your videos and the work time are kept. This can't be undone."))
                    } else {
                        crate::i18n::fmt(
                            tl!("“{name}” hasn't been exported yet. If you delete its {frames}, you can't make a video of it anymore. This can't be undone."),
                            &[("name", &name), ("frames", &what)],
                        )
                    }
                };
                let command = if all > 0 { tc::DELETE_FRAMES } else { tc::DELETE_RECORDING };
                s.confirm = Some(Confirm { command, id: id.clone(), name: name.clone(), text, frames: all > 0 });
            }
        });
    });
}

/// The plug-in's confirmation box over the panel: Cancel / Delete.
fn confirm_modal(ctx: &egui::Context, t: &Tokens, s: &mut TimelapseUi, ask: &mut Option<Ask>) {
    let Some(c) = s.confirm.clone() else { return };
    let mut close = false;
    egui::Modal::new(egui::Id::new("timelapse-confirm")).show(ctx, |ui| {
        ui.set_max_width(320.0);
        ui.label(RichText::new(&c.text).size(11.0).color(t.text));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 6.0) / 2.0;
            if button(ui, t, tl!("Cancel"), Look::Plain, Icon::None, Some(w), 28.0, true).clicked() {
                close = true;
            }
            let label = if c.frames { tl!("Delete frames") } else { tl!("Delete folder") };
            if button(ui, t, label, Look::Danger, Icon::None, Some(w), 28.0, true).clicked() {
                *ask = Some(Ask::Confirm(c.clone()));
                close = true;
            }
        });
    });
    if close {
        s.confirm = None;
    }
}

fn settings_view(ui: &mut Ui, t: &Tokens, settings: &Settings, ask: &mut Option<Ask>) {
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Frames folder")));
        path_box(ui, t, &settings.folder, tl!("Not chosen yet"));
        desc(ui, t, tl!("Where the recorded frames are kept. Videos are saved wherever you choose when you export."));
        let label = if settings.folder.is_empty() { tl!("Choose folder…") } else { tl!("Change folder…") };
        let w = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(11.0), t.text).size().x + 22.0;
        if button(ui, t, label, Look::Plain, Icon::None, Some(w), 24.0, true).clicked() {
            *ask = Some(Ask::ChooseFolder);
        }
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Recording quality")));
        let qualities = ["low", "default", "high"];
        let sel = qualities.iter().position(|q| *q == settings.quality.as_str()).unwrap_or(1);
        if let Some(i) = segmented(ui, t, "timelapse-quality", &[tl!("Low"), tl!("Default"), tl!("High")], Some(sel), Some(t.accent), None, true) {
            *ask = Some(Ask::Settings(json!({"quality": qualities.get(i).copied().unwrap_or("default")})));
        }
        desc(ui, t, quality_desc(&settings.quality));
        if switch_row(ui, t, settings.hide_undone, tl!("Remove frames with undo")) {
            *ask = Some(Ask::Settings(json!({"hideUndone": !settings.hide_undone})));
        }
        desc(ui, t, tl!("Undoing something removes those recorded frames."));
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Reference layers")));
        let mut tag = settings.reference_tag.clone();
        let mut tags: Vec<String> = tc::REFERENCE_TAGS.iter().map(|s| s.to_string()).collect();
        if !tags.contains(&tag) {
            tags.insert(0, tag.clone());
        }
        let options: Vec<(String, &str)> = tags.iter().map(|s| (s.clone(), s.as_str())).collect();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Name tag")).size(11.0).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::dropdown_literal(ui, "timelapse-tag", &mut tag, &options, 118.0) {
                    *ask = Some(Ask::Settings(json!({"referenceTag": tag})));
                }
            });
        });
        desc(ui, t, tl!("Layers with this in their name are left out of the timelapse."));
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, tl!("Work timer")));
        let mut idle = settings.idle_sec;
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Idle after")).size(11.0).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::widgets::dropdown(ui, "timelapse-idle", &mut idle, &IDLES, 118.0) {
                    *ask = Some(Ask::Settings(json!({"idleSec": idle})));
                }
            });
        });
        desc(ui, t, tl!("The timer stops counting after this long without activity."));
    });
    card(ui, t, false, |ui| {
        ui.label(heading(t, "ffmpeg"));
        path_box(ui, t, &settings.ffmpeg, tl!("Automatic"));
        desc(ui, t, tl!("Makes the MP4. Automatic looks next to PhotoCraft, then on PATH."));
        ui.horizontal(|ui| {
            let w = ui.painter().layout_no_wrap(tl!("Choose…").to_string(), egui::FontId::proportional(11.0), t.text).size().x + 22.0;
            if button(ui, t, tl!("Choose…"), Look::Plain, Icon::None, Some(w), 24.0, true).clicked() {
                *ask = Some(Ask::ChooseFfmpeg);
            }
            if !settings.ffmpeg.is_empty() {
                let w = ui.painter().layout_no_wrap(tl!("Automatic").to_string(), egui::FontId::proportional(11.0), t.text).size().x + 22.0;
                if button(ui, t, tl!("Automatic"), Look::Plain, Icon::None, Some(w), 24.0, true).clicked() {
                    *ask = Some(Ask::Settings(json!({"ffmpeg": ""})));
                }
            }
        });
    });
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(tl!("Timelapse Recorder · PhotoCraft")).size(10.0).color(t.text_faint));
    });
}

fn apply(app: &mut PhotocraftApp, ask: Option<Ask>) {
    let r = match ask {
        None => return,
        Some(Ask::RefreshLibrary) => {
            app.timelapse.library = None;
            return;
        }
        Some(Ask::CancelJob(id)) => {
            app.session.cancel_job(id);
            return;
        }
        Some(Ask::Confirm(c)) => {
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

    fn harness(s: photocraft_engine::Session, view: View) -> egui_kittest::Harness<'static, PhotocraftApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(s, crate::Services::default());
            app.ui.panels.timelapse = true;
            app.timelapse.view = view;
            app
        })
    }

    fn recorded(dir: &std::path::Path) -> photocraft_engine::Session {
        let _ = std::fs::remove_dir_all(dir);
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute(tc::SETTINGS, json!({"folder": dir.to_string_lossy()})).unwrap();
        s.execute(tc::RECORD, json!({})).unwrap();
        assert_eq!(s.execute(tc::CAPTURE, json!({})).unwrap()["captured"], true);
        s
    }

    /// Window › Timelapse shows the panel in the dock, and the record button pauses.
    #[test]
    fn the_panel_docks_and_its_record_button_pauses() {
        let dir = std::env::temp_dir().join(format!("photocraft-timelapse-ui-dock-{}", std::process::id()));
        let mut h = harness(recorded(&dir), View::Record);
        h.run_steps(4);
        assert!(h.query_by_label("Recording").is_some(), "the green record button");
        assert!(h.query_by_label("Reference layer").is_some() || h.query_by_label("REFERENCE LAYER").is_some());
        h.get_by_label("Recording").click();
        h.run_steps(3);
        let st = h.state();
        assert!(!st.session.timelapse.track(st.session.active().unwrap().doc.id).unwrap().recording, "paused");
        assert!(h.query_by_label("Paused").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The Library lists a paused recording; Delete frames asks first, then deletes them.
    #[test]
    fn the_library_lists_recordings_and_deletes_frames_after_a_yes() {
        let dir = std::env::temp_dir().join(format!("photocraft-timelapse-ui-library-{}", std::process::id()));
        let mut s = recorded(&dir);
        s.execute(tc::PAUSE, json!({})).unwrap();
        let mut h = harness(s, View::Library);
        h.run_steps(4);
        assert!(h.query_by_label("NOT EXPORTED").is_some(), "tagged");
        assert!(h.query_by_label("Delete folder").is_none(), "it still has frames");
        h.get_by_label("Delete frames").click();
        h.run_steps(3);
        assert!(h.state().timelapse.confirm.is_some(), "asks first");
        assert_eq!(h.state().session.timelapse.track(h.state().session.active().unwrap().doc.id).unwrap().rec.meta.frames.len(), 1, "nothing deleted yet");
        // The list's button and the confirmation's.
        let buttons = h.query_all_by_label("Delete frames").collect::<Vec<_>>();
        assert_eq!(buttons.len(), 2);
        buttons[1].click();
        h.run_steps(4);
        let st = h.state();
        assert!(st.timelapse.confirm.is_none());
        assert_eq!(st.ui.status, "Frames deleted.");
        assert!(st.session.timelapse.track(st.session.active().unwrap().doc.id).unwrap().rec.meta.frames.is_empty());
        assert!(h.query_by_label("Delete folder").is_some(), "now the recording itself can go");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
