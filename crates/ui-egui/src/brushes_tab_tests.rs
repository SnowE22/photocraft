//! The preset grid (the Brush Preset picker's and the Brushes tab's) is centred: every row has the
//! same left margin, and the spare width is split between the two sides.

use egui::{Rect, accesskit::Role, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

#[test]
fn grid_columns_centre_whole_columns_past_the_indent() {
    // 6 × 44 + 5 × 3 = 279 of the 290 past a 4 pt indent: 5 spare on each side (rounded down).
    assert_eq!(grid_columns(294.0, 44.0, 4.0, GRID_GAP), (9.0, 6));
    // An exact fit leaves only the indent.
    assert_eq!(grid_columns(4.0 + 279.0, 44.0, 4.0, GRID_GAP), (4.0, 6));
    // One point short of a column: one fewer, centred.
    assert_eq!(grid_columns(4.0 + 278.0, 44.0, 4.0, GRID_GAP), (4.0 + 23.0, 5));
    // The picker's cards centre with their own tighter gap: 6 × 44 + 5 × 1 = 269 of the 290.
    assert_eq!(grid_columns(294.0, 44.0, 4.0, CARD_GAP), (4.0 + 10.0, 6));
    for (w, cell, indent) in [(294.0, 44.0, 4.0), (360.0, 52.0, 20.0), (500.0, 52.0, 20.0)] {
        let (left, cols) = grid_columns(w, cell, indent, GRID_GAP);
        let right = w - left - (cols as f32 * cell + (cols - 1) as f32 * GRID_GAP);
        assert!(((left - indent) - right).abs() <= 1.0, "{w}: left {left}, right {right}");
    }
}

#[test]
fn grid_columns_never_panic_or_return_no_columns() {
    for w in [0.0, -50.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30] {
        for cell in [0.0, -1.0, f32::NAN, f32::INFINITY, 44.0, 1e-30] {
            let (left, cols) = grid_columns(w, cell, 4.0, GRID_GAP);
            assert!((1..=256).contains(&cols), "{w} {cell}: {cols}");
            assert!(left.is_finite() && left >= 0.0, "{w} {cell}: {left}");
        }
    }
    // Narrower than one cell: one column at the indent.
    assert_eq!(grid_columns(30.0, 44.0, 4.0, GRID_GAP), (4.0, 1));
}

/// The picker's cards: one full-width 2 × 2 card per preset (tip and stroke on top, the name
/// across the bottom), and a hidden part shrinks the card — with only the tip and the name on,
/// the name takes the stroke's cell and the card takes its own height ([`CARD_TIP_NAME_H`]) and
/// width ([`CARD_TIP_NAME_W`]) snapped to the list's left edge; with only the tip on, the card
/// takes the tip-only cell's own height, with the grid's whole columns centred.
#[test]
fn brush_picker_cards_follow_the_part_boxes() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.run_steps(8);
    let presets = h.state().session.tools.presets.clone();
    let cards = |h: &Harness<'_, PhotocraftApp>| -> Vec<Rect> {
        h.query_all_by_role(Role::Button)
            .filter(|n| n.accesskit_node().label().is_some_and(|l| presets.iter().any(|p| p.name == l)))
            .map(|n| n.rect())
            .collect()
    };
    let full = CARD_MAX_H;
    let one_row = CARD_MAX_H - CARD_NAME_H;
    let all_on = cards(&h);
    assert!(all_on.len() >= 2, "{} cards", all_on.len());
    assert!(all_on.iter().all(|r| (r.height() - full).abs() < 0.5 && r.width() > 250.0), "{all_on:?}");
    h.state_mut().ui.brush_picker_list.show_stroke = false;
    h.run_steps(2);
    let tip_name = cards(&h);
    assert!(
        tip_name.iter().all(|r| (r.height() - CARD_TIP_NAME_H).abs() < 0.5 && (r.width() - CARD_TIP_NAME_W).abs() < 0.5),
        "tip + name: the name in the stroke's cell, its own cell size"
    );
    assert!(tip_name.iter().all(|r| (r.left() - all_on[0].left()).abs() < 0.5), "snapped to the left edge: {tip_name:?}");
    h.state_mut().ui.brush_picker_list.show_name = false;
    h.run_steps(2);
    let tip_only = cards(&h);
    assert!(tip_only.iter().all(|r| (r.height() - CARD_TIP_ONLY_H).abs() < 0.5), "tip only: its own cell");
    assert!(tip_only[0].left() > all_on[0].left() + 5.0, "the tip-only grid's columns stay centred: {tip_only:?}");
    h.state_mut().ui.brush_picker_list.show_stroke = true;
    h.run_steps(2);
    assert!(cards(&h).iter().all(|r| (r.height() - one_row).abs() < 0.5), "tip + stroke: one row");
}

