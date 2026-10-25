//! ⇧+ and ⇧− step through the blend modes, as in Photoshop (#3102): the Brush's or Pencil's mode
//! while one of them is active, the Gradient's with the Gradient tool, and otherwise (Move, the
//! selection tools…) the active layer's. The list wraps at either end. ⇧⌥<letter> picks a mode
//! outright (⇧⌥M Multiply, ⇧⌥S Screen…, [`mode_for_key`]) for the same targets, though with several
//! layers selected it sets all of them. Each press is one call of
//! an engine command (`tools.setBrush`, `layer.setProps`), so it records and undoes like a pick
//! from the dropdown.

use egui::{Event, Key};
use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The mode `step` places after (or before, when negative) `current` in `modes`, wrapping around.
/// A mode that isn't listed steps from the top of the list.
pub fn stepped(modes: &[BlendMode], current: BlendMode, step: i32) -> Option<BlendMode> {
    let n = i32::try_from(modes.len()).ok().filter(|n| *n > 0)?;
    let at = modes.iter().position(|m| *m == current).and_then(|i| i32::try_from(i).ok());
    let next = match at {
        Some(i) => (i + step).rem_euclid(n),
        None if step > 0 => 0,
        None => n - 1,
    };
    modes.get(usize::try_from(next).ok()?).copied()
}

/// Photoshop's ⇧⌥<letter> blend-mode keys: the mode a letter picks. Pass Through (P) is the only
/// mode that is not a layer mode; it applies to groups alone. Darker Color, Lighter Color, Subtract
/// and Divide have no key in Photoshop either.
pub fn mode_for_key(k: Key) -> Option<BlendMode> {
    Some(match k {
        Key::N => BlendMode::Normal,
        Key::I => BlendMode::Dissolve,
        Key::P => BlendMode::PassThrough,
        Key::K => BlendMode::Darken,
        Key::M => BlendMode::Multiply,
        Key::B => BlendMode::ColorBurn,
        Key::A => BlendMode::LinearBurn,
        Key::G => BlendMode::Lighten,
        Key::S => BlendMode::Screen,
        Key::D => BlendMode::ColorDodge,
        Key::W => BlendMode::LinearDodge,
        Key::O => BlendMode::Overlay,
        Key::F => BlendMode::SoftLight,
        Key::H => BlendMode::HardLight,
        Key::V => BlendMode::VividLight,
        Key::J => BlendMode::LinearLight,
        Key::Z => BlendMode::PinLight,
        Key::L => BlendMode::HardMix,
        Key::E => BlendMode::Difference,
        Key::X => BlendMode::Exclusion,
        Key::U => BlendMode::Hue,
        Key::T => BlendMode::Saturation,
        Key::C => BlendMode::Color,
        Key::Y => BlendMode::Luminosity,
        _ => return None,
    })
}

/// One blend-mode key press: step through the list, or pick a mode outright.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Step(i32),
    Mode(BlendMode),
}

/// What `e` asks of the blend mode, if anything. The single test both of "is there work this frame"
/// and of "which events does this module take", so the two cannot disagree.
///
/// - ⇧+ / ⇧− (no ⌥): [`Press::Step`]. Key repeats keep stepping.
/// - ⇧⌥<letter>: [`Press::Mode`], read from the physical key first so a non-Latin layout still
///   reaches the Latin letter's place. A held key does not repeat: the mode is already set.
fn press_of(e: &Event) -> Option<Press> {
    let Event::Key { key, physical_key, pressed: true, repeat, modifiers, .. } = e else { return None };
    if modifiers.command || modifiers.ctrl || !modifiers.shift {
        return None;
    }
    if !modifiers.alt {
        return match key {
            Key::Plus | Key::Equals => Some(Press::Step(1)),
            Key::Minus => Some(Press::Step(-1)),
            _ => None,
        };
    }
    if *repeat {
        return None;
    }
    physical_key.and_then(mode_for_key).or_else(|| mode_for_key(*key)).map(Press::Mode)
}

