//! The Actions panel's set folders and its menu commands beyond recording and playback:
//! `actions.newSet`, `actions.renameSet`, `actions.deleteSet` (Photoshop's New Set…, Set
//! Options… and deleting a set), `actions.duplicate` (Duplicate, for an action or a whole set),
//! `actions.clear` (Clear All Actions), `actions.setStep` (Record Again… stores a step's new
//! values) and `actions.options` (Allow Tool Recording and Playback Options…).
//!
//! Like the other action-editing commands they refuse to run during playback. Action names stay
//! unique across sets, so copies are named "… copy", "… copy 2", as Photoshop names them.

use serde_json::{Value, json};

use crate::actions_cmds::{Action, Playback, flush_recording, not_playing, resolve};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// A non-blank string param, trimmed.
fn name_param(p: &Value, key: &str, cmd: &str) -> Result<Option<String>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) if !n.trim().is_empty() => Ok(Some(n.trim().to_string())),
        Some(Value::String(_)) => Err(bad(cmd, format!("`{key}` is empty"))),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a string"))),
    }
}

/// The set `p.set` names, which must exist.
fn existing_set(s: &Session, p: &Value, cmd: &str) -> Result<String> {
    let set = name_param(p, "set", cmd)?.ok_or_else(|| bad(cmd, "needs `set`, a set name"))?;
    if s.actions.sets().contains(&set) { Ok(set) } else { Err(bad(cmd, format!("no set named `{set}`"))) }
}

/// `base`, or `base N` from 2 up, whichever `taken` doesn't have yet.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..u32::MAX).map(|n| format!("{base} {n}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{base} {}", u32::MAX))
}

fn new_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.newSet";
    not_playing(s, cmd)?;
    let sets = s.actions.sets();
    let name = match name_param(p, "name", cmd)? {
        Some(name) if sets.contains(&name) => return Err(bad(cmd, format!("a set named `{name}` already exists"))),
        Some(name) => name,
        None => (1..).map(|n| format!("Set {n}")).find(|n| !sets.contains(n)).unwrap_or_default(),
    };
    s.actions.prefs.sets = sets;
    s.actions.prefs.sets.push(name.clone());
    s.actions.touch();
    Ok(json!({"set": name}))
}

fn rename_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.renameSet";
    not_playing(s, cmd)?;
    let set = existing_set(s, p, cmd)?;
    let name = name_param(p, "name", cmd)?.ok_or_else(|| bad(cmd, "needs `name`"))?;
    if name != set && s.actions.sets().contains(&name) {
        return Err(bad(cmd, format!("a set named `{name}` already exists")));
    }
    let mut sets = s.actions.sets();
    for n in &mut sets {
        if *n == set {
            *n = name.clone();
        }
    }
    s.actions.prefs.sets = sets;
    for a in s.actions.list.iter_mut().filter(|a| a.set.as_ref() == Some(&set)) {
        a.set = Some(name.clone());
    }
    s.actions.touch();
    Ok(json!({"set": name}))
}

fn delete_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.deleteSet";
    not_playing(s, cmd)?;
    let set = existing_set(s, p, cmd)?;
    if s.actions.recording.and_then(|(_, i)| s.actions.list.get(i)).is_some_and(|a| a.set.as_ref() == Some(&set)) {
        return Err(bad(cmd, "stop recording before deleting the set it records into"));
    }
    let before = s.actions.list.len();
    let recording = s.actions.recording.map(|(_, i)| i);
    let kept: Vec<bool> = s.actions.list.iter().map(|a| a.set.as_ref() != Some(&set)).collect();
    if let (Some((_, i)), Some(r)) = (s.actions.recording.as_mut(), recording) {
        *i = r - kept.iter().take(r).filter(|k| !**k).count();
    }
    let mut keep = kept.iter();
    s.actions.list.retain(|_| keep.next().copied().unwrap_or(true));
    s.actions.prefs.sets = s.actions.sets().into_iter().filter(|n| *n != set).collect();
    s.actions.touch();
    Ok(json!({"deleted": set, "actions": before - s.actions.list.len()}))
}

