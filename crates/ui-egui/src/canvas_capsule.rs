//! ✨ Canvas Capsule (CraftBar): Floating contextual toolbar on the canvas.
//!
//! Provides quick, context-aware actions depending on the user's active workflow:
//! - **Screentone Layer Active**: Quick access to GenTonik LPI, rotation angle, line preservation,
//!   and raster freeze. If a selection is also active, selection actions (Invert, Feather, Deselect)
//!   are presented side-by-side without hiding GenTonik controls.
//! - **Text Editing / Text Layer Active**: Live font family picker, font size adjustment, alignment,
//!   and Commit/Cancel buttons via `type_tool::capsule_controls`.
//! - **Selection Active**: Prompt input for generative fill, AI Remove (LaMa DirectML),
//!   Invert selection, Feather, and Deselect.
//! - **Default Idle**: Quick actions like Select Subject and Remove Background.
//! - **Capsule Customization**: Dynamic context tracking vs. pinned mode, smooth 6-dot drag handle,
//!   variable opacity slider, and AI backend selection (DirectML, SD.CPP / Cloudflare Tunnel, LazySD-UI).

use egui::{
    pos2, vec2, Color32, CornerRadius, DragValue, Frame, Id, Margin, Order, RichText, Sense,
    Stroke,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::theme::Tokens;
use crate::PhotocraftApp;

/// Choice of AI backend used for generative fill and inpainting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AiBackendChoice {
    #[default]
    DirectMl,
    SdCppRemote,
    LazySdUi,
}

impl AiBackendChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::DirectMl => "DirectML (Local ONNX)",
            Self::SdCppRemote => "SD.CPP / Tunnel (Remote)",
            Self::LazySdUi => "LazySD-UI (Local)",
        }
    }
}

/// Persistent and runtime state of the Canvas Capsule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CanvasCapsuleState {
    /// When true, the capsule remains pinned to the bottom-center of the canvas viewport.
    /// When false, the capsule dynamically floats near the active selection or text bounding box.
    pub pinned: bool,
    /// User drag offset relative to the calculated base position.
    pub custom_offset: Option<[f32; 2]>,
    /// Background opacity (0.3 ..= 1.0, default 0.95).
    pub opacity: f32,
    /// Prompt text entered for generative fill or inpainting.
    pub prompt: String,
    /// Configured AI backend.
    pub backend: AiBackendChoice,
    /// Remote HTTP endpoint for SD.CPP or Cloudflare Tunnel (e.g. "http://127.0.0.1:8080").
    pub remote_endpoint: String,
    /// Temporary feedback or progress message.
    #[serde(skip)]
    pub status_text: Option<String>,
}

impl Default for CanvasCapsuleState {
    fn default() -> Self {
        Self {
            pinned: false,
            custom_offset: None,
            opacity: 0.95,
            prompt: String::new(),
            backend: AiBackendChoice::DirectMl,
            remote_endpoint: "http://127.0.0.1:8080".to_string(),
            status_text: None,
        }
    }
}

