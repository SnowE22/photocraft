//! The Fill tool (`paint.fill`), after Clip Studio Paint's: a click fill that reads other layers,
//! closes gaps in line art and tucks under it (Area Scaling), and its Enclose and Fill (a lasso)
//! and Paint Unfilled Area (a painted stroke), which fill every enclosed area they cover.
//! `layer.setReference` marks the layers Reference › Reference Layers reads.
//!
//! The composite of the referenced layers is kept between clicks ([`ReferenceCache`]) while the
//! layers it was made from are unchanged: colouring line art clicks many times on the same lines,
//! and compositing a large page is most of what a click costs.

use std::sync::{Arc, PoisonError};

use photocraft_algo::fill::{self, FillOptions, Scaling, Target};
use photocraft_algo::paint::FillPaint;
use photocraft_algo::selection::{Region, rgba8_image};
use photocraft_color::BlendMode;
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::{Surface, Tile};
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
        match_color: None,
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

/// A surface's tiles, held (so their addresses can't be reused) and compared by identity: tiles
/// are shared copy-on-write, so the same tiles mean the same pixels.
struct Tiles {
    tiles: Vec<(TileCoord, Arc<Tile>)>,
    default: Vec<u32>,
    format: photocraft_color::PixelFormat,
}

impl Tiles {
    fn of(s: &Surface) -> Self {
        Tiles { tiles: s.tiles().map(|(c, t)| (*c, t.clone())).collect(), default: s.default_pixel().iter().map(|v| v.to_bits()).collect(), format: s.format() }
    }
}

impl PartialEq for Tiles {
    fn eq(&self, o: &Self) -> bool {
        self.format == o.format
            && self.default == o.default
            && self.tiles.len() == o.tiles.len()
            && self.tiles.iter().zip(&o.tiles).all(|((a, x), (b, y))| a == b && Arc::ptr_eq(x, y))
    }
}

impl Tiles {
    /// Where `self` and `o` differ: the tiles one has and the other hasn't or holds other pixels
    /// in. `None` when their format or default pixel differs (everywhere, then).
    fn changed(&self, o: &Tiles) -> Option<Vec<Rect>> {
        if self.format != o.format || self.default != o.default {
            return None;
        }
        let (mut a, mut b) = (self.tiles.iter().peekable(), o.tiles.iter().peekable());
        let mut out = Vec::new();
        loop {
            match (a.peek(), b.peek()) {
                (Some((ca, ta)), Some((cb, tb))) if ca == cb => {
                    if !Arc::ptr_eq(ta, tb) {
                        out.push(ca.rect());
                    }
                    a.next();
                    b.next();
                }
                (Some((ca, _)), Some((cb, _))) if ca < cb => {
                    out.push(ca.rect());
                    a.next();
                }
                (Some(_), Some((cb, _))) => {
                    out.push(cb.rect());
                    b.next();
                }
                (Some((ca, _)), None) => {
                    out.push(ca.rect());
                    a.next();
                }
                (None, Some((cb, _))) => {
                    out.push(cb.rect());
                    b.next();
                }
                (None, None) => return Some(out),
            }
        }
    }
}

/// What a layer's content is, apart from its pixels.
#[derive(PartialEq)]
enum ContentKey {
    Raster,
    /// Type and Smart Objects composite as their rendered pixels (the key's tiles); type mixes
    /// its edges with the text gamma.
    Text,
    Smart,
    /// A shape's effects and its stroke over clipped layers follow its path: the shape without
    /// its pixels, which are the key's tiles.
    Shape(Box<photocraft_doc::ShapeLayer>),
    Group(Option<photocraft_doc::Artboard>),
    Adjustment(photocraft_doc::Adjustment),
    Fill(photocraft_doc::Fill),
}

/// Everything about a shown layer that the composite reads, except its pixels and its mask's.
#[derive(PartialEq)]
struct Props {
    blend: BlendMode,
    opacity: u32,
    fill: u32,
    clipped: bool,
    excluded_channels: u32,
    blend_if: photocraft_doc::BlendIf,
    advanced: photocraft_doc::AdvancedBlending,
    effects: photocraft_doc::Effects,
    vector_mask: Option<photocraft_doc::vector::VectorMask>,
    mask: Option<(bool, u32, u32)>,
    content: ContentKey,
}

