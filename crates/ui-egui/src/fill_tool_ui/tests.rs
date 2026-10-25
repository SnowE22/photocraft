use super::*;
use crate::canvas::{ToolEvent, tool_event};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;

/// A 40 × 40 document: a white Background, "Lines" (a box with a 3 px gap at the top, marked as
/// a reference layer) and an empty "Colour" layer, which is active; red foreground, the Fill tool.
fn inked() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 40, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Lines"})).unwrap();
    let lines = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("ink", |doc, _| {
            let surf = doc.layer_mut(lines).unwrap().surface_mut().unwrap();
            for r in [Rect::new(5, 5, 18, 6), Rect::new(21, 5, 35, 6), Rect::new(5, 34, 35, 35), Rect::new(5, 5, 6, 35), Rect::new(34, 5, 35, 35)] {
                surf.fill_rect(r, &[0.0, 0.0, 0.0, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.run("layer.setReference", json!({})).unwrap();
    app.run("layer.new.layer", json!({"name": "Colour"})).unwrap();
    let colour = app.session.active().unwrap().active_layer.unwrap();
    app.run("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0], "background": [1.0, 1.0, 1.0, 1.0]})).unwrap();
    app.ui.tool = Tool::Fill;
    let o = &mut app.ui.tool_options.fill_tool;
    for sub in [SubTool::Click, SubTool::Enclose, SubTool::Unfilled] {
        let o = o.of_mut(sub);
        o.reference = "reference".into();
        o.anti_alias = false;
    }
    (app, colour)
}

fn alpha(app: &PhotocraftApp, id: LayerId, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn drag(app: &mut PhotocraftApp, points: &[[f64; 2]]) {
    let none = egui::Modifiers::NONE;
    let [x, y] = points[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, none);
    for &[x, y] in &points[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, none);
    }
    let [x, y] = points[points.len() - 1];
    tool_event(app, ToolEvent::Up { x, y }, none);
}

#[test]
fn the_fill_tool_shares_the_paint_buckets_slot_and_key() {
    assert_eq!(Tool::from_name("Fill Tool"), Some(Tool::Fill));
    assert_eq!(Tool::Fill.key(), 'G');
    assert!(Tool::ALL.contains(&Tool::Fill));
    assert_ne!(crate::icons::tool_icon(Tool::Fill), crate::icons::tool_icon(Tool::PaintBucket));
    assert!(crate::icons::exists(crate::icons::tool_icon(Tool::Fill)));
}

#[test]
fn a_click_fills_from_the_reference_layer_and_holds_at_the_gap() {
    let (mut app, colour) = inked();
    drag(&mut app, &[[20.0, 20.0]]);
    assert_eq!(alpha(&app, colour, 20, 20), 1.0);
    assert_eq!(alpha(&app, colour, 5, 20), 1.0, "Area Scaling tucks it under the 1 px line");
    assert_eq!(alpha(&app, colour, 0, 0), 0.0, "Close Gap held");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Fill Tool"));
    // Close Gap off: it runs out of the box.
    app.run("edit.undo", json!({})).unwrap();
    app.ui.tool_options.fill_tool.click.close_gap = 0.0;
    drag(&mut app, &[[20.0, 20.0]]);
    assert_eq!(alpha(&app, colour, 0, 0), 1.0);
    // Without a reference layer to read, the click says so.
    app.run("edit.undo", json!({})).unwrap();
    app.run("layer.setReference", json!({"on": false})).unwrap();
    let lines = app.session.active().unwrap().doc.walk().iter().find(|(_, _, l)| l.name == "Lines").map(|(_, _, l)| l.id).unwrap();
    app.run("layer.setReference", json!({"layer": lines.0, "on": false})).unwrap();
    drag(&mut app, &[[20.0, 20.0]]);
    assert!(app.ui.status_error && app.ui.status.contains("reference layer"), "{}", app.ui.status);
}

#[test]
fn enclose_and_fill_fills_what_the_lasso_goes_round() {
    let (mut app, colour) = inked();
    app.ui.tool_options.fill_tool.sub_tool = SubTool::Enclose;
    drag(&mut app, &[[2.0, 2.0], [38.0, 2.0], [38.0, 38.0], [2.0, 38.0]]);
    assert_eq!(alpha(&app, colour, 20, 20), 1.0);
    assert_eq!(alpha(&app, colour, 1, 1), 0.0);
    // Half way round: nothing is enclosed, and the status bar says so.
    app.run("edit.undo", json!({})).unwrap();
    drag(&mut app, &[[2.0, 2.0], [20.0, 2.0], [20.0, 38.0], [2.0, 38.0]]);
    assert_eq!(alpha(&app, colour, 10, 20), 0.0);
    assert!(!app.ui.status_error && !app.ui.status.is_empty());
}

#[test]
fn paint_unfilled_area_fills_what_the_stroke_covers() {
    let (mut app, colour) = inked();
    let o = &mut app.ui.tool_options.fill_tool;
    o.sub_tool = SubTool::Unfilled;
    o.unfilled.size = 80.0;
    drag(&mut app, &[[10.0, 20.0], [30.0, 20.0]]);
    assert_eq!(alpha(&app, colour, 20, 20), 1.0);
}

#[test]
fn the_options_reach_the_command() {
    let (mut app, _) = inked();
    let o = &mut app.ui.tool_options.fill_tool.click;
    o.exclude_layer = true;
    o.exclude_locked = true;
    o.mode = BlendMode::Multiply;
    o.scaling_mode = "round".into();
    o.opacity = 40.0;
    let p = params(&app);
    assert_eq!(p["exclude"], json!(["layer", "locked"]));
    assert_eq!(p["scalingMode"], "round");
    assert_eq!(p["opacity"], 40.0);
    assert_eq!(photocraft_engine::commands::blend_from_str(p["mode"].as_str().unwrap()), Some(BlendMode::Multiply));
    // The tool's reset puts the Clip Studio defaults back.
    crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Fill);
    assert_eq!(app.ui.tool_options.fill_tool, FillToolOptions::default());
    // Saved options without the Fill tool's load with its defaults.
    let o: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(o.fill_tool.click.close_gap, GAP_LEVELS[1]);
}

#[test]
fn each_sub_tool_keeps_its_own_settings() {
    let (mut app, _) = inked();
    let o = &mut app.ui.tool_options.fill_tool;
    o.sub_tool = SubTool::Enclose;
    o.current_mut().tolerance = 70.0;
    o.current_mut().opacity = 25.0;
    assert_eq!(params(&app)["tolerance"], 70.0);
    assert_eq!(params(&app)["opacity"], 25.0);
    // The click and Paint Unfilled Area keep theirs.
    let o = &mut app.ui.tool_options.fill_tool;
    o.sub_tool = SubTool::Click;
    assert_eq!(params(&app)["tolerance"], 10.0);
    assert_eq!(params(&app)["opacity"], 100.0);
    app.ui.tool_options.fill_tool.sub_tool = SubTool::Unfilled;
    assert_eq!(params(&app)["tolerance"], 10.0);
    // The number keys set the opacity of the sub tool in use.
    crate::opacity_keys::press(&mut app, 5, false, 0.0);
    assert_eq!(app.ui.tool_options.fill_tool.unfilled.opacity, 50.0);
    assert_eq!(app.ui.tool_options.fill_tool.click.opacity, 100.0);
    // Settings saved before the split (one flat set) load as the click's.
    let old: FillToolOptions = serde_json::from_value(json!({"tolerance": 33.0, "subTool": "enclose"})).unwrap();
    assert_eq!((old.click.tolerance, old.enclose.tolerance, old.sub_tool), (33.0, 10.0, SubTool::Enclose));
    let back: FillToolOptions = serde_json::from_value(serde_json::to_value(&app.ui.tool_options.fill_tool).unwrap()).unwrap();
    assert_eq!(back, app.ui.tool_options.fill_tool);
}

#[test]
fn undo_widens_close_gap_until_the_fill_holds_and_another_tool_puts_it_back() {
    let (mut app, colour) = inked();
    let o = &mut app.ui.tool_options.fill_tool;
    o.undo_widens_gap = true;
    o.click.close_gap = 0.0;
    o.click.area_scaling = 0.0;
    // It leaks through the 3 px gap; each Undo closes bigger gaps for the next click.
    drag(&mut app, &[[20.0, 20.0]]);
    assert_eq!(alpha(&app, colour, 0, 0), 1.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 2.0);
    drag(&mut app, &[[20.0, 20.0]]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 4.0);
    drag(&mut app, &[[20.0, 20.0]]);
    assert_eq!(alpha(&app, colour, 20, 20), 1.0);
    assert_eq!(alpha(&app, colour, 0, 0), 0.0, "the widened gap holds");
    // Kept: the next click (another area) is back at the user's own Close Gap.
    drag(&mut app, &[[2.0, 2.0]]);
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 0.0);
    // Undo, then another tool: back too.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 2.0);
    app.ui.tool = Tool::Brush;
    tick(&mut app);
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 0.0);
    // Off, an Undo leaves Close Gap alone; so does undoing something else.
    app.ui.tool = Tool::Fill;
    app.ui.tool_options.fill_tool.undo_widens_gap = false;
    drag(&mut app, &[[20.0, 20.0]]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 0.0);
    app.ui.tool_options.fill_tool.undo_widens_gap = true;
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ui.tool_options.fill_tool.click.close_gap, 0.0);
}

#[test]
fn the_options_bar_and_the_layer_menu_offer_the_fill_tool() {
    use egui_kittest::kittest::Queryable;
    let (app, _) = inked();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1800.0, 800.0)).with_max_steps(32).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    for label in ["Close Gap", "Area Scaling", "Tolerance", "Contiguous", "Refer"] {
        assert!(h.query_by_label_contains(label).is_some(), "missing {label}");
    }
    let st = h.state().session.active().unwrap();
    let lines = st.doc.walk().iter().find(|(_, _, l)| l.name == "Lines").map(|(_, _, l)| (*l).clone()).unwrap();
    let colour = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let item = |l: &photocraft_doc::Layer| crate::layer_menu_ui::entries(l, false, false).into_iter().flatten().find(|(_, id)| *id == "layer.setReference").map(|(label, _)| label);
    assert_eq!(item(&lines), Some("Clear Reference Layer"));
    assert_eq!(item(&colour), Some("Set as Reference Layer"));
    assert!(crate::layer_menu_ui::entries(&colour, true, false).into_iter().flatten().all(|(_, id)| id != "layer.setReference"));
}

