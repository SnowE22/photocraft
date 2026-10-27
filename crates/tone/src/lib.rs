//! Screentone parameters shared by the document model, the procedural renderer, the engine and
//! the UI.
//!
//! Keeping [`ScreentoneSpec`] here lets a document store a live screentone layer without making
//! `photocraft-doc` depend on the filter crate. The renderer re-exports these types, so existing
//! callers can keep using `photocraft_algo::ScreentoneSpec`.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// 12 pattern types supported by the GenTonik screentone engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PatternType {
    #[default]
    Dots,
    Lines,
    Crosshatch,
    Noise,
    GaussianNoise,
    Stipple,
    Hexgrid,
    Checker,
    Concentric,
    Stars,
    Hearts,
    Triangles,
}

/// Geometric dot shapes for raster dots and satellites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DotShape {
    #[default]
    Circle,
    Square,
    Diamond,
    Hexagon,
}

/// Gradient mapping type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScreentoneGradientType {
    #[default]
    None,
    Linear,
    Radial,
}

/// Screentone generation mode: halftone shading from underlying artwork (`Shading`) or a uniform
/// procedural sheet (`Sheet`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScreentoneMode {
    #[default]
    Shading,
    Sheet,
}

/// Renderer backend used to generate screentone pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScreentoneBackend {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

/// Parameters for procedural screentone generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreentoneSpec {
    // ---- Pattern ----
    pub pattern_type: PatternType,
    pub dot_shape: DotShape,
    pub dot_size: f32,
    pub spacing_x: f32,
    pub spacing_y: f32,
    pub density: f32,
    pub angle: f32,
    pub line_width: f32,
    pub cross_angle: f32,
    /// Phase offset of the lattice in the tone's own rotated frame: where cell `0, 0`
    /// anchors (the origin the on-canvas gizmo's handle drags). `0, 0` anchors the lattice
    /// at the render bounds' center and matches every pre-origin document.
    pub origin_x: f32,
    pub origin_y: f32,

    // ---- Satellites ----
    pub satellite_enabled: bool,
    pub satellite_size: f32,
    pub satellite_distance: f32,
    pub satellite_count: u32,
    pub satellite_angle: f32,
    pub satellite_dot_shape: DotShape,
    pub satellite_stretch: f32,

    // ---- Distortion & Geometry ----
    pub aspect_x: f32,
    pub aspect_y: f32,
    pub row_offset: f32,
    pub merge_factor: f32,
    pub jitter_pos: f32,
    pub jitter_size: f32,
    pub roundness: f32,

    // ---- Gradient Mapping ----
    pub grad_type: ScreentoneGradientType,
    pub grad_angle: f32,
    pub grad_reverse: bool,
    pub grad_midpoint: f32,
    pub grad_size_start: f32,
    pub grad_size_end: f32,
    pub grad_stretch_start: f32,
    pub grad_stretch_end: f32,
    pub grad_color_trans: bool,

    // ---- Colors ----
    /// Straight sRGB RGBA for foreground (pattern/ink).
    pub foreground: [f32; 4],
    /// Straight sRGB RGBA for background.
    pub background: [f32; 4],

    // ---- Rotations ----
    pub rot_canvas: f32,
    pub rot_pattern: f32,

    // ---- Manga Shading & Antialiasing ----
    /// Generation mode: `Shading` modulates dot/satellite sizes by underlying luminance with paper
    /// and shadow thresholds; `Sheet` fills the frame uniformly.
    pub mode: ScreentoneMode,
    /// Pre-halftone brightness offset (-1.0..=1.0, default 0.0).
    pub brightness: f32,
    /// Pre-halftone contrast multiplier around midpoint 0.5 (0.0..=4.0, default 1.0).
    pub contrast: f32,
    /// Luminance cutoff for clean white paper (pixels with adjusted luma >= `white_threshold`
    /// draw 0% ink).
    pub white_threshold: f32,
    /// Luminance cutoff for deep shadow beta fill (pixels with adjusted luma <= `black_threshold`
    /// fill with 100% solid foreground ink).
    pub black_threshold: f32,
    /// Gamma curve applied to underlying luminance before threshold mapping.
    pub tone_gamma: f32,
    /// Preserve dark high-contrast line art as solid ink instead of breaking lines into dots.
    pub preserve_lines: bool,
    /// Luminance threshold below which detected edges are preserved as solid ink (0.0..=1.0).
    pub line_threshold: f32,
    /// Line art thickness / dilation radius in pixels (0.5..=5.0, default 1.0).
    pub line_thickness: f32,
    /// Modulate dot size by underlying layer darkness (halftone manga shading mode).
    pub use_source_luminance: bool,
    /// Edge smoothness: 1.0 = smooth antialiased, 0.0 = pure 1-bit manga threshold.
    pub smoothness: f32,
    /// Seed for noise and stipple patterns.
    pub seed: u32,
    /// Execution backend: `Auto` (GPU on large canvases, CPU on small), `Gpu`, or `Cpu`.
    pub backend: ScreentoneBackend,
}

impl Default for ScreentoneSpec {
    fn default() -> Self {
        Self {
            pattern_type: PatternType::Dots,
            dot_shape: DotShape::Circle,
            dot_size: 6.0,
            spacing_x: 16.0,
            spacing_y: 16.0,
            density: 0.5,
            angle: 0.0,
            line_width: 1.5,
            cross_angle: 45.0,
            origin_x: 0.0,
            origin_y: 0.0,

            satellite_enabled: false,
            satellite_size: 4.0,
            satellite_distance: 18.0,
            satellite_count: 4,
            satellite_angle: 0.0,
            satellite_dot_shape: DotShape::Circle,
            satellite_stretch: 0.0,

            aspect_x: 1.0,
            aspect_y: 1.0,
            row_offset: 0.0,
            merge_factor: 0.0,
            jitter_pos: 0.0,
            jitter_size: 0.0,
            roundness: 0.0,

            grad_type: ScreentoneGradientType::None,
            grad_angle: 90.0,
            grad_reverse: false,
            grad_midpoint: 0.5,
            grad_size_start: 1.0,
            grad_size_end: 0.0,
            grad_stretch_start: 1.0,
            grad_stretch_end: 1.0,
            grad_color_trans: false,

            foreground: [0.0, 0.0, 0.0, 1.0],
            background: [1.0, 1.0, 1.0, 0.0],

            rot_canvas: 0.0,
            rot_pattern: 0.0,

            mode: ScreentoneMode::Shading,
            brightness: 0.0,
            contrast: 1.0,
            white_threshold: 0.85,
            black_threshold: 0.08,
            tone_gamma: 1.0,
            preserve_lines: false,
            line_threshold: 0.24,
            line_thickness: 1.0,
            use_source_luminance: true,
            smoothness: 1.0,
            seed: 42,
            backend: ScreentoneBackend::Auto,
        }
    }
}

impl ScreentoneSpec {
    /// Pixels a tile reads outside itself when source luminance is modulated.
    pub fn halo_radius(&self) -> i32 {
        if self.use_source_luminance && self.mode == ScreentoneMode::Shading {
            let max_spacing = self.spacing_x.max(self.spacing_y).clamp(1.0, 256.0);
            let line_pad = if self.preserve_lines { self.line_thickness.ceil() as i32 + 16 } else { 0 };
            ((max_spacing * 1.5).ceil() as i32 + 14).max(line_pad)
        } else {
            0
        }
    }
}
