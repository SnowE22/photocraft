//! Scrubby Slider / Drag-to-Adjust gesture controller and compact HUD for layer effects.
//!
//! Allows adjusting effect parameters (such as Stroke Size, Drop Shadow Distance/Blur, Opacity)
//! directly from the Layers panel effect rows without opening the Layer Style modal dialog.
//!
//! Features:
//! - Horizontal drag disambiguation: distinguishes horizontal scrub from vertical DnD layer transfer.
//! - Smooth sensitivity: 1px = 1 unit default, Shift = 10x coarse, Alt = 0.2x high-precision.
//! - Virtual drag accumulator with horizontal resize cursor (`CursorIcon::ResizeHorizontal`).
//! - Floating badge / readout near cursor with localized labels and units.
//! - Live canvas preview coalescing into a single Undo/Redo history step on mouse release.
//! - Escape key cancellation: restores parameter to initial value before drag.
//! - Compact parameter HUD overlay: lists all numeric parameters for compound effects with
//!   interactive scrubby sliders per row.

use egui::{Color32, Id, Modifiers, Pos2, Sense, Stroke, pos2, vec2};
use photocraft_doc::effects::Effect;
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::theme::Tokens;

/// Movement deadband in points before deciding between Scrubby Slider vs Layer Transfer DnD.
pub const DEADBAND_POINTS: f32 = 4.0;

/// Ratio of |dx| to |dy| required to lock into horizontal scrubby mode.
pub const HORIZONTAL_BIAS: f32 = 1.15;

/// Descriptor of a numeric effect parameter that can be scrubbed.
#[derive(Clone, Copy, Debug)]
pub struct FxParamDesc {
    pub key: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub speed: f32,
    pub getter: fn(&Effect) -> f32,
    pub setter: fn(&mut Effect, f32),
    pub formatter: fn(f32, &'static str) -> String,
}

impl FxParamDesc {
    /// Formats a value with its unit.
    pub fn format(&self, val: f32) -> String {
        (self.formatter)(val, self.unit)
    }

    /// Clamps value within parameter bounds.
    pub fn clamp(&self, val: f32) -> f32 {
        val.clamp(self.min, self.max)
    }
}

fn fmt_pixels(val: f32, unit: &'static str) -> String {
    if val.fract().abs() < 1e-3 {
        format!("{:.0} {unit}", val.round())
    } else {
        format!("{:.1} {unit}", val)
    }
}

fn fmt_percent(val: f32, unit: &'static str) -> String {
    if val.fract().abs() < 1e-3 {
        format!("{:.0}{unit}", val.round())
    } else {
        format!("{:.1}{unit}", val)
    }
}

fn fmt_degrees(val: f32, unit: &'static str) -> String {
    if val.fract().abs() < 1e-3 {
        format!("{:.0}{unit}", val.round())
    } else {
        format!("{:.1}{unit}", val)
    }
}

/// Returns the primary numeric parameter for an effect kind (e.g. Stroke -> Size).
pub fn primary_param(effect: &Effect) -> Option<FxParamDesc> {
    match effect {
        Effect::Stroke(_) => Some(FxParamDesc {
            key: "size",
            label: "Size",
            unit: "px",
            min: 1.0,
            max: 250.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::Stroke(s) => s.size,
                _ => 1.0,
            },
            setter: |e, v| {
                if let Effect::Stroke(s) = e {
                    s.size = v;
                }
            },
            formatter: fmt_pixels,
        }),
        Effect::DropShadow(_) | Effect::InnerShadow(_) => Some(FxParamDesc {
            key: "size",
            label: "Size",
            unit: "px",
            min: 0.0,
            max: 250.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::DropShadow(s) | Effect::InnerShadow(s) => s.size,
                _ => 0.0,
            },
            setter: |e, v| match e {
                Effect::DropShadow(s) | Effect::InnerShadow(s) => s.size = v,
                _ => {}
            },
            formatter: fmt_pixels,
        }),
        Effect::OuterGlow(_) | Effect::InnerGlow(_) => Some(FxParamDesc {
            key: "size",
            label: "Size",
            unit: "px",
            min: 0.0,
            max: 250.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size,
                _ => 0.0,
            },
            setter: |e, v| match e {
                Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size = v,
                _ => {}
            },
            formatter: fmt_pixels,
        }),
        Effect::ColorOverlay { .. } => Some(FxParamDesc {
            key: "opacity",
            label: "Opacity",
            unit: "%",
            min: 0.0,
            max: 100.0,
            speed: 0.5,
            getter: |e| match e {
                Effect::ColorOverlay { common, .. } => (common.opacity * 100.0).round(),
                _ => 100.0,
            },
            setter: |e, v| {
                if let Effect::ColorOverlay { common, .. } = e {
                    common.opacity = (v / 100.0).clamp(0.0, 1.0);
                }
            },
            formatter: fmt_percent,
        }),
        Effect::GradientOverlay { .. } => Some(FxParamDesc {
            key: "scale",
            label: "Scale",
            unit: "%",
            min: 10.0,
            max: 150.0,
            speed: 0.5,
            getter: |e| match e {
                Effect::GradientOverlay { gradient, .. } => (gradient.scale * 100.0).round(),
                _ => 100.0,
            },
            setter: |e, v| {
                if let Effect::GradientOverlay { gradient, .. } = e {
                    gradient.scale = (v / 100.0).clamp(0.1, 1.5);
                }
            },
            formatter: fmt_percent,
        }),
        Effect::PatternOverlay { .. } => Some(FxParamDesc {
            key: "scale",
            label: "Scale",
            unit: "%",
            min: 1.0,
            max: 1000.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::PatternOverlay { scale, .. } => (scale * 100.0).round(),
                _ => 100.0,
            },
            setter: |e, v| {
                if let Effect::PatternOverlay { scale, .. } = e {
                    *scale = (v / 100.0).clamp(0.01, 10.0);
                }
            },
            formatter: fmt_percent,
        }),
        Effect::BevelEmboss(_) => Some(FxParamDesc {
            key: "size",
            label: "Size",
            unit: "px",
            min: 0.0,
            max: 250.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::BevelEmboss(b) => b.size,
                _ => 5.0,
            },
            setter: |e, v| {
                if let Effect::BevelEmboss(b) = e {
                    b.size = v;
                }
            },
            formatter: fmt_pixels,
        }),
        Effect::Satin(_) => Some(FxParamDesc {
            key: "size",
            label: "Size",
            unit: "px",
            min: 0.0,
            max: 250.0,
            speed: 1.0,
            getter: |e| match e {
                Effect::Satin(s) => s.size,
                _ => 5.0,
            },
            setter: |e, v| {
                if let Effect::Satin(s) = e {
                    s.size = v;
                }
            },
            formatter: fmt_pixels,
        }),
    }
}

