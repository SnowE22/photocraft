//! Balloon commands: comic tails merged with a shape layer, procedural thought/shout balloons and
//! SAI-style click linework.
//!
//! All geometry comes from `photocraft_vector::balloon` and lands in an ordinary
//! [`ShapeLayer`](photocraft_doc::ShapeLayer) path, so Free Transform, history, PSD export and the
//! rest of the shape tooling keep working unchanged.

use photocraft_doc::{Color, Fill, Layer, LayerContent, LayerId, ShapeLayer};
use photocraft_geom::Point;
use photocraft_vector as vector;
use photocraft_vector::balloon::{self, BalloonTail, CloudParams, SaiPoint, SpikeParams, TailPreset};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::vector_cmds::{bad, has_doc, layer_id, nums, parse_fill, pt, refresh_shape, shape_info, with_shape};
use crate::{EngineError, Result, Session};

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:expr, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// A finite finite-coordinate point parameter.
fn point(cmd: &str, p: &Value, key: &str) -> Result<Point> {
    p.get(key).and_then(|v| pt(v)).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite [x, y] point")))
}

/// Two points from `[[x, y], [x, y]]`.
fn point_pair(v: Option<&Value>) -> Option<[(f64, f64); 2]> {
    let arr = v?.as_array()?;
    let [a, b] = arr.as_slice() else { return None };
    let pa = pt(a)?;
    let pb = pt(b)?;
    Some([(pa.x, pa.y), (pb.x, pb.y)])
}

fn finite_point(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite() && x.abs() <= 1.0e7 && y.abs() <= 1.0e7
}

fn opt_f64(p: &Value, key: &str, cmd: &str) -> Result<Option<f64>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be a finite number"))),
    }
}

fn opt_u32(p: &Value, key: &str, cmd: &str) -> Result<Option<u32>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => u32::try_from(v.as_u64().ok_or_else(|| bad(cmd, format!("`{key}` must be a non-negative integer")))?)
            .map(Some)
            .map_err(|_| bad(cmd, format!("`{key}` is too large"))),
    }
}

fn opt_bool(p: &Value, key: &str, cmd: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(cmd, format!("`{key}` must be true or false"))),
    }
}

