//! Actions panel: record and replay command sequences.
//!
//! The list and the recording flag live on the engine session (`actions.record` / `stop` /
//! `play` / `list` / `get` / `delete` / `move` / `rename`), so the panel, the CLI and MCP share them. This module
//! keeps only which row is selected and which rows are expanded.

use egui::{Align2, Color32, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use photocraft_engine::actions_cmds::{self, Action};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Which row is selected and which rows are expanded. The action list itself is
/// [`photocraft_engine::Session::actions`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionsUi {
    pub selected: Option<usize>,
    pub selected_step: Option<usize>,
    pub expanded: Vec<bool>,
    /// Set folders shown collapsed (Photoshop's set disclosure triangles).
    pub collapsed_sets: Vec<String>,
    /// Photoshop's Button Mode: each action is a coloured button, and a click plays it.
    #[serde(default)]
    pub button_mode: bool,
    /// The set folder selected (a click on its row), when no action is.
    pub selected_set: Option<String>,
    /// Steps picked together with Ctrl or Shift (with [`Self::selected_step`], the one picked
    /// last): Delete removes them all.
    pub selected_steps: std::collections::BTreeSet<usize>,
    /// Steps opened to show their values, as (action, step).
    #[serde(skip)]
    pub open_steps: std::collections::BTreeSet<(usize, usize)>,
    #[serde(skip)]
    reveal_recording: Option<(usize, usize)>,
    /// The action that just played, scrolled into view on the next frame.
    #[serde(skip)]
    reveal_action: Option<usize>,
    /// The action whose name is being edited in place (double-click on its name).
    #[serde(skip)]
    renaming: Option<RenameAction>,
}

impl ActionsUi {
    /// No step picked.
    pub fn clear_steps(&mut self) {
        self.selected_step = None;
        self.selected_steps.clear();
    }

    /// Only step `step` picked.
    pub fn select_step(&mut self, step: usize) {
        self.selected_step = Some(step);
        self.selected_steps = std::iter::once(step).collect();
    }

    /// The picked steps of the selected action, in order.
    pub fn step_selection(&self) -> Vec<usize> {
        let mut steps: std::collections::BTreeSet<usize> = self.selected_steps.clone();
        steps.extend(self.selected_step);
        if self.selected_step.is_none() {
            steps.clear();
        }
        steps.into_iter().collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct RenameAction {
    index: usize,
    text: String,
    focused: bool,
}

pub(crate) fn label_of(id: &str) -> String {
    photocraft_engine::commands::find(id)
        .map(|c| c.label)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        // Shell menu items recorded as steps, like Save a Copy (#4373).
        .or_else(|| crate::menu_catalog::CATALOG.iter().find(|c| c.3 == id).map(|c| c.1))
        .map(|label| label.trim_end_matches('…').to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The selected action, or the first one when nothing is selected.
pub fn selected_action(app: &PhotocraftApp) -> Option<&Action> {
    let list = &app.session.actions.list;
    let idx = app.ui.actions.selected.filter(|i| *i < list.len());
    idx.and_then(|i| list.get(i)).or_else(|| list.first())
}

/// Index of the playback target. Photoshop's Play button uses the first action when
/// the panel has actions but none of its rows is selected.
fn playback_selection(app: &PhotocraftApp) -> Option<usize> {
    let n = app.session.actions.list.len();
    if n == 0 { None } else { Some(app.ui.actions.selected.filter(|i| *i < n).unwrap_or(0)) }
}

/// Steps in the `[[id, params], …]` shape batch and droplets already accept, without the
/// steps switched off in the panel.
pub fn action_steps(action: &Action) -> Vec<Value> {
    action.enabled_steps().map(|(_, (id, p))| json!([id, p])).collect()
}

/// Named action bindings use the existing persisted shortcut overrides. Keeping the
/// name in the key means selecting or deleting another row cannot retarget F6.
pub(crate) const SHORTCUT_PREFIX: &str = actions_cmds::PLAY_SHORTCUT_PREFIX;

pub(crate) fn shortcut_id(name: &str) -> String {
    format!("{SHORTCUT_PREFIX}{name}")
}

/// The Set Shortcut dialog's marker (an action's context menu › Set Shortcut…): any key, with any
/// modifiers, plays the action, beyond Photoshop's F1–F15.
const KEY_MARK: &str = "__actionShortcut";

/// Open Set Shortcut for action `name`, listening for its key at once.
pub(crate) fn open_set_shortcut(app: &mut PhotocraftApp, name: &str) -> u64 {
    let current = app.session.prefs().shortcuts.get(&shortcut_id(name)).cloned().unwrap_or_default();
    let mut f = serde_json::Map::new();
    f.insert(KEY_MARK.into(), json!(true));
    f.insert("__label".into(), json!("Action Shortcut"));
    f.insert("action".into(), json!(name));
    f.insert("key".into(), json!(current));
    f.insert("message".into(), json!(""));
    app.ui.open_dialog(crate::state::DialogKind::Command, f)
}

pub(crate) fn owns_set_shortcut(f: &serde_json::Map<String, Value>) -> bool {
    f.contains_key(KEY_MARK)
}

/// The commands that already answer to `key` (their menu labels), other than action `name`.
fn key_holders(app: &PhotocraftApp, key: &str, name: &str) -> Vec<String> {
    let Some(sc) = crate::shortcuts::parse(key) else { return Vec::new() };
    let own = shortcut_id(name);
    let items = crate::menus::menu_items(app);
    crate::shortcut_dispatch::bindings(app)
        .into_iter()
        .filter(|(id, b)| *b == sc && *id != own)
        .map(|(id, _)| match id.strip_prefix(SHORTCUT_PREFIX) {
            Some(action) => action.to_string(),
            None => items.iter().find(|i| i.id == id).map_or(id, |i| tl!(i.label.as_str()).trim_end_matches('…').to_string()),
        })
        .collect()
}

/// The dialog: the key now set and the next key combination pressed, which replaces it. Esc
/// keeps it, Backspace or Delete clears it.
pub(crate) fn set_shortcut_body(app: &PhotocraftApp, ui: &mut Ui, f: &mut serde_json::Map<String, Value>) {
    let name = f.get("action").and_then(Value::as_str).unwrap_or_default().to_string();
    let pressed = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key, pressed: true, modifiers, .. } if !crate::prefs_ui::is_modifier_key(*key) => Some((*key, *modifiers)),
            _ => None,
        })
    });
    if let Some((key, m)) = pressed {
        ui.input_mut(|i| {
            i.consume_key(m, key);
        });
        let message = if matches!(key, egui::Key::Backspace | egui::Key::Delete) && m.is_none() {
            f.insert("key".into(), json!(""));
            tl!("No shortcut.").to_string()
        } else if key == egui::Key::Escape || key == egui::Key::Enter {
            String::new()
        } else {
            match crate::prefs_ui::shortcut_text(key, m) {
                Some(sc) if crate::prefs_ui::reserved_shortcut(&sc) => {
                    crate::i18n::fmt(tl!("{shortcut} is reserved by the system and can't be assigned."), &[("shortcut", &crate::shortcuts::pretty(&sc))])
                }
                Some(sc) => {
                    let holders = key_holders(app, &sc, &name);
                    f.insert("key".into(), json!(sc));
                    if holders.is_empty() {
                        String::new()
                    } else {
                        crate::i18n::fmt(
                            tl!("{shortcut} is also the shortcut of {commands}: it will play this action instead."),
                            &[("shortcut", &crate::shortcuts::pretty(&sc)), ("commands", &holders.join(", "))],
                        )
                    }
                }
                None => String::new(),
            }
        };
        f.insert("message".into(), json!(message));
    }
    let t = Tokens::get(ui.ctx());
    ui.label(crate::i18n::fmt(tl!("Press the keys that play “{action}”."), &[("action", &name)]));
    ui.add_space(8.0);
    let key = f.get("key").and_then(Value::as_str).unwrap_or_default();
    let shown = if key.is_empty() { tl!("None").to_string() } else { crate::shortcuts::pretty(key) };
    ui.horizontal(|ui| {
        ui.label(tl!("Shortcut:"));
        ui.label(egui::RichText::new(shown).strong());
    });
    ui.add_space(4.0);
    let message = f.get("message").and_then(Value::as_str).unwrap_or_default();
    let hint = if message.is_empty() { tl!("Any key works, with Ctrl, Alt and Shift. Backspace removes the shortcut.") } else { message };
    ui.label(egui::RichText::new(hint).color(t.text_dim));
}

/// OK: the action plays on the key now (none when cleared).
pub(crate) fn set_shortcut_confirm(app: &mut PhotocraftApp, f: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let name = f.get("action").and_then(Value::as_str).ok_or("no action")?.to_string();
    if !app.session.actions.list.iter().any(|a| a.name == name) {
        return Err(tl!("The action no longer exists.").into());
    }
    let key = f.get("key").and_then(Value::as_str).unwrap_or_default().to_string();
    assign_shortcut(app, &name, &key)
}

pub(crate) fn assign_shortcut(app: &mut PhotocraftApp, name: &str, shortcut: &str) -> Result<Value, String> {
    let id = shortcut_id(name);
    let mut set = serde_json::Map::new();
    // A function key belongs to only one action. Ordinary menu bindings remain
    // intact and become available again when the action binding is removed.
    for (other, assigned) in &app.session.prefs().shortcuts {
        if other.starts_with(SHORTCUT_PREFIX) && other != &id && assigned == shortcut && !shortcut.is_empty() {
            set.insert(other.clone(), json!(""));
        }
    }
    set.insert(id, json!(shortcut));
    app.run("edit.keyboardShortcuts", json!({"set": set, "allowUnknown": true, "removeConflicts": false}))
}

/// Replay edits synchronously, with view steps handled by the shell in the same order.
/// Engine steps keep the headless action semantics (no dialogs or background jobs).
pub(crate) fn play(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    play_owned(app, params, None)
}

/// `suspended` is the recording a paused part of the same call took ([`PausedPlayback::record`]); it comes
/// back, with that named call journaled, only when the whole call ends.
fn play_owned(app: &mut PhotocraftApp, params: &Value, suspended: Option<Value>) -> Result<Value, String> {
    let (action, from) = match actions_cmds::playback_plan(&app.session, params) {
        Ok(plan) => plan,
        Err(e) => {
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let limit = match actions_cmds::step_limit(params) {
        Ok(limit) => limit,
        Err(e) => {
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let top = app.session.actions.playback_stack.is_empty();
    let recording = match actions_cmds::begin_playback(&mut app.session, &action) {
        Ok(r) => r,
        Err(e) => {
            restore_recording(app, suspended.as_ref());
            return Err(e.to_string());
        }
    };
    let owner = recording
        .map(|(_, idx)| {
            let into = app.session.actions.list.get(idx).map(|a| a.name.clone());
            json!({"index": idx, "into": into, "action": action.name, "from": from, "steps": limit})
        })
        .or(suspended);
    let mut ran = 0u64;
    let mut failed = None;
    let mut paused = None;
    for (i, (id, p)) in action.enabled_steps().skip_while(|(i, _)| *i < from).take(limit.unwrap_or(usize::MAX)) {
        if !actions_cmds::consume_playback_step(&mut app.session) {
            failed = Some(json!({"step": i, "id": id, "error": format!("action playback exceeded the {}-step limit", actions_cmds::MAX_PLAYBACK_STEPS)}));
            break;
        }
        // The gate judges the step before its dialog opens: a denied step neither runs nor
        // pauses.
        if let Some(auth) = app.session.authorize
            && let Err(e) = auth(id, p)
        {
            failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
            break;
        }
        // The dialog toggle: show the step's dialog with its recorded values and continue
        // from the next step when it is confirmed (`resume`). Agents play headlessly.
        if action.dialogs.contains(&i) && !app.automation_input && (crate::filter_dialog::has_dialog(id) || crate::adjust_dialog::has_dialog(id)) {
            let label = label_of(id);
            let dialog = crate::dialogs::open_command_dialog(app, id, &label);
            // A step-limited playback (Ctrl+Play) plays only what is left of its steps after the dialog.
            let left = limit.map(|l| l.saturating_sub(usize::try_from(ran).unwrap_or(usize::MAX).saturating_add(1)));
            let token = app.paused_playbacks.pause(action.name.clone(), i + 1, left, app.session.authorize);
            if let Some(d) = app.ui.dialog_mut(dialog) {
                if let Some(o) = p.as_object() {
                    // `__` keys are the dialog's own markers (`params_of` drops them), not step values.
                    d.fields.extend(o.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())));
                }
                d.fields.insert(RESUME.into(), json!(token));
            }
            paused = Some(json!({"step": i, "id": id, "dialog": dialog}));
            break;
        }
        let target = app.session.active().map(|d| d.doc.id);
        let result = if id == "actions.play" {
            play(app, p)
        } else if actions_cmds::shell_save_command(id) {
            // Write synchronously, so the next step doesn't meet a document locked by a
            // background save.
            let background = std::mem::replace(&mut app.background_jobs, false);
            let r = app.replay_save(id, p);
            app.background_jobs = background;
            r
        } else if actions_cmds::shell_view_command(id) {
            app.sync_views();
            app.run(id, p.clone())
        } else {
            app.session.execute(id, p.clone()).map_err(|e| e.to_string())
        };
        match result {
            Ok(value) => {
                if let Some(error) = actions_cmds::nested_failure(id, &value).or_else(|| actions_cmds::save_target_closed(&app.session, id, target)) {
                    failed = Some(json!({"step": i, "id": id, "error": error}));
                    break;
                }
                if let Some(inner) = value.get("paused").filter(|p| p.is_object()) {
                    // A called action paused for a dialog; it resumes alone when confirmed.
                    paused = Some(inner.clone());
                    break;
                }
                ran += 1;
            }
            Err(error) => {
                failed = Some(json!({"step": i, "id": id, "error": error}));
                break;
            }
        }
    }
    actions_cmds::finish_playback(&mut app.session, None, &action, from, limit);
    match paused.as_ref().and_then(|p| p.get("dialog")).and_then(Value::as_u64).and_then(|d| app.ui.dialog_mut(d)) {
        // Paused for a dialog: the call goes on after it, so recording stays suspended.
        Some(d) if failed.is_none() => {
            let entry = d.fields.get(RESUME).and_then(Value::as_u64).and_then(|t| app.paused_playbacks.paused.get_mut(&t));
            match (owner, entry) {
                (Some(owner), Some(p)) => p.record = Some(owner),
                // No continuation to carry the recording: the call ends here.
                (owner, _) => restore_recording(app, owner.as_ref()),
            }
        }
        _ => restore_recording(app, owner.as_ref()),
    }
    if top {
        show_played(app, &action.name);
    }
    Ok(match (failed, paused) {
        (Some(f), _) => json!({"action": action.name, "ran": ran, "failed": f}),
        (None, Some(p)) => json!({"action": action.name, "ran": ran, "paused": p}),
        (None, None) => json!({"action": action.name, "ran": ran}),
    })
}

/// The action that just played is selected (its set opened and the row scrolled to), so the
/// Actions panel shows which one ran however it was started: Play, its key, Button Mode, Batch.
/// Steps picked in that action stay picked, so Play on a step can be repeated.
fn show_played(app: &mut PhotocraftApp, name: &str) {
    let list = &app.session.actions.list;
    let Some(i) = list.iter().position(|a| a.name == name) else { return };
    let a = &mut app.ui.actions;
    if a.selected != Some(i) {
        a.selected = Some(i);
        a.clear_steps();
    }
    a.selected_set = None;
    if let Some(set) = list.get(i).and_then(|x| x.set.as_ref()) {
        a.collapsed_sets.retain(|c| c != set);
    }
    a.reveal_action = Some(i);
}

/// Execute the actual menu command, not a synthetic keystroke. Fit is deferred
/// until canvas layout, so it uses the dimensions after all preceding action steps.
pub(crate) fn run_view(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.sync_views();
    let i = app.session.active_index().ok_or("no document")?;
    let v = app.ui.views.get_mut(i).ok_or("no document view")?;
    v.fill_pending = false;
    match id {
        "view.fitOnScreen" => {
            v.fit_pending = true;
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.zoomIn" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, 1, v.doc_size);
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.zoomOut" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, -1, v.doc_size);
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        "view.actualPixels" => {
            v.fit_pending = false;
            v.zoom = 1.0;
            v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        }
        _ => return Err(format!("unsupported view command: {id}")),
    }
    v.zoom_anchor = None;
    app.session.journal.push((id.into(), params));
    Ok(Value::Null)
}

/// The Record button: into the selected action, after its selected step (Photoshop records from
/// the step that is selected), or at its end. With no action selected, New Action… asks first.
fn begin_recording(app: &mut PhotocraftApp) {
    if app.session.actions.recording.is_some() {
        return;
    }
    let Some(action) = app.ui.actions.selected.filter(|i| *i < app.session.actions.list.len()) else {
        crate::actions_dialogs::new_action(app);
        return;
    };
    let mut params = json!({"action": action});
    if let Some(step) = app.ui.actions.selected_step {
        params["at"] = json!(step + 1);
    }
    if app.run("actions.record", params).is_err() {
        return;
    }
    let a = &mut app.ui.actions;
    a.expanded.resize(a.expanded.len().max(action + 1), false);
    a.expanded[action] = true;
}

pub(crate) fn report_play(app: &mut PhotocraftApp, v: &Value) {
    if let Some(paused) = v.get("paused").filter(|p| p.is_object()) {
        let step = paused.get("step").and_then(Value::as_u64).unwrap_or(0);
        let template = "Step {step} is waiting for its dialog";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string())]);
        app.ui.status_error = false;
        return;
    }
    let Some(failed) = v.get("failed").filter(|f| f.is_object()) else {
        let ran = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
        app.ui.status = crate::i18n::trn(crate::i18n::current(), ran, "Played {n} step", "Played {n} steps");
        app.ui.status_error = false;
        return;
    };
    let step = failed.get("step").and_then(Value::as_u64).unwrap_or(0);
    let id = failed.get("id").and_then(Value::as_str).unwrap_or("");
    let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
    let template = "Step {step} ({id}) failed: {error}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("step", &(step + 1).to_string()), ("id", id), ("error", error)]);
    app.ui.status_error = true;
}

