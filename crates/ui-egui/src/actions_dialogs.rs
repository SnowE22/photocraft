//! The Actions panel's dialogs, as Photoshop has them: New Action… (Record starts recording),
//! New Set…, Action Options…, Set Options…, Playback Options…, Insert Menu Item…, Insert
//! Conditional… and the delete confirmation. Each is a command dialog marked with
//! [`MARK`]: its `body` draws the fields and `confirm` runs the engine's `actions.*` commands.

use egui::Ui;
use serde_json::{Map, Value, json};

use photocraft_engine::actions_cmds::{self, BUTTON_COLORS, DEFAULT_SET, Playback};

use crate::state::DialogKind;
use crate::{PhotocraftApp, widgets};

const MARK: &str = "__actionsDialog";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn kind(fields: &Map<String, Value>) -> &str {
    fields.get(MARK).and_then(Value::as_str).unwrap_or_default()
}

fn text(fields: &Map<String, Value>, key: &str) -> String {
    fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn flag(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn open(app: &mut PhotocraftApp, kind: &str, label: &str, more: Value) -> u64 {
    let mut fields = more.as_object().cloned().unwrap_or_default();
    fields.insert(MARK.into(), json!(kind));
    fields.insert("__label".into(), json!(label));
    app.ui.open_dialog(DialogKind::Command, fields)
}

/// The OK button's label: Record for New Action, as in Photoshop.
pub fn ok_label(fields: &Map<String, Value>) -> Option<&'static str> {
    owns(fields).then(|| if kind(fields) == "newAction" { tl!("Record") } else { tl!("OK") })
}

/// The set a new action or set goes after: the selected row's, else the first set.
fn current_set(app: &PhotocraftApp) -> Option<String> {
    let a = &app.ui.actions;
    a.selected
        .and_then(|i| app.session.actions.list.get(i))
        .and_then(|a| a.set.clone())
        .or_else(|| a.selected_set.clone())
        .or_else(|| app.session.actions.sets().into_iter().next())
}

/// New Action…: a free "Action N" name, the current set, no key, no colour.
pub fn new_action(app: &mut PhotocraftApp) -> u64 {
    let list = &app.session.actions.list;
    let name = (1..).map(|n| format!("Action {n}")).find(|n| !list.iter().any(|a| a.name == *n)).unwrap_or_default();
    let set = current_set(app).unwrap_or_else(|| DEFAULT_SET.to_string());
    open(app, "newAction", "New Action", json!({"name": name, "set": set, "key": 0, "shift": false, "ctrl": false, "color": ""}))
}

/// New Set…: a free "Set N" name.
pub fn new_set(app: &mut PhotocraftApp) -> u64 {
    let sets = app.session.actions.sets();
    let name = (1..).map(|n| format!("Set {n}")).find(|n| !sets.contains(n)).unwrap_or_default();
    open(app, "newSet", "New Set", json!({"name": name}))
}

/// Action Options… for action `index`: its name, key and colour.
pub fn action_options(app: &mut PhotocraftApp, index: usize) -> Option<u64> {
    let a = app.session.actions.list.get(index)?;
    let key = app.session.prefs().shortcuts.get(&crate::actions::shortcut_id(&a.name)).cloned().unwrap_or_default();
    let (n, shift, ctrl) = actions_cmds::parse_action_key(&key).unwrap_or((0, false, false));
    let fields = json!({"action": a.name, "name": a.name, "key": n, "shift": shift, "ctrl": ctrl, "color": a.color, "otherKey": !key.is_empty() && n == 0});
    Some(open(app, "actionOptions", "Action Options", fields))
}

/// Set Options… for `set`: its name.
pub fn set_options(app: &mut PhotocraftApp, set: &str) -> u64 {
    open(app, "setOptions", "Set Options", json!({"set": set, "name": set}))
}

pub fn playback_options(app: &mut PhotocraftApp) -> u64 {
    let (mode, seconds) = match app.session.actions.prefs.playback {
        Playback::Accelerated => ("accelerated", 1.0),
        Playback::StepByStep => ("stepByStep", 1.0),
        Playback::Pause { seconds } => ("pause", seconds),
    };
    open(app, "playback", "Playback Options", json!({"mode": mode, "seconds": seconds}))
}

/// Where Insert… puts a step: after the selected step, else at the end of the selected action.
fn insert_point(app: &PhotocraftApp) -> Option<(usize, usize)> {
    let action = app.ui.actions.selected.filter(|i| *i < app.session.actions.list.len())?;
    let len = crate::actions::visible_steps(app, action).len();
    Some((action, app.ui.actions.selected_step.map_or(len, |s| (s + 1).min(len))))
}

pub fn insert_menu_item(app: &mut PhotocraftApp) -> Result<u64, String> {
    let (action, at) = insert_point(app).ok_or_else(|| tl!("Select an action first").to_string())?;
    Ok(open(app, "menuItem", "Insert Menu Item", json!({"action": action, "at": at, "query": "", "item": ""})))
}

pub fn insert_conditional(app: &mut PhotocraftApp) -> Result<u64, String> {
    let (action, at) = insert_point(app).ok_or_else(|| tl!("Select an action first").to_string())?;
    Ok(open(app, "conditional", "Conditional Action", json!({"action": action, "at": at, "if": "hasLayers", "then": "", "else": ""})))
}

/// What the trash button deletes, asked first as Photoshop does (Alt skips the question).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Set(String),
    Action(usize),
    Steps(usize, Vec<usize>),
}

