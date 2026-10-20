//! The shell's half of File › Automate › Topaz Gigapixel AI… (`photocraft_engine::gigapixel_cmds`).
//!
//! The command sends the flattened image to Gigapixel's app and returns where it put it. While
//! Gigapixel has it, a strip at the bottom of the window says so and offers Cancel, and twice a
//! second the folder is looked at. When the upscaled export appears (Gigapixel writes it next to
//! the image, as its default export settings do), it is applied (`topazGigapixel.apply`): the
//! document resized to it and the image on top as "Topaz Gigapixel layer", as Photoshop's
//! plug-in leaves it. Then the folder is removed. Editing goes on meanwhile.
//!
//! File › Automate › Topaz Gigapixel Presets lists the saved presets: a click runs one with
//! Gigapixel's command-line tool, as a background job, without opening Gigapixel. Manage
//! Presets… makes, edits and deletes them.

use photocraft_engine::gigapixel_cmds::presets::GigapixelPreset;
use photocraft_engine::gigapixel_cmds::{APPLY_ID, Pending};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::menus::MenuItem;
use crate::state::DialogKind;
use crate::widgets;

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
    egui::Area::new(egui::Id::new("gigapixel-waiting")).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0)).order(egui::Order::Foreground).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Waiting for Topaz Gigapixel AI: export the upscaled image there to bring it back."));
                    if ui.button(tl!("Cancel")).clicked() {
                        cancelled = true;
                    }
                });
            });
        },
    );
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
            // Gigapixel embedded the settings it used: they can become a preset.
            let message = if r["settings"].is_object() {
                tl!("Topaz Gigapixel AI: upscaled {scale}× to {w} × {h} px (new layer). Keep these settings with Topaz Gigapixel Presets › Save Preset from Last Upscale")
            } else {
                tl!("Topaz Gigapixel AI: upscaled {scale}× to {w} × {h} px (new layer)")
            };
            app.ui.status = crate::i18n::fmt(message, &[("scale", &scale), ("w", &wd), ("h", &ht)]);
        }
        Err(e) => {
            app.ui.status_error = true;
            app.ui.status = crate::i18n::fmt(tl!("Topaz Gigapixel AI's image could not be applied: {error}"), &[("error", &e)]);
        }
    }
    w.pending.clean_up();
}

// File › Automate › Topaz Gigapixel Presets: the saved presets (a click runs one with Gigapixel's
// command-line tool, in the background) and Manage Presets…, a dialog that makes, edits and
// deletes them: one upscale (model, size, the strengths Gigapixel would otherwise pick itself)
// or two saved presets run together, one after the other or each on its own layer.

/// The submenu.
const SUBMENU: &str = "Topaz Gigapixel Presets";
/// Menu ids of the saved presets: this plus the preset's index.
const PRESET_ITEM: &str = "file.automate.topazGigapixel.preset.";
/// Manage Presets… (opens [`open_manager`]).
pub const MANAGE_ID: &str = "file.automate.topazGigapixel.presets.manage";
/// The dialog's marker field.
const MARK: &str = "__gigapixelPresets";

/// Run Two Presets…: two saved presets on the image now, chained or each on its own layer.
pub const RUN_TWO_ID: &str = "file.automate.topazGigapixel.presets.runTwo";
/// The Run Two Presets dialog's marker field.
const RUN_MARK: &str = "__gigapixelRunTwo";

/// Save Preset from Last Upscale… (the settings Gigapixel's app last sent back).
pub const LAST_ID: &str = "file.automate.topazGigapixel.presets.fromLast";

/// Gigapixel's models as the dialog offers them: the CLI's name and version (`model:version`),
/// with the names its app shows ("" lets it choose). Without a version Gigapixel 8.4 runs V2 of
/// the models that have one.
pub const MODELS: [(&str, &str); 14] = [
    ("", "Auto"),
    ("std:2", "Standard V2"),
    ("std:1", "Standard"),
    ("hf:2", "High Fidelity V2"),
    ("hf:1", "High Fidelity"),
    ("lowres:2", "Low Resolution V2"),
    ("lowres:1", "Low Resolution"),
    ("art", "Art & CG"),
    ("lines", "Lines"),
    ("text", "Text & Shapes"),
    ("vc", "Very Compressed"),
    ("recovery:2", "Recovery V2"),
    ("recovery:1", "Recovery"),
    ("redefine", "Redefine"),
];

/// The models with versions, run at V2 when a preset names none.
const VERSIONED: [&str; 4] = ["std", "hf", "lowres", "recovery"];

/// The dialog's model key for a preset's `model` and `modelVersion`.
fn model_key(settings: &Value) -> String {
    let Some(model) = settings.get("model").and_then(Value::as_str) else { return String::new() };
    match settings.get("modelVersion").and_then(Value::as_u64) {
        Some(v) => format!("{model}:{v}"),
        None if VERSIONED.contains(&model) => format!("{model}:2"),
        None => model.to_string(),
    }
}

