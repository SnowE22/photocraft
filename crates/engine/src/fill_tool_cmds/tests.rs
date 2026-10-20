use super::*;
use photocraft_doc::LayerId;

const RED: &str = "#ff0000";

/// A 40 × 40 document: a white Background, "Lines" (a box with a 3 px gap at the top) and an
/// empty "Colour" layer above, which is active.
fn inked(depth: u32) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Lines"})).unwrap();
    let lines = s.active().unwrap().active_layer.unwrap();
    s.edit("ink", |doc, _| {
        let surf = doc.layer_mut(lines).unwrap().surface_mut().unwrap();
        let ink = [0.0, 0.0, 0.0, 1.0];
        for r in [Rect::new(5, 5, 18, 6), Rect::new(21, 5, 35, 6), Rect::new(5, 34, 35, 35), Rect::new(5, 5, 6, 35), Rect::new(34, 5, 35, 35)] {
            surf.fill_rect(r, &ink);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.new.layer", json!({"name": "Colour"})).unwrap();
    let colour = s.active().unwrap().active_layer.unwrap();
    (s, lines, colour)
}

fn rgba(s: &Session, id: LayerId, x: i32, y: i32) -> [f32; 4] {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)
}

fn is_red(c: [f32; 4]) -> bool {
    c[0] > 0.99 && c[1] < 0.01 && c[3] > 0.99
}

#[test]
fn fills_the_colour_layer_from_line_art_on_a_reference_layer_through_a_gap() {
    for depth in [8, 16, 32] {
        let (mut s, lines, colour) = inked(depth);
        s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap();
        let p = json!({"x": 20, "y": 20, "reference": "reference", "closeGap": 4, "antiAlias": false, "color": RED});
        let r = s.execute(FILL_ID, p).unwrap();
        assert_eq!(r["filled"], true, "{depth}");
        assert!(is_red(rgba(&s, colour, 20, 20)) && is_red(rgba(&s, colour, 6, 6)), "{depth}");
        assert_eq!(rgba(&s, colour, 0, 0)[3], 0.0, "the gap held ({depth})");
        assert_eq!(rgba(&s, colour, 5, 20)[3], 0.0, "not on the line");
        assert_eq!(rgba(&s, lines, 20, 20)[3], 0.0, "only the editing layer is painted");
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Fill Tool"));
        // Without Close Gap the same click runs out of the box.
        s.execute("edit.undo", json!({})).unwrap();
        s.execute(FILL_ID, json!({"x": 20, "y": 20, "reference": "reference", "antiAlias": false, "color": RED})).unwrap();
        assert!(is_red(rgba(&s, colour, 0, 0)));
    }
}

#[test]
fn area_scaling_tucks_the_fill_under_the_lines() {
    let (mut s, _, colour) = inked(8);
    s.execute(FILL_ID, json!({"x": 20, "y": 20, "reference": "all", "closeGap": 4, "areaScaling": 2, "antiAlias": false, "color": RED})).unwrap();
    assert!(is_red(rgba(&s, colour, 5, 20)), "under the 1 px line");
    assert_eq!(rgba(&s, colour, 4, 20)[3], 0.0, "and no further");
}

#[test]
fn each_reference_reads_its_own_layers() {
    let (mut s, lines, colour) = inked(8);
    let at = |s: &mut Session, reference: &str| {
        s.execute(FILL_ID, json!({"x": 20, "y": 20, "reference": reference, "closeGap": 4, "antiAlias": false, "color": RED}))
    };
    // The empty editing layer alone: everything is one area.
    at(&mut s, "layer").unwrap();
    assert!(is_red(rgba(&s, colour, 0, 0)));
    s.execute("edit.undo", json!({})).unwrap();
    // Every visible layer: the lines on the white paper.
    at(&mut s, "all").unwrap();
    assert!(is_red(rgba(&s, colour, 20, 20)) && rgba(&s, colour, 0, 0)[3] == 0.0);
    s.execute("edit.undo", json!({})).unwrap();
    // No reference layer yet.
    let e = at(&mut s, "reference").unwrap_err().to_string();
    assert!(e.contains("reference layer"), "{e}");
    // The selected layers: Colour (active) and Lines.
    s.active_mut().unwrap().selected_layers = vec![lines, colour];
    at(&mut s, "selected").unwrap();
    assert!(is_red(rgba(&s, colour, 20, 20)) && rgba(&s, colour, 0, 0)[3] == 0.0);
    s.execute("edit.undo", json!({})).unwrap();
    // A hidden reference layer is not read.
    s.execute(REFERENCE_ID, json!({"layer": lines, "on": true})).unwrap();
    s.edit("hide", |doc, _| {
        doc.layer_mut(lines).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    assert!(at(&mut s, "reference").is_err());
}

#[test]
fn the_editing_layer_can_be_left_out_of_the_reference() {
    let (mut s, _, colour) = inked(8);
    // A red stripe on the Colour layer splits the box.
    s.edit("stripe", |doc, _| {
        doc.layer_mut(colour).unwrap().surface_mut().unwrap().fill_rect(Rect::new(6, 20, 34, 21), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let p = |exclude: Value| json!({"x": 20, "y": 10, "reference": "all", "closeGap": 4, "antiAlias": false, "color": "#0000ff", "exclude": exclude});
    s.execute(FILL_ID, p(json!([]))).unwrap();
    assert_eq!(rgba(&s, colour, 20, 30)[3], 0.0, "the stripe stops it");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute(FILL_ID, p(json!(["layer"]))).unwrap();
    assert!(rgba(&s, colour, 20, 30)[2] > 0.99, "the stripe is not read, so the fill goes past it");
}

#[test]
fn enclose_and_fill_and_paint_unfilled_area_fill_the_enclosed_areas() {
    let (mut s, lines, colour) = inked(8);
    s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap();
    let base = json!({"reference": "reference", "closeGap": 4, "antiAlias": false, "color": RED});
    let with = |k: &str, v: Value| {
        let mut p = base.clone();
        p[k] = v;
        p
    };
    // A lasso round the whole box.
    let r = s.execute(FILL_ID, with("lasso", json!([[2, 2], [38, 2], [38, 38], [2, 38]]))).unwrap();
    assert_eq!(r["filled"], true);
    assert!(is_red(rgba(&s, colour, 20, 20)) && rgba(&s, colour, 1, 1)[3] == 0.0);
    s.execute("edit.undo", json!({})).unwrap();
    // A lasso round half of it encloses nothing.
    assert_eq!(s.execute(FILL_ID, with("lasso", json!([[2, 2], [20, 2], [20, 38], [2, 38]]))).unwrap()["filled"], false);
    // A stroke across the box covers only part of it; one covering all of it fills it.
    let mut p = with("stroke", json!([[0, 20], [39, 20]]));
    p["size"] = json!(6);
    assert_eq!(s.execute(FILL_ID, p.clone()).unwrap()["filled"], false);
    p["size"] = json!(80);
    s.execute(FILL_ID, p).unwrap();
    assert!(is_red(rgba(&s, colour, 20, 20)));
}

#[test]
fn transparent_contents_erase_and_blend_modes_apply() {
    let (mut s, lines, colour) = inked(8);
    s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap();
    let p = json!({"x": 20, "y": 20, "reference": "reference", "closeGap": 4, "antiAlias": false, "color": "#808080"});
    s.execute(FILL_ID, p.clone()).unwrap();
    let mut erase = p.clone();
    erase["contents"] = json!("transparent");
    erase["opacity"] = json!(50);
    s.execute(FILL_ID, erase).unwrap();
    assert!((rgba(&s, colour, 20, 20)[3] - 0.5).abs() < 0.01);
    let mut multiply = p;
    multiply["mode"] = json!("multiply");
    s.execute(FILL_ID, multiply).unwrap();
    let c = rgba(&s, colour, 20, 20);
    assert!(c[3] > 0.7 && c[0] < 0.45, "{c:?}");
}

#[test]
fn bad_params_are_errors_that_change_nothing() {
    let (mut s, _, _) = inked(8);
    let steps = s.active().unwrap().history.past_len();
    for p in [
        json!({}),
        json!({"x": 20}),
        json!({"x": 20, "y": 20, "tolerance": "lots"}),
        json!({"x": 20, "y": 20, "reference": "everything"}),
        json!({"x": 20, "y": 20, "scalingMode": "oval"}),
        json!({"x": 20, "y": 20, "contiguous": 1}),
        json!({"x": 20, "y": 20, "exclude": ["paper"]}),
        json!({"x": 20, "y": 20, "exclude": "layer"}),
        json!({"x": 20, "y": 20, "contents": "pattern"}),
        json!({"x": 20, "y": 20, "contents": "color"}),
        json!({"x": 20, "y": 20, "mode": "sparkle"}),
        json!({"x": 20, "y": 20, "targetColor": "black"}),
        json!({"lasso": "round"}),
        json!({"lasso": [[1]]}),
        json!({"stroke": [[1, "a"]]}),
        json!({"stroke": [[1, 1]], "size": "big"}),
        json!({"x": 20, "y": 20, "target": {"channel": 9}}),
    ] {
        assert!(s.execute(FILL_ID, p.clone()).is_err(), "{p}");
    }
    // Out of range numbers are clamped, and a click off the canvas fills nothing.
    for p in [json!({"x": 20, "y": 20, "closeGap": 1e9, "areaScaling": -1e9, "tolerance": -5}), json!({"x": -5, "y": 1e300})] {
        s.execute(FILL_ID, p.clone()).unwrap();
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert!(s.execute(REFERENCE_ID, json!({"on": "yes"})).is_err());
    assert!(s.execute(REFERENCE_ID, json!({"layer": 999_999})).is_err());
    let mut empty = Session::new();
    assert!(empty.execute(FILL_ID, json!({"x": 1, "y": 1})).is_err());
    assert!(empty.execute(REFERENCE_ID, json!({})).is_err());
}

#[test]
fn set_reference_toggles_and_undoes() {
    let (mut s, lines, _) = inked(8);
    let flag = |s: &Session| s.active().unwrap().doc.layer(lines).unwrap().reference;
    assert_eq!(s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap()["reference"], true);
    assert!(flag(&s));
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Set as Reference Layer"));
    let steps = s.active().unwrap().history.past_len();
    s.execute(REFERENCE_ID, json!({"layer": lines, "on": true})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no step when nothing changes");
    s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap();
    assert!(!flag(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(flag(&s));
}

/// The referenced composite is kept between clicks, and any change to the line art (its pixels,
/// its visibility, its opacity, which layers are referenced) is seen by the next click.
#[test]
fn the_kept_reference_follows_every_change_to_the_line_art() {
    let (mut s, lines, colour) = inked(8);
    s.execute(REFERENCE_ID, json!({"layer": lines})).unwrap();
    let click = |s: &mut Session, x: i32, y: i32| {
        s.execute(FILL_ID, json!({"x": x, "y": y, "reference": "reference", "closeGap": 4, "antiAlias": false, "color": RED})).unwrap()
    };
    // The box (gap closed): the outside stays empty.
    s.execute(FILL_ID, json!({"x": 20, "y": 20, "reference": "reference", "closeGap": 4, "antiAlias": false, "color": RED})).unwrap();
    assert!(is_red(rgba(&s, colour, 20, 20)) && !is_red(rgba(&s, colour, 2, 2)));
    assert!(s.fill_reference.lock().unwrap().is_some(), "kept for the next click");
    // A wall drawn across the box splits it: the next click sees the wall.
    s.edit("wall", |doc, _| {
        doc.layer_mut(lines).unwrap().surface_mut().unwrap().fill_rect(Rect::new(5, 19, 35, 21), &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": colour.0})).unwrap();
    s.execute(FILL_ID, json!({"x": 20, "y": 25, "reference": "reference", "closeGap": 4, "antiAlias": false, "color": "#00ff00"})).unwrap();
    let green = |s: &Session, x, y| {
        let c = rgba(s, colour, x, y);
        c[1] > 0.99 && c[0] < 0.01
    };
    assert!(green(&s, 20, 25) && !green(&s, 20, 12), "the wall stops the fill");
    // Hidden line art: nothing is referenced any more.
    s.execute("layer.setProps", json!({"layer": lines.0, "visible": false})).unwrap();
    s.execute("layer.select", json!({"layer": colour.0})).unwrap();
    assert!(s.execute(FILL_ID, json!({"x": 2, "y": 2, "reference": "reference"})).is_err(), "no visible reference");
    s.execute("layer.setProps", json!({"layer": lines.0, "visible": true, "opacity": 0.05})).unwrap();
    s.execute("layer.select", json!({"layer": colour.0})).unwrap();
    // At 5 % the lines are within the colour margin: the fill crosses them now.
    let r = click(&mut s, 2, 2);
    assert_eq!(r["filled"], true);
    assert!(is_red(rgba(&s, colour, 2, 2)) && is_red(rgba(&s, colour, 20, 12)), "faint lines no longer stop it");
}
