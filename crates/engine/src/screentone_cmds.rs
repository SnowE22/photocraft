//! Manga & comic screentone generator (`filter.screentone`, `layer.screentone`) backed by
//! [`photocraft_algo::screentone`] (the GenTonik engine).
//!
//! `filter.screentone` runs through [`crate::filters::run_filter`] like every other filter, so
//! selection, smart objects, history and `filter.lastFilter` work the same way; with
//! `useSourceLuminance` it modulates the raster by the layer's own darkness (manga shading).
//! `layer.screentone` renders a procedural sheet onto a **new** pixel layer (no source needed),
//! which is what the Screentones panel uses to drop a tone onto the document.
//!
//! Both commands take the same flat parameter object (the dialog's notation), so a preset saved
//! from one is replayable through the other.

use photocraft_algo::{self as algo, DotShape, FilterParams, PatternType, ScreentoneGradientType, ScreentoneSpec};
use photocraft_doc::{Layer, LayerContent};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Applies the screentone to the active pixel layer (Filter menu).
pub const FILTER: &str = "filter.screentone";
/// Renders a procedural screentone sheet onto a new pixel layer (Screentones panel).
pub const LAYER: &str = "layer.screentone";

const PATTERNS: &str = "dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles";
const SHAPES: &str = "circle|square|diamond|hexagon";
const GRADIENTS: &str = "none|linear|radial";

/// The flat parameter notation shared by both commands (Photoshop-style dialog units).
const PARAMS: &str = r##"{"preset":text,"patternType":"dots|lines|crosshatch|noise|gaussianNoise|stipple|hexgrid|checker|concentric|stars|hearts|triangles","dotShape":"circle|square|diamond|hexagon","dotSize":0..500=6,"spacingX":1..2000=16,"spacingY":1..2000=16,"density":0..1=0.5,"angle":-360..360=0,"lineWidth":0..200=1.5,"crossAngle":-360..360=45,"roundness":0..1=0,"smoothness":0..1=1,"useSourceLuminance":bool=true,"satelliteEnabled":bool,"satelliteSize":0..500=4,"satelliteDistance":0..2000=18,"satelliteCount":1..4=4,"satelliteAngle":-360..360=0,"satelliteShape":"circle|square|diamond|hexagon","satelliteStretch":0..10=0,"aspectX":0.05..20=1,"aspectY":0.05..20=1,"rowOffset":-1..1=0,"mergeFactor":0..1=0,"jitterPosition":0..100=0,"jitterSize":0..100=0,"gradientType":"none|linear|radial","gradientAngle":-360..360=90,"gradientReverse":bool,"gradientMidpoint":0..1=0.5,"gradientSizeStart":0..10=1,"gradientSizeEnd":0..10=0,"gradientStretchStart":0..10=1,"gradientStretchEnd":0..10=1,"gradientColorTransition":bool,"foreground":color=#000000,"background":color,"rotateCanvas":-360..360=0,"rotatePattern":-360..360=0,"seed":u32=42,"layer":id?,"name":text?} → {layer,bounds}"##;

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

/// The screentone parameters for `p`, clamped into the ranges [`PARAMS`] documents. A named
/// `preset` seeds every field the call does not set explicitly.
pub fn spec(p: &Value) -> ScreentoneSpec {
    let base = p
        .get("preset")
        .and_then(Value::as_str)
        .and_then(|id| algo::screentone::built_in_presets().into_iter().find(|pr| pr.id == id))
        .map_or_else(ScreentoneSpec::default, |pr| pr.spec);
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
        use_source_luminance: b_any(p, &["useSourceLuminance"], base.use_source_luminance),
        smoothness: f_any(p, &["smoothness"], base.smoothness).clamp(0.0, 1.0),
        seed: whole(p, "seed", base.seed),
    }
}

/// The algorithm parameters for the screentone ids.
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    matches!(id, FILTER | LAYER).then(|| FilterParams::Screentone { spec: spec(p) })
}

/// The choices of the enumerated parameters.
fn choices(key: &str) -> Option<&'static str> {
    match key {
        "patternType" => Some(PATTERNS),
        "dotShape" | "satelliteShape" => Some(SHAPES),
        "gradientType" => Some(GRADIENTS),
        _ => None,
    }
}

/// Rejects unknown choices and wrong-typed structured params for a new invocation. Every number
/// is clamped in [`spec`], so an out-of-range value can never panic or corrupt a document.
pub(crate) fn validate_params(id: &str, p: &Value) -> Result<()> {
    if !matches!(id, FILTER | LAYER) {
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

/// Renders a procedural screentone sheet onto a new pixel layer above the active one.
fn create_layer(s: &mut Session, p: &Value) -> Result<Value> {
    validate_params(LAYER, p)?;
    let mut spec = spec(p);
    // A generated sheet has no source luminance to modulate by: always procedural.
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
    let area = algo::output_area(&params, bounds, bounds, sel_bounds);
    if area.is_empty() {
        return Err(EngineError::Other("nothing to render: the target area is empty".into()));
    }
    let rendered = algo::apply(&Surface::new(st.doc.pixel_format()), &params, area, bounds, None);

    let name = p
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            p.get("preset")
                .and_then(Value::as_str)
                .and_then(|id| algo::screentone::built_in_presets().into_iter().find(|pr| pr.id == id).map(|pr| pr.name.to_string()))
        })
        .unwrap_or_else(|| "Screentone".to_string());
    let id = s.edit("Screentone", |doc, active| {
        let l = Layer::new(doc.next_layer_name(&name), LayerContent::Raster(rendered));
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({
        "layer": id.0,
        "bounds": [area.x0, area.y0, area.x1, area.y1],
    }))
}

/// The screentone commands: the Filter-menu filter and the panel's layer generator.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILTER,
            label: "Screentone…",
            menu: &["Filter", "Pixelate"],
            shortcut: None,
            params: PARAMS,
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
            // The filter edits the active layer in place, as one history step.
            let count = s.active().unwrap().history.entries().len();
            s.execute(FILTER, json!({"patternType": "lines", "useSourceLuminance": false})).unwrap_or_else(|e| panic!("filter@{depth}: {e}"));
            assert_eq!(s.active().unwrap().history.entries().len(), count + 1);
            s.execute("edit.undo", json!({})).unwrap();
            s.execute("edit.undo", json!({})).unwrap();
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
}
