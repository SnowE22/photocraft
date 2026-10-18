//! Moving and deleting several brush presets and folders at once: the Brushes panel's selection
//! (Shift- and Ctrl-click, as in Photoshop) dragged or deleted as one gesture.
//!
//! Items are addressed by their place in the panel's tree. The top level holds loose presets
//! ([`BrushPreset::group`] empty) and groups; a group holds presets and nested folders. A
//! folder's *path* is its full path from the top: `["Kit"]` is the group `Kit`, `["Kit", "Inks"]`
//! its folder `Inks`. A preset's place is its group followed by its folders (`[]` when loose).
//!
//! A move keeps the moved items in panel order and lands them as one block: before or after a
//! sibling, or at the end of the destination. Moving also puts the library in panel order (every
//! folder's presets together), which the panel shows the same way.

use photocraft_paint::BrushPreset;
use serde_json::{Value, json};

use crate::brush_preset_cmds::{MAX_FOLDER_DEPTH, MAX_NAME};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Most items one command takes.
pub const MAX_ITEMS: usize = 20_000;
/// Longest place: a group and its nested folders.
pub const MAX_PATH: usize = MAX_FOLDER_DEPTH + 1;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Part `i` of a preset's place (0 = its group), `None` past its end or for a loose preset.
pub fn place_part(p: &BrushPreset, i: usize) -> Option<&str> {
    if p.group.is_empty() {
        None
    } else if i == 0 {
        Some(p.group.as_str())
    } else {
        p.folder.get(i - 1).map(String::as_str)
    }
}

/// How deep a preset sits: 0 when loose, 1 directly in a group, more in nested folders.
pub fn place_len(p: &BrushPreset) -> usize {
    if p.group.is_empty() { 0 } else { 1 + p.folder.len() }
}

/// The preset's place: its group then its folders (empty when loose).
pub fn place(p: &BrushPreset) -> Vec<String> {
    (0..place_len(p)).filter_map(|i| place_part(p, i).map(str::to_string)).collect()
}

/// Is the preset in the folder at `path` (or below it)? Everything is under `[]`.
pub fn under(p: &BrushPreset, path: &[String]) -> bool {
    match path.split_first() {
        None => true,
        Some((g, rest)) => !p.group.is_empty() && p.group == *g && p.folder.starts_with(rest),
    }
}

fn set_place(p: &mut BrushPreset, path: &[String]) {
    match path.split_first() {
        None => {
            p.group.clear();
            p.folder.clear();
        }
        Some((g, rest)) => {
            p.group.clone_from(g);
            p.folder = rest.to_vec();
        }
    }
}

/// Library indices in panel order: the tree walked top to bottom, each level's presets and
/// folders in order of their first preset.
pub fn panel_order(presets: &[BrushPreset]) -> Vec<usize> {
    let mut out = Vec::with_capacity(presets.len());
    walk(presets, (0..presets.len()).collect(), 0, &mut out);
    out
}

/// One level of [`panel_order`]; the recursion is at most [`MAX_PATH`] deep (deeper parts count
/// as presets of the last level).
fn walk(presets: &[BrushPreset], idx: Vec<usize>, depth: usize, out: &mut Vec<usize>) {
    enum Slot {
        Preset(usize),
        Folder(usize),
    }
    let mut slots = Vec::new();
    let mut folders: Vec<(&str, Vec<usize>)> = Vec::new();
    for i in idx {
        let part = if depth < MAX_PATH { presets.get(i).and_then(|p| place_part(p, depth)) } else { None };
        match part {
            None => slots.push(Slot::Preset(i)),
            Some(n) => match folders.iter().position(|(f, _)| *f == n) {
                Some(k) => {
                    if let Some((_, v)) = folders.get_mut(k) {
                        v.push(i);
                    }
                }
                None => {
                    slots.push(Slot::Folder(folders.len()));
                    folders.push((n, vec![i]));
                }
            },
        }
    }
    let mut folders: Vec<Vec<usize>> = folders.into_iter().map(|(_, v)| v).collect();
    for s in slots {
        match s {
            Slot::Preset(i) => out.push(i),
            Slot::Folder(k) => {
                if let Some(v) = folders.get_mut(k) {
                    walk(presets, std::mem::take(v), depth + 1, out);
                }
            }
        }
    }
}

