//! The Fill tool (`paint.fill`), after Clip Studio Paint's: its options bar, the click, and the
//! drags of its other two sub tools, Enclose and Fill (a lasso) and Paint Unfilled Area (a brush
//! stroke), which fill every enclosed area they wholly cover.

use egui::{Color32, Pos2, Ui};
use photocraft_color::BlendMode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::{PhotocraftApp, icons, widgets};

/// The Fill tool's sub tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubTool {
    /// A click fills the area under it.
    #[default]
    Click,
    /// A lasso fills every area it encloses.
    Enclose,
    /// A brush stroke fills every area it covers: the specks a fill left behind.
    Unfilled,
}

/// The Fill tool's options (`ToolOptions::fill_tool`), as `paint.fill` takes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FillToolOptions {
    pub sub_tool: SubTool,
    /// layer | all | selected | folder | reference
    pub reference: String,
    pub exclude_layer: bool,
    pub exclude_text: bool,
    pub exclude_locked: bool,
    /// Colour margin, %.
    pub tolerance: f32,
    /// Follow adjacent pixels.
    pub contiguous: bool,
    /// px; 0 = off.
    pub close_gap: f32,
    /// px; negative shrinks.
    pub area_scaling: f32,
    /// darkest | round | square
    pub scaling_mode: String,
    /// foreground | background | transparent
    pub contents: String,
    pub mode: BlendMode,
    pub anti_alias: bool,
    pub image_border: bool,
    /// What Enclose and Fill and Paint Unfilled Area fill: transparent | whiteAndTransparent.
    pub target_color: String,
    /// Paint Unfilled Area's brush diameter, px.
    pub size: f32,
}

impl Default for FillToolOptions {
    fn default() -> Self {
        Self {
            sub_tool: SubTool::Click,
            reference: "all".into(),
            exclude_layer: false,
            exclude_text: false,
            exclude_locked: false,
            tolerance: 10.0,
            contiguous: true,
            close_gap: 3.0,
            area_scaling: 1.0,
            scaling_mode: "darkest".into(),
            contents: "foreground".into(),
            mode: BlendMode::Normal,
            anti_alias: true,
            image_border: false,
            target_color: "transparent".into(),
            size: 40.0,
        }
    }
}

/// The `paint.fill` params for the options, without the click, lasso or stroke.
pub fn params(app: &PhotocraftApp) -> Value {
    let o = &app.ui.tool_options.fill_tool;
    let exclude: Vec<&str> = [(o.exclude_layer, "layer"), (o.exclude_text, "text"), (o.exclude_locked, "locked")].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect();
    json!({
        "reference": o.reference,
        "exclude": exclude,
        "tolerance": o.tolerance,
        "contiguous": o.contiguous,
        "closeGap": o.close_gap,
        "areaScaling": o.area_scaling,
        "scalingMode": o.scaling_mode,
        "antiAlias": o.anti_alias,
        "imageBorder": o.image_border,
        "targetColor": o.target_color,
        "contents": o.contents,
        "mode": format!("{:?}", o.mode),
        "opacity": app.ui.tool_options.fill_opacity,
        "size": o.size,
        "target": crate::canvas::paint_target(app),
    })
}