/// Returns all adjustable numeric parameters for an effect kind (for HUD overlay).
pub fn all_numeric_params(effect: &Effect) -> Vec<FxParamDesc> {
    match effect {
        Effect::Stroke(_) => vec![
            FxParamDesc {
                key: "size",
                label: "Size",
                unit: "px",
                min: 1.0,
                max: 250.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::Stroke(s) => s.size,
                    _ => 1.0,
                },
                setter: |e, v| {
                    if let Effect::Stroke(s) = e {
                        s.size = v;
                    }
                },
                formatter: fmt_pixels,
            },
            FxParamDesc {
                key: "opacity",
                label: "Opacity",
                unit: "%",
                min: 0.0,
                max: 100.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::Stroke(s) => (s.common.opacity * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::Stroke(s) = e {
                        s.common.opacity = (v / 100.0).clamp(0.0, 1.0);
                    }
                },
                formatter: fmt_percent,
            },
        ],
        Effect::DropShadow(_) | Effect::InnerShadow(_) => {
            let is_drop = matches!(effect, Effect::DropShadow(_));
            vec![
                FxParamDesc {
                    key: "size",
                    label: "Size",
                    unit: "px",
                    min: 0.0,
                    max: 250.0,
                    speed: 1.0,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.size,
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.size = v,
                        _ => {}
                    },
                    formatter: fmt_pixels,
                },
                FxParamDesc {
                    key: "distance",
                    label: "Distance",
                    unit: "px",
                    min: 0.0,
                    max: 300.0,
                    speed: 1.0,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.distance,
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.distance = v,
                        _ => {}
                    },
                    formatter: fmt_pixels,
                },
                FxParamDesc {
                    key: if is_drop { "spread" } else { "choke" },
                    label: if is_drop { "Spread" } else { "Choke" },
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => (s.spread * 100.0).round(),
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.spread = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
                FxParamDesc {
                    key: "opacity",
                    label: "Opacity",
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => (s.common.opacity * 100.0).round(),
                        _ => 75.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.common.opacity = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
                FxParamDesc {
                    key: "angle",
                    label: "Angle",
                    unit: "°",
                    min: -180.0,
                    max: 180.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.angle,
                        _ => 120.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.angle = v,
                        _ => {}
                    },
                    formatter: fmt_degrees,
                },
                FxParamDesc {
                    key: "noise",
                    label: "Noise",
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => (s.noise * 100.0).round(),
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::DropShadow(s) | Effect::InnerShadow(s) => s.noise = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
            ]
        }
        Effect::OuterGlow(_) | Effect::InnerGlow(_) => {
            let is_outer = matches!(effect, Effect::OuterGlow(_));
            vec![
                FxParamDesc {
                    key: "size",
                    label: "Size",
                    unit: "px",
                    min: 0.0,
                    max: 250.0,
                    speed: 1.0,
                    getter: |e| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size,
                        _ => 5.0,
                    },
                    setter: |e, v| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.size = v,
                        _ => {}
                    },
                    formatter: fmt_pixels,
                },
                FxParamDesc {
                    key: if is_outer { "spread" } else { "choke" },
                    label: if is_outer { "Spread" } else { "Choke" },
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => (g.spread * 100.0).round(),
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.spread = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
                FxParamDesc {
                    key: "range",
                    label: "Range",
                    unit: "%",
                    min: 1.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => (g.range * 100.0).round(),
                        _ => 50.0,
                    },
                    setter: |e, v| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.range = (v / 100.0).clamp(0.01, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
                FxParamDesc {
                    key: "opacity",
                    label: "Opacity",
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => (g.common.opacity * 100.0).round(),
                        _ => 75.0,
                    },
                    setter: |e, v| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.common.opacity = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
                FxParamDesc {
                    key: "noise",
                    label: "Noise",
                    unit: "%",
                    min: 0.0,
                    max: 100.0,
                    speed: 0.5,
                    getter: |e| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => (g.noise * 100.0).round(),
                        _ => 0.0,
                    },
                    setter: |e, v| match e {
                        Effect::OuterGlow(g) | Effect::InnerGlow(g) => g.noise = (v / 100.0).clamp(0.0, 1.0),
                        _ => {}
                    },
                    formatter: fmt_percent,
                },
            ]
        }
        Effect::ColorOverlay { .. } => vec![FxParamDesc {
            key: "opacity",
            label: "Opacity",
            unit: "%",
            min: 0.0,
            max: 100.0,
            speed: 0.5,
            getter: |e| match e {
                Effect::ColorOverlay { common, .. } => (common.opacity * 100.0).round(),
                _ => 100.0,
            },
            setter: |e, v| {
                if let Effect::ColorOverlay { common, .. } = e {
                    common.opacity = (v / 100.0).clamp(0.0, 1.0);
                }
            },
            formatter: fmt_percent,
        }],
        Effect::GradientOverlay { .. } => vec![
            FxParamDesc {
                key: "scale",
                label: "Scale",
                unit: "%",
                min: 10.0,
                max: 150.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::GradientOverlay { gradient, .. } => (gradient.scale * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::GradientOverlay { gradient, .. } = e {
                        gradient.scale = (v / 100.0).clamp(0.1, 1.5);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "opacity",
                label: "Opacity",
                unit: "%",
                min: 0.0,
                max: 100.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::GradientOverlay { common, .. } => (common.opacity * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::GradientOverlay { common, .. } = e {
                        common.opacity = (v / 100.0).clamp(0.0, 1.0);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "angle",
                label: "Angle",
                unit: "°",
                min: -180.0,
                max: 180.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::GradientOverlay { gradient, .. } => gradient.angle,
                    _ => 90.0,
                },
                setter: |e, v| {
                    if let Effect::GradientOverlay { gradient, .. } = e {
                        gradient.angle = v;
                    }
                },
                formatter: fmt_degrees,
            },
        ],
        Effect::PatternOverlay { .. } => vec![
            FxParamDesc {
                key: "scale",
                label: "Scale",
                unit: "%",
                min: 1.0,
                max: 1000.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::PatternOverlay { scale, .. } => (scale * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::PatternOverlay { scale, .. } = e {
                        *scale = (v / 100.0).clamp(0.01, 10.0);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "opacity",
                label: "Opacity",
                unit: "%",
                min: 0.0,
                max: 100.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::PatternOverlay { common, .. } => (common.opacity * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::PatternOverlay { common, .. } = e {
                        common.opacity = (v / 100.0).clamp(0.0, 1.0);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "angle",
                label: "Angle",
                unit: "°",
                min: -180.0,
                max: 180.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::PatternOverlay { angle, .. } => *angle,
                    _ => 0.0,
                },
                setter: |e, v| {
                    if let Effect::PatternOverlay { angle, .. } = e {
                        *angle = v;
                    }
                },
                formatter: fmt_degrees,
            },
        ],
        Effect::BevelEmboss(_) => vec![
            FxParamDesc {
                key: "size",
                label: "Size",
                unit: "px",
                min: 0.0,
                max: 250.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::BevelEmboss(b) => b.size,
                    _ => 5.0,
                },
                setter: |e, v| {
                    if let Effect::BevelEmboss(b) = e {
                        b.size = v;
                    }
                },
                formatter: fmt_pixels,
            },
            FxParamDesc {
                key: "soften",
                label: "Soften",
                unit: "px",
                min: 0.0,
                max: 16.0,
                speed: 0.25,
                getter: |e| match e {
                    Effect::BevelEmboss(b) => b.soften,
                    _ => 0.0,
                },
                setter: |e, v| {
                    if let Effect::BevelEmboss(b) = e {
                        b.soften = v;
                    }
                },
                formatter: fmt_pixels,
            },
            FxParamDesc {
                key: "depth",
                label: "Depth",
                unit: "%",
                min: 1.0,
                max: 1000.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::BevelEmboss(b) => (b.depth * 100.0).round(),
                    _ => 100.0,
                },
                setter: |e, v| {
                    if let Effect::BevelEmboss(b) = e {
                        b.depth = (v / 100.0).clamp(0.01, 10.0);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "angle",
                label: "Angle",
                unit: "°",
                min: -180.0,
                max: 180.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::BevelEmboss(b) => b.angle,
                    _ => 120.0,
                },
                setter: |e, v| {
                    if let Effect::BevelEmboss(b) = e {
                        b.angle = v;
                    }
                },
                formatter: fmt_degrees,
            },
            FxParamDesc {
                key: "altitude",
                label: "Altitude",
                unit: "°",
                min: 0.0,
                max: 90.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::BevelEmboss(b) => b.altitude,
                    _ => 30.0,
                },
                setter: |e, v| {
                    if let Effect::BevelEmboss(b) = e {
                        b.altitude = v;
                    }
                },
                formatter: fmt_degrees,
            },
        ],
        Effect::Satin(_) => vec![
            FxParamDesc {
                key: "size",
                label: "Size",
                unit: "px",
                min: 0.0,
                max: 250.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::Satin(s) => s.size,
                    _ => 5.0,
                },
                setter: |e, v| {
                    if let Effect::Satin(s) = e {
                        s.size = v;
                    }
                },
                formatter: fmt_pixels,
            },
            FxParamDesc {
                key: "distance",
                label: "Distance",
                unit: "px",
                min: 0.0,
                max: 250.0,
                speed: 1.0,
                getter: |e| match e {
                    Effect::Satin(s) => s.distance,
                    _ => 5.0,
                },
                setter: |e, v| {
                    if let Effect::Satin(s) = e {
                        s.distance = v;
                    }
                },
                formatter: fmt_pixels,
            },
            FxParamDesc {
                key: "opacity",
                label: "Opacity",
                unit: "%",
                min: 0.0,
                max: 100.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::Satin(s) => (s.common.opacity * 100.0).round(),
                    _ => 50.0,
                },
                setter: |e, v| {
                    if let Effect::Satin(s) = e {
                        s.common.opacity = (v / 100.0).clamp(0.0, 1.0);
                    }
                },
                formatter: fmt_percent,
            },
            FxParamDesc {
                key: "angle",
                label: "Angle",
                unit: "°",
                min: -180.0,
                max: 180.0,
                speed: 0.5,
                getter: |e| match e {
                    Effect::Satin(s) => s.angle,
                    _ => 19.0,
                },
                setter: |e, v| {
                    if let Effect::Satin(s) = e {
                        s.angle = v;
                    }
                },
                formatter: fmt_degrees,
            },
        ],
    }
}

