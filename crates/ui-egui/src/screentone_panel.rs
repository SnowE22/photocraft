//! The Screentones dock tab: the GenTonik screentone generator's panel.
//!
//! The tab lives in the Color dock column after Patterns (a `Group::Color` tab in `dock`), and
//! edits one [`ScreentoneSpec`], turning it into document pixels through `layer.screentone` (a
//! procedural sheet on a new layer), `filter.screentone` (the same generator applied to the
//! active layer, where Use Source Luminance can halftone by that layer's own darkness) or
//! `pattern.screentone` (the sheet saved into the pattern library). The spec's serde names are
//! the commands' flat parameter notation, so what the panel sends is exactly what a filter
//! dialog or a saved preset would.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use photocraft_algo::{ScreentoneBackend, ScreentoneMode, ScreentoneSpec};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The Window-menu id of the panel.
pub const PANEL: &str = "window.panel.screentone";

/// The pattern types the panel offers, in the commands' own notation.
const PATTERNS: [&str; 12] = ["dots", "lines", "crosshatch", "noise", "gaussianNoise", "stipple", "hexgrid", "checker", "concentric", "stars", "hearts", "triangles"];
/// The dot shapes the panel offers (the dots' and the satellites').
const SHAPES: [&str; 4] = ["circle", "square", "diamond", "hexagon"];
/// The gradient types the panel offers for modulation.
const GRADIENTS: [&str; 3] = ["none", "linear", "radial"];

/// Panel state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScreentoneUi {
    /// The generator's settings; picking a preset replaces them wholesale.
    pub spec: ScreentoneSpec,
    /// The built-in preset the settings came from ("" = the panel's own settings).
    pub preset: String,
    /// The new layer's name; empty = the preset's name, or "Screentone".
    pub name: String,
}

impl Default for ScreentoneUi {
    fn default() -> Self {
        Self {
            spec: ScreentoneSpec {
                mode: ScreentoneMode::Shading,
                use_source_luminance: true,
                dot_size: 4.5,
                spacing_x: 8.0,
                spacing_y: 8.0,
                angle: 45.0,
                density: 0.75,
                background: [1.0, 1.0, 1.0, 1.0],
                preserve_lines: true,
                line_threshold: 0.24,
                line_thickness: 1.0,
                ..ScreentoneSpec::default()
            },
            preset: String::new(),
            name: String::new(),
        }
    }
}

/// The enum's notation id, from its serde name (the commands' notation): `Dots` → `"dots"`.
fn id_of<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// The enum value an id names, or `None` for anything else.
fn from_id<T: serde::de::DeserializeOwned>(id: &str) -> Option<T> {
    serde_json::from_value(json!(id)).ok()
}

/// Window-menu ids handled here.
pub fn handles(id: &str) -> bool {
    id == PANEL
}

/// The panel's tab index in the Color dock group (`dock`); both pro and Studio orders.
pub(crate) const COLOR_TAB: usize = 4;

/// Window › Screentones: whether the Color card shows this tab now.
pub fn showing(app: &PhotocraftApp) -> bool {
    app.ui.panels.color && app.ui.dock_tabs.color == COLOR_TAB && !app.ui.dock.is_collapsed(crate::dock::Group::Color)
}

/// Window › Screentones: toggles the panel (selects and reveals the Color card tab, as Window ›
/// Patterns does; a collapsed Color group counts as not showing: the item expands it #129).
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !handles(id) {
        return None;
    }
    let want = params.get("show").and_then(Value::as_bool);
    let show = want.unwrap_or(!showing(app));
    app.ui.panels.color = show || (app.ui.panels.color && app.ui.dock_tabs.color != COLOR_TAB);
    if show {
        app.ui.dock_tabs.color = COLOR_TAB;
        crate::dock::reveal(app, crate::dock::Group::Color);
    }
    Some(Ok(json!({"visible": show})))
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    handles(id).then_some(showing(app))
}

/// The parameters for `layer.screentone`, `filter.screentone` and `pattern.screentone`: the
/// spec's own serde object (the commands' flat notation) plus the sheet name when the panel's
/// field holds one.
pub(crate) fn params(st: &ScreentoneUi) -> Value {
    let mut p = serde_json::to_value(&st.spec).unwrap_or_else(|_| json!({}));
    if let Some(map) = p.as_object_mut() {
        let name = st.name.trim();
        if !name.is_empty() {
            map.insert("name".into(), json!(name));
        } else {
            map.remove("name");
        }
    }
    p
}

