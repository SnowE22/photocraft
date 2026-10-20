use super::*;
use photocraft_engine::actions_cmds::Action;

fn app_with(actions: Vec<Action>) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session.actions.list = actions;
    app
}

fn layer(name: &str) -> (String, Value) {
    ("layer.new.layer".to_string(), json!({"name": name}))
}

fn step_names(app: &PhotocraftApp, action: usize) -> Vec<String> {
    app.session.actions.list[action].steps.iter().map(|(_, p)| p["name"].as_str().unwrap_or("").to_string()).collect()
}

fn set_field(app: &mut PhotocraftApp, dialog: u64, key: &str, value: Value) {
    app.ui.dialog_mut(dialog).unwrap().fields.insert(key.into(), value);
}

/// New Action… names the action, picks its set, key and colour; Record starts recording into it.
#[test]
fn new_action_records_into_the_chosen_set_with_its_key_and_color() {
    let mut app = app_with(vec![]);
    let set = new_set(&mut app);
    set_field(&mut app, set, "name", json!("Mine"));
    crate::dialogs::confirm(&mut app, set).unwrap();
    assert_eq!(app.ui.actions.selected_set.as_deref(), Some("Mine"));
    let d = new_action(&mut app);
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["set"], "Mine", "the selected set");
    assert_eq!(ok_label(&app.ui.dialog_mut(d).unwrap().fields), Some("Record"));
    set_field(&mut app, d, "name", json!("Grain"));
    set_field(&mut app, d, "key", json!(5));
    set_field(&mut app, d, "shift", json!(true));
    set_field(&mut app, d, "color", json!("blue"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert!(app.session.actions.recording.is_some(), "Record starts recording");
    app.run("layer.new.layer", json!({"name": "A"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let a = &app.session.actions.list[0];
    assert_eq!((a.name.as_str(), a.set.as_deref(), a.color.as_str()), ("Grain", Some("Mine"), "blue"));
    assert_eq!(app.session.prefs().shortcuts.get("actions.play:Grain").map(String::as_str), Some("Shift+F5"));
    assert_eq!(app.ui.actions.selected, Some(0));
    // With no set at all, New Action makes Default Actions.
    let mut empty = app_with(vec![]);
    let d = new_action(&mut empty);
    crate::dialogs::confirm(&mut empty, d).unwrap();
    assert_eq!(empty.session.actions.list[0].set.as_deref(), Some(DEFAULT_SET));
}

/// Action Options… renames the action and changes its key and colour.
#[test]
fn action_options_rename_and_rekey() {
    let mut app = app_with(vec![Action::new("Old", vec![layer("A")])]);
    let d = action_options(&mut app, 0).unwrap();
    set_field(&mut app, d, "name", json!("New"));
    set_field(&mut app, d, "key", json!(3));
    set_field(&mut app, d, "color", json!("red"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    let a = &app.session.actions.list[0];
    assert_eq!((a.name.as_str(), a.color.as_str()), ("New", "red"));
    assert_eq!(app.session.prefs().shortcuts.get("actions.play:New").map(String::as_str), Some("F3"));
    let d = set_options(&mut app, "Nope");
    assert!(crate::dialogs::confirm(&mut app, d).is_err(), "no such set");
}

/// Playback Options… switches to Step by Step; the panel then plays one step a frame, with the
/// step selected.
#[test]
fn step_by_step_plays_one_step_a_frame() {
    let mut app = app_with(vec![Action::new("Three", vec![layer("1"), layer("2"), layer("3")])]);
    let d = playback_options(&mut app);
    set_field(&mut app, d, "mode", json!("stepByStep"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.actions.prefs.playback, Playback::StepByStep);
    crate::actions::start(&mut app, vec!["Three".into()], 0, None);
    let ctx = egui::Context::default();
    let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
    let mut seen = Vec::new();
    for t in 0..6 {
        ctx.run_ui(egui::RawInput { time: Some(f64::from(t)), ..Default::default() }, |ui| crate::actions::tick(&mut app, ui.ctx())).textures_delta.clear();
        seen.push((layers(&app), app.ui.actions.selected_step));
    }
    assert_eq!(seen[..3], [(2, Some(0)), (3, Some(1)), (4, Some(2))], "one step a frame, the running step selected");
    assert!(app.action_run.is_none(), "done");
    assert_eq!(layers(&app), 4);
}

/// Insert Conditional… adds a conditional step after the selected step.
#[test]
fn insert_conditional_adds_the_step() {
    let mut app = app_with(vec![Action::new("A", vec![layer("1"), layer("2")]), Action::new("B", vec![])]);
    assert!(insert_conditional(&mut app).is_err(), "needs an action");
    app.ui.actions.selected = Some(0);
    app.ui.actions.select_step(0);
    let d = insert_conditional(&mut app).unwrap();
    set_field(&mut app, d, "if", json!("landscape"));
    set_field(&mut app, d, "then", json!("B"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    let step = &app.session.actions.list[0].steps[1];
    assert_eq!(step.0, "actions.conditional");
    assert_eq!(step.1, json!({"if": "landscape", "then": "B", "else": null}));
    assert_eq!(app.ui.actions.selected_step, Some(1));
}

/// The action that played is selected, so the panel shows which one ran, from a key or anywhere.
#[test]
fn the_action_that_played_is_selected() {
    let mut a = Action::new("First", vec![layer("1")]);
    a.set = Some("S".into());
    let mut b = Action::new("Second", vec![layer("2")]);
    b.set = Some("S".into());
    let mut app = app_with(vec![a, b]);
    app.ui.actions.collapsed_sets = vec!["S".into()];
    app.ui.actions.selected = Some(0);
    app.ui.actions.select_step(0);
    app.run("actions.play", json!({"action": "Second"})).unwrap();
    assert_eq!(app.ui.actions.selected, Some(1));
    assert!(app.ui.actions.selected_step.is_none());
    assert!(app.ui.actions.collapsed_sets.is_empty(), "its set opens to show it");
    // Played again with a step picked in it, the step stays picked.
    app.ui.actions.select_step(0);
    app.run("actions.play", json!({"action": "Second"})).unwrap();
    assert_eq!(app.ui.actions.selected_step, Some(0));
}

/// Ctrl-click and Shift-click pick several steps; Delete removes them all at once.
#[test]
fn several_steps_are_picked_and_deleted_together() {
    let steps = ["1", "2", "3", "4", "5"].map(layer).to_vec();
    let mut app = app_with(vec![Action::new("A", steps)]);
    let click = |app: &mut PhotocraftApp, step: usize, ctrl: bool, shift: bool| crate::actions::tests::click_step(app, 0, step, ctrl, shift);
    click(&mut app, 0, false, false);
    click(&mut app, 2, true, false);
    assert_eq!(app.ui.actions.step_selection(), [0, 2]);
    click(&mut app, 4, false, true);
    assert_eq!(app.ui.actions.step_selection(), [2, 3, 4], "Shift picks from the last picked step");
    click(&mut app, 3, true, false);
    assert_eq!(app.ui.actions.step_selection(), [2, 4], "Ctrl takes one out");
    let picked = app.ui.actions.step_selection();
    let d = confirm_delete(&mut app, Target::Steps(0, picked));
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["message"], "Delete the selection?");
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(step_names(&app, 0), ["1", "2", "4"]);
    assert!(app.ui.actions.step_selection().is_empty());
    // A plain click starts over.
    click(&mut app, 1, true, false);
    click(&mut app, 0, false, false);
    assert_eq!(app.ui.actions.step_selection(), [0]);
}

/// Recording with a step selected records after that step.
#[test]
fn recording_starts_after_the_selected_step() {
    let mut app = app_with(vec![Action::new("A", vec![layer("1"), layer("2")])]);
    app.ui.actions.selected = Some(0);
    app.ui.actions.select_step(0);
    crate::actions::tests::record_button(&mut app);
    app.run("layer.new.layer", json!({"name": "new"})).unwrap();
    assert_eq!(crate::actions::visible_steps(&app, 0).iter().map(|(_, p)| p["name"].as_str().unwrap()).collect::<Vec<_>>(), ["1", "new", "2"]);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(step_names(&app, 0), ["1", "new", "2"]);
    // No action selected: New Action… asks first.
    app.ui.actions.selected = None;
    crate::actions::tests::record_button(&mut app);
    assert!(app.session.actions.recording.is_none());
    assert!(app.ui.dialogs.iter().any(|d| kind(&d.fields) == "newAction"));
}

/// Record Again… opens the step's dialog with its values and stores what OK confirms.
#[test]
fn record_again_stores_the_new_values() {
    let mut app = app_with(vec![Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 2.0}))])]);
    let r = crate::actions::tests::record_again_step(&mut app, 0, 0).unwrap();
    let d = r["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(d).unwrap().fields["radius"], json!(2.0), "the recorded value");
    set_field(&mut app, d, "radius", json!(5.0));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.actions.list[0].steps[0].1["radius"], json!(5.0));
    // A step without a dialog has nothing to record again.
    app.session.actions.list[0].steps.push(layer("x"));
    assert!(crate::actions::tests::record_again_step(&mut app, 0, 1).is_err());
}

/// Delete asks first; deleting a set takes its actions.
#[test]
fn deleting_a_set_asks_and_takes_its_actions() {
    let mut a = Action::new("A", vec![]);
    a.set = Some("S".into());
    let mut app = app_with(vec![a, Action::new("B", vec![])]);
    let d = confirm_delete(&mut app, Target::Set("S".into()));
    assert!(app.ui.dialog_mut(d).unwrap().fields["message"].as_str().unwrap().contains("“S”"));
    crate::dialogs::confirm(&mut app, d).unwrap();
    assert_eq!(app.session.actions.list.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["B"]);
}