pub fn confirm_delete(app: &mut PhotocraftApp, target: Target) -> u64 {
    let (message, params) = match &target {
        Target::Set(set) => (crate::i18n::fmt(tl!("Delete the set “{name}”?"), &[("name", set)]), json!({"set": set})),
        Target::Action(i) => {
            let name = app.session.actions.list.get(*i).map(|a| a.name.clone()).unwrap_or_default();
            (crate::i18n::fmt(tl!("Delete the action “{name}”?"), &[("name", &name)]), json!({"action": name}))
        }
        Target::Steps(i, steps) => (tl!("Delete the selection?").to_string(), json!({"action": i, "steps": steps})),
    };
    open(app, "delete", "Actions", json!({"message": message, "target": params}))
}

/// Delete what `target` names (the confirmation's OK, or Alt+trash).
pub fn delete(app: &mut PhotocraftApp, target: &Value) -> Result<Value, String> {
    if let Some(set) = target.get("set").and_then(Value::as_str) {
        let r = app.run("actions.deleteSet", json!({"set": set}))?;
        app.ui.actions.selected_set = None;
        return Ok(r);
    }
    if let Some(steps) = target.get("steps").and_then(Value::as_array) {
        let action = target.get("action").cloned().unwrap_or(Value::Null);
        let mut steps: Vec<u64> = steps.iter().filter_map(Value::as_u64).collect();
        // The last first, so the indices of the rest stay right.
        steps.sort_unstable();
        steps.dedup();
        let mut last = Ok(Value::Null);
        for step in steps.into_iter().rev() {
            last = app.run("actions.delete", json!({"action": action, "step": step}));
            if last.is_err() {
                break;
            }
        }
        app.ui.actions.clear_steps();
        return last;
    }
    let name = target.get("action").cloned().unwrap_or(Value::Null);
    let index = app.session.actions.list.iter().position(|a| Some(a.name.as_str()) == name.as_str());
    let r = app.run("actions.delete", json!({"action": name}))?;
    if let Some(i) = index
        && i < app.ui.actions.expanded.len()
    {
        app.ui.actions.expanded.remove(i);
    }
    app.ui.actions.selected = None;
    app.ui.actions.clear_steps();
    Ok(r)
}

/// The function key choices: None and F2–F12 (F1 is Help), as in Photoshop.
fn key_options() -> Vec<(u64, String)> {
    std::iter::once((0, tl!("None").to_string())).chain((2..=12).map(|n| (n, format!("F{n}")))).collect()
}

fn color_options() -> Vec<(String, &'static str)> {
    let labels = [tl!("None"), tl!("Red"), tl!("Orange"), tl!("Yellow"), tl!("Green"), tl!("Blue"), tl!("Violet"), tl!("Gray")];
    BUTTON_COLORS.iter().zip(labels).map(|(c, l)| (c.to_string(), l)).collect()
}

fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 22.0], egui::Label::new(label));
        add(ui);
    });
}

fn name_row(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let mut name = text(fields, "name");
    row(ui, tl!("Name:"), |ui| {
        let r = widgets::text_field(ui, egui::Id::new("actions-dialog-name"), &mut name, "", 220.0, false);
        if !fields.contains_key("__focused") {
            r.request_focus();
            fields.insert("__focused".into(), json!(true));
        }
    });
    fields.insert("name".into(), json!(name));
}