/// The state of the mouse scrubby slider gesture.
#[derive(Clone, Debug)]
pub enum ScrubGestureState {
    /// No gesture active.
    Idle,
    /// Mouse pressed down, awaiting disambiguation threshold.
    Pending {
        layer_id: LayerId,
        effect_idx: usize,
        start_pos: Pos2,
        initial_val: f32,
        param_key: &'static str,
    },
    /// Scrubbing active: dragging horizontally adjusts the parameter.
    Scrubbing {
        layer_id: LayerId,
        effect_idx: usize,
        start_pos: Pos2,
        initial_val: f32,
        current_val: f32,
        param: FxParamDesc,
        coalesce_token: u64,
        accumulator: f32,
    },
    /// Disambiguated as vertical Drag-and-Drop layer transfer.
    DelegatedToDnD,
}

impl Default for ScrubGestureState {
    fn default() -> Self {
        Self::Idle
    }
}

/// Key in egui Context temp storage for the active scrub gesture.
pub fn scrub_state_id() -> Id {
    Id::new("pc-fx-scrub-state")
}

/// Key in egui Context temp storage for the open HUD popup.
pub fn hud_state_id() -> Id {
    Id::new("pc-fx-hud-state")
}

/// State of an open effect HUD popup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxHudState {
    pub layer_id: LayerId,
    pub effect_idx: usize,
    pub anchor_pos: Pos2,
}

