//! Layers panel disclosure state: expanding and collapsing layer groups (#126).
//!
//! A group's open/closed state is document data ([`photocraft_doc::Group::expanded`], the PSD
//! section divider's open/closed folder type, saved in `.pcraft` too) but, as in Photoshop,
//! toggling it is a view change: no history step, and a clean document stays clean.

use std::sync::Arc;

use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "layer.setExpanded";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn has_group(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.walk().iter().any(|(_, _, l)| l.is_group()) { Ok(()) } else { Err("the document has no layer groups".into()) }
}

fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(format!("`{key}` must be true or false"))),
    }
}

/// Deepest nesting the toggles walk (deeper groups are left as they are).
const MAX_DEPTH: usize = 256;

fn set_all(layers: &mut [Layer], expanded: bool) -> usize {
    set_below(layers, expanded, MAX_DEPTH)
}

/// Set every group in `layers` and, `depth` levels down, the groups inside them.
fn set_below(layers: &mut [Layer], expanded: bool, depth: usize) -> usize {
    let mut n = 0;
    for l in layers {
        if let LayerContent::Group(g) = &mut l.content {
            g.expanded = expanded;
            n += 1;
            if depth > 0 {
                n += set_below(&mut g.children, expanded, depth - 1);
            }
        }
    }
    n
}

/// The list holding layer `id` (the document's top level or a group's children).
fn siblings_of(layers: &mut Vec<Layer>, id: LayerId, depth: usize) -> Option<&mut Vec<Layer>> {
    if layers.iter().any(|l| l.id == id) {
        return Some(layers);
    }
    if depth == 0 {
        return None;
    }
    layers.iter_mut().find_map(|l| match &mut l.content {
        LayerContent::Group(g) => siblings_of(&mut g.children, id, depth - 1),
        _ => None,
    })
}

/// Which groups a click on one group's disclosure triangle sets, as in Photoshop's Layers panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Just that group.
    Group,
    /// Every group beside it, in the same folder (⌘-click); not the groups inside them.
    Siblings,
    /// That group and every group inside it (⌥-click).
    Tree,
    /// Every group beside it and every group inside those (⌘⌥-click).
    SiblingsTree,
}

fn scope_param(p: &Value) -> Result<Scope> {
    Ok(match p.get("scope") {
        None | Some(Value::Null) => Scope::Group,
        Some(Value::String(s)) => match s.as_str() {
            "group" => Scope::Group,
            "siblings" => Scope::Siblings,
            "tree" => Scope::Tree,
            "siblingsTree" => Scope::SiblingsTree,
            other => return Err(bad(format!("unknown scope `{other}` (group, siblings, tree, siblingsTree)"))),
        },
        Some(_) => return Err(bad("`scope` must be group, siblings, tree or siblingsTree")),
    })
}

