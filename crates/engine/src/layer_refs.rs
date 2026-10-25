//! Layers named in recorded action steps. The Layers panel, the canvas and agents name a layer by
//! its id, which means nothing in another document (and the same id is another layer there), so a
//! recorded step that keeps it changes some other layer, or fails, when the action plays on
//! another document. Recording names it as Photoshop does instead: `{"current": true}` for the
//! layer that was current, `{"name": "Ink"}` (with `"occurrence"` among layers of that name, from
//! the bottom, when several share it) for another; dispatch turns those back into the ids of the
//! document the step runs on, so every command still reads a plain id.

use photocraft_doc::{Document, LayerId};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session};

/// The params of `id` that name a layer by id.
fn keys(id: &str) -> &'static [&'static str] {
    match id {
        "layer.moveTo" => &["layer", "layers", "target"],
        "layer.layerStyle.transferEffects" | "layer.layerMask.transfer" | "layer.smartFilter.transfer" => &["from", "to"],
        "edit.autoAlignLayers" => &["layers", "reference"],
        "type.attachToPath" => &["layer", "layers", "shape"],
        "shape.presets.place" => &["layer", "layers", "addTo"],
        // Recorded by name and occurrence already (`layer_multi_cmds::recorded_selection`).
        "layer.select" => &[],
        _ if id.starts_with("actions.") => &[],
        _ => &["layer", "layers"],
    }
}

/// Steps that act on the current layer without a `layer`: theirs is left out for the current
/// layer, Photoshop's "Set Current Layer" (an eye ⌥-click, a rename).
fn current_by_default(id: &str) -> bool {
    matches!(id, "layer.setProps" | "layer.showOnly")
}

/// Steps that always act on the current layer when an action plays (user report: an action's
/// Show Only must show the layer you're on, not the one ⌥-clicked while recording): recorded
/// without a layer, and a layer an older recording named is ignored.
fn current_only(id: &str) -> bool {
    id == "layer.showOnly"
}

fn reference(doc: &Document, active: Option<LayerId>, id: LayerId) -> Option<Value> {
    if Some(id) == active {
        return Some(json!({"current": true}));
    }
    let name = &doc.layer(id)?.name;
    let same: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| l.name == *name).map(|(_, _, l)| l.id).collect();
    let occurrence = same.iter().position(|l| *l == id)?;
    Some(if same.len() > 1 { json!({"name": name, "occurrence": occurrence}) } else { json!({"name": name}) })
}

/// A step's params as recorded, read before it runs (a rename changes the name): its layer ids
/// as references. `None`: recorded as given.
pub(crate) fn record(s: &Session, id: &str, p: &Value) -> Option<Value> {
    let st = s.active()?;
    let keys = keys(id);
    let mut out = p.clone();
    let fields = out.as_object_mut()?;
    let mut changed = false;
    for key in keys {
        let Some(v) = fields.get(*key) else { continue };
        let new = match v {
            Value::Number(n) => n.as_u64().and_then(|l| reference(&st.doc, st.active_layer, LayerId(l))),
            // Every element a layer id, or the list stays as it is.
            Value::Array(a) => {
                a.iter().map(|e| e.as_u64().and_then(|l| reference(&st.doc, st.active_layer, LayerId(l)))).collect::<Option<Vec<_>>>().map(Value::Array)
            }
            _ => None,
        };
        if let Some(new) = new {
            fields.insert((*key).to_string(), new);
            changed = true;
        }
    }
    if current_only(id) {
        let named = fields.remove("layer").is_some() | fields.remove("layers").is_some();
        return (changed || named).then_some(out);
    }
    if current_by_default(id) {
        let current = json!({"current": true});
        if fields.get("layer") == Some(&current) {
            fields.remove("layer");
        }
        if fields.get("layers").and_then(Value::as_array).is_some_and(|a| a.len() == 1 && a.first() == Some(&current)) {
            fields.remove("layers");
        }
    }
    changed.then_some(out)
}

fn resolve_one(s: &Session, cmd: &str, v: &Value) -> Result<Option<LayerId>> {
    let Some(o) = v.as_object() else { return Ok(None) };
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.to_string(), msg };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if o.get("current").and_then(Value::as_bool) == Some(true) {
        return st.active_layer.map(Some).ok_or_else(|| bad("no current layer".into()));
    }
    let Some(name) = o.get("name") else { return Ok(None) };
    let name = name.as_str().ok_or_else(|| bad(format!("a layer's `name` must be a string, got {name}")))?;
    let occurrence = match o.get("occurrence") {
        None => 0,
        Some(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(format!("`occurrence` must be a non-negative integer, got {n}")))?,
    };
    st.doc
        .walk()
        .into_iter()
        .filter(|(_, _, l)| l.name == name)
        .nth(occurrence)
        .map(|(_, _, l)| Some(l.id))
        .ok_or_else(|| bad(if occurrence == 0 { format!("no layer named \"{name}\"") } else { format!("no layer #{} named \"{name}\"", occurrence + 1) }))
}

/// Dispatch: the layer references of a recorded step back to the layer ids of the active
/// document. Params without any are returned as they are.
pub(crate) fn resolve(s: &Session, id: &str, mut p: Value) -> Result<Value> {
    let keys = keys(id);
    let Some(fields) = p.as_object_mut() else { return Ok(p) };
    if current_only(id) {
        // Only a recorded reference: a plain id is a live ⌥-click on that layer's eye.
        for key in ["layer", "layers"] {
            let recorded = |v: &Value| v.is_object() || v.as_array().is_some_and(|a| a.iter().any(Value::is_object));
            if fields.get(key).is_some_and(recorded) {
                fields.remove(key);
            }
        }
        return Ok(p);
    }
    for key in keys {
        let Some(v) = fields.get(*key) else { continue };
        let new = match v {
            Value::Object(_) => resolve_one(s, id, v)?.map(|l| json!(l.0)),
            Value::Array(a) if a.iter().any(Value::is_object) => {
                let mut ids = Vec::with_capacity(a.len());
                for e in a {
                    ids.push(match resolve_one(s, id, e)? {
                        Some(l) => json!(l.0),
                        None => e.clone(),
                    });
                }
                Some(Value::Array(ids))
            }
            _ => None,
        };
        if let Some(new) = new {
            fields.insert((*key).to_string(), new);
        }
    }
    Ok(p)
}