/// The strengths a preset may set (Gigapixel picks them itself otherwise): param, label, default.
/// Detail is Recovery's own; Recovery and Redefine take none of the other three.
const STRENGTHS: [(&str, &str, f64); 5] = [
    ("denoise", "Denoise:", 20.0),
    ("sharpen", "Sharpen:", 20.0),
    ("compression", "Fix Compression:", 20.0),
    ("detail", "Detail:", 50.0),
    ("faceRecovery", "Face Recovery:", 60.0),
];

/// Does the model (a dialog key) take strength `key`?
fn takes(model: &str, key: &str) -> bool {
    let model = model.split(':').next().unwrap_or("");
    match key {
        "detail" => model == "recovery",
        "denoise" | "sharpen" | "compression" => !matches!(model, "recovery" | "redefine"),
        _ => true,
    }
}

/// Inserts the submenu after File › Automate › Topaz Gigapixel AI….
pub fn insert_menu_items(app: &PhotocraftApp, items: &mut Vec<MenuItem>) {
    let Some(at) = items.iter().position(|i| i.id == photocraft_engine::gigapixel_cmds::ID) else { return };
    let Some(mut path) = items.get(at).map(|i| i.path.clone()) else { return };
    path.push(SUBMENU.into());
    let enabled = app.session.is_enabled(photocraft_engine::gigapixel_cmds::ID);
    let item = |id: String, label: String, enabled: bool| MenuItem { id, label, path: path.clone(), shortcut: None, enabled, checked: None, color: None };
    let mut rows: Vec<MenuItem> =
        app.session.presets.gigapixel_presets.iter().enumerate().map(|(i, p)| item(format!("{PRESET_ITEM}{i}"), p.name.clone(), enabled)).collect();
    if !rows.is_empty() {
        rows.push(item("---".into(), "---".into(), false));
    }
    rows.push(item(RUN_TWO_ID.into(), "Run Two Presets…".into(), enabled && singles(&app.session.presets.gigapixel_presets).len() >= 2));
    rows.push(item(LAST_ID.into(), "Save Preset from Last Upscale…".into(), app.session.presets.gigapixel_last.is_some()));
    rows.push(item(MANAGE_ID.into(), "Manage Presets…".into(), true));
    for (k, row) in rows.into_iter().enumerate() {
        items.insert(at + 1 + k, row);
    }
}

/// Enabled state of the submenu's items (`None` for other ids).
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if id == MANAGE_ID {
        return Some(true);
    }
    if id == LAST_ID {
        return Some(app.session.presets.gigapixel_last.is_some());
    }
    if id == RUN_TWO_ID {
        return Some(singles(&app.session.presets.gigapixel_presets).len() >= 2 && app.session.is_enabled(photocraft_engine::gigapixel_cmds::ID));
    }
    let i: usize = id.strip_prefix(PRESET_ITEM)?.parse().ok()?;
    Some(app.session.presets.gigapixel_presets.len() > i && app.session.is_enabled(photocraft_engine::gigapixel_cmds::ID))
}

/// Runs a preset (`file.automate.topazGigapixel.preset.<index>`) or opens Manage Presets….
pub fn menu(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    if id == MANAGE_ID {
        return Some(Ok(json!({"dialog": open_manager(app)})));
    }
    if id == RUN_TWO_ID {
        return Some(open_run_two(app).map(|d| json!({"dialog": d})).ok_or_else(|| tl!("Save two presets with their own settings first; then this one can run them together.").into()));
    }
    if id == LAST_ID {
        return Some(open_from_last(app).map(|d| json!({"dialog": d})).ok_or_else(|| tl!("No upscale has come back from Topaz Gigapixel AI yet").into()));
    }
    let i: usize = id.strip_prefix(PRESET_ITEM)?.parse().ok()?;
    let Some(name) = app.session.presets.gigapixel_presets.get(i).map(|p| p.name.clone()) else {
        return Some(Err(tl!("There is no such Topaz Gigapixel preset").into()));
    };
    Some(app.run(photocraft_engine::gigapixel_cmds::ID, json!({"preset": name})))
}

/// Open Manage Presets… (an open one comes back), on the first saved preset or a new one.
pub fn open_manager(app: &mut PhotocraftApp) -> u64 {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(MARK)) {
        return d.id;
    }
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__label".into(), json!("Topaz Gigapixel Presets"));
    let first = app.session.presets.gigapixel_presets.first().cloned();
    load(&mut f, first.as_ref(), &app.session.presets.gigapixel_presets);
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Save Preset from Last Upscale…: Manage Presets… on a new preset holding the settings Gigapixel's
/// app last sent back, named after them ("Low Resolution V2 2×"). `None` when none came back.
pub fn open_from_last(app: &mut PhotocraftApp) -> Option<u64> {
    let settings = app.session.presets.gigapixel_last.clone()?;
    let all = app.session.presets.gigapixel_presets.clone();
    let base = photocraft_engine::gigapixel_cmds::describe_settings(&settings);
    let name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base} {n}") }).find(|n| !all.iter().any(|p| p.name.eq_ignore_ascii_case(n)))?;
    let id = open_manager(app);
    let d = app.ui.dialog_mut(id)?;
    load(&mut d.fields, Some(&GigapixelPreset { name, settings }), &all);
    d.fields.insert("editing".into(), json!(""));
    Some(id)
}