/// Consumes this frame's ⇧+ / ⇧− presses (⇧= on a US layout is `+`; on a German one
/// `layout_keys::apply` has already turned ⇧+ and ⇧- into `+` and `-`) and ⇧⌥<letter> presses, and
/// applies them in the order they came.
///
/// Most frames carry no such key: a read-only scan of the frame's events finds that out without
/// taking the input lock for writing, and nothing is collected or allocated until a key matches.
pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.events.iter().any(|e| press_of(e).is_some())) {
        return;
    }
    let mut presses: Vec<Press> = Vec::new();
    ctx.input_mut(|i| {
        i.events.retain(|e| match press_of(e) {
            Some(p) => {
                presses.push(p);
                false
            }
            None => true,
        });
    });
    for p in presses {
        match p {
            Press::Step(step) => press(app, step),
            Press::Mode(mode) => set_mode(app, mode),
        }
    }
}

/// Sets the blend mode outright (⇧⌥<letter>), where [`press`] steps it: the Brush's or Pencil's with
/// one of them active, the Gradient's with the Gradient tool, and otherwise every selected layer's in
/// a single history step. Layers that already have the mode, the Background (it has none) and, for
/// Pass Through, anything but a group are left out; if none remain nothing happens.
///
/// The Paint Bucket has no blend mode of its own in the engine (`paint.bucket` takes an opacity
/// only), so its key does nothing, as do the other tools whose mode isn't the brush's.
pub fn set_mode(app: &mut PhotocraftApp, mode: BlendMode) {
    match app.ui.tool {
        Tool::Brush | Tool::Pencil => {
            if let Ok(mode) = serde_json::to_value(mode) {
                let _ = app.run("tools.setBrush", json!({ "brush": { "mode": mode } }));
            }
        }
        Tool::Gradient => {
            let before = app.ui.tool_options.clone();
            app.ui.tool_options.gradient_blend_mode = mode;
            // Live mode: a selected gradient fill layer follows, as with the dropdown.
            crate::gradient_ui::options_changed(app, &before);
        }
        t if t.is_brushlike() || t == Tool::MagicEraser || t == Tool::PaintBucket => {}
        _ => {
            let Some(st) = app.session.active() else { return };
            let layers: Vec<u64> = st
                .selected_layers()
                .into_iter()
                .filter_map(|id| st.doc.layer(id))
                .filter(|l| l.blend != mode && (mode != BlendMode::PassThrough || l.is_group()) && !crate::doc_props_ui::is_background(&st.doc, l))
                .map(|l| l.id.0)
                .collect();
            if !layers.is_empty() {
                let _ = app.run("layer.setProps", json!({ "layers": layers, "blend": mode.label() }));
            }
        }
    }
}