/// A path param (`["Kit", "Inks"]`); absent or null is `[]`, the top level.
fn path_param(v: Option<&Value>, cmd: &str, key: &str) -> Result<Vec<String>> {
    let err = || bad(cmd, format!("`{key}` must be a list of at most {MAX_PATH} folder names (non-empty strings of at most {MAX_NAME} characters)"));
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= MAX_PATH => a
            .iter()
            .map(|x| x.as_str().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME).map(str::to_string).ok_or_else(err))
            .collect(),
        Some(_) => Err(err()),
    }
}

fn name_of<'a>(v: &'a Value, cmd: &str, key: &str) -> Result<&'a str> {
    v.as_str()
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.chars().count() <= MAX_NAME)
        .ok_or_else(|| bad(cmd, format!("`{key}` must be a name (a non-empty string of at most {MAX_NAME} characters)")))
}

fn index_of(presets: &[BrushPreset], name: &str) -> Option<usize> {
    presets.iter().position(|x| x.name.eq_ignore_ascii_case(name))
}

/// A preset (by library index) or a folder (by path).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Item {
    Preset(usize),
    Folder(Vec<String>),
}

/// The `items` param: `[{"preset": name} | {"folder": [path]}]`, each existing; repeats dropped.
fn items_param(lib: &[BrushPreset], p: &Value, cmd: &str) -> Result<Vec<Item>> {
    let list = p.get("items").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing `items` (a list of {\"preset\": name} or {\"folder\": [path]})"))?;
    if list.is_empty() {
        return Err(bad(cmd, "`items` is empty"));
    }
    if list.len() > MAX_ITEMS {
        return Err(bad(cmd, format!("`items` lists more than {MAX_ITEMS} items")));
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for v in list {
        let item = if let Some(n) = v.get("preset") {
            let n = name_of(n, cmd, "preset")?;
            Item::Preset(index_of(lib, n).ok_or_else(|| bad(cmd, format!("no brush preset named `{n}`")))?)
        } else if v.get("folder").is_some() {
            let f = path_param(v.get("folder"), cmd, "folder")?;
            if f.is_empty() {
                return Err(bad(cmd, "`folder` can't be empty (the top level is not a folder)"));
            }
            if !lib.iter().any(|x| under(x, &f)) {
                return Err(bad(cmd, format!("no brush preset folder `{}`", f.join("/"))));
            }
            Item::Folder(f)
        } else {
            return Err(bad(cmd, "each item is {\"preset\": name} or {\"folder\": [path]}"));
        };
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    Ok(out)
}

/// The library span (first and last index, in a library in panel order) of `child` (`{"preset":
/// name}` or `{"folder": name}`), which must sit directly in `dest`.
fn child_span(lib: &[BrushPreset], dest: &[String], child: &Value, cmd: &str, key: &str) -> Result<(usize, usize)> {
    if let Some(n) = child.get("preset") {
        let n = name_of(n, cmd, key)?;
        let k = index_of(lib, n).ok_or_else(|| bad(cmd, format!("`{key}`: no brush preset named `{n}`")))?;
        let here = lib.get(k).is_some_and(|x| place_len(x) == dest.len() && under(x, dest));
        if !here {
            return Err(bad(cmd, format!("`{key}`: `{n}` is not directly in that folder")));
        }
        return Ok((k, k));
    }
    if let Some(n) = child.get("folder") {
        let n = name_of(n, cmd, key)?;
        let mut path = dest.to_vec();
        path.push(n.to_string());
        let first = lib.iter().position(|x| under(x, &path));
        let last = lib.iter().rposition(|x| under(x, &path));
        return match (first, last) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => Err(bad(cmd, format!("`{key}`: no folder named `{n}` there"))),
        };
    }
    Err(bad(cmd, format!("`{key}` is {{\"preset\": name}} or {{\"folder\": name}}")))
}

fn move_items(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.moveItems";
    let lib = &s.tools.presets;
    let items = items_param(lib, p, cmd)?;
    let dest = path_param(p.get("into"), cmd, "into")?;
    let create = match p.get("create") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`create` must be true or false")),
    };
    let exists = |path: &[String]| path.is_empty() || lib.iter().any(|x| under(x, path));
    if create {
        let Some((leaf, parent)) = dest.split_last() else { return Err(bad(cmd, "`create` needs an `into` folder path")) };
        if exists(&dest) {
            return Err(bad(cmd, format!("a folder named `{leaf}` already exists there")));
        }
        if !exists(parent) {
            return Err(bad(cmd, format!("no brush preset folder `{}`", parent.join("/"))));
        }
    } else if !exists(&dest) {
        return Err(bad(cmd, format!("no brush preset folder `{}`", dest.join("/"))));
    }
    for it in &items {
        if let Item::Folder(f) = it
            && dest.starts_with(f)
        {
            return Err(bad(cmd, "`into` is a folder being moved, or inside one"));
        }
    }
    let (before, after) = (p.get("before").filter(|v| !v.is_null()), p.get("after").filter(|v| !v.is_null()));
    if before.is_some() && after.is_some() {
        return Err(bad(cmd, "give `before` or `after`, not both"));
    }
    // Work on the library in panel order: every folder's presets sit together.
    let order = panel_order(lib);
    let mut new_index = vec![0usize; lib.len()];
    for (k, &i) in order.iter().enumerate() {
        if let Some(slot) = new_index.get_mut(i) {
            *slot = k;
        }
    }
    let mut work: Vec<BrushPreset> = order.iter().filter_map(|&i| lib.get(i).cloned()).collect();
    // Each moved preset's new place. Outer folders first: a folder's content that is also listed
    // moves with it and keeps its place inside it.
    let mut items: Vec<Item> = items
        .into_iter()
        .map(|it| match it {
            Item::Preset(i) => Item::Preset(new_index.get(i).copied().unwrap_or(i)),
            f => f,
        })
        .collect();
    items.sort_by_key(|it| match it {
        Item::Folder(f) => f.len(),
        Item::Preset(_) => usize::MAX,
    });
    let mut target: Vec<Option<Vec<String>>> = vec![None; work.len()];
    let mut leaves: Vec<String> = Vec::new();
    for it in &items {
        match it {
            Item::Folder(f) => {
                if items.iter().any(|o| matches!(o, Item::Folder(g) if g.len() < f.len() && f.starts_with(g))) {
                    continue;
                }
                let Some(leaf) = f.last() else { continue };
                if leaves.contains(leaf) {
                    return Err(bad(cmd, format!("two of the folders are named `{leaf}`")));
                }
                leaves.push(leaf.clone());
                for (k, x) in work.iter().enumerate() {
                    if under(x, f) {
                        let mut np = dest.clone();
                        np.push(leaf.clone());
                        np.extend(place(x).into_iter().skip(f.len()));
                        if np.len() > MAX_PATH {
                            return Err(bad(cmd, format!("folders can be nested at most {MAX_FOLDER_DEPTH} deep inside a group")));
                        }
                        if let Some(t) = target.get_mut(k) {
                            *t = Some(np);
                        }
                    }
                }
            }
            Item::Preset(k) => {
                if let Some(t) = target.get_mut(*k)
                    && t.is_none()
                {
                    *t = Some(dest.clone());
                }
            }
        }
    }
    // A folder landing in `dest` must not meet a folder of the same name that stays there.
    for leaf in &leaves {
        let mut there = dest.clone();
        there.push(leaf.clone());
        if work.iter().zip(&target).any(|(x, t)| t.is_none() && under(x, &there)) {
            return Err(bad(cmd, format!("a folder named `{leaf}` already exists there")));
        }
    }
    let first_moved = target.iter().position(Option::is_some).unwrap_or(work.len());
    // Where the block goes, as an index into `work`.
    let cut = match (before, after) {
        (Some(c), _) => child_span(&work, &dest, c, cmd, "before")?.0,
        (None, Some(c)) => child_span(&work, &dest, c, cmd, "after")?.1 + 1,
        (None, None) if create => first_moved,
        (None, None) => work.iter().rposition(|x| under(x, &dest)).map_or(work.len(), |k| k + 1),
    };
    let at = target.iter().take(cut).filter(|t| t.is_none()).count();
    let mut block = Vec::new();
    let mut rest = Vec::with_capacity(work.len());
    for (mut x, t) in work.drain(..).zip(target) {
        match t {
            Some(np) => {
                if place(&x) != np {
                    set_place(&mut x, &np);
                    // A built-in moved elsewhere is the user's now (built-ins regenerate in place).
                    x.builtin = false;
                }
                block.push(x);
            }
            None => rest.push(x),
        }
    }
    let moved = block.len();
    let tail = rest.split_off(at.min(rest.len()));
    rest.extend(block);
    rest.extend(tail);
    s.tools.presets = rest;
    s.brush_presets_changed();
    Ok(json!({ "moved": moved, "into": dest }))
}

