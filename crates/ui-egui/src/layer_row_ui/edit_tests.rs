//! Type editing through real Layers panel mouse and keyboard events (#537).

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

fn harness(session: photocraft_engine::Session, ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut h =
        Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(session, crate::Services::default())
        });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    h
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, button: PointerButton) {
    h.hover_at(at);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    for _ in 0..40 {
        h.step();
    }
    click(h, at, PointerButton::Primary);
    click(h, at, PointerButton::Primary);
}

fn row(h: &Harness<'_, PhotocraftApp>, id: u64) -> super::RowRects {
    super::recorded(&h.ctx).into_iter().find(|r| r.layer == id).expect("layer row drawn")
}

fn thumbnail(h: &Harness<'_, PhotocraftApp>, id: u64) -> Pos2 {
    let r = row(h, id).row;
    // Top-level Pro row: 6 pt padding, 28 pt eye column, 24 pt layer thumbnail.
    pos2(r.left() + 6.0 + 28.0 + 12.0, r.center().y)
}

fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 640, "height": 480})).unwrap();
    s
}

fn text(h: &Harness<'_, PhotocraftApp>, id: u64) -> String {
    match &h.state().session.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => panic!("expected text layer"),
    }
}

#[test]
fn text_thumbnail_opens_inline_editing_and_typing_is_one_undo_step() {
    for ppp in [1.0, 2.0] {
        let mut s = session();
        let id = s.execute("type.create", json!({"text": "Hello 世界", "size": 48, "x": 100, "y": 180})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.new.layer", json!({"name": "Other active layer"})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        let layers = s.active().unwrap().doc.layers.len();
        let mut h = harness(s, ppp);
        let at = thumbnail(&h, id);
        double_click(&mut h, at);
        assert_eq!(h.state().ui.tool, crate::state::Tool::Type);
        let ed = h.state().ui.text_edit.as_ref().expect("inline text editor");
        assert_eq!((ed.layer, ed.anchor, ed.caret, ed.created), (id, 0, 8, false));
        assert!(h.state().ui.dialogs.is_empty(), "no Layer Style dialog");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps, "entering native text adds no edit");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers);
        for typed in ["New", " text"] {
            h.event(Event::Text(typed.into()));
            h.run_steps(2);
        }
        assert_eq!(text(&h, id), "New text");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 1);
        crate::type_tool::commit(h.state_mut());
        assert!(h.state_mut().session.undo());
        assert_eq!(text(&h, id), "Hello 世界");
    }
}

#[test]
fn text_name_still_renames_and_mask_double_click_does_not_edit_text() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Hello", "name": "Title"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("prefs.set", json!({"values": {"tools.doubleClickLayerMaskLaunchesSelectAndMask": false}})).unwrap();
    let mut h = harness(s, 1.0);
    let at = row(&h, id).name.unwrap().center();
    double_click(&mut h, at);
    assert_eq!(super::renaming(&h.ctx), Some(id));
    assert!(h.state().ui.text_edit.is_none());
    super::end_rename(&h.ctx, false);
    h.run_steps(2);
    let masks = crate::mask_thumbs_ui::recorded(&h.ctx, id).unwrap().0;
    let at = masks.iter().find(|(kind, _)| *kind == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1.center();
    double_click(&mut h, at);
    assert!(h.state().ui.text_edit.is_none());
    assert!(h.state().ui.dialogs.is_empty());
}

