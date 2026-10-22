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
fn enclose_and_fill_reads_all_layers_over_a_white_background() {
    // The Fill tool refers to All Layers by default, and a page has a white Background: it is
    // left out, as Clip Studio's paper is, so the transparent areas between the lines fill.
    let (mut s, _, colour) = inked(8);
    let base = json!({"reference": "all", "closeGap": 4, "antiAlias": false, "color": RED});
    let mut p = base.clone();
    p["lasso"] = json!([[2, 2], [38, 2], [38, 38], [2, 38]]);
    assert_eq!(s.execute(FILL_ID, p).unwrap()["filled"], true);
    assert!(is_red(rgba(&s, colour, 20, 20)) && rgba(&s, colour, 1, 1)[3] == 0.0);
    s.execute("edit.undo", json!({})).unwrap();
    let mut p = base.clone();
    p["stroke"] = json!([[0, 20], [39, 20]]);
    p["size"] = json!(80);
    assert_eq!(s.execute(FILL_ID, p).unwrap()["filled"], true);
    assert!(is_red(rgba(&s, colour, 20, 20)));
    // White and Transparent still reads the white paper.
    s.execute("edit.undo", json!({})).unwrap();
    let mut p = base;
    p["lasso"] = json!([[2, 2], [38, 2], [38, 38], [2, 38]]);
    p["targetColor"] = json!("whiteAndTransparent");
    assert_eq!(s.execute(FILL_ID, p).unwrap()["filled"], true);
}