fn run(app: &mut PhotocraftApp, p: Value, nothing: &str) {
    match app.run(photocraft_engine::fill_tool_cmds::FILL_ID, p) {
        Ok(r) if r["filled"] == json!(false) => {
            app.ui.status = nothing.into();
            app.ui.status_error = false;
        }
        Ok(_) => {}
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// A press with the Fill tool: the click sub tool fills there and takes the press; the others
/// leave it to the canvas, which collects their drag for [`finish`].
pub fn press(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    if app.ui.tool_options.fill_tool.sub_tool != SubTool::Click {
        return false;
    }
    let mut p = params(app);
    p["x"] = json!(x.floor());
    p["y"] = json!(y.floor());
    run(app, p, tl!("Nothing to fill there"));
    true
}

/// The end of an Enclose and Fill or Paint Unfilled Area drag over `points` (document pixels).
pub fn finish(app: &mut PhotocraftApp, points: &[[f64; 3]]) {
    let pts: Vec<[f64; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    let mut p = params(app);
    match app.ui.tool_options.fill_tool.sub_tool {
        SubTool::Click => return,
        SubTool::Enclose if pts.len() < 3 => return,
        SubTool::Enclose => p["lasso"] = json!(pts),
        SubTool::Unfilled => p["stroke"] = json!(pts),
    }
    run(app, p, tl!("No enclosed area to fill there"));
}

/// The drag in progress: the lasso's outline, or the stroke's footprint in green.
pub fn draw_drag(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, points: &[[f64; 3]]) {
    match app.ui.tool_options.fill_tool.sub_tool {
        SubTool::Click => {}
        SubTool::Enclose => {
            let pts: Vec<Pos2> = points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32).round() + egui::vec2(0.5, 0.5)).collect();
            crate::tool_feedback::draw_ants(painter, &pts, false);
        }
        SubTool::Unfilled => {
            let Some(st) = app.session.active() else { return };
            let size = [st.doc.size.width, st.doc.size.height];
            let doc_rect = xf.doc_rect(st.doc.bounds());
            let colour = stroke_colour(painter.ctx());
            let diameter = app.ui.tool_options.fill_tool.size;
            let trail = app.trail.get_or_insert_with(|| crate::stroke_trail::Trail::new(size));
            trail.feed(points, diameter);
            trail.draw(painter, doc_rect, xf.flip, colour);
        }
    }
}

/// Clip Studio paints the Paint Unfilled Area stroke green.
fn stroke_colour(ctx: &egui::Context) -> Color32 {
    crate::theme::Tokens::get(ctx).success.gamma_multiply(0.5)
}

/// The pointer: the bucket for a click, a crosshair for the lasso, the brush's circle for the
/// stroke.
pub fn cursor(app: &PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, p: Pos2, zoom: f32, precise: bool) -> egui::CursorIcon {
    match app.ui.tool_options.fill_tool.sub_tool {
        SubTool::Click if precise => egui::CursorIcon::Crosshair,
        SubTool::Click => crate::canvas::paint_bucket_cursor(ctx, p),
        SubTool::Enclose => egui::CursorIcon::Crosshair,
        SubTool::Unfilled => {
            let r = (app.ui.tool_options.fill_tool.size * zoom / 2.0).max(1.0);
            painter.circle_stroke(p, r + 1.0, egui::Stroke::new(1.0, Color32::from_black_alpha(160)));
            painter.circle_stroke(p, r, egui::Stroke::new(1.0, Color32::WHITE));
            egui::CursorIcon::Crosshair
        }
    }
}

/// The options bar: the sub tools and what decides the area up front, the rest (the colour,
/// what the fill leaves out, the canvas edge) in its gear menu, so it fits a laptop's width.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut Ui) {
    use crate::panels::opt_label;
    ui.spacing_mut().item_spacing.x = 2.0;
    let o = &mut app.ui.tool_options.fill_tool;
    for (sub, icon, tip) in [
        (SubTool::Click, "fill-tool", tl!("Fill: click an area")),
        (SubTool::Enclose, "lasso", tl!("Enclose and Fill: draw round the areas to fill")),
        (SubTool::Unfilled, "brush", tl!("Paint Unfilled Area: paint over the gaps a fill left")),
    ] {
        if icons::button(ui, icon, 24.0, o.sub_tool == sub, tip).clicked() {
            o.sub_tool = sub;
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    widgets::vline(ui, 22.0);
    opt_label(ui, tl!("Refer"));
    let refs = [
        ("layer".to_string(), tl!("Editing Layer")),
        ("all".to_string(), tl!("All Layers")),
        ("selected".to_string(), tl!("Selected Layers")),
        ("folder".to_string(), tl!("Layers in Folder")),
        ("reference".to_string(), tl!("Reference Layers")),
    ];
    widgets::dropdown(ui, "fill-tool-reference", &mut o.reference, &refs, 124.0);
    more_menu(o, ui);
    widgets::vline(ui, 22.0);
    opt_label(ui, tl!("Tolerance"));
    widgets::value_field(ui, &mut o.tolerance, 0.0..=100.0, "%", 58.0);
    opt_label(ui, tl!("Close Gap"));
    widgets::value_field(ui, &mut o.close_gap, 0.0..=100.0, "px", 58.0).on_hover_text(tl!("Treat gaps in the lines up to this wide as closed (0: off)"));
    opt_label(ui, tl!("Area Scaling"));
    widgets::value_field(ui, &mut o.area_scaling, -100.0..=100.0, "px", 58.0).on_hover_text(tl!("Grow the fill under the lines, or shrink it"));
    let modes = [("darkest".to_string(), tl!("To Darkest Pixel")), ("round".to_string(), tl!("Round")), ("square".to_string(), tl!("Square"))];
    widgets::dropdown(ui, "fill-tool-scaling", &mut o.scaling_mode, &modes, 118.0);
    widgets::vline(ui, 22.0);
    match o.sub_tool {
        SubTool::Click => {
            widgets::checkbox(ui, &mut o.contiguous, tl!("Contiguous")).on_hover_text(tl!("Follow Adjacent Pixels"));
        }
        SubTool::Enclose | SubTool::Unfilled => {
            opt_label(ui, tl!("Target"));
            let targets = [("transparent".to_string(), tl!("Transparent")), ("whiteAndTransparent".to_string(), tl!("White and Transparent"))];
            widgets::dropdown(ui, "fill-tool-target", &mut o.target_color, &targets, 150.0);
            if o.sub_tool == SubTool::Unfilled {
                opt_label(ui, tl!("Size"));
                widgets::value_field(ui, &mut o.size, 1.0..=2000.0, "px", 62.0);
            }
        }
    }
    widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias"));
    widgets::vline(ui, 22.0);
    if o.contents != "transparent" {
        let blend: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
        widgets::dropdown_wheel_hovered(ui, "fill-tool-mode", &mut o.mode, &blend, 96.0);
    }
    opt_label(ui, tl!("Opacity"));
    widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
}

/// The gear menu: the colour the fill lays down, the layers a multi-layer reference leaves out,
/// and whether the canvas edge closes gaps.
fn more_menu(o: &mut FillToolOptions, ui: &mut Ui) {
    let r = icons::button(ui, "settings", 24.0, false, tl!("Fill Tool Options"));
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(220.0);
        ui.label(tl!("Color"));
        for (value, label) in [("foreground", tl!("Foreground")), ("background", tl!("Background")), ("transparent", tl!("Transparent"))] {
            if ui.radio(o.contents == value, label).clicked() {
                o.contents = value.into();
            }
        }
        ui.separator();
        ui.label(tl!("Exclude")).on_hover_text(tl!("Layers the fill doesn't read"));
        ui.add_enabled_ui(o.reference != "layer", |ui| {
            widgets::checkbox(ui, &mut o.exclude_layer, tl!("Editing layer"));
            widgets::checkbox(ui, &mut o.exclude_text, tl!("Text layers"));
            widgets::checkbox(ui, &mut o.exclude_locked, tl!("Locked layers"));
        });
        ui.separator();
        widgets::checkbox(ui, &mut o.image_border, tl!("Image Border")).on_hover_text(tl!("The canvas edge closes gaps as a line does"));
    });
}

/// Whether `tool` is the Fill tool with a sub tool that drags.
pub fn drags(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Fill && app.ui.tool_options.fill_tool.sub_tool != SubTool::Click
}

#[cfg(test)]
mod tests;
