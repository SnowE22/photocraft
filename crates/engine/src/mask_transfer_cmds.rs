//! Dragging a mask thumbnail onto another layer in the Layers panel (`layer.layerMask.transfer`):
//! the mask moves there, or with ⌥ (Alt) is copied, as in Photoshop. A layer or vector mask; one
//! that the target already has is replaced. One history step.

use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const TRANSFER_ID: &str = "layer.layerMask.transfer";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: TRANSFER_ID.into(), msg: msg.into() }
}

fn layer(p: &Value, key: &str) -> Result<LayerId> {
    p.get(key).and_then(Value::as_u64).map(LayerId).ok_or_else(|| bad(format!("`{key}` must be a layer id")))
}

fn transfer(s: &mut Session, p: &Value) -> Result<Value> {
    let (from, to) = (layer(p, "from")?, layer(p, "to")?);
    let copy = match p.get("copy") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("`copy` must be true or false"))?,
    };
    let vector = match p.get("kind").and_then(Value::as_str) {
        None | Some("pixel") => false,
        Some("vector") => true,
        Some(_) => return Err(bad("`kind` must be pixel or vector")),
    };
    if p.get("kind").is_some_and(|k| !k.is_null() && !k.is_string()) {
        return Err(bad("`kind` must be pixel or vector"));
    }
    if from == to {
        return Err(bad("the mask is already on that layer"));
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let source = d.doc.layer(from).ok_or(EngineError::NoLayer(from))?;
    d.doc.layer(to).ok_or(EngineError::NoLayer(to))?;
    let has = if vector { source.vector_mask.is_some() } else { source.mask.is_some() };
    if !has {
        return Err(bad(if vector { "that layer has no vector mask" } else { "that layer has no layer mask" }));
    }
    if d.doc.is_background(to) {
        return Err(EngineError::Other("The Background can't have a mask. Make it a layer first (Layer › New › Layer from Background).".into()));
    }
    if d.doc.effective_locks(to).all || (!copy && d.doc.effective_locks(from).all) {
        return Err(EngineError::Other("Could not move the mask because the layer is locked.".into()));
    }
    let label = match (copy, vector) {
        (false, false) => "Move Layer Mask",
        (true, false) => "Copy Layer Mask",
        (false, true) => "Move Vector Mask",
        (true, true) => "Copy Vector Mask",
    };
    s.edit(label, |doc, _| {
        let src = doc.layer_mut(from).ok_or(EngineError::NoLayer(from))?;
        if vector {
            let mask = if copy { src.vector_mask.clone() } else { src.vector_mask.take() };
            doc.layer_mut(to).ok_or(EngineError::NoLayer(to))?.vector_mask = mask;
        } else {
            let mask = if copy { src.mask.clone() } else { src.mask.take() };
            doc.layer_mut(to).ok_or(EngineError::NoLayer(to))?.mask = mask;
        }
        Ok(())
    })?;
    Ok(json!({ "from": from.0, "to": to.0, "copied": copy }))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: TRANSFER_ID,
        label: "Move Layer Mask",
        menu: &[],
        shortcut: None,
        params: r##"{"from":layer id,"to":layer id,"copy":bool=false (⌥: copy instead of move),"kind":"pixel|vector"="pixel"} (replaces a mask the target has)"##,
        enabled: has_doc,
        run: transfer,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;

    /// A document with two pixel layers; the lower has a mask hiding its left half.
    fn session() -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        let a = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.edit("hide", |doc, _| {
            doc.layer_mut(a).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(0, 0, 10, 10), &[0.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
        let b = s.active().unwrap().active_layer.unwrap();
        (s, a, b)
    }

    fn mask_at(s: &Session, id: LayerId, x: i32) -> Option<f32> {
        s.active().unwrap().doc.layer(id).unwrap().mask.as_ref().map(|m| m.surface.sample_channel(x, 5, 0))
    }

    #[test]
    fn a_dragged_mask_moves_and_with_alt_is_copied() {
        let (mut s, a, b) = session();
        s.execute(TRANSFER_ID, json!({"from": a.0, "to": b.0})).unwrap();
        assert_eq!((mask_at(&s, a, 2), mask_at(&s, b, 2), mask_at(&s, b, 15)), (None, Some(0.0), Some(1.0)), "moved, as it was");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Move Layer Mask"));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!((mask_at(&s, a, 2), mask_at(&s, b, 2)), (Some(0.0), None), "one undo step puts it back");
        s.execute(TRANSFER_ID, json!({"from": a.0, "to": b.0, "copy": true})).unwrap();
        assert_eq!((mask_at(&s, a, 2), mask_at(&s, b, 2)), (Some(0.0), Some(0.0)), "copied");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Copy Layer Mask"));
    }

    #[test]
    fn a_vector_mask_moves_too() {
        let (mut s, a, b) = session();
        s.edit("vector", |doc, _| {
            doc.layer_mut(a).unwrap().vector_mask = Some(photocraft_doc::VectorMask::new(Default::default()));
            Ok(())
        })
        .unwrap();
        s.execute(TRANSFER_ID, json!({"from": a.0, "to": b.0, "kind": "vector"})).unwrap();
        let d = s.active().unwrap();
        assert!(d.doc.layer(a).unwrap().vector_mask.is_none() && d.doc.layer(b).unwrap().vector_mask.is_some());
        assert!(d.doc.layer(a).unwrap().mask.is_some(), "the layer mask stays");
    }

    #[test]
    fn bad_transfers_are_errors_that_change_nothing() {
        let (mut s, a, b) = session();
        let bg = s.active().unwrap().doc.layers[0].id;
        let steps = s.active().unwrap().history.past_len();
        for p in [
            json!({}),
            json!({"from": a.0}),
            json!({"from": a.0, "to": a.0}),
            json!({"from": b.0, "to": a.0}),
            json!({"from": a.0, "to": bg.0}),
            json!({"from": a.0, "to": 999_999}),
            json!({"from": a.0, "to": b.0, "copy": "yes"}),
            json!({"from": a.0, "to": b.0, "kind": "alpha"}),
            json!({"from": a.0, "to": b.0, "kind": 3}),
            json!({"from": a.0, "to": b.0, "kind": "vector"}),
            json!({"from": "a", "to": b.0}),
        ] {
            assert!(s.execute(TRANSFER_ID, p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(Session::new().execute(TRANSFER_ID, json!({"from": 1, "to": 2})).is_err());
    }
}