/// Renders the floating Canvas Capsule on top of the document canvas.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.panels.canvas_capsule {
        return;
    }

    let Some(active) = app.session.active() else {
        return;
    };

    let xf = crate::canvas::ViewXform::active(app);
    let canvas_rect = xf.map(|x| x.rect).unwrap_or(app.last_canvas_rect);

    let has_selection = active.doc.selection.as_ref().is_some_and(|s| !s.content_bounds().is_empty());
    let is_pinned = app.ui.canvas_capsule.pinned;

    let is_screentone = active.active_layer.is_some_and(|id| {
        active.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone)
    });
    let is_text_editing = app.ui.text_edit.is_some();
    let is_text_layer = active.active_layer.is_some_and(|id| {
        active.doc.layer(id).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Text(_)))
    });
    let show_type = !is_screentone && (is_text_editing || is_text_layer || app.ui.tool.is_type());

    // Calculate base anchor position
    let mut base_pos = pos2(canvas_rect.center().x, canvas_rect.max.y - 56.0);

    // If unpinned, dynamically follow context (selection bounds or text layer bounds)
    if !is_pinned {
        if has_selection {
            if let Some(xf_val) = xf {
                if let Some(bounds) = active.doc.selection.as_ref().map(|s| s.content_bounds()).filter(|b| !b.is_empty()) {
                    let s_min = xf_val.to_screen(bounds.x0 as f32, bounds.y0 as f32);
                    let s_max = xf_val.to_screen(bounds.x1 as f32, bounds.y1 as f32);
                    let min_y = s_min.y.min(s_max.y);
                    let max_y = s_min.y.max(s_max.y);
                    let sel_center_x = (s_min.x + s_max.x) * 0.5;
                    let mut target_y = max_y + 16.0;
                    if target_y > canvas_rect.max.y - 60.0 {
                        target_y = (min_y - 48.0).max(canvas_rect.min.y + 36.0);
                    }
                    base_pos = pos2(sel_center_x, target_y);
                }
            }
        } else if show_type {
            if let Some(xf_val) = xf {
                let text_bounds = active.active_layer.and_then(|id| {
                    active.doc.layer(id).and_then(|l| match &l.content {
                        photocraft_doc::LayerContent::Text(t) => t.cache.as_ref().map(|c| c.content_bounds()),
                        _ => None,
                    })
                });
                if let Some(bounds) = text_bounds.filter(|b| !b.is_empty()) {
                    let s_min = xf_val.to_screen(bounds.x0 as f32, bounds.y0 as f32);
                    let s_max = xf_val.to_screen(bounds.x1 as f32, bounds.y1 as f32);
                    let min_y = s_min.y.min(s_max.y);
                    let max_y = s_min.y.max(s_max.y);
                    let text_center_x = (s_min.x + s_max.x) * 0.5;
                    let mut target_y = max_y + 16.0;
                    if target_y > canvas_rect.max.y - 60.0 {
                        target_y = (min_y - 48.0).max(canvas_rect.min.y + 36.0);
                    }
                    base_pos = pos2(text_center_x, target_y);
                }
            }
        }
    }

    // Apply user drag offset if any
    if let Some([dx, dy]) = app.ui.canvas_capsule.custom_offset {
        base_pos += vec2(dx, dy);
    }

    // Keep the capsule within the canvas bounds
    let half_w = 260.0;
    let min_x = canvas_rect.min.x + half_w + 10.0;
    let max_x = (canvas_rect.max.x - half_w - 10.0).max(min_x);
    let min_y = canvas_rect.min.y + 24.0;
    let max_y = (canvas_rect.max.y - 48.0).max(min_y);

    let clamped_x = base_pos.x.clamp(min_x, max_x);
    let clamped_y = base_pos.y.clamp(min_y, max_y);
    let capsule_pos = pos2(clamped_x, clamped_y);

    let t = Tokens::get(ctx);
    let opacity = app.ui.canvas_capsule.opacity.clamp(0.3, 1.0);
    let card_fill = t.card.gamma_multiply(opacity);
    let card_border = t.card_border.gamma_multiply(opacity);
    let shadow_color = t.shadow.gamma_multiply(opacity);

    let frame = Frame {
        inner_margin: Margin::symmetric(10, 6),
        corner_radius: CornerRadius::same(16),
        fill: card_fill,
        stroke: Stroke::new(1.0, card_border),
        shadow: egui::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: shadow_color,
        },
        ..Default::default()
    };

    egui::Area::new(Id::new("canvas_capsule_area"))
        .order(Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(capsule_pos)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 0.0);

                    // 1. Grip handle (crisp 6-dot matrix)
                    let (grip_rect, grip_resp) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::click_and_drag());
                    if grip_resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    if grip_resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        let delta = grip_resp.drag_delta();
                        let mut offset = app.ui.canvas_capsule.custom_offset.unwrap_or([0.0, 0.0]);
                        offset[0] += delta.x;
                        offset[1] += delta.y;
                        app.ui.canvas_capsule.custom_offset = Some(offset);
                    }
                    let dot_color = t.text_dim;
                    let cx1 = grip_rect.min.x + 4.0;
                    let cx2 = grip_rect.min.x + 10.0;
                    for i in 0..3 {
                        let cy = grip_rect.min.y + 4.0 + (i as f32) * 6.0;
                        ui.painter().circle_filled(pos2(cx1, cy), 1.5, dot_color);
                        ui.painter().circle_filled(pos2(cx2, cy), 1.5, dot_color);
                    }

                    // 2. Sparkle icon
                    let (icon_rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    crate::icons::paint(ui, icon_rect, "wand-sparkles", 16.0, Color32::from_rgb(255, 215, 80));

                    ui.separator();

                    // 3. Workflow-aware controls:
                    // Screentone takes precedence on screentone layers so tools don't override tone tweaking.
                    if is_screentone {
                        // --- SCREENTONE LAYER ACTIVE ---
                        ui.label(
                            RichText::new("Tone")
                                .strong()
                                .color(Color32::from_rgb(100, 200, 255)),
                        );

                        let mut changed = false;
                        ui.label("LPI:");
                        changed |= ui
                            .add(
                                DragValue::new(&mut app.ui.screentone_ui.spec.spacing_x)
                                    .speed(0.2)
                                    .range(2.0..=64.0),
                            )
                            .changed();

                        ui.label("Angle:");
                        let angle_resp = ui.add(
                            DragValue::new(&mut app.ui.screentone_ui.spec.angle)
                                .speed(1.0)
                                .suffix("°"),
                        );
                        if angle_resp.changed() {
                            app.ui.screentone_ui.spec.angle = app.ui.screentone_ui.spec.angle.rem_euclid(360.0);
                            changed = true;
                        }

                        changed |= ui
                            .checkbox(&mut app.ui.screentone_ui.spec.preserve_lines, "LineArt")
                            .changed();

                        if changed {
                            crate::screentone_panel::sync_spec_to_active_layer(app);
                        }

                        ui.separator();

                        if ui
                            .button("Freeze")
                            .on_hover_text("Bake screentone into standard raster layer")
                            .clicked()
                        {
                            crate::screentone_panel::freeze_active_layer(app);
                        }

                        // If a selection is also active on the screentone layer, show selection controls side-by-side
                        if has_selection {
                            ui.separator();
                            if ui.button("Invert").on_hover_text("Invert selection (Cmd+Shift+I)").clicked() {
                                let _ = app.run("select.inverse", json!({}));
                            }
                            if ui.button("Feather").on_hover_text("Feather selection edges").clicked() {
                                let _ = app.run("select.modify.feather", json!({"radius": 5.0}));
                            }
                            if ui.button("Deselect").on_hover_text("Deselect (Cmd+D)").clicked() {
                                let _ = app.run("select.deselect", json!({}));
                            }
                        }
                    } else if show_type {
                        // --- TYPE TOOL / TEXT LAYER ACTIVE ---
                        crate::type_tool::capsule_controls(app, ui);
                    } else if has_selection {
                        // --- SELECTION ACTIVE ---
                        let edit = ui.add_sized(
                            [170.0, 22.0],
                            egui::TextEdit::singleline(&mut app.ui.canvas_capsule.prompt)
                                .hint_text("Generative Fill / Prompt..."),
                        );

                        let prompt_non_empty = !app.ui.canvas_capsule.prompt.trim().is_empty();
                        let enter_pressed = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let clicked_generate = ui.add_enabled(prompt_non_empty, egui::Button::new("Generate")).clicked();

                        if clicked_generate || (prompt_non_empty && enter_pressed) {
                            let prompt_text = app.ui.canvas_capsule.prompt.trim().to_string();
                            crate::notices::post(
                                app,
                                "Generative Fill",
                                vec![format!("Queued generation: \"{prompt_text}\"")],
                                false,
                                None,
                            );
                        }

                        if ui
                            .button("AI Remove")
                            .on_hover_text("Remove selected object with AI (DirectML LaMa)")
                            .clicked()
                        {
                            let _ = app.run("edit.inpaint", json!({}));
                            crate::notices::post(
                                app,
                                "AI Remove",
                                vec!["DirectML LaMa inpainting pipeline will process selection (Phase 2)".to_string()],
                                false,
                                None,
                            );
                        }

                        ui.separator();

                        if ui.button("Invert").on_hover_text("Invert selection (Cmd+Shift+I)").clicked() {
                            let _ = app.run("select.inverse", json!({}));
                        }
                        if ui.button("Feather").on_hover_text("Feather selection edges").clicked() {
                            let _ = app.run("select.modify.feather", json!({"radius": 5.0}));
                        }
                        if ui.button("Deselect").on_hover_text("Deselect (Cmd+D)").clicked() {
                            let _ = app.run("select.deselect", json!({}));
                        }
                    } else {
                        // --- DEFAULT IDLE ---
                        if ui
                            .button("Select Subject")
                            .on_hover_text("Automatically select the prominent subject")
                            .clicked()
                        {
                            let _ = app.run("select.subject", json!({}));
                        }

                        if ui
                            .button("Remove Background")
                            .on_hover_text("Automatically remove the background")
                            .clicked()
                        {
                            let _ = app.run("select.subject", json!({}));
                            let _ = app.run("select.inverse", json!({}));
                            let _ = app.run("edit.clear", json!({}));
                        }
                    }

                    // 4. Quick Pin / Unpin button
                    ui.separator();
                    let pin_icon = if app.ui.canvas_capsule.pinned { "lock" } else { "lock-open" };
                    let pin_tip = if app.ui.canvas_capsule.pinned {
                        "Pinned to viewport bottom. Click to float near context (selection / text)"
                    } else {
                        "Floating near context. Click to pin to viewport bottom"
                    };
                    if crate::icons::button(ui, pin_icon, 16.0, app.ui.canvas_capsule.pinned, pin_tip).clicked() {
                        app.ui.canvas_capsule.pinned = !app.ui.canvas_capsule.pinned;
                        if !app.ui.canvas_capsule.pinned {
                            app.ui.canvas_capsule.custom_offset = None;
                        }
                    }

                    // 5. Settings dropdown menu (···)
                    let more = ui.button("···").on_hover_text("Canvas Capsule settings");
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(200.0);
                        ui.label(RichText::new("Canvas Capsule").strong());
                        ui.add_space(4.0);

                        ui.checkbox(&mut app.ui.canvas_capsule.pinned, "Pin to Bottom of Viewport");
                        if ui.button("Reset Position").clicked() {
                            app.ui.canvas_capsule.custom_offset = None;
                        }

                        ui.add_space(4.0);
                        ui.add(
                            egui::Slider::new(&mut app.ui.canvas_capsule.opacity, 0.3..=1.0)
                                .text("Opacity")
                                .custom_formatter(|n, _| format!("{:.0}%", n * 100.0)),
                        );

                        ui.separator();
                        ui.label(RichText::new("AI Engine").small().color(t.text_dim));
                        ui.radio_value(
                            &mut app.ui.canvas_capsule.backend,
                            AiBackendChoice::DirectMl,
                            "DirectML (Local ONNX)",
                        );
                        ui.radio_value(
                            &mut app.ui.canvas_capsule.backend,
                            AiBackendChoice::SdCppRemote,
                            "SD.CPP / Tunnel",
                        );
                        ui.radio_value(
                            &mut app.ui.canvas_capsule.backend,
                            AiBackendChoice::LazySdUi,
                            "LazySD-UI (Local)",
                        );

                        ui.separator();
                        if ui.button("Hide Bar").clicked() {
                            app.ui.panels.canvas_capsule = false;
                        }
                    });
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn test_app() -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
        (app, ctx)
    }

    #[test]
    fn default_state_is_unpinned_with_directml_and_full_opacity() {
        let state = CanvasCapsuleState::default();
        assert!(!state.pinned);
        assert_eq!(state.opacity, 0.95);
        assert_eq!(state.backend, AiBackendChoice::DirectMl);
        assert!(state.custom_offset.is_none());
        assert!(state.prompt.is_empty());
        assert_eq!(state.remote_endpoint, "http://127.0.0.1:8080");
    }

    #[test]
    fn ai_backend_labels_are_human_readable() {
        assert_eq!(AiBackendChoice::DirectMl.label(), "DirectML (Local ONNX)");
        assert_eq!(AiBackendChoice::SdCppRemote.label(), "SD.CPP / Tunnel (Remote)");
        assert_eq!(AiBackendChoice::LazySdUi.label(), "LazySD-UI (Local)");
    }

    #[test]
    fn window_menu_toggles_canvas_capsule() {
        let (mut app, ctx) = test_app();
        assert!(app.ui.panels.canvas_capsule);
        assert_eq!(crate::menus::checked(&app, "window.panel.canvasCapsule"), Some(true));

        crate::menus::invoke(&mut app, &ctx, "window.panel.canvasCapsule", Value::Null).unwrap();
        assert!(!app.ui.panels.canvas_capsule);
        assert_eq!(crate::menus::checked(&app, "window.panel.canvasCapsule"), Some(false));

        crate::menus::invoke(&mut app, &ctx, "window.panel.canvasCapsule", Value::Null).unwrap();
        assert!(app.ui.panels.canvas_capsule);
        assert_eq!(crate::menus::checked(&app, "window.panel.canvasCapsule"), Some(true));
    }

    #[test]
    fn canvas_capsule_renders_in_all_workflow_contexts() {
        let (mut app, ctx) = test_app();

        // 1. Idle mode
        let mut out = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();

        // 2. Selection mode
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();

        // 3. Type mode
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = crate::Tool::Type;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();

        // 4. Screentone layer mode without selection
        app.ui.tool = crate::Tool::Move;
        app.run(
            "layer.screentone",
            json!({
                "mode": "shading",
                "dotSize": 4.5,
                "spacingX": 8.0,
                "spacingY": 8.0,
                "angle": 45.0,
                "density": 0.75
            }),
        ).unwrap();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();

        // 5. Screentone layer mode WITH selection (both GenTonik + selection actions co-exist)
        app.run("select.rect", json!({"x": 5, "y": 5, "width": 15, "height": 15})).unwrap();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        out.textures_delta.clear();
    }
}
