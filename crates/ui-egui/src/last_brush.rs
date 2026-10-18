//! The brush the tool was last using, remembered across restarts (`Preferences::last_brush`) so
//! the next launch starts with the same brush, options bar and painting cursor outline, and every
//! other painting tool's own brush (`Preferences::tool_brushes`), so the Brush and the Eraser
//! don't swap brushes across a restart.
//!
//! Saved once the pointer is up (a slider drag saves once, like the Brush Preset picker's view),
//! and change-driven: `remember_brush` watches `ToolState::brush_rev` and the tools' brushes have a
//! revision of their own, so an unchanged brush costs a counter compare per frame, not a clone and
//! a JSON serialisation of the whole brush.

use crate::PhotocraftApp;

/// Remember the current brush once the pointer is up.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    photocraft_engine::brush_cmds::remember_brush(&mut app.session);
    if app.prefs_rt.tool_brushes_saved_rev != app.prefs_rt.tool_brushes_rev {
        app.prefs_rt.tool_brushes_saved_rev = app.prefs_rt.tool_brushes_rev;
        let now = crate::paint_mouse::tool_brushes_json(app);
        if app.session.prefs().tool_brushes != now {
            app.session.prefs.edit(|p| p.tool_brushes = now);
        }
    }
}

/// Restore the remembered brush at launch; stored tips load when the preset store attaches
/// (`Session::attach_preset_store`).
pub fn restore(app: &mut PhotocraftApp) {
    photocraft_engine::brush_cmds::restore_brush(&mut app.session);
    crate::paint_mouse::restore_tool_brushes(app);
}
