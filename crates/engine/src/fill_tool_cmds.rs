//! The Fill tool (`paint.fill`), after Clip Studio Paint's: a click fill that reads other layers,
//! closes gaps in line art and tucks under it (Area Scaling), and its Enclose and Fill (a lasso)
//! and Paint Unfilled Area (a painted stroke), which fill every enclosed area they cover.
//! `layer.setReference` marks the layers Reference › Reference Layers reads.

use photocraft_algo::fill::{self, FillOptions, Scaling, Target};
use photocraft_algo::paint::FillPaint;
use photocraft_algo::selection::{Region, rgba8_image};
use photocraft_color::BlendMode;
use photocraft_doc::{LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str, color_param, layer_param};
use crate::{EngineError, Result, Session};

pub const FILL_ID: &str = "paint.fill";
pub const REFERENCE_ID: &str = "layer.setReference";

/// The most lasso or stroke points a fill takes.
const MAX_POINTS: usize = 100_000;

/// What the fill reads to find its area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reference {
    /// The layer being filled, alone (Clip Studio's Refer Only Editing Layer).
    #[default]
    Layer,
    /// Every visible layer.
    All,
    /// The selected layers of the Layers panel.
    Selected,
    /// The layers in the editing layer's group.
    Folder,
    /// The layers marked with `layer.setReference`.
    Reference,
}

impl Reference {
    pub const NAMES: [&str; 5] = ["layer", "all", "selected", "folder", "reference"];

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "layer" => Some(Self::Layer),
            "all" => Some(Self::All),
            "selected" => Some(Self::Selected),
            "folder" => Some(Self::Folder),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// Layers left out of a multi-layer reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Exclude {
    /// The layer being filled.
    layer: bool,
    text: bool,
    locked: bool,
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: FILL_ID.into(), msg: msg.into() }
}

fn number(p: &Value, key: &str, default: f64, range: std::ops::RangeInclusive<f64>) -> Result<f32> {
    Ok(crate::eraser_cmds::number(p, FILL_ID, key, default, range)? as f32)
}

fn flag(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("`{key}` must be true or false"))),
    }
}

fn name<'a>(p: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_str().ok_or_else(|| bad(format!("`{key}` must be a string"))),
    }
}

