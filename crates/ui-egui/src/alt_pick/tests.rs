use super::*;
use serde_json::json;

/// A 40×40 white document whose top-left 4×4 pixels are black; red foreground, the Brush.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40, "background": "white"})).unwrap();
    app.session
        .edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            let black = photocraft_raster::from_rgba(&bg.format(), [0.0, 0.0, 0.0, 1.0]);
            bg.fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &black);
            Ok(())
        })
        .unwrap();
    app.run("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    app.ui.tool = Tool::Brush;
    app
}

/// One frame at `t` with `mods` held and the pointer over document point `hover`.
fn frame_at(ctx: &egui::Context, app: &mut PhotocraftApp, t: f64, mods: Modifiers, hover: Option<[f64; 2]>) {
    let raw = egui::RawInput { time: Some(t), events: vec![egui::Event::ModifiersChanged(mods)], ..Default::default() };
    ctx.run_ui(raw, |ui| frame(app, ui.ctx(), Tool::Brush, hover, true)).textures_delta.clear();
}

fn fg(app: &PhotocraftApp) -> [f32; 3] {
    let c = app.session.tools.foreground;
    [c[0], c[1], c[2]]
}

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];
const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

#[test]
fn off_by_default_alt_waits_for_a_click() {
    let mut app = app();
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut app, 0.0, Modifiers::ALT, Some([1.0, 1.0]));
    assert_eq!(fg(&app), RED);
    assert!(eyedropper_shows(&app, 0.0), "the pipette shows at once, as before");
}

#[test]
fn pressing_alt_picks_at_once_and_the_pipette_waits_for_the_hold() {
    let mut app = app();
    app.ui.tool_options.eyedropper_alt_instant = true;
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut app, 1.0, Modifiers::ALT, Some([1.0, 1.0]));
    assert_eq!(fg(&app), BLACK, "picked on the press, no click");
    assert!(!eyedropper_shows(&app, 1.1), "a tap doesn't flash the pipette");
    assert!(eyedropper_shows(&app, 1.25), "held 250 ms (the default), Alt works as usual");
    // Held on and moved: the press picked once, nothing more without a click.
    frame_at(&ctx, &mut app, 1.1, Modifiers::ALT, Some([30.0, 30.0]));
    assert_eq!(fg(&app), BLACK);
    // Let go and press again over white.
    frame_at(&ctx, &mut app, 1.2, Modifiers::NONE, Some([30.0, 30.0]));
    assert!(eyedropper_shows(&app, 1.2));
    frame_at(&ctx, &mut app, 1.3, Modifiers::ALT, Some([30.0, 30.0]));
    assert_eq!(fg(&app), WHITE);
    // A hold of 0: the pipette at once too.
    app.ui.tool_options.eyedropper_alt_hold_ms = 0.0;
    assert!(eyedropper_shows(&app, 1.3));
}

#[test]
fn only_a_plain_alt_press_over_the_canvas_picks() {
    let mut app = app();
    app.ui.tool_options.eyedropper_alt_instant = true;
    let ctx = egui::Context::default();
    // Pressed off the canvas, then moved onto it still held: nothing.
    frame_at(&ctx, &mut app, 0.0, Modifiers::ALT, None);
    frame_at(&ctx, &mut app, 0.1, Modifiers::ALT, Some([1.0, 1.0]));
    assert_eq!(fg(&app), RED);
    frame_at(&ctx, &mut app, 0.2, Modifiers::NONE, None);
    // Ctrl+Alt is the brush resize, ⇧Alt another shortcut.
    for (i, mods) in [Modifiers { ctrl: true, alt: true, ..Modifiers::NONE }, Modifiers { shift: true, alt: true, ..Modifiers::NONE }].into_iter().enumerate() {
        frame_at(&ctx, &mut app, 1.0 + i as f64, mods, Some([1.0, 1.0]));
        frame_at(&ctx, &mut app, 1.5 + i as f64, Modifiers::NONE, None);
    }
    assert_eq!(fg(&app), RED);
    // A tool Alt doesn't turn into the Eyedropper.
    let raw = egui::RawInput { time: Some(5.0), events: vec![egui::Event::ModifiersChanged(Modifiers::ALT)], ..Default::default() };
    ctx.run_ui(raw, |ui| frame(&mut app, ui.ctx(), Tool::Move, Some([1.0, 1.0]), true)).textures_delta.clear();
    assert_eq!(fg(&app), RED);
    // Broken settings stay in range.
    app.ui.tool_options.eyedropper_alt_hold_ms = f32::NAN;
    assert!(eyedropper_shows(&app, 5.0));
}

/// A pick on Alt shows a brief ring of the colour at the pointer.
#[test]
fn a_pick_on_alt_flashes_at_the_pointer() {
    let mut app = app();
    app.ui.tool_options.eyedropper_alt_instant = true;
    let ctx = egui::Context::default();
    assert!(!flashing(&app));
    frame_at(&ctx, &mut app, 2.0, Modifiers::ALT, Some([1.0, 1.0]));
    assert!(flashing(&app), "the pick shows");
    frame_at(&ctx, &mut app, 2.1, Modifiers::NONE, Some([1.0, 1.0]));
    assert!(flashing(&app), "letting go of Alt doesn't cut it short");
}