/// The saved presets that are one upscale each (the ones that can be run together).
fn singles(all: &[GigapixelPreset]) -> Vec<&GigapixelPreset> {
    all.iter().filter(|p| p.combined().is_none()).collect()
}

/// Run Two Presets… (an open one comes back): the first two one-upscale presets, chained.
/// `None` with fewer than two saved.
pub fn open_run_two(app: &mut PhotocraftApp) -> Option<u64> {
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.fields.contains_key(RUN_MARK)) {
        return Some(d.id);
    }
    let all = app.session.presets.gigapixel_presets.clone();
    let names: Vec<String> = singles(&all).iter().map(|p| p.name.clone()).collect();
    let (first, second) = (names.first()?.clone(), names.get(1)?.clone());
    let mut f = Map::new();
    f.insert(RUN_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Run Two Topaz Gigapixel Presets"));
    f.insert("first".into(), json!(first));
    f.insert("second".into(), json!(second));
    f.insert("combine".into(), json!("chain"));
    Some(app.ui.open_dialog(DialogKind::Command, f))
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(MARK) || f.contains_key(RUN_MARK)
}

/// The dialog's OK: Save in Manage Presets, Run in Run Two Presets.
pub fn ok_label(f: &Map<String, Value>) -> &'static str {
    if f.contains_key(RUN_MARK) { tl!("Run") } else { tl!("Save") }
}

fn text<'a>(f: &'a Map<String, Value>, k: &str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or("")
}

fn num(f: &Map<String, Value>, k: &str, default: f64) -> f64 {
    f.get(k).and_then(Value::as_f64).unwrap_or(default)
}

fn flag(f: &Map<String, Value>, k: &str) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// The dialog's fields for `preset` (`None`: a new one, named so it is unique among `all`).
fn load(f: &mut Map<String, Value>, preset: Option<&GigapixelPreset>, all: &[GigapixelPreset]) {
    let s = preset.map(|p| p.settings.clone()).unwrap_or_else(|| json!({"model": "std", "scale": 2}));
    let name = match preset {
        Some(p) => p.name.clone(),
        None => (1..).map(|n| if n == 1 { "Upscale".to_string() } else { format!("Upscale {n}") }).find(|n| !all.iter().any(|p| p.name.eq_ignore_ascii_case(n))).unwrap_or_default(),
    };
    f.insert("editing".into(), json!(preset.map(|p| p.name.clone()).unwrap_or_default()));
    f.insert("name".into(), json!(name));
    let combined = s.get("presets").and_then(Value::as_array).cloned();
    f.insert("kind".into(), json!(if combined.is_some() { "two" } else { "one" }));
    let names = combined.unwrap_or_default();
    let singles: Vec<&str> = all.iter().filter(|p| p.combined().is_none()).map(|p| p.name.as_str()).collect();
    f.insert("first".into(), json!(names.first().and_then(Value::as_str).or(singles.first().copied()).unwrap_or("")));
    f.insert("second".into(), json!(names.get(1).and_then(Value::as_str).or(singles.get(1).copied()).unwrap_or("")));
    f.insert("combine".into(), json!(s.get("combine").and_then(Value::as_str).unwrap_or("chain")));
    f.insert("model".into(), json!(model_key(&s)));
    f.insert("faceRecoveryVersion".into(), s.get("faceRecoveryVersion").cloned().unwrap_or(Value::Null));
    let (size, amount) = ["width", "height"].iter().find_map(|k| s.get(*k).and_then(Value::as_f64).map(|v| (*k, v))).unwrap_or(("scale", s.get("scale").and_then(Value::as_f64).unwrap_or(2.0)));
    f.insert("size".into(), json!(size));
    f.insert("amount".into(), json!(amount));
    for (key, _, default) in STRENGTHS {
        let v = s.get(key).and_then(Value::as_f64);
        f.insert(format!("{key}On"), json!(v.is_some()));
        f.insert(key.into(), json!(v.unwrap_or(default)));
    }
}

/// The `settings` the fields describe.
fn settings(f: &Map<String, Value>) -> Value {
    if text(f, "kind") == "two" {
        return json!({"presets": [text(f, "first"), text(f, "second")], "combine": text(f, "combine")});
    }
    let mut s = Map::new();
    let key = text(f, "model");
    let (model, version) = key.split_once(':').map_or((key, None), |(m, v)| (m, v.parse::<u64>().ok()));
    if !model.is_empty() {
        s.insert("model".into(), json!(model));
        if let Some(v) = version {
            s.insert("modelVersion".into(), json!(v));
        }
    }
    let size = match text(f, "size") {
        "width" => "width",
        "height" => "height",
        _ => "scale",
    };
    let amount = num(f, "amount", 2.0);
    s.insert(size.into(), if size == "scale" { json!(amount) } else { json!(amount.round()) });
    for (k, _, default) in STRENGTHS {
        if takes(key, k) && flag(f, &format!("{k}On")) {
            s.insert(k.into(), json!(num(f, k, default).round()));
        }
    }
    if s.contains_key("faceRecovery")
        && let Some(v) = f.get("faceRecoveryVersion").filter(|v| !v.is_null())
    {
        s.insert("faceRecoveryVersion".into(), v.clone());
    }
    Value::Object(s)
}