/// The footer slider's widths take the raw scale; heights, padding and text stop at
/// [`CARD_MIN_H_SCALE`], so at the slider's 0.15 the cards stand at 42 pt ([`CARD_MAX_H`] at that
/// floor) with the widths a fraction of that.
#[test]
fn brush_picker_cards_floor_their_height_but_not_their_width() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.state_mut().ui.brush_picker_list.scale = 0.15;
    h.run_steps(8);
    let presets = h.state().session.tools.presets.clone();
    let cards: Vec<Rect> = h
        .query_all_by_role(Role::Button)
        .filter(|n| n.accesskit_node().label().is_some_and(|l| presets.iter().any(|p| p.name == l)))
        .map(|n| n.rect())
        .collect();
    assert!(cards.len() >= 2, "{} cards", cards.len());
    for r in &cards {
        assert!((r.height() - CARD_MAX_H * CARD_MIN_H_SCALE).abs() < 0.5, "the height floors at the min scale: {r:?}");
        // A column's share can run up to 1/col past the raw max width ([`card_columns`]); the
        // floored scale's widths would be four times this (0.60 / 0.15).
        assert!(r.width() <= CARD_MAX_W * 0.15 * 1.5, "the width keeps the raw scale, not the floored one: {r:?}");
    }
}

/// The gear's menu goes the moment a slider or the resize grip is grabbed. A menu closes on a
/// click, and a drag never becomes one, so the menu stayed up — and the picker's own interaction
/// pulled the picker over it. Each press is held down (no release yet, nothing classified) while
/// the menu is checked, so only the grab itself can have closed it.
#[test]
fn brush_picker_menu_closes_when_a_slider_or_the_grip_is_grabbed() {
    fn click_gear(h: &Harness<'_, PhotocraftApp>) {
        h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().as_deref() == Some("Brush Preset Options")).expect("the gear button").click();
    }
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.run_steps(8);
    let picker = h.ctx.memory(|m| m.area_rect(egui::Id::new("canvas-brush-picker"))).expect("the picker is shown");
    assert!(!egui::Popup::is_any_open(&h.ctx), "nothing open to start with");
    click_gear(&h);
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx), "the gear opens its menu");
    // The Size slider, under the row that holds the "Size" label: the row stands as tall as its
    // 24 pt value field with the label centred in it, so the slider's middle sits 12 pt below the
    // label's, plus the style's spacing and half the slider's 18 pt — 27 pt at the 6 pt spacing.
    let size_label = h.query_all_by_label(crate::i18n::t("Size")).find(|n| picker.contains(n.rect().center())).expect("the picker's Size label").rect();
    let slider = egui::pos2(size_label.left() + 100.0, size_label.center().y + 27.0);
    h.drag_at(slider);
    h.run_steps(2);
    assert!(!egui::Popup::is_any_open(&h.ctx), "grabbing the Size slider dismisses the menu");
    h.drop_at(slider);
    h.run_steps(2);
    click_gear(&h);
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx), "the gear opens its menu again");
    // The grip is the frame's bottom-right 16 × 16, so its middle is 8 pt off the picker's corner.
    let grip = picker.right_bottom() - vec2(8.0, 8.0);
    h.drag_at(grip);
    h.run_steps(2);
    assert!(!egui::Popup::is_any_open(&h.ctx), "grabbing the resize grip dismisses the menu");
    h.drop_at(grip);
    h.run_steps(2);
}

