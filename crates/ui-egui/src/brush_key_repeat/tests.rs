use super::*;
use egui::{Event, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

#[test]
fn the_pause_and_the_interval_set_when_each_step_comes() {
    let r = KeyRepeat::default();
    assert!((r.due(10.0, 10.0) - 10.5).abs() < 1e-9, "the first repeat waits half a second");
    assert!((r.due(10.0, 10.5) - 10.55).abs() < 1e-9, "then every 50 ms");
    let at_once = KeyRepeat { pause: false, interval_ms: 50.0 };
    assert!((at_once.due(10.0, 10.0) - 10.05).abs() < 1e-9, "no pause: the next interval");
    let slow = KeyRepeat { pause: false, interval_ms: 1000.0 };
    assert!((slow.due(10.0, 11.0) - 12.0).abs() < 1e-9, "one step a second");
    let slow_pause = KeyRepeat { pause: true, interval_ms: 1000.0 };
    assert!((slow_pause.due(10.0, 10.0) - 11.0).abs() < 1e-9, "an interval longer than the pause wins");
    // Out of range or broken settings stay within bounds.
    for ms in [f32::NAN, -5.0, 0.0] {
        let r = KeyRepeat { pause: false, interval_ms: ms };
        assert!(r.due(0.0, 0.0) >= f64::from(MIN_INTERVAL_MS) / 1000.0 - 1e-9, "{ms}");
    }
    assert!((KeyRepeat { pause: false, interval_ms: 1e9 }.due(0.0, 0.0) - 5.0).abs() < 1e-9);
    let held = Held { id: "tools.increaseBrushSize".into(), key: Key::CloseBracket, pressed: 0.0, last: 0.0, deferred: false };
    assert!(swallows(Some(&held), "tools.increaseBrushSize", true), "the held key's system repeat");
    assert!(!swallows(Some(&held), "tools.increaseBrushSize", false), "a fresh press");
    assert!(!swallows(Some(&held), "tools.decreaseBrushHardness", true), "another brush key");
    assert!(!swallows(None, "tools.increaseBrushSize", true), "nothing held: it runs");
    assert!(!swallows(Some(&held), "edit.undo", true));
}

/// One frame at time `t` (seconds) with `events`, running the repeat clock.
fn frame(ctx: &egui::Context, app: &mut PhotocraftApp, t: f64, events: Vec<Event>, press: bool) {
    let raw = egui::RawInput { time: Some(t), events, ..Default::default() };
    ctx.run_ui(raw, |ui| {
        tick(app, ui.ctx());
        if press {
            // What the shortcut dispatch does for the press itself.
            crate::shortcut_dispatch::dispatch(app, ui.ctx(), "tools.increaseBrushSize");
            pressed(app, ui.ctx(), "tools.increaseBrushSize", Key::CloseBracket, true);
        }
    })
    .textures_delta
    .clear();
}

fn bracket(pressed: bool) -> Vec<Event> {
    vec![Event::Key { key: Key::CloseBracket, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }]
}

/// The sizes after frames at `times` (seconds) while ] is held from 0 and let go at `release`.
fn hold(repeat: KeyRepeat, times: &[f64], release: f64) -> Vec<f32> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("tools.setBrush", json!({"brush": {"size": 100}})).unwrap();
    app.ui.tool_options.brush_key_repeat = repeat;
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 0.0, bracket(true), true);
    let mut sizes = vec![app.session.tools.brush.size];
    for &t in times {
        let events = if t == release { bracket(false) } else { Vec::new() };
        frame(&ctx, &mut app, t, events, false);
        sizes.push(app.session.tools.brush.size);
    }
    sizes
}

/// Holding ] grows the brush once at once, then at the settings' pace, and letting go stops it.
#[test]
fn holding_the_size_key_steps_at_the_chosen_pace() {
    // No pause, a step every 100 ms.
    let s = hold(KeyRepeat { pause: false, interval_ms: 100.0 }, &[0.05, 0.1, 0.15, 0.2, 0.3, 0.35, 0.4, 0.5], 0.35);
    assert_eq!(s, vec![125.0, 125.0, 156.0, 156.0, 195.0, 244.0, 244.0, 244.0, 244.0], "a step at 0.1, 0.2, 0.3, none after the release at 0.35");
    // The pause: nothing more until half a second.
    let s = hold(KeyRepeat { pause: true, interval_ms: 100.0 }, &[0.1, 0.3, 0.45, 0.5, 0.6], 99.0);
    assert_eq!(s, vec![125.0, 125.0, 125.0, 125.0, 156.0, 195.0]);
    // A second between steps.
    let s = hold(KeyRepeat { pause: false, interval_ms: 1000.0 }, &[0.5, 0.99, 1.0, 1.5, 2.0], 99.0);
    assert_eq!(s, vec![125.0, 125.0, 125.0, 156.0, 156.0, 195.0]);
}

