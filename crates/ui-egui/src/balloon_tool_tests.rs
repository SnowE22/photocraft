//! The Balloon tool's drag gestures (`vector_ui::finish_balloon`): a tail unioned with the body
//! and procedural cloud / spiky balloons created from a drag rectangle.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use photocraft_doc::LayerContent;

fn app_with(w: u32, h: u32) -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.ui.tool = Tool::Balloon;
    app
}

fn active_layer_id(app: &PhotocraftApp) -> u64 {
    app.session.active().unwrap().active_layer.unwrap().0
}

fn shape_subpaths(app: &PhotocraftApp, layer: u64) -> usize {
    let st = app.session.active().unwrap();
    match &st.doc.layer(photocraft_doc::LayerId(layer)).unwrap().content {
        LayerContent::Shape(sh) => sh.path.subpaths.len(),
        _ => 0,
    }
}

#[test]
fn a_tail_drag_unions_the_tail_with_the_active_balloon() {
    let mut app = app_with(400, 400);
    let id = app.run("shape.create", json!({"kind": "ellipse", "rect": [100, 100, 200, 150]})).unwrap()["layer"].as_u64().unwrap();
    let before = shape_subpaths(&app, id);
    let history = app.session.active().unwrap().history.past_len();
    app.ui.tool_options.balloon_mode = "tail".into();
    app.ui.tool_options.balloon_preset = "arc".into();
    crate::vector_ui::finish_balloon(&mut app, [200.0, 245.0], [210.0, 360.0]);
    assert_eq!(app.ui.status_error, false, "status: {}", app.ui.status);
    assert_eq!(shape_subpaths(&app, id), before, "the tail joins the balloon contour");
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1, "one undoable step");
}

#[test]
fn cloud_and_spiky_drags_create_balloon_layers() {
    for (mode, knot_floor) in [("cloud", 18usize), ("spiky", 12usize)] {
        let mut app = app_with(400, 300);
        let layers = app.session.active().unwrap().doc.layers.len();
        app.ui.tool_options.balloon_mode = mode.into();
        crate::vector_ui::finish_balloon(&mut app, [40.0, 40.0], [300.0, 240.0]);
        assert_eq!(app.ui.status_error, false, "{mode} status: {}", app.ui.status);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers.len(), layers + 1, "{mode} adds one shape layer");
        let id = active_layer_id(&app);
        let LayerContent::Shape(sh) = &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content else {
            panic!("{mode} did not create a shape layer")
        };
        assert!(sh.path.subpaths[0].knots.len() >= knot_floor, "{mode}: {} knots", sh.path.subpaths[0].knots.len());
    }
}

#[test]
fn a_click_is_not_a_balloon_and_a_missing_shape_layer_is_reported() {
    let mut app = app_with(200, 200);
    // A click (no drag) creates nothing.
    app.ui.tool_options.balloon_mode = "cloud".into();
    crate::vector_ui::finish_balloon(&mut app, [50.0, 50.0], [51.0, 51.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "only the background");
    // A tail drag without any shape layer under it explains itself instead of failing silently.
    app.ui.tool_options.balloon_mode = "tail".into();
    crate::vector_ui::finish_balloon(&mut app, [50.0, 50.0], [150.0, 150.0]);
    assert!(app.ui.status_error, "the tool reports the missing balloon body");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
}

fn pen_app() -> PhotocraftApp {
    let mut app = app_with(400, 300);
    app.ui.tool_options.balloon_mode = "pen".into();
    app
}

fn active_shape(app: &PhotocraftApp) -> photocraft_doc::ShapeLayer {
    let id = active_layer_id(app);
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        other => panic!("not a shape: {}", other.kind_name()),
    }
}

#[test]
fn pen_clicks_become_one_smoothed_tapered_stroke() {
    let mut app = pen_app();
    let layers = app.session.active().unwrap().doc.layers.len();
    for (x, y) in [(40.0, 200.0), (120.0, 90.0), (220.0, 140.0), (340.0, 60.0)] {
        crate::vector_ui::balloon_pen_down(&mut app, x, y, egui::Modifiers::NONE);
    }
    assert_eq!(app.ui.balloon_pen.as_ref().map(|p| p.points.len()), Some(4));
    crate::vector_ui::balloon_pen_commit(&mut app);
    assert!(app.ui.balloon_pen.is_none(), "committing clears the gesture");
    assert_eq!(app.ui.status_error, false, "status: {}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    let shape = active_shape(&app);
    assert!(shape.fill.is_some(), "tapered ink is a filled outline");
    assert_eq!(shape.path.subpaths.len(), 1);
}

#[test]
fn pen_nodes_move_toggle_corner_and_change_pressure() {
    let mut app = pen_app();
    crate::vector_ui::balloon_pen_down(&mut app, 50.0, 50.0, egui::Modifiers::NONE);
    crate::vector_ui::balloon_pen_down(&mut app, 150.0, 50.0, egui::Modifiers::NONE);
    // Dragging a node without a modifier lowers its pressure.
    crate::vector_ui::balloon_pen_down(&mut app, 150.0, 50.0, egui::Modifiers::NONE);
    crate::vector_ui::balloon_pen_move(&mut app, 150.0, 200.0);
    crate::vector_ui::balloon_pen_up(&mut app);
    let pen = app.ui.balloon_pen.as_ref().unwrap();
    assert!(pen.points[1][2] < 0.5, "pressure {}", pen.points[1][2]);
    assert_eq!((pen.points[1][0], pen.points[1][1]), (150.0, 50.0), "a weight drag does not move the node");
    // ⌥-click toggles a corner.
    crate::vector_ui::balloon_pen_down(&mut app, 150.0, 50.0, egui::Modifiers { alt: true, ..Default::default() });
    assert_eq!(app.ui.balloon_pen.as_ref().unwrap().points[1][3], 1.0);
    // ⌘/Ctrl-drag moves the node instead.
    crate::vector_ui::balloon_pen_down(&mut app, 150.0, 50.0, egui::Modifiers { command: true, ..Default::default() });
    crate::vector_ui::balloon_pen_move(&mut app, 180.0, 70.0);
    crate::vector_ui::balloon_pen_up(&mut app);
    let moved = app.ui.balloon_pen.as_ref().unwrap().points[1];
    assert_eq!((moved[0], moved[1]), (180.0, 70.0));
    // ⌫ removes the last node.
    crate::vector_ui::balloon_pen_undo_last(&mut app);
    assert_eq!(app.ui.balloon_pen.as_ref().unwrap().points.len(), 1);
    crate::vector_ui::balloon_pen_undo_last(&mut app);
    assert!(app.ui.balloon_pen.is_none(), "the empty gesture is gone");
}

#[test]
fn a_one_node_pen_stroke_is_not_committed() {
    let mut app = pen_app();
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::vector_ui::balloon_pen_down(&mut app, 50.0, 50.0, egui::Modifiers::NONE);
    crate::vector_ui::balloon_pen_commit(&mut app);
    assert!(app.ui.status_error, "the tool asks for a second node");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
}