/// What a referenced composite was made from, per layer in stacking order: where it sits,
/// whether it shows, its properties and its tiles (and its mask's and Photoshop-rendered fill's).
#[derive(PartialEq)]
struct LayerKey {
    id: LayerId,
    path: Vec<usize>,
    shown: bool,
    props: Option<Props>,
    tiles: Option<Tiles>,
    mask: Option<Tiles>,
    fill_cache: Option<(photocraft_doc::Fill, Tiles)>,
    /// A change to its pixels or mask changes the composite only there: no effects on it or the
    /// groups it is in, no mask feather.
    local: bool,
}

#[derive(PartialEq)]
struct ReferenceKey {
    doc: photocraft_doc::DocId,
    area: Rect,
    mode: photocraft_color::ColorMode,
    depth: photocraft_color::SampleType,
    layers: Vec<LayerKey>,
}

/// The last referenced composite and what it was made from.
pub(crate) struct ReferenceCache {
    key: ReferenceKey,
    img: Arc<Vec<[u8; 4]>>,
}

/// The key of `doc` (with the layers left out of the reference already hidden), or `None` when a
/// shown layer is video, whose look the key can't see cheaply: those documents are composited
/// every time.
fn reference_key(doc: &Document, area: Rect) -> Option<ReferenceKey> {
    let mut layers = Vec::new();
    // The groups above the current layer that have effects: what they draw follows their content.
    let mut fx_groups: Vec<Vec<usize>> = Vec::new();
    for (path, _, l) in doc.walk() {
        let shown = doc.shown_at(&path);
        fx_groups.retain(|g| path.starts_with(g));
        if !shown {
            layers.push(LayerKey { id: l.id, path, shown, props: None, tiles: None, mask: None, fill_cache: None, local: false });
            continue;
        }
        let (content, tiles) = match &l.content {
            LayerContent::Raster(s) => (ContentKey::Raster, Some(Tiles::of(s))),
            LayerContent::Text(t) => (ContentKey::Text, t.cache.as_ref().map(Tiles::of)),
            LayerContent::Smart(s) => (ContentKey::Smart, s.cache.as_ref().map(Tiles::of)),
            LayerContent::Shape(sh) => {
                let path = photocraft_doc::ShapeLayer { path: sh.path.clone(), fill: sh.fill.clone(), stroke: sh.stroke.clone(), live: sh.live.clone(), cache: None, psd_raw: None };
                (ContentKey::Shape(Box::new(path)), sh.cache.as_ref().map(Tiles::of))
            }
            LayerContent::Group(g) => (ContentKey::Group(g.artboard.clone()), None),
            LayerContent::Adjustment(a) => (ContentKey::Adjustment(a.clone()), None),
            LayerContent::Fill(f) => (ContentKey::Fill(f.clone()), None),
        };
        if l.video.is_some() {
            return None;
        }
        let fx = photocraft_compose::effects::has_effects(l);
        let local = !fx && fx_groups.is_empty() && l.mask.as_ref().is_none_or(|m| m.feather == 0.0);
        if fx && l.children().is_some() {
            fx_groups.push(path.clone());
        }
        let props = Props {
            blend: l.blend,
            opacity: l.opacity.to_bits(),
            fill: l.fill_opacity.to_bits(),
            clipped: l.clipped,
            excluded_channels: l.excluded_channels,
            blend_if: l.blend_if.clone(),
            advanced: l.advanced,
            effects: l.effects.clone(),
            vector_mask: l.vector_mask.clone(),
            mask: l.mask.as_ref().map(|m| (m.enabled, m.density.to_bits(), m.feather.to_bits())),
            content,
        };
        let mask = l.mask.as_ref().map(|m| Tiles::of(&m.surface));
        let fill_cache = l.fill_cache.as_ref().map(|c| (c.fill.clone(), Tiles::of(&c.surface)));
        layers.push(LayerKey { id: l.id, path, shown, props: Some(props), tiles, mask, fill_cache, local });
    }
    Some(ReferenceKey { doc: doc.id, area, mode: doc.mode, depth: doc.depth, layers })
}

