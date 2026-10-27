//! Manga & comic screentone generator (`filter.screentone`, `layer.screentone`) backed by
//! [`photocraft_algo::screentone`] (the GenTonik engine).
//!
//! `filter.screentone` runs through [`crate::filters::run_filter`] like every other filter, so
//! selection, smart objects, history and `filter.lastFilter` work the same way; with
//! `useSourceLuminance` it modulates the raster by the layer's own darkness (manga shading).
//! `layer.screentone` renders a procedural sheet onto a **new** pixel layer (no source needed),
//! which is what the Screentones panel uses to drop a tone onto the document; when a selection
//! is active the new layer carries it as its own mask, so the tone keeps the selection's shape
//! (not just its frame) in the document. The same command with a `layer` id **restyles** that
//! layer in place — same id, name and mask, one history step — which is the on-canvas gizmo's
//! commit for a pointer drag of the pending spec.
//!
//! Both commands take the same flat parameter object (the dialog's notation), so a preset saved
//! from one is replayable through the other. `pattern.screentone` renders that same sheet into
//! the app-level pattern library (the panel's `Save as Pattern`), so a favourite tone survives
//! restarts and can fill layers and brush textures like any other library pattern.

use photocraft_algo::{self as algo, DotShape, FilterParams, PatternType, ScreentoneBackend, ScreentoneGradientType, ScreentoneMode, ScreentoneSpec};
use photocraft_color::PixelFormat;
use photocraft_doc::{Layer, LayerContent, LayerMask, Pattern, Rect, ScreentoneLayer};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Applies the screentone to the active pixel layer (Filter menu).
pub const FILTER: &str = "filter.screentone";
/// Renders a procedural screentone sheet onto a new pixel layer (Screentones panel).
pub const LAYER: &str = "layer.screentone";
/// Renders the same sheet straight into the pattern library (`Save as Pattern` on the panel).
pub const PATTERN: &str = "pattern.screentone";

const PATTERNS: &str = "dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles";
const SHAPES: &str = "circle|square|diamond|hexagon";
const GRADIENTS: &str = "none|linear|radial";
const MODES: &str = "shading|sheet";

/// The flat parameter notation for [`FILTER`] (Photoshop-style dialog units).
const FILTER_PARAMS: &str = r##"{"mode":"shading|sheet","patternType":"dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles","dotShape":"circle|square|diamond|hexagon","dotSize":0..500=6,"spacingX":1..2000=16,"spacingY":1..2000=16,"density":0..1=0.5,"angle":-360..360=0,"lineWidth":0..200=1.5,"crossAngle":-360..360=45,"originX":-100000..100000=0,"originY":-100000..100000=0,"roundness":0..1=0,"smoothness":0..1=1,"brightness":-1..1=0,"contrast":0..4=1,"whiteThreshold":0..1=0.85,"blackThreshold":0..1=0.08,"toneGamma":0.01..10=1,"preserveLines":bool,"lineThreshold":0..1=0.24,"lineThickness":0.5..5=1,"useSourceLuminance":bool=true,"satelliteEnabled":bool,"satelliteSize":0..500=4,"satelliteDistance":0..2000=18,"satelliteCount":1..4=4,"satelliteAngle":-360..360=0,"satelliteShape":"circle|square|diamond|hexagon","satelliteStretch":0..10=0,"aspectX":0.05..20=1,"aspectY":0.05..20=1,"rowOffset":-1..1=0,"mergeFactor":0..1=0,"jitterPosition":0..100=0,"jitterSize":0..100=0,"gradientType":"none|linear|radial","gradientAngle":-360..360=90,"gradientReverse":bool,"gradientMidpoint":0..1=0.5,"gradientSizeStart":0..10=1,"gradientSizeEnd":0..10=0,"gradientStretchStart":0..10=1,"gradientStretchEnd":0..10=1,"gradientColorTransition":bool,"foreground":color=#000000,"background":color,"rotateCanvas":-360..360=0,"rotatePattern":-360..360=0,"seed":u32=42}"##;

/// The flat parameter notation for [`LAYER`].
const PARAMS: &str = r##"{"preset":text,"mode":"shading|sheet","patternType":"dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles","dotShape":"circle|square|diamond|hexagon","dotSize":0..500=6,"spacingX":1..2000=16,"spacingY":1..2000=16,"density":0..1=0.5,"angle":-360..360=0,"lineWidth":0..200=1.5,"crossAngle":-360..360=45,"originX":-100000..100000=0,"originY":-100000..100000=0,"roundness":0..1=0,"smoothness":0..1=1,"brightness":-1..1=0,"contrast":0..4=1,"whiteThreshold":0..1=0.85,"blackThreshold":0..1=0.08,"toneGamma":0.01..10=1,"preserveLines":bool,"lineThreshold":0..1=0.24,"lineThickness":0.5..5=1,"useSourceLuminance":bool=true,"satelliteEnabled":bool,"satelliteSize":0..500=4,"satelliteDistance":0..2000=18,"satelliteCount":1..4=4,"satelliteAngle":-360..360=0,"satelliteShape":"circle|square|diamond|hexagon","satelliteStretch":0..10=0,"aspectX":0.05..20=1,"aspectY":0.05..20=1,"rowOffset":-1..1=0,"mergeFactor":0..1=0,"jitterPosition":0..100=0,"jitterSize":0..100=0,"gradientType":"none|linear|radial","gradientAngle":-360..360=90,"gradientReverse":bool,"gradientMidpoint":0..1=0.5,"gradientSizeStart":0..10=1,"gradientSizeEnd":0..10=0,"gradientStretchStart":0..10=1,"gradientStretchEnd":0..10=1,"gradientColorTransition":bool,"foreground":color=#000000,"background":color,"rotateCanvas":-360..360=0,"rotatePattern":-360..360=0,"seed":u32=42,"layer":id?,"name":text?} → {layer,bounds}"##;

/// [PATTERN]'s notation: the same dialog fields, but the pattern library is the target.
const PATTERN_PARAMS: &str = r##"{"preset":text,"mode":"shading|sheet","patternType":"dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles","dotShape":"circle|square|diamond|hexagon","dotSize":0..500=6,"spacingX":1..2000=16,"spacingY":1..2000=16,"density":0..1=0.5,"angle":-360..360=0,"lineWidth":0..200=1.5,"crossAngle":-360..360=45,"originX":-100000..100000=0,"originY":-100000..100000=0,"roundness":0..1=0,"smoothness":0..1=1,"brightness":-1..1=0,"contrast":0..4=1,"whiteThreshold":0..1=0.85,"blackThreshold":0..1=0.08,"toneGamma":0.01..10=1,"preserveLines":bool,"lineThreshold":0..1=0.24,"lineThickness":0.5..5=1,"useSourceLuminance":bool=true,"satelliteEnabled":bool,"satelliteSize":0..500=4,"satelliteDistance":0..2000=18,"satelliteCount":1..4=4,"satelliteAngle":-360..360=0,"satelliteShape":"circle|square|diamond|hexagon","satelliteStretch":0..10=0,"aspectX":0.05..20=1,"aspectY":0.05..20=1,"rowOffset":-1..1=0,"mergeFactor":0..1=0,"jitterPosition":0..100=0,"jitterSize":0..100=0,"gradientType":"none|linear|radial","gradientAngle":-360..360=90,"gradientReverse":bool,"gradientMidpoint":0..1=0.5,"gradientSizeStart":0..10=1,"gradientSizeEnd":0..10=0,"gradientStretchStart":0..10=1,"gradientStretchEnd":0..10=1,"gradientColorTransition":bool,"foreground":color=#000000,"background":color,"rotateCanvas":-360..360=0,"rotatePattern":-360..360=0,"seed":u32=42,"name":text?} → {pattern,bounds}"##;