/// One ⇧+ (`step` 1) or ⇧− (`step` -1) press.
pub fn press(app: &mut PhotocraftApp, step: i32) {
    match app.ui.tool {
        Tool::Brush | Tool::Pencil => {
            let current = app.session.tools.brush.mode;
            if let Some(mode) = stepped(&BlendMode::LAYER_MODES, current, step)
                && let Ok(mode) = serde_json::to_value(mode)
            {
                let _ = app.run("tools.setBrush", json!({ "brush": { "mode": mode } }));
            }
        }
        Tool::Gradient => {
            let before = app.ui.tool_options.clone();
            if let Some(mode) = stepped(&BlendMode::LAYER_MODES, before.gradient_blend_mode, step) {
                app.ui.tool_options.gradient_blend_mode = mode;
                // Live mode: a selected gradient fill layer follows, as with the dropdown.
                crate::gradient_ui::options_changed(app, &before);
            }
        }
        // Their blend mode isn't the brush's (the Eraser has none): leave them alone.
        t if t.is_brushlike() || t == Tool::MagicEraser || t == Tool::PaintBucket => {}
        _ => {
            let Some(st) = app.session.active() else { return };
            let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return };
            // The locked Background has no blend mode.
            if crate::doc_props_ui::is_background(&st.doc, layer) {
                return;
            }
            let modes: Vec<BlendMode> = std::iter::once(BlendMode::PassThrough).filter(|_| layer.is_group()).chain(BlendMode::LAYER_MODES).collect();
            if let Some(mode) = stepped(&modes, layer.blend, step) {
                let id = layer.id.0;
                let _ = app.run("layer.setProps", json!({ "layer": id, "blend": mode.label() }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(tool: Tool) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = tool;
        app
    }

    fn layer_blend(app: &PhotocraftApp) -> BlendMode {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().blend
    }

    /// Feeds one key press with `modifiers` through the app's shortcut handling.
    fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: egui::Modifiers) {
        let raw = egui::RawInput {
            events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(app, ctx);
        ctx.end_pass().textures_delta.clear();
    }

    /// #3102: on a German layout ⇧+ types `*`, which egui reports as the physical `]`, and ⇧- types
    /// `_` as the physical `/`. With the Brush they step its mode, not its hardness; a US ⇧] (`}`)
    /// still raises the hardness.
    #[test]
    fn german_shift_plus_and_minus_step_the_mode_and_us_shift_bracket_keeps_the_hardness() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Brush);
        a.session.tools.brush.hardness = 0.5;
        let shift = egui::Modifiers::SHIFT;
        let typed = |a: &mut PhotocraftApp, key: Key, physical_key: Key, text: &str| {
            let press = egui::Event::Key { key, physical_key: Some(physical_key), pressed: true, repeat: false, modifiers: shift };
            let raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(shift), press, egui::Event::Text(text.into())], ..Default::default() };
            ctx.begin_pass(raw);
            crate::shortcuts::handle(a, &ctx);
            ctx.end_pass().textures_delta.clear();
        };
        typed(&mut a, Key::CloseBracket, Key::CloseBracket, "*");
        assert_eq!(a.session.tools.brush.mode, BlendMode::Dissolve);
        assert_eq!(a.session.tools.brush.hardness, 0.5, "not Increase Brush Hardness");
        typed(&mut a, Key::Slash, Key::Slash, "_");
        assert_eq!(a.session.tools.brush.mode, BlendMode::Normal);
        // US: ⇧] arrives as `}` from the physical `]`.
        typed(&mut a, Key::CloseCurlyBracket, Key::CloseBracket, "}");
        assert!(a.session.tools.brush.hardness > 0.5);
        assert_eq!(a.session.tools.brush.mode, BlendMode::Normal);
    }

    #[test]
    fn stepping_wraps_at_both_ends() {
        let m = BlendMode::LAYER_MODES;
        assert_eq!(stepped(&m, BlendMode::Normal, 1), Some(BlendMode::Dissolve));
        assert_eq!(stepped(&m, BlendMode::Normal, -1), m.last().copied());
        assert_eq!(stepped(&m, m[m.len() - 1], 1), Some(BlendMode::Normal));
        assert_eq!(stepped(&[], BlendMode::Normal, 1), None);
    }

    /// #3102: with the Move tool ⇧+ / ⇧− step the active layer's mode, one undo step each; with
    /// the Brush they step the brush's mode and leave the layer alone.
    #[test]
    fn shift_plus_and_minus_step_the_layer_or_the_brush_mode() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Move);
        let undo = a.session.active().unwrap().history.past_len();
        key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
        assert_eq!(layer_blend(&a), BlendMode::Dissolve);
        key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
        assert_eq!(layer_blend(&a), BlendMode::Darken);
        key(&mut a, &ctx, Key::Minus, egui::Modifiers::SHIFT);
        assert_eq!(layer_blend(&a), BlendMode::Dissolve);
        assert_eq!(a.session.active().unwrap().history.past_len(), undo + 3, "one undo step per press");
        // Unshifted or ⌘ presses are not blend-mode keys.
        key(&mut a, &ctx, Key::Minus, egui::Modifiers::NONE);
        key(&mut a, &ctx, Key::Plus, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(layer_blend(&a), BlendMode::Dissolve);
        a.ui.tool = Tool::Brush;
        key(&mut a, &ctx, Key::Minus, egui::Modifiers::SHIFT);
        assert_eq!(a.session.tools.brush.mode, BlendMode::LAYER_MODES[BlendMode::LAYER_MODES.len() - 1]);
        assert_eq!(layer_blend(&a), BlendMode::Dissolve, "the layer is untouched");
        // The Background keeps Normal; no document is a no-op.
        a.ui.tool = Tool::Move;
        let bg = a.session.active().unwrap().doc.layers[0].id;
        a.run("layer.select", json!({"layer": bg.0})).unwrap();
        key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
        assert_eq!(a.session.active().unwrap().doc.layers[0].blend, BlendMode::Normal);
        let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        press(&mut none, 1);
        set_mode(&mut none, BlendMode::Multiply);
    }

    const SHIFT_ALT: egui::Modifiers = egui::Modifiers { shift: true, alt: true, ..egui::Modifiers::NONE };

    #[test]
    fn every_listed_key_names_a_distinct_mode_and_each_mode_has_at_most_one_key() {
        let keys: Vec<Key> = Key::ALL.iter().copied().filter(|k| mode_for_key(*k).is_some()).collect();
        assert_eq!(keys.len(), 24, "Photoshop's 23 layer modes with a key, and Pass Through");
        let mut modes: Vec<BlendMode> = keys.iter().filter_map(|k| mode_for_key(*k)).collect();
        modes.sort_by_key(|m| m.label());
        modes.dedup();
        assert_eq!(modes.len(), 24);
        assert_eq!(mode_for_key(Key::M), Some(BlendMode::Multiply));
        assert_eq!(mode_for_key(Key::P), Some(BlendMode::PassThrough));
        assert_eq!(mode_for_key(Key::Num1), None);
    }

    /// ⇧⌥M is Multiply, not the Marquee's M: the blend keys run before the tool keys.
    #[test]
    fn shift_alt_letter_picks_the_layer_mode_before_any_tool_key_runs() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Move);
        let steps = a.session.active().unwrap().history.past_len();
        key(&mut a, &ctx, Key::M, SHIFT_ALT);
        assert_eq!(layer_blend(&a), BlendMode::Multiply);
        assert_eq!(a.ui.tool, Tool::Move, "the key was not taken for the Marquee tool");
        assert_eq!(a.session.active().unwrap().history.past_len(), steps + 1, "one undo step");
        // The same mode again changes nothing and adds no step.
        key(&mut a, &ctx, Key::M, SHIFT_ALT);
        assert_eq!(a.session.active().unwrap().history.past_len(), steps + 1);
        key(&mut a, &ctx, Key::S, SHIFT_ALT);
        assert_eq!(layer_blend(&a), BlendMode::Screen);
        // ⌘ or Ctrl with them is not this shortcut; plain ⇧ or ⌥ letters are tool keys, not modes.
        key(&mut a, &ctx, Key::O, SHIFT_ALT | egui::Modifiers::COMMAND);
        key(&mut a, &ctx, Key::O, egui::Modifiers::ALT);
        assert_eq!(layer_blend(&a), BlendMode::Screen);
    }

    #[test]
    fn shift_alt_letter_reads_the_physical_key_and_ignores_key_repeats() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Move);
        let feed = |a: &mut PhotocraftApp, key: Key, physical: Option<Key>, repeat: bool| {
            let raw = egui::RawInput {
                events: vec![
                    egui::Event::ModifiersChanged(SHIFT_ALT),
                    egui::Event::Key { key, physical_key: physical, pressed: true, repeat, modifiers: SHIFT_ALT },
                ],
                ..Default::default()
            };
            ctx.begin_pass(raw);
            handle(a, &ctx);
            ctx.end_pass().textures_delta.clear();
        };
        // A layout whose logical key isn't a letter still picks by the key's place.
        feed(&mut a, Key::Comma, Some(Key::M), false);
        assert_eq!(layer_blend(&a), BlendMode::Multiply);
        let steps = a.session.active().unwrap().history.past_len();
        // A repeat of the held key in egui's pass loop does not repeat or change state.
        feed(&mut a, Key::Comma, Some(Key::M), true);
        assert_eq!(layer_blend(&a), BlendMode::Multiply, "a held key does not repeat");
        assert_eq!(a.session.active().unwrap().history.past_len(), steps);
        // An event with repeat: true for a different mode is ignored as well.
        ctx.input_mut(|i| {
            i.events.push(egui::Event::Key {
                key: Key::S,
                physical_key: None,
                pressed: true,
                repeat: true,
                modifiers: SHIFT_ALT,
            });
        });
        handle(&mut a, &ctx);
        assert_eq!(layer_blend(&a), BlendMode::Multiply, "a held key does not repeat");
        assert_eq!(a.session.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn a_frame_without_a_blend_key_leaves_every_event_in_place() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Move);
        let events = vec![
            egui::Event::PointerMoved(egui::pos2(3.0, 4.0)),
            egui::Event::Key { key: Key::M, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        ];
        ctx.begin_pass(egui::RawInput { events: events.clone(), ..Default::default() });
        handle(&mut a, &ctx);
        assert_eq!(ctx.input(|i| i.events.len()), events.len());
        ctx.end_pass().textures_delta.clear();
    }

    /// Pass Through is a group's mode; the Background has no mode; several layers change in one step.
    #[test]
    fn direct_modes_skip_the_background_and_pass_through_applies_to_groups_only() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Move);
        key(&mut a, &ctx, Key::P, SHIFT_ALT);
        assert_eq!(layer_blend(&a), BlendMode::Normal, "not a group");
        a.run("layer.groupLayers", json!({})).unwrap();
        let group = a.session.active().unwrap().active_layer.unwrap();
        assert!(a.session.active().unwrap().doc.layer(group).unwrap().is_group());
        key(&mut a, &ctx, Key::P, SHIFT_ALT);
        assert_eq!(layer_blend(&a), BlendMode::PassThrough);
        // Background plus a layer selected: only the layer takes the mode.
        a.run("layer.new.layer", json!({})).unwrap();
        let top = a.session.active().unwrap().active_layer.unwrap();
        let bg = a.session.active().unwrap().doc.layers[0].id;
        a.run("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
        let steps = a.session.active().unwrap().history.past_len();
        key(&mut a, &ctx, Key::D, SHIFT_ALT);
        let st = a.session.active().unwrap();
        assert_eq!(st.doc.layer(top).unwrap().blend, BlendMode::ColorDodge);
        assert_eq!(st.doc.layer(bg).unwrap().blend, BlendMode::Normal);
        assert_eq!(st.history.past_len(), steps + 1);
    }

    #[test]
    fn direct_modes_reach_the_brush_and_the_gradient_not_the_layer() {
        let ctx = egui::Context::default();
        let mut a = app(Tool::Brush);
        key(&mut a, &ctx, Key::M, SHIFT_ALT);
        assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
        assert_eq!(layer_blend(&a), BlendMode::Normal, "the layer is untouched");
        a.ui.tool = Tool::Gradient;
        key(&mut a, &ctx, Key::S, SHIFT_ALT);
        assert_eq!(a.ui.tool_options.gradient_blend_mode, BlendMode::Screen);
        // The Paint Bucket has no blend mode to set.
        a.ui.tool = Tool::PaintBucket;
        key(&mut a, &ctx, Key::O, SHIFT_ALT);
        assert_eq!(layer_blend(&a), BlendMode::Normal);
    }
}
