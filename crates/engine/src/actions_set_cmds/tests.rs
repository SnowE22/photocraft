use super::*;
use crate::actions_cmds::{DEFAULT_SET, Playback};
use serde_json::json;

fn names(s: &Session) -> Vec<(String, Option<String>)> {
    s.actions.list.iter().map(|a| (a.name.clone(), a.set.clone())).collect()
}

fn layer(name: &str) -> (String, Value) {
    ("layer.new.layer".to_string(), json!({"name": name}))
}

fn step_names(s: &Session, action: usize) -> Vec<String> {
    s.actions.list[action].steps.iter().map(|(_, p)| p["name"].as_str().unwrap_or("").to_string()).collect()
}

/// New Set makes an empty folder; renaming moves its actions; deleting takes them with it.
#[test]
fn sets_are_made_renamed_and_deleted_with_their_actions() {
    let mut s = Session::new();
    assert_eq!(s.execute("actions.newSet", json!({})).unwrap()["set"], "Set 1");
    assert_eq!(s.execute("actions.newSet", json!({})).unwrap()["set"], "Set 2");
    assert_eq!(s.actions.sets(), ["Set 1", "Set 2"], "empty sets are kept");
    assert!(s.execute("actions.newSet", json!({"name": "Set 1"})).is_err(), "names are unique");
    s.execute("actions.record", json!({"name": "A", "set": "Set 2"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("actions.record", json!({"name": "B", "set": "Set 1"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("actions.record", json!({"name": "C", "set": "Set 2"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(
        names(&s),
        [("A".into(), Some("Set 2".into())), ("C".into(), Some("Set 2".into())), ("B".into(), Some("Set 1".into()))],
        "a new action goes last in its set"
    );
    s.execute("actions.renameSet", json!({"set": "Set 2", "name": "Mine"})).unwrap();
    assert_eq!(s.actions.sets(), ["Set 1", "Mine"], "renaming keeps the set's place");
    assert_eq!(s.actions.list[0].set.as_deref(), Some("Mine"));
    let r = s.execute("actions.deleteSet", json!({"set": "Mine"})).unwrap();
    assert_eq!(r["actions"], 2);
    assert_eq!(names(&s), [("B".into(), Some("Set 1".into()))]);
    assert_eq!(s.actions.sets(), ["Set 1"]);
    // An action in a set Load Actions made gets its folder listed.
    s.actions.list.push(Action { set: Some("Loaded".into()), ..Action::new("D", vec![]) });
    assert_eq!(s.actions.sets(), ["Set 1", "Loaded"]);
    for bad in [json!({}), json!({"set": "Nope"}), json!({"set": ""}), json!({"set": 3})] {
        assert!(s.execute("actions.deleteSet", bad.clone()).is_err(), "{bad}");
        assert!(s.execute("actions.renameSet", bad.clone()).is_err(), "{bad}");
    }
    assert!(s.execute("actions.renameSet", json!({"set": "Set 1", "name": "Loaded"})).is_err(), "no two sets of one name");
}

/// Deleting a set before the action being recorded keeps the recording on that action.
#[test]
fn deleting_an_earlier_set_keeps_the_recording_attached() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.actions.list.push(Action { set: Some("Old".into()), ..Action::new("Gone", vec![]) });
    s.execute("actions.record", json!({"name": "Live", "set": DEFAULT_SET})).unwrap();
    assert!(s.execute("actions.deleteSet", json!({"set": DEFAULT_SET})).is_err(), "not the set being recorded into");
    s.execute("actions.deleteSet", json!({"set": "Old"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "X"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(step_names(&s, 0), ["X"]);
}

/// Duplicate copies an action after itself, or a set with all its actions.
#[test]
fn duplicate_copies_an_action_or_a_set() {
    let mut s = Session::new();
    s.actions.list.push(Action { set: Some("Set 1".into()), color: "red".into(), ..Action::new("Grain", vec![layer("A")]) });
    s.actions.list.push(Action { set: Some("Set 1".into()), ..Action::new("Fade", vec![]) });
    assert_eq!(s.execute("actions.duplicate", json!({"action": "Grain"})).unwrap()["action"], "Grain copy");
    assert_eq!(s.execute("actions.duplicate", json!({"action": "Grain"})).unwrap()["action"], "Grain copy 2");
    let list: Vec<&str> = s.actions.list.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(list, ["Grain", "Grain copy 2", "Grain copy", "Fade"]);
    assert_eq!(s.actions.list[2].steps, [layer("A")]);
    assert_eq!(s.actions.list[2].color, "red");
    let r = s.execute("actions.duplicate", json!({"set": "Set 1"})).unwrap();
    assert_eq!(r["set"], "Set 1 copy");
    assert_eq!(s.actions.sets(), ["Set 1", "Set 1 copy"]);
    let copies: Vec<&str> = s.actions.list.iter().filter(|a| a.set.as_deref() == Some("Set 1 copy")).map(|a| a.name.as_str()).collect();
    assert_eq!(copies, ["Grain copy 3", "Grain copy 2 copy", "Grain copy copy", "Fade copy"]);
    for bad in [json!({}), json!({"action": 99}), json!({"set": "None"})] {
        assert!(s.execute("actions.duplicate", bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn clear_all_actions_empties_the_panel() {
    let mut s = Session::new();
    s.execute("actions.newSet", json!({})).unwrap();
    s.actions.list.push(Action::new("A", vec![]));
    s.execute("actions.record", json!({"action": 0})).unwrap();
    assert!(s.execute("actions.clear", json!({})).is_err(), "not while recording");
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.execute("actions.clear", json!({})).unwrap()["deleted"], 1);
    assert!(s.actions.list.is_empty() && s.actions.sets().is_empty());
}

/// Record Again keeps the step's command and stores its new values.
#[test]
fn set_step_replaces_the_values() {
    let mut s = Session::new();
    s.actions.list.push(Action::new("A", vec![layer("Old")]));
    s.execute("actions.setStep", json!({"action": 0, "step": 0, "params": {"name": "New"}})).unwrap();
    assert_eq!(s.actions.list[0].steps, [layer("New")]);
    for bad in [json!({"action": 0, "step": 1, "params": {}}), json!({"action": 0, "step": 0}), json!({"action": 0, "params": {}}), json!({"action": 0, "step": 0, "params": 3})]
    {
        assert!(s.execute("actions.setStep", bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn options_set_tool_recording_and_playback() {
    let mut s = Session::new();
    let r = s.execute("actions.options", json!({})).unwrap();
    assert_eq!(r, json!({"allowToolRecording": false, "playback": {"mode": "accelerated"}}));
    s.execute("actions.options", json!({"allowToolRecording": true, "playback": {"mode": "pause", "seconds": 2}})).unwrap();
    assert!(s.actions.prefs.allow_tool_recording);
    assert_eq!(s.actions.prefs.playback, Playback::Pause { seconds: 2.0 });
    assert_eq!(s.actions.prefs.playback.delay(), Some(2.0));
    s.execute("actions.options", json!({"playback": {"mode": "stepByStep"}})).unwrap();
    assert!(s.actions.prefs.allow_tool_recording, "omitted keys are left alone");
    for bad in [
        json!({"allowToolRecording": "yes"}),
        json!({"playback": "fast"}),
        json!({"playback": {"mode": "pause", "seconds": -1}}),
        json!({"playback": {"mode": "pause", "seconds": 1000}}),
        json!({"playback": {"mode": "warp"}}),
    ] {
        assert!(s.execute("actions.options", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.actions.prefs.playback, Playback::StepByStep, "rejected options change nothing");
}

/// Brush strokes and brush settings are recorded only with Allow Tool Recording; View commands
/// and preferences never are, as in Photoshop.
#[test]
fn recording_leaves_out_what_photoshop_does_not_record() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let paint = |s: &mut Session| {
        s.execute("tools.setBrush", json!({"brush": {"size": 4}})).unwrap();
        s.execute("paint.stroke", json!({"points": [[2, 2], [20, 20]]})).unwrap();
        s.execute("view.newGuide", json!({"orientation": "vertical", "position": 4})).unwrap();
    };
    s.execute("actions.record", json!({"name": "Paint"})).unwrap();
    paint(&mut s);
    s.execute("actions.stop", json!({})).unwrap();
    let ids = |s: &Session, i: usize| s.actions.list[i].steps.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&s, 0), ["view.newGuide"], "only the guide: strokes and brush settings need Allow Tool Recording");
    s.execute("actions.options", json!({"allowToolRecording": true})).unwrap();
    s.execute("actions.record", json!({"name": "Tools"})).unwrap();
    paint(&mut s);
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(ids(&s, 1), ["tools.setBrush", "paint.stroke", "view.newGuide"]);
    assert!(crate::actions_cmds::never_recorded("view.fitOnScreen") && crate::actions_cmds::never_recorded("edit.keyboardShortcuts"));
    assert!(!crate::actions_cmds::recordable(&s, "view.zoomIn"), "Insert Menu Item adds View steps");
    assert!(crate::actions_cmds::recordable(&s, "layer.new.layer"));
}

/// With a step selected, recording inserts after it (Photoshop), and later edits keep the place.
#[test]
fn recording_into_an_action_inserts_after_the_selected_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.actions.list.push(Action::new("A", vec![layer("1"), layer("2"), layer("3")]));
    s.actions.list[0].disabled.insert(2);
    s.execute("actions.record", json!({"action": 0, "at": 1})).unwrap();
    s.execute("layer.new.layer", json!({"name": "x"})).unwrap();
    assert_eq!(crate::actions_cmds::pending_at(&s, 0), 1);
    s.execute("actions.toggle", json!({"action": 0, "step": 0, "enabled": true})).unwrap();
    s.execute("layer.new.layer", json!({"name": "y"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(step_names(&s, 0), ["1", "x", "y", "2", "3"]);
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [4], "the unchecked step moved with its step");
    // A step deleted before the insertion point while recording keeps it right.
    s.execute("actions.record", json!({"action": 0, "at": 2})).unwrap();
    s.execute("actions.delete", json!({"action": 0, "step": 0})).unwrap();
    s.execute("layer.new.layer", json!({"name": "z"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(step_names(&s, 0), ["x", "z", "y", "2", "3"]);
    // At the end, or without `at`, steps are appended.
    s.execute("actions.record", json!({"action": 0, "at": 5})).unwrap();
    s.execute("layer.new.layer", json!({"name": "end"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(step_names(&s, 0).last().map(String::as_str), Some("end"));
    for bad in [json!({"action": 0, "at": 99}), json!({"action": 0, "at": -1}), json!({"action": 0, "at": "x"}), json!({"name": "N", "set": 4})] {
        assert!(s.execute("actions.record", bad.clone()).is_err(), "{bad}");
        assert!(s.actions.recording.is_none());
    }
}

/// Play with `steps` plays only that many (Ctrl+Play plays the selected step alone).
#[test]
fn play_can_run_one_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.actions.list.push(Action::new("A", vec![layer("1"), layer("2"), layer("3")]));
    let r = s.execute("actions.play", json!({"action": 0, "from": 1, "steps": 1})).unwrap();
    assert_eq!(r["ran"], 1);
    let layers: Vec<String> = s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.clone()).collect();
    assert_eq!(layers, ["2"]);
    assert!(s.execute("actions.play", json!({"action": 0, "steps": -1})).is_err());
    assert!(s.execute("actions.play", json!({"action": 0, "steps": "one"})).is_err());
}

/// The sets and options are saved with the actions and come back.
#[test]
fn sets_and_options_persist_with_the_preset_store() {
    let dir = std::env::temp_dir().join(format!("photocraft-action-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.attach_preset_store(crate::preset_store::open_dir(&dir));
    s.execute("actions.newSet", json!({"name": "Empty"})).unwrap();
    s.execute("actions.options", json!({"allowToolRecording": true, "playback": {"mode": "stepByStep"}})).unwrap();
    s.sync_preset_store();
    let mut back = Session::new();
    back.attach_preset_store(crate::preset_store::open_dir(&dir));
    assert_eq!(back.actions.sets(), ["Empty"]);
    assert!(back.actions.prefs.allow_tool_recording);
    assert_eq!(back.actions.prefs.playback, Playback::StepByStep);
    let _ = std::fs::remove_dir_all(&dir);
}
