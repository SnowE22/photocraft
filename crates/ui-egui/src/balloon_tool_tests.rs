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