fn delete_items(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.deleteItems";
    let lib = &s.tools.presets;
    let items = items_param(lib, p, cmd)?;
    let mut gone = vec![false; lib.len()];
    for it in &items {
        match it {
            Item::Preset(i) => {
                if let Some(g) = gone.get_mut(*i) {
                    *g = true;
                }
            }
            Item::Folder(f) => {
                for (g, x) in gone.iter_mut().zip(lib) {
                    *g |= under(x, f);
                }
            }
        }
    }
    let before = lib.len();
    let mut flags = gone.into_iter();
    s.tools.presets.retain(|_| !flags.next().unwrap_or(false));
    if s.tools.current_preset.as_ref().is_some_and(|c| index_of(&s.tools.presets, c).is_none()) {
        s.tools.current_preset = None;
        s.tools.brush_changed();
    }
    s.brush_presets_changed();
    Ok(json!({ "deleted": before - s.tools.presets.len(), "count": s.tools.presets.len() }))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: always, run: $run, journal: true }
    };
}

/// Multi-item brush preset organisation specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "brush.presets.moveItems",
            "Move Brushes",
            r##"{"items":[{"preset":name} | {"folder":[path]}] (a folder's path from the top: ["Group"] or ["Group","Folder"]; a listed folder's own content that is also listed moves with it),"into":[path]?=[] (the destination folder; [] = the top level, where presets are loose and folders become groups),"create":bool?=false (`into` is a new folder, made where the first moved item was; its parent must exist),"before":{"preset":name}|{"folder":name}? (a sibling directly in `into` to land before) | "after":{"preset":name}|{"folder":name}? (or after)=end of `into`} → {moved, into}"##,
            move_items
        ),
        spec!(
            "brush.presets.deleteItems",
            "Delete Brushes",
            r##"{"items":[{"preset":name} | {"folder":[path]}] (a folder goes with every preset in it)} → {deleted, count}"##,
            delete_items
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(name: &str, place: &[&str]) -> BrushPreset {
        let mut p = BrushPreset { name: name.into(), brush: Default::default(), builtin: false, group: String::new(), folder: Vec::new(), captures: Default::default(), tool: None };
        set_place(&mut p, &place.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        p
    }

    /// A session with only these presets: loose `L1`, group `A` (with folder `Ink`), loose `L2`,
    /// group `B`.
    fn lib() -> Session {
        let mut s = Session::new();
        s.tools.presets = vec![
            preset("L1", &[]),
            preset("A1", &["A"]),
            preset("I1", &["A", "Ink"]),
            preset("I2", &["A", "Ink"]),
            preset("A2", &["A"]),
            preset("L2", &[]),
            preset("B1", &["B"]),
            preset("B2", &["B"]),
        ];
        s
    }

    /// The panel as text: loose presets and folders by name, in panel order.
    fn panel(s: &Session) -> Vec<String> {
        let mut out = Vec::new();
        let mut open: Vec<String> = Vec::new();
        for i in panel_order(&s.tools.presets) {
            let x = &s.tools.presets[i];
            let pl = place(x);
            while !pl.starts_with(&open) {
                open.pop();
            }
            while open.len() < pl.len() {
                open.push(pl[open.len()].clone());
                out.push(format!("[{}]", open.join("/")));
            }
            out.push(x.name.clone());
        }
        out
    }

    fn run(s: &mut Session, p: Value) -> Value {
        s.execute("brush.presets.moveItems", p).unwrap()
    }

    #[test]
    fn a_preset_moves_out_of_its_group_between_two_groups() {
        let mut s = lib();
        // Out of group B, between group A and the loose L2: before L2 at the top level.
        run(&mut s, json!({"items": [{"preset": "B2"}], "before": {"preset": "L2"}}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "[A/Ink]", "I1", "I2", "A2", "B2", "L2", "[B]", "B1"]);
        let b2 = &s.tools.presets[index_of(&s.tools.presets, "B2").unwrap()];
        assert!(b2.group.is_empty() && b2.folder.is_empty());
        // Right before group B, and after group A.
        run(&mut s, json!({"items": [{"preset": "B2"}], "before": {"folder": "B"}}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "[A/Ink]", "I1", "I2", "A2", "L2", "B2", "[B]", "B1"]);
        run(&mut s, json!({"items": [{"preset": "B2"}], "after": {"folder": "A"}}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "[A/Ink]", "I1", "I2", "A2", "B2", "L2", "[B]", "B1"]);
        // To the end of the top level, and back into a group's folder.
        run(&mut s, json!({"items": [{"preset": "B2"}]}));
        assert_eq!(panel(&s).last().map(String::as_str), Some("B2"));
        run(&mut s, json!({"items": [{"preset": "B2"}], "into": ["A", "Ink"], "after": {"preset": "I1"}}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "[A/Ink]", "I1", "B2", "I2", "A2", "L2", "[B]", "B1"]);
        assert_eq!(s.tools.presets[index_of(&s.tools.presets, "B2").unwrap()].folder, ["Ink"]);
    }

    #[test]
    fn several_items_move_as_one_block_in_panel_order() {
        let mut s = lib();
        // Listed out of order: they land in panel order. The folder keeps its content.
        let r = run(&mut s, json!({"items": [{"preset": "B1"}, {"folder": ["A", "Ink"]}, {"preset": "L1"}], "into": ["B"], "before": {"preset": "B2"}}));
        assert_eq!(r, json!({"moved": 4, "into": ["B"]}));
        assert_eq!(panel(&s), ["[A]", "A1", "A2", "L2", "[B]", "L1", "[B/Ink]", "I1", "I2", "B1", "B2"]);
        // A group moved into another group becomes a folder; a folder moved to the top, a group.
        run(&mut s, json!({"items": [{"folder": ["A"]}], "into": ["B"]}));
        assert_eq!(panel(&s), ["L2", "[B]", "L1", "[B/Ink]", "I1", "I2", "B1", "B2", "[B/A]", "A1", "A2"]);
        run(&mut s, json!({"items": [{"folder": ["B", "A"]}], "before": {"preset": "L2"}}));
        assert_eq!(panel(&s), ["[A]", "A1", "A2", "L2", "[B]", "L1", "[B/Ink]", "I1", "I2", "B1", "B2"]);
        // A folder and a preset inside it: the preset stays inside.
        run(&mut s, json!({"items": [{"preset": "I2"}, {"folder": ["B", "Ink"]}]}));
        assert_eq!(panel(&s), ["[A]", "A1", "A2", "L2", "[B]", "L1", "B1", "B2", "[Ink]", "I1", "I2"]);
    }

    #[test]
    fn a_selection_dropped_next_to_one_of_its_own_items_stays_put() {
        let mut s = lib();
        let before = panel(&s);
        run(&mut s, json!({"items": [{"preset": "A1"}, {"preset": "A2"}], "into": ["A"], "before": {"preset": "A1"}}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "A2", "[A/Ink]", "I1", "I2", "L2", "[B]", "B1", "B2"]);
        let mut s = lib();
        run(&mut s, json!({"items": [{"folder": ["A"]}], "after": {"folder": "A"}}));
        assert_eq!(panel(&s), before);
    }

    #[test]
    fn create_wraps_the_items_in_a_new_folder_where_the_first_one_was() {
        let mut s = lib();
        run(&mut s, json!({"items": [{"preset": "L2"}, {"folder": ["B"]}], "into": ["Group 1"], "create": true}));
        assert_eq!(panel(&s), ["L1", "[A]", "A1", "[A/Ink]", "I1", "I2", "A2", "[Group 1]", "L2", "[Group 1/B]", "B1", "B2"]);
        run(&mut s, json!({"items": [{"preset": "A1"}], "into": ["A", "Dry"], "create": true}));
        assert_eq!(panel(&s)[..6], ["L1", "[A]", "[A/Dry]", "A1", "[A/Ink]", "I1"]);
    }

    #[test]
    fn delete_items_removes_presets_and_whole_folders() {
        let mut s = lib();
        s.tools.current_preset = Some("I1".into());
        let r = s.execute("brush.presets.deleteItems", json!({"items": [{"preset": "L1"}, {"folder": ["A", "Ink"]}, {"preset": "I2"}]})).unwrap();
        assert_eq!(r, json!({"deleted": 3, "count": 5}));
        assert_eq!(panel(&s), ["[A]", "A1", "A2", "L2", "[B]", "B1", "B2"]);
        assert_eq!(s.tools.current_preset, None);
    }

    #[test]
    fn bad_moves_and_deletes_are_errors_and_change_nothing() {
        let mut s = lib();
        let before = s.tools.presets.clone();
        for p in [
            json!({}),
            json!({"items": []}),
            json!({"items": "L1"}),
            json!({"items": [{"preset": "Nope"}]}),
            json!({"items": [{"preset": ""}]}),
            json!({"items": [{"preset": 3}]}),
            json!({"items": [{"folder": []}]}),
            json!({"items": [{"folder": ["Nope"]}]}),
            json!({"items": [{"folder": "A"}]}),
            json!({"items": [{}]}),
            json!({"items": [{"preset": "L1"}], "into": ["Nope"]}),
            json!({"items": [{"preset": "L1"}], "into": "A"}),
            json!({"items": [{"preset": "L1"}], "into": [""]}),
            json!({"items": [{"folder": ["A"]}], "into": ["A", "Ink"]}),
            json!({"items": [{"folder": ["A"]}], "into": ["A"]}),
            json!({"items": [{"folder": ["A", "Ink"]}], "into": ["B"], "before": {"preset": "A1"}}),
            json!({"items": [{"preset": "L1"}], "before": {"preset": "L2"}, "after": {"preset": "L2"}}),
            json!({"items": [{"preset": "L1"}], "before": {"folder": "Nope"}}),
            json!({"items": [{"preset": "L1"}], "before": {"preset": "A1"}}),
            json!({"items": [{"preset": "L1"}], "before": "L2"}),
            json!({"items": [{"preset": "L1"}], "create": "yes"}),
            json!({"items": [{"preset": "L1"}], "create": true}),
            json!({"items": [{"preset": "L1"}], "into": ["A"], "create": true}),
            json!({"items": [{"preset": "L1"}], "into": ["Nope", "New"], "create": true}),
            // An unknown `before`.
            json!({"items": [{"folder": ["A", "Ink"]}, {"folder": ["B"]}], "into": ["C"], "create": true, "before": {"preset": "x"}}),
        ] {
            assert!(s.execute("brush.presets.moveItems", p.clone()).is_err(), "{p}");
            assert_eq!(s.tools.presets, before, "{p}");
        }
        s.tools.presets.push(preset("X1", &["Ink"]));
        let before = s.tools.presets.clone();
        assert!(s.execute("brush.presets.moveItems", json!({"items": [{"folder": ["A", "Ink"]}]})).is_err(), "a group named Ink is already at the top");
        assert!(s.execute("brush.presets.moveItems", json!({"items": [{"folder": ["A", "Ink"]}, {"folder": ["Ink"]}], "into": ["B"]})).is_err(), "two folders named Ink");
        assert_eq!(s.tools.presets, before);
        let mut full = vec!["Deep".to_string()];
        full.extend((0..MAX_FOLDER_DEPTH).map(|i| format!("d{i}")));
        let mut d = preset("D1", &[]);
        set_place(&mut d, &full);
        s.tools.presets.push(d);
        let before = s.tools.presets.clone();
        assert!(s.execute("brush.presets.moveItems", json!({"items": [{"folder": ["Deep"]}], "into": ["B"]})).is_err(), "too deep");
        assert_eq!(s.tools.presets, before);
        for p in [json!({}), json!({"items": []}), json!({"items": [{"preset": "Nope"}]}), json!({"items": [{"folder": ["Nope"]}]}), json!({"items": [7]})] {
            assert!(s.execute("brush.presets.deleteItems", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.tools.presets, before);
    }

    #[test]
    fn panel_order_groups_each_folder_together() {
        let mut s = Session::new();
        s.tools.presets = vec![preset("A1", &["A"]), preset("L1", &[]), preset("B1", &["B"]), preset("A2", &["A"]), preset("I1", &["A", "Ink"])];
        let names: Vec<&str> = panel_order(&s.tools.presets).into_iter().map(|i| s.tools.presets[i].name.as_str()).collect();
        assert_eq!(names, ["A1", "A2", "I1", "L1", "B1"]);
        // A move puts the library itself in that order.
        run(&mut s, json!({"items": [{"preset": "L1"}]}));
        let names: Vec<&str> = s.tools.presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["A1", "A2", "I1", "B1", "L1"]);
    }
}
