//! Holding a brush key (`[` `]` ⇧`[` ⇧`]`, `brush_key_cmds`): the size or hardness steps on the
//! app's own clock instead of the system's key repeat, so the pause before it starts and the time
//! between steps are settings ([`KeyRepeat`], the gear beside the brush picker's Size field). The
//! system's repeat presses for these keys are dropped ([`swallows`]); [`tick`] steps while the key
//! is down.
//!
//! While a stroke is being painted the brush stays as the stroke began (user report: holding `]`
//! and starting to paint drew the cursor at a size the stroke wasn't using, and the release
//! re-rendered the stroke at it): a press or a step waits for the stroke to end ([`painting`]).

use egui::{Key, Ui};
use serde::{Deserialize, Serialize};

use crate::{PhotocraftApp, icons, widgets};

/// The brush keys whose holding this module paces.
pub const IDS: [&str; 4] = ["tools.decreaseBrushSize", "tools.increaseBrushSize", "tools.decreaseBrushHardness", "tools.increaseBrushHardness"];

/// The system-like pause before a held key repeats, seconds.
const PAUSE_S: f64 = 0.5;
/// The quickest and slowest step a setting may ask for, ms.
pub const MIN_INTERVAL_MS: f32 = 10.0;
pub const MAX_INTERVAL_MS: f32 = 5000.0;

/// How a held brush key repeats (`ToolOptions::brush_key_repeat`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyRepeat {
    /// Wait half a second before repeating, as the system does; off, the steps start at once.
    pub pause: bool,
    /// Time between steps while the key is held, ms.
    pub interval_ms: f32,
}

impl Default for KeyRepeat {
    fn default() -> Self {
        Self { pause: true, interval_ms: 50.0 }
    }
}

impl KeyRepeat {
    fn interval_s(&self) -> f64 {
        let ms = if self.interval_ms.is_finite() { self.interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS) } else { 50.0 };
        f64::from(ms) / 1000.0
    }

    /// When the step after one at `last` is due, for a key pressed at `pressed` (seconds).
    pub fn due(&self, pressed: f64, last: f64) -> f64 {
        let next = last + self.interval_s();
        if self.pause && last <= pressed { next.max(pressed + PAUSE_S) } else { next }
    }
}

/// A brush key being held: which command, the key, when it went down and when it last stepped.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub id: String,
    pub key: Key,
    pub pressed: f64,
    pub last: f64,
    /// Pressed during a stroke: its first step comes when the stroke ends.
    pub deferred: bool,
}

/// A brush stroke is being painted: the brush keys wait for it to end.
pub fn painting(app: &PhotocraftApp) -> bool {
    app.drag.as_ref().is_some_and(|d| crate::brush_resize::applies(d.tool))
}

/// Whether the shortcut `id` fired by a key event is the system repeating the brush key being
/// held: dropped, as [`tick`] does the repeating. A repeat of anything else runs (egui calls any
/// press of a key it still thinks is down a repeat, and a lost release mustn't eat a press).
pub fn swallows(held: Option<&Held>, id: &str, repeat: bool) -> bool {
    repeat && IDS.contains(&id) && held.is_some_and(|h| h.id == id)
}

/// A brush key went down (not a repeat): start pacing it. `ran`: its command ran (false when it
/// waits for the stroke being painted, [`painting`]).
pub fn pressed(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, key: Key, ran: bool) {
    if IDS.contains(&id) {
        let now = ctx.input(|i| i.time);
        app.brush_key_held = Some(Held { id: id.into(), key, pressed: now, last: now, deferred: !ran });
        ctx.request_repaint();
    }
}

/// Each frame: while the key is down, step when the next step is due; stop when it's let go.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(held) = app.brush_key_held.clone() else { return };
    let (down, now) = ctx.input(|i| (i.key_down(held.key), i.time));
    let interval = std::time::Duration::from_secs_f64(app.ui.tool_options.brush_key_repeat.interval_s());
    if painting(app) {
        // The stroke keeps its brush; the key carries on once it ends (a press made during the
        // stroke still takes its step then, even if let go).
        if down || held.deferred {
            ctx.request_repaint_after(interval);
        } else {
            app.brush_key_held = None;
        }
        return;
    }
    if held.deferred {
        crate::shortcut_dispatch::dispatch(app, ctx, &held.id);
        app.brush_key_held = down.then_some(Held { last: now, deferred: false, ..held });
        ctx.request_repaint_after(interval);
        return;
    }
    if !down {
        app.brush_key_held = None;
        return;
    }
    let due = app.ui.tool_options.brush_key_repeat.due(held.pressed, held.last);
    if now + 1e-6 >= due {
        crate::shortcut_dispatch::dispatch(app, ctx, &held.id);
        if let Some(h) = app.brush_key_held.as_mut() {
            h.last = now;
        }
        ctx.request_repaint_after(interval);
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
    }
}

/// The gear beside the Size field: how holding `[` / `]` (and ⇧ for hardness) repeats.
pub fn settings_button(ui: &mut Ui, r: &mut KeyRepeat) {
    let button = icons::button(ui, "settings", 22.0, false, tl!("Size Key Repeat"));
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(240.0);
        ui.label(egui::RichText::new(tl!("Holding [ or ] to resize the brush")).strong());
        widgets::checkbox(ui, &mut r.pause, tl!("Pause before repeating")).on_hover_text(tl!("Wait half a second before the size keeps changing, as typing does"));
        ui.horizontal(|ui| {
            ui.label(tl!("Change every"));
            widgets::value_field(ui, &mut r.interval_ms, MIN_INTERVAL_MS..=MAX_INTERVAL_MS, "ms", 72.0);
        });
    });
}

#[cfg(test)]
mod tests;