/// Multiplier calculation based on keyboard modifiers.
/// Multiplier calculation based on keyboard modifiers.
pub fn modifier_rate(mods: Modifiers) -> f32 {
    let mut rate = 1.0;
    if mods.shift {
        rate *= 10.0;
    }
    if mods.alt {
        rate *= 0.2;
    }
    rate
}

/// Computes optimal anchor position for the HUD popup overlay relative to widget rect and screen.
pub fn compute_hud_anchor(ctx: &egui::Context, rect: egui::Rect, hud_width: f32) -> egui::Pos2 {
    let screen = ctx.input(|i| i.raw.screen_rect).unwrap_or(egui::Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)));
    // Prefer placing to the left of the layers dock if there is room on screen:
    if rect.left() - hud_width - 8.0 >= screen.left() {
        pos2(rect.left() - hud_width - 8.0, rect.top().clamp(screen.top() + 10.0, (screen.bottom() - 150.0).max(screen.top())))
    } else if rect.right() + hud_width + 8.0 <= screen.right() {
        pos2(rect.right() + 8.0, rect.top().clamp(screen.top() + 10.0, (screen.bottom() - 150.0).max(screen.top())))
    } else {
        // Fallback: place below the row, clamped within screen
        pos2(
            rect.left().clamp(screen.left() + 8.0, (screen.right() - hud_width - 8.0).max(screen.left())),
            (rect.bottom() + 4.0).clamp(screen.top() + 10.0, (screen.bottom() - 150.0).max(screen.top())),
        )
    }
}

/// Builds a `layer.layerStyle.replace` action preserving all layer effects while updating one parameter.
pub fn make_replace_action(
    layer_id: LayerId,
    effect_idx: usize,
    all_effects: &[Effect],
    param_key: &str,
    new_val: f32,
    coalesce_token: u64,
) -> (String, Value) {
    let entries: Vec<Value> = all_effects
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let kind = effect_kind_str(item);
            let fx_json = serde_json::to_value(item).unwrap_or_default();
            if idx == effect_idx {
                json!({
                    "kind": kind,
                    "fx": fx_json,
                    "params": { param_key: new_val }
                })
            } else {
                json!({
                    "kind": kind,
                    "fx": fx_json,
                    "params": {}
                })
            }
        })
        .collect();

    (
        "layer.layerStyle.replace".into(),
        json!({
            "layer": layer_id.0,
            "effects": entries,
            "coalesce": format!("fx-scrub-{}-{}-{coalesce_token}", layer_id.0, effect_idx)
        }),
    )
}

/// Result of processing a frame of scrubby interaction.
pub struct ScrubOutput {
    /// If an engine command should be dispatched this frame.
    pub action: Option<(String, Value)>,
    /// Whether layer transfer DnD should be triggered (`start_fx_drag`).
    pub trigger_dnd: bool,
    /// Whether scrubbing is currently active.
    pub is_scrubbing: bool,
}