/// A recorded layer reference, as the Actions panel shows it.
#[derive(Debug, PartialEq, Eq)]
pub enum Described {
    Current,
    /// The layer's name, with its occurrence when several share it: "Twin (2)".
    Named(String),
}

/// How a recorded reference reads in the Actions panel. `None` for anything else.
pub fn describe(v: &Value) -> Option<Described> {
    let o = v.as_object()?;
    if o.get("current").and_then(Value::as_bool) == Some(true) {
        return Some(Described::Current);
    }
    let name = o.get("name")?.as_str()?;
    Some(Described::Named(match o.get("occurrence").and_then(Value::as_u64) {
        Some(n) if n > 0 => format!("{name} ({})", n + 1),
        _ => name.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (Session, LayerId, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        let t1 = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        let t2 = s.active().unwrap().active_layer.unwrap();
        (s, a, t1, t2)
    }

    #[test]
    fn ids_are_recorded_as_the_current_layer_or_by_name() {
        let (s, a, t1, t2) = session();
        assert_eq!(record(&s, "layer.hideLayers", &json!({"layer": t2.0})), Some(json!({"layer": {"current": true}})));
        assert_eq!(record(&s, "layer.hideLayers", &json!({"layer": a.0})), Some(json!({"layer": {"name": "A"}})));
        assert_eq!(
            record(&s, "layer.moveTo", &json!({"layers": [a.0, t1.0], "target": t2.0})),
            Some(json!({"layers": [{"name": "A"}, {"name": "Twin", "occurrence": 0}], "target": {"current": true}}))
        );
        // The current layer is left out where that is the step's default; Show Only always
        // plays on the current layer, whichever eye was ⌥-clicked.
        assert_eq!(record(&s, "layer.showOnly", &json!({"layer": t2.0})), Some(json!({})));
        assert_eq!(record(&s, "layer.showOnly", &json!({"layer": a.0})), Some(json!({})));
        assert_eq!(record(&s, "layer.showOnly", &json!({})), None);
        assert_eq!(
            record(&s, "layer.setProps", &json!({"layer": t1.0, "visible": false})),
            Some(json!({"layer": {"name": "Twin", "occurrence": 0}, "visible": false}))
        );
        // Not layer ids: as given.
        assert_eq!(record(&s, "layer.hideLayers", &json!({"layer": u64::MAX})), None);
        assert_eq!(record(&s, "layer.hideLayers", &json!({"layers": [a.0, u64::MAX]})), None);
        assert_eq!(record(&s, "edit.fill", &json!({"color": "#ff0000"})), None);
        assert_eq!(record(&s, "layer.select", &json!({"layer": a.0})), None);
    }

    #[test]
    fn references_resolve_in_the_document_they_play_on() {
        let (s, a, t1, t2) = session();
        assert_eq!(resolve(&s, "layer.hideLayers", json!({"layer": {"name": "A"}})).unwrap(), json!({"layer": a.0}));
        assert_eq!(resolve(&s, "layer.hideLayers", json!({"layer": {"current": true}})).unwrap(), json!({"layer": t2.0}));
        assert_eq!(
            resolve(&s, "layer.moveTo", json!({"layers": [{"name": "Twin", "occurrence": 1}, t1.0], "target": {"name": "A"}})).unwrap(),
            json!({"layers": [t2.0, t1.0], "target": a.0})
        );
        // Plain params pass through.
        assert_eq!(resolve(&s, "layer.hideLayers", json!({"layer": 5})).unwrap(), json!({"layer": 5}));
        // A Show Only step an older recording tied to a layer plays on the current one; a live
        // ⌥-click (a plain id) still names its layer.
        assert_eq!(resolve(&s, "layer.showOnly", json!({"layer": {"name": "A"}})).unwrap(), json!({}));
        assert_eq!(resolve(&s, "layer.showOnly", json!({"layers": [{"name": "A"}]})).unwrap(), json!({}));
        assert_eq!(resolve(&s, "layer.showOnly", json!({"layer": a.0})).unwrap(), json!({"layer": a.0}));
        // Bad references are errors, not panics.
        for bad in [
            json!({"layer": {"name": "nope"}}),
            json!({"layer": {"name": 3}}),
            json!({"layer": {"name": "Twin", "occurrence": 2}}),
            json!({"layer": {"name": "Twin", "occurrence": -1}}),
            json!({"layers": [{"name": "nope"}]}),
        ] {
            assert!(resolve(&s, "layer.hideLayers", bad.clone()).is_err(), "{bad}");
        }
        assert!(resolve(&Session::new(), "layer.hideLayers", json!({"layer": {"current": true}})).is_err());
    }

    #[test]
    fn describe_names_the_layer() {
        assert_eq!(describe(&json!({"name": "A"})), Some(Described::Named("A".into())));
        assert_eq!(describe(&json!({"name": "A", "occurrence": 1})), Some(Described::Named("A (2)".into())));
        assert_eq!(describe(&json!({"current": true})), Some(Described::Current));
        assert_eq!(describe(&json!(4)), None);
    }
}