/// Restyles the active `LayerContent::Screentone` layer in place when the panel's spec changes,
/// coalescing consecutive panel edits on that layer into one history step.
pub(crate) fn sync_spec_to_active_layer(app: &mut PhotocraftApp) -> bool {
    let Some((doc_id, layer_id, layer_spec)) = app.session.active().and_then(|st| {
        let id = st.active_layer?;
        let photocraft_doc::LayerContent::Screentone(tone) = &st.doc.layer(id)?.content else {
            return None;
        };
        Some((st.doc.id, id, tone.spec.clone()))
    }) else {
        return false;
    };
    if app.ui.screentone_ui.spec == layer_spec {
        return false;
    }
    log::info!(
        "[SCREENTONE:UI] sync_spec_to_active_layer: doc={}, layer={}, backend={:?}, mode={:?}, pattern={:?}, dot_size={:.1}, spacing=({:.1}, {:.1}), density={:.2}",
        doc_id.0, layer_id.0, app.ui.screentone_ui.spec.backend, app.ui.screentone_ui.spec.mode,
        app.ui.screentone_ui.spec.pattern_type, app.ui.screentone_ui.spec.dot_size,
        app.ui.screentone_ui.spec.spacing_x, app.ui.screentone_ui.spec.spacing_y,
        app.ui.screentone_ui.spec.density
    );
    let mut p = params(&app.ui.screentone_ui);
    if let Some(map) = p.as_object_mut() {
        map.insert("layer".into(), json!(layer_id.0));
        map.insert("coalesce".into(), json!(format!("screentone-panel:{}:{}", doc_id.0, layer_id.0)));
    }
    let result = crate::preset_panels::run(app, photocraft_engine::screentone_cmds::LAYER, p);
    crate::screentone_gizmo::bind_layer_result(app, photocraft_engine::screentone_cmds::LAYER, &result);
    true
}

/// Freezes (rasterizes) the active `LayerContent::Screentone` layer into a regular pixel layer,
/// preserving its layer mask and detaching the on-canvas gizmo so the next screentone can be
/// created independently.
pub(crate) fn freeze_active_layer(app: &mut PhotocraftApp) -> bool {
    let Some(id) = app
        .session
        .active()
        .and_then(|s| s.active_layer.filter(|&id| s.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone)))
    else {
        return false;
    };
    let _ = crate::preset_panels::run(app, "layer.rasterize.layer", json!({"layer": id.0}));
    crate::screentone_gizmo::detach(app);
    true
}