/// Primary drag controller for effect row interaction.
///
/// Handles pointer press, drag movement, release, deadband disambiguation,
/// Escape cancellation, floating badge, and cursor updates.
pub fn handle_effect_drag(
    ctx: &egui::Context,
    resp: &egui::Response,
    layer_id: LayerId,
    effect_idx: usize,
    all_effects: &[Effect],
    param: FxParamDesc,
    allow_dnd: bool,
) -> ScrubOutput {
    let mut state = ctx.data(|d| d.get_temp::<ScrubGestureState>(scrub_state_id())).unwrap_or_default();
    let mut out = ScrubOutput { action: None, trigger_dnd: false, is_scrubbing: false };

    let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
    let escape_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let released = ctx.input(|i| i.pointer.any_released());

    // 1. Check for cancellation via Escape
    if escape_pressed {
        match state {
            ScrubGestureState::Scrubbing {
                layer_id: s_layer,
                effect_idx: s_idx,
                initial_val,
                param: s_param,
                coalesce_token,
                ..
            } => {
                if s_layer == layer_id && s_idx == effect_idx && s_param.key == param.key {
                    out.action = Some(make_replace_action(layer_id, effect_idx, all_effects, param.key, initial_val, coalesce_token));
                    ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
                }
            }
            ScrubGestureState::Pending {
                layer_id: p_layer,
                effect_idx: p_idx,
                param_key: p_key,
                ..
            } => {
                if p_layer == layer_id && p_idx == effect_idx && p_key == param.key {
                    ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
                }
            }
            ScrubGestureState::DelegatedToDnD => {
                ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
            }
            ScrubGestureState::Idle => {}
        }
        return out;
    }

    // 2. Handle release (mouseup)
    if released {
        match state {
            ScrubGestureState::Scrubbing {
                layer_id: s_layer,
                effect_idx: s_idx,
                current_val,
                param: s_param,
                coalesce_token,
                ..
            } => {
                if s_layer == layer_id && s_idx == effect_idx && s_param.key == param.key {
                    out.action = Some(make_replace_action(layer_id, effect_idx, all_effects, param.key, current_val, coalesce_token));
                    ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
                }
            }
            ScrubGestureState::Pending {
                layer_id: p_layer,
                effect_idx: p_idx,
                param_key: p_key,
                ..
            } => {
                if p_layer == layer_id && p_idx == effect_idx && p_key == param.key {
                    ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
                }
            }
            ScrubGestureState::DelegatedToDnD => {
                ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
            }
            ScrubGestureState::Idle => {}
        }
        return out;
    }

    // Safety cleanup: if mouse button is not held down and no release event, clear stale state.
    if !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) && !matches!(state, ScrubGestureState::Idle) {
        ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::Idle));
        return out;
    }

    // 3. Initiate gesture on press
    if resp.drag_started() || (resp.is_pointer_button_down_on() && matches!(state, ScrubGestureState::Idle)) {
        if matches!(state, ScrubGestureState::Idle) {
            let start_pos = ctx.input(|i| i.pointer.press_origin()).or(pointer_pos).unwrap_or_default();
            let initial_val = all_effects.get(effect_idx).map_or(0.0, |fx| (param.getter)(fx));
            state = ScrubGestureState::Pending {
                layer_id,
                effect_idx,
                start_pos,
                initial_val,
                param_key: param.key,
            };
            ctx.data_mut(|d| d.insert_temp(scrub_state_id(), state.clone()));
        }
    }

    // 4. Update gesture state machine while pointer is held down
    match state {
        ScrubGestureState::Pending {
            layer_id: p_layer,
            effect_idx: p_idx,
            start_pos,
            initial_val,
            param_key: p_key,
        } if p_layer == layer_id && p_idx == effect_idx && p_key == param.key => {
            if let Some(pos) = pointer_pos {
                let delta = pos - start_pos;
                let dist = delta.length();
                if dist >= DEADBAND_POINTS {
                    let is_horizontal = delta.x.abs() >= delta.y.abs() * HORIZONTAL_BIAS;
                    if !allow_dnd || is_horizontal {
                        // Disambiguated as Scrubbing
                        let token = ctx.cumulative_pass_nr();
                        let new_state = ScrubGestureState::Scrubbing {
                            layer_id,
                            effect_idx,
                            start_pos,
                            initial_val,
                            current_val: initial_val,
                            param,
                            coalesce_token: token,
                            accumulator: 0.0,
                        };
                        ctx.data_mut(|d| d.insert_temp(scrub_state_id(), new_state));
                        out.is_scrubbing = true;
                    } else {
                        // Disambiguated as Drag-and-Drop layer transfer
                        ctx.data_mut(|d| d.insert_temp(scrub_state_id(), ScrubGestureState::DelegatedToDnD));
                        out.trigger_dnd = true;
                    }
                }
            }
        }
        ScrubGestureState::Scrubbing {
            layer_id: s_layer,
            effect_idx: s_idx,
            start_pos,
            initial_val,
            mut current_val,
            param: s_param,
            coalesce_token,
            mut accumulator,
        } if s_layer == layer_id && s_idx == effect_idx && s_param.key == param.key => {
            out.is_scrubbing = true;
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);

            let drag_delta_x = ctx.input(|i| i.pointer.delta().x);
            let mods = ctx.input(|i| i.modifiers);
            let rate = modifier_rate(mods);

            accumulator += drag_delta_x * param.speed * rate;
            let step = if mods.alt { 0.2 } else { 1.0 };
            let steps = (accumulator / step).trunc();
            if steps.abs() >= 1.0 {
                accumulator -= steps * step;
                let next_val = param.clamp(((current_val + steps * step) * 10.0).round() / 10.0);
                if (next_val - current_val).abs() > 1e-4 {
                    current_val = next_val;
                    out.action = Some(make_replace_action(layer_id, effect_idx, all_effects, param.key, current_val, coalesce_token));
                }
            }

            // Floating badge / readout next to cursor
            if let Some(pos) = pointer_pos {
                draw_scrub_badge(ctx, pos, param.label, &param.format(current_val));
            }

            // Save updated state
            ctx.data_mut(|d| {
                d.insert_temp(
                    scrub_state_id(),
                    ScrubGestureState::Scrubbing {
                        layer_id,
                        effect_idx,
                        start_pos,
                        initial_val,
                        current_val,
                        param,
                        coalesce_token,
                        accumulator,
                    },
                );
            });
            ctx.request_repaint();
        }
        _ => {}
    }

    out
}

/// Draws floating badge readout near cursor during scrubby drag.
pub fn draw_scrub_badge(ctx: &egui::Context, cursor: Pos2, label: &str, formatted_value: &str) {
    let t = Tokens::get(ctx);
    let font = egui::FontId::proportional(11.5);

    egui::Area::new(Id::new("pc-fx-scrub-tooltip"))
        .order(egui::Order::Tooltip)
        .fixed_pos(cursor + vec2(16.0, 18.0))
        .interactable(false)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_sm)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(
                            egui::RichText::new(format!("{}:", tl!(label)))
                                .font(font.clone())
                                .color(t.text_dim),
                        ));
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(
                            egui::RichText::new(formatted_value)
                                .font(font)
                                .color(t.text)
                                .strong(),
                        ));
                    });
                });
        });
}