/// With the preference off, double-clicking a pixel mask targets it and opens its settings.
/// The default preference on launches Select and Mask instead (the test above).
#[test]
fn pixel_mask_double_click_with_preference_off_opens_properties() {
    let mut s = session();
    let id = s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("prefs.set", json!({"values": {"tools.doubleClickLayerMaskLaunchesSelectAndMask": false}})).unwrap();
    let mut h = harness(s, 1.0);
    let masks = crate::mask_thumbs_ui::recorded(&h.ctx, id).unwrap().0;
    let at = masks.iter().find(|(kind, _)| *kind == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1.center();
    double_click(&mut h, at);
    assert!(h.state().ui.mask_target, "the mask is targeted");
    assert!(h.state().ui.panels.properties, "Properties opened");
    assert!(!h.state().ui.dock.collapsed.contains(&crate::dock::Group::Properties), "Properties un-collapsed");
    assert!(h.state().ui.dialogs.is_empty(), "no Select and Mask dialog with the preference off");
}

#[test]
fn vector_mask_double_click_opens_its_properties() {
    let mut s = session();
    let id = s.execute("layer.new.layer", json!({"name": "Vector"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
    let mut h = harness(s, 1.0);
    let masks = crate::mask_thumbs_ui::recorded(&h.ctx, id).unwrap().0;
    let at = masks.iter().find(|(kind, _)| *kind == crate::mask_thumbs_ui::MaskKind::Vector).unwrap().1.center();
    double_click(&mut h, at);
    assert!(h.state().ui.vector_mask_target, "the vector mask is targeted");
    assert!(h.state().ui.panels.properties, "Properties opened");
    assert!(!h.state().ui.dock.collapsed.contains(&crate::dock::Group::Properties), "Properties un-collapsed");
}

#[test]
fn mask_double_click_launches_select_and_mask_with_preference() {
    let mut s = session();
    let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("prefs.set", json!({"values": {"tools.doubleClickLayerMaskLaunchesSelectAndMask": true}})).unwrap();
    let mut h = harness(s, 1.0);
    let masks = crate::mask_thumbs_ui::recorded(&h.ctx, id).unwrap().0;
    let at = masks.iter().find(|(kind, _)| *kind == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1.center();
    double_click(&mut h, at);
    assert!(h.state().ui.mask_target);
    let dialog = h.state().ui.dialogs.last().expect("select and mask dialog should be opened");
    assert_eq!(dialog.fields.get("__command").and_then(|v| v.as_str()), Some("select.refineEdge"));
}

#[test]
fn edit_type_command_rejects_missing_or_non_type_layers_without_changes() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    for has_document in [false, true] {
        if has_document {
            app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
        }
        let tool = app.ui.tool;
        assert!(!crate::menus::is_enabled(&app, "type.editText"));
        assert!(crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).is_err());
        assert_eq!(app.ui.tool, tool);
        assert!(app.ui.text_edit.is_none());
    }
}

#[test]
fn layer_reselection_commits_type_edit_and_cancel_preserves_committed_text() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text": "Hello", "size": 48, "x": 100, "y": 180})).unwrap()["layer"].as_u64().unwrap();
    // Stand-in for a PSD's retained Photoshop rendering, displaced from our glyph layout.
    let cached = {
        let st = s.active_mut().unwrap();
        let mut doc = (*st.doc).clone();
        let LayerContent::Text(t) = &mut doc.layer_mut(LayerId(id)).unwrap().content else { panic!("text") };
        let mut moved = t.clone();
        moved.transform = photocraft_geom::Affine::translate(350.0, 280.0);
        let cache = photocraft_text::shared().lock().unwrap().render(&moved, 72.0, photocraft_color::PixelFormat::RGBA8).1.surface;
        t.cache = Some(cache.clone());
        st.doc = std::sync::Arc::new(doc);
        st.revision += 1;
        cache
    };
    let steps = s.active().unwrap().history.entries().len();
    let mut h = harness(s, 1.0);
    let at = thumbnail(&h, id);
    double_click(&mut h, at);
    let key = h.state().ui.text_edit.as_ref().expect("inline text editor").session.clone();
    h.event(Event::Text("Changed".into()));
    h.run_steps(2);
    // The first row click commits the edit; the second opens a new session (#5584).
    double_click(&mut h, at);
    assert_ne!(h.state().ui.text_edit.as_ref().unwrap().session, key);
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 1);
    crate::type_tool::cancel(h.state_mut());
    assert_eq!(text(&h, id), "Changed");
    assert!(h.state_mut().session.undo());
    assert_eq!(text(&h, id), "Hello");
    let st = h.state().session.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(LayerId(id)).unwrap().content else { panic!("text") };
    assert_eq!(t.cache.as_ref().unwrap().content_bounds(), cached.content_bounds());
}

/// The Color Picker a Solid Color layer's thumbnail opened, if any.
fn picker_on(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    h.state().ui.dialogs.iter().any(|d| d.fields.get("__params").and_then(|p| p.get("layer")) == Some(&json!(id)))
}

fn wait(h: &mut Harness<'_, PhotocraftApp>, seconds: f32) {
    for _ in 0..(seconds * 60.0).round() as usize {
        h.step();
    }
}

/// User report: a double-click on a Solid Color layer's thumbnail sometimes didn't open the Color
/// Picker. The two clicks count as Windows (and Photoshop) counts them: up to 0.5 s apart (egui's
/// own double-click is 0.3 s), and on the same part of the same layer's row even when the row
/// moved between them (the first click can change what the panels show).
#[test]
fn a_solid_color_thumbnail_double_clicked_at_windows_speed_opens_the_color_picker() {
    let mut s = session();
    let id = s.execute("layer.newFillLayer.solidColor", json!({"color": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h = harness(s, 1.0);
    wait(&mut h, 1.0);
    // 0.4 s apart: a double-click on Windows, two clicks for egui.
    let at = thumbnail(&h, id);
    click(&mut h, at, PointerButton::Primary);
    wait(&mut h, 0.3);
    click(&mut h, at, PointerButton::Primary);
    assert!(picker_on(&h, id), "the Color Picker opened");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!picker_on(&h, id));
    // The row moves between the clicks: still the same layer's thumbnail.
    wait(&mut h, 1.0);
    click(&mut h, at, PointerButton::Primary);
    h.state_mut().run("layer.new.layer", json!({"name": "Above"})).unwrap();
    h.state_mut().run("layer.select", json!({"layer": id})).unwrap();
    h.run_steps(2);
    let moved = thumbnail(&h, id);
    assert!(moved.y > at.y + 10.0, "the row moved down: {at:?} → {moved:?}");
    click(&mut h, moved, PointerButton::Primary);
    assert!(picker_on(&h, id), "the moved row's second click completes the double-click");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    // Too slow (0.7 s), or on another layer: two single clicks.
    wait(&mut h, 1.0);
    click(&mut h, moved, PointerButton::Primary);
    wait(&mut h, 0.6);
    click(&mut h, moved, PointerButton::Primary);
    assert!(!picker_on(&h, id), "0.7 s apart is no double-click");
}

/// Double-clicking an adjustment layer's thumbnail opens Properties on its settings, expanding the
/// panel when it is collapsed to an icon (Photoshop).
#[test]
fn double_clicking_an_adjustment_thumbnail_opens_properties() {
    let mut s = session();
    let id = s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h = harness(s, 1.0);
    assert!(h.state().ui.dock.collapsed.contains(&crate::dock::Group::Properties), "collapsed to start with");
    let at = thumbnail(&h, id);
    double_click(&mut h, at);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(id)));
    assert!(app.ui.panels.properties, "Properties is shown");
    assert!(!app.ui.dock.collapsed.contains(&crate::dock::Group::Properties), "and expanded");
    assert!(!app.ui.mask_target, "on the adjustment's settings, not its mask");
}

/// The user's workspace: Color, Properties, Adjustments and Actions share one tab group, left on
/// Color. Double-clicking an adjustment thumbnail brings the Properties tab to the front.
#[test]
fn double_clicking_an_adjustment_thumbnail_fronts_properties_in_a_shared_tab_group() {
    let mut s = session();
    let id = s.execute("layer.newAdjustmentLayer.curves", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h = harness(s, 1.0);
    for panel in ["properties", "adjustments", "actions"] {
        h.state_mut().run("window.panel.move", json!({"panel": panel, "anchor": "color"})).unwrap();
    }
    h.state_mut().run("window.panel.activate", json!({"panel": "color"})).unwrap();
    h.run_steps(4);
    let layout = h.state().ui.docking.clone().expect("a customized workspace");
    assert!(crate::panel_docking::panel_active(&layout, "color"), "Color is the group's front tab");
    let at = thumbnail(&h, id);
    double_click(&mut h, at);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(id)));
    let layout = app.ui.docking.as_ref().expect("still customized");
    assert!(crate::panel_docking::panel_active(layout, "properties"), "Properties came to the front");
    assert!(!crate::panel_docking::panel_active(layout, "color"));
    assert!(!app.ui.mask_target, "on the adjustment's settings, not its mask");
}