/// Draws the tab body: the dock's own scrolling wraps it in both the native and shared docks.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::screentone_gizmo::sync_active_layer(app);
    let t = Tokens::get(ui.ctx());
    let label = crate::filter_dialog::label;
    let mut st = std::mem::take(&mut app.ui.screentone_ui);
    let mut action: Option<&'static str> = None;
    let mut freeze_clicked = false;
    let row = |ui: &mut egui::Ui, text: &str| {
        ui.label(egui::RichText::new(text).color(t.text_dim).size(11.5));
    };

    ui.horizontal(|ui| {
        row(ui, tl!("Mode:"));
        if ui
            .selectable_label(st.spec.mode == ScreentoneMode::Shading, tl!("Manga / Shading"))
            .clicked()
        {
            log::info!("[SCREENTONE:UI] Mode switched to: Manga / Shading (use_source_luminance=true)");
            st.spec.mode = ScreentoneMode::Shading;
            st.spec.use_source_luminance = true;
            st.spec.background[3] = 1.0;
        }
        if ui
            .selectable_label(st.spec.mode == ScreentoneMode::Sheet, tl!("Sheet / Pattern"))
            .clicked()
        {
            log::info!("[SCREENTONE:UI] Mode switched to: Sheet / Pattern (use_source_luminance=false)");
            st.spec.mode = ScreentoneMode::Sheet;
            st.spec.use_source_luminance = false;
            st.spec.background[3] = 0.0;
        }
    });
    ui.horizontal(|ui| {
        row(ui, tl!("Engine:"));
        if ui
            .selectable_label(st.spec.backend == ScreentoneBackend::Auto, tl!("Auto"))
            .on_hover_text(tl!("GPU on large canvases, CPU on small"))
            .clicked()
        {
            log::info!("[SCREENTONE:UI] Engine switched to: Auto");
            st.spec.backend = ScreentoneBackend::Auto;
        }
        if ui
            .selectable_label(st.spec.backend == ScreentoneBackend::Gpu, tl!("⚡ GPU"))
            .on_hover_text(tl!("Ultra-fast real-time rendering for giant files"))
            .clicked()
        {
            log::info!("[SCREENTONE:UI] Engine switched to: ⚡ GPU");
            st.spec.backend = ScreentoneBackend::Gpu;
        }
        if ui
            .selectable_label(st.spec.backend == ScreentoneBackend::Cpu, tl!("⚙ CPU"))
            .on_hover_text(tl!("Exact reference CPU engine"))
            .clicked()
        {
            log::info!("[SCREENTONE:UI] Engine switched to: ⚙ CPU");
            st.spec.backend = ScreentoneBackend::Cpu;
        }
    });
    ui.add_space(4.0);

    egui::Grid::new("screentone-grid").num_columns(2).spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
        // The built-in presets seed the whole spec; "Custom" keeps what the panel has.
        row(ui, &label("preset"));
        let presets = photocraft_algo::screentone::built_in_presets();
        let mut opts: Vec<(String, &str)> = std::iter::once((String::new(), tl!("Custom"))).collect();
        opts.extend(presets.iter().map(|p| (p.id.to_string(), p.name)));
        let mut cur = st.preset.clone();
        if crate::widgets::dropdown_literal(ui, "screentone-preset", &mut cur, &opts, 176.0) && cur != st.preset {
            st.preset = cur.clone();
            if let Some(p) = presets.iter().find(|p| p.id == cur) {
                let prev_mode = st.spec.mode;
                let prev_bg_alpha = st.spec.background[3];
                let prev_brightness = st.spec.brightness;
                let prev_contrast = st.spec.contrast;
                let prev_white = st.spec.white_threshold;
                let prev_black = st.spec.black_threshold;
                let prev_gamma = st.spec.tone_gamma;
                let prev_lines = st.spec.preserve_lines;
                let prev_line_thresh = st.spec.line_threshold;
                let prev_line_thick = st.spec.line_thickness;
                st.spec = p.spec.clone();
                st.spec.mode = prev_mode;
                st.spec.use_source_luminance = prev_mode == ScreentoneMode::Shading;
                st.spec.background[3] = prev_bg_alpha;
                st.spec.brightness = prev_brightness;
                st.spec.contrast = prev_contrast;
                st.spec.white_threshold = prev_white;
                st.spec.black_threshold = prev_black;
                st.spec.tone_gamma = prev_gamma;
                st.spec.preserve_lines = prev_lines;
                st.spec.line_threshold = prev_line_thresh;
                st.spec.line_thickness = prev_line_thick;
            }
        }
        ui.end_row();

        row(ui, &label("patternType"));
        let mut cur = id_of(&st.spec.pattern_type);
        let labels: Vec<String> = PATTERNS.iter().map(|p| crate::filter_dialog::source_label(p)).collect();
        let opts: Vec<(String, &str)> = PATTERNS.iter().map(|p| p.to_string()).zip(labels.iter().map(String::as_str)).collect();
        if crate::widgets::dropdown(ui, "screentone-pattern", &mut cur, &opts, 176.0)
            && let Some(p) = from_id(&cur)
        {
            st.spec.pattern_type = p;
        }
        ui.end_row();

        row(ui, &label("dotShape"));
        let mut cur = id_of(&st.spec.dot_shape);
        let labels: Vec<String> = SHAPES.iter().map(|s| crate::filter_dialog::source_label(s)).collect();
        let opts: Vec<(String, &str)> = SHAPES.iter().map(|s| s.to_string()).zip(labels.iter().map(String::as_str)).collect();
        if crate::widgets::dropdown(ui, "screentone-shape", &mut cur, &opts, 176.0)
            && let Some(s) = from_id(&cur)
        {
            st.spec.dot_shape = s;
        }
        ui.end_row();

        row(ui, &label("dotSize"));
        crate::widgets::value_field(ui, &mut st.spec.dot_size, 0.0..=500.0, "px", 76.0);
        ui.end_row();

        row(ui, &label("spacingX"));
        crate::widgets::value_field(ui, &mut st.spec.spacing_x, 1.0..=2000.0, "px", 76.0);
        ui.end_row();

        row(ui, &label("spacingY"));
        crate::widgets::value_field(ui, &mut st.spec.spacing_y, 1.0..=2000.0, "px", 76.0);
        ui.end_row();

        row(ui, tl!("Row Offset:"));
        crate::widgets::value_field(ui, &mut st.spec.row_offset, -1.0..=1.0, "", 76.0);
        ui.end_row();

        row(ui, &label("angle"));
        if crate::widgets::value_field(ui, &mut st.spec.angle, 0.0..=360.0, "°", 76.0).changed() {
            st.spec.angle = st.spec.angle.rem_euclid(360.0);
        }
        ui.end_row();

        row(ui, &label("density"));
        crate::widgets::value_field(ui, &mut st.spec.density, 0.0..=1.0, "", 76.0);
        ui.end_row();

        row(ui, &label("lineWidth"));
        crate::widgets::value_field(ui, &mut st.spec.line_width, 0.0..=200.0, "px", 76.0);
        ui.end_row();

        row(ui, &label("crossAngle"));
        crate::widgets::value_field(ui, &mut st.spec.cross_angle, -180.0..=180.0, "°", 76.0);
        ui.end_row();

        row(ui, &label("roundness"));
        crate::widgets::value_field(ui, &mut st.spec.roundness, 0.0..=1.0, "", 76.0);
        ui.end_row();

        row(ui, &label("smoothness"));
        crate::widgets::value_field(ui, &mut st.spec.smoothness, 0.0..=1.0, "", 76.0);
        ui.end_row();

        row(ui, &label("seed"));
        ui.horizontal(|ui| {
            let mut seed = st.spec.seed as f32;
            if crate::widgets::value_field(ui, &mut seed, 0.0..=30_000.0, "", 76.0).changed() {
                st.spec.seed = seed.round().max(0.0) as u32;
            }
            if ui.button(tl!("Random")).clicked() {
                st.spec.seed = crate::filter_dialog::random_seed(ui.input(|i| i.time).to_bits(), st.spec.seed as i64) as u32;
            }
        });
        ui.end_row();

        row(ui, "");
        let mut opaque_paper = st.spec.background[3] >= 0.5;
        if crate::widgets::checkbox(ui, &mut opaque_paper, tl!("Opaque Paper")).changed() {
            st.spec.background[3] = if opaque_paper { 1.0 } else { 0.0 };
        }
        ui.end_row();

        if st.spec.mode == ScreentoneMode::Shading {
            row(ui, tl!("Brightness:"));
            crate::widgets::value_field(ui, &mut st.spec.brightness, -1.0..=1.0, "", 76.0);
            ui.end_row();

            row(ui, tl!("Contrast:"));
            crate::widgets::value_field(ui, &mut st.spec.contrast, 0.0..=4.0, "", 76.0);
            ui.end_row();

            row(ui, tl!("White Cutoff:"));
            crate::widgets::value_field(ui, &mut st.spec.white_threshold, 0.0..=1.0, "", 76.0);
            ui.end_row();

            row(ui, tl!("Black Cutoff:"));
            crate::widgets::value_field(ui, &mut st.spec.black_threshold, 0.0..=1.0, "", 76.0);
            ui.end_row();

            row(ui, tl!("Tone Gamma:"));
            crate::widgets::value_field(ui, &mut st.spec.tone_gamma, 0.1..=5.0, "", 76.0);
            ui.end_row();

            row(ui, "");
            crate::widgets::checkbox(ui, &mut st.spec.preserve_lines, tl!("Preserve Lines"));
            ui.end_row();

            if st.spec.preserve_lines {
                row(ui, tl!("Line Threshold:"));
                crate::widgets::value_field(ui, &mut st.spec.line_threshold, 0.05..=0.65, "", 76.0);
                ui.end_row();

                row(ui, tl!("Line Thickness:"));
                crate::widgets::value_field(ui, &mut st.spec.line_thickness, 0.5..=5.0, "px", 76.0);
                ui.end_row();
            }
        }
    });

    ui.add_space(4.0);

    egui::CollapsingHeader::new(tl!("Satellites"))
        .id_salt("screentone-satellites")
        .default_open(st.spec.satellite_enabled)
        .show(ui, |ui| {
            egui::Grid::new("screentone-satellites-grid").num_columns(2).spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
                row(ui, "");
                crate::widgets::checkbox(ui, &mut st.spec.satellite_enabled, tl!("Enable"));
                ui.end_row();

                row(ui, tl!("Count:"));
                let mut count = st.spec.satellite_count as f32;
                if crate::widgets::value_field(ui, &mut count, 1.0..=4.0, "", 76.0).changed() {
                    st.spec.satellite_count = count.round().clamp(1.0, 4.0) as u32;
                }
                ui.end_row();

                row(ui, tl!("Distance:"));
                crate::widgets::value_field(ui, &mut st.spec.satellite_distance, 0.0..=1000.0, "px", 76.0);
                ui.end_row();

                row(ui, tl!("Size:"));
                crate::widgets::value_field(ui, &mut st.spec.satellite_size, 0.0..=500.0, "px", 76.0);
                ui.end_row();

                row(ui, tl!("Angle:"));
                crate::widgets::value_field(ui, &mut st.spec.satellite_angle, -180.0..=180.0, "°", 76.0);
                ui.end_row();

                row(ui, tl!("Shape:"));
                let mut cur = id_of(&st.spec.satellite_dot_shape);
                let labels: Vec<String> = SHAPES.iter().map(|s| crate::filter_dialog::source_label(s)).collect();
                let opts: Vec<(String, &str)> = SHAPES.iter().map(|s| s.to_string()).zip(labels.iter().map(String::as_str)).collect();
                if crate::widgets::dropdown(ui, "screentone-sat-shape", &mut cur, &opts, 176.0)
                    && let Some(s) = from_id(&cur)
                {
                    st.spec.satellite_dot_shape = s;
                }
                ui.end_row();

                row(ui, tl!("Stretch:"));
                crate::widgets::value_field(ui, &mut st.spec.satellite_stretch, 0.0..=10.0, "", 76.0);
                ui.end_row();
            });
        });

    egui::CollapsingHeader::new(tl!("Distortion & Jitter"))
        .id_salt("screentone-distortion")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("screentone-distortion-grid").num_columns(2).spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
                row(ui, tl!("Aspect X:"));
                crate::widgets::value_field(ui, &mut st.spec.aspect_x, 0.1..=10.0, "", 76.0);
                ui.end_row();

                row(ui, tl!("Aspect Y:"));
                crate::widgets::value_field(ui, &mut st.spec.aspect_y, 0.1..=10.0, "", 76.0);
                ui.end_row();

                row(ui, tl!("Merge:"));
                crate::widgets::value_field(ui, &mut st.spec.merge_factor, 0.0..=1.0, "", 76.0);
                ui.end_row();

                row(ui, tl!("Jitter Pos:"));
                crate::widgets::value_field(ui, &mut st.spec.jitter_pos, 0.0..=100.0, "%", 76.0);
                ui.end_row();

                row(ui, tl!("Jitter Size:"));
                crate::widgets::value_field(ui, &mut st.spec.jitter_size, 0.0..=100.0, "%", 76.0);
                ui.end_row();
            });
        });

    egui::CollapsingHeader::new(tl!("Gradient"))
        .id_salt("screentone-gradient")
        .default_open(st.spec.grad_type != photocraft_algo::ScreentoneGradientType::None)
        .show(ui, |ui| {
            egui::Grid::new("screentone-gradient-grid").num_columns(2).spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
                row(ui, tl!("Type:"));
                let mut cur = id_of(&st.spec.grad_type);
                let labels: Vec<String> = GRADIENTS.iter().map(|g| crate::filter_dialog::source_label(g)).collect();
                let opts: Vec<(String, &str)> = GRADIENTS.iter().map(|g| g.to_string()).zip(labels.iter().map(String::as_str)).collect();
                if crate::widgets::dropdown(ui, "screentone-grad-type", &mut cur, &opts, 176.0)
                    && let Some(g) = from_id(&cur)
                {
                    st.spec.grad_type = g;
                }
                ui.end_row();

                row(ui, tl!("Angle:"));
                crate::widgets::value_field(ui, &mut st.spec.grad_angle, -180.0..=180.0, "°", 76.0);
                ui.end_row();

                row(ui, "");
                crate::widgets::checkbox(ui, &mut st.spec.grad_reverse, tl!("Reverse"));
                ui.end_row();

                row(ui, tl!("Midpoint:"));
                crate::widgets::value_field(ui, &mut st.spec.grad_midpoint, 0.0..=1.0, "", 76.0);
                ui.end_row();

                row(ui, tl!("Start Size:"));
                crate::widgets::value_field(ui, &mut st.spec.grad_size_start, 0.0..=10.0, "", 76.0);
                ui.end_row();

                row(ui, tl!("End Size:"));
                crate::widgets::value_field(ui, &mut st.spec.grad_size_end, 0.0..=10.0, "", 76.0);
                ui.end_row();
            });
        });

    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(4.0);
    let can_freeze = app
        .session
        .active()
        .is_some_and(|s| s.active_layer.is_some_and(|id| s.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone)));
    ui.horizontal(|ui| {
        if ui.button(tl!("New Layer")).clicked() {
            action = Some(photocraft_engine::screentone_cmds::LAYER);
        }
        if ui.button(tl!("Apply")).clicked() {
            action = Some(photocraft_engine::screentone_cmds::FILTER);
        }
        if ui
            .add_enabled(can_freeze, egui::Button::new(tl!("Freeze Layer")))
            .on_hover_text(tl!("Bake the live screentone layer into standard pixels while keeping its mask"))
            .clicked()
        {
            freeze_clicked = true;
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("Save as Pattern")).clicked() {
            action = Some(photocraft_engine::screentone_cmds::PATTERN);
        }
        ui.add(
            egui::TextEdit::singleline(&mut st.name)
                .hint_text(tl!("Pattern Name"))
                .desired_width(116.0),
        );
    });
    app.ui.screentone_ui = st;
    if freeze_clicked {
        log::info!("[SCREENTONE:UI] Freeze (Rasterize) active layer clicked");
        freeze_active_layer(app);
    } else if let Some(command) = action {
        let params = params(&app.ui.screentone_ui);
        log::info!("[SCREENTONE:UI] Action clicked: command={}, params={:?}", command, params);
        let result = crate::preset_panels::run(app, command, params);
        // A New Layer run's result binds the gizmo to the generated sheet: the drags restyle
        // that layer in place (see `screentone_gizmo`).
        crate::screentone_gizmo::bind_layer_result(app, command, &result);
    } else {
        // Defer full-canvas CPU restyle while the pointer is actively dragging a slider so
        // slider movement stays at 60–144 FPS; commit as soon as the pointer releases.
        let pointer_down = ui.ctx().input(|i| i.pointer.any_down());
        if !pointer_down {
            sync_spec_to_active_layer(app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::PatternType;

    fn app(depth: u64) -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
        app.sync_views();
        (app, ctx)
    }

    #[test]
    fn window_menu_toggles_the_dock_tab() {
        let (mut app, ctx) = app(8);
        // The tab lives in the Color dock group after Patterns, in both pro and Studio orders.
        for pro in [false, true] {
            assert_eq!(crate::dock::Group::Color.tabs(pro).get(COLOR_TAB), Some(&"GenTonik"), "pro {pro}");
        }
        assert!(crate::menus::is_live(PANEL), "{PANEL}");
        crate::menus::invoke(&mut app, &ctx, PANEL, Value::Null).unwrap();
        assert!(app.ui.panels.color, "the Color card shows");
        assert_eq!(app.ui.dock_tabs.color, COLOR_TAB, "the tab is selected");
        assert_eq!(checked(&app, PANEL), Some(true));
        crate::menus::invoke(&mut app, &ctx, PANEL, json!({"show": false})).unwrap();
        assert_eq!(checked(&app, PANEL), Some(false));
        assert_eq!(checked(&app, "window.panel.layers"), None);
    }

    #[test]
    fn the_params_are_the_commands_own_notation() {
        let mut st = ScreentoneUi::default();
        st.spec.dot_size = 9.0;
        st.spec.pattern_type = PatternType::Crosshatch;
        st.name = " Tone ".into();
        let p = params(&st);
        assert_eq!(p["dotSize"], json!(9.0));
        assert_eq!(p["patternType"], json!("crosshatch"));
        assert_eq!(p["dotShape"], json!("circle"));
        assert_eq!(p["name"], json!("Tone"));
        // No name field: the command falls back to the preset's name.
        st.name = "  ".into();
        assert!(params(&st).get("name").is_none());
        // A preset replaces the whole spec, so the params follow it.
        let preset = photocraft_algo::screentone::built_in_presets().into_iter().next().unwrap();
        st.spec = preset.spec.clone();
        assert_eq!(params(&st)["dotSize"], json!(preset.spec.dot_size));
        // Every id the panel offers round-trips through the commands' notation.
        for id in PATTERNS {
            assert_eq!(id_of(&from_id::<PatternType>(id).unwrap()), id);
        }
        for id in SHAPES {
            assert_eq!(id_of(&from_id::<photocraft_algo::DotShape>(id).unwrap()), id);
        }
        for id in GRADIENTS {
            assert_eq!(id_of(&from_id::<photocraft_algo::ScreentoneGradientType>(id).unwrap()), id);
        }
        assert!(from_id::<PatternType>("spiral").is_none());
    }

    #[test]
    fn the_panel_makes_a_screentone_layer_at_every_depth() {
        for depth in [8, 16, 32] {
            let (mut app, ctx) = app(depth);
            app.ui.screentone_ui.spec.pattern_type = PatternType::Checker;
            app.ui.screentone_ui.spec.spacing_x = 8.0;
            app.ui.screentone_ui.spec.spacing_y = 8.0;
            app.ui.screentone_ui.name = "Tone".into();
            let before = app.session.active().unwrap().doc.layer_count();
            let p = params(&app.ui.screentone_ui);
            crate::menus::invoke(&mut app, &ctx, photocraft_engine::screentone_cmds::LAYER, p).unwrap_or_else(|e| panic!("layer@{depth}: {e}"));
            let d = app.session.active().unwrap();
            assert_eq!(d.doc.layer_count(), before + 1, "depth {depth}");
            // The document makes the name unique ("Tone", "Tone 1", …).
            assert!(d.doc.layer(d.active_layer.unwrap()).unwrap().name.starts_with("Tone"));
        }
    }

    #[test]
    fn a_failed_generation_reports_in_the_status_line() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let p = params(&app.ui.screentone_ui);
        crate::menus::invoke(&mut app, &ctx, photocraft_engine::screentone_cmds::LAYER, p).unwrap_err();
        assert!(app.session.active().is_none(), "no document was opened");
    }

    #[test]
    fn freeze_active_layer_bakes_screentone_keeps_mask_and_detaches_gizmo() {
        let (mut app, ctx) = app(8);
        crate::menus::invoke(&mut app, &ctx, PANEL, Value::Null).unwrap();
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let p = params(&app.ui.screentone_ui);
        let res = crate::preset_panels::run(&mut app, photocraft_engine::screentone_cmds::LAYER, p);
        crate::screentone_gizmo::bind_layer_result(&mut app, photocraft_engine::screentone_cmds::LAYER, &res);

        let id = app.session.active().unwrap().active_layer.unwrap();
        assert!(app.session.active().unwrap().doc.layer(id).unwrap().is_screentone());
        assert!(crate::screentone_gizmo::showing(&app), "gizmo shows while live screentone layer is active");

        assert!(freeze_active_layer(&mut app));
        let layer = app.session.active().unwrap().doc.layer(id).unwrap();
        assert!(!layer.is_screentone(), "frozen layer is now standard raster pixels");
        assert!(layer.mask.is_some(), "selection mask is preserved after freezing");
        assert!(!crate::screentone_gizmo::showing(&app), "gizmo hides immediately once frozen");

        // Changing panel sliders after freezing must not modify the frozen layer.
        app.ui.screentone_ui.spec.spacing_x = 32.0;
        assert!(!sync_spec_to_active_layer(&mut app));
    }
}