fn report_error(app: &mut PhotocraftApp, e: String) {
    if e != "cancelled" {
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
        app.ui.status_error = true;
    }
}

/// The height of a set, action or step row, rows flush: as dense as Photoshop's list (it fits
/// about 9 rows where 24–26 pt rows with 6 pt gaps fitted 7, user report).
const ROW_H: f32 = 23.0;
/// A step's parameter line under an opened step.
const PARAM_H: f32 = 17.0;
/// An action's button in Button Mode (buttons keep their gaps).
const BUTTON_H: f32 = 26.0;
/// Photoshop's two toggle columns, then the tree: the check, the dialog toggle, the content.
const CHECK_X: f32 = 11.0;
const DIALOG_X: f32 = 31.0;
const TREE_X: f32 = 44.0;
/// Where the check column ends and the dialog column begins.
const CHECK_SPLIT: f32 = 21.0;

/// The steps the panel shows for `action`: its steps with the ones being recorded in place,
/// where they will land.
pub(crate) fn visible_steps(app: &PhotocraftApp, action: usize) -> Vec<(String, Value)> {
    let Some(a) = app.session.actions.list.get(action) else { return Vec::new() };
    let at = actions_cmds::pending_at(&app.session, action);
    let mut steps: Vec<(String, Value)> = a.steps.clone();
    let pending: Vec<_> = actions_cmds::pending_steps(&app.session, action).cloned().collect();
    steps.splice(at.min(steps.len())..at.min(steps.len()), pending);
    steps
}

/// "3 steps", or "2/3 steps" when some are switched off (Photoshop shows a red partial check).
fn step_count(steps: usize, disabled: usize) -> String {
    if disabled == 0 {
        crate::i18n::trn(crate::i18n::current(), steps as u64, "{n} step", "{n} steps")
    } else {
        let count = crate::i18n::trn(crate::i18n::current(), steps as u64, "{n} step", "{n} steps");
        format!("{}/{count}", steps.saturating_sub(disabled))
    }
}

fn button_fill(color: &str, t: &Tokens) -> Color32 {
    // Photoshop's Actions button colours. Empty uses the panel card so an uncoloured action stays readable.
    match color {
        "red" => Color32::from_rgb(196, 64, 64),
        "orange" => Color32::from_rgb(204, 120, 40),
        "yellow" => Color32::from_rgb(196, 176, 48),
        "green" => Color32::from_rgb(56, 140, 72),
        "blue" => Color32::from_rgb(48, 104, 176),
        "violet" => Color32::from_rgb(120, 72, 168),
        "gray" => t.text_dim,
        _ => t.card,
    }
}

fn button_label_color(color: &str, t: &Tokens) -> Color32 {
    if color.is_empty() { t.text } else { Color32::from_rgb(20, 20, 20) }
}

/// A toggle column's state: everything below on, some (Photoshop draws it red), or none.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tri {
    On,
    Partial,
    Off,
}

impl Tri {
    fn of(on: usize, of: usize) -> Tri {
        if on == of {
            Tri::On
        } else if on == 0 {
            Tri::Off
        } else {
            Tri::Partial
        }
    }

    fn merge(self, other: Tri) -> Tri {
        if self == other { self } else { Tri::Partial }
    }
}

const PARTIAL: Color32 = Color32::from_rgb(220, 60, 50);

/// Photoshop's check column: a check mark when the item plays, red when only some of it does,
/// nothing when it is switched off.
fn paint_check(ui: &egui::Ui, rect: Rect, state: Tri, t: &Tokens) {
    let b = Rect::from_center_size(rect.center(), vec2(11.0, 11.0));
    let color = match state {
        Tri::On => t.text,
        Tri::Partial => PARTIAL,
        Tri::Off => return,
    };
    let pts = vec![pos2(b.left() + 1.5, b.center().y), pos2(b.left() + 4.5, b.bottom() - 2.0), pos2(b.right() - 1.0, b.top() + 1.5)];
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.6, color)));
}

/// The dialog column: `None` for an item with no dialog to show.
fn paint_dialog(ui: &egui::Ui, rect: Rect, state: Option<Tri>, t: &Tokens) {
    let color = match state {
        None => return,
        Some(Tri::On) => t.icon,
        Some(Tri::Partial) => PARTIAL,
        Some(Tri::Off) => t.text_faint.gamma_multiply(0.35),
    };
    crate::icons::paint(ui, rect, "message-square", 11.0, color);
}

fn paint_triangle(ui: &egui::Ui, center: egui::Pos2, open: bool, t: &Tokens) {
    let tri = Rect::from_center_size(center, vec2(8.0, 8.0));
    let pts = if open { vec![tri.left_top(), tri.right_top(), tri.center_bottom()] } else { vec![tri.left_top(), tri.right_center(), tri.left_bottom()] };
    ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
}

/// Whether playback can show `id`'s dialog (the dialog column has a toggle for it).
fn has_dialog(id: &str) -> bool {
    crate::filter_dialog::has_dialog(id) || crate::adjust_dialog::has_dialog(id)
}

fn step_label(id: &str, params: &Value) -> String {
    if id == "actions.play" {
        let target = params.get("action").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        format!("{}: {target}", label_of(id))
    } else if id == "layer.setProps" && params.get("layer").is_none() && params.get("layers").is_none() {
        // Photoshop's name for a recorded rename, opacity or blend change of the current layer.
        tl!("Set Current Layer").to_string()
    } else if id == "layer.select" {
        let target = params
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| params.get("layer").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| params.get("layer").and_then(Value::as_u64).map(|id| format!("ID {id}")));
        target.map_or_else(|| label_of(id), |target| format!("{} ({target})", label_of(id)))
    } else if id == "actions.conditional" {
        // "If hasLayers: FS8-Layers / FS8-No-Layers"
        let cond = params.get("if").and_then(Value::as_str).unwrap_or("?");
        let branch = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("–").to_string();
        format!("{}: {cond} → {} / {}", label_of(id), branch("then"), branch("else"))
    } else if id == photocraft_engine::atn_cmds::STOP_MESSAGE {
        format!("{}: {}", label_of(id), params.get("message").and_then(Value::as_str).unwrap_or_default())
    } else if id == photocraft_engine::atn_cmds::PHOTOSHOP_STEP {
        // A step from a .atn file PhotoCraft can't play yet: its Photoshop name, marked.
        let name = params.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or("?");
        crate::i18n::fmt(tl!("{name} (Photoshop)"), &[("name", name)])
    } else {
        label_of(id)
    }
}

/// A step's recorded values as Photoshop lists them under an opened step: "Name: Layer 1".
fn param_lines(params: &Value) -> Vec<String> {
    let Some(o) = params.as_object() else { return Vec::new() };
    o.iter()
        .filter(|(k, _)| !k.starts_with("__") && k.as_str() != "coalesce")
        .map(|(k, v)| {
            use photocraft_engine::layer_refs::{Described, describe};
            let layer = |d: Described| match d {
                Described::Current => tl!("Current Layer").to_string(),
                Described::Named(name) => name,
            };
            // Recorded layers: their names, or the current layer.
            let layers = v.as_array().and_then(|a| a.iter().map(describe).collect::<Option<Vec<_>>>()).filter(|l| !l.is_empty());
            let value = match (v, describe(v), layers) {
                (_, Some(d), _) => layer(d),
                (_, _, Some(l)) => l.into_iter().map(layer).collect::<Vec<_>>().join(", "),
                (Value::String(s), ..) => s.clone(),
                (Value::Number(n), ..) => n.as_f64().filter(|f| f.fract() != 0.0).map_or_else(|| n.to_string(), |f| format!("{f:.2}").trim_end_matches('0').to_string()),
                (other, ..) => other.to_string(),
            };
            let value: String = if value.chars().count() > 60 { value.chars().take(59).chain(std::iter::once('…')).collect() } else { value };
            format!("{}: {value}", words(k))
        })
        .collect()
}

/// `blendMode` → "Blend Mode".
fn words(key: &str) -> String {
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.push(c);
        } else if c == '_' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Command params for a picked file: a real path keeps the journal small; browser files arrive
/// as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Load a `.atn` file's bytes, or Photoshop's whole Actions palette, into the Actions panel (the
/// panel menu's Load Actions… and Import Photoshop Actions).
pub fn load_bytes(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let r = app.run("actions.load", source(name, bytes))?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let unsupported = r["unsupported"].as_array().map_or(0, Vec::len);
    let shown = crate::file_open::display_name(name);
    let loaded = match r["sets"].as_array().map(Vec::len) {
        Some(sets) => crate::i18n::fmt(
            &crate::i18n::trn(crate::i18n::current(), n, "Loaded {n} action in {sets} sets from {file}", "Loaded {n} actions in {sets} sets from {file}"),
            &[("sets", &sets.to_string()), ("file", &shown)],
        ),
        None => crate::i18n::fmt(
            &crate::i18n::trn(crate::i18n::current(), n, "Loaded {n} action from {file}", "Loaded {n} actions from {file}"),
            &[("file", &shown)],
        ),
    };
    app.ui.status_error = false;
    app.ui.status = if unsupported > 0 {
        let note = crate::i18n::trn(
            crate::i18n::current(),
            unsupported as u64,
            "{n} step has no PhotoCraft equivalent yet",
            "{n} steps have no PhotoCraft equivalent yet",
        );
        let template = "{message} ({note})";
        crate::i18n::fmt(tl!(template), &[("message", &loaded), ("note", &note)])
    } else {
        loaded
    };
    // Show what arrived: select the first loaded action, expanded.
    let list = &app.session.actions.list;
    if let Some(first) = r["actions"][0].as_str()
        && let Some(i) = list.iter().position(|a| a.name == first)
    {
        app.ui.actions.selected = Some(i);
        app.ui.actions.selected_set = None;
        app.ui.actions.clear_steps();
        app.ui.actions.expanded.resize(list.len(), false);
        if let Some(e) = app.ui.actions.expanded.get_mut(i) {
            *e = true;
        }
    }
    app.ui.panels.history = true;
    Ok(r)
}

/// Load Actions… adds the file's sets; Replace Actions… (`replace`) clears the panel first.
fn load_dialog(app: &mut PhotocraftApp, replace: bool) -> Result<Value, String> {
    // A `.psp` is Photoshop's Actions palette (`Actions Palette.psp`): every set at once.
    app.pick_file_bytes_filtered(&["atn", "psp"], move |app, name, bytes| {
        if replace {
            app.run("actions.clear", json!({}))?;
            app.ui.actions = ActionsUi { button_mode: app.ui.actions.button_mode, ..Default::default() };
        }
        load_bytes(app, &name, &bytes)
    })
}

/// The panel menu's Import Photoshop Actions: every set of the Actions panel of the newest
/// Photoshop on this computer (its `Actions Palette.psp`), replacing PhotoCraft's sets of the
/// same names, so loading again after changing them in Photoshop updates them.
pub fn import_photoshop(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some((path, bytes)) = app.services.photoshop_actions.as_mut().and_then(|f| f()) else {
        return Err(tl!("No Photoshop Actions palette was found on this computer").to_string());
    };
    load_bytes(app, &path, &bytes)
}

/// Save Actions…: the selected set (every action of it), or every action when none came from a
/// set, as a `.atn` file Photoshop loads.
fn save_dialog(app: &mut PhotocraftApp, set: Option<String>) -> Result<Value, String> {
    let set = set.or_else(|| app.ui.actions.selected_set.clone()).or_else(|| selected_action(app).and_then(|a| a.set.clone()));
    let params = match &set {
        Some(set) => json!({"set": set, "skipUnsupported": true}),
        None => json!({"skipUnsupported": true}),
    };
    let r = app.run("actions.save", params)?;
    let bytes = r["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode).ok_or("the export returned no data")?;
    let n = r["actions"].as_array().map_or(0, Vec::len) as u64;
    let skipped = r["skipped"].as_array().map_or(0, Vec::len);
    let file = format!("{}.atn", r["set"].as_str().unwrap_or("Actions"));
    app.pick_save(&file, move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        let shown = crate::file_open::display_name(&path);
        let saved =
            crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, "Saved {n} action to {file}", "Saved {n} actions to {file}"), &[("file", &shown)]);
        app.ui.status_error = false;
        app.ui.status = if skipped > 0 {
            let note = crate::i18n::trn(
                crate::i18n::current(),
                skipped as u64,
                "{n} step with no Photoshop equivalent left out",
                "{n} steps with no Photoshop equivalent left out",
            );
            let template = "{message} ({note})";
            crate::i18n::fmt(tl!(template), &[("message", &saved), ("note", &note)])
        } else {
            saved
        };
        Ok(json!({"path": path, "actions": n, "skipped": skipped}))
    })
}

const INSERT_STOP: &str = "__insertStop";

pub fn owns_insert_stop(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(INSERT_STOP)
}

/// Insert Stop…: a message step after the selected step (or at the end of the selected action).
fn insert_stop_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let action = app.ui.actions.selected.filter(|i| *i < app.session.actions.list.len()).ok_or_else(|| tl!("Select an action first").to_string())?;
    let len = visible_steps(app, action).len();
    let at = app.ui.actions.selected_step.map_or(len, |s| (s + 1).min(len));
    let fields =
        json!({INSERT_STOP: true, "__command": "actions.insert", "__label": "Insert Stop", "action": action, "at": at, "message": "", "allowContinue": false});
    let id = app.ui.open_dialog(crate::state::DialogKind::Command, fields.as_object().cloned().unwrap_or_default());
    Ok(json!({"dialog": id}))
}

pub fn insert_stop_body(ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    ui.label(tl!("Message:"));
    let mut message = fields.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
    ui.add(egui::TextEdit::multiline(&mut message).desired_rows(4).desired_width(320.0));
    fields.insert("message".into(), json!(message));
    ui.add_space(6.0);
    let mut allow = fields.get("allowContinue").and_then(Value::as_bool).unwrap_or(false);
    ui.checkbox(&mut allow, tl!("Allow Continue"));
    fields.insert("allowContinue".into(), json!(allow));
}