/// Projection of `p` onto the segment `a..b`, clamped to the segment.
fn project_on(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 < 1.0e-12 {
        return a;
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (a.0 + dx * t, a.1 + dy * t)
}

/// The point of the shape's contour nearest to `target` (used when a tail has a `tip` but no
/// explicit mouth): the tail then starts on the balloon's edge facing the words.
fn nearest_on_body(s: &Session, id: LayerId, target: Point, cmd: &str) -> Result<(f64, f64)> {
    let doc = s.active().map(|d| &d.doc).ok_or(EngineError::NoDocument)?;
    let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(shape) = &layer.content else {
        return Err(bad(cmd, "layer is not a shape layer"));
    };
    let target = (target.x, target.y);
    let mut best: Option<((f64, f64), f64)> = None;
    for poly in vector::flatten_path(&shape.path, 0.25) {
        if !poly.closed || poly.pts.len() < 2 {
            continue;
        }
        let n = poly.pts.len();
        for i in 0..n {
            let a = poly.pts[i];
            let b = poly.pts[(i + 1) % n];
            let candidate = project_on(a, b, target);
            let d = (candidate.0 - target.0).hypot(candidate.1 - target.1);
            if best.is_none_or(|(_, best_d)| d < best_d) {
                best = Some((candidate, d));
            }
        }
    }
    best.map(|(point, _)| point).ok_or_else(|| bad(cmd, "the shape has no closed contour to attach a tail to"))
}

fn has_shape(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or_else(|| "no document open".to_string())?;
    let id = st.active_layer.ok_or_else(|| "no active layer".to_string())?;
    let layer = st.doc.layer(id).ok_or_else(|| "no active layer".to_string())?;
    if matches!(layer.content, photocraft_doc::LayerContent::Shape(_)) { Ok(()) } else { Err("the active layer is not a shape layer".to_string()) }
}

/// `shape.balloon.tail`: union a tail with the balloon body.
fn balloon_tail(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.balloon.tail";
    let id = layer_id(s, p)?;
    let preset_id = match p.get("preset") {
        None => "wedge",
        Some(v) => v.as_str().ok_or_else(|| bad(CMD, "`preset` must be a string (wedge|arc|double|thought)"))?,
    };
    let preset = TailPreset::from_id(preset_id).ok_or_else(|| bad(CMD, format!("unknown tail preset `{preset_id}` (wedge|arc|double|thought)")))?;
    let tip = point(CMD, p, "tip")?;
    if !finite_point(tip.x, tip.y) {
        return Err(bad(CMD, "`tip` is out of range"));
    }
    let mouth_width = opt_f64(p, "mouthWidth", CMD)?.unwrap_or(18.0);
    if !(0.0..=1.0e4).contains(&mouth_width) {
        return Err(bad(CMD, "`mouthWidth` must be between 0 and 10000 px"));
    }
    let base = match p.get("base") {
        // The mouth is given explicitly.
        Some(v) => point_pair(Some(v)).ok_or_else(|| bad(CMD, "`base` must be [[x, y], [x, y]]"))?,
        // `from` is the drag start on the balloon; without it the mouth sits on the contour point
        // nearest the tip, so a plain `tip` already gives a usable tail.
        None => {
            let from = match p.get("from") {
                Some(_) => {
                    let from = point(CMD, p, "from")?;
                    (from.x, from.y)
                }
                None => nearest_on_body(s, id, tip, CMD)?,
            };
            let tail = BalloonTail::from_drag(preset, from, (tip.x, tip.y), mouth_width);
            if (tail.base[0].0 - tail.base[1].0).abs() < 1.0e-9 && (tail.base[0].1 - tail.base[1].1).abs() < 1.0e-9 {
                return Err(bad(CMD, "`from` and `tip` must not coincide (the tail mouth would be empty)"));
            }
            tail.base
        }
    };
    if !finite_point(base[0].0, base[0].1) || !finite_point(base[1].0, base[1].1) {
        return Err(bad(CMD, "`base` is out of range"));
    }
    let curve = opt_f64(p, "curve", CMD)?.map(|c| c.clamp(0.0, 1.0));
    let bubbles = opt_u32(p, "bubbles", CMD)?.map_or(3, |b| b.clamp(1, 12));
    let bubble_radius = opt_f64(p, "bubbleRadius", CMD)?.unwrap_or(6.0);
    if !(0.5..=200.0).contains(&bubble_radius) {
        return Err(bad(CMD, "`bubbleRadius` must be between 0.5 and 200 px"));
    }
    let mut tail = BalloonTail { preset, base, tip: (tip.x, tip.y), curve: curve.unwrap_or(0.12), bubbles, bubble_radius };
    if let Some(curve) = curve {
        tail.curve = curve;
    }

    with_shape(s, id, "Balloon Tail", |sh, _| {
        let merged = balloon::attach_tail(&sh.path, &tail);
        vector::flatten::validate_shape(&merged, sh.stroke.as_ref()).map_err(|e| bad(CMD, e))?;
        sh.path = merged;
        // A merged outline no longer matches the live-shape parameters that produced the body.
        sh.live = None;
        sh.psd_raw = None;
        Ok(())
    })?;
    shape_info(s, id)
}

/// `shape.balloon.create`: a procedural thought cloud or shout balloon as a new shape layer.
fn balloon_create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.balloon.create";
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("cloud");
    let rect = nums::<4>(p, "rect").ok_or_else(|| bad(CMD, "`rect` must be [x, y, w, h]"))?;
    if rect.iter().any(|v| !v.is_finite() || v.abs() > 1.0e7) || rect[2].abs() < 1.0 || rect[3].abs() < 1.0 {
        return Err(bad(CMD, "`rect` must be finite with a non-zero width and height"));
    }
    let (path, default_name) = match kind {
        "cloud" | "thought" => {
            let params = CloudParams {
                bumps: opt_u32(p, "bumps", CMD)?.map_or(9, |v| v.clamp(3, 64)),
                amplitude: opt_f64(p, "amplitude", CMD)?.map_or(0.18, |v| v.clamp(0.0, 1.0)),
                jitter: opt_f64(p, "jitter", CMD)?.map_or(0.35, |v| v.clamp(0.0, 1.0)),
                seed: opt_u32(p, "seed", CMD)?.unwrap_or(1),
            };
            (balloon::cloud(rect, params), "Thought Cloud")
        }
        "spiky" | "shout" | "flash" => {
            let params = SpikeParams {
                spikes: opt_u32(p, "spikes", CMD)?.map_or(12, |v| v.clamp(3, 128)),
                length: opt_f64(p, "length", CMD)?.map_or(0.45, |v| v.clamp(0.0, 2.0)),
                jitter: opt_f64(p, "jitter", CMD)?.map_or(0.35, |v| v.clamp(0.0, 1.0)),
                inward: opt_bool(p, "inward", CMD)?.unwrap_or(false),
                smooth: opt_bool(p, "smooth", CMD)?.unwrap_or(false),
                seed: opt_u32(p, "seed", CMD)?.unwrap_or(1),
            };
            (balloon::spiky(rect, params), "Shout Balloon")
        }
        other => return Err(bad(CMD, format!("unknown balloon kind `{other}` (cloud|spiky)"))),
    };
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let fill = match p.get("fill") {
        Some(v) => parse_fill(v).map_err(|e| bad(CMD, e))?,
        None => Some(Fill::Solid(fgc)),
    };
    let stroke = match p.get("stroke") {
        Some(v) => crate::vector_cmds::parse_stroke(v, None, fgc).map_err(|e| bad(CMD, e))?,
        None => None,
    };
    vector::flatten::validate_shape(&path, stroke.as_ref()).map_err(|e| bad(CMD, e))?;

    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let mut sh = ShapeLayer { path, fill, stroke, live: None, ..Default::default() };
    let id = s.edit("New Balloon", |doc, active| {
        refresh_shape(doc, &mut sh);
        let name = name.unwrap_or_else(|| doc.next_layer_name(default_name));
        let id = doc.insert_new(*active, Layer::new(name, photocraft_doc::LayerContent::Shape(sh)));
        *active = Some(id);
        Ok(id)
    })?;
    shape_info(s, id)
}