/// Function Key with Shift and Control, and Color: New Action and Action Options.
fn key_rows(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let mut key = fields.get("key").and_then(Value::as_u64).unwrap_or(0);
    let (mut shift, mut ctrl) = (flag(fields, "shift"), flag(fields, "ctrl"));
    let keys = key_options();
    let options: Vec<(u64, &str)> = keys.iter().map(|(k, l)| (*k, l.as_str())).collect();
    row(ui, tl!("Function Key:"), |ui| {
        widgets::dropdown_literal(ui, "actions-dialog-key", &mut key, &options, 80.0);
        ui.add_enabled_ui(key > 0, |ui| {
            widgets::checkbox(ui, &mut shift, &crate::shortcuts::pretty("Shift"));
            widgets::checkbox(ui, &mut ctrl, &crate::shortcuts::pretty("Cmd"));
        });
    });
    if flag(fields, "otherKey") && key == 0 {
        ui.label(egui::RichText::new(tl!("The action has another shortcut (Set Shortcut…); picking a key here replaces it.")).small());
    }
    let mut color = text(fields, "color");
    let colors = color_options();
    let options: Vec<(String, &str)> = colors.iter().map(|(c, l)| (c.clone(), *l)).collect();
    row(ui, tl!("Color:"), |ui| {
        widgets::dropdown_literal(ui, "actions-dialog-color", &mut color, &options, 120.0);
    });
    fields.insert("key".into(), json!(key));
    fields.insert("shift".into(), json!(shift));
    fields.insert("ctrl".into(), json!(ctrl));
    fields.insert("color".into(), json!(color));
}

/// Menu items Insert Menu Item can add: live commands a step can run, matching `query`.
fn menu_matches(app: &PhotocraftApp, query: &str) -> Vec<(String, String)> {
    let q = query.trim().to_lowercase();
    crate::menus::menu_items(app)
        .into_iter()
        .filter(|i| crate::menus::is_live(&i.id) && step_command(&i.id))
        .map(|i| {
            let path: Vec<String> = i.path.iter().map(|p| tl!(p.as_str()).to_string()).collect();
            (i.id, format!("{} › {}", path.join(" › "), tl!(i.label.as_str()).trim_end_matches('…')))
        })
        .filter(|(id, label)| q.is_empty() || label.to_lowercase().contains(&q) || id.to_lowercase().contains(&q))
        .take(200)
        .collect()
}

/// Whether playback can run `id` as a step.
fn step_command(id: &str) -> bool {
    photocraft_engine::commands::find(id).is_some() || actions_cmds::shell_view_command(id) || actions_cmds::shell_save_command(id)
}

pub fn body(app: &PhotocraftApp, ui: &mut Ui, fields: &mut Map<String, Value>) {
    match kind(fields) {
        "newAction" => {
            name_row(ui, fields);
            let mut set = text(fields, "set");
            let mut sets = app.session.actions.sets();
            if !sets.contains(&set) {
                sets.insert(0, set.clone());
            }
            let options: Vec<(String, &str)> = sets.iter().map(|s| (s.clone(), s.as_str())).collect();
            row(ui, tl!("Set:"), |ui| {
                widgets::dropdown_literal(ui, "actions-dialog-set", &mut set, &options, 220.0);
            });
            fields.insert("set".into(), json!(set));
            key_rows(ui, fields);
        }
        "newSet" | "setOptions" => name_row(ui, fields),
        "actionOptions" => {
            name_row(ui, fields);
            key_rows(ui, fields);
        }
        "playback" => {
            let mut mode = text(fields, "mode");
            ui.label(egui::RichText::new(tl!("Performance")).strong());
            for (m, label) in [("accelerated", tl!("Accelerated")), ("stepByStep", tl!("Step by Step"))] {
                if ui.radio(mode == m, label).clicked() {
                    mode = m.into();
                }
            }
            let mut seconds = fields.get("seconds").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            ui.horizontal(|ui| {
                if ui.radio(mode == "pause", tl!("Pause For:")).clicked() {
                    mode = "pause".into();
                }
                widgets::value_field(ui, &mut seconds, 0.0..=actions_cmds::MAX_PAUSE_S, tl!("seconds"), 64.0);
            });
            fields.insert("mode".into(), json!(mode));
            fields.insert("seconds".into(), json!(seconds));
        }
        "menuItem" => {
            let item = text(fields, "item");
            let shown = if item.is_empty() { tl!("None Selected").to_string() } else { crate::actions::label_of(&item) };
            row(ui, tl!("Menu Item:"), |ui| {
                ui.label(egui::RichText::new(shown).strong());
            });
            ui.label(egui::RichText::new(tl!("Type part of a menu item's name, then pick it below.")).small());
            let mut query = text(fields, "query");
            let r = widgets::text_field(ui, egui::Id::new("actions-menu-item-find"), &mut query, tl!("Find"), 320.0, false);
            if !fields.contains_key("__focused") {
                r.request_focus();
                fields.insert("__focused".into(), json!(true));
            }
            let mut picked = None;
            egui::ScrollArea::vertical().id_salt("actions-menu-items").max_height(220.0).show(ui, |ui| {
                ui.set_min_width(320.0);
                for (id, label) in menu_matches(app, &query) {
                    if ui.selectable_label(id == item, label).clicked() {
                        picked = Some(id);
                    }
                }
            });
            if let Some(id) = picked {
                fields.insert("item".into(), json!(id));
            }
            fields.insert("query".into(), json!(query));
        }
        "conditional" => {
            let mut cond = text(fields, "if");
            let options: Vec<(String, &str)> = photocraft_engine::actions_edit_cmds::CONDITIONS.iter().map(|c| (c.to_string(), condition_label(c))).collect();
            row(ui, tl!("If Current:"), |ui| {
                widgets::dropdown_literal(ui, "actions-cond-if", &mut cond, &options, 220.0);
            });
            let names: Vec<String> = std::iter::once(String::new()).chain(app.session.actions.list.iter().map(|a| a.name.clone())).collect();
            let none = tl!("None");
            let options: Vec<(String, &str)> = names.iter().map(|n| (n.clone(), if n.is_empty() { none } else { n.as_str() })).collect();
            for (key, label) in [("then", tl!("Then Play Action:")), ("else", tl!("Else Play Action:"))] {
                let mut v = text(fields, key);
                row(ui, label, |ui| {
                    widgets::dropdown_literal(ui, &format!("actions-cond-{key}"), &mut v, &options, 220.0);
                });
                fields.insert(key.into(), json!(v));
            }
            fields.insert("if".into(), json!(cond));
        }
        "delete" => {
            ui.label(text(fields, "message"));
        }
        _ => {}
    }
}