/// Where the composite made for `old` differs from the one `new` would make, when only pixels of
/// layers whose changes stay local differ: the box of the changed tiles (empty when nothing
/// changed). `None` when anything else changed, and the whole composite is made again.
fn changed_area(old: &ReferenceKey, new: &ReferenceKey) -> Option<Rect> {
    if old.doc != new.doc || old.area != new.area || old.mode != new.mode || old.depth != new.depth || old.layers.len() != new.layers.len() {
        return None;
    }
    let mut damage = Rect::EMPTY;
    for (a, b) in old.layers.iter().zip(&new.layers) {
        if a.id != b.id || a.path != b.path || a.shown != b.shown || a.props != b.props || a.fill_cache != b.fill_cache || a.local != b.local {
            return None;
        }
        for (x, y) in [(&a.tiles, &b.tiles), (&a.mask, &b.mask)] {
            let rects = match (x, y) {
                (Some(x), Some(y)) => x.changed(y)?,
                (None, None) => continue,
                _ => return None,
            };
            if !rects.is_empty() && !a.local {
                return None;
            }
            for r in rects {
                damage = damage.union(&r);
            }
        }
    }
    Some(damage.intersect(&new.area))
}

/// How many pixels `r` covers.
fn pixels(r: Rect) -> u64 {
    u64::from(r.width()) * u64::from(r.height())
}

/// `img`, the composite of `doc` over `area`, made again over `d` (inside `area`).
fn patch(img: &mut [[u8; 4]], area: Rect, doc: &Document, d: Rect) {
    let part = composite_rgba8(doc, d);
    let (w, dw) = (area.width() as usize, d.width() as usize);
    for (row, src) in part.chunks_exact(dw.max(1)).enumerate() {
        let at = (d.y0 - area.y0) as usize + row;
        let x = (d.x0 - area.x0) as usize;
        if let Some(dst) = img.get_mut(at * w + x..at * w + x + dw) {
            dst.copy_from_slice(src);
        }
    }
}

/// The composite of `doc` over `area` as straight RGBA8.
fn composite_rgba8(doc: &Document, area: Rect) -> Vec<[u8; 4]> {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let px = photocraft_compose::render(doc, area).px;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        px.par_iter().map(|p| p.map(q)).collect()
    }
    #[cfg(target_arch = "wasm32")]
    px.iter().map(|p| p.map(q)).collect()
}