/// `[[x, y], …]` under `key`, if given.
fn points(p: &Value, key: &str) -> Result<Option<Vec<(f32, f32)>>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let a = v.as_array().ok_or_else(|| bad(format!("`{key}` must be a list of [x, y] points")))?;
    if a.len() > MAX_POINTS {
        return Err(bad(format!("`{key}` has more than {MAX_POINTS} points")));
    }
    a.iter()
        .map(|pt| {
            let xy = pt.as_array().filter(|xy| xy.len() >= 2);
            let c = |i: usize| xy.and_then(|xy| xy.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
            c(0).zip(c(1)).ok_or_else(|| bad(format!("`{key}` must be a list of [x, y] points")))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn options(p: &Value) -> Result<FillOptions> {
    let scaling = name(p, "scalingMode", "darkest")?;
    Ok(FillOptions {
        tolerance: number(p, "tolerance", 10.0, 0.0..=100.0)?,
        contiguous: flag(p, "contiguous", true)?,
        close_gap: number(p, "closeGap", 0.0, 0.0..=f64::from(fill::MAX_GAP))?,
        scale: number(p, "areaScaling", 0.0, -f64::from(fill::MAX_SCALE)..=f64::from(fill::MAX_SCALE))?,
        scaling: Scaling::from_name(scaling).ok_or_else(|| bad("`scalingMode` must be darkest, round or square"))?,
        anti_alias: flag(p, "antiAlias", true)?,
        image_border: flag(p, "imageBorder", false)?,
    })
}

fn exclusions(p: &Value) -> Result<Exclude> {
    let mut ex = Exclude::default();
    let Some(v) = p.get("exclude").filter(|v| !v.is_null()) else { return Ok(ex) };
    for item in v.as_array().ok_or_else(|| bad("`exclude` must be a list"))? {
        match item.as_str() {
            Some("layer") => ex.layer = true,
            Some("text") => ex.text = true,
            Some("locked") => ex.locked = true,
            _ => return Err(bad("`exclude` takes \"layer\", \"text\" and \"locked\"")),
        }
    }
    Ok(ex)
}

/// The pixels the fill reads, over the canvas: the target's own for [`Reference::Layer`], else the
/// composite of the referenced layers (hidden layers are never read).
fn reference_image(s: &Session, r: Reference, ex: Exclude, p: &Value) -> Result<(Rect, Vec<[u8; 4]>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    if r == Reference::Layer {
        let surf = crate::channel_cmds::read_target_surface(&d.doc, d.active_layer, p)?;
        return Ok((area, rgba8_image(surf, area)));
    }
    let active = d.active_layer;
    let folder = active.and_then(|id| d.doc.path_of(id)).map(|mut path| {
        path.pop();
        path
    });
    let reference_groups: Vec<Vec<usize>> = d.doc.walk().into_iter().filter(|(_, _, l)| l.reference && l.children().is_some()).map(|(path, _, _)| path).collect();
    let mut doc = (*d.doc).clone();
    let mut any = false;
    for (path, _, l) in d.doc.walk() {
        if l.children().is_some() {
            continue;
        }
        let referenced = match r {
            Reference::Layer | Reference::All => true,
            Reference::Selected => d.selected_layers.contains(&l.id) || Some(l.id) == active,
            Reference::Folder => folder.as_ref().is_some_and(|f| path.starts_with(f)),
            Reference::Reference => l.reference || reference_groups.iter().any(|g| path.starts_with(g)),
        };
        let left_out = (ex.layer && Some(l.id) == active)
            || (ex.text && matches!(l.content, LayerContent::Text(_)))
            || (ex.locked && {
                let locks = d.doc.locks_at(&path);
                locks.all || locks.pixels
            });
        if referenced && !left_out {
            any |= d.doc.shown_at(&path);
        } else if let Some(m) = doc.layer_mut(l.id) {
            m.visible = false;
        }
    }
    if !any {
        return Err(EngineError::Other(
            match r {
                Reference::Reference => "There is no visible reference layer to fill from. Mark the line art with Set as Reference Layer.",
                Reference::Selected => "None of the selected layers is visible to fill from.",
                _ => "There is no visible layer to fill from.",
            }
            .into(),
        ));
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Ok((area, photocraft_compose::render(&doc, area).px.iter().map(|px| px.map(q)).collect()))
}

/// The area a fill covers: around the click, or the enclosed areas inside the lasso or stroke.
fn region(s: &Session, p: &Value) -> Result<Option<Region>> {
    let o = options(p)?;
    let r = Reference::from_name(name(p, "reference", "layer")?).ok_or_else(|| bad(format!("`reference` must be one of {}", Reference::NAMES.join(", "))))?;
    let target = Target::from_name(name(p, "targetColor", "transparent")?).ok_or_else(|| bad("`targetColor` must be transparent or whiteAndTransparent"))?;
    let (lasso, stroke) = (points(p, "lasso")?, points(p, "stroke")?);
    let size = number(p, "size", 20.0, 1.0..=f64::from(fill::MAX_RADIUS) * 2.0)?;
    let seed = match (p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x.floor().clamp(-1e9, 1e9) as i32, y.floor().clamp(-1e9, 1e9) as i32)),
        _ => None,
    };
    if seed.is_none() && lasso.is_none() && stroke.is_none() {
        return Err(bad("give `x` and `y` (a click), `lasso` (Enclose and Fill) or `stroke` (Paint Unfilled Area)"));
    }
    let (area, img) = reference_image(s, r, exclusions(p)?, p)?;
    Ok(if let Some(pts) = lasso {
        fill::enclosed_region(&img, area, &fill::lasso_mask(&pts, area), target, &o)
    } else if let Some(pts) = stroke {
        fill::enclosed_region(&img, area, &fill::stroke_mask(&pts, size / 2.0, area), target, &o)
    } else {
        seed.and_then(|seed| fill::click_region(&img, area, seed, &o))
    })
}

fn run_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let opacity = number(p, "opacity", 100.0, 1.0..=100.0)? / 100.0;
    let blend = match name(p, "mode", "normal")? {
        "normal" => BlendMode::Normal,
        m => blend_from_str(m).ok_or_else(|| bad(format!("unknown blend mode `{m}`")))?,
    };
    let contents = name(p, "contents", "foreground")?;
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    let (paint, c) = match contents {
        "foreground" => (FillPaint::Blend(blend), color_param(p, "color", fg)),
        "background" => (FillPaint::Blend(blend), color_param(p, "color", bg)),
        "color" => (FillPaint::Blend(blend), p.get("color").map(|_| color_param(p, "color", fg)).ok_or_else(|| bad("`contents` color needs a `color`"))?),
        "transparent" => (FillPaint::Erase, [0.0; 4]),
        _ => return Err(bad("`contents` must be foreground, background, color or transparent")),
    };
    let Some(region) = region(s, p)? else { return Ok(json!({ "filled": false })) };
    // A colour picked as CMYK fills a CMYK document with the picked inks (#4196).
    let inks = s.tools.picked_cmyk([c[0], c[1], c[2]]).filter(|_| c[3] >= 1.0 && paint == FillPaint::Blend(BlendMode::Normal));
    s.edit("Fill Tool", |doc, active| {
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        photocraft_algo::paint::fill_region(surf, &region, opacity, paint, sel.as_ref(), lock, |_, _| c, inks);
        surf.prune();
        Ok(())
    })?;
    Ok(json!({ "filled": true, "bounds": [region.bbox.x0, region.bbox.y0, region.bbox.x1, region.bbox.y1] }))
}

/// Marks (or unmarks) a layer as a reference layer; `on` omitted toggles it.
fn set_reference(s: &mut Session, p: &Value) -> Result<Value> {
    let id: LayerId = layer_param(s, p)?;
    let bad = |msg: &str| EngineError::BadParams { cmd: REFERENCE_ID.into(), msg: msg.into() };
    let on = match p.get("on") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad("`on` must be true or false"))?),
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let now = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?.reference;
    let on = on.unwrap_or(!now);
    if on != now {
        s.edit(if on { "Set as Reference Layer" } else { "Clear Reference Layer" }, |doc, _| {
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.reference = on;
            Ok(())
        })?;
    }
    Ok(json!({ "reference": on }))
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: FILL_ID,
            label: "Fill Tool",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px (a click) | "lasso":[[x,y],…] (Enclose and Fill) | "stroke":[[x,y],…] with "size":px=20 (Paint Unfilled Area),"reference":"layer|all|selected|folder|reference"="layer","exclude":["layer"|"text"|"locked"]? (multi-layer references),"tolerance":0..100=10 (% colour margin),"contiguous":bool=true (Follow adjacent pixels),"closeGap":0..100=0 (px),"areaScaling":-100..100=0 (px),"scalingMode":"darkest|round|square"="darkest","antiAlias":bool=true,"imageBorder":bool=false,"targetColor":"transparent|whiteAndTransparent"="transparent" (lasso/stroke),"contents":"foreground|background|color|transparent"="foreground","color":"#rrggbb"?,"opacity":1..100=100,"mode":"normal|multiply|…"="normal","target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target} → {"filled":bool,"bounds":[x0,y0,x1,y1]?}"##,
            enabled: crate::commands::has_paintable,
            run: run_fill,
            journal: true,
        },
        CommandSpec {
            id: REFERENCE_ID,
            label: "Set as Reference Layer",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?=active,"on":bool? (omitted: toggles)} → {"reference":bool}"##,
            enabled: has_layer,
            run: set_reference,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