pub fn insert_stop_confirm(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let action = fields.get("action").and_then(Value::as_u64).ok_or("the dialog names no action")?;
    let params = json!({
        "action": action,
        "at": fields.get("at").cloned().unwrap_or(Value::Null),
        "id": photocraft_engine::atn_cmds::STOP_MESSAGE,
        "params": {"message": fields.get("message").cloned().unwrap_or(json!("")), "allowContinue": fields.get("allowContinue").cloned().unwrap_or(json!(false))},
    });
    let r = app.run("actions.insert", params)?;
    if let (Some(a), Some(step)) = (usize::try_from(action).ok(), r["step"].as_u64().and_then(|s| usize::try_from(s).ok())) {
        app.ui.actions.selected = Some(a);
        app.ui.actions.select_step(step);
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if let Some(e) = app.ui.actions.expanded.get_mut(a) {
            *e = true;
        }
    }
    Ok(r)
}

/// The key a paused playback leaves in its dialog's fields: the token of its [`PausedPlayback`].
pub const RESUME: &str = "__resumeAction";
/// The key Record Again… leaves in a step's dialog: `{action, step}`.
const RERECORD: &str = "__recordAgain";

/// Resume the recording a paused call suspended and journal that call, once.
fn restore_recording(app: &mut PhotocraftApp, owner: Option<&Value>) {
    let Some(owner) = owner else { return };
    let (Some(idx), Some(into), Some(name)) = (
        owner.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()),
        owner.get("into").and_then(Value::as_str),
        owner.get("action").and_then(Value::as_str),
    ) else {
        return;
    };
    let from = owner.get("from").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let steps = owner.get("steps").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    // `suspended_guard` refuses what would move the recorded action, but another command may
    // still replace the list: the call goes to the action of that name, never to whatever now
    // sits at the old index, and is dropped if that action is gone.
    let list = &app.session.actions.list;
    let target = if list.get(idx).is_some_and(|a| a.name == into) { Some(idx) } else { list.iter().position(|a| a.name == into) };
    if let Some(idx) = target
        && app.session.actions.recording.is_none()
    {
        actions_cmds::restore_recording(&mut app.session, idx, name, from, steps);
    }
}

/// Whether a paused playback holds a suspended recording.
fn suspended(app: &PhotocraftApp) -> bool {
    app.paused_playbacks.paused.values().any(|p| p.record.is_some())
}

/// While a step dialog holds a suspended recording, refuse what would lose its call: a new
/// recording (the restore would find recording taken) or deleting/moving actions (the recorded
/// action could go away or change place). The engine checks these against `recording`, which
/// is `None` then.
pub(crate) fn suspended_guard(app: &PhotocraftApp, id: &str) -> Result<(), String> {
    if matches!(id, "actions.record" | "actions.delete" | "actions.move") && suspended(app) {
        return Err(tl!("Confirm or cancel the open action step dialog first").into());
    }
    Ok(())
}

/// A step dialog's command failed: the action ends there.
pub fn abandoned(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    let record = fields.get(RESUME).and_then(|t| app.paused_playbacks.take(t)).and_then(|p| p.record);
    restore_recording(app, record.as_ref());
}

/// The automation gate a session installs ([`photocraft_engine::Session::authorize`]).
type Gate = fn(&str, &Value) -> photocraft_engine::Result<()>;

/// A playback paused at a step dialog: the action and step it continues from, and the gate it
/// was playing under (`None` for one a person started).
#[derive(Clone, Debug)]
pub(crate) struct PausedPlayback {
    action: String,
    from: usize,
    /// The steps still to play, for a step-limited playback.
    steps: Option<usize>,
    gate: Option<Gate>,
    /// The recording the paused call suspended: `{index, into, action, from, steps}` (`into` is the
    /// name of the action being recorded). A step dialog only closes through
    /// `dialogs::confirm` (OK, `↩`, automation) or `dialogs::cancel` (Cancel, Esc, the close
    /// box, automation), which end the call through `resume`, `abandoned` or `stopped`, so the
    /// recording comes back exactly once.
    record: Option<Value>,
}

/// The paused playbacks, by the token their dialog carries. The dialog's fields hold only the
/// token; what plays next, and under which gate, is kept here.
#[derive(Debug, Default)]
pub(crate) struct PausedPlaybacks {
    last: u64,
    paused: std::collections::HashMap<u64, PausedPlayback>,
}

impl PausedPlaybacks {
    fn pause(&mut self, action: String, from: usize, steps: Option<usize>, gate: Option<Gate>) -> u64 {
        self.last = self.last.wrapping_add(1);
        self.paused.insert(self.last, PausedPlayback { action, from, steps, gate, record: None });
        self.last
    }

    fn take(&mut self, token: &Value) -> Option<PausedPlayback> {
        token.as_u64().and_then(|token| self.paused.remove(&token))
    }
}

/// A step dialog confirmed: continue the action after it (`dialogs::confirm` calls this).
///
/// The remaining steps run under the gate the playback paused with, however the dialog was
/// confirmed: a control request, synthetic Enter or OK, or a click.
pub fn resume(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let Some(token) = fields.get(RESUME) else { return Ok(()) };
    let Some(paused) = app.paused_playbacks.take(token) else {
        return Err("this dialog doesn't belong to a paused action".into());
    };
    let owner = paused.record.clone();
    // The dialog's command may run as a background job; the next step needs its result.
    if let Some(doc) = app.session.active().map(|d| d.doc.id)
        && let Some(job) = app.session.job_on(doc)
        && let Err(e) = app.session.wait_job(job.id)
    {
        restore_recording(app, owner.as_ref());
        let template = "Action playback failed: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e.to_string())]);
        app.ui.status_error = true;
        return Err(e.to_string());
    }
    let mut params = json!({"action": paused.action, "from": paused.from});
    if let Some(steps) = paused.steps {
        params["steps"] = json!(steps);
    }
    let saved = app.session.authorize;
    app.session.authorize = paused.gate.or(saved);
    let played = play_owned(app, &params, owner);
    app.session.authorize = saved;
    match played {
        Ok(v) => {
            report_play(app, &v);
            Ok(())
        }
        Err(e) => {
            let template = "Action playback failed: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
            app.ui.status_error = true;
            Err(e)
        }
    }
}

/// A step dialog cancelled: the action stops there, as in Photoshop.
pub fn stopped(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>) {
    if fields.contains_key(RESUME) {
        abandoned(app, fields);
        app.ui.status = tl!("Action playback stopped").to_string();
        app.ui.status_error = false;
    }
}

/// Record Again…: the selected step's dialog opens with its recorded values; OK runs it and
/// stores the new values in the step (`rerecorded`).
fn record_again(app: &mut PhotocraftApp, action: usize, step: usize) -> Result<Value, String> {
    if app.session.actions.recording.is_some() {
        return Err(tl!("Stop recording first").into());
    }
    let (id, params) = visible_steps(app, action).get(step).cloned().ok_or_else(|| tl!("Select a step first").to_string())?;
    if !has_dialog(&id) {
        return Err(tl!("This step has no dialog to record again").into());
    }
    let name = app.session.actions.list.get(action).map(|a| a.name.clone()).unwrap_or_default();
    let dialog = crate::dialogs::open_command_dialog(app, &id, &label_of(&id));
    if let Some(d) = app.ui.dialog_mut(dialog) {
        if let Some(o) = params.as_object() {
            d.fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        d.fields.insert(RERECORD.into(), json!({"action": name, "step": step}));
    }
    Ok(json!({"dialog": dialog}))
}

/// Record Again's dialog confirmed with `params`: they become the step's values.
pub fn rerecorded(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, params: &Value) {
    let Some(r) = fields.get(RERECORD) else { return };
    if let Err(e) = app.run("actions.setStep", json!({"action": r.get("action"), "step": r.get("step"), "params": params})) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A playback the panel paces over frames (Playback Options › Step by Step or Pause For): the
/// actions still to play, the next step, and when it is due.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    queue: Vec<String>,
    next: usize,
    limit: Option<usize>,
    ran: u64,
    due: f64,
}

/// Play `names` in turn (a set plays all its actions), the first from step `from`: at once, or
/// paced over frames as Playback Options ask. Recording plays at once, so the recorded call is
/// one step.
pub(crate) fn start(app: &mut PhotocraftApp, names: Vec<String>, from: usize, limit: Option<usize>) {
    let delay = app.session.actions.prefs.playback.delay();
    if delay.is_none() || app.session.actions.recording.is_some() || app.automation_input {
        let mut from = from;
        for name in names {
            let mut params = json!({"action": name, "from": from});
            if let Some(limit) = limit {
                params["steps"] = json!(limit);
            }
            from = 0;
            match app.run("actions.play", params) {
                Ok(v) => {
                    report_play(app, &v);
                    if v.get("failed").is_some() || v.get("paused").is_some() {
                        return;
                    }
                }
                Err(e) => return report_error(app, e),
            }
        }
        return;
    }
    app.action_run = Some(Run { queue: names, next: from, limit, ran: 0, due: 0.0 });
}

/// Each frame: a paced playback plays its next step when it is due, with the step selected so
/// the panel shows where it is.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut run) = app.action_run.take() else { return };
    let now = ctx.input(|i| i.time);
    if now < run.due {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(run.due - now));
        app.action_run = Some(run);
        return;
    }
    let Some(name) = run.queue.first().cloned() else { return };
    let Some(index) = app.session.actions.list.iter().position(|a| a.name == name) else {
        app.ui.status = tl!("Action playback stopped").to_string();
        return;
    };
    let action = &app.session.actions.list[index];
    let next = action.enabled_steps().map(|(i, _)| i).find(|i| *i >= run.next);
    let Some(step) = next.filter(|_| run.limit.is_none_or(|l| run.ran < l as u64)) else {
        // This action is done: the next one in the queue, or the end.
        run.queue.remove(0);
        if run.queue.is_empty() {
            report_play(app, &json!({"action": name, "ran": run.ran}));
        } else {
            run.next = 0;
            app.action_run = Some(run);
            ctx.request_repaint();
        }
        return;
    };
    let a = &mut app.ui.actions;
    a.selected = Some(index);
    a.select_step(step);
    match app.run("actions.play", json!({"action": name, "from": step, "steps": 1})) {
        Ok(v) if v.get("failed").is_some() || v.get("paused").is_some() => report_play(app, &v),
        Ok(v) => {
            run.ran += v.get("ran").and_then(Value::as_u64).unwrap_or(0);
            run.next = step + 1;
            // Played step by step, the document redraws before the next step.
            run.due = now + app.session.actions.prefs.playback.delay().unwrap_or(0.0);
            app.ui.actions.selected = Some(index);
            app.ui.actions.select_step(step);
            app.action_run = Some(run);
            ctx.request_repaint();
        }
        Err(e) => report_error(app, e),
    }
}

/// Stop: ends recording, or a paced playback.
fn stop(app: &mut PhotocraftApp) {
    if app.session.actions.recording.is_some() {
        let _ = app.run("actions.stop", json!({}));
    } else if app.action_run.take().is_some() {
        app.ui.status = tl!("Action playback stopped").to_string();
        app.ui.status_error = false;
    }
}

/// The Play button: the selected set's actions, or the selected action from its selected step;
/// with Ctrl, only that step (Photoshop). Nothing selected plays the first action.
fn play_selection(app: &mut PhotocraftApp, only_step: bool) {
    if app.action_run.is_some() {
        return;
    }
    if let Some(set) = app.ui.actions.selected_set.clone().filter(|_| app.ui.actions.selected.is_none()) {
        let names: Vec<String> = app.session.actions.list.iter().filter(|a| a.set.as_ref() == Some(&set)).map(|a| a.name.clone()).collect();
        return start(app, names, 0, None);
    }
    let Some(i) = playback_selection(app) else { return };
    let name = app.session.actions.list[i].name.clone();
    let from = if app.ui.actions.selected == Some(i) { app.ui.actions.selected_step.unwrap_or(0) } else { 0 };
    start(app, vec![name], from, (only_step && app.ui.actions.selected_step.is_some()).then_some(1));
}

/// The Actions entries of the panel group's menu ("…"), in Photoshop's order.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let recording = app.session.actions.recording.is_some();
    let list_len = app.session.actions.list.len();
    let action = app.ui.actions.selected.filter(|i| *i < list_len);
    let step = action.and(app.ui.actions.selected_step);
    let set = app.ui.actions.selected_set.clone().filter(|_| action.is_none());
    let mut act: Option<&str> = None;
    let entry = |ui: &mut egui::Ui, enabled: bool, label: &str| ui.add_enabled(enabled, egui::Button::new(label)).clicked();
    let mut button_mode = app.ui.actions.button_mode;
    if ui.add_enabled(!recording, egui::Checkbox::new(&mut button_mode, tl!("Button Mode"))).clicked() {
        act = Some("buttons");
    }
    ui.separator();
    if entry(ui, !recording, tl!("New Action…")) {
        act = Some("newAction");
    }
    if entry(ui, !recording, tl!("New Set…")) {
        act = Some("newSet");
    }
    if entry(ui, action.is_some() || set.is_some(), tl!("Duplicate")) {
        act = Some("duplicate");
    }
    if entry(ui, action.is_some() || set.is_some(), tl!("Delete")) {
        act = Some("delete");
    }
    if entry(ui, list_len > 0, tl!("Play")) {
        act = Some("play");
    }
    ui.separator();
    if entry(ui, action.is_some() && !recording, tl!("Start Recording")) {
        act = Some("record");
    }
    let again = step.is_some_and(|s| visible_steps(app, action.unwrap_or(0)).get(s).is_some_and(|(id, _)| has_dialog(id)));
    if entry(ui, again && !recording, tl!("Record Again…")) {
        act = Some("again");
    }
    if entry(ui, action.is_some(), tl!("Insert Menu Item…")) {
        act = Some("menuItem");
    }
    if entry(ui, action.is_some(), tl!("Insert Stop…")) {
        act = Some("stop");
    }
    if entry(ui, action.is_some(), tl!("Insert Conditional…")) {
        act = Some("conditional");
    }
    ui.separator();
    let options = if set.is_some() { tl!("Set Options…") } else { tl!("Action Options…") };
    if entry(ui, action.is_some() || set.is_some(), options) {
        act = Some("options");
    }
    if entry(ui, true, tl!("Playback Options…")) {
        act = Some("playback");
    }
    let mut tools = app.session.actions.prefs.allow_tool_recording;
    if ui.checkbox(&mut tools, tl!("Allow Tool Recording")).on_hover_text(tl!("Record brush and other painting strokes, and the brush settings they use")).clicked() {
        act = Some("tools");
    }
    ui.separator();
    if entry(ui, list_len > 0 && !recording, tl!("Clear All Actions")) {
        act = Some("clear");
    }
    if entry(ui, !recording, tl!("Load Actions…")) {
        act = Some("load");
    }
    if entry(ui, !recording, tl!("Replace Actions…")) {
        act = Some("replace");
    }
    if entry(ui, list_len > 0, tl!("Save Actions…")) {
        act = Some("save");
    }
    if app.services.photoshop_actions.is_some() && entry(ui, !recording, tl!("Import Photoshop Actions")) {
        act = Some("photoshop");
    }
    let Some(a) = act else { return };
    ui.close();
    let r: Result<Value, String> = match a {
        "buttons" => {
            app.ui.actions.button_mode = !app.ui.actions.button_mode;
            Ok(Value::Null)
        }
        "newAction" => Ok(json!(crate::actions_dialogs::new_action(app))),
        "newSet" => Ok(json!(crate::actions_dialogs::new_set(app))),
        "duplicate" => match (&set, action) {
            (Some(set), _) => app.run("actions.duplicate", json!({"set": set})),
            (None, Some(i)) => app.run("actions.duplicate", json!({"action": i})),
            _ => Ok(Value::Null),
        },
        "delete" => {
            delete_selection(app, false);
            Ok(Value::Null)
        }
        "play" => {
            play_selection(app, false);
            Ok(Value::Null)
        }
        "record" => {
            begin_recording(app);
            Ok(Value::Null)
        }
        "again" => match (action, step) {
            (Some(a), Some(s)) => record_again(app, a, s),
            _ => Ok(Value::Null),
        },
        "menuItem" => crate::actions_dialogs::insert_menu_item(app).map(Value::from),
        "stop" => insert_stop_dialog(app),
        "conditional" => crate::actions_dialogs::insert_conditional(app).map(Value::from),
        "options" => match (&set, action) {
            (Some(set), _) => Ok(json!(crate::actions_dialogs::set_options(app, set))),
            (None, Some(i)) => Ok(json!(crate::actions_dialogs::action_options(app, i))),
            _ => Ok(Value::Null),
        },
        "playback" => Ok(json!(crate::actions_dialogs::playback_options(app))),
        "tools" => app.run("actions.options", json!({"allowToolRecording": tools})),
        "clear" => {
            let r = app.run("actions.clear", json!({}));
            if r.is_ok() {
                app.ui.actions = ActionsUi { button_mode: app.ui.actions.button_mode, ..Default::default() };
            }
            r
        }
        "load" => load_dialog(app, false),
        "replace" => load_dialog(app, true),
        "photoshop" => import_photoshop(app),
        _ => save_dialog(app, None),
    };
    if let Err(e) = r
        && e != "cancelled"
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[derive(Clone, Copy)]
struct DragRow {
    action: usize,
    step: Option<usize>,
}