/// The pixels the fill reads, over the canvas: the target's own for [`Reference::Layer`], else the
/// composite of the referenced layers (hidden layers are never read), kept for the next click.
fn reference_image(s: &Session, r: Reference, ex: Exclude, p: &Value) -> Result<(Rect, Arc<Vec<[u8; 4]>>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    if r == Reference::Layer {
        let surf = crate::channel_cmds::read_target_surface(&d.doc, d.active_layer, p)?;
        return Ok((area, Arc::new(rgba8_image(surf, area))));
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
    let key = reference_key(&doc, area);
    let mut cache = s.fill_reference.lock().unwrap_or_else(PoisonError::into_inner);
    if let (Some(key), Some(c)) = (key, cache.as_mut()) {
        match changed_area(&c.key, &key) {
            // Unchanged: the kept composite.
            Some(d) if d.is_empty() => return Ok((area, c.img.clone())),
            // Only some pixels changed (the last fill, with Refer to All Layers): those tiles
            // are composited again and patched in.
            Some(d) if pixels(d) * 2 < pixels(area) => {
                patch(Arc::make_mut(&mut c.img).as_mut_slice(), area, &doc, d);
                c.key = key;
                return Ok((area, c.img.clone()));
            }
            _ => {}
        }
    }
    // Let go of the old one before making the next.
    *cache = None;
    let key = reference_key(&doc, area);
    let img = Arc::new(composite_rgba8(&doc, area));
    *cache = key.map(|key| ReferenceCache { key, img: img.clone() });
    Ok((area, img))
}

/// A drag with the Fill tool (or the Magic Wand) going on from its click, `gesture` in the
/// params: Clip Studio fills each further area the pointer reaches that is like the one clicked.
/// It keeps the colour the first click filled and the areas filled so far.
pub(crate) struct Gesture {
    id: String,
    doc: photocraft_doc::DocId,
    color: [u8; 4],
    filled: Vec<Region>,
}

/// The area a fill (or the Magic Wand with the Fill tool's options) covers: around the click,
/// or the enclosed areas inside the lasso or stroke. A click in a drag's `gesture` fills only an
/// area like the first one's that it hasn't filled yet.
pub(crate) fn region(s: &Session, p: &Value) -> Result<Option<Region>> {
    let mut o = options(p)?;
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
    let gesture = match p.get("gesture") {
        None | Some(Value::Null) => None,
        Some(Value::String(g)) if !g.is_empty() && g.len() <= 64 => Some(g.clone()),
        Some(_) => return Err(bad("`gesture` must be a short string")),
    };
    let doc = s.active().map(|d| d.doc.id).ok_or(EngineError::NoDocument)?;
    if let (Some(g), Some((x, y))) = (&gesture, seed) {
        let state = s.fill_gesture.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(st) = state.as_ref().filter(|st| st.id == *g && st.doc == doc) {
            // Already filled by this drag: nothing more to do there (and no composite needed).
            if st.filled.iter().any(|r| r.at(x, y) > 0.5) {
                return Ok(None);
            }
            o.match_color = Some(st.color);
        }
    }
    let (area, img) = reference_image(s, r, exclusions(p)?, p)?;
    let region = if let Some(pts) = lasso {
        fill::enclosed_region(&img, area, &fill::lasso_mask(&pts, area), target, &o)
    } else if let Some(pts) = stroke {
        fill::enclosed_region(&img, area, &fill::stroke_mask(&pts, size / 2.0, area), target, &o)
    } else {
        seed.and_then(|seed| fill::click_region(&img, area, seed, &o))
    };
    if let (Some(g), Some((x, y))) = (gesture, seed) {
        let mut state = s.fill_gesture.lock().unwrap_or_else(PoisonError::into_inner);
        let same = state.as_ref().is_some_and(|st| st.id == g && st.doc == doc);
        if !same {
            let at = area.contains(x, y).then(|| (y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize);
            *state = at.and_then(|i| img.get(i)).map(|c| Gesture { id: g, doc, color: *c, filled: Vec::new() });
        }
        if let (Some(st), Some(r)) = (state.as_mut(), &region)
            // A long drag over specks can't grow this without bound.
            && st.filled.len() < 4096
        {
            st.filled.push(r.clone());
        }
    }
    Ok(region)
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
            params: r##"{"x":px,"y":px (a click) | "lasso":[[x,y],…] (Enclose and Fill) | "stroke":[[x,y],…] with "size":px=20 (Paint Unfilled Area),"reference":"layer|all|selected|folder|reference"="layer","exclude":["layer"|"text"|"locked"]? (multi-layer references),"tolerance":0..100=10 (% colour margin),"contiguous":bool=true (Follow adjacent pixels),"closeGap":0..100=0 (px),"areaScaling":-100..100=0 (px),"scalingMode":"darkest|round|square"="darkest","antiAlias":bool=true,"imageBorder":bool=false,"targetColor":"transparent|whiteAndTransparent"="transparent" (lasso/stroke),"contents":"foreground|background|color|transparent"="foreground","color":"#rrggbb"?,"opacity":1..100=100,"mode":"normal|multiply|…"="normal","target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target,"gesture":str? (a drag going on from its click: later clicks with the same gesture fill only areas like the first one's they haven't filled; pass the same \"coalesce\" to make the drag one undo step)} → {"filled":bool,"bounds":[x0,y0,x1,y1]?}"##,
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