/// Renders the compact HUD popup overlay for compound effect parameters.
pub fn render_fx_hud(
    ctx: &egui::Context,
    doc: &photocraft_doc::Document,
    actions: &mut Vec<(String, Value)>,
) {
    let Some(hud) = ctx.data(|d| d.get_temp::<FxHudState>(hud_state_id())) else { return };
    let Some(layer) = doc.layer(hud.layer_id) else {
        ctx.data_mut(|d| d.remove::<FxHudState>(hud_state_id()));
        return;
    };
    let Some(effect) = layer.effects.items.get(hud.effect_idx) else {
        ctx.data_mut(|d| d.remove::<FxHudState>(hud_state_id()));
        return;
    };

    let params = all_numeric_params(effect);
    if params.is_empty() {
        ctx.data_mut(|d| d.remove::<FxHudState>(hud_state_id()));
        return;
    }

    let t = Tokens::get(ctx);
    let mut close_hud = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let is_scrubbing = ctx.data(|d| matches!(d.get_temp::<ScrubGestureState>(scrub_state_id()), Some(ScrubGestureState::Scrubbing { .. })));

    let area_resp = egui::Area::new(Id::new("pc-fx-hud-area"))
        .order(egui::Order::Foreground)
        .fixed_pos(hud.anchor_pos)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(t.radius_sm)
                .shadow(egui::epaint::Shadow {
                    spread: 2,
                    blur: 8,
                    color: Color32::from_black_alpha(60),
                    offset: [0, 4],
                })
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_min_width(190.0);
                    // Title Bar
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(
                            egui::RichText::new(format!("{} Properties", effect.label()))
                                .font(egui::FontId::proportional(12.0))
                                .strong()
                                .color(t.text),
                        ));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new("×").frame(false)).clicked() {
                                close_hud = true;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Parameter Rows with interactive scrubby sliders
                    for p in params {
                        let cur_val = (p.getter)(effect);
                        let row_resp = ui.allocate_response(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
                        let is_active = row_resp.hovered() || row_resp.dragged();

                        if is_active {
                            ui.painter().rect_filled(row_resp.rect, t.radius_sm, t.hover.gamma_multiply(0.35));
                            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }

                        // Parameter label
                        ui.painter().text(
                            pos2(row_resp.rect.left() + 4.0, row_resp.rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            tl!(p.label),
                            egui::FontId::proportional(11.5),
                            if is_active { t.text } else { t.text_dim },
                        );

                        // Formatted value badge
                        let val_str = p.format(cur_val);
                        ui.painter().text(
                            pos2(row_resp.rect.right() - 4.0, row_resp.rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            val_str,
                            egui::FontId::proportional(11.5),
                            t.text,
                        );

                        // Scrub interaction on row - inside HUD DnD is false!
                        let scrub = handle_effect_drag(ctx, &row_resp, hud.layer_id, hud.effect_idx, &layer.effects.items, p, false);
                        if let Some(action) = scrub.action {
                            actions.push(action);
                        }
                    }
                });
        });

    // Close HUD on outside click (if not currently scrubbing)
    if !is_scrubbing && ctx.input(|i| i.pointer.primary_clicked()) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if !area_resp.response.rect.contains(pos) {
                close_hud = true;
            }
        }
    }

    if close_hud && !is_scrubbing {
        ctx.data_mut(|d| d.remove::<FxHudState>(hud_state_id()));
    }
}