/// Drops use an insertion line, converted to the final index expected by actions.move.
fn drag_row(ui: &egui::Ui, response: &egui::Response, row: DragRow, t: &Tokens) -> Option<(DragRow, usize)> {
    response.dnd_set_drag_payload(row);
    let source = response.dnd_hover_payload::<DragRow>()?;
    if source.step.is_some() != row.step.is_some() || (source.step.is_some() && source.action != row.action) {
        return None;
    }
    let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= response.rect.center().y));
    let index = row.step.unwrap_or(row.action);
    let from = source.step.unwrap_or(source.action);
    let slot = index.saturating_add(usize::from(after));
    let to = if from < slot { slot.saturating_sub(1) } else { slot };
    let y = if after { response.rect.bottom() } else { response.rect.top() };
    ui.painter().line_segment([pos2(response.rect.left() + TREE_X, y), pos2(response.rect.right(), y)], Stroke::new(2.0, t.accent));
    response.dnd_release_payload::<DragRow>().map(|source| (*source, to))
}

fn move_row(app: &mut PhotocraftApp, source: DragRow, to: usize) {
    let mut params = json!({"action": source.action, "to": to});
    if let Some(step) = source.step {
        params["step"] = json!(step);
    }
    if app.run("actions.move", params).is_err() {
        return;
    }
    if source.step.is_some() {
        app.ui.actions.selected = Some(source.action);
        app.ui.actions.select_step(to);
    } else {
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if source.action < app.ui.actions.expanded.len() && to < app.ui.actions.expanded.len() {
            let expanded = app.ui.actions.expanded.remove(source.action);
            app.ui.actions.expanded.insert(to, expanded);
        }
        app.ui.actions.selected = Some(to);
        app.ui.actions.clear_steps();
        // Dropped among another set's actions: the action joins that set (the row above wins).
        let list = &app.session.actions.list;
        let neighbour = to.checked_sub(1).and_then(|j| list.get(j)).or_else(|| list.get(to + 1)).and_then(|a| a.set.clone());
        if list.get(to).is_some_and(|a| a.set != neighbour) {
            let _ = app.run("actions.assignSet", json!({"action": to, "set": neighbour}));
        }
    }
}

/// The trash button: the selected steps, action or set, after Photoshop's question (Alt
/// deletes at once).
fn delete_selection(app: &mut PhotocraftApp, at_once: bool) {
    use crate::actions_dialogs::Target;
    let a = &app.ui.actions;
    let target = match (a.selected, a.selected_set.clone()) {
        (Some(i), _) => {
            let steps = a.step_selection();
            if steps.is_empty() { Target::Action(i) } else { Target::Steps(i, steps) }
        }
        (None, Some(set)) => Target::Set(set),
        (None, None) => return,
    };
    if at_once {
        let params = match &target {
            Target::Set(set) => json!({"set": set}),
            Target::Action(i) => json!({"action": app.session.actions.list.get(*i).map(|a| a.name.clone())}),
            Target::Steps(i, steps) => json!({"action": i, "steps": steps}),
        };
        if let Err(e) = crate::actions_dialogs::delete(app, &params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    } else {
        crate::actions_dialogs::confirm_delete(app, target);
    }
}

/// The in-place name field of the action being renamed. Enter, Tab or clicking elsewhere
/// returns the new name to commit; Escape cancels and keeps the old name.
fn rename_field(app: &mut PhotocraftApp, ui: &mut egui::Ui, index: usize, rect: Rect) -> Option<String> {
    let mut r = app.ui.actions.renaming.take().filter(|r| r.index == index)?;
    let id = egui::Id::new(("action-rename-field", index));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).id(id).font(egui::FontId::proportional(12.0)));
    if !r.focused {
        te.request_focus();
        // Like Photoshop, the whole name starts selected so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        r.focused = true;
        app.ui.actions.renaming = Some(r);
        return None;
    }
    let (enter, tab, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        Some(r.text)
    } else {
        app.ui.actions.renaming = Some(r);
        None
    }
}

fn rename_action(app: &mut PhotocraftApp, index: usize, name: String) {
    if let Err(e) = app.run("actions.rename", json!({"action": index, "name": name})) {
        let template = "Could not rename the action: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
        app.ui.status_error = true;
    }
}

/// One action's row data for a frame. Only an opened action's steps are copied: a whole
/// imported Photoshop palette (well over a thousand steps, some with embedded tables) would
/// otherwise be copied every frame.
struct Row {
    name: String,
    /// How many steps it has, those being recorded included.
    count: usize,
    /// Its steps, when they are shown.
    steps: Vec<(String, Value)>,
    disabled: std::collections::BTreeSet<usize>,
    dialogs: std::collections::BTreeSet<usize>,
    set: Option<String>,
    color: String,
    check: Tri,
    /// The dialog column over the steps that have a dialog; `None` when none has.
    dialog: Option<Tri>,
}

impl Row {
    fn new(app: &PhotocraftApp, i: usize, a: &Action, shown: bool) -> Row {
        let pending: Vec<&(String, Value)> = actions_cmds::pending_steps(&app.session, i).collect();
        let count = a.steps.len() + pending.len();
        let check = Tri::of(count - a.disabled.iter().filter(|s| **s < count).count(), count);
        // Recorded steps land at `pending_at`: number the steps as the panel shows them.
        let at = actions_cmds::pending_at(&app.session, i);
        let ids = a.steps.iter().take(at).chain(pending.iter().copied()).chain(a.steps.iter().skip(at)).map(|(id, _)| id.as_str());
        let with: Vec<usize> = ids.enumerate().filter(|(_, id)| has_dialog(id)).map(|(s, _)| s).collect();
        let dialog = (!with.is_empty()).then(|| Tri::of(with.iter().filter(|s| a.dialogs.contains(s)).count(), with.len()));
        Row {
            name: a.name.clone(),
            count,
            steps: if shown { visible_steps(app, i) } else { Vec::new() },
            disabled: a.disabled.clone(),
            dialogs: a.dialogs.clone(),
            set: a.set.clone(),
            color: a.color.clone(),
            check,
            dialog,
        }
    }
}

/// The panel's order: each set folder (in panel order) with its actions, then actions outside
/// any set. Indices are into the engine's flat list.
fn display_order(sets: &[String], rows: &[Row]) -> Vec<(Option<String>, Vec<usize>)> {
    let mut out: Vec<(Option<String>, Vec<usize>)> =
        sets.iter().map(|set| (Some(set.clone()), rows.iter().enumerate().filter(|(_, r)| r.set.as_ref() == Some(set)).map(|(i, _)| i).collect())).collect();
    let loose: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.set.is_none()).map(|(i, _)| i).collect();
    if !loose.is_empty() {
        out.push((None, loose));
    }
    out
}

/// What a click in the list asks for, applied once the list is drawn.
enum Click {
    ToggleSet(String),
    SelectSet(String),
    SetOptions(String),
    CheckSet(String, Tri),
    DialogSet(String, Option<Tri>),
    SelectAction(usize),
    ToggleAction(usize),
    ActionOptions(usize),
    CheckAction(usize),
    DialogAction(usize, Option<Tri>),
    Play(usize),
    Step { action: usize, step: usize, ctrl: bool, shift: bool },
    ToggleStep(usize, usize),
    CheckStep(usize, usize),
    DialogStep(usize, usize),
    PlayStep(usize, usize),
    RecordAgain(usize, usize),
}