/// Save needs a name and, for two presets, two chosen.
pub fn can_confirm(f: &Map<String, Value>) -> bool {
    if f.contains_key(RUN_MARK) {
        return !text(f, "first").is_empty() && !text(f, "second").is_empty();
    }
    !owns(f) || (!text(f, "name").trim().is_empty() && (text(f, "kind") != "two" || (!text(f, "first").is_empty() && !text(f, "second").is_empty())))
}

/// Save: the preset (replacing the one being edited).
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if f.contains_key(RUN_MARK) {
        let p = json!({"presets": [text(f, "first"), text(f, "second")], "combine": text(f, "combine")});
        return app.run(photocraft_engine::gigapixel_cmds::ID, p);
    }
    let editing = text(f, "editing");
    let mut p = json!({"name": text(f, "name").trim(), "settings": settings(f)});
    if !editing.is_empty() && app.session.presets.gigapixel_presets.iter().any(|g| g.name == editing) {
        p["replace"] = json!(editing);
    }
    let r = app.run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, p)?;
    app.ui.status_error = false;
    app.ui.status = crate::i18n::fmt(tl!("Saved Topaz Gigapixel preset “{name}”"), &[("name", text(f, "name").trim())]);
    Ok(r)
}

/// The name Run Two Presets suggests for saving the pair: "First → Then" or "First + Then".
fn pair_name(f: &Map<String, Value>) -> String {
    let joint = if text(f, "combine") == "layers" { " + " } else { " → " };
    format!("{}{joint}{}", text(f, "first"), text(f, "second"))
}

/// Run Two Presets' Save: the two presets, as chosen, saved as one preset that the Topaz
/// Gigapixel Presets submenu lists with the others (an empty name takes the suggested one).
fn save_two(app: &mut PhotocraftApp, f: &mut Map<String, Value>) -> Result<String, String> {
    let typed = text(f, "saveName").trim().to_string();
    let name = if typed.is_empty() { pair_name(f) } else { typed };
    let settings = json!({"presets": [text(f, "first"), text(f, "second")], "combine": text(f, "combine")});
    app.run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": name, "settings": settings}))?;
    f.insert("saveName".into(), json!(""));
    Ok(name)
}

fn save_two_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(8.0);
    widgets::hairline(ui);
    ui.add_space(6.0);
    let mut name = text(f, "saveName").to_string();
    let mut save = false;
    row(ui, tl!("Save as:"), |ui| {
        let hint = pair_name(f);
        let r = ui.add(egui::TextEdit::singleline(&mut name).char_limit(255).desired_width(220.0).hint_text(hint));
        save |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        save |= widgets::secondary_button(ui, tl!("Save Preset"), 0.0).on_hover_text(tl!("Keep these two presets as one, listed with the others")).clicked();
    });
    f.insert("saveName".into(), json!(name));
    if save && can_confirm(f) {
        match save_two(app, f) {
            Ok(name) => {
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(tl!("Saved Topaz Gigapixel preset “{name}”"), &[("name", &name)]);
            }
            Err(e) => {
                app.ui.status_error = true;
                app.ui.status = e;
            }
        }
    }
}

/// Width of the labels' column.
const LABELS: f32 = 130.0;

/// A row: `label` right-aligned in the labels' column, then `add`.
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(LABELS, 22.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_min_width(LABELS);
            ui.add(egui::Label::new(label).selectable(false));
        });
        add(ui);
    });
}

