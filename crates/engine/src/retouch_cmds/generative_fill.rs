//! Edit › Generative Fill: the selection filled by a local generative model, on a new layer.
//!
//! As in Photoshop, the fill sees every visible layer (the composite) and lands on a new layer
//! above the active one ("Generative Layer"), shaped by the selection, so the picture under it is
//! untouched and the fill can be masked, moved or deleted. The default model is the diffusion one
//! (`photocraft_ml::diffusion`), which draws what most likely belongs there, with an optional
//! `prompt`; `model: "lama"` uses the fast one.
//!
//! It is also how a panel is extended past its edge (outpainting): enlarge the canvas (Image ›
//! Canvas Size or the Crop tool), select the new, empty area and Generative Fill. Transparent
//! pixels around the selection are unknown to the model rather than a reason to refuse.

use super::remove::{AiModel, diffusion_settings, fill_with_model};
use super::*;
use photocraft_color::ColorMode;
use photocraft_doc::Layer;
use photocraft_ml::MlError;
use photocraft_ml::inpaint::{Colours, Request};

const CMD: &str = "edit.generativeFill";

/// Largest selection, in pixels of its bounding box.
const MAX_PIXELS: u64 = 64 << 20;

/// Selected at least this much counts as filled.
const SELECTED: f32 = 0.5;

pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.selection.is_none() {
        return Err("make a selection first".into());
    }
    match d.doc.mode {
        ColorMode::Rgb | ColorMode::Indexed | ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => Ok(()),
        _ => Err("Generative Fill works on RGB and Grayscale images".into()),
    }
}

/// What the fill works on, checked before the job starts.
struct Plan {
    window: Rect,
    hole: Vec<bool>,
    alpha: Vec<f32>,
    model: AiModel,
    /// LaMa's Quality mode (`quality: "high"`).
    high: bool,
    settings: photocraft_ml::diffusion::Settings,
    above: Option<LayerId>,
    name: String,
}

fn plan(s: &Session, p: &Value) -> Result<Plan> {
    let model = match p.get("model") {
        None | Some(Value::Null) => AiModel::Diffusion,
        Some(_) => AiModel::parse(p, CMD)?,
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let sel = d.doc.selection.as_ref().ok_or_else(|| bad(CMD, "make a selection first"))?;
    let canvas = d.doc.bounds();
    let bounds = sel.content_bounds().intersect(&canvas);
    if bounds.is_empty() {
        return Err(bad(CMD, "the selection is outside the canvas"));
    }
    let pixels = u64::from(bounds.width()) * u64::from(bounds.height());
    if pixels > MAX_PIXELS {
        return Err(bad(CMD, format!("the selection is too large ({pixels} pixels, at most {MAX_PIXELS})")));
    }
    let alpha = super::patch::coverage(sel, bounds);
    let hole: Vec<bool> = alpha.iter().map(|c| *c >= SELECTED).collect();
    if !hole.iter().any(|h| *h) {
        return Err(bad(CMD, "nothing is selected enough to fill (the selection is under 50 %)"));
    }
    if let Err(e) = model.availability() {
        return Err(bad(CMD, format!("Generative Fill needs a local model: {e}")));
    }
    let prompt = p.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let name = if prompt.is_empty() { d.doc.next_layer_name("Generative Layer") } else { prompt.chars().take(60).collect() };
    Ok(Plan { window: bounds, hole, alpha, model, high: super::remove::high_quality(p, CMD)?, settings: diffusion_settings(p), above: d.active_layer, name })
}

pub(super) fn generative_fill(s: &mut Session, p: &Value) -> Result<Value> {
    let plan = plan(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let label = "Generative Fill";
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.progress(0.0, label);
            let base = doc.pixel_format();
            // RGB or grey with alpha: the composite, transparent where nothing is.
            let fmt = PixelFormat::new(base.mode, base.sample, true);
            let colours = match base.mode {
                ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone => Colours::Gray,
                _ => Colours::Rgb,
            };
            let img = composite_region(&doc, None, SampleLayers::All, plan.window, fmt);
            let req = Request {
                canvas: doc.bounds(),
                window: plan.window,
                hole: &plan.hole,
                pixels: &img.data,
                ch: img.ch,
                colours,
                alpha: alpha_index(&fmt),
                linear: fmt.sample == SampleType::F32,
                transparent_unknown: true,
            };
            let message = if plan.model.ready() { label } else { "Generative Fill: loading the model (the first time takes up to a minute)" };
            let mut device = None;
            let out = ctx.stage(0.0, 1.0, message, |ctl| {
                fill_with_model(plan.model, plan.high, &plan.settings, &req, &mut |r| composite_region(&doc, None, SampleLayers::All, r, fmt).data, ctl, &mut device)
            });
            let mut data = match out {
                Ok(d) => d,
                Err(MlError::Cancelled) => return Err(EngineError::Cancelled),
                Err(e) => return Err(bad(CMD, format!("Generative Fill failed: {e}"))),
            };
            clamp_samples(&fmt, &mut data);
            // The layer: the fill where selected, faded by a soft selection's edge.
            let ch = img.ch;
            for ((px, a), h) in data.chunks_exact_mut(ch).zip(&plan.alpha).zip(&plan.hole) {
                if let Some(last) = px.last_mut() {
                    *last = if *h || *a > 0.0 { a.clamp(0.0, 1.0) } else { 0.0 };
                }
            }
            Ok((Region { rect: plan.window, ch, data }, fmt, plan.above, plan.name, device))
        },
        move |s, (region, fmt, above, name, device)| {
            let (damage, id) = s.edit(label, |doc, active| {
                let mut l = Layer::raster(name.clone(), fmt);
                crate::pixels_mut(&mut l)?.write_region(region.rect, &region.data);
                let id = doc.insert_above(above, l);
                *active = Some(id);
                Ok((region.rect, id))
            })?;
            if let Some(st) = s.active_mut() {
                st.last_damage = Some(damage);
            }
            Ok(json!({ "damage": damage_json(damage), "layer": id.0, "device": device.map(|d| d.name()) }))
        },
    )
}