fn apply(app: &mut PhotocraftApp, click: Click) {
    let rows = app.session.actions.list.len();
    match click {
        Click::ToggleSet(set) => match app.ui.actions.collapsed_sets.iter().position(|c| *c == set) {
            Some(i) => {
                app.ui.actions.collapsed_sets.remove(i);
            }
            None => app.ui.actions.collapsed_sets.push(set),
        },
        Click::SelectSet(set) => {
            app.ui.actions.selected = None;
            app.ui.actions.clear_steps();
            app.ui.actions.selected_set = Some(set);
        }
        Click::SetOptions(set) => {
            crate::actions_dialogs::set_options(app, &set);
        }
        Click::CheckSet(set, state) => {
            let on = state != Tri::On;
            let idx: Vec<usize> = (0..rows).filter(|i| app.session.actions.list[*i].set.as_ref() == Some(&set)).collect();
            for i in idx {
                let _ = app.run("actions.toggle", json!({"action": i, "enabled": on}));
            }
        }
        Click::DialogSet(set, state) => {
            let on = state != Some(Tri::On);
            let idx: Vec<usize> = (0..rows).filter(|i| app.session.actions.list[*i].set.as_ref() == Some(&set)).collect();
            for i in idx {
                set_dialogs(app, i, on);
            }
        }
        Click::SelectAction(i) => {
            app.ui.actions.selected = Some(i);
            app.ui.actions.selected_set = None;
            app.ui.actions.clear_steps();
        }
        Click::ToggleAction(i) => {
            let a = &mut app.ui.actions;
            a.expanded.resize(a.expanded.len().max(i + 1), false);
            a.expanded[i] = !a.expanded[i];
        }
        Click::ActionOptions(i) => {
            crate::actions_dialogs::action_options(app, i);
        }
        Click::CheckAction(i) => {
            let _ = app.run("actions.toggle", json!({"action": i}));
        }
        Click::DialogAction(i, state) => set_dialogs(app, i, state != Some(Tri::On)),
        Click::Play(i) => {
            if let Some(name) = app.session.actions.list.get(i).map(|a| a.name.clone()) {
                start(app, vec![name], 0, None);
            }
        }
        Click::Step { action, step, ctrl, shift } => {
            let a = &mut app.ui.actions;
            a.selected_set = None;
            if a.selected != Some(action) || !(ctrl || shift) {
                a.selected = Some(action);
                a.select_step(step);
            } else if shift {
                // Shift: every step from the one picked last to this one.
                let anchor = a.selected_step.unwrap_or(step);
                a.selected_steps = (anchor.min(step)..=anchor.max(step)).collect();
                a.selected_step = Some(anchor);
            } else {
                // Ctrl (⌘): add or take out one step.
                let mut steps: std::collections::BTreeSet<usize> = a.step_selection().into_iter().collect();
                if !steps.remove(&step) {
                    steps.insert(step);
                }
                a.selected_step = if steps.contains(&step) { Some(step) } else { steps.iter().next_back().copied() };
                a.selected_steps = steps;
            }
        }
        Click::ToggleStep(action, step) => {
            let key = (action, step);
            let open = &mut app.ui.actions.open_steps;
            if !open.remove(&key) {
                open.insert(key);
            }
        }
        Click::CheckStep(action, step) => {
            let _ = app.run("actions.toggle", json!({"action": action, "step": step}));
        }
        Click::DialogStep(action, step) => {
            let _ = app.run("actions.dialog", json!({"action": action, "step": step}));
        }
        Click::PlayStep(action, step) => {
            if let Some(name) = app.session.actions.list.get(action).map(|a| a.name.clone()) {
                app.ui.actions.selected = Some(action);
                app.ui.actions.select_step(step);
                start(app, vec![name], step, Some(1));
            }
        }
        Click::RecordAgain(action, step) => {
            if let Err(e) = record_again(app, action, step) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}

/// Switch the dialogs of action `i`'s steps that have one on or off.
fn set_dialogs(app: &mut PhotocraftApp, i: usize, on: bool) {
    let steps: Vec<usize> = visible_steps(app, i).iter().enumerate().filter(|(_, (id, _))| has_dialog(id)).map(|(s, _)| s).collect();
    for step in steps {
        let _ = app.run("actions.dialog", json!({"action": i, "step": step, "enabled": on}));
    }
}

/// Actions outside any set (recorded by an agent, or saved by an older PhotoCraft) go into
/// Default Actions: Photoshop keeps every action in a set.
fn adopt_loose_actions(app: &mut PhotocraftApp) {
    if app.session.actions.playing > 0 {
        return;
    }
    let loose: Vec<usize> = app.session.actions.list.iter().enumerate().filter(|(_, a)| a.set.is_none()).map(|(i, _)| i).collect();
    for i in loose {
        let _ = app.run("actions.assignSet", json!({"action": i, "set": actions_cmds::DEFAULT_SET}));
    }
}

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    adopt_loose_actions(app);
    let recording = app.session.actions.recording;
    let list_len = app.session.actions.list.len();
    // While recording, the newest recorded step is scrolled into view.
    let recording_row = recording.and_then(|(_, i)| {
        let n = actions_cmds::pending_steps(&app.session, i).count();
        (i < list_len).then(|| (i, actions_cmds::pending_at(&app.session, i) + n))
    });
    let reveal = recording_row.filter(|row| app.ui.actions.reveal_recording != Some(*row));
    app.ui.actions.reveal_recording = recording_row;
    if let Some((i, _)) = reveal {
        app.ui.actions.expanded.resize(list_len, false);
        if let Some(expanded) = app.ui.actions.expanded.get_mut(i) {
            *expanded = true;
        }
        if let Some(set) = app.session.actions.list.get(i).and_then(|a| a.set.clone()) {
            app.ui.actions.collapsed_sets.retain(|c| *c != set);
        }
    }
    let button_mode = app.ui.actions.button_mode;
    let rows: Vec<Row> = app
        .session
        .actions
        .list
        .iter()
        .enumerate()
        .map(|(i, a)| Row::new(app, i, a, !button_mode && app.ui.actions.expanded.get(i).copied().unwrap_or(false)))
        .collect();
    let sets = app.session.actions.sets();
    {
        let a = &mut app.ui.actions;
        if a.selected.is_some_and(|i| i >= rows.len()) {
            a.selected = None;
            a.clear_steps();
        }
        if let Some(n) = a.selected.and_then(|i| rows.get(i)).map(|r| r.count) {
            a.selected_steps.retain(|s| *s < n);
            if a.selected_step.is_some_and(|s| s >= n) {
                a.clear_steps();
            }
        }
        if a.selected_set.as_ref().is_some_and(|s| !sets.contains(s)) {
            a.selected_set = None;
        }
        if a.expanded.len() > rows.len() {
            a.expanded.truncate(rows.len());
        }
        if a.renaming.as_ref().is_some_and(|r| r.index >= rows.len()) {
            a.renaming = None;
        }
    }
    let mut renamed = None;
    let mut clicks: Vec<Click> = Vec::new();
    let mut save_set: Option<String> = None;
    let max_h = (ui.available_height() - 38.0).max(0.0);
    // Set Shortcut… in an action's menu: its dialog opens once the list is drawn.
    let mut open_key_for: Option<String> = None;
    let mut moved = None;
    let reveal_action = app.ui.actions.reveal_action.take();
    let playing_step = app.action_run.as_ref().map(|_| (app.ui.actions.selected, app.ui.actions.selected_step));
    let (ctrl, shift, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.shift, i.modifiers.alt));
    egui::ScrollArea::vertical().id_salt("actions-rows").max_height(max_h).min_scrolled_height(max_h).auto_shrink([false, false]).show(ui, |ui| {
        // Rows flush and ROW_H tall, as dense as Photoshop's list (and the History panel's);
        // Button Mode keeps its buttons apart.
        if !button_mode {
            ui.spacing_mut().item_spacing.y = 0.0;
        }
        if rows.is_empty() && sets.is_empty() {
            ui.label(egui::RichText::new(tl!("Record ● a sequence of edits, then play ▶ it on any document.")).color(t.text_faint).size(11.5));
        }
        if egui::DragAndDrop::has_payload_of_type::<DragRow>(ui.ctx()) {
            let clip = ui.clip_rect();
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| clip.contains(*p)) {
                let delta = if p.y < clip.top() + 22.0 {
                    8.0
                } else if p.y > clip.bottom() - 22.0 {
                    -8.0
                } else {
                    0.0
                };
                if delta != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, delta));
                    ui.ctx().request_repaint();
                }
            }
        }
        for (set, members) in display_order(&sets, &rows) {
            // Button Mode shows the actions alone, as buttons; the list shows set folders.
            if let Some(set) = set.as_deref().filter(|_| !button_mode) {
                let collapsed = app.ui.actions.collapsed_sets.iter().any(|c| c == set);
                let (hr, hresp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let selected = app.ui.actions.selected.is_none() && app.ui.actions.selected_set.as_deref() == Some(set);
                hresp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, !collapsed, set));
                if selected {
                    ui.painter().rect_filled(hr, 0.0, t.row_selected);
                } else if hresp.hovered() {
                    ui.painter().rect_filled(hr, 0.0, t.hover.gamma_multiply(0.5));
                }
                let check = members.iter().filter_map(|i| rows.get(*i)).map(|r| r.check).reduce(Tri::merge).unwrap_or(Tri::On);
                let dialog = members.iter().filter_map(|i| rows.get(*i)).filter_map(|r| r.dialog).reduce(Tri::merge);
                let x0 = hr.left();
                paint_check(ui, Rect::from_center_size(pos2(x0 + CHECK_X, hr.center().y), vec2(16.0, 16.0)), check, &t);
                paint_dialog(ui, Rect::from_center_size(pos2(x0 + DIALOG_X, hr.center().y), vec2(16.0, 16.0)), dialog, &t);
                paint_triangle(ui, pos2(x0 + TREE_X + 6.0, hr.center().y), !collapsed, &t);
                crate::icons::paint(
                    ui,
                    Rect::from_center_size(pos2(x0 + TREE_X + 22.0, hr.center().y), vec2(16.0, 16.0)),
                    if collapsed { "folder" } else { "folder-open" },
                    12.0,
                    t.icon,
                );
                ui.painter().text(pos2(x0 + TREE_X + 34.0, hr.center().y), Align2::LEFT_CENTER, set, egui::FontId::proportional(12.0), t.text);
                let x = hresp.interact_pointer_pos().map(|p| p.x - x0);
                // A double-click on the name opens Set Options; on the columns or the triangle it
                // is two clicks, as in Photoshop.
                if hresp.double_clicked() && x.is_some_and(|x| x >= TREE_X + 14.0) {
                    clicks.push(Click::SetOptions(set.to_string()));
                } else if hresp.clicked() {
                    clicks.push(match x {
                        Some(x) if x < CHECK_SPLIT => Click::CheckSet(set.to_string(), check),
                        Some(x) if x < TREE_X && dialog.is_some() => Click::DialogSet(set.to_string(), dialog),
                        Some(x) if (TREE_X..TREE_X + 14.0).contains(&x) => Click::ToggleSet(set.to_string()),
                        _ => Click::SelectSet(set.to_string()),
                    });
                }
                hresp.context_menu(|ui| {
                    if ui.button(tl!("Set Options…")).clicked() {
                        clicks.push(Click::SetOptions(set.to_string()));
                        ui.close();
                    }
                    if ui.button(tl!("Save Set…")).clicked() {
                        save_set = Some(set.to_string());
                        ui.close();
                    }
                });
                if collapsed {
                    continue;
                }
            }
            for i in members {
                let Some(row) = rows.get(i) else { continue };
                let Row { name, count, steps, disabled, dialogs, color: swatch, .. } = row;
                let indent = if set.is_some() && !button_mode { 14.0 } else { 0.0 };
                let row_h = if button_mode { BUTTON_H } else { ROW_H };
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
                let x0 = rect.left();
                let cx = x0 + TREE_X + indent;
                if reveal == Some((i, 0)) || reveal_action == Some(i) {
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                let sel = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step.is_none();
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, sel, name));
                if button_mode {
                    ui.painter().rect_filled(rect, 3.0, button_fill(swatch, &t));
                } else if sel {
                    ui.painter().rect_filled(rect, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
                }
                if let Some(change) = drag_row(ui, &resp, DragRow { action: i, step: None }, &t) {
                    moved = Some(change);
                }
                let expanded = app.ui.actions.expanded.get(i).copied().unwrap_or(false) && !button_mode;
                let (check, dialog) = (row.check, row.dialog);
                if !button_mode {
                    paint_check(ui, Rect::from_center_size(pos2(x0 + CHECK_X, rect.center().y), vec2(16.0, 16.0)), check, &t);
                    paint_dialog(ui, Rect::from_center_size(pos2(x0 + DIALOG_X, rect.center().y), vec2(16.0, 16.0)), dialog, &t);
                    paint_triangle(ui, pos2(cx + 6.0, rect.center().y), expanded, &t);
                }
                let recording_this = recording.is_some_and(|(_, r)| r == i);
                let name_x = if button_mode { x0 + 8.0 } else { cx + 18.0 };
                let name_color = if button_mode { button_label_color(swatch, &t) } else { t.text };
                let name_rect = if app.ui.actions.renaming.as_ref().is_some_and(|r| r.index == i) {
                    let field = Rect::from_min_max(pos2(cx + 14.0, rect.top() + 3.0), pos2((rect.right() - 80.0).max(cx + 120.0), rect.bottom() - 3.0));
                    if let Some(text) = rename_field(app, ui, i, field) {
                        renamed = Some((i, text));
                    }
                    field
                } else {
                    ui.painter().text(pos2(name_x, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), name_color)
                };
                let key = app.session.prefs().shortcuts.get(&shortcut_id(name)).filter(|s| !s.is_empty()).map(|k| crate::shortcuts::pretty(k));
                if !button_mode {
                    ui.painter().text(
                        pos2(rect.right() - 8.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        // The step count, then the shortcut at the right edge.
                        key.map_or_else(|| step_count(*count, disabled.len()), |key| format!("{} · {key}", step_count(*count, disabled.len()))),
                        egui::FontId::proportional(11.0),
                        t.text_faint,
                    );
                } else if let Some(key) = key {
                    ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, key, egui::FontId::proportional(11.0), name_color);
                }
                if recording_this {
                    ui.painter().circle_filled(pos2(rect.right() - 64.0, rect.center().y), 4.0, Color32::from_rgb(230, 60, 60));
                }
                let x = resp.interact_pointer_pos().map(|p| p.x - x0);
                let double = resp.double_clicked();
                if double && button_mode {
                    // The first click already played it.
                } else if double && resp.interact_pointer_pos().is_some_and(|p| name_rect.expand2(vec2(4.0, 13.0)).contains(p)) {
                    // Photoshop renames in place when the name itself is double-clicked, and
                    // opens Action Options elsewhere on the row past the triangle (on the columns
                    // or the triangle a double-click is two clicks).
                    app.ui.actions.renaming = Some(RenameAction { index: i, text: name.clone(), focused: false });
                } else if double && x.is_some_and(|x| x >= cx - x0 + 14.0) {
                    clicks.push(Click::ActionOptions(i));
                } else if resp.clicked() {
                    clicks.push(match x {
                        _ if button_mode => Click::Play(i),
                        Some(x) if x < CHECK_SPLIT => Click::CheckAction(i),
                        Some(x) if x < TREE_X && dialog.is_some() => Click::DialogAction(i, dialog),
                        Some(x) if (TREE_X + indent..cx + 14.0).contains(&x) => Click::ToggleAction(i),
                        _ => Click::SelectAction(i),
                    });
                    if button_mode {
                        clicks.push(Click::SelectAction(i));
                    }
                }
                resp.context_menu(|ui| {
                    if ui.button(tl!("Action Options…")).clicked() {
                        clicks.push(Click::ActionOptions(i));
                        ui.close();
                    }
                    ui.separator();
                    ui.label(tl!("Keyboard Shortcuts"));
                    // Photoshop's Action Options: a function key, with Shift and/or Control (⌘).
                    let current = app.session.prefs().shortcuts.get(&shortcut_id(name)).cloned().unwrap_or_default();
                    let assigned = actions_cmds::parse_action_key(&current);
                    // Modifiers picked before a key are kept for the key picked next.
                    let mods_id = egui::Id::new(("action-key-modifiers", name.as_str()));
                    let (mut shift, mut command) =
                        assigned.map_or_else(|| ui.ctx().data(|d| d.get_temp::<(bool, bool)>(mods_id)).unwrap_or_default(), |(_, s, c)| (s, c));
                    let key_now = assigned.map(|(n, _, _)| n);
                    ui.horizontal(|ui| {
                        let toggled = ui.checkbox(&mut shift, crate::shortcuts::pretty("Shift")).changed()
                            | ui.checkbox(&mut command, crate::shortcuts::pretty("Cmd")).changed();
                        if toggled {
                            match key_now.and_then(|n| actions_cmds::action_key(n, shift, command)) {
                                Some(key) => {
                                    let _ = assign_shortcut(app, name, &key);
                                }
                                None => {
                                    ui.ctx().data_mut(|d| d.insert_temp(mods_id, (shift, command)));
                                }
                            }
                        }
                    });
                    for n in std::iter::once(None).chain((1..=12).map(Some)) {
                        let label = n.map_or_else(|| tl!("None").to_string(), |n| format!("F{n}"));
                        if ui.selectable_label(key_now == n, label).clicked() {
                            let key = n.and_then(|n| actions_cmds::action_key(n, shift, command)).unwrap_or_default();
                            let _ = assign_shortcut(app, name, &key);
                            ui.close();
                        }
                    }
                    if ui.button(tl!("Set Shortcut…")).clicked() {
                        open_key_for = Some(name.clone());
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button(tl!("Color"), |ui| {
                        for (id, label) in [
                            ("", tl!("None")),
                            ("red", tl!("Red")),
                            ("orange", tl!("Orange")),
                            ("yellow", tl!("Yellow")),
                            ("green", tl!("Green")),
                            ("blue", tl!("Blue")),
                            ("violet", tl!("Violet")),
                            ("gray", tl!("Gray")),
                        ] {
                            if ui.selectable_label(swatch == id, label).clicked() {
                                let _ = app.run("actions.setColor", json!({"action": i, "color": id}));
                                ui.close();
                            }
                        }
                    });
                });
                if !expanded {
                    continue;
                }
                let picked = if app.ui.actions.selected == Some(i) { app.ui.actions.step_selection() } else { Vec::new() };
                for (step, (id, params)) in steps.iter().enumerate() {
                    let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                    if reveal.is_some_and(|(a, n)| a == i && n == step + 1) || playing_step == Some((Some(i), Some(step))) {
                        ui.scroll_to_rect(r, None);
                    }
                    let label = step_label(id, params);
                    let selected = picked.contains(&step);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
                    if selected {
                        ui.painter().rect_filled(r, 0.0, t.row_selected);
                    } else if response.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                    }
                    if let Some(change) = drag_row(ui, &response, DragRow { action: i, step: Some(step) }, &t) {
                        moved = Some(change);
                    }
                    let on = !disabled.contains(&step);
                    let lines = param_lines(params);
                    let open = !lines.is_empty() && app.ui.actions.open_steps.contains(&(i, step));
                    let sx = cx + 14.0;
                    if !lines.is_empty() {
                        paint_triangle(ui, pos2(sx + 6.0, r.center().y), open, &t);
                    }
                    let color = if on { t.text_dim } else { t.text_faint };
                    ui.painter().text(pos2(sx + 18.0, r.center().y), Align2::LEFT_CENTER, &label, egui::FontId::proportional(11.5), color);
                    // Photoshop's check column: unchecked steps are skipped by playback, Batch and droplets.
                    let check = ui.interact(
                        Rect::from_center_size(pos2(x0 + CHECK_X, r.center().y), vec2(18.0, 18.0)),
                        ui.id().with(("action-step-check", i, step)),
                        Sense::click(),
                    );
                    check.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, format!("{}: {label}", tl!("Include in playback"))));
                    paint_check(ui, check.rect, if on { Tri::On } else { Tri::Off }, &t);
                    // Photoshop's dialog column, for steps that have a dialog: playback shows it,
                    // seeded with the recorded values, and continues when it is confirmed.
                    let with_dialog = dialogs.contains(&step);
                    let dialog_hit = has_dialog(id).then(|| {
                        let dialog = ui.interact(
                            Rect::from_center_size(pos2(x0 + DIALOG_X, r.center().y), vec2(18.0, 18.0)),
                            ui.id().with(("action-step-dialog", i, step)),
                            Sense::click(),
                        );
                        // A toggle button, not a check box, so the check column stays the only CheckBox role.
                        dialog.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, true, with_dialog, format!("{}: {label}", tl!("Show dialog during playback")))
                        });
                        paint_dialog(ui, dialog.rect, Some(if with_dialog { Tri::On } else { Tri::Off }), &t);
                        dialog.on_hover_text(tl!("Show dialog during playback"))
                    });
                    let x = response.interact_pointer_pos().map(|p| p.x - x0);
                    if check.on_hover_text(tl!("Include in playback")).clicked() {
                        clicks.push(Click::CheckStep(i, step));
                    } else if dialog_hit.is_some_and(|d| d.clicked()) {
                        clicks.push(Click::DialogStep(i, step));
                    } else if response.double_clicked() {
                        // Ctrl+double-click plays the step alone; a double-click records it again.
                        clicks.push(if ctrl { Click::PlayStep(i, step) } else { Click::RecordAgain(i, step) });
                    } else if response.clicked() {
                        if !lines.is_empty() && x.is_some_and(|x| (sx - x0..sx - x0 + 14.0).contains(&x)) {
                            clicks.push(Click::ToggleStep(i, step));
                        } else {
                            clicks.push(Click::Step { action: i, step, ctrl, shift });
                        }
                    }
                    if open {
                        for line in lines {
                            let (pr, _) = ui.allocate_exact_size(vec2(ui.available_width(), PARAM_H), Sense::hover());
                            ui.painter().text(pos2(sx + 34.0, pr.center().y), Align2::LEFT_CENTER, line, egui::FontId::proportional(11.0), t.text_faint);
                        }
                    }
                }
            }
        }
    });
    for click in clicks {
        apply(app, click);
    }
    if let Some(set) = save_set
        && let Err(e) = save_dialog(app, Some(set))
        && e != "cancelled"
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some((index, name)) = renamed {
        rename_action(app, index, name);
    }
    if let Some((source, to)) = moved {
        move_row(app, source, to);
    }
    ui.add_space(4.0);
    // Photoshop's buttons: Stop, Record, Play, Create new set, Create new action, Delete.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let recording_now = app.session.actions.recording.is_some();
        let busy = recording_now || app.action_run.is_some();
        let stop_button = crate::icons::button(ui, "square", 24.0, false, tl!("Stop playing/recording"));
        if stop_button.clicked() && busy {
            stop(app);
        }
        let (r, rec) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
        if rec.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        ui.painter().circle_filled(r.center(), 5.5, if recording_now { Color32::from_rgb(230, 60, 60) } else { t.icon });
        rec.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Begin recording")));
        if rec.on_hover_text(tl!("Begin recording")).clicked() && !recording_now {
            begin_recording(app);
        }
        let play = crate::icons::button(ui, "play", 24.0, false, tl!("Play selection"));
        play.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Play selection")));
        if play.clicked() {
            play_selection(app, ctrl);
        }
        let new_set = crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new set"));
        new_set.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !recording_now, tl!("Create new set")));
        if new_set.clicked() && !recording_now {
            crate::actions_dialogs::new_set(app);
        }
        let new_action = crate::icons::button(ui, "file-plus", 24.0, false, tl!("Create new action"));
        new_action.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !recording_now, tl!("Create new action")));
        if new_action.clicked() && !recording_now {
            crate::actions_dialogs::new_action(app);
        }
        let a = &app.ui.actions;
        let can_delete = (a.selected.is_some() || a.selected_set.is_some()) && (!recording_now || a.selected_step.is_some());
        ui.add_enabled_ui(can_delete, |ui| {
            let response = crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, can_delete, tl!("Delete")));
            if response.clicked() {
                delete_selection(app, alt);
            }
        });
    });
    if let Some(name) = open_key_for {
        open_set_shortcut(app, &name);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A click on step `step` of action `action`, with Ctrl and Shift as given.
    pub(crate) fn click_step(app: &mut PhotocraftApp, action: usize, step: usize, ctrl: bool, shift: bool) {
        apply(app, Click::Step { action, step, ctrl, shift });
    }

    /// The Record button.
    pub(crate) fn record_button(app: &mut PhotocraftApp) {
        begin_recording(app);
    }

    pub(crate) fn record_again_step(app: &mut PhotocraftApp, action: usize, step: usize) -> Result<Value, String> {
        record_again(app, action, step)
    }

    #[test]
    fn recorded_layers_read_as_names_or_the_current_layer() {
        let lines = param_lines(&json!({"layer": {"name": "Ink", "occurrence": 1}, "target": {"current": true}, "layers": [{"name": "A"}, {"current": true}], "opacity": 0.5}));
        assert_eq!(lines, ["Layer: Ink (2)", "Layers: A, Current Layer", "Opacity: 0.5", "Target: Current Layer"]);
    }

    /// An action's Set Shortcut… takes any key with any modifiers (beyond Photoshop's F-keys), and
    /// that key plays it; system keys are refused and Backspace clears it.
    #[test]
    fn any_key_combination_plays_an_action() {
        use egui::{Event, Key, Modifiers};
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
        });
        h.state_mut().run("file.new", json!({"width": 16, "height": 16})).unwrap();
        h.state_mut().run("actions.record", json!({"name": "New Layer"})).unwrap();
        h.state_mut().run("layer.new.layer", json!({})).unwrap();
        h.state_mut().run("actions.stop", json!({})).unwrap();
        h.run_steps(2);
        let layers = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.layers.len();
        let press = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, key: Key, modifiers: Modifiers| {
            for pressed in [true, false] {
                h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
            }
            h.run_steps(3);
        };
        let dialog = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| owns_set_shortcut(&d.fields)).map(|d| (d.id, d.fields.clone()));
        let id = open_set_shortcut(h.state_mut(), "New Layer");
        h.run_steps(2);
        // Alt+F4 belongs to the system.
        press(&mut h, Key::F4, Modifiers::ALT);
        let (_, f) = dialog(&h).unwrap();
        assert_eq!(f["key"], json!(""));
        assert!(f["message"].as_str().unwrap().contains("reserved"), "{f:?}");
        // Ctrl+Alt+K.
        let ctrl_alt = Modifiers { alt: true, ctrl: true, command: true, ..Modifiers::NONE };
        press(&mut h, Key::K, ctrl_alt);
        let (_, f) = dialog(&h).unwrap();
        let key = f["key"].as_str().unwrap().to_string();
        assert!(key.contains('K') && key.contains("Alt"), "{key}");
        set_shortcut_confirm(h.state_mut(), &f).unwrap();
        h.state_mut().ui.close_dialog(id);
        assert_eq!(h.state().session.prefs().shortcuts.get(&shortcut_id("New Layer")), Some(&key));
        let before = layers(&h);
        press(&mut h, Key::K, ctrl_alt);
        assert_eq!(layers(&h), before + 1, "Ctrl+Alt+K played the action");
        // A plain letter works too; Backspace clears the shortcut.
        let id = open_set_shortcut(h.state_mut(), "New Layer");
        h.run_steps(2);
        press(&mut h, Key::Backspace, Modifiers::NONE);
        let (_, f) = dialog(&h).unwrap();
        assert_eq!(f["key"], json!(""));
        set_shortcut_confirm(h.state_mut(), &f).unwrap();
        h.state_mut().ui.close_dialog(id);
        assert!(h.state().session.prefs().shortcuts.get(&shortcut_id("New Layer")).is_none_or(String::is_empty));
        let mut gone = f.clone();
        gone.insert("action".into(), json!("Missing"));
        assert!(set_shortcut_confirm(h.state_mut(), &gone).is_err());
    }

    /// Import Photoshop Actions loads every set of Photoshop's Actions palette at once, and says
    /// so; without a Photoshop on the computer it says that instead.
    #[test]
    fn import_photoshop_actions_loads_the_whole_palette() {
        use photocraft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
        use photocraft_psd::descriptor::UnicodeString;
        let set = |name: &str, actions: &[&str]| AtnFile {
            name: UnicodeString::new_nul(name),
            expanded: true,
            actions: actions
                .iter()
                .map(|a| AtnAction {
                    name: UnicodeString::new_nul(a),
                    items: vec![AtnItem { event: AtnEvent::Code(*b"Fltn"), dictionary: "Flatten Image".into(), ..Default::default() }],
                    ..Default::default()
                })
                .collect(),
            warnings: vec![],
        };
        let palette = photocraft_psd::atn::write_palette(&[set("Mine", &["One", "Two"]), set("Theirs", &["Three"])]).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.services.photoshop_actions = Some(Box::new(move || Some(("C:/PS/Actions Palette.psp".into(), palette.clone()))));
        import_photoshop(&mut app).unwrap();
        let names: Vec<(&str, Option<&str>)> = app.session.actions.list.iter().map(|a| (a.name.as_str(), a.set.as_deref())).collect();
        assert_eq!(names, [("One", Some("Mine")), ("Two", Some("Mine")), ("Three", Some("Theirs"))]);
        assert!(app.ui.status.contains("3 actions in 2 sets"), "{}", app.ui.status);
        // Again: the sets are replaced, not doubled.
        import_photoshop(&mut app).unwrap();
        assert_eq!(app.session.actions.list.len(), 3);
        app.services.photoshop_actions = Some(Box::new(|| None));
        assert!(import_photoshop(&mut app).is_err(), "no Photoshop, no palette");
    }

    #[test]
    fn panel_menu_lists_load_save_and_existing_action_commands() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut empty_app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        empty_app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut empty = Harness::builder().with_size(vec2(240.0, 280.0)).build_ui_state(|ui, app| panel_menu(app, ui), empty_app);
        empty.run_steps(2);
        assert!(empty.query_all_by_label("Load Actions…").count() > 0);
        assert!(empty.query_all_by_label("New Action…").count() > 0);
        assert!(empty.query_all_by_label("Start Recording").count() > 0);
        empty.get_by_label("New Action…").click();
        empty.run_steps(2);
        // Photoshop's New Action dialog names the action before Record makes it.
        assert_eq!(empty.state().ui.dialogs.len(), 1, "the New Action dialog");

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.session.actions.list.push(Action::new("Make Layer", vec![("layer.new.layer".into(), json!({}))]));
        app.ui.actions.selected = Some(0);
        let mut h = Harness::builder().with_size(vec2(240.0, 320.0)).build_ui_state(|ui, app| panel_menu(app, ui), app);
        h.run_steps(2);
        for label in ["Load Actions…", "Save Actions…", "Insert Stop…", "Play", "Delete", "Start Recording"] {
            assert!(h.query_all_by_label(label).count() > 0, "{label}");
        }
        let layers = h.state().session.active().unwrap().doc.layers.len();
        h.get_by_label("Play").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
    }

    #[test]
    fn selected_layer_name_is_visible_during_recording_and_after_stop() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4})).unwrap();
        app.run("layer.setProps", json!({"name":"Background"})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        // Another layer selected first: selecting the one already selected isn't recorded.
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("actions.record", json!({"name":"Select photo"})).unwrap();
        app.ui.actions.expanded = vec![true];
        app.run("layer.select", json!({"layer":layer.0})).unwrap();
        let mut h = Harness::builder().with_size(vec2(400.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        h.get_by_label("Select Layer (Background)");
        h.state_mut().run("actions.stop", json!({})).unwrap();
        h.run_steps(4);
        h.get_by_label("Select Layer (Background)");
        assert_eq!(step_label("layer.select", &json!({"layer":26})), "Select Layer (ID 26)");
        assert_eq!(step_label("layer.select", &json!({"layer":"Background"})), "Select Layer (Background)");
    }

    #[test]
    fn actions_list_grows_with_panel_and_keeps_footer_visible() {
        use egui_kittest::{Harness, kittest::Queryable};
        let bottom = |height: f32| {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            for i in 0..50 {
                app.session.actions.list.push(Action { name: format!("Action {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
            }
            let mut h = Harness::builder().with_size(vec2(320.0, height)).build_ui_state(|ui, app| panel(app, ui), app);
            h.run_steps(3);
            let rect = h.get_by_label("Play selection").rect();
            assert!(rect.bottom() <= height, "footer must fit: {rect:?}");
            rect.top()
        };
        assert!(bottom(700.0) - bottom(300.0) > 350.0, "list uses the additional panel height instead of stopping at 320 px");
        assert!(crate::dock::Group::History.scrolls_itself(1));
    }

    /// #7646: an action's function key can take Shift and Command, as in Photoshop's Action
    /// Options, and only that combination plays it.
    #[test]
    fn a_function_key_with_shift_and_command_plays_its_action() {
        use egui::{Event, Key, Modifiers};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.session.actions.list.push(Action {
            name: "Action 1".into(),
            steps: vec![("layer.new.layer".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        let key = photocraft_engine::atn_cmds::function_key_name(2, true, true);
        assert_eq!(key, "Cmd+Shift+F2");
        assign_shortcut(&mut app, "Action 1", &key).unwrap();
        let ctx = egui::Context::default();
        let press = |app: &mut PhotocraftApp, modifiers| {
            ctx.begin_pass(egui::RawInput {
                events: vec![
                    Event::ModifiersChanged(modifiers),
                    Event::Key { key: Key::F2, physical_key: None, pressed: true, repeat: false, modifiers },
                    Event::Key { key: Key::F2, physical_key: None, pressed: false, repeat: false, modifiers },
                ],
                ..Default::default()
            });
            crate::shortcuts::handle(app, &ctx);
            ctx.end_pass().textures_delta.clear();
        };
        let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
        let before = layers(&app);
        press(&mut app, Modifiers::NONE);
        press(&mut app, Modifiers::SHIFT);
        assert_eq!(layers(&app), before, "F2 and Shift+F2 alone don't play it");
        press(&mut app, Modifiers::SHIFT | Modifiers::COMMAND);
        assert_eq!(layers(&app), before + 1, "Cmd+Shift+F2 plays it");
    }

    #[test]
    fn double_clicking_an_action_name_renames_it_in_place() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.session.actions.list.push(Action {
            name: "Action 1".into(),
            steps: vec![("layer.new.layer".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        let layers = app.session.active().unwrap().doc.layers.len();
        // 60 fps steps, so two clicks a frame apart count as a double-click.
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let rename = |h: &mut Harness<'_, PhotocraftApp>, text: &str| {
            // The name, right of the two toggle columns and the triangle (inside its set).
            let name = h.get_by_label("Action 1").rect().left_center() + vec2(TREE_X + 14.0 + 30.0, 0.0);
            h.event(egui::Event::PointerMoved(name));
            // Wait out the double-click window of the previous attempt.
            h.run_steps(30);
            for pressed in [true, false, true, false] {
                h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
            h.event(egui::Event::Text(text.into()));
            h.run_steps(1);
            h.key_press(egui::Key::Enter);
            h.run_steps(3);
        };
        // Whitespace keeps the old name and reports why.
        rename(&mut h, "  ");
        assert_eq!(h.state().session.actions.list[0].name, "Action 1");
        assert!(h.state().ui.status_error);
        rename(&mut h, "Sepia");
        assert_eq!(h.state().session.actions.list[0].name, "Sepia");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "renaming must not play the action");
    }

    #[test]
    fn play_button_targets_first_action_if_nothing_is_selected() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        assert_eq!(playback_selection(&app), None);
        app.session.actions.list.push(Action {
            name: "Make Layer".into(),
            steps: vec![("layer.new.layer".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        app.session.actions.list.push(Action {
            name: "Fit".into(),
            steps: vec![("view.fitOnScreen".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        assert_eq!(playback_selection(&app), Some(0));
        app.ui.actions.selected = Some(1);
        assert_eq!(playback_selection(&app), Some(1));
        app.ui.actions.selected = Some(999);
        assert_eq!(playback_selection(&app), Some(0));

        app.ui.actions.selected = None;
        use egui_kittest::{Harness, kittest::Queryable};
        let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let before = h.state().session.active().unwrap().doc.layers.len();
        h.get_by_label("Play selection").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
        assert!(!h.state().ui.status_error);
    }

    #[test]
    fn step_check_box_excludes_the_step_from_playback_and_batch() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n})));
        app.session.actions.list.push(Action::new("Three", steps.to_vec()));
        app.ui.actions.expanded = vec![true];
        let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let checks = h.query_all_by_role(egui::accesskit::Role::CheckBox).count();
        assert_eq!(checks, 3);
        h.query_all_by_role(egui::accesskit::Role::CheckBox).nth(1).unwrap().click();
        h.run_steps(3);
        let action = &h.state().session.actions.list[0];
        assert_eq!(action.disabled.iter().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(action_steps(action).len(), 2, "Batch and droplets get only the checked steps");
        assert_eq!(step_count(3, action.disabled.len()), "2/3 steps");
        h.get_by_label("Play selection").click();
        h.run_steps(3);
        let names: Vec<_> = h.state().session.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.clone()).collect();
        assert_eq!(names, ["A", "C"]);
    }

    #[test]
    fn playback_errors_are_visible_in_status_bar() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.session.actions.list.push(Action {
            name: "Bad action".into(),
            steps: vec![("not.a.command".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        h.get_by_label("Play selection").click();
        h.run_steps(3);
        assert!(h.state().ui.status_error);
        assert!(h.state().ui.status.contains("not.a.command") || h.state().ui.status.contains("unknown command"));
    }

    #[test]
    fn recording_reveals_new_action_and_latest_step_in_a_long_panel() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4})).unwrap();
        for i in 0..30 {
            app.session.actions.list.push(Action { name: format!("Existing {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
        }
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        h.state_mut().run("actions.record", json!({})).unwrap();
        h.run_steps(20);
        let rect = h.get_by_label("Action 31").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "new action visible: {rect:?}");
        for _ in 0..20 {
            h.state_mut().run("select.all", json!({})).unwrap();
        }
        h.state_mut().run("select.inverse", json!({})).unwrap();
        h.run_steps(20);
        let rect = h.get_by_label("Inverse").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "latest step visible: {rect:?}");
    }

    #[test]
    fn drag_steps_and_action_headers_reorders_and_keeps_selection() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action {
            name: "First".into(),
            steps: vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        app.session.actions.list.push(Action { name: "Second".into(), steps: vec![], disabled: Default::default(), ..Default::default() });
        app.ui.actions.expanded = vec![true, false];
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
            h.run_steps(2);
            h.event(egui::Event::PointerMoved(to));
            h.run_steps(2);
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(3);
        }
        let from = h.get_by_label("Zoom Out").rect().center();
        let target = h.get_by_label("100%").rect();
        drag(&mut h, from, pos2(target.center().x, target.bottom() - 2.0));
        assert_eq!(
            h.state().session.actions.list[0].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(),
            ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]
        );
        assert_eq!(h.state().ui.actions.selected_step, Some(2));
        let from = h.get_by_label("Second").rect().center();
        let target = h.get_by_label("First").rect();
        drag(&mut h, from, pos2(target.center().x, target.top() + 2.0));
        assert_eq!(h.state().session.actions.list[0].name, "Second");
        assert_eq!(h.state().ui.actions.expanded, [false, true]);
        assert_eq!(h.state().ui.actions.selected, Some(0));
    }

    #[test]
    fn nested_shell_actions_include_view_steps_and_propagate_failures() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":40,"height":40})).unwrap();
        app.session.actions.list.push(Action {
            name: "Child".into(),
            steps: vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        app.run("actions.record", json!({"name":"Parent"})).unwrap();
        app.run("actions.play", json!({"action":"Child"})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, [("actions.play".into(), json!({"action":"Child"}))]);
        assert_eq!(step_label("actions.play", &json!({"action":"Child"})), "Play Action: Child");
        app.run("view.actualPixels", json!({})).unwrap();
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert!(app.ui.views[0].fit_pending);
        app.session.authorize =
            Some(|id, _| if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
        assert!(app.session.actions.playback_stack.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }

    #[test]
    fn panel_shows_pending_steps_and_trash_deletes_only_the_clicked_step() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.run("actions.record", json!({})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("select.inverse", json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(2);
        assert!(h.state().session.actions.recording.is_some());
        h.get_by_label("All");
        h.get_by_label("Inverse").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.actions.selected_step, Some(1));
        // Delete asks first, as Photoshop does.
        h.get_by_label("Delete").click();
        h.run_steps(2);
        let ask = h.state().ui.dialogs.iter().find(|d| crate::actions_dialogs::owns(&d.fields)).map(|d| d.id).unwrap();
        crate::dialogs::confirm(h.state_mut(), ask).unwrap();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["select.all"]);
        assert_eq!(h.state().session.actions.list.len(), 1);
        assert!(h.state().session.actions.recording.is_some());
        assert!(h.state().ui.actions.selected_step.is_none());
        delete_selection(h.state_mut(), true);
        assert_eq!(h.state().session.actions.list.len(), 1, "repeated trash cannot delete the action being recorded");
        h.state_mut().run("select.deselect", json!({})).unwrap();
        h.run_steps(2);
        h.get_by_label("Deselect");
        h.state_mut().run("actions.stop", json!({})).unwrap();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["select.all", "select.deselect"]);
        h.get_by_label("Deselect").click();
        h.run_steps(2);
        delete_selection(h.state_mut(), true);
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["select.all"]);
    }

    #[test]
    fn view_menu_commands_are_inserted_not_recorded_and_replay_after_resize() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
        app.run("actions.record", json!({})).unwrap();
        app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
        // Photoshop doesn't record the View menu...
        crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[0].steps.len(), 1);
        // ...Insert Menu Item adds it.
        app.ui.actions.selected = Some(0);
        let dialog = crate::actions_dialogs::insert_menu_item(&mut app).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("item".into(), json!("view.fitOnScreen"));
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert_eq!(app.session.actions.list.len(), 1);
        assert_eq!(app.session.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["image.canvasSize", "view.fitOnScreen"]);
        assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
        // Persisted actions retain the menu step, and playback fits the new document.
        let saved = serde_json::to_string(&app.session.actions.list).unwrap();
        app.session.actions.list = serde_json::from_str(&saved).unwrap();
        app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 2);
        assert!(result.get("failed").is_none(), "{result}");
        let idx = app.session.active_index().unwrap();
        assert!(app.ui.views[idx].fit_pending);
        let doc = &app.session.active().unwrap().doc;
        crate::canvas::fit_view(&mut app.ui.views[idx], doc, vec2(100.0, 100.0), 1.0, 1.0);
        assert_eq!(app.ui.views[idx].zoom, 0.75);
        assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
        assert!(!app.ui.views[idx].fit_pending);
    }

    #[test]
    fn view_action_failure_resume_and_authorization() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action {
            name: "Fit".into(),
            steps: vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
            disabled: Default::default(),
            ..Default::default()
        });
        let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
        assert_eq!(result["ran"], 0);
        assert_eq!(result["failed"]["step"], 0);
        assert_eq!(app.session.actions.playing, 0);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.session.authorize =
            Some(
                |id, _| {
                    if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
                },
            );
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
        let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(!app.ui.views[0].fit_pending);
        assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
        app.session.actions.playing = 1;
        assert!(app.run("actions.play", json!({"action": 0})).is_err());
    }

    #[test]
    fn record_and_replay_on_another_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        app.run("actions.record", json!({})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
        app.run("actions.stop", json!({})).unwrap();
        let a = &app.session.actions.list[0];
        assert_eq!(a.steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "select.rect", "edit.fill"]);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(played["ran"], 3);
        assert!(played.get("failed").is_none(), "{played}");
        let d = &app.session.active().unwrap().doc;
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn recorded_save_as_and_save_replay_and_write_the_file() {
        let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let log = written.clone();
        let services = crate::Services {
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |path, _| {
                log.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("actions.record", json!({})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/result.psd"})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        let steps = &app.session.actions.list[0].steps;
        assert_eq!(steps, &[("layer.new.layer".into(), json!({})), ("file.saveAs".into(), json!({"path": "out/result.psd"})), ("file.save".into(), json!({}))]);
        written.borrow_mut().clear();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        assert_eq!(*written.borrow(), ["out/result.psd", "out/result.psd"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out/result.psd"));
    }

    /// An app with the real importer/exporter and writer whose Save dialogs answer `answers`.
    fn disk_app(answers: Vec<Option<crate::file_dialog::FileDialogAnswer>>) -> PhotocraftApp {
        let services = crate::Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            export: Some(Box::new(|doc, path, _| {
                photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
            file_dialog: Some(crate::file_dialog::fake(answers).0),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.background_jobs = false;
        app
    }

    /// Records Invert + File › Save on the file `a`, then plays it on the file `b` (given a
    /// new layer first when `layered`). Returns the played result and whether `b` / `a`
    /// changed on disk.
    fn record_save_on_a_play_on_b(ext: &str, layered: bool) -> (Value, bool, bool) {
        use crate::file_dialog::FileDialogAnswer::SaveTo;
        let dir = std::env::temp_dir().join(format!("photocraft-rec-save-{ext}-{layered}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = |n: &str| dir.join(format!("{n}.{ext}")).to_string_lossy().into_owned();
        let (a, b) = (path("a"), path("b"));
        // The dialog confirms the suggested name, which is the document's own file.
        let mut app = disk_app(vec![Some(SaveTo(a.clone())), Some(SaveTo(a.clone()))]);
        let ctx = egui::Context::default();
        for p in [&a, &b] {
            app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
            app.save_as(Some(p.clone())).unwrap();
        }
        let (before_a, before_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        app.open_path(&a).unwrap();
        app.run("actions.record", json!({})).unwrap();
        app.run("image.adjustments.invert", json!({})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        app.run("actions.stop", json!({})).unwrap();
        std::fs::write(&a, &before_a).unwrap();
        app.open_path(&b).unwrap();
        if layered {
            app.run("layer.new.layer", json!({})).unwrap();
        }
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        let r = (played, std::fs::read(&b).unwrap() != before_b, std::fs::read(&a).unwrap() != before_a);
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    #[test]
    fn recorded_save_overwrites_the_layered_file_the_action_is_played_on() {
        let (played, b_changed, a_changed) = record_save_on_a_play_on_b("psd", false);
        assert!(played.get("failed").is_none(), "{played}");
        assert!(b_changed && !a_changed);
    }

    #[test]
    fn recorded_save_overwrites_the_flat_file_the_action_is_played_on() {
        let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", false);
        assert!(played.get("failed").is_none(), "{played}");
        assert!(b_changed, "Save did not overwrite the file the action was played on");
        assert!(!a_changed, "Save wrote the file the action was recorded on");
    }

    #[test]
    fn recorded_save_never_flattens_layers_over_a_flat_file() {
        // The played-on document has layers its PNG can't keep: the step fails, nothing is written (#3806).
        let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", true);
        assert!(played.get("failed").is_some(), "{played}");
        assert!(!b_changed && !a_changed);
    }

    #[test]
    fn set_folders_group_actions_and_collapse() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut sharpen = Action::new("Sharpen", vec![("filter.sharpen.sharpen".into(), json!({}))]);
        sharpen.set = Some("Retouch".into());
        let mut soften = Action::new("Soften", vec![("filter.blur.blur".into(), json!({}))]);
        soften.set = Some("Retouch".into());
        app.session.actions.list.push(Action::new("Loose", vec![]));
        app.session.actions.list.push(sharpen);
        app.session.actions.list.push(Action::new("Also loose", vec![]));
        app.session.actions.list.push(soften);
        let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        // Loose actions go into Default Actions (Photoshop keeps every action in a set); each
        // folder row has its actions under it.
        let folder = h.get_by_label("Retouch").rect();
        let default = h.get_by_label(actions_cmds::DEFAULT_SET).rect();
        assert!(default.top() < h.get_by_label("Loose").rect().top());
        assert!(h.get_by_label("Also loose").rect().top() < folder.top());
        assert!(h.get_by_label("Sharpen").rect().top() > folder.top());
        assert!(h.get_by_label("Soften").rect().top() > h.get_by_label("Sharpen").rect().top());
        assert!(h.state().session.actions.list.iter().all(|a| a.set.is_some()));
        assert!(display_order(&[], &[]).is_empty());
        // Clicking the folder selects it; its triangle collapses it, and the state is remembered.
        h.get_by_label("Retouch").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.actions.selected_set.as_deref(), Some("Retouch"));
        let triangle = pos2(folder.left() + TREE_X + 6.0, folder.center().y);
        let click = |h: &mut Harness<'_, PhotocraftApp>| {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: triangle, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(30);
        };
        h.hover_at(triangle);
        click(&mut h);
        assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"]);
        assert!(h.query_by_label("Sharpen").is_none());
        assert!(h.query_by_label("Loose").is_some());
        click(&mut h);
        assert!(h.state().ui.actions.collapsed_sets.is_empty());
        assert!(h.query_by_label("Sharpen").is_some());
    }

    #[test]
    fn moving_an_action_next_to_another_set_joins_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut a = Action::new("A", vec![]);
        a.set = Some("One".into());
        let mut b = Action::new("B", vec![]);
        b.set = Some("Two".into());
        app.session.actions.list.push(a);
        app.session.actions.list.push(b);
        app.session.actions.list.push(Action::new("C", vec![]));
        // C dropped below B: it joins B's set, "Two".
        move_row(&mut app, DragRow { action: 2, step: None }, 2);
        assert_eq!(app.session.actions.list[2].set.as_deref(), Some("Two"));
        // B dropped to the top, above A: the row below decides when there is none above.
        move_row(&mut app, DragRow { action: 1, step: None }, 0);
        assert_eq!((app.session.actions.list[0].name.as_str(), app.session.actions.list[0].set.as_deref()), ("B", Some("One")));
        // C dropped between A (One) and nothing: the row above wins.
        move_row(&mut app, DragRow { action: 2, step: None }, 2);
        assert_eq!((app.session.actions.list[2].name.as_str(), app.session.actions.list[2].set.as_deref()), ("C", Some("One")));
    }

    #[test]
    fn insert_stop_dialog_adds_a_stop_after_the_selected_step() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.session.actions.list.push(Action::new("A", vec![("select.all".into(), json!({})), ("select.inverse".into(), json!({}))]));
        assert!(insert_stop_dialog(&mut app).is_err(), "needs a selected action");
        app.ui.actions.selected = Some(0);
        app.ui.actions.selected_step = Some(0);
        let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        assert!(owns_insert_stop(&d.fields));
        assert_eq!(d.fields["at"], 1);
        d.fields.insert("message".into(), json!("Check the mask"));
        d.fields.insert("allowContinue".into(), json!(true));
        crate::dialogs::confirm(&mut app, id).unwrap();
        let steps = &app.session.actions.list[0].steps;
        assert_eq!(steps[1], (photocraft_engine::atn_cmds::STOP_MESSAGE.to_string(), json!({"message": "Check the mask", "allowContinue": true})));
        assert_eq!((app.ui.actions.selected, app.ui.actions.selected_step), (Some(0), Some(1)));
        assert_eq!(step_label(&steps[1].0, &steps[1].1), "Stop: Check the mask");
        // Without a selected step the stop goes last; the panel's menu wiring is exercised by name.
        app.ui.actions.selected_step = None;
        let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["at"], 3);
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert_eq!(app.session.actions.list[0].steps.len(), 3);
    }

    #[test]
    fn dialog_worker_failure_stops_action_before_following_steps() {
        for (background, locked) in [(true, true), (false, true), (true, false)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
            let mut action = Action::new(
                "Worker Result",
                vec![
                    ("layer.setProps".into(), json!({"name": "Prefix", "locks": {"pixels": locked}})),
                    ("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})),
                    ("layer.new.layer".into(), json!({"name": "Suffix"})),
                ],
            );
            action.dialogs.insert(1);
            app.session.actions.list.push(action);
            app.background_jobs = background;
            let paused = play(&mut app, &json!({"action": "Worker Result"})).unwrap();
            let dialog = paused["paused"]["dialog"].as_u64().unwrap();
            let result = crate::dialogs::confirm(&mut app, dialog);
            let suffix = app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "Suffix");

            assert_eq!(suffix, !locked, "background={background}, locked={locked}: {result:?}");
            if locked && background {
                assert!(result.as_ref().is_err_and(|e| e.contains("locked")), "failed worker must fail the dialog confirmation: {result:?}");
                assert!(app.ui.status_error, "the action must report the worker failure");
                assert!(app.ui.status.contains("locked"), "{}", app.ui.status);
                assert_eq!(app.session.jobs_with_recent()[0].state, "failed");
            } else if locked {
                assert!(result.is_err(), "the foreground command rejects the locked layer");
            } else {
                assert!(result.is_ok(), "the successful worker lets the action continue: {result:?}");
            }
            assert_eq!(app.session.actions.playing, 0);
            assert!(app.session.actions.playback_stack.is_empty());
        }
    }

    #[test]
    fn a_dialog_toggled_step_pauses_playback_and_resumes_on_ok() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut a = Action::new(
            "Blur",
            vec![("layer.new.layer".into(), json!({})), ("filter.blur.gaussianBlur".into(), json!({"radius": 3})), ("select.all".into(), json!({}))],
        );
        a.dialogs.insert(1);
        app.session.actions.list.push(a);
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        assert_eq!(v["ran"], 1, "{v}");
        assert_eq!(v["paused"]["step"], 1);
        let id = v["paused"]["dialog"].as_u64().unwrap();
        let d = app.ui.dialog_mut(id).unwrap();
        assert_eq!(d.fields["radius"], 3, "the dialog opens on the recorded values");
        assert!(d.fields[RESUME].is_u64(), "the dialog carries only the pause's token: {}", d.fields[RESUME]);
        assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing after the dialog ran yet");
        report_play(&mut app, &v);
        assert!(app.ui.status.contains("dialog"), "{}", app.ui.status);
        // OK runs the blur with the dialog's values and plays the rest.
        app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!(5));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.ui.dialogs.is_empty());
        assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the dialog ran");
        assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
        // Cancel stops the action there.
        app.run("select.deselect", json!({})).unwrap();
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_none());
        assert!(app.ui.status.contains("stopped"), "{}", app.ui.status);
        // With background jobs (the desktop default) the resumed steps wait for the dialog's
        // filter to land instead of meeting a locked document.
        app.background_jobs = true;
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the background blur ran");
        assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
        app.background_jobs = false;
        // Agents play headlessly: no pause, the recorded values run.
        app.automation_input = true;
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        assert_eq!(v["ran"], 3, "{v}");
        assert!(v.get("paused").is_none());
    }

    /// An 8×6 document, a Child action whose first step (a blur) shows its dialog and whose
    /// second step adds a "Suffix" layer, and recording into Owner started.
    fn recording_with_dialog_child() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
        let mut child =
            Action::new("Child", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})), ("layer.new.layer".into(), json!({"name": "Suffix"}))]);
        child.dialogs.insert(0);
        app.session.actions.list.push(child);
        app.run("actions.record", json!({"name": "Owner"})).unwrap();
        app
    }

    fn suffixes(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().doc.walk().iter().filter(|(_, _, l)| l.name == "Suffix").count()
    }

    fn child_call() -> Vec<(String, Value)> {
        vec![("actions.play".to_string(), json!({"action": "Child"}))]
    }

    /// Play Child and return the id of its step dialog.
    fn pause_child(app: &mut PhotocraftApp) -> u64 {
        let v = play(app, &json!({"action": "Child"})).unwrap();
        v["paused"]["dialog"].as_u64().unwrap()
    }

    #[test]
    fn a_recorded_call_paused_by_a_dialog_records_one_call_and_replays_once() {
        let mut app = recording_with_dialog_child();
        let refusal = crate::i18n::t("Confirm or cancel the open action step dialog first").to_string();

        // Confirm: the dialog command and the resumed rest stay inside the one named call.
        let live = app.session.journal.len();
        let id = pause_child(&mut app);
        assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
        let names = |app: &PhotocraftApp| app.session.actions.list.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        let before = names(&app);
        assert_eq!(app.run("actions.record", json!({"name": "Intruder"})), Err(refusal.clone()), "no second recording while the call is suspended");
        assert_eq!(app.run("actions.delete", json!({"action": "Owner"})), Err(refusal.clone()), "the suspended owner can't be deleted");
        assert_eq!(app.run("actions.move", json!({"action": "Owner", "to": 0})), Err(refusal.clone()), "the suspended owner can't be moved");
        assert_eq!(names(&app), before, "the refused commands leave the list alone");
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.session.actions.recording.is_some(), "recording comes back when the call completes");
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call());
        assert_eq!(suffixes(&app), 1);

        // Replaying the recording repeats the call once.
        app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
        let start = app.session.journal.len();
        app.automation_input = true;
        let v = play(&mut app, &json!({"action": "Owner"})).unwrap();
        app.automation_input = false;
        assert!(v.get("failed").is_none(), "{v}");
        assert_eq!(suffixes(&app), 1, "replay creates the suffix once");
        let edits = |from: usize| -> Vec<(String, Value)> {
            app.session.journal[from..].iter().filter(|(id, _)| !id.starts_with("actions.") && id != "file.new").cloned().collect()
        };
        let expected = vec![("filter.blur.gaussianBlur".to_string(), json!({"radius": 1.5})), ("layer.new.layer".to_string(), json!({"name": "Suffix"}))];
        assert_eq!(edits(start), expected, "replay journals the blur and the suffix once each, in order");
        assert_eq!(edits(live), expected[..].iter().chain(&expected).cloned().collect::<Vec<_>>(), "live then replay");

        // Cancel: the call ends at the dialog and is recorded once.
        app.run("actions.record", json!({"name": "Cancelled"})).unwrap();
        let id = pause_child(&mut app);
        crate::dialogs::cancel(&mut app, id).unwrap();
        assert!(app.session.actions.recording.is_some(), "cancel restores recording");
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[2].steps, child_call());
    }

    #[test]
    fn a_failing_step_dialog_command_ends_the_recorded_call_once() {
        let mut app = recording_with_dialog_child();
        let id = pause_child(&mut app);
        assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
        app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
        assert!(app.session.actions.recording.is_some(), "recording comes back when the dialog command fails");
        assert_eq!(suffixes(&app), 0, "the action stops at the failed step");
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call());
    }

    #[test]
    fn an_abandoned_step_dialog_leaves_no_paused_playback() {
        let mut app = recording_with_dialog_child();
        let id = pause_child(&mut app);
        app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
        assert!(app.paused_playbacks.paused.is_empty(), "the failed dialog ends its pause");
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call(), "the call is recorded once");
        app.run("actions.record", json!({"name": "Next"})).unwrap();
    }

    #[test]
    fn a_failing_resumed_playback_ends_the_recorded_call_once() {
        let mut app = recording_with_dialog_child();
        let id = pause_child(&mut app);
        // The rest of the call can't be found any more: resuming it fails.
        app.session.actions.list[0].name = "Renamed".into();
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the resumed playback fails");
        assert!(app.session.actions.recording.is_some(), "recording comes back when the resumed playback fails");
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call());
    }

    #[test]
    fn a_suspended_call_is_recorded_into_its_action_after_the_list_changes() {
        let mut app = recording_with_dialog_child();
        let id = pause_child(&mut app);
        // Something else puts an action where Owner was.
        app.session.actions.list.insert(1, Action::new("Bystander", vec![]));
        crate::dialogs::confirm(&mut app, id).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert!(app.session.actions.list[1].steps.is_empty(), "the action now at the old index gets nothing");
        assert_eq!(app.session.actions.list[2].name, "Owner");
        assert_eq!(app.session.actions.list[2].steps, child_call());
    }

    #[test]
    fn escape_on_a_step_dialog_ends_the_recorded_call_once() {
        let app = recording_with_dialog_child();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(2);
        pause_child(h.state_mut());
        h.run_steps(4);
        assert!(h.state().session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.dialogs.is_empty(), "Esc closes the step dialog");
        assert!(h.state().session.actions.recording.is_some(), "Esc restores recording");
        let app = h.state_mut();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call());
    }

    #[test]
    fn a_step_dialog_keeps_its_continuation_token() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        for (name, steps) in [
            ("First", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]),
            ("Second", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]),
        ] {
            let mut action = Action::new(name, steps);
            action.dialogs.insert(0);
            app.session.actions.list.push(action);
        }
        let first = play(&mut app, &json!({"action": "First"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
        let second = play(&mut app, &json!({"action": "Second"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
        let first_token = app.ui.dialog_mut(first).unwrap().fields[RESUME].clone();
        let second_token = app.ui.dialog_mut(second).unwrap().fields[RESUME].clone();
        for token in [second_token.clone(), Value::Null, json!("invalid"), json!({"action": "Second", "from": 1})] {
            let (request, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": first, "field": RESUME, "value": token}));
            let response = crate::control::handle(&mut app, &egui::Context::default(), &request);
            assert!(matches!(response, crate::control::Outcome::Done(ref v) if v["error"].as_str().is_some_and(|e| e.contains("read-only"))));
            assert_eq!(app.ui.dialog_mut(first).unwrap().fields[RESUME], first_token);
            assert_eq!(app.ui.dialog_mut(second).unwrap().fields[RESUME], second_token);
            assert_eq!(app.paused_playbacks.paused.len(), 2);
        }
        crate::dialogs::confirm(&mut app, first).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_none(), "First must not run Second's remaining step");
        crate::dialogs::confirm(&mut app, second).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some());
        assert!(app.paused_playbacks.paused.is_empty());
    }

    fn recording_with_displace_child() -> PhotocraftApp {
        let mut app = recording_with_dialog_child();
        app.background_jobs = false;
        app.session.actions.list[0].steps[0] = ("filter.distort.displace".into(), json!({}));
        app
    }

    fn assert_displace_call_finished(app: &mut PhotocraftApp, suffix_count: usize) {
        assert!(!app.file_dialog_open());
        assert!(app.ui.dialogs.is_empty());
        assert!(app.paused_playbacks.paused.is_empty());
        assert!(app.session.actions.recording.is_some());
        assert_eq!(suffixes(app), suffix_count);
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, child_call(), "record the child call exactly once");
        app.run("actions.record", json!({"name": "Next"})).unwrap();
    }

    #[test]
    fn a_displace_step_restores_recording_when_the_map_picker_fails_or_is_cancelled() {
        use crate::file_dialog::FileDialogAnswer;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.png").to_string_lossy().into_owned();
        for answer in [
            None,
            Some(FileDialogAnswer::Unavailable("no picker available".into())),
            Some(FileDialogAnswer::Paths(Vec::new())),
            Some(FileDialogAnswer::Contents("map.png".into(), Vec::new())),
            Some(FileDialogAnswer::Paths(vec![missing])),
        ] {
            let mut app = recording_with_displace_child();
            let (show, _) = crate::file_dialog::fake(vec![answer]);
            app.services.file_dialog = Some(show);
            let id = pause_child(&mut app);
            crate::dialogs::confirm(&mut app, id).unwrap();
            assert!(app.file_dialog_open());
            assert!(app.session.actions.recording.is_none());
            app.poll_file_dialog(&egui::Context::default(), None);
            assert_displace_call_finished(&mut app, 0);
        }
    }

    #[test]
    fn a_displace_step_restores_recording_when_the_map_picker_cannot_start() {
        let mut app = recording_with_displace_child();
        let id = pause_child(&mut app);
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert_displace_call_finished(&mut app, 0);
    }

    #[test]
    fn a_displace_step_resumes_and_records_its_call_once_after_choosing_a_map() {
        let mut app = recording_with_displace_child();
        let dir = tempfile::tempdir().unwrap();
        let map = dir.path().join("map.png");
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
        std::fs::write(&map, bytes).unwrap();
        let (show, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![map.to_string_lossy().into_owned()]))]);
        app.services.file_dialog = Some(show);
        let id = pause_child(&mut app);
        crate::dialogs::confirm(&mut app, id).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_displace_call_finished(&mut app, 1);
    }

    #[test]
    fn a_step_dialog_takes_only_the_steps_values() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 2, "__command": "select.all", "__label": "Other"}))]);
        a.dialogs.insert(0);
        app.session.actions.list.push(a);
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        let d = app.ui.dialog_mut(v["paused"]["dialog"].as_u64().unwrap()).unwrap();
        assert_eq!(d.fields["radius"], 2);
        assert_eq!(d.fields["__command"], "filter.blur.gaussianBlur");
        assert_ne!(d.fields["__label"], "Other");
    }

    fn deny_blur(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "filter.blur.gaussianBlur" { Err(photocraft_engine::EngineError::Other("blur denied".into())) } else { Ok(()) }
    }

    #[test]
    fn a_step_the_gate_denies_opens_no_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
        a.dialogs.insert(0);
        app.session.actions.list.push(a);
        app.session.authorize = Some(deny_blur);
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        app.session.authorize = None;
        assert!(v["failed"]["error"].as_str().is_some_and(|e| e.contains("blur denied")), "{v}");
        assert!(app.ui.dialogs.is_empty(), "no step dialog opened");
        assert!(app.paused_playbacks.paused.is_empty());
    }

    fn deny_select_all(id: &str, _: &Value) -> photocraft_engine::Result<()> {
        if id == "select.all" { Err(photocraft_engine::EngineError::Other("select.all denied".into())) } else { Ok(()) }
    }

    /// An action an automation client plays (`control::handle` installs the gate, then removes
    /// it) pauses at a step dialog; confirming the dialog later, here with synthetic Enter or
    /// OK, runs the remaining steps under the same gate.
    #[test]
    fn a_paused_playback_keeps_its_automation_gate_when_the_dialog_is_confirmed() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut a = Action::new("Seeded", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]);
        a.dialogs.insert(0);
        app.session.actions.list.push(a);
        app.session.authorize = Some(deny_select_all);
        let v = play(&mut app, &json!({"action": "Seeded"})).unwrap();
        app.session.authorize = None;
        let id = v["paused"]["dialog"].as_u64().unwrap();
        app.automation_input = true;
        let _ = crate::dialogs::confirm(&mut app, id);
        app.automation_input = false;
        assert!(app.session.active().unwrap().doc.selection.is_none(), "the denied step did not run");
        assert!(app.ui.status.contains("select.all denied"), "{}", app.ui.status);
    }

    /// The dialog names only its pause: changing its fields can't resume another action.
    #[test]
    fn a_step_dialog_resumes_only_the_playback_that_paused_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
        a.dialogs.insert(0);
        app.session.actions.list.push(a);
        app.session.actions.list.push(Action::new("Seeded", vec![("select.all".into(), json!({}))]));
        let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert(RESUME.into(), json!({"action": "Seeded", "from": 0}));
        let r = crate::dialogs::confirm(&mut app, id);
        assert!(app.session.active().unwrap().doc.selection.is_none(), "the other action did not play");
        assert!(r.is_err_and(|e| e.contains("paused action")));
    }

    #[test]
    fn button_mode_plays_the_clicked_action_and_hides_its_steps() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.session.actions.list.push(Action::new("Print", vec![("layer.new.layer".into(), json!({}))]));
        app.session.actions.list[0].color = "blue".into();
        app.ui.actions.expanded = vec![true];
        app.ui.actions.button_mode = true;
        let mut h = Harness::builder().with_size(vec2(420.0, 280.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 0, "button mode hides the step list");
        let before = h.state().session.active().unwrap().doc.layers.len();
        h.get_by_label("Print").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1, "a click plays the action");
        // Button Mode is a panel menu item in Photoshop, not a button.
        assert!(h.query_by_label("Button Mode").is_none());
        h.state_mut().ui.actions.button_mode = false;
        h.run_steps(2);
        assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 1, "the step list returns when Button Mode is turned off");
    }

    /// An imported Close with Save: Yes plays as Save then Close. A Save Document script event
    /// that closes the saved document stops playback, so the Close never reaches another dirty
    /// document (#7367).
    #[test]
    fn save_hook_closing_the_target_stops_an_imported_close_with_save() {
        use photocraft_psd::atn::{self, AtnEvent, AtnItem};
        use photocraft_psd::descriptor::{Descriptor, Id, Value as D};
        use std::{cell::RefCell, rc::Rc};
        // A one-step .atn: Close (`Cls `) with Save: Yes.
        let seed = Action::new("Seed", vec![("layer.new.layer".into(), json!({}))]);
        let (bytes, _) = photocraft_engine::atn_cmds::export("Set", &[&seed], &[], false).unwrap();
        let mut f = atn::parse(&bytes).unwrap();
        f.actions[0].items = vec![AtnItem {
            event: AtnEvent::Code(*b"Cls "),
            dictionary: "Close".into(),
            descriptor: Some(Descriptor::new("null").with("Svng", D::Enumerated { type_id: Id::new("YsN "), value: Id::new("Ys  ") })),
            ..Default::default()
        }];
        let imported = photocraft_engine::atn_cmds::import(&atn::write(&f).unwrap()).unwrap();
        let close_with_save = imported.actions[0].clone();
        assert_eq!(close_with_save.steps, [("file.save".into(), json!({})), ("file.close".into(), json!({}))]);
        for hook in ["none", "activate", "close"] {
            let writes = Rc::new(RefCell::new(Vec::<String>::new()));
            let w = writes.clone();
            let services = crate::Services {
                export: Some(Box::new(|doc, _, _| Ok((doc.name.clone().into_bytes(), Vec::new())))),
                write: Some(Box::new(move |path, _| {
                    w.borrow_mut().push(path.to_string());
                    Ok(())
                })),
                ..Default::default()
            };
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            // B is dirty with an unsaved KeepB layer; A is active and has its own file.
            for name in ["B", "A"] {
                app.run("file.new", json!({"name": name, "width": 2, "height": 2})).unwrap();
            }
            assert!(app.session.set_active(0));
            app.run("layer.new.layer", json!({"name": "KeepB"})).unwrap();
            assert!(app.session.set_active(1));
            app.session.active_mut().unwrap().path = Some("A.psd".into());
            app.session.actions.list.push(close_with_save.clone());
            let steps = match hook {
                "activate" => json!([["document.activate", {"document": 0}]]),
                "close" => json!([["file.close", {}]]),
                _ => Value::Null,
            };
            if !steps.is_null() {
                app.run("file.scripts.scriptEventsManager", json!({"add": {"event": "saveDocument", "steps": steps}})).unwrap();
            }
            let r = app.run("actions.play", json!({"action": 0})).unwrap();
            assert_eq!(*writes.borrow(), ["A.psd"], "hook={hook}: A is saved once");
            let names: Vec<Vec<String>> = app.session.documents().iter().map(|d| d.doc.layers.iter().map(|l| l.name.clone()).collect()).collect();
            assert_eq!(app.session.documents().len(), 1, "hook={hook}: only A closes; result={r}");
            assert_eq!(app.session.active().unwrap().doc.name, "B", "hook={hook}");
            assert_eq!(names, [["Background", "KeepB"]], "hook={hook}: B stays open with its unsaved layer");
            if hook == "close" {
                assert_eq!(r["failed"]["step"], 0, "{r}");
                assert_eq!(r["failed"]["id"], "file.save", "{r}");
                assert!(r["failed"]["error"].as_str().unwrap().contains("closed during the save"), "{r}");
            } else {
                assert!(r.get("failed").is_none(), "hook={hook}: {r}");
            }
            assert_eq!(app.session.actions.playing, 0);
            assert!(app.session.actions.playback_stack.is_empty());
        }
    }
}