fn f_any(p: &Value, keys: &[&str], d: f32) -> f32 {
    for &k in keys {
        if let Some(v) = p.get(k).and_then(Value::as_f64) {
            return v as f32;
        }
    }
    d
}
fn b_any(p: &Value, keys: &[&str], d: bool) -> bool {
    for &k in keys {
        if let Some(v) = p.get(k).and_then(Value::as_bool) {
            return v;
        }
    }
    d
}
fn whole(p: &Value, k: &str, d: u32) -> u32 {
    p.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(d)
}

/// `[r, g, b(, a)]` (0–1), `#rrggbb`, `#rrggbbaa`, or `d` for anything else.
fn colour(p: &Value, k: &str, d: [f32; 4]) -> [f32; 4] {
    match p.get(k) {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c = |j: usize, dv: f32| a.get(j).and_then(Value::as_f64).map_or(dv, |v| v as f32);
            [c(0, d[0]), c(1, d[1]), c(2, d[2]), c(3, d[3])]
        }
        Some(Value::String(h)) if h.starts_with('#') => {
            let hex = h.trim_start_matches('#');
            let ch = |j: usize| hex.get(j..j + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map(|v| f32::from(v) / 255.0);
            let (Some(r), Some(g), Some(bl)) = (ch(0), ch(2), ch(4)) else { return d };
            [r, g, bl, ch(6).unwrap_or(d[3])]
        }
        _ => d,
    }
}

/// `patternType` as a `PatternType`, or `base` when absent or unknown.
fn pattern(p: &Value, base: PatternType) -> PatternType {
    match p.get("patternType").and_then(Value::as_str) {
        Some("dots") => PatternType::Dots,
        Some("lines") => PatternType::Lines,
        Some("crosshatch") => PatternType::Crosshatch,
        Some("noise") => PatternType::Noise,
        Some("gaussianNoise" | "gaussian_noise") => PatternType::GaussianNoise,
        Some("stipple") => PatternType::Stipple,
        Some("hexgrid") => PatternType::Hexgrid,
        Some("checker") => PatternType::Checker,
        Some("concentric") => PatternType::Concentric,
        Some("stars") => PatternType::Stars,
        Some("hearts") => PatternType::Hearts,
        Some("triangles") => PatternType::Triangles,
        _ => base,
    }
}

/// `k` as a `DotShape`, or `base` when absent or unknown.
fn shape(p: &Value, k: &str, base: DotShape) -> DotShape {
    match p.get(k).and_then(Value::as_str) {
        Some("circle") => DotShape::Circle,
        Some("square") => DotShape::Square,
        Some("diamond") => DotShape::Diamond,
        Some("hexagon") => DotShape::Hexagon,
        _ => base,
    }
}

fn shape_any(p: &Value, keys: &[&str], base: DotShape) -> DotShape {
    for &k in keys {
        if let Some(s) = p.get(k).and_then(Value::as_str) {
            match s {
                "circle" => return DotShape::Circle,
                "square" => return DotShape::Square,
                "diamond" => return DotShape::Diamond,
                "hexagon" => return DotShape::Hexagon,
                _ => {}
            }
        }
    }
    base
}

fn gradient_any(p: &Value, keys: &[&str], base: ScreentoneGradientType) -> ScreentoneGradientType {
    for &k in keys {
        if let Some(s) = p.get(k).and_then(Value::as_str) {
            match s {
                "none" => return ScreentoneGradientType::None,
                "linear" => return ScreentoneGradientType::Linear,
                "radial" => return ScreentoneGradientType::Radial,
                _ => {}
            }
        }
    }
    base
}

fn mode_from(p: &Value, base: ScreentoneMode) -> ScreentoneMode {
    match p.get("mode").and_then(Value::as_str) {
        Some("shading") => ScreentoneMode::Shading,
        Some("sheet") => ScreentoneMode::Sheet,
        _ => base,
    }
}

/// The screentone parameters for `p`, clamped into the ranges [`PARAMS`] documents. A named
/// `preset` seeds every field the call does not set explicitly.
pub fn spec(p: &Value) -> ScreentoneSpec {
    let base = p
        .get("preset")
        .and_then(Value::as_str)
        .and_then(|id| algo::screentone::built_in_presets().into_iter().find(|pr| pr.id == id))
        .map_or_else(ScreentoneSpec::default, |pr| pr.spec);
    let parsed_mode = if p.get("mode").is_some() {
        mode_from(p, base.mode)
    } else if let Some(use_luma) = p.get("useSourceLuminance").and_then(Value::as_bool) {
        if use_luma { ScreentoneMode::Shading } else { ScreentoneMode::Sheet }
    } else {
        base.mode
    };
    let use_source_luminance = if p.get("useSourceLuminance").is_some() {
        b_any(p, &["useSourceLuminance"], base.use_source_luminance)
    } else if p.get("mode").is_some() {
        parsed_mode == ScreentoneMode::Shading
    } else {
        base.use_source_luminance
    };
    let backend = p.get("backend").and_then(Value::as_str).and_then(|s| match s {
        "auto" | "Auto" => Some(ScreentoneBackend::Auto),
        "gpu" | "Gpu" => Some(ScreentoneBackend::Gpu),
        "cpu" | "Cpu" => Some(ScreentoneBackend::Cpu),
        _ => None,
    }).unwrap_or(base.backend);
    ScreentoneSpec {
        pattern_type: pattern(p, base.pattern_type),
        dot_shape: shape(p, "dotShape", base.dot_shape),
        dot_size: f_any(p, &["dotSize"], base.dot_size).clamp(0.0, 500.0),
        spacing_x: f_any(p, &["spacingX"], base.spacing_x).clamp(1.0, 2000.0),
        spacing_y: f_any(p, &["spacingY"], base.spacing_y).clamp(1.0, 2000.0),
        density: f_any(p, &["density"], base.density).clamp(0.0, 1.0),
        angle: f_any(p, &["angle"], base.angle).clamp(-360.0, 360.0),
        line_width: f_any(p, &["lineWidth"], base.line_width).clamp(0.0, 200.0),
        cross_angle: f_any(p, &["crossAngle"], base.cross_angle).clamp(-360.0, 360.0),
        origin_x: f_any(p, &["originX"], base.origin_x).clamp(-100_000.0, 100_000.0),
        origin_y: f_any(p, &["originY"], base.origin_y).clamp(-100_000.0, 100_000.0),
        satellite_enabled: b_any(p, &["satelliteEnabled"], base.satellite_enabled),
        satellite_size: f_any(p, &["satelliteSize"], base.satellite_size).clamp(0.0, 500.0),
        satellite_distance: f_any(p, &["satelliteDistance"], base.satellite_distance).clamp(0.0, 2000.0),
        satellite_count: whole(p, "satelliteCount", base.satellite_count).clamp(1, 4),
        satellite_angle: f_any(p, &["satelliteAngle"], base.satellite_angle).clamp(-360.0, 360.0),
        satellite_dot_shape: shape_any(p, &["satelliteShape", "satelliteDotShape"], base.satellite_dot_shape),
        satellite_stretch: f_any(p, &["satelliteStretch"], base.satellite_stretch).clamp(0.0, 10.0),
        aspect_x: f_any(p, &["aspectX"], base.aspect_x).clamp(0.05, 20.0),
        aspect_y: f_any(p, &["aspectY"], base.aspect_y).clamp(0.05, 20.0),
        row_offset: f_any(p, &["rowOffset"], base.row_offset).clamp(-1.0, 1.0),
        merge_factor: f_any(p, &["mergeFactor"], base.merge_factor).clamp(0.0, 1.0),
        jitter_pos: f_any(p, &["jitterPosition", "jitterPos"], base.jitter_pos).clamp(0.0, 100.0),
        jitter_size: f_any(p, &["jitterSize"], base.jitter_size).clamp(0.0, 100.0),
        roundness: f_any(p, &["roundness"], base.roundness).clamp(0.0, 1.0),
        grad_type: gradient_any(p, &["gradientType", "gradType"], base.grad_type),
        grad_angle: f_any(p, &["gradientAngle", "gradAngle"], base.grad_angle).clamp(-360.0, 360.0),
        grad_reverse: b_any(p, &["gradientReverse", "gradReverse"], base.grad_reverse),
        grad_midpoint: f_any(p, &["gradientMidpoint", "gradMidpoint"], base.grad_midpoint).clamp(0.0, 1.0),
        grad_size_start: f_any(p, &["gradientSizeStart", "gradSizeStart"], base.grad_size_start).clamp(0.0, 10.0),
        grad_size_end: f_any(p, &["gradientSizeEnd", "gradSizeEnd"], base.grad_size_end).clamp(0.0, 10.0),
        grad_stretch_start: f_any(p, &["gradientStretchStart", "gradStretchStart"], base.grad_stretch_start).clamp(0.0, 10.0),
        grad_stretch_end: f_any(p, &["gradientStretchEnd", "gradStretchEnd"], base.grad_stretch_end).clamp(0.0, 10.0),
        grad_color_trans: b_any(p, &["gradientColorTransition", "gradColorTrans"], base.grad_color_trans),
        foreground: colour(p, "foreground", base.foreground),
        background: colour(p, "background", base.background),
        rot_canvas: f_any(p, &["rotateCanvas", "rotCanvas"], base.rot_canvas).clamp(-360.0, 360.0),
        rot_pattern: f_any(p, &["rotatePattern", "rotPattern"], base.rot_pattern).clamp(-360.0, 360.0),
        mode: parsed_mode,
        brightness: f_any(p, &["brightness"], base.brightness).clamp(-1.0, 1.0),
        contrast: f_any(p, &["contrast"], base.contrast).clamp(0.0, 4.0),
        white_threshold: f_any(p, &["whiteThreshold"], base.white_threshold).clamp(0.0, 1.0),
        black_threshold: f_any(p, &["blackThreshold"], base.black_threshold).clamp(0.0, 1.0),
        tone_gamma: f_any(p, &["toneGamma"], base.tone_gamma).clamp(0.01, 10.0),
        preserve_lines: b_any(p, &["preserveLines"], base.preserve_lines),
        line_threshold: f_any(p, &["lineThreshold"], base.line_threshold).clamp(0.0, 1.0),
        line_thickness: f_any(p, &["lineThickness"], base.line_thickness).clamp(0.5, 5.0),
        use_source_luminance,
        smoothness: f_any(p, &["smoothness"], base.smoothness).clamp(0.0, 1.0),
        seed: whole(p, "seed", base.seed),
        backend,
    }
}

