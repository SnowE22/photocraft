use serde_json::json;

use crate::Session;
use photocraft_doc::LayerId;

/// A 40×20 document with a red layer that has a reveal-all layer mask.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Red"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    (s, id)
}

fn alpha_at(s: &Session, x: i32, y: i32) -> f32 {
    let doc = &s.active().unwrap().doc;
    let px = photocraft_compose::render(doc, photocraft_geom::Rect::from_xywh(x, y, 1, 1)).px[0];
    // Red over white: green is 0 where the red layer shows and 1 where its masks hide it.
    1.0 - px[1]
}

#[test]
fn a_second_mask_hides_what_it_hides_and_both_masks_multiply() {
    let (mut s, id) = session();
    assert!(alpha_at(&s, 30, 10) > 0.99, "the red layer shows");
    // The second mask from a selection of the left half: the right half is hidden.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute(super::ADD, json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none(), "the selection became the mask");
    assert!(alpha_at(&s, 30, 10) < 0.01 && alpha_at(&s, 10, 10) > 0.99);
    // The first mask hides the top half as well: only the bottom left shows.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000", "target": "mask"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert!(alpha_at(&s, 10, 5) < 0.01 && alpha_at(&s, 10, 15) > 0.99 && alpha_at(&s, 30, 15) < 0.01);
    // Painting on the second mask (`"target":"secondMask"`) edits it, not the first.
    s.execute("select.rect", json!({"x": 20, "y": 10, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff", "target": "secondMask"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert!(alpha_at(&s, 30, 15) > 0.99 && alpha_at(&s, 30, 5) < 0.01);
    let l = s.active().unwrap().doc.layer(id).unwrap().clone();
    assert_eq!(l.mask.as_ref().unwrap().value(30, 15), 1.0, "the first mask is untouched there");
    // Disabled, it stops hiding; deleted, it is gone; undo brings it back.
    s.execute(super::ENABLED, json!({"enabled": false})).unwrap();
    assert!(alpha_at(&s, 30, 15) > 0.99 && alpha_at(&s, 10, 15) > 0.99);
    s.execute(super::ENABLED, json!({"enabled": true})).unwrap();
    s.execute(super::DELETE, json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().second_mask.is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(id).unwrap().second_mask.is_some());
}

#[test]
fn moving_the_layer_moves_both_masks() {
    let (mut s, id) = session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 20})).unwrap();
    s.execute(super::ADD, json!({})).unwrap();
    s.execute("layer.translate", json!({"dx": 5, "dy": 0})).unwrap();
    let l = s.active().unwrap().doc.layer(id).unwrap().clone();
    let m = l.second_mask.unwrap();
    assert!(m.value(12, 5) > 0.99 && m.value(2, 5) < 0.01, "the second mask moved with the layer");
}

#[test]
fn convert_to_vector_mask_traces_the_revealed_area() {
    let (mut s, id) = session();
    s.execute("select.rect", json!({"x": 10, "y": 4, "width": 20, "height": 12})).unwrap();
    s.execute(super::ADD, json!({})).unwrap();
    s.execute(super::TO_VECTOR, json!({})).unwrap();
    let l = s.active().unwrap().doc.layer(id).unwrap().clone();
    assert!(l.second_mask.is_none());
    let vm = l.vector_mask.expect("a vector mask");
    assert!(!vm.path.subpaths.is_empty());
    assert!(alpha_at(&s, 20, 10) > 0.99 && alpha_at(&s, 2, 2) < 0.01);
    // With a vector mask there, converting again is refused.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute(super::ADD, json!({})).unwrap();
    assert!(s.execute(super::TO_VECTOR, json!({})).is_err());
}

#[test]
fn bad_params_are_errors_not_panics() {
    let (mut s, _) = session();
    for (id, p) in [
        (super::DELETE, json!({})),
        (super::ENABLED, json!({"enabled": true})),
        (super::TO_VECTOR, json!({})),
        (super::ADD, json!({"hide": "yes"})),
        (super::ADD, json!({"layer": 999_999})),
        (super::ENABLED, json!({"enabled": 3})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
    s.execute(super::ADD, json!({"hide": true})).unwrap();
    assert!(s.execute(super::ADD, json!({})).is_err(), "one second mask a layer");
    assert!(s.execute(super::TO_VECTOR, json!({"tolerance": 99})).is_err());
    // A layer without a layer mask takes none.
    s.execute("layer.new.layer", json!({})).unwrap();
    assert!(s.execute(super::ADD, json!({})).is_err());
}