/// The system's own repeat presses of ] change nothing: the clock above does the repeating.
#[test]
fn the_systems_key_repeat_is_ignored() {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 100}})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.state_mut().ui.tool_options.brush_key_repeat = KeyRepeat { pause: true, interval_ms: 5000.0 };
    h.run_steps(2);
    let key = |h: &mut Harness<'_, PhotocraftApp>, pressed: bool, repeat: bool| {
        h.event(Event::Key { key: Key::CloseBracket, physical_key: None, pressed, repeat, modifiers: Modifiers::NONE });
    };
    key(&mut h, true, false);
    h.step();
    assert_eq!(h.state().session.tools.brush.size, 125.0, "the press steps once");
    for _ in 0..5 {
        key(&mut h, true, true);
    }
    h.step();
    assert_eq!(h.state().session.tools.brush.size, 125.0, "repeats wait for the clock");
    key(&mut h, false, false);
    h.step();
    assert!(h.state().brush_key_held.is_none());
}

/// The gear sits right of the brush picker's Size field and changes the settings.
#[test]
fn the_settings_live_beside_the_size_field() {
    use egui_kittest::kittest::Queryable;
    let mut repeat = KeyRepeat::default();
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 500.0)).build_ui(|ui| {
        let mut b = photocraft_engine::paint::BrushSettings::default();
        let mut st = Default::default();
        crate::brush_picker::body(ui, &mut b, &[], None, &mut st, egui::vec2(360.0, 420.0), &mut repeat);
    });
    h.run();
    h.get_by_label("Size Key Repeat").click();
    h.run();
    assert!(h.query_by_label("Pause before repeating").is_some());
    assert!(h.query_by_label_contains("Change every").is_some());
}

/// User report: resizing with ] while starting to paint drew the cursor at a size the stroke
/// wasn't using, and the release re-rendered the stroke at it. The brush now stays as the stroke
/// began until it ends; a press made meanwhile takes its step afterwards, and a held key resumes.
#[test]
fn brush_keys_wait_for_the_stroke_being_painted() {
    use crate::canvas::{ToolEvent, tool_event};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 10, "hardness": 1.0}})).unwrap();
    app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
    app.ui.tool = crate::state::Tool::Brush;
    let ctx = egui::Context::default();
    let key = |ctx: &egui::Context, app: &mut PhotocraftApp, t: f64, pressed: bool| {
        let raw = egui::RawInput { time: Some(t), events: bracket(pressed), ..Default::default() };
        ctx.run_ui(raw, |ui| {
            tick(app, ui.ctx());
            crate::shortcut_dispatch::dispatch_pressed(app, ui.ctx(), crate::shortcut_dispatch::Focus::None, false);
        })
        .textures_delta
        .clear();
    };
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 50.0, pressure: 1.0 }, m);
    assert!(painting(&app), "the stroke is being painted");
    // A ] tap mid-stroke: the brush (and so the cursor and the stroke) stays at 10.
    key(&ctx, &mut app, 0.0, true);
    key(&ctx, &mut app, 0.02, false);
    assert_eq!(app.session.tools.brush.size, 10.0);
    tool_event(&mut app, ToolEvent::Move { x: 180.0, y: 50.0, pressure: 1.0 }, m);
    key(&ctx, &mut app, 0.1, false);
    assert_eq!(app.session.tools.brush.size, 10.0, "still painting");
    tool_event(&mut app, ToolEvent::Up { x: 180.0, y: 50.0 }, m);
    // The committed stroke is the 10 px one that was shown: about 10 px tall at its middle.
    let doc = app.session.active().unwrap().doc.clone();
    let px = photocraft_compose::flatten(&doc);
    let inked = (0..100).filter(|y| px.px[(y * 200 + 100) as usize][0] < 0.5).count();
    assert!((9..=11).contains(&inked), "the stroke is {inked} px wide, not the resized brush");
    // Then the tap takes its step.
    key(&ctx, &mut app, 0.2, false);
    assert!(app.session.tools.brush.size > 10.0, "the deferred press applies after the stroke");
    assert!(app.brush_key_held.is_none(), "and the released key stops there");

    // A key held through a stroke: no steps while painting, steps again after.
    let size = app.session.tools.brush.size;
    app.ui.tool_options.brush_key_repeat = KeyRepeat { pause: false, interval_ms: 50.0 };
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, m);
    key(&ctx, &mut app, 1.0, true);
    for t in [1.1, 1.2, 1.3] {
        let raw = egui::RawInput { time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
    }
    assert_eq!(app.session.tools.brush.size, size, "held through the stroke: unchanged");
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, m);
    for t in [1.4, 1.5] {
        let raw = egui::RawInput { time: Some(t), ..Default::default() };
        ctx.run_ui(raw, |ui| tick(&mut app, ui.ctx())).textures_delta.clear();
    }
    assert!(app.session.tools.brush.size > size, "still held after the stroke: it steps again");
}