#[test]
fn the_editing_layers_paint_blocks_the_fill_and_the_fill_goes_behind_it() {
    for depth in [8, 16, 32] {
        let (mut s, _, colour) = inked(depth);
        // A pale stroke on the colour layer across the 3 px gap, with a faint edge inside the box.
        s.edit("close the gap", |doc, _| {
            let surf = doc.layer_mut(colour).unwrap().surface_mut().unwrap();
            surf.fill_rect(Rect::new(16, 5, 23, 6), &[0.95, 0.95, 1.0, 1.0]);
            surf.fill_rect(Rect::new(16, 6, 23, 7), &[0.95, 0.95, 1.0, 0.3]);
            Ok(())
        })
        .unwrap();
        let p = json!({"x": 20, "y": 20, "reference": "all", "tolerance": 60, "antiAlias": false, "color": RED});
        s.execute(FILL_ID, p.clone()).unwrap();
        assert!(is_red(rgba(&s, colour, 0, 0)), "a pale stroke doesn't stop a fill at a high tolerance ({depth})");
        s.execute("edit.undo", json!({})).unwrap();
        let mut blocked = p.clone();
        blocked["layerBlocks"] = json!(true);
        s.execute(FILL_ID, blocked.clone()).unwrap();
        assert!(is_red(rgba(&s, colour, 20, 20)), "{depth}");
        assert_eq!(rgba(&s, colour, 0, 0)[3], 0.0, "the stroke on the editing layer holds ({depth})");
        // The stroke stays; its faint edge has the fill behind it: no see-through line.
        let core = rgba(&s, colour, 20, 5);
        assert!(core[2] > 0.99 && core[0] > 0.9 && core[3] > 0.99, "{core:?} ({depth})");
        let edge = rgba(&s, colour, 20, 6);
        assert!(edge[3] > 0.99 && edge[0] > 0.9 && edge[1] < 0.5, "{edge:?} ({depth})");
        // Editing Layer alone: the option has nothing to add (the layer is what is read).
        s.execute("edit.undo", json!({})).unwrap();
        blocked["reference"] = json!("layer");
        assert!(s.execute(FILL_ID, blocked).is_ok());
        assert!(s.execute(FILL_ID, json!({"x": 1, "y": 1, "layerBlocks": "yes"})).is_err());
    }
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

/// With Refer to All Layers each fill changes a referenced layer, so the next click's composite
/// is the kept one patched where that fill painted: the same pixels as compositing everything
/// again, with an adjustment layer, a group and a layer mask in the document.
#[test]
fn all_layers_patch_the_kept_composite_where_the_last_fill_painted() {
    let (mut s, _lines, colour) = inked(8);
    s.execute("layer.newAdjustmentLayer.curves", json!({"points": [[0, 0], [128, 150], [255, 255]]})).unwrap();
    s.execute("layer.new.layer", json!({"name": "In group"})).unwrap();
    s.execute("select.rect", json!({"x": 30, "y": 30, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#2040ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.select", json!({"layer": colour.0})).unwrap();
    let full = |s: &Session| {
        let doc = s.active().unwrap().doc.clone();
        composite_rgba8(&doc, doc.bounds())
    };
    let read = |s: &Session| reference_image(s, Reference::All, Exclude::default(), &json!({}), false).unwrap().1;
    for (i, (x, y, c)) in [(20, 20, "#ff0000"), (2, 2, "#00ff00"), (20, 25, "#0000ff"), (37, 2, "#ffff00")].into_iter().enumerate() {
        s.execute(FILL_ID, json!({"x": x, "y": y, "reference": "all", "closeGap": 4, "color": c})).unwrap();
        // What the next click reads: the kept composite, patched where this fill painted (or
        // made again when the fill covered most of the page).
        let now = read(&s);
        assert_eq!(*now, full(&s), "click {i}: the patched composite is the full one");
    }
    // On a page much bigger than a tile, a small change is patched into the kept composite in
    // place.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 1024, "background": "white"})).unwrap();
    s.execute("layer.newAdjustmentLayer.curves", json!({"points": [[0, 0], [128, 150], [255, 255]]})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute(FILL_ID, json!({"x": 5, "y": 5, "reference": "all", "tolerance": 0, "color": RED})).unwrap();
    let first = Arc::as_ptr(&read(&s));
    s.execute("select.rect", json!({"x": 600, "y": 600, "width": 30, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff00ff"})).unwrap();
    let now = read(&s);
    assert_eq!(Arc::as_ptr(&now), first, "patched in place, not made again");
    assert_eq!(*now, full(&s));
}

/// Type and shape layers are kept in the composite like pixel layers: a fill beside them patches
/// it, and the next click sees edited type and a moved shape.
#[test]
fn type_and_shapes_stay_in_the_kept_composite_and_are_followed() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 600, "background": "white"})).unwrap();
    let text = s.execute("type.create", json!({"text": "Hi", "size": 40, "x": 600, "y": 100, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [20, 300, 120, 400], "fill": "#0000ff"})).unwrap();
    let shape = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let full = |s: &Session| {
        let doc = s.active().unwrap().doc.clone();
        composite_rgba8(&doc, doc.bounds())
    };
    let read = |s: &Session| reference_image(s, Reference::All, Exclude::default(), &json!({}), false).unwrap().1;
    s.execute(FILL_ID, json!({"x": 5, "y": 5, "reference": "all", "tolerance": 0, "color": RED})).unwrap();
    let first = Arc::as_ptr(&read(&s));
    s.execute("select.rect", json!({"x": 900, "y": 500, "width": 30, "height": 30})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff00ff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let now = read(&s);
    assert_eq!(Arc::as_ptr(&now), first, "kept with type and a shape in the document");
    assert_eq!(*now, full(&s));
    drop(now);
    s.execute("type.edit", json!({"layer": text, "text": "Hello there"})).unwrap();
    assert_eq!(*read(&s), full(&s), "the edited type");
    s.execute("layer.translate", json!({"layer": shape.0, "dx": 30, "dy": 10})).unwrap();
    assert_eq!(*read(&s), full(&s), "the moved shape");
}

/// A drag going on from its click (`gesture`) fills each further area like the first one it
/// reaches, skips what it filled and areas unlike the first (the lines), and is one undo step.
#[test]
fn a_drag_fills_each_like_area_it_reaches_in_one_step() {
    let (mut s, _lines, colour) = inked(8);
    let fill = |s: &mut Session, x: i32, y: i32| {
        s.execute(FILL_ID, json!({"x": x, "y": y, "reference": "all", "closeGap": 4, "antiAlias": false, "color": RED, "gesture": "g1", "coalesce": "g1"})).unwrap()["filled"].clone()
    };
    let steps = s.active().unwrap().history.past_len();
    assert_eq!(fill(&mut s, 20, 20), true, "the click fills the box");
    assert_eq!(fill(&mut s, 25, 25), false, "already filled by this drag");
    assert_eq!(fill(&mut s, 5, 12), false, "a line is unlike the white the drag started on");
    assert_eq!(fill(&mut s, 2, 2), true, "the white outside the box is like it: filled too");
    assert!(is_red(rgba(&s, colour, 20, 20)) && is_red(rgba(&s, colour, 2, 2)));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step for the drag");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!is_red(rgba(&s, colour, 20, 20)) && !is_red(rgba(&s, colour, 2, 2)));
    // A new gesture starts from its own click again.
    let r = s.execute(FILL_ID, json!({"x": 5, "y": 12, "reference": "all", "gesture": "g2"})).unwrap();
    assert_eq!(r["filled"], true, "a new drag starting on a line fills the line");
    assert!(s.execute(FILL_ID, json!({"x": 1, "y": 1, "gesture": 5})).is_err());
}

/// The Magic Wand with the Fill tool's options selects what that fill would fill.
#[test]
fn the_magic_wand_closes_gaps_like_the_fill() {
    let (mut s, _lines, _colour) = inked(8);
    let sel = |s: &Session, x: i32, y: i32| s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0));
    s.execute("select.magicWand", json!({"x": 20, "y": 20, "tolerance": 25, "sampleAllLayers": true, "antiAlias": false})).unwrap();
    assert!(sel(&s, 20, 20) > 0.5 && sel(&s, 2, 2) > 0.5, "without Close Gap it runs out through the gap");
    s.execute("select.magicWand", json!({"x": 20, "y": 20, "tolerance": 25, "closeGap": 4, "reference": "all", "antiAlias": false})).unwrap();
    assert!(sel(&s, 20, 20) > 0.5 && sel(&s, 2, 2) < 0.5, "with Close Gap it stops at the gap");
    // A drag adds the next like area once, as one step.
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("select.magicWand", json!({"x": 20, "y": 20, "closeGap": 4, "reference": "all", "gesture": "w", "coalesce": "w"})).unwrap();
    assert_eq!(r["selected"], true);
    let r = s.execute("select.magicWand", json!({"x": 22, "y": 22, "closeGap": 4, "reference": "all", "gesture": "w", "coalesce": "w", "mode": "add"})).unwrap();
    assert_eq!(r["added"], false, "already selected by this drag");
    s.execute("select.magicWand", json!({"x": 2, "y": 2, "closeGap": 4, "reference": "all", "gesture": "w", "coalesce": "w", "mode": "add"})).unwrap();
    assert!(sel(&s, 20, 20) > 0.5 && sel(&s, 2, 2) > 0.5);
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
}