/// Photoshop's wording for an Insert Conditional condition.
fn condition_label(c: &str) -> &'static str {
    match c {
        "always" => tl!("Always"),
        "never" => tl!("Never"),
        "isOpen" => tl!("Document Is Open"),
        "landscape" => tl!("Document Is Landscape"),
        "square" => tl!("Document Is Square"),
        "portrait" => tl!("Document Is Portrait"),
        "modeIsRGB" => tl!("Document Mode Is RGB"),
        "modeIsCMYK" => tl!("Document Mode Is CMYK"),
        "modeIsGrayscale" => tl!("Document Mode Is Grayscale"),
        "modeIsLab" => tl!("Document Mode Is Lab"),
        "modeIsBitmap" => tl!("Document Mode Is Bitmap"),
        "modeIsIndexed" => tl!("Document Mode Is Indexed"),
        "modeIsDuotone" => tl!("Document Mode Is Duotone"),
        "modeIsMultichannel" => tl!("Document Mode Is Multichannel"),
        "depth8" => tl!("Document Is 8 Bits/Channel"),
        "depth16" => tl!("Document Is 16 Bits/Channel"),
        "depth32" => tl!("Document Is 32 Bits/Channel"),
        "unsaved" => tl!("Document Has Unsaved Changes"),
        "hasSelection" => tl!("Document Has a Selection"),
        "hasLayers" => tl!("Document Has Layers"),
        "hasAlphaChannels" => tl!("Document Has Alpha Channels"),
        "isBackground" => tl!("Layer Is Background"),
        "isPixelLayer" => tl!("Layer Is a Pixel Layer"),
        "isAdjustmentLayer" => tl!("Layer Is an Adjustment Layer"),
        "isShapeLayer" => tl!("Layer Is a Shape Layer"),
        "isTypeLayer" => tl!("Layer Is a Type Layer"),
        "isSmartObject" => tl!("Layer Is a Smart Object"),
        "isLayerGroup" => tl!("Layer Is a Group"),
        "isLocked" => tl!("Layer Is Locked"),
        "isVisible" => tl!("Layer Is Visible"),
        "hasPixelMask" => tl!("Layer Has Pixel Mask"),
        "hasVectorMask" => tl!("Layer Has Vector Mask"),
        "hasEffects" => tl!("Layer Has Effects"),
        _ => "?",
    }
}

/// The key the dialog's Function Key, Shift and Control make (empty for None).
fn chosen_key(fields: &Map<String, Value>) -> String {
    let n = fields.get("key").and_then(Value::as_u64).and_then(|n| u16::try_from(n).ok()).unwrap_or(0);
    actions_cmds::action_key(n, flag(fields, "shift"), flag(fields, "ctrl")).unwrap_or_default()
}

