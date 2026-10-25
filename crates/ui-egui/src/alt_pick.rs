//! Alt with a painting tool is the temporary Eyedropper (`canvas::alt_samples`). With the
//! Eyedropper's Sample on Alt on, pressing Alt picks the colour under the pointer at once, with no
//! click. A quick tap is only that pick: Alt works as usual (the pipette and its ring, click or
//! drag to sample) once it has been held for the Hold time, so a tap doesn't flash the pipette.
//! The pick shows as a brief ring of the colour taken at the pointer ([`draw_flash`]).

use egui::Modifiers;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The longest Hold time, ms.
pub const MAX_HOLD_MS: f32 = 5000.0;

/// Where a press of Alt is (`PhotocraftApp::alt_pick`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AltPick {
    /// When Alt went down (egui time, seconds); `None` while it's up.
    down_at: Option<f64>,
    /// This press picked its colour.
    picked: bool,
    /// The last pick: when (egui time), where (document pixels) and the colour taken, for the
    /// brief ring that shows it.
    flash: Option<(f64, [f64; 2], egui::Color32)>,
}

/// How long the ring of a pick shows, seconds.
const FLASH_S: f64 = 0.35;

/// Plain Alt: with ⌘/Ctrl or ⇧ it means something else (brush resize, other shortcuts).
fn plain_alt(m: Modifiers) -> bool {
    m.alt && !m.ctrl && !m.command && !m.shift
}

/// Each frame for each canvas view: notes Alt going down and, the frame it does, picks the
/// colour at `hover` (document pixels) when Sample on Alt is on and `tool` samples with Alt.
/// `active` says the view shows the active document (the one sampled).
pub fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, tool: Tool, hover: Option<[f64; 2]>, active: bool) {
    let (mods, now) = ctx.input(|i| (i.modifiers, i.time));
    if !plain_alt(mods) {
        app.alt_pick = AltPick { flash: app.alt_pick.flash, ..AltPick::default() };
        return;
    }
    let down_at = *app.alt_pick.down_at.get_or_insert(now);
    let o = &app.ui.tool_options;
    let (instant, hold) = (o.eyedropper_alt_instant, hold_s(o.eyedropper_alt_hold_ms));
    // Only the frame Alt went down: moving onto the canvas with it held picks nothing.
    if !instant || app.alt_pick.picked || down_at != now || app.drag.is_some() || !active || !crate::canvas::alt_samples(tool, mods) {
        return;
    }
    let Some([x, y]) = hover else { return };
    app.alt_pick.picked = true;
    crate::canvas::sample_eyedropper(app, x, y, Modifiers::NONE, false);
    let c = app.session.tools.foreground;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    app.alt_pick.flash = Some((now, [x, y], egui::Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))));
    ctx.request_repaint();
    // The pipette appears when the hold time is up, pointer moving or not.
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(hold));
}

/// The ring a pick leaves at the pointer for a moment: a disc of the colour taken, inside a ring
/// that widens and fades, so a pick with no click is seen to have happened.
pub fn draw_flash(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &crate::canvas::ViewXform) {
    let Some((at, [x, y], color)) = app.alt_pick.flash else { return };
    let now = painter.ctx().input(|i| i.time);
    let k = ((now - at) / FLASH_S) as f32;
    if !(0.0..1.0).contains(&k) {
        app.alt_pick.flash = None;
        return;
    }
    let p = xf.to_screen(x as f32, y as f32);
    let fade = 1.0 - k;
    let r = 7.0 + 7.0 * k;
    painter.circle_filled(p, 5.0, color.gamma_multiply(fade));
    painter.circle_stroke(p, r + 1.0, egui::Stroke::new(1.0, egui::Color32::from_black_alpha((140.0 * fade) as u8)));
    painter.circle_stroke(p, r, egui::Stroke::new(2.0, color.gamma_multiply(fade)));
    painter.ctx().request_repaint();
}

/// Whether the ring of a pick is showing (tests).
pub fn flashing(app: &PhotocraftApp) -> bool {
    app.alt_pick.flash.is_some()
}

fn hold_s(ms: f32) -> f64 {
    f64::from(if ms.is_finite() { ms.clamp(0.0, MAX_HOLD_MS) } else { 0.0 }) / 1000.0
}

/// Whether held Alt shows and acts as the temporary Eyedropper at time `now`: at once, or with
/// Sample on Alt once it has been held for the Hold time.
pub fn eyedropper_shows(app: &PhotocraftApp, now: f64) -> bool {
    let o = &app.ui.tool_options;
    !o.eyedropper_alt_instant || app.alt_pick.down_at.is_none_or(|t| now - t + 1e-9 >= hold_s(o.eyedropper_alt_hold_ms))
}

#[cfg(test)]
mod tests;