/// The click fills on the press (before the release), and dragging on fills each further area
/// like the first that the pointer reaches, as one undo step.
#[test]
fn a_press_fills_at_once_and_a_drag_fills_on() {
    let (mut app, colour) = inked();
    let none = egui::Modifiers::NONE;
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, none);
    assert_eq!(alpha(&app, colour, 20, 20), 1.0, "filled on the press");
    assert_eq!(alpha(&app, colour, 0, 0), 0.0, "the gap held");
    // Out across the line to the outside: filled too, the line itself not.
    for x in [16.0, 10.0, 5.0, 2.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 20.0, pressure: 1.0 }, none);
    }
    tool_event(&mut app, ToolEvent::Up { x: 2.0, y: 20.0 }, none);
    assert_eq!(alpha(&app, colour, 0, 0), 1.0, "the drag reached the outside and filled it");
    assert!(app.fill_drag.is_none(), "the release ends the drag");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(alpha(&app, colour, 20, 20), 0.0);
    assert_eq!(alpha(&app, colour, 0, 0), 0.0);
}

/// Close Gap is five buttons: each picks its gap, and the chosen one again turns it off.
#[test]
fn close_gap_is_five_buttons() {
    use egui_kittest::kittest::Queryable;
    let mut gap = 0.0f32;
    for (button, want) in [(3, GAP_LEVELS[2]), (1, GAP_LEVELS[0]), (1, 0.0), (5, GAP_LEVELS[4])] {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(300.0, 40.0)).build_ui(|ui| {
            ui.horizontal(|ui| close_gap_buttons(ui, &mut gap, "fill"));
        });
        h.run();
        h.get_by_label(&format!("fill close gap {button}")).click();
        h.run();
        drop(h);
        assert_eq!(gap, want, "button {button}");
    }
}