/// A copy of `a` named after it, unique among `list` and the names already given in `extra`.
fn copy_of(a: &Action, list: &[Action], extra: &[String]) -> Action {
    let mut copy = a.clone();
    copy.name = unique(&format!("{} copy", a.name), |n| list.iter().any(|x| x.name == n) || extra.iter().any(|x| x == n));
    copy
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.duplicate";
    not_playing(s, cmd)?;
    if p.get("action").is_none() {
        // A set: the copy, with copies of its actions, goes right after it.
        let set = existing_set(s, p, cmd)?;
        let name = unique(&format!("{set} copy"), |n| s.actions.sets().iter().any(|x| x == n));
        let mut names = Vec::new();
        let mut copies = Vec::new();
        for a in s.actions.list.iter().filter(|a| a.set.as_ref() == Some(&set)) {
            let mut copy = copy_of(a, &s.actions.list, &names);
            copy.set = Some(name.clone());
            names.push(copy.name.clone());
            copies.push(copy);
        }
        let mut sets = s.actions.sets();
        let at = sets.iter().position(|n| *n == set).map_or(sets.len(), |i| i + 1);
        sets.insert(at, name.clone());
        s.actions.prefs.sets = sets;
        s.actions.list.extend(copies);
        s.actions.touch();
        return Ok(json!({"set": name, "actions": names}));
    }
    let idx = resolve(&s.actions.list, p, cmd)?;
    if s.actions.recording.is_some_and(|(_, i)| i == idx) {
        flush_recording(s)?;
    }
    let copy = copy_of(&s.actions.list[idx], &s.actions.list, &[]);
    let name = copy.name.clone();
    s.actions.list.insert(idx + 1, copy);
    if let Some((_, i)) = s.actions.recording.as_mut()
        && *i > idx
    {
        *i += 1;
    }
    s.actions.touch();
    Ok(json!({"action": name, "index": idx + 1}))
}

fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    let cmd = "actions.clear";
    not_playing(s, cmd)?;
    if s.actions.recording.is_some() {
        return Err(bad(cmd, "stop recording first"));
    }
    let n = s.actions.list.len();
    s.actions.list.clear();
    s.actions.prefs.sets.clear();
    s.actions.touch();
    Ok(json!({"deleted": n}))
}

fn set_step(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.setStep";
    not_playing(s, cmd)?;
    let idx = resolve(&s.actions.list, p, cmd)?;
    let step = p.get("step").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(cmd, "`step` must be a non-negative integer"))?;
    let params = match p.get("params") {
        Some(v @ Value::Object(_)) => v.clone(),
        _ => return Err(bad(cmd, "needs `params`, an object")),
    };
    if s.actions.recording.is_some_and(|(_, i)| i == idx) {
        flush_recording(s)?;
    }
    let action = &mut s.actions.list[idx];
    let Some((id, old)) = action.steps.get_mut(step) else {
        return Err(bad(cmd, "step is out of range"));
    };
    *old = params;
    let result = json!({"action": action.name, "step": step, "id": id});
    s.actions.touch();
    Ok(result)
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.options";
    let tools = match p.get("allowToolRecording") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad(cmd, "`allowToolRecording` must be true or false")),
    };
    let playback = match p.get("playback") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let playback: Playback = serde_json::from_value(v.clone())
                .map_err(|_| bad(cmd, r#"`playback` must be {"mode":"accelerated"}, {"mode":"stepByStep"} or {"mode":"pause","seconds":n}"#))?;
            if let Playback::Pause { seconds } = playback
                && !(0.0..=crate::actions_cmds::MAX_PAUSE_S).contains(&seconds)
            {
                return Err(bad(cmd, format!("pause must be 0–{} seconds", crate::actions_cmds::MAX_PAUSE_S)));
            }
            Some(playback)
        }
    };
    let prefs = &mut s.actions.prefs;
    if let Some(t) = tools {
        prefs.allow_tool_recording = t;
    }
    if let Some(pb) = playback {
        prefs.playback = pb;
    }
    let result = json!({"allowToolRecording": prefs.allow_tool_recording, "playback": prefs.playback});
    if tools.is_some() || playback.is_some() {
        s.actions.touch();
    }
    Ok(result)
}

pub fn specs() -> Vec<CommandSpec> {
    let spec = |id, label, params, run| CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: always, run, journal: false };
    vec![
        spec("actions.newSet", "New Set", r##"{"name":str?="Set N"} → {set}. A new, empty set folder at the end of the Actions panel."##, new_set),
        spec("actions.renameSet", "Rename Set", r##"{"set":name, "name":str} → {set}. Set Options…: the set's actions move with it."##, rename_set),
        spec(
            "actions.deleteSet",
            "Delete Set",
            r##"{"set":name} → {deleted, actions:count}. Deletes the set folder and every action in it; refused while recording into one of them."##,
            delete_set,
        ),
        spec(
            "actions.duplicate",
            "Duplicate Action",
            r##"{"action":name|index} → {action, index} or {"set":name} → {set, actions:[name…]}. The copy goes right after the original, named "… copy" (action names are unique across sets)."##,
            duplicate,
        ),
        spec("actions.clear", "Clear All Actions", r##"{} → {deleted:count}. Deletes every action and set; refused while recording."##, clear),
        spec(
            "actions.setStep",
            "Record Step Again",
            r##"{"action":name|index, "step":index, "params":{}} → {action, step, id}. Record Again…: the step keeps its command with new params."##,
            set_step,
        ),
        spec(
            "actions.options",
            "Actions Options",
            r##"{"allowToolRecording":bool?, "playback":{"mode":"accelerated"|"stepByStep"}|{"mode":"pause","seconds":0..60}?} → {allowToolRecording, playback}. Allow Tool Recording records painting and toning strokes and brush settings; Playback Options paces playback in the app (headless playback is always accelerated). Omitted keys are left alone."##,
            options,
        ),
    ]
}

#[cfg(test)]
mod tests;