fn select_action(app: &mut PhotocraftApp, name: &str) {
    if let Some(i) = app.session.actions.list.iter().position(|a| a.name == name) {
        let a = &mut app.ui.actions;
        a.selected = Some(i);
        a.selected_set = None;
        a.clear_steps();
        a.expanded.resize(app.session.actions.list.len(), false);
        if let Some(e) = a.expanded.get_mut(i) {
            *e = true;
        }
    }
}

pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let name = text(fields, "name").trim().to_string();
    match kind(fields) {
        "newAction" => {
            let set = text(fields, "set");
            if !app.session.actions.sets().contains(&set) {
                app.run("actions.newSet", json!({"name": set}))?;
            }
            let r = app.run("actions.record", json!({"name": name, "set": set}))?;
            let key = chosen_key(fields);
            if !key.is_empty() {
                crate::actions::assign_shortcut(app, &name, &key)?;
            }
            let color = text(fields, "color");
            if !color.is_empty() {
                app.run("actions.setColor", json!({"action": name, "color": color}))?;
            }
            select_action(app, &name);
            Ok(r)
        }
        "newSet" => {
            let r = app.run("actions.newSet", json!({"name": name}))?;
            app.ui.actions.selected = None;
            app.ui.actions.clear_steps();
            app.ui.actions.selected_set = Some(name);
            Ok(r)
        }
        "setOptions" => {
            let set = text(fields, "set");
            let r = app.run("actions.renameSet", json!({"set": set, "name": name}))?;
            for c in app.ui.actions.collapsed_sets.iter_mut().filter(|c| **c == set) {
                *c = name.clone();
            }
            if app.ui.actions.selected_set.as_deref() == Some(set.as_str()) {
                app.ui.actions.selected_set = Some(name);
            }
            Ok(r)
        }
        "actionOptions" => {
            let old = text(fields, "action");
            if !app.session.actions.list.iter().any(|a| a.name == old) {
                return Err(tl!("The action no longer exists.").into());
            }
            if name != old {
                app.run("actions.rename", json!({"action": old, "name": name}))?;
            }
            let key = chosen_key(fields);
            // A key set with Set Shortcut… stays unless a function key replaces it.
            if !key.is_empty() || !flag(fields, "otherKey") {
                crate::actions::assign_shortcut(app, &name, &key)?;
            }
            app.run("actions.setColor", json!({"action": name, "color": text(fields, "color")}))
        }
        "playback" => {
            let seconds = fields.get("seconds").and_then(Value::as_f64).unwrap_or(1.0);
            let playback = match text(fields, "mode").as_str() {
                "stepByStep" => json!({"mode": "stepByStep"}),
                "pause" => json!({"mode": "pause", "seconds": seconds}),
                _ => json!({"mode": "accelerated"}),
            };
            app.run("actions.options", json!({"playback": playback}))
        }
        "menuItem" => {
            let item = text(fields, "item");
            if item.is_empty() {
                return Err(tl!("Pick a menu item first").into());
            }
            let action = fields.get("action").cloned().unwrap_or(Value::Null);
            let r = app.run("actions.insert", json!({"action": action, "at": fields.get("at"), "id": item}))?;
            // An inserted menu item shows its dialog when played, as in Photoshop.
            if crate::filter_dialog::has_dialog(&item) || crate::adjust_dialog::has_dialog(&item) {
                app.run("actions.dialog", json!({"action": action, "step": r["step"], "enabled": true}))?;
            }
            select_inserted(app, &action, &r);
            Ok(r)
        }
        "conditional" => {
            let branch = |k: &str| Some(text(fields, k)).filter(|v| !v.is_empty()).map_or(Value::Null, Value::from);
            let params = json!({"if": text(fields, "if"), "then": branch("then"), "else": branch("else")});
            let action = fields.get("action").cloned().unwrap_or(Value::Null);
            let r = app.run("actions.insert", json!({"action": action, "at": fields.get("at"), "id": "actions.conditional", "params": params}))?;
            select_inserted(app, &action, &r);
            Ok(r)
        }
        "delete" => delete(app, fields.get("target").unwrap_or(&Value::Null)),
        other => Err(format!("unknown Actions dialog `{other}`")),
    }
}

fn select_inserted(app: &mut PhotocraftApp, action: &Value, r: &Value) {
    if let (Some(a), Some(step)) = (action.as_u64().and_then(|a| usize::try_from(a).ok()), r["step"].as_u64().and_then(|s| usize::try_from(s).ok())) {
        app.ui.actions.selected = Some(a);
        app.ui.actions.select_step(step);
    }
}

#[cfg(test)]
mod tests;