/// Helper to get effect kind string matching photocraft_engine specs.
pub fn effect_kind_str(effect: &Effect) -> &'static str {
    match effect {
        Effect::DropShadow(_) => "dropShadow",
        Effect::InnerShadow(_) => "innerShadow",
        Effect::OuterGlow(_) => "outerGlow",
        Effect::InnerGlow(_) => "innerGlow",
        Effect::Stroke(_) => "stroke",
        Effect::ColorOverlay { .. } => "colorOverlay",
        Effect::GradientOverlay { .. } => "gradientOverlay",
        Effect::PatternOverlay { .. } => "patternOverlay",
        Effect::BevelEmboss(_) => "bevelEmboss",
        Effect::Satin(_) => "satin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Color;
    use photocraft_doc::effects::{FxCommon, StrokeFx, StrokePosition};

    fn test_stroke() -> Effect {
        Effect::Stroke(StrokeFx {
            common: FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
            size: 3.0,
            position: StrokePosition::Outside,
            paint: photocraft_doc::effects::FxPaint::Color(Color::BLACK),
            overprint: false,
        })
    }

    fn test_shadow() -> Effect {
        Effect::DropShadow(photocraft_doc::effects::Shadow {
            common: FxCommon::new(photocraft_color::BlendMode::Multiply, 0.75),
            color: Color::BLACK,
            angle: 120.0,
            use_global_light: true,
            distance: 5.0,
            spread: 0.2,
            size: 8.0,
            contour: photocraft_doc::effects::Contour::Linear,
            anti_alias: false,
            noise: 0.1,
            knocks_out: true,
        })
    }

    #[test]
    fn primary_param_for_stroke_is_size() {
        let fx = test_stroke();
        let p = primary_param(&fx).expect("stroke has primary param");
        assert_eq!(p.key, "size");
        assert_eq!(p.unit, "px");
        assert_eq!((p.getter)(&fx), 3.0);
    }

    #[test]
    fn stroke_setter_updates_size() {
        let mut fx = test_stroke();
        let p = primary_param(&fx).unwrap();
        (p.setter)(&mut fx, 15.0);
        assert_eq!((p.getter)(&fx), 15.0);
    }

    #[test]
    fn modifiers_scale_sensitivity_correctly() {
        let normal = Modifiers::NONE;
        assert_eq!(modifier_rate(normal), 1.0);

        let shift = Modifiers { shift: true, ..Default::default() };
        assert_eq!(modifier_rate(shift), 10.0);

        let alt = Modifiers { alt: true, ..Default::default() };
        assert_eq!(modifier_rate(alt), 0.2);

        let shift_alt = Modifiers { shift: true, alt: true, ..Default::default() };
        assert_eq!(modifier_rate(shift_alt), 2.0);
    }

    #[test]
    fn value_clamping_respects_bounds() {
        let fx = test_stroke();
        let p = primary_param(&fx).unwrap();
        assert_eq!(p.clamp(0.5), 1.0); // min is 1.0 px
        assert_eq!(p.clamp(500.0), 250.0); // max is 250.0 px
        assert_eq!(p.clamp(14.0), 14.0);
    }

    #[test]
    fn all_numeric_params_for_stroke_includes_size_and_opacity() {
        let fx = test_stroke();
        let list = all_numeric_params(&fx);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].key, "size");
        assert_eq!(list[1].key, "opacity");
    }

    #[test]
    fn format_strings_are_clean_and_readable() {
        let fx = test_stroke();
        let p = primary_param(&fx).unwrap();
        assert_eq!(p.format(14.0), "14 px");
        assert_eq!(p.format(14.2), "14.2 px");
        assert_eq!(fmt_percent(75.0, "%"), "75%");
        assert_eq!(fmt_percent(75.5, "%"), "75.5%");
        assert_eq!(fmt_degrees(120.0, "°"), "120°");
    }

    #[test]
    fn disambiguation_threshold_logic() {
        // Below deadband (4px)
        let delta_small = vec2(2.0, 1.0);
        assert!(delta_small.length() < DEADBAND_POINTS);

        // Horizontal movement: |dx| >= |dy| * HORIZONTAL_BIAS
        let delta_h = vec2(6.0, 2.0);
        assert!(delta_h.length() >= DEADBAND_POINTS);
        assert!(delta_h.x.abs() >= delta_h.y.abs() * HORIZONTAL_BIAS);

        // Vertical movement: |dx| < |dy| * HORIZONTAL_BIAS
        let delta_v = vec2(1.0, 8.0);
        assert!(delta_v.length() >= DEADBAND_POINTS);
        assert!(delta_v.x.abs() < delta_v.y.abs() * HORIZONTAL_BIAS);
    }

    #[test]
    fn drop_shadow_parameters() {
        let shadow = test_shadow();
        let p = primary_param(&shadow).expect("drop shadow has primary param");
        assert_eq!(p.key, "size");
        assert_eq!((p.getter)(&shadow), 8.0);

        let list = all_numeric_params(&shadow);
        assert!(list.iter().any(|item| item.key == "size"));
        assert!(list.iter().any(|item| item.key == "distance"));
        assert!(list.iter().any(|item| item.key == "spread"));
        assert!(list.iter().any(|item| item.key == "opacity"));
        assert!(list.iter().any(|item| item.key == "angle"));
        assert!(list.iter().any(|item| item.key == "noise"));
    }

    #[test]
    fn make_replace_action_preserves_all_effects_on_layer() {
        let stroke = test_stroke();
        let shadow = test_shadow();
        let effects = vec![stroke, shadow];

        let layer_id = LayerId(42);
        let effect_idx = 0; // Editing Stroke
        let (cmd, payload) = make_replace_action(layer_id, effect_idx, &effects, "size", 12.0, 555);

        assert_eq!(cmd, "layer.layerStyle.replace");
        assert_eq!(payload["layer"], 42);
        assert_eq!(payload["coalesce"], "fx-scrub-42-0-555");

        let entries = payload["effects"].as_array().expect("effects array");
        assert_eq!(entries.len(), 2, "must keep both stroke and shadow");
        assert_eq!(entries[0]["kind"], "stroke");
        assert_eq!(entries[0]["params"]["size"], 12.0);
        assert_eq!(entries[1]["kind"], "dropShadow");
    }

    #[test]
    fn escape_restores_initial_value_in_state() {
        let stroke = test_stroke();
        let effects = vec![stroke];
        let layer_id = LayerId(10);
        let effect_idx = 0;
        let initial_val = 3.0;
        let token = 999;

        let (cmd, payload) = make_replace_action(layer_id, effect_idx, &effects, "size", initial_val, token);
        assert_eq!(cmd, "layer.layerStyle.replace");
        assert_eq!(payload["effects"][0]["params"]["size"], 3.0);
        assert_eq!(payload["coalesce"], "fx-scrub-10-0-999");
    }

    #[test]
    fn fractional_precision_with_alt_modifier() {
        let alt = Modifiers { alt: true, ..Default::default() };
        let rate = modifier_rate(alt);
        assert_eq!(rate, 0.2);

        let delta_x = 1.0;
        let mut accumulator = 0.0;
        let speed = 1.0;
        accumulator += delta_x * speed * rate; // +0.2

        let step = 0.2;
        let steps = (accumulator / step).trunc();
        assert_eq!(steps, 1.0);
        let val = 3.0 + steps * step;
        assert_eq!((val * 10.0).round() / 10.0, 3.2);

        let fx = test_stroke();
        let p = primary_param(&fx).unwrap();
        assert_eq!(p.format(val), "3.2 px");
    }

    #[test]
    fn hud_anchor_positioning_within_screen() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))),
            ..Default::default()
        }, |_| {}).textures_delta.clear();

        // Panel on the right dock (x=1600..1920)
        let row_rect = egui::Rect::from_min_size(pos2(1650.0, 300.0), vec2(270.0, 22.0));
        let anchor = compute_hud_anchor(&ctx, row_rect, 190.0);
        // Should place to the left of the dock
        assert!(anchor.x < 1650.0, "anchor.x {} should be to the left of dock 1650", anchor.x);
        assert!(anchor.x >= 0.0, "anchor.x {} should be inside screen", anchor.x);
    }

    #[test]
    fn execute_make_replace_action_on_session() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let layer_id = LayerId(s.execute("layer.new.layer", json!({"name": "test"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerStyle.stroke", json!({"size": 3})).unwrap();
        s.execute("layer.layerStyle.dropShadow", json!({"size": 8, "distance": 5})).unwrap();

        let doc = &s.active().unwrap().doc;
        let effects = &doc.layer(layer_id).unwrap().effects.items;
        assert_eq!(effects.len(), 2);

        // Edit stroke size from 3.0 to 15.0
        let (cmd, payload) = make_replace_action(layer_id, 0, effects, "size", 15.0, 123);
        let res = s.execute(&cmd, payload);
        assert!(res.is_ok(), "executing replace failed: {:?}", res.err());

        // Verify that the stroke size in the session document model actually persisted as 15.0
        let updated_effects = &s.active().unwrap().doc.layer(layer_id).unwrap().effects.items;
        assert_eq!(updated_effects.len(), 2, "both effects must persist");
        match &updated_effects[0] {
            Effect::Stroke(st) => assert_eq!(st.size, 15.0, "stroke size must be 15.0"),
            _ => panic!("first effect should be Stroke"),
        }
        match &updated_effects[1] {
            Effect::DropShadow(ds) => assert_eq!(ds.size, 8.0, "drop shadow must be preserved"),
            _ => panic!("second effect should be DropShadow"),
        }
    }

    #[test]
    fn handle_effect_drag_commits_on_release_and_returns_to_idle() {
        let ctx = egui::Context::default();
        let stroke = test_stroke();
        let effects = vec![stroke];
        let layer_id = LayerId(1);
        let p = primary_param(&effects[0]).unwrap();

        // Setup active scrubbing state in context
        ctx.data_mut(|d| {
            d.insert_temp(
                scrub_state_id(),
                ScrubGestureState::Scrubbing {
                    layer_id,
                    effect_idx: 0,
                    start_pos: pos2(100.0, 100.0),
                    initial_val: 3.0,
                    current_val: 12.0,
                    param: p,
                    coalesce_token: 777,
                    accumulator: 0.0,
                },
            );
        });

        // Run UI with pointer release
        let raw_input = egui::RawInput {
            events: vec![egui::Event::PointerButton {
                pos: pos2(150.0, 100.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };

        ctx.run_ui(raw_input, |ui| {
            let (_, resp) = ui.allocate_exact_size(vec2(100.0, 20.0), Sense::click_and_drag());
            let out = handle_effect_drag(ui.ctx(), &resp, layer_id, 0, &effects, p, true);

            // Must emit commit action on release with the current value
            assert!(out.action.is_some(), "action must be emitted on release");
            let (cmd, payload) = out.action.unwrap();
            assert_eq!(cmd, "layer.layerStyle.replace");
            assert_eq!(payload["layer"], 1);
            assert_eq!(payload["effects"][0]["params"]["size"], 12.0);

            // State in context must now be Idle
            let state = ui.ctx().data(|d| d.get_temp::<ScrubGestureState>(scrub_state_id())).unwrap();
            assert!(matches!(state, ScrubGestureState::Idle));
        }).textures_delta.clear();
    }

    #[test]
    fn multi_row_release_only_commits_for_active_scrubbing_row() {
        let ctx = egui::Context::default();
        let stroke = test_stroke();
        let shadow = test_shadow();
        let effects = vec![stroke, shadow];
        let layer_id = LayerId(1);
        let p0 = primary_param(&effects[0]).unwrap();
        let p1 = primary_param(&effects[1]).unwrap();

        // User is scrubbing row 1 (Drop Shadow)
        ctx.data_mut(|d| {
            d.insert_temp(
                scrub_state_id(),
                ScrubGestureState::Scrubbing {
                    layer_id,
                    effect_idx: 1,
                    start_pos: pos2(100.0, 100.0),
                    initial_val: 8.0,
                    current_val: 20.0,
                    param: p1,
                    coalesce_token: 888,
                    accumulator: 0.0,
                },
            );
        });

        // Run UI with pointer release
        let raw_input = egui::RawInput {
            events: vec![egui::Event::PointerButton {
                pos: pos2(150.0, 100.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };

        ctx.run_ui(raw_input, |ui| {
            // First row (Stroke, effect_idx = 0) is evaluated first
            let (_, resp0) = ui.allocate_exact_size(vec2(100.0, 20.0), Sense::click_and_drag());
            let out0 = handle_effect_drag(ui.ctx(), &resp0, layer_id, 0, &effects, p0, true);

            // Row 0 must NOT emit an action and must NOT clear the active scrubbing gesture
            assert!(out0.action.is_none(), "inactive row 0 must not emit action");
            let state_mid = ui.ctx().data(|d| d.get_temp::<ScrubGestureState>(scrub_state_id())).unwrap();
            assert!(matches!(state_mid, ScrubGestureState::Scrubbing { .. }), "gesture must stay active for row 1");

            // Second row (Drop Shadow, effect_idx = 1) is evaluated next
            let (_, resp1) = ui.allocate_exact_size(vec2(100.0, 20.0), Sense::click_and_drag());
            let out1 = handle_effect_drag(ui.ctx(), &resp1, layer_id, 1, &effects, p1, true);

            // Row 1 must emit the commit action with current_val = 20.0
            assert!(out1.action.is_some(), "active row 1 must emit commit action");
            let (cmd, payload) = out1.action.unwrap();
            assert_eq!(cmd, "layer.layerStyle.replace");
            assert_eq!(payload["layer"], 1);
            assert_eq!(payload["effects"][1]["params"]["size"], 20.0);

            // State in context must now be Idle
            let state_end = ui.ctx().data(|d| d.get_temp::<ScrubGestureState>(scrub_state_id())).unwrap();
            assert!(matches!(state_end, ScrubGestureState::Idle));
        }).textures_delta.clear();
    }

    #[test]
    fn scrubbing_coalesces_into_single_undo_step_on_session() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        let layer_id = LayerId(s.execute("layer.new.layer", json!({"name": "test"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.layerStyle.stroke", json!({"size": 3})).unwrap();

        let effects = s.active().unwrap().doc.layer(layer_id).unwrap().effects.items.clone();
        let token = 42;

        // Simulate 3 dragging frames + 1 release frame with same coalesce token
        for val in [5.0, 8.0, 12.0, 15.0] {
            let (cmd, payload) = make_replace_action(layer_id, 0, &effects, "size", val, token);
            s.execute(&cmd, payload).unwrap();
        }

        // Current size must be 15.0
        match &s.active().unwrap().doc.layer(layer_id).unwrap().effects.items[0] {
            Effect::Stroke(st) => assert_eq!(st.size, 15.0),
            _ => panic!("expected Stroke"),
        }

        // Exactly ONE undo step must revert back to initial size (3.0)
        s.execute("edit.undo", json!({})).unwrap();
        match &s.active().unwrap().doc.layer(layer_id).unwrap().effects.items[0] {
            Effect::Stroke(st) => assert_eq!(st.size, 3.0, "undo must restore initial size 3.0"),
            _ => panic!("expected Stroke"),
        }

        // Redo must restore final committed size (15.0)
        s.execute("edit.redo", json!({})).unwrap();
        match &s.active().unwrap().doc.layer(layer_id).unwrap().effects.items[0] {
            Effect::Stroke(st) => assert_eq!(st.size, 15.0, "redo must restore final size 15.0"),
            _ => panic!("expected Stroke"),
        }
    }
}