/// The upper controls (Size, Hardness) share a block at most [`crate::brush_picker::HEAD_MAX_W`]
/// wide, flush with the picker's left edge: on a picker dragged wider, a click on the Size slider
/// just inside the block jumps the size, and the same click just past the block lands on nothing.
/// A click on a slider sets it to the spot clicked, so a size that doesn't move is a click that
/// missed.
#[test]
fn brush_picker_upper_controls_stop_at_their_max_width() {
    fn click(h: &mut Harness<'_, PhotocraftApp>, pos: egui::Pos2) {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(2);
        }
    }
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    });
    h.state_mut().run("file.new", serde_json::json!({"width": 400, "height": 300})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    // Dragged wide: 500 pt of content, well past the block.
    h.state_mut().ui.brush_picker = Some([300.0, 120.0]);
    h.state_mut().ui.brush_picker_size = Some([500.0, 420.0]);
    h.run_steps(8);
    let picker = h.ctx.memory(|m| m.area_rect(egui::Id::new("canvas-brush-picker"))).expect("the picker is shown");
    let size_label = h.query_all_by_label(crate::i18n::t("Size")).find(|n| picker.contains(n.rect().center())).expect("the picker's Size label").rect();
    // The Size slider's middle: 12 pt below the label's (the row stands as tall as its 24 pt value
    // field, the label centred in it), the style's spacing, then half the slider's 18 pt — 27 pt
    // at the 6 pt spacing.
    let y = size_label.center().y + 27.0;
    let before = h.state().session.tools.brush.size;
    click(&mut h, egui::pos2(size_label.left() + crate::brush_picker::HEAD_MAX_W - 10.0, y));
    let inside = h.state().session.tools.brush.size;
    assert_ne!(inside, before, "the slider takes a click just inside the block");
    click(&mut h, egui::pos2(size_label.left() + crate::brush_picker::HEAD_MAX_W + 10.0, y));
    assert_eq!(h.state().session.tools.brush.size, inside, "just past the block the click lands on nothing");
}

/// Photoshop's tiles: as many of the slider's width as fit, stretched to fill the row; never no
/// column or a nonsense size.
#[test]
fn tile_columns_fill_the_row() {
    // 3 × 96 + 2 × 2 = 292 fits in 300: three tiles, stretched to 98.
    assert_eq!(tile_columns(300.0, 96.0), (98.0, 3));
    // Narrower than one tile: one, as wide as the room.
    assert_eq!(tile_columns(80.0, 96.0), (80.0, 1));
    // The slider's smallest tiles pack many to a row.
    assert_eq!(tile_columns(300.0, 40.0).1, 7);
    for room in [0.0, -50.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30] {
        for want in [0.0, -1.0, f32::NAN, f32::INFINITY, 96.0, 1e-30] {
            let (w, cols) = tile_columns(room, want);
            assert!((1..=256).contains(&cols), "{room} {want}: {cols}");
            assert!(w.is_finite() && w >= 1.0, "{room} {want}: {w}");
        }
    }
}

#[test]
fn tile_heights_follow_the_parts_and_the_slider() {
    let stroke_name = (true, true, false);
    // A stroke row with the name under it: 3 + 13 + 14 + 3 at the default width (Photoshop's
    // 33 pt tiles).
    assert_eq!(tile_height(stroke_name, TILE_DEFAULT, 98.0), 33.0);
    assert!(tile_height(stroke_name, 300.0, 300.0) > tile_height(stroke_name, TILE_DEFAULT, 98.0), "bigger tiles are taller");
    // Tips alone are square; the name alone is a short row; nothing on reads as stroke and name.
    assert_eq!(tile_height((false, false, true), TILE_DEFAULT, 98.0), 98.0);
    assert_eq!(tile_height((true, false, false), TILE_DEFAULT, 98.0), 22.0);
    assert_eq!(tile_height((false, false, false), TILE_DEFAULT, 98.0), 33.0);
    for want in [f32::NAN, f32::INFINITY, -3.0] {
        assert!(tile_height(stroke_name, want, 98.0).is_finite());
    }
}

/// Shift- and Ctrl-clicks build the selection as in Photoshop.
#[test]
fn clicks_build_the_selection() {
    let it = |n: &str| BrushItem::Preset(n.into());
    let visible = [it("a"), BrushItem::Folder(vec!["G".into()]), it("b"), it("c")];
    let mut st = BrushesPanelState::default();
    click(&mut st, it("a"), false, false, &visible);
    assert_eq!(st.selected, [it("a")]);
    click(&mut st, it("b"), true, false, &visible);
    assert_eq!(st.selected, [it("a"), BrushItem::Folder(vec!["G".into()]), it("b")]);
    click(&mut st, BrushItem::Folder(vec!["G".into()]), false, true, &visible);
    assert_eq!(st.selected, [it("a"), it("b")]);
    // Ctrl moved the anchor to G: Ctrl+Shift adds G to c.
    click(&mut st, it("c"), true, true, &visible);
    assert_eq!(st.selected, [it("a"), it("b"), BrushItem::Folder(vec!["G".into()]), it("c")]);
    click(&mut st, it("c"), false, false, &visible);
    assert_eq!(st.selected, [it("c")]);
    // An item not drawn (a collapsed group's) selects alone.
    click(&mut st, it("zzz"), true, false, &visible);
    assert_eq!(st.selected, [it("zzz")]);
}