/// The algorithm parameters for the screentone ids.
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    matches!(id, FILTER | LAYER).then(|| FilterParams::Screentone { spec: spec(p) })
}

/// The choices of the enumerated parameters.
fn choices(key: &str) -> Option<&'static str> {
    match key {
        "mode" => Some(MODES),
        "patternType" => Some(PATTERNS),
        "dotShape" | "satelliteShape" => Some(SHAPES),
        "gradientType" => Some(GRADIENTS),
        _ => None,
    }
}

/// Rejects unknown choices and wrong-typed structured params for a new invocation. Every number
/// is clamped in [`spec`], so an out-of-range value can never panic or corrupt a document.
pub(crate) fn validate_params(id: &str, p: &Value) -> Result<()> {
    if !matches!(id, FILTER | LAYER | PATTERN) {
        return Ok(());
    }
    let bad = |msg: String| EngineError::BadParams { cmd: id.into(), msg };
    if p.is_null() {
        return Ok(());
    }
    let Some(map) = p.as_object() else {
        return Err(bad("params must be a JSON object".into()));
    };
    for (key, value) in map {
        if let Some(allowed) = choices(key) {
            if !value.as_str().is_some_and(|v| allowed.split('|').any(|c| c == v)) {
                return Err(bad(format!("`{key}` must be one of {allowed}")));
            }
            continue;
        }
        match key.as_str() {
            "layer" => {
                if value.as_u64().is_none() {
                    return Err(bad("`layer` must be an unsigned layer id".into()));
                }
            }
            "preset" | "name" => {
                if !value.is_null() && value.as_str().is_none() {
                    return Err(bad(format!("`{key}` must be a string")));
                }
            }
            "foreground" | "background" if !value.is_string() && !value.is_array() => {
                return Err(bad(format!("`{key}` must be a colour")));
            }
            _ => {}
        }
    }
    Ok(())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The sheet's name: an explicit `name`, else the seeding preset's name, else "Screentone".
fn requested_name(p: &Value) -> String {
    p.get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            p.get("preset")
                .and_then(Value::as_str)
                .and_then(|id| algo::screentone::built_in_presets().into_iter().find(|pr| pr.id == id).map(|pr| pr.name.to_string()))
        })
        .unwrap_or_else(|| "Screentone".to_string())
}

/// Whether `surf` holds non-white, non-transparent ink inside `rect` that can modulate halftone
/// dot sizes (`(1.0 - luma) * alpha > 1e-3`).
fn has_source_ink(surf: &Surface, rect: Rect) -> bool {
    let cb = surf.content_bounds().intersect(&rect);
    if cb.is_empty() {
        return false;
    }
    let fmt = surf.format();
    let ch = fmt.channels();
    let color_ch = fmt.mode.color_channels();
    if ch == 0 || color_ch == 0 {
        return false;
    }
    let has_alpha = fmt.alpha;
    let buf = surf.read_region(cb);
    buf.chunks_exact(ch).any(|px| {
        let a = if has_alpha { px[ch - 1].clamp(0.0, 1.0) } else { 1.0 };
        if a <= 1e-3 {
            return false;
        }
        let luma = match fmt.mode {
            photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Bitmap => px[0],
            photocraft_color::ColorMode::Cmyk if color_ch >= 4 => {
                let k = px[3];
                let r = (1.0 - px[0]) * (1.0 - k);
                let g = (1.0 - px[1]) * (1.0 - k);
                let b = (1.0 - px[2]) * (1.0 - k);
                0.2126 * r + 0.7152 * g + 0.0722 * b
            }
            photocraft_color::ColorMode::Lab => px[0],
            _ if color_ch >= 3 => 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2],
            _ => px[0],
        }
        .clamp(0.0, 1.0);
        (1.0 - luma) * a > 1e-3
    })
}

/// Samples the underlying tonal source inside `rect` when `useSourceLuminance` is active:
///
/// 1. If `active` is a non-screentone pixel layer that already has tonal ink in `rect`, its own
///    surface region is sampled directly (with transparent pixels treated as 0 ink by `algo`).
/// 2. Otherwise (empty layer, group, or restyling an existing screentone layer `exclude`), all
///    visible non-screentone pixel layers (excluding `exclude`) are composited over white paper so
///    transparent layers above white paper or multiple sketch/shading layers combine naturally.
/// 3. If the resulting source has no tonal ink at all (e.g. a blank white canvas), returns `None`
///    so the caller falls back to a uniform procedural sheet (`use_source_luminance = false`).
fn sample_source_surface(
    doc: &photocraft_doc::Document,
    active: Option<photocraft_doc::LayerId>,
    exclude: Option<photocraft_doc::LayerId>,
    rect: Rect,
    fmt: PixelFormat,
) -> Option<Surface> {
    if rect.is_empty() {
        return None;
    }
    if let Some(id) = active
        && Some(id) != exclude
        && let Some(layer) = doc.layer(id)
        && !layer.is_screentone()
        && let Some(surf) = layer.surface()
        && has_source_ink(surf, rect)
    {
        let mut out = Surface::new(surf.format());
        let r = surf.content_bounds().intersect(&rect);
        let px = surf.read_region(r);
        out.write_region(r, &px);
        return Some(out);
    }

    let mut flat = doc.clone();
    for l in &mut flat.layers {
        if Some(l.id) == exclude || l.is_screentone() {
            l.visible = false;
        }
    }
    let comp = photocraft_compose::render(&flat, rect).over_background([1.0, 1.0, 1.0]);
    let mut out = Surface::new(fmt);
    let ch = fmt.channels();
    let color_ch = fmt.mode.color_channels();
    let has_alpha = fmt.alpha;
    let mut buf = vec![0.0f32; (rect.width() as usize) * (rect.height() as usize) * ch];
    for (dst, rgba) in buf.chunks_exact_mut(ch).zip(comp.px.iter()) {
        let [r, g, b, _] = *rgba;
        match fmt.mode {
            photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Bitmap => {
                dst[0] = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            }
            photocraft_color::ColorMode::Cmyk if color_ch >= 4 => {
                let k = (1.0f32 - r.max(g).max(b)).clamp(0.0, 1.0);
                if k < 1.0 - 1e-6 {
                    dst[0] = (1.0 - r - k) / (1.0 - k);
                    dst[1] = (1.0 - g - k) / (1.0 - k);
                    dst[2] = (1.0 - b - k) / (1.0 - k);
                }
                dst[3] = k;
            }
            _ if color_ch >= 3 => {
                dst[0] = r;
                dst[1] = g;
                dst[2] = b;
            }
            _ => {
                dst[0] = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            }
        }
        if has_alpha {
            dst[ch - 1] = 1.0;
        }
    }
    out.write_region(rect, &buf);
    has_source_ink(&out, rect).then_some(out)
}