/// One click-placed node of `shape.balloon.stroke`.
fn sai_point(v: &Value) -> Option<SaiPoint> {
    let (x, y, pressure, corner) = if let Some(a) = v.as_array() {
        let x = a.first().and_then(Value::as_f64)?;
        let y = a.get(1).and_then(Value::as_f64)?;
        (x, y, 1.0f64, a.get(2).map_or(false, |c| c.as_bool().unwrap_or(false)))
    } else {
        let x = v.get("x").and_then(Value::as_f64)?;
        let y = v.get("y").and_then(Value::as_f64)?;
        let pressure = v.get("pressure").and_then(Value::as_f64).unwrap_or(1.0);
        let corner = v.get("corner").and_then(Value::as_bool).unwrap_or(false);
        (x, y, pressure, corner)
    };
    if !finite_point(x, y) || !pressure.is_finite() {
        return None;
    }
    Some(SaiPoint { p: (x, y), pressure: pressure.clamp(0.0, 1.0) as f32, corner })
}

/// `shape.balloon.stroke`: Catmull-Rom smoothed click linework, tapered by point pressure.
fn balloon_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shape.balloon.stroke";
    let raw = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(CMD, "`points` must be an array"))?;
    if raw.len() < 2 || raw.len() > 4096 {
        return Err(bad(CMD, "`points` must contain 2..=4096 nodes"));
    }
    let mut points = Vec::with_capacity(raw.len());
    for item in raw {
        points.push(sai_point(item).ok_or_else(|| bad(CMD, "every point needs finite `x`, `y` and `pressure`"))?);
    }
    let width = opt_f64(p, "width", CMD)?.unwrap_or(6.0);
    if !(0.01..=1.0e5).contains(&width) {
        return Err(bad(CMD, "`width` must be between 0.01 and 100000 px"));
    }
    let min_ratio = opt_f64(p, "minRatio", CMD)?.unwrap_or(0.05).clamp(0.0, 1.0);
    let tapered = opt_bool(p, "tapered", CMD)?.unwrap_or(true);
    let fg = s.tools.foreground;
    let fgc = Color::rgba(fg[0], fg[1], fg[2], fg[3]);
    let path = if tapered { balloon::tapered_path(&points, width, min_ratio) } else { balloon::sai_stroke(&points, false) };
    if path.is_empty() {
        return Err(bad(CMD, "the points produce no drawable path"));
    }
    let (fill, stroke) = if tapered {
        let fill = match p.get("fill") {
            Some(v) => parse_fill(v).map_err(|e| bad(CMD, e))?,
            None => Some(Fill::Solid(fgc)),
        };
        (fill, None)
    } else {
        let base = photocraft_doc::ShapeStroke { width: width as f32, paint: Fill::Solid(fgc), ..Default::default() };
        let stroke = match p.get("stroke") {
            Some(v) => crate::vector_cmds::parse_stroke(v, Some(base), fgc).map_err(|e| bad(CMD, e))?,
            None => Some(base),
        };
        (None, stroke)
    };
    vector::flatten::validate_shape(&path, stroke.as_ref()).map_err(|e| bad(CMD, e))?;
    let name = p.get("name").and_then(Value::as_str).unwrap_or("Vector Stroke").to_string();
    let mut sh = ShapeLayer { path, fill, stroke, live: None, ..Default::default() };
    let id = s.edit("Vector Stroke", |doc, active| {
        refresh_shape(doc, &mut sh);
        let id = doc.insert_new(*active, Layer::new(name, photocraft_doc::LayerContent::Shape(sh)));
        *active = Some(id);
        Ok(id)
    })?;
    shape_info(s, id)
}