/// The Magic Wand with Close Gap on stops at the gap, and dragging on adds the like areas it
/// reaches, as one step. Off, it is Photoshop's wand: it runs out through the gap.
#[test]
fn the_wand_closes_gaps_and_drags_on_like_the_fill() {
    let (mut app, _colour) = inked();
    app.ui.tool = Tool::MagicWand;
    app.ui.tool_options.anti_alias = false;
    app.ui.tool_options.sample_all.magic_wand = true;
    let none = egui::Modifiers::NONE;
    let sel = |app: &PhotocraftApp, x, y| app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0));
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, none);
    assert!(sel(&app, 20, 20) > 0.5 && sel(&app, 1, 1) > 0.5, "off: through the gap");
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool_options.wand_fill.close_gap = GAP_LEVELS[1];
    let steps = app.session.active().unwrap().history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, none);
    assert!(sel(&app, 20, 20) > 0.5 && sel(&app, 1, 1) < 0.5, "on: selected on the press, the gap held");
    for x in [12.0, 6.0, 2.0] {
        tool_event(&mut app, ToolEvent::Move { x, y: 20.0, pressure: 1.0 }, none);
    }
    tool_event(&mut app, ToolEvent::Up { x: 2.0, y: 20.0 }, none);
    assert!(sel(&app, 20, 20) > 0.5 && sel(&app, 1, 1) > 0.5, "the drag added the outside");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}