/// Renders a procedural screentone sheet onto a new pixel layer above the active one.
///
/// With a `layer` id the same command **restyles** that existing pixel layer instead: the sheet
/// is re-rendered and its raster content replaced in place — same layer id, name and mask, one
/// history step. That is the on-canvas gizmo's per-drag commit (see `screentone_gizmo`): drags
/// edit the pending spec live, and the pointer-up runs this once.
fn create_layer(s: &mut Session, p: &Value) -> Result<Value> {
    validate_params(LAYER, p)?;
    let mut spec = spec(p);

    if let Some(id) = p.get("layer").and_then(Value::as_u64) {
        if p.get("useSourceLuminance").is_none()
            && p.get("mode").is_none()
            && p.get("preset").is_none()
            && let Some(st) = s.active()
            && let Some(layer) = st.doc.layer(photocraft_doc::LayerId(id))
            && let LayerContent::Screentone(prev) = &layer.content
        {
            spec.use_source_luminance = prev.spec.use_source_luminance;
            spec.mode = prev.spec.mode;
        }
        return restyle_layer(s, photocraft_doc::LayerId(id), FilterParams::Screentone { spec });
    }

    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc_bounds = st.doc.bounds();
    let selection = st.doc.selection.clone();
    let sel_bounds = selection.as_ref().map(Surface::content_bounds).filter(|b| !b.is_empty());
    let bounds = sel_bounds.unwrap_or(doc_bounds);
    if bounds.is_empty() {
        return Err(EngineError::Other("the document is empty".into()));
    }
    // The spec's originX/originY is a phase inside the render frame; a live layer stores the
    // equivalent document-space anchor so its lattice stays put when the frame changes.
    let origin = algo::screentone::origin_doc(&spec, bounds);
    let sample_rect = bounds.inflate(spec.halo_radius()).intersect(&doc_bounds);
    let is_gpu = spec.backend == ScreentoneBackend::Gpu
        || (spec.backend == ScreentoneBackend::Auto && bounds.width() >= 512 && bounds.height() >= 512);
    let skip_cpu_bake = is_gpu;
    log::info!(
        "[SCREENTONE:CMD] create_screentone: bounds={:?}, mode={:?}, backend={:?}, use_source_lum={}, is_gpu={}, skip_cpu_bake={}",
        bounds, spec.mode, spec.backend, spec.use_source_luminance, is_gpu, skip_cpu_bake
    );
    let t0 = std::time::Instant::now();
    let mut rendered = if skip_cpu_bake {
        Surface::new(st.doc.pixel_format())
    } else if spec.mode == ScreentoneMode::Shading && spec.use_source_luminance {
        if let Some(src) = sample_source_surface(&st.doc, st.active_layer, None, sample_rect, st.doc.pixel_format()) {
            src
        } else {
            spec.use_source_luminance = false;
            Surface::new(st.doc.pixel_format())
        }
    } else {
        spec.use_source_luminance = false;
        Surface::new(st.doc.pixel_format())
    };
    let area = if skip_cpu_bake {
        bounds
    } else {
        algo::screentone::render_screentone_to_surface(&mut rendered, bounds, &spec, origin)
    };
    log::info!(
        "[SCREENTONE:CMD] create_screentone: finished in {:?}, rendered_tiles={}, area={:?}",
        t0.elapsed(), rendered.tile_count(), area
    );
    if area.is_empty() {
        return Err(EngineError::Other("nothing to render: the target area is empty".into()));
    }

    let name = requested_name(p);
    // An active selection keeps shaping the sheet for good: the layer carries it as its mask
    // (as in Photoshop), so the tone shows only inside the selection now and later.
    let mask = selection.as_ref().map(|sel| {
        let r = doc_bounds;
        let ch = sel.channels().max(1);
        let v = sel.read_region(r);
        let coverage: Vec<f32> = v.chunks_exact(ch).map(|px| px[0]).collect();
        let mut mask = LayerMask::reveal_all();
        mask.surface.write_region(r, &coverage);
        mask.surface.prune();
        mask
    });
    let opaque_paper = spec.background[3] >= 0.99;
    let content = ScreentoneLayer::new(spec, origin, rendered);
    let id = s.edit("Screentone", |doc, active| {
        let mut l = Layer::new(doc.next_layer_name(&name), LayerContent::Screentone(content));
        l.mask = mask;
        // Opaque paper shading layers sit in Normal blend mode so clean white paper covers the
        // reference drawing underneath; transparent film sheets darken via Multiply.
        l.blend = if opaque_paper {
            photocraft_color::BlendMode::Normal
        } else {
            photocraft_color::BlendMode::Multiply
        };
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({
        "layer": id.0,
        "bounds": [area.x0, area.y0, area.x1, area.y1],
    }))
}

/// The reference bounds a sheet renders over — the lattice anchor sits at their center
/// ([`ScreentoneSpec`]'s origin is a phase against that center), so creation and restyle must
/// share this rule or the pattern would slide between a gizmo drag's preview and its committed
/// render: a restyled layer covers the region its mask reveals, a not-yet-created sheet covers
/// the active selection's frame, and the whole canvas covers the rest.
pub fn sheet_bounds(s: &Session, target: Option<u64>) -> Rect {
    let Some(st) = s.active() else {
        return Rect::EMPTY;
    };
    let canvas = st.doc.bounds();
    if let Some(id) = target
        && let Some(layer) = st.doc.layer(photocraft_doc::LayerId(id))
        && let Some(mask) = layer.mask.as_ref()
    {
        return revealed_bounds(mask, canvas).unwrap_or(canvas);
    }
    if let Some(sel) = st.doc.selection.as_ref().map(Surface::content_bounds).filter(|b| !b.is_empty()) {
        return sel.intersect(&canvas);
    }
    canvas
}

/// The tight bounding box of pixels within `canvas` where `mask` reveals (`px > 0.0`).
/// Works identically for edge-touching masks and interior selection masks.
fn revealed_bounds(mask: &LayerMask, canvas: Rect) -> Option<Rect> {
    if !mask.enabled || canvas.is_empty() {
        return Some(canvas);
    }
    if mask.surface.content_bounds().is_empty() {
        return (mask.surface.default_pixel().first().copied().unwrap_or(1.0) > 0.0).then_some(canvas);
    }
    let ch = mask.surface.channels().max(1);
    let buf = mask.surface.read_region(canvas);
    let w = canvas.width() as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (canvas.x1, canvas.y1, canvas.x0, canvas.y0);
    for (i, px) in buf.chunks_exact(ch).enumerate() {
        if px[0] > 0.0 {
            let x = canvas.x0 + (i % w) as i32;
            let y = canvas.y0 + (i / w) as i32;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1, y1))
}

/// The gizmo's restyle commit: re-render the sheet with the pending spec and replace the pixel
/// layer's content in place. The layer id, name and mask survive — only the pixels change — and
/// the swap is a single journalled edit, so a drag undoes as exactly one step. Covers
/// [`sheet_bounds`] with the target; the mask is never re-derived from the selection.
fn restyle_layer(s: &mut Session, id: photocraft_doc::LayerId, mut params: FilterParams) -> Result<Value> {
    let FilterParams::Screentone { spec } = &mut params else {
        return Err(EngineError::Other("restyle expects screentone parameters".into()));
    };

    let st = s.active().ok_or(EngineError::NoDocument)?;
    let bounds = sheet_bounds(s, Some(id.0));
    let Some(layer) = st.doc.layer(id) else {
        return Err(EngineError::Other(format!("no layer {} to restyle", id.0)));
    };
    let Some(surface) = layer.surface() else {
        return Err(EngineError::Other(format!("layer {} is not a pixel layer", id.0)));
    };
    let origin = algo::screentone::origin_doc(spec, bounds);
    let sample_rect = bounds.inflate(spec.halo_radius()).intersect(&st.doc.bounds());
    let is_gpu = spec.backend == ScreentoneBackend::Gpu
        || (spec.backend == ScreentoneBackend::Auto && bounds.width() >= 512 && bounds.height() >= 512);
    let skip_cpu_bake = is_gpu;
    log::info!(
        "[SCREENTONE:CMD] restyle_layer: layer={}, bounds={:?}, mode={:?}, backend={:?}, use_source_lum={}, is_gpu={}, skip_cpu_bake={}",
        id.0, bounds, spec.mode, spec.backend, spec.use_source_luminance, is_gpu, skip_cpu_bake
    );
    let t0 = std::time::Instant::now();
    let mut rendered = if skip_cpu_bake {
        Surface::new(surface.format())
    } else if spec.mode == ScreentoneMode::Shading && spec.use_source_luminance {
        if let Some(src) = sample_source_surface(&st.doc, None, Some(id), sample_rect, surface.format()) {
            src
        } else {
            spec.use_source_luminance = false;
            Surface::new(surface.format())
        }
    } else {
        spec.use_source_luminance = false;
        Surface::new(surface.format())
    };
    let area = if skip_cpu_bake {
        bounds
    } else {
        algo::screentone::render_screentone_to_surface(&mut rendered, bounds, spec, origin)
    };
    log::info!(
        "[SCREENTONE:CMD] restyle_layer: finished in {:?}, rendered_tiles={}, area={:?}",
        t0.elapsed(), rendered.tile_count(), area
    );
    if area.is_empty() {
        return Err(EngineError::Other("nothing to render: the layer's area is empty".into()));
    }
    let spec = spec.clone();
    // Render a standalone sheet, then swap it for the layer's content in one edit. Live
    // screentone layers keep their parameters; older raster sheets stay raster (the legacy
    // in-place restyle the gizmo used before screentone content existed).
    s.edit("Screentone", |doc, _| {
        let layer = doc
            .layer_mut(id)
            .ok_or_else(|| EngineError::Other("the layer to restyle is gone".into()))?;
        if let LayerContent::Screentone(previous) = &mut layer.content {
            let was_opaque = previous.spec.background[3] >= 0.99;
            let now_opaque = spec.background[3] >= 0.99;
            if was_opaque != now_opaque {
                layer.blend = if now_opaque {
                    photocraft_color::BlendMode::Normal
                } else {
                    photocraft_color::BlendMode::Multiply
                };
            }
            previous.spec = spec;
            previous.origin = origin;
            previous.surface = rendered;
        } else {
            layer.content = LayerContent::Raster(rendered);
        }
        Ok(())
    })?;
    Ok(json!({
        "layer": id.0,
        "bounds": [area.x0, area.y0, area.x1, area.y1],
    }))
}

/// Renders the same sheet straight into the pattern library: what the panel's "Save as Pattern"
/// calls. Like [`create_layer`], the selection's frame bounds the tile when one is active, but
/// nothing touches the document — the library is app state, as in `edit.definePattern`.
fn create_pattern(s: &mut Session, p: &Value) -> Result<Value> {
    validate_params(PATTERN, p)?;
    let mut spec = spec(p);
    // A generated sheet has no source luminance to modulate by: always procedural.
    spec.mode = ScreentoneMode::Sheet;
    spec.use_source_luminance = false;
    let params = FilterParams::Screentone { spec };

    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc_bounds = st.doc.bounds();
    let selection = st.doc.selection.clone();
    let sel_bounds = selection.as_ref().map(Surface::content_bounds).filter(|b| !b.is_empty());
    let bounds = sel_bounds.unwrap_or(doc_bounds);
    if bounds.is_empty() {
        return Err(EngineError::Other("the document is empty".into()));
    }
    if bounds.width() > 4000 || bounds.height() > 4000 {
        return Err(EngineError::Other("patterns are limited to 4000 × 4000 px".into()));
    }
    let area = algo::output_area(&params, bounds, bounds, sel_bounds);
    if area.is_empty() {
        return Err(EngineError::Other("nothing to render: the target area is empty".into()));
    }
    let rendered = algo::apply(&Surface::new(st.doc.pixel_format()), &params, area, bounds, None);
    let fmt = rendered.format();
    let channels = fmt.channels();
    let px = rendered.read_region(area);

    // Opaque patterns drop their alpha channel, as `edit.definePattern` does.
    let opaque = px.chunks_exact(channels).all(|q| q[channels - 1] >= 1.0);
    let mut tile = Surface::new(if opaque { PixelFormat::new(fmt.mode, fmt.sample, false) } else { fmt });
    tile.write_region(Rect::new(0, 0, area.width() as i32, area.height() as i32), &px);
    let pat = Pattern::new(requested_name(p), tile, area.width() as u32, area.height() as u32);
    let id = pat.id.clone();
    if !s.patterns.items.iter().any(|q| q.id == id) {
        s.patterns.items.push(pat);
    }
    Ok(json!({"pattern": id, "bounds": [area.x0, area.y0, area.x1, area.y1]}))
}

/// The screentone commands: the Filter-menu filter, the panel's layer generator and the panel's
/// "Save as Pattern" library action.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILTER,
            label: "Screentone…",
            menu: &["Filter", "Pixelate"],
            shortcut: None,
            params: FILTER_PARAMS,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| crate::filters::run_filter(s, FILTER, p),
            journal: true,
        },
        CommandSpec {
            id: LAYER,
            label: "Screentone Layer",
            menu: &[],
            shortcut: None,
            params: PARAMS,
            enabled: has_doc,
            run: create_layer,
            journal: true,
        },
        CommandSpec {
            id: PATTERN,
            label: "Screentone Pattern",
            menu: &[],
            shortcut: None,
            params: PATTERN_PARAMS,
            enabled: has_doc,
            run: create_pattern,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(depth: u64) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 96, "height": 72, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s
    }

    fn read_layer(s: &Session, id: u64) -> Vec<f32> {
        s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 96, 72))
    }

    #[test]
    fn filter_and_layer_commands_work_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            let r = s.execute(LAYER, json!({"patternType": "dots", "dotSize": 5, "spacingX": 12, "spacingY": 12})).unwrap_or_else(|e| panic!("layer@{depth}: {e}"));
            let id = r["layer"].as_u64().unwrap();
            assert!(r["bounds"].is_array(), "{r}");
            assert!(read_layer(&s, id).as_chunks::<4>().0.iter().any(|p| p[3] > 0.0), "layer@{depth} drew nothing");
            // The filter edits a pixel layer in place, as one history step. A live screentone is
            // procedural, so the filter needs a fresh raster target.
            s.execute("layer.new.layer", json!({})).unwrap();
            let count = s.active().unwrap().history.entries().len();
            s.execute(FILTER, json!({"patternType": "lines", "useSourceLuminance": false})).unwrap_or_else(|e| panic!("filter@{depth}: {e}"));
            assert_eq!(s.active().unwrap().history.entries().len(), count + 1);
            for _ in 0..3 {
                s.execute("edit.undo", json!({})).unwrap();
            }
        }
    }

    #[test]
    fn presets_seed_the_parameters_and_are_replayable() {
        let preset = algo::screentone::built_in_presets().into_iter().next().unwrap();
        let seeded = spec(&json!({"preset": preset.id}));
        assert_eq!(seeded.dot_size, preset.spec.dot_size);
        assert_eq!(seeded.spacing_x, preset.spec.spacing_x);
        // An explicit key overrides the preset's value.
        let overridden = spec(&json!({"preset": preset.id, "dotSize": 9.0}));
        assert_eq!(overridden.dot_size, 9.0);
        assert_eq!(overridden.spacing_x, preset.spec.spacing_x);
        // Unknown ids fall back to the defaults instead of failing.
        assert_eq!(spec(&json!({"preset": "nope"})), ScreentoneSpec::default());
    }

    #[test]
    fn out_of_range_numbers_clamp_and_unknown_choices_are_rejected() {
        let clamped = spec(&json!({
            "spacingX": -50.0,
            "density": 9.0,
            "seed": 4_294_967_295u32,
            "satelliteCount": 99,
            "foreground": [2.0, -1.0, 0.5],
        }));
        assert_eq!(clamped.spacing_x, 1.0);
        assert_eq!(clamped.density, 1.0);
        assert_eq!(clamped.seed, u32::MAX);
        assert_eq!(clamped.satellite_count, 4);

        for p in [
            json!({"patternType": "spiral"}),
            json!({"dotShape": 3}),
            json!({"gradientType": "conic"}),
            json!({"layer": "x"}),
            json!({"preset": 5}),
            json!({"foreground": true}),
        ] {
            let e = validate_params(FILTER, &p).unwrap_err();
            assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
        }
        assert!(validate_params(FILTER, &json!({})).is_ok());
        assert!(validate_params(FILTER, &Value::Null).is_ok());
        // The validation is scoped to these two commands.
        assert!(validate_params("filter.blur.gaussianBlur", &json!({"nope": 1})).is_ok());
    }

    #[test]
    fn bad_params_leave_the_document_and_history_untouched() {
        let mut s = session(8);
        let before = s.active().unwrap().doc.clone();
        let count = s.active().unwrap().history.entries().len();
        for p in [json!({"patternType": "spiral"}), json!({"layer": "x"}), json!({"foreground": 1})] {
            assert!(s.execute(LAYER, p.clone()).is_err(), "{p}");
            assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().history.entries().len(), count);
        }
        // No document at all is a graceful error, not a panic.
        let mut empty = Session::new();
        assert!(empty.execute(LAYER, json!({})).is_err());
        assert!(empty.execute(FILTER, json!({})).is_err());
    }

    #[test]
    fn selection_bounds_limit_the_generated_sheet() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let b: Vec<i64> = r["bounds"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        assert_eq!(b, vec![0, 0, 40, 72], "the sheet is clipped to the selection");
        let id = r["layer"].as_u64().unwrap();
        let outside = read_layer(&s, id).as_chunks::<4>().0.iter().enumerate().filter(|(i, p)| (i % 96) >= 41 && p[3] > 0.0).count();
        assert_eq!(outside, 0, "nothing drawn outside the selection");
    }

    #[test]
    fn an_active_selection_becomes_the_new_layers_mask() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let id = r["layer"].as_u64().unwrap();
        let mask = s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.clone().expect("the selection becomes the layer's mask");
        assert!(mask.enabled);
        // Inside the rect the mask reveals, outside it hides — the tone keeps the selection's shape.
        for (x, y, want) in [(20, 40, 1.0), (39, 71, 1.0), (50, 40, 0.0), (80, 50, 0.0)] {
            assert_eq!(mask.value(x, y), want, "mask at ({x}, {y})");
        }
        // The selection itself is untouched: re-rendering keeps the shape bound to the layer.
        assert!(s.active().unwrap().doc.selection.is_some());
    }

    #[test]
    fn a_layer_without_selection_carries_no_mask() {
        let mut s = session(8);
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let id = r["layer"].as_u64().unwrap();
        assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.is_none());
    }

    #[test]
    fn pattern_command_fills_the_library_without_touching_the_document() {
        for depth in [8, 16, 32] {
            let mut s = session(depth);
            let doc = s.active().unwrap().doc.clone();
            let before = s.patterns.items.len();
            let r = s.execute(PATTERN, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "name": "Mesh"})).unwrap_or_else(|e| panic!("pattern@{depth}: {e}"));
            assert!(r["pattern"].is_string(), "{r}");
            assert_eq!(s.patterns.items.len(), before + 1, "depth {depth}");
            let pat = s.patterns.items.last().unwrap();
            assert_eq!(pat.name, "Mesh");
            assert!(pat.width > 0 && pat.height > 0, "pattern {width}×{height}", width = pat.width, height = pat.height);
            // The tile holds visible tone and matches the document's colour mode.
            let rect = photocraft_geom::Rect::new(0, 0, pat.width as i32, pat.height as i32);
            assert!(pat.surface.read_region(rect).chunks_exact(pat.surface.channels()).any(|q| q.iter().any(|v| *v > 0.5)), "pattern@{depth} drew nothing");
            // The library edit never touched the document itself.
            assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "depth {depth}");
        }
        // Defining the same tile again reuses the pattern instead of duplicating it.
        let mut s = session(8);
        let a = s.execute(PATTERN, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "seed": 7})).unwrap();
        let b = s.execute(PATTERN, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "seed": 7})).unwrap();
        assert_eq!(a["pattern"], b["pattern"], "identical tiles dedupe");
    }

    #[test]
    fn pattern_command_bounds_the_tile_and_names_it() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let preset = algo::screentone::built_in_presets().into_iter().next().unwrap();
        let r = s.execute(PATTERN, json!({"preset": preset.id})).unwrap_or_else(|e| panic!("pattern: {e}"));
        assert!(r["bounds"].is_array(), "{r}");
        let pat = s.patterns.items.last().unwrap();
        assert_eq!(pat.name, preset.name, "the preset's name is the fallback");
        assert_eq!((pat.width, pat.height), (40, 72), "the selection frame bounds the tile");
    }

    #[test]
    fn pattern_command_without_a_document_fails_gracefully() {
        let mut empty = Session::new();
        assert!(empty.execute(PATTERN, json!({})).is_err());
        // Out-of-range and wrong-typed keys fail before anything is defined.
        assert!(empty.active().is_none());
    }

    #[test]
    fn pattern_command_rejects_bad_params_like_the_other_screentone_commands() {
        let mut s = session(8);
        let before = s.patterns.items.len();
        for p in [json!({"patternType": "spiral"}), json!({"dotShape": 3}), json!({"name": 5})] {
            assert!(s.execute(PATTERN, p.clone()).is_err(), "{p}");
            assert_eq!(s.patterns.items.len(), before, "{p}");
        }
    }

    #[test]
    fn restyle_replaces_the_content_keeping_identity_mask_and_one_history_step() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "name": "Mesh"})).unwrap();
        let id = r["layer"].as_u64().unwrap();
        let layer = s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
        let name = layer.name.clone();
        assert!(layer.mask.is_some(), "the generated sheet carries the selection");
        let before = read_layer(&s, id);
        let count = s.active().unwrap().history.entries().len();

        let r2 = s.execute(LAYER, json!({"layer": id, "patternType": "dots", "spacingX": 12, "spacingY": 12})).unwrap();
        assert_eq!(r2["layer"].as_u64().unwrap(), id, "the same layer restyles in place");
        assert!(r2["bounds"].is_array(), "{r2}");
        let state = s.active().unwrap();
        let layer = state.doc.layer(photocraft_doc::LayerId(id)).unwrap();
        assert_eq!(layer.name, name, "the name survives a restyle");
        assert!(layer.mask.is_some(), "the mask survives a restyle");
        assert_eq!(state.history.entries().len(), count + 1, "one restyle = one history step");
        assert_ne!(read_layer(&s, id), before, "the content changes with the new pattern");
        // Restyling a live sheet keeps it procedural: the spec and its document anchor update
        // together, so the cache never becomes a detached raster.
        let expected = ScreentoneSpec { use_source_luminance: false, ..spec(&json!({"patternType": "dots", "spacingX": 12, "spacingY": 12})) };
        let LayerContent::Screentone(st) = &layer.content else { panic!("restyled content is {}", layer.content.kind_name()) };
        assert_eq!(st.spec, expected);
        assert_eq!(st.origin, algo::screentone::origin_doc(&expected, Rect::new(0, 0, 40, 72)));

        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(read_layer(&s, id), before, "undo restores the previous sheet");
    }

    #[test]
    fn restyle_renders_under_the_layers_mask_not_the_current_selection() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let id = r["layer"].as_u64().unwrap();
        // The selection has moved on since the layer was generated: the restyle ignores it and
        // re-renders under the mask the generation left behind.
        s.execute("select.rect", json!({"x": 60, "y": 0, "width": 20, "height": 72})).unwrap();
        let r2 = s.execute(LAYER, json!({"layer": id, "patternType": "dots", "spacingX": 12, "spacingY": 12})).unwrap();
        let b: Vec<i64> = r2["bounds"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        assert_eq!(b, vec![0, 0, 40, 72], "the mask bounds render, not the 20 px selection");
        let state = s.active().unwrap();
        let mask = state.doc.layer(photocraft_doc::LayerId(id)).unwrap().mask.clone().expect("mask intact");
        for (x, y, want) in [(20i32, 40, 1.0f32), (39, 71, 1.0), (50, 40, 0.0), (80, 50, 0.0)] {
            assert_eq!(mask.value(x, y), want, "mask at ({x}, {y}) is untouched");
        }
        let px = read_layer(&s, id).as_chunks::<4>().0.to_vec();
        assert!(
            !px.iter().enumerate().any(|(i, q)| (i % 96) > 41 && q[3] > 0.0),
            "nothing appears outside the mask bounds",
        );
    }

    #[test]
    fn restyle_fails_cleanly_for_unknown_or_non_pixel_layers() {
        let mut s = session(8);
        let e = s.execute(LAYER, json!({"layer": 9999, "patternType": "dots"})).unwrap_err();
        assert!(matches!(e, EngineError::Other(ref m) if m.contains("no layer 9999")), "{e}");
        // A group is not a pixel layer to restyle.
        s.execute("layer.new.group", json!({"name": "Group"})).unwrap();
        let gid = s.active().unwrap().active_layer.unwrap().0;
        let count = s.active().unwrap().history.entries().len();
        let e = s.execute(LAYER, json!({"layer": gid, "patternType": "dots"})).unwrap_err();
        assert!(matches!(e, EngineError::Other(ref m) if m.contains("not a pixel layer")), "{e}");
        assert_eq!(s.active().unwrap().history.entries().len(), count, "no history for refused restyles");
    }

    #[test]
    fn a_generated_sheet_is_live_screentone_content_with_a_document_origin() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 8, "y": 6, "width": 64, "height": 48})).unwrap();
        let r = s.execute(
            LAYER,
            json!({"patternType": "dots", "spacingX": 12, "spacingY": 12, "originX": 3.0, "originY": -2.0, "rotCanvas": 30.0}),
        )
        .unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let LayerContent::Screentone(st) = &layer.content else { panic!("generated content is {}", layer.content.kind_name()) };
        assert!(layer.is_screentone());
        assert_eq!(layer.blend, photocraft_color::BlendMode::Multiply, "manga tones darken what is below");
        assert!(!st.spec.use_source_luminance, "a generated sheet has no source to shade by");
        assert_eq!(st.origin, algo::screentone::origin_doc(&st.spec, Rect::new(8, 6, 72, 54)));
        assert!(layer.clone().surface_mut().is_none(), "a plain pixel edit must not detach the cache from the spec");
    }

    #[test]
    fn rasterize_bakes_a_live_screentone_and_keeps_its_mask() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "name": "Tone"})).unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let (name, mask, pixels) = (layer.name.clone(), layer.mask.clone(), read_layer(&s, id.0));
        assert!(layer.is_screentone());
        let count = s.active().unwrap().history.entries().len();

        s.execute("layer.rasterize.layer", json!({"layer": id.0})).unwrap();

        let state = s.active().unwrap();
        let layer = state.doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster(_)), "the live content became pixels");
        assert!(!layer.is_screentone());
        assert_eq!(layer.name, name);
        assert_eq!(layer.mask, mask, "rasterizing does not touch the layer mask");
        assert_eq!(read_layer(&s, id.0), pixels, "the baked pixels are the rendered cache");
        assert_eq!(state.history.entries().len(), count + 1, "one rasterize = one history step");
    }

    #[test]
    fn apply_mask_bakes_a_live_screentone_in_one_step() {
        let mut s = session(8);
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let before = read_layer(&s, id.0);
        let alpha = |x: i32, y: i32| ((y * 96 + x) * 4 + 3) as usize;
        s.execute("select.rect", json!({"x": 20, "y": 20, "width": 30, "height": 20})).unwrap();
        s.execute("layer.layerMask.hideSelection", json!({})).unwrap();
        let count = s.active().unwrap().history.entries().len();

        s.execute("layer.layerMask.apply", json!({})).unwrap();

        let state = s.active().unwrap();
        let layer = state.doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster(_)), "applying to a live tone bakes it");
        assert!(layer.mask.is_none(), "Apply consumes the mask");
        assert_eq!(state.history.entries().len(), count + 1, "rasterize + multiply = one history step");
        let after = read_layer(&s, id.0);
        let hidden = (20..40).flat_map(|y| (20..50).map(move |x| (x, y))).map(|(x, y)| alpha(x, y)).find(|&i| before[i] > 0.0).expect("the tone covers the selection");
        assert_eq!(after[hidden], 0.0, "the hidden selection's alpha is baked away");
        let shown = (40..72).flat_map(|y| (50..96).map(move |x| (x, y))).map(|(x, y)| alpha(x, y)).find(|&i| before[i] > 0.0).expect("the tone covers the canvas");
        assert_eq!(after[shown], before[shown], "revealed pixels keep their alpha");
    }

    #[test]
    fn a_transform_bakes_a_live_screentone_to_pixels() {
        let mut s = session(8);
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 72})).unwrap();
        let r = s.execute(LAYER, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10})).unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let (masked, before) = (layer.mask.as_ref().map(|m| (m.enabled, m.linked)), layer.surface().unwrap().content_bounds());
        assert!(layer.is_screentone());
        let count = s.active().unwrap().history.entries().len();

        s.execute("edit.transform", json!({"layer": id.0, "matrix": [1, 0, 0, 1, 5, 3], "interpolation": "nearest"})).unwrap();

        let state = s.active().unwrap();
        let layer = state.doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster(_)), "a transform the tone's parameters cannot describe bakes it");
        assert!(!layer.is_screentone());
        assert_eq!(layer.mask.as_ref().map(|m| (m.enabled, m.linked)), masked, "transforming keeps the layer mask (a linked one moves with it)");
        let moved = layer.surface().unwrap().content_bounds();
        assert_eq!(moved, Rect::new(before.x0 + 5, before.y0 + 3, before.x1 + 5, before.y1 + 3), "the warped pixels moved");
        assert_eq!(state.history.entries().len(), count + 1, "one transform = one history step");
    }

    #[test]
    fn a_colour_mode_conversion_bakes_a_live_screentone() {
        let mut s = session(8);
        let r = s.execute(LAYER, json!({"patternType": "dots"})).unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let before = read_layer(&s, id.0);
        assert!(before.as_chunks::<4>().0.iter().any(|p| p[3] > 0.0), "the tone drew pixels");

        s.execute("image.mode.cmyk", json!({})).unwrap();

        let layer = s.active().unwrap().doc.layer(id).unwrap();
        assert!(matches!(layer.content, LayerContent::Raster(_)), "the tone bakes when its colours' space changes");
        assert_eq!(layer.surface().unwrap().format().mode, photocraft_color::ColorMode::Cmyk);
        assert!(layer.surface().unwrap().content_bounds().width() > 0, "the baked pixels are still there");
    }

    #[test]
    fn test_create_layer_with_source_luminance_captures_underlying_tones() {
        let mut s = session(8);
        // Paint dark ink on the left half (x: 0..48) and leave the right half (x: 48..96) blank/white.
        let active_id = s.active().unwrap().active_layer.unwrap();
        s.edit("Paint Gradient", |doc, _| {
            let surf = doc.layer_mut(active_id).unwrap().surface_mut().unwrap();
            let dark_rect = Rect::new(0, 0, 48, 72);
            let dark_px = vec![0.15f32, 0.15, 0.15, 1.0].repeat((48 * 72) as usize);
            surf.write_region(dark_rect, &dark_px);
            Ok(())
        })
        .unwrap();

        let r = s
            .execute(
                LAYER,
                json!({
                    "patternType": "dots",
                    "dotSize": 6.0,
                    "spacingX": 10.0,
                    "spacingY": 10.0,
                    "useSourceLuminance": true
                }),
            )
            .unwrap();
        let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        let layer = s.active().unwrap().doc.layer(id).unwrap();
        let LayerContent::Screentone(st) = &layer.content else {
            panic!("expected Screentone layer, got {}", layer.content.kind_name());
        };
        assert!(st.spec.use_source_luminance, "source luminance remains enabled when underlying artwork has tonal ink");

        let px = read_layer(&s, id.0);
        let mut left_ink = 0.0f32;
        let mut right_ink = 0.0f32;
        for y in 0..72usize {
            for x in 0..96usize {
                let a = px[(y * 96 + x) * 4 + 3];
                if x < 44 {
                    left_ink += a;
                } else if x >= 52 {
                    right_ink += a;
                }
            }
        }
        assert!(left_ink > 100.0, "dark region produces dense halftone dots (got {left_ink})");
        assert!(
            right_ink < left_ink * 0.05,
            "white/transparent region produces no halftone dots (left={left_ink}, right={right_ink})"
        );
    }

    #[test]
    fn test_gizmo_showing_only_when_screentone_layer_active() {
        let mut s = session(8);
        let active_raster = s.active().unwrap().active_layer.unwrap();
        assert!(
            !s.active().unwrap().doc.layer(active_raster).unwrap().is_screentone(),
            "a regular raster layer is not a live screentone"
        );

        let r = s.execute(LAYER, json!({"patternType": "dots"})).unwrap();
        let tone_id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
        assert_eq!(s.active().unwrap().active_layer, Some(tone_id));
        assert!(
            s.active().unwrap().doc.layer(tone_id).unwrap().is_screentone(),
            "newly generated screentone layer is active and is_screentone() == true"
        );
    }

    #[test]
    fn test_smart_filter_screentone_on_smart_object_preserves_smart_type() {
        let mut s = session(8);
        let active_id = s.active().unwrap().active_layer.unwrap();
        s.edit("Paint Midtone", |doc, _| {
            let surf = doc.layer_mut(active_id).unwrap().surface_mut().unwrap();
            let rect = Rect::new(0, 0, 48, 72);
            let px = vec![0.3f32, 0.3, 0.3, 1.0].repeat((48 * 72) as usize);
            surf.write_region(rect, &px);
            Ok(())
        })
        .unwrap();

        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let smart_id = s.active().unwrap().active_layer.unwrap();
        assert!(matches!(
            s.active().unwrap().doc.layer(smart_id).unwrap().content,
            LayerContent::Smart(_)
        ));

        s.execute(
            FILTER,
            json!({
                "patternType": "dots",
                "dotSize": 6.0,
                "spacingX": 10.0,
                "spacingY": 10.0,
                "useSourceLuminance": true
            }),
        )
        .unwrap();

        let layer = s.active().unwrap().doc.layer(smart_id).unwrap();
        assert!(!layer.is_screentone(), "smart object must not become a procedural screentone layer");
        let LayerContent::Smart(so) = &layer.content else {
            panic!("expected LayerContent::Smart, got {}", layer.content.kind_name());
        };
        assert_eq!(so.smart_filters.len(), 1, "screentone filter attached as non-destructive SmartFilter");
        assert_eq!(so.smart_filters[0].command, FILTER);

        let px = read_layer(&s, smart_id.0);
        let mut left_alpha = 0.0f32;
        let mut right_alpha = 0.0f32;
        for y in 0..72usize {
            for x in 0..96usize {
                let a = px[(y * 96 + x) * 4 + 3];
                if x < 44 {
                    left_alpha += a;
                } else if x >= 52 {
                    right_alpha += a;
                }
            }
        }
        assert!(left_alpha > 50.0, "smart filter modulated halftone dots over the painted left half");
        assert_eq!(right_alpha, 0.0, "transparent right half stays clear");
    }

    #[test]
    fn test_shading_mode_vs_sheet_mode_on_artwork() {
        let mut s = session(8);
        let active_id = s.active().unwrap().active_layer.unwrap();
        // Paint 3 vertical bands: beta black (0.02), midtone (0.45), and paper highlight (0.95).
        s.edit("Paint Bands", |doc, _| {
            let surf = doc.layer_mut(active_id).unwrap().surface_mut().unwrap();
            let mut px = vec![0.0f32; 96 * 72 * 4];
            for y in 0..72usize {
                for x in 0..96usize {
                    let v = if x < 32 { 0.02 } else if x < 64 { 0.45 } else { 0.95 };
                    let idx = (y * 96 + x) * 4;
                    px[idx..idx + 4].copy_from_slice(&[v, v, v, 1.0]);
                }
            }
            surf.write_region(Rect::new(0, 0, 96, 72), &px);
            Ok(())
        })
        .unwrap();

        let r = s
            .execute(
                LAYER,
                json!({
                    "mode": "shading",
                    "patternType": "dots",
                    "dotSize": 5.0,
                    "spacingX": 10.0,
                    "spacingY": 10.0,
                    "whiteThreshold": 0.85,
                    "blackThreshold": 0.08,
                    "toneGamma": 1.0,
                    "satelliteEnabled": true,
                    "satelliteSize": 3.0,
                    "satelliteDistance": 4.0
                }),
            )
            .unwrap();
        let id = r["layer"].as_u64().unwrap();
        let px_shading = read_layer(&s, id);
        let sh_slice = px_shading.as_slice();
        let beta_ink: f32 = (0..72usize).flat_map(|y| (0..24usize).map(move |x| sh_slice[(y * 96 + x) * 4 + 3])).sum();
        let mid_ink: f32 = (0..72usize).flat_map(|y| (36..60usize).map(move |x| sh_slice[(y * 96 + x) * 4 + 3])).sum();
        let paper_ink: f32 = (0..72usize).flat_map(|y| (72..96usize).map(move |x| sh_slice[(y * 96 + x) * 4 + 3])).sum();
        assert!(beta_ink > mid_ink, "deep shadow band gets maximum ink dots without binary flat clipping (got {beta_ink} vs midtone {mid_ink})");
        assert!(beta_ink > 500.0, "deep shadow renders full-density halftone dots (got {beta_ink})");
        assert_eq!(paper_ink, 0.0, "highlight paper band above whiteThreshold stays 100% clean");

        // Switching the same layer to `mode: "sheet"` restyles it as a uniform sheet over the paper band too.
        s.execute(
            LAYER,
            json!({
                "layer": id,
                "mode": "sheet",
                "patternType": "dots",
                "dotSize": 5.0,
                "spacingX": 10.0,
                "spacingY": 10.0,
                "satelliteEnabled": true,
                "satelliteSize": 3.0,
                "satelliteDistance": 4.0
            }),
        )
        .unwrap();
        let px_sheet = read_layer(&s, id);
        let sheet_slice = px_sheet.as_slice();
        let sheet_paper_ink: f32 = (0..72usize).flat_map(|y| (72..96usize).map(move |x| sheet_slice[(y * 96 + x) * 4 + 3])).sum();
        assert!(sheet_paper_ink > 50.0, "Sheet mode draws uniform pattern over paper highlights too (got {sheet_paper_ink})");
    }
}