/// First / Then / Results, from the one-upscale presets `singles` (name, label).
fn two_preset_rows(ui: &mut egui::Ui, f: &mut Map<String, Value>, singles: &[(String, &str)]) {
    for (key, label) in [("first", tl!("First:")), ("second", tl!("Then:"))] {
        let mut v = text(f, key).to_string();
        row(ui, label, |ui| {
            if widgets::dropdown_literal(ui, &format!("gigapixel-preset-{key}"), &mut v, singles, 220.0) {
                f.insert(key.into(), json!(v));
            }
        });
    }
    let mut combine = text(f, "combine").to_string();
    row(ui, tl!("Results:"), |ui| {
        let options = [("chain".to_string(), tl!("One after the other")), ("layers".to_string(), tl!("Each on its own layer"))];
        if widgets::dropdown(ui, "gigapixel-preset-combine", &mut combine, &options, 220.0) {
            f.insert("combine".into(), json!(combine));
        }
    });
    let hint = if combine == "layers" {
        tl!("Both presets upscale the image; each result becomes a layer, to compare or blend.")
    } else {
        tl!("The second preset upscales the first one's result.")
    };
    ui.label(egui::RichText::new(hint).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.set_min_width(420.0);
    if f.contains_key(RUN_MARK) {
        let all = app.session.presets.gigapixel_presets.clone();
        let names: Vec<(String, &str)> = singles(&all).into_iter().map(|p| (p.name.clone(), p.name.as_str())).collect();
        two_preset_rows(ui, f, &names);
        save_two_row(app, ui, f);
        return;
    }
    let all = app.session.presets.gigapixel_presets.clone();
    // Which preset the fields edit: a saved one, or a new one ("").
    let mut editing = text(f, "editing").to_string();
    let mut choices: Vec<(String, &str)> = vec![(String::new(), tl!("New Preset"))];
    choices.extend(all.iter().map(|p| (p.name.clone(), p.name.as_str())));
    let mut delete = false;
    row(ui, tl!("Preset:"), |ui| {
        if widgets::dropdown_literal(ui, "gigapixel-preset-pick", &mut editing, &choices, 220.0) {
            let picked = all.iter().find(|p| p.name == editing).cloned();
            load(f, picked.as_ref(), &all);
        }
        if !editing.is_empty() && widgets::secondary_button(ui, tl!("Delete"), 70.0).clicked() {
            delete = true;
        }
    });
    if delete {
        match app.run(photocraft_engine::gigapixel_cmds::presets::DELETE_ID, json!({"name": editing})) {
            Ok(_) => {
                let rest = app.session.presets.gigapixel_presets.clone();
                load(f, rest.first(), &rest);
            }
            Err(e) => {
                app.ui.status_error = true;
                app.ui.status = e;
            }
        }
        return;
    }
    ui.add_space(6.0);
    let mut name = text(f, "name").to_string();
    row(ui, tl!("Name:"), |ui| {
        ui.add(egui::TextEdit::singleline(&mut name).char_limit(255).desired_width(220.0));
    });
    f.insert("name".into(), json!(name));
    // Two presets need two one-upscale presets saved first (not the one being edited).
    let singles: Vec<(String, &str)> = all.iter().filter(|p| p.combined().is_none() && p.name != editing).map(|p| (p.name.clone(), p.name.as_str())).collect();
    let mut kind = text(f, "kind").to_string();
    row(ui, tl!("Upscale:"), |ui| {
        let options = [("one".to_string(), tl!("With these settings")), ("two".to_string(), tl!("With two presets"))];
        if widgets::dropdown(ui, "gigapixel-preset-kind", &mut kind, &options, 220.0) {
            f.insert("kind".into(), json!(kind));
        }
    });
    ui.add_space(6.0);
    if kind == "two" {
        if singles.len() < 2 {
            ui.label(egui::RichText::new(tl!("Save two presets with their own settings first; then this one can run them together.")).color(crate::theme::Tokens::get(ui.ctx()).text_dim));
            return;
        }
        two_preset_rows(ui, f, &singles);
        return;
    }
    let mut model = text(f, "model").to_string();
    row(ui, tl!("Model:"), |ui| {
        if widgets::dropdown_literal(ui, "gigapixel-preset-model", &mut model, &MODELS.map(|(k, l)| (k.to_string(), if k.is_empty() { tl!("Auto") } else { l })), 220.0) {
            f.insert("model".into(), json!(model));
        }
    });
    let mut size = text(f, "size").to_string();
    let mut amount = num(f, "amount", 2.0) as f32;
    row(ui, tl!("Size:"), |ui| {
        let options = [("scale".to_string(), tl!("Scale")), ("width".to_string(), tl!("Width")), ("height".to_string(), tl!("Height"))];
        if widgets::dropdown(ui, "gigapixel-preset-size", &mut size, &options, 100.0) {
            amount = if size == "scale" { 2.0 } else { 4000.0 };
        }
        if size == "scale" {
            widgets::value_field(ui, &mut amount, 1.0..=16.0, "×", 70.0);
        } else {
            widgets::value_field(ui, &mut amount, 1.0..=65_000.0, "px", 90.0);
        }
    });
    f.insert("size".into(), json!(size));
    f.insert("amount".into(), json!(f64::from(amount)));
    ui.add_space(4.0);
    // A strength is the preset's own, or Auto: Gigapixel picks it for each image.
    for (key, label, default) in STRENGTHS.into_iter().filter(|(k, _, _)| takes(&model, k)) {
        let on_key = format!("{key}On");
        let mut auto = !flag(f, &on_key);
        let mut v = num(f, key, default) as f32;
        row(ui, tl!(label), |ui| {
            ui.add_enabled_ui(!auto, |ui| widgets::value_field(ui, &mut v, 0.0..=100.0, "", 60.0));
            widgets::checkbox(ui, &mut auto, "Auto");
        });
        f.insert(on_key, json!(!auto));
        f.insert(key.into(), json!(f64::from(v)));
    }
    ui.add_space(6.0);
    let dim = crate::theme::Tokens::get(ui.ctx()).text_dim;
    ui.label(egui::RichText::new(tl!("Auto lets Gigapixel choose.")).color(dim));
    ui.label(egui::RichText::new(tl!("Run a saved preset from File › Automate › Topaz Gigapixel Presets.")).color(dim));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
        })
    }

    fn manager(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Option<(u64, Map<String, Value>)> {
        h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| (d.id, d.fields.clone()))
    }

    fn set(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, key: &str, v: Value) {
        let (id, _) = manager(h).unwrap();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.into(), v);
        h.run_steps(2);
    }

    /// File › Automate › Topaz Gigapixel Presets lists the saved presets, then Manage Presets….
    #[test]
    fn the_submenu_lists_the_saved_presets() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let sub = ["File", "Automate", SUBMENU];
        let rows = |app: &PhotocraftApp| -> Vec<(String, String)> {
            crate::menus::menu_items(app).into_iter().filter(|i| i.path == sub).map(|i| (i.id, i.label)).collect()
        };
        assert_eq!(
            rows(&app),
            [
                (RUN_TWO_ID.to_string(), "Run Two Presets…".to_string()),
                (LAST_ID.to_string(), "Save Preset from Last Upscale…".to_string()),
                (MANAGE_ID.to_string(), "Manage Presets…".to_string())
            ]
        );
        assert!(!crate::menus::is_enabled(&app, LAST_ID), "no upscale came back yet");
        assert!(crate::menus::is_enabled(&app, MANAGE_ID));
        app.run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": "Faces 2x", "settings": {"model": "std", "scale": 2}})).unwrap();
        app.run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": "Art 4x", "settings": {"model": "art", "scale": 4}})).unwrap();
        let labels: Vec<String> = rows(&app).into_iter().map(|(_, l)| l).collect();
        assert_eq!(labels, ["Faces 2x", "Art 4x", "---", "Run Two Presets…", "Save Preset from Last Upscale…", "Manage Presets…"]);
        // The submenu sits right after Topaz Gigapixel AI….
        let items = crate::menus::menu_items(&app);
        let at = items.iter().position(|i| i.id == photocraft_engine::gigapixel_cmds::ID).unwrap();
        assert_eq!(items[at + 1].id, format!("{PRESET_ITEM}0"));
        // With no document a preset can't run; an unknown index is no item.
        assert!(!crate::menus::is_enabled(&app, &format!("{PRESET_ITEM}0")));
        assert_eq!(is_enabled(&app, &format!("{PRESET_ITEM}7")), Some(false));
        assert!(menu(&mut app, &format!("{PRESET_ITEM}7")).unwrap().is_err());
        assert!(menu(&mut app, "file.automate.batch").is_none());
    }

    /// Manage Presets… saves a one-upscale preset from its fields, edits (renames) it, makes a
    /// two-preset one from saved presets and deletes.
    #[test]
    fn manage_presets_saves_edits_and_deletes() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness();
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, MANAGE_ID, json!({})).unwrap();
        h.run_steps(4);
        for label in ["Preset:", "Name:", "Upscale:", "Model:", "Size:", "Denoise:", "Face Recovery:", "Save", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "missing {label}");
        }
        let (_, f) = manager(&h).unwrap();
        assert_eq!(text(&f, "name"), "Upscale");
        assert_eq!(settings(&f), json!({"model": "std", "modelVersion": 2, "scale": 2.0}));
        set(&mut h, "name", json!("Faces 2x"));
        set(&mut h, "faceRecoveryOn", json!(true));
        set(&mut h, "faceRecovery", json!(55.4));
        h.get_by_label("Save").click();
        h.run_steps(3);
        assert!(manager(&h).is_none(), "closed");
        let saved = h.state().session.presets.gigapixel_presets.clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].settings, json!({"model": "std", "modelVersion": 2, "scale": 2.0, "faceRecovery": 55.0}));

        // Reopened on the saved preset: rename it and size it by width.
        crate::menus::invoke(h.state_mut(), &ctx, MANAGE_ID, json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(text(&manager(&h).unwrap().1, "editing"), "Faces 2x");
        assert!(flag(&manager(&h).unwrap().1, "faceRecoveryOn"));
        set(&mut h, "name", json!("Faces wide"));
        set(&mut h, "size", json!("width"));
        set(&mut h, "amount", json!(3999.6));
        set(&mut h, "model", json!(""));
        h.get_by_label("Save").click();
        h.run_steps(3);
        let saved = h.state().session.presets.gigapixel_presets.clone();
        assert_eq!(saved.len(), 1, "replaced, not added");
        assert_eq!(saved[0].name, "Faces wide");
        assert_eq!(saved[0].settings, json!({"width": 4000.0, "faceRecovery": 55.0}));

        // Two presets need two saved; then they combine.
        h.state_mut().run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": "Art", "settings": {"model": "art", "scale": 2}})).unwrap();
        crate::menus::invoke(h.state_mut(), &ctx, MANAGE_ID, json!({})).unwrap();
        h.run_steps(3);
        set(&mut h, "editing", json!(""));
        let (id, _) = manager(&h).unwrap();
        let mut f = Map::new();
        load(&mut f, None, &h.state().session.presets.gigapixel_presets);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.extend(f);
        set(&mut h, "kind", json!("two"));
        assert!(h.query_by_label("First:").is_some() && h.query_by_label("Then:").is_some() && h.query_by_label("Results:").is_some());
        set(&mut h, "name", json!("Both"));
        set(&mut h, "combine", json!("layers"));
        assert_eq!(settings(&manager(&h).unwrap().1), json!({"presets": ["Faces wide", "Art"], "combine": "layers"}));
        h.get_by_label("Save").click();
        h.run_steps(3);
        let names: Vec<String> = h.state().session.presets.gigapixel_presets.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, ["Faces wide", "Art", "Both"]);

        // Delete removes the preset being edited.
        crate::menus::invoke(h.state_mut(), &ctx, MANAGE_ID, json!({})).unwrap();
        h.run_steps(3);
        h.get_by_label("Delete").click();
        h.run_steps(3);
        let names: Vec<String> = h.state().session.presets.gigapixel_presets.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, ["Art", "Both"]);
        assert_eq!(text(&manager(&h).unwrap().1, "editing"), "Art", "on the next one");
    }

    /// The submenu is reachable in the menu bar: File › Automate › Topaz Gigapixel Presets shows
    /// the saved presets and Manage Presets…, which opens the dialog.
    #[test]
    fn the_submenu_opens_from_the_menu_bar() {
        use egui_kittest::kittest::Queryable;
        // Tall enough for the whole submenu below File › Automate.
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1100.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
        });
        h.state_mut().run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": "Faces 2x", "settings": {"scale": 2}})).unwrap();
        h.run_steps(2);
        let hover = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, label: &str| {
            let r = h.get_by_label_contains(label).rect();
            for x in [r.left() + 20.0, r.center().x, r.right() - 8.0] {
                h.event(egui::Event::PointerMoved(egui::pos2(x, r.center().y)));
                h.run_steps(2);
            }
        };
        h.get_by_label("File").click();
        h.run_steps(3);
        assert!(h.query_by_label_contains("Automate").is_some(), "File is open");
        hover(&mut h, "Automate");
        assert!(h.query_by_label("Topaz Gigapixel AI…").is_some(), "File › Automate is open");
        hover(&mut h, SUBMENU);
        assert!(h.query_by_label("Faces 2x").is_some(), "the saved preset is listed");
        let manage = h.get_by_label("Manage Presets…").rect();
        h.event(egui::Event::PointerMoved(manage.center()));
        h.run_steps(2);
        h.get_by_label("Manage Presets…").click();
        h.run_steps(4);
        assert!(manager(&h).is_some(), "Manage Presets… opened the dialog");
    }

    /// Save Preset from Last Upscale… opens a new preset holding the settings Gigapixel's app
    /// last sent back, named after them, which Save keeps as they are.
    #[test]
    fn a_preset_is_made_from_the_last_upscale() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness();
        let ctx = h.ctx.clone();
        assert!(crate::menus::invoke(h.state_mut(), &ctx, LAST_ID, json!({})).is_err(), "nothing came back yet");
        let last = json!({"model": "recovery", "modelVersion": 2, "scale": 4.0, "detail": 50.0, "faceRecovery": 80.0, "faceRecoveryVersion": 2});
        h.state_mut().session.presets.gigapixel_last = Some(last.clone());
        h.state_mut().run(photocraft_engine::gigapixel_cmds::presets::SAVE_ID, json!({"name": "Recovery V2 4×", "settings": {"scale": 2}})).unwrap();
        assert!(crate::menus::is_enabled(h.state(), LAST_ID));
        crate::menus::invoke(h.state_mut(), &ctx, LAST_ID, json!({})).unwrap();
        h.run_steps(4);
        let (_, f) = manager(&h).unwrap();
        assert_eq!(text(&f, "editing"), "", "a new preset");
        assert_eq!(text(&f, "name"), "Recovery V2 4× 2", "named after the settings, not over a saved one");
        assert_eq!(text(&f, "model"), "recovery:2");
        assert_eq!(settings(&f), last);
        // Recovery shows its Detail and not the strengths it doesn't take.
        assert!(h.query_by_label("Detail:").is_some());
        assert!(h.query_by_label("Denoise:").is_none());
        h.get_by_label("Save").click();
        h.run_steps(3);
        let saved = h.state().session.presets.gigapixel_presets.clone();
        assert_eq!(saved.len(), 2);
        assert_eq!((saved[1].name.as_str(), &saved[1].settings), ("Recovery V2 4× 2", &last));
        // Another model drops the strengths Recovery used that it doesn't take.
        let mut f = f;
        f.insert("model".into(), json!("art"));
        assert_eq!(settings(&f), json!({"model": "art", "scale": 4.0, "faceRecovery": 80.0, "faceRecoveryVersion": 2}));
        // Presets saved without a version read as the V2 Gigapixel runs.
        assert_eq!(model_key(&json!({"model": "hf"})), "hf:2");
        assert_eq!(model_key(&json!({"model": "art"})), "art");
        assert_eq!(model_key(&json!({"model": "std", "modelVersion": 1})), "std:1");
        assert_eq!(model_key(&json!({})), "");
    }

    /// Run Two Presets… offers the one-upscale presets, the first two chosen, and runs both on
    /// the image: chained, or each on its own layer.
    #[test]
    fn run_two_presets_runs_both() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness();
        let ctx = h.ctx.clone();
        h.state_mut().run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let save = photocraft_engine::gigapixel_cmds::presets::SAVE_ID;
        h.state_mut().run(save, json!({"name": "Faces", "settings": {"model": "std", "scale": 2}})).unwrap();
        assert!(open_run_two(h.state_mut()).is_none(), "one preset isn't two");
        h.state_mut().run(save, json!({"name": "Art", "settings": {"model": "art", "scale": 2}})).unwrap();
        h.state_mut().run(save, json!({"name": "Both", "settings": {"presets": ["Faces", "Art"]}})).unwrap();
        crate::menus::invoke(h.state_mut(), &ctx, RUN_TWO_ID, json!({})).unwrap();
        h.run_steps(4);
        let (id, f) = h.state().ui.dialogs.iter().find(|d| d.fields.contains_key(RUN_MARK)).map(|d| (d.id, d.fields.clone())).unwrap();
        assert_eq!((text(&f, "first"), text(&f, "second"), text(&f, "combine")), ("Faces", "Art", "chain"));
        for label in ["First:", "Then:", "Results:", "Run", "Cancel"] {
            assert!(h.query_by_label(label).is_some(), "missing {label}");
        }
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("combine".into(), json!("layers"));
        // The run itself: Gigapixel is never started in tests, so it stops at launching, after
        // the presets were found and checked.
        photocraft_engine::gigapixel_cmds::disable_launching();
        let mut f = f;
        f.insert("combine".into(), json!("layers"));
        let e = confirm(h.state_mut(), &f).unwrap_err();
        assert!(e.contains("disabled"), "{e}");
        f.insert("second".into(), json!("Both"));
        assert!(!confirm(h.state_mut(), &f).unwrap_err().contains("disabled"), "a preset that combines others can't be one of the two");
    }

    /// Run Two Presets saves the pair as it's set up, as one preset the submenu lists with the
    /// others; an empty name takes the suggested "First → Then" / "First + Then".
    #[test]
    fn run_two_presets_saves_the_pair_as_a_preset() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness();
        let save = photocraft_engine::gigapixel_cmds::presets::SAVE_ID;
        h.state_mut().run(save, json!({"name": "Faces", "settings": {"model": "std", "scale": 2}})).unwrap();
        h.state_mut().run(save, json!({"name": "Art", "settings": {"model": "art", "scale": 2}})).unwrap();
        open_run_two(h.state_mut()).unwrap();
        h.run_steps(4);
        assert!(h.query_by_label("Save Preset").is_some() && h.query_by_label("Save as:").is_some());
        let mut f = h.state().ui.dialogs.iter().find(|d| d.fields.contains_key(RUN_MARK)).map(|d| d.fields.clone()).unwrap();
        f.insert("combine".into(), json!("layers"));
        assert_eq!(save_two(h.state_mut(), &mut f).unwrap(), "Faces + Art");
        f.insert("combine".into(), json!("chain"));
        f.insert("saveName".into(), json!("  Both  "));
        assert_eq!(save_two(h.state_mut(), &mut f).unwrap(), "Both");
        let all = &h.state().session.presets.gigapixel_presets;
        let settings = |n: &str| all.iter().find(|p| p.name == n).map(|p| p.settings.clone());
        assert_eq!(settings("Faces + Art"), Some(json!({"presets": ["Faces", "Art"], "combine": "layers"})));
        assert_eq!(settings("Both"), Some(json!({"presets": ["Faces", "Art"], "combine": "chain"})));
        // The same name again saves over it; one of the pair's own names is refused (it would
        // run itself).
        f.insert("combine".into(), json!("layers"));
        f.insert("saveName".into(), json!("Both"));
        save_two(h.state_mut(), &mut f).unwrap();
        let both = h.state().session.presets.gigapixel_presets.iter().filter(|p| p.name == "Both").map(|p| p.settings["combine"].clone()).collect::<Vec<_>>();
        assert_eq!(both, [json!("layers")]);
        f.insert("saveName".into(), json!("Faces"));
        assert!(save_two(h.state_mut(), &mut f).is_err());
        // Both are listed with the other presets, and run from there.
        let labels: Vec<String> = crate::menus::menu_items(h.state()).into_iter().filter(|i| i.path == ["File", "Automate", SUBMENU]).map(|i| i.label).collect();
        assert_eq!(labels[..4], ["Faces", "Art", "Faces + Art", "Both"]);
    }

    #[test]
    fn save_needs_a_name_and_two_presets() {
        let mut f = Map::new();
        f.insert(MARK.into(), json!(true));
        load(&mut f, None, &[]);
        assert!(can_confirm(&f));
        f.insert("name".into(), json!("  "));
        assert!(!can_confirm(&f));
        f.insert("name".into(), json!("Two"));
        f.insert("kind".into(), json!("two"));
        assert!(!can_confirm(&f), "no presets to combine");
        assert!(can_confirm(&Map::new()), "other dialogs");
    }

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