pub fn specs() -> Vec<CommandSpec> {
    let stroke = r##""stroke":{"width":px,"color":"#rrggbb"|fill,"opacity":0..100,"align":"inside|center|outside","cap":"butt|round|square","join":"miter|round|bevel","miterLimit":n,"dashes":[multiples of width],"dashOffset":n}|null"##;
    let fill = r##""fill":"#rrggbb"|[r,g,b,a]|{"gradient":{…}}|{"pattern":name}|null (no fill)"##;
    vec![
        spec!(
            "shape.balloon.tail",
            "Add Balloon Tail",
            [],
            leak(format!(
                r##"{{"layer":id?,"preset":"wedge|arc|double|thought"="wedge","tip":[x,y] (required),"base":[[x,y],[x,y]]? or "from":[x,y] + "mouthWidth":px=18 (the drag points),"curve":0..1?,"bubbles":1..12=3,"bubbleRadius":px=6 (thought)}} → shape.info. The tail and body are unioned into ONE contour: one fill, one stroke, no seam. {fill} stays as the layer's; the layer must be a shape layer."##
            )),
            has_shape,
            balloon_tail
        ),
        spec!(
            "shape.balloon.create",
            "New Balloon",
            [],
            leak(format!(
                r##"{{"kind":"cloud|spiky"="cloud","rect":[x,y,w,h] (required),"bumps":3..64=9,"amplitude":0..1=0.18,"jitter":0..1=0.35,"spikes":3..128=12,"length":0..2=0.45,"inward":bool=false,"smooth":bool=false,"seed":int=1,{fill}=foreground,{stroke}=none,"name":str?}} → shape.info. cloud = scalloped thought balloon; spiky = shout/flash balloon."##
            )),
            has_doc,
            balloon_create
        ),
        spec!(
            "shape.balloon.stroke",
            "New Vector Stroke",
            [],
            leak(format!(
                r##"{{"points":[{{"x":px,"y":px,"pressure":0..1=1,"corner":bool=false}} | [x,y,corner?]] (2..4096, click nodes smoothed with a Catmull-Rom spline),"width":px=6,"minRatio":0..1=0.05 (taper floor),"tapered":bool=true (filled outline, weight per node) | false (uniform stroke path),"name":str?}} → shape.info"##
            )),
            has_doc,
            balloon_stroke
        ),
        CommandSpec {
            id: "shape.balloon.presets",
            label: "Balloon Tail Presets",
            menu: &[],
            shortcut: None,
            params: r##"{} → {presets:[{id,label,mouth}]}"##,
            enabled: |_: &Session| Ok(()),
            run: |_, _| {
                let names = [(TailPreset::Wedge, "Wedge", "straight tapered wedge"), (TailPreset::Arc, "Arc", "bow-sided tail"), (TailPreset::Double, "Double", "two tails, one mouth"), (TailPreset::Thought, "Thought", "shrinking bubbles")];
                Ok(json!({"presets": names.iter().map(|(p, label, about)| json!({"id": p.id(), "label": label, "mouth": p.mouth(), "about": about})).collect::<Vec<_>>()}))
            },
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