/// `{"layer":id?,"expanded":bool?,"all":bool?,"scope":..?}`: open or close a group (no
/// `expanded`: toggle). `scope` widens it within the group's folder, as Photoshop's modifier
/// clicks do: `siblings` (every group beside it, ⌘-click), `tree` (it and every group inside it,
/// ⌥-click) or `siblingsTree` (both, ⌘⌥-click). With `all`, every group in the document takes the
/// new state; without a layer, `all` toggles from "any group open".
fn set_expanded(s: &mut Session, p: &Value) -> Result<Value> {
    let want = opt_bool(p, "expanded")?;
    let all = opt_bool(p, "all")?.unwrap_or(false);
    let scope = scope_param(p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let layer = match p.get("layer") {
        None | Some(Value::Null) if all => None,
        None | Some(Value::Null) => Some(st.active_layer.ok_or_else(|| bad("no layer given and no active layer"))?),
        Some(v) => Some(LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be a layer id"))?)),
    };
    let current = match layer {
        Some(id) => match &st.doc.layer(id).ok_or(EngineError::NoLayer(id))?.content {
            LayerContent::Group(g) => g.expanded,
            _ => return Err(bad(format!("layer {} is not a group", id.0))),
        },
        None => st.doc.walk().iter().any(|(_, _, l)| matches!(&l.content, LayerContent::Group(g) if g.expanded)),
    };
    let expanded = want.unwrap_or(!current);
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    // Copy-on-write: history snapshots keep their own state (layers share tiles, so this is cheap).
    let doc = Arc::make_mut(&mut st.doc);
    let changed = if all {
        set_all(&mut doc.layers, expanded)
    } else {
        let id = layer.ok_or_else(|| bad("no layer"))?;
        match scope {
            Scope::Group | Scope::Tree => match doc.layer_mut(id).map(|l| &mut l.content) {
                Some(LayerContent::Group(g)) => {
                    g.expanded = expanded;
                    1 + if scope == Scope::Tree { set_below(&mut g.children, expanded, MAX_DEPTH) } else { 0 }
                }
                _ => return Err(EngineError::NoLayer(id)),
            },
            Scope::Siblings | Scope::SiblingsTree => {
                let list = siblings_of(&mut doc.layers, id, MAX_DEPTH).ok_or(EngineError::NoLayer(id))?;
                if scope == Scope::Siblings {
                    set_below(list, expanded, 0)
                } else {
                    set_below(list, expanded, MAX_DEPTH)
                }
            }
        }
    };
    Ok(json!({"expanded": expanded, "groups": changed}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Expand/Collapse Group",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"expanded":bool?,"all":bool?,"scope":"group"|"siblings"|"tree"|"siblingsTree"?} (no expanded: toggle; scope widens it within the group's folder: siblings = every group beside it, tree = it and every group inside it, siblingsTree = both; all: every group; not an undo step)"##,
        enabled: has_group,
        run: set_expanded,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (Session, u64, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({"name": "deep"})).unwrap();
        let inner = s.execute("layer.groupLayers", json!({"name": "inner"})).unwrap()["layer"].as_u64().unwrap();
        let outer = s.execute("layer.groupLayers", json!({"layer": inner, "name": "outer"})).unwrap()["layer"].as_u64().unwrap();
        (s, inner, outer)
    }

    fn expanded(s: &Session, id: u64) -> bool {
        matches!(&s.active().unwrap().doc.layer(LayerId(id)).unwrap().content, LayerContent::Group(g) if g.expanded)
    }

    #[test]
    fn toggles_one_group_without_history_or_dirtying() {
        let (mut s, inner, outer) = session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let undo_steps = s.active().unwrap().history.entries().len();
        assert!(expanded(&s, inner) && expanded(&s, outer));
        assert_eq!(s.execute(ID, json!({"layer": inner})).unwrap()["expanded"], false);
        assert!(!expanded(&s, inner) && expanded(&s, outer));
        assert_eq!(s.active().unwrap().history.entries().len(), undo_steps, "no history step");
        assert_eq!(s.active().unwrap().saved_revision, s.active().unwrap().revision, "still clean");
        s.execute(ID, json!({"layer": inner, "expanded": true})).unwrap();
        assert!(expanded(&s, inner));
        // The inspect JSON reports it.
        s.execute(ID, json!({"layer": outer, "expanded": false})).unwrap();
        assert_eq!(s.execute("document.inspect", json!({})).unwrap()["layers"][0]["expanded"], false);
    }

    #[test]
    fn all_sets_every_group() {
        let (mut s, inner, outer) = session();
        s.execute(ID, json!({"layer": outer, "all": true})).unwrap();
        assert!(!expanded(&s, inner) && !expanded(&s, outer));
        assert_eq!(s.execute(ID, json!({"all": true})).unwrap()["groups"], 2);
        assert!(expanded(&s, inner) && expanded(&s, outer));
    }

    /// Photoshop's modifier clicks on a disclosure triangle: ⌘ sets the groups beside it, ⌥ it and
    /// the groups inside it, ⌘⌥ both, never reaching outside its folder.
    #[test]
    fn scopes_follow_photoshops_modifier_clicks() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        let group = |s: &mut Session, p: Value| s.execute("layer.groupLayers", p).unwrap()["layer"].as_u64().unwrap();
        // Outer holds A (holding a1, holding a2) and B (holding b1); D sits beside Outer.
        s.execute("layer.new.layer", json!({"name": "pa"})).unwrap();
        let a2 = group(&mut s, json!({"name": "a2"}));
        let a1 = group(&mut s, json!({"layer": a2, "name": "a1"}));
        let a = group(&mut s, json!({"layer": a1, "name": "A"}));
        s.execute("layer.new.layer", json!({"name": "pb"})).unwrap();
        let b1 = group(&mut s, json!({"name": "b1"}));
        let b = group(&mut s, json!({"layer": b1, "name": "B"}));
        s.execute("layer.select", json!({"layer": a})).unwrap();
        s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
        let outer = group(&mut s, json!({"name": "Outer"}));
        s.execute("layer.new.layer", json!({"name": "pd"})).unwrap();
        let d = group(&mut s, json!({"name": "D"}));
        let all = [a2, a1, a, b1, b, outer, d];
        let open = |s: &Session| all.iter().map(|id| expanded(s, *id)).collect::<Vec<_>>();
        assert!(open(&s).iter().all(|o| *o), "{:?}", open(&s));
        // ⌘: A and B (beside each other in Outer) close; a1, b1, Outer and D stay open.
        s.execute(ID, json!({"layer": a, "expanded": false, "scope": "siblings"})).unwrap();
        assert_eq!(open(&s), [true, true, false, true, false, true, true]);
        // ⌥: A and everything inside it.
        s.execute(ID, json!({"layer": a, "expanded": false, "scope": "tree"})).unwrap();
        assert_eq!(open(&s), [false, false, false, true, false, true, true]);
        // ⌘⌥ on B opens A, B and everything inside them, and not D beside Outer.
        s.execute(ID, json!({"layer": d, "expanded": false})).unwrap();
        s.execute(ID, json!({"layer": b, "expanded": true, "scope": "siblingsTree"})).unwrap();
        assert_eq!(open(&s), [true, true, true, true, true, true, false]);
        assert!(s.execute(ID, json!({"layer": a, "scope": "everything"})).is_err());
        assert!(s.execute(ID, json!({"layer": a, "scope": 2})).is_err());
    }

    #[test]
    fn bad_params_fail_gracefully() {
        let (mut s, _, _) = session();
        let bg = s.active().unwrap().doc.layers[0].id.0;
        for p in [
            json!({"layer": bg}),
            json!({"layer": u64::MAX}),
            json!({"layer": "x"}),
            json!({"layer": -1}),
            json!({"expanded": "yes", "all": true}),
            json!({"all": 3}),
        ] {
            assert!(s.execute(ID, p.clone()).is_err(), "{p}");
        }
        let mut empty = Session::new();
        assert!(empty.execute(ID, json!({})).is_err());
        empty.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(empty.execute(ID, json!({"all": true})).is_err(), "no groups: disabled");
    }
}