/// The panel's tiles share their rows and fill them: at the default size several tiles sit in a
/// row, all as wide, ending at the list's right edge.
#[test]
fn the_brushes_panel_tiles_fill_their_rows() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::Window::new("Brushes").fixed_size(vec2(320.0, 700.0)).show(&ctx, |ui| show(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(6);
    let presets = h.state().session.tools.presets.clone();
    let tiles: Vec<Rect> = h
        .query_all_by_role(Role::Button)
        .filter(|n| n.accesskit_node().label().is_some_and(|l| presets.iter().any(|p| p.name == l)))
        .map(|n| n.rect())
        .collect();
    let first_row: Vec<&Rect> = tiles.iter().filter(|r| (r.top() - tiles[0].top()).abs() < 0.5).collect();
    assert!(first_row.len() >= 2, "{} tiles in the first row", first_row.len());
    assert!(first_row.iter().all(|r| (r.width() - first_row[0].width()).abs() < 0.5), "the tiles share the row evenly");
    let right = first_row.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
    let rows_right = tiles.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
    assert!((right - rows_right).abs() < 0.5, "full rows end at the same edge");
}

/// #4454: a card's size number sat under the tip thumbnail of any medium or large brush (the
/// tip took up to 44.8 pt of a 42 pt cell, the number anchored to the cell's bottom), so the
/// "30" of Hard Round and the "45" of Soft Round couldn't be read. At every footer scale that
/// shows numbers, in each card kind, the tip now stays above the number, and bigger brushes still
/// get bigger tips.
#[test]
fn card_tips_never_cover_their_size_number() {
    let number_h = std::rc::Rc::new(std::cell::Cell::new(0.0_f32));
    let seen = number_h.clone();
    let mut h = Harness::new_ui(move |ui| {
        let g = ui.painter().layout_no_wrap("45".into(), egui::FontId::proportional(CARD_NUM_FONT), Color32::WHITE);
        seen.set(g.size().y);
    });
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run();
    let number_h = number_h.get();
    assert!(number_h > 8.0, "the number has a real height: {number_h}");
    let mut scale = CARD_COMPACT_SCALE + 0.01;
    while scale <= *crate::brush_picker::SCALE_RANGE.end() + 1e-3 {
        let scale_h = scale.max(CARD_MIN_H_SCALE);
        // The tip cell of a full card (name row under it), a tip-and-name card and a tip-only one,
        // as [`preset_card`] sizes them in a wide column.
        let cells = [
            vec2(CARD_TIP_W * scale, (CARD_MAX_H - CARD_PAD * 2.0 - CARD_NAME_H) * scale_h),
            vec2(CARD_TIP_W * scale, (CARD_TIP_NAME_H - CARD_PAD * 2.0) * scale_h),
            vec2((CARD_TIP_ONLY_W - 16.0) * scale, (CARD_TIP_ONLY_H - CARD_PAD * 2.0) * scale_h),
        ];
        for size in cells {
            let cell = Rect::from_min_size(egui::pos2(10.0, 20.0), size);
            let number_top = cell.bottom() - 1.0 - number_h;
            let mut last = 0.0;
            for brush in [1.0, 5.0, 13.0, 30.0, 45.0, 100.0, 2500.0] {
                let tip = card_tip_rect(cell, Some(number_h), brush, scale);
                assert!(tip.bottom() <= number_top + 1e-3, "scale {scale}, cell {size:?}, {brush} px: tip {tip:?} reaches the number at {number_top}");
                assert!(cell.expand(1e-3).contains_rect(tip), "scale {scale}, {brush} px: tip {tip:?} leaves its cell {cell:?}");
                assert!(tip.width() >= last, "scale {scale}, {brush} px: a bigger brush gets a smaller tip");
                last = tip.width();
            }
        }
        scale += 0.05;
    }
    // The tip-only grid's cells are tall enough for the number: a big brush's tip still spans
    // the cell's width.
    let cell = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(CARD_TIP_ONLY_W - 16.0, CARD_TIP_ONLY_H - CARD_PAD * 2.0));
    assert!((card_tip_rect(cell, Some(number_h), 45.0, 1.0).width() - cell.width()).abs() < 1e-3);
    // Compact cards: no number, the tip fills the cell; nonsense never makes a non-finite rect.
    assert_eq!(card_tip_rect(cell, None, 45.0, 0.5).width(), cell.width() - 2.0);
    for (n, size, scale) in [(f32::NAN, 30.0, 1.0), (500.0, f32::NAN, 1.0), (12.0, 30.0, f32::INFINITY), (12.0, -5.0, 0.0)] {
        let r = card_tip_rect(cell, Some(n), size, scale);
        assert!(r.min.is_finite() && r.max.is_finite(), "{n} {size} {scale}: {r:?}");
    }
}
