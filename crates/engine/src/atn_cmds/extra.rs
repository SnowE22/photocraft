//! Photoshop event shapes the first table did not read, found in Photoshop 2024–2026 actions
//! (a user's whole Actions palette, 2026-10-18): Create/Release Clipping Mask, Ungroup Layers,
//! Color Range, the Camera Raw filter, Diffuse, Photocopy, Auto-Align Layers, Fit Image, NTSC
//! Colors, selections combined with Add To / Subtract From / Intersect, and more forms of `set`
//! (the foreground and background colours, Blend If, a mask or the composite as the selection,
//! the Magic Wand, a Solid Color layer's colour, a Color Lookup's table, a Color Overlay),
//! `make` (layer masks, named and dimmed adjustment layers, work paths, vector masks,
//! snapshots), `select` and `delete`.
//!
//! The parent module's rules hold: a step imports to commands that do what Photoshop's does, or
//! stays a Photoshop step (so it fails at playback with its name rather than doing something
//! else). Keys a converter doesn't know make the step unsupported rather than being dropped.

use super::*;

// ------------------------------------------------------------------ set

/// The `set` forms the table doesn't read; `None` leaves the step unsupported.
pub(super) fn set_from(d: &Descriptor) -> Option<Mapped> {
    let r = get_ref(d, "null")?;
    match (ref_class(r).as_deref(), target(r)) {
        // The foreground or background colour (Color panel, picker, eyedropper, swatches).
        (Some("Clr"), Target::Property(p)) if p == "FrgC" || p == "BckC" => {
            let c = get_obj(d, "T   ").and_then(color_hex)?;
            let key = if p == "FrgC" { "foreground" } else { "background" };
            Some(step("tools.setColors", json!({ key: c })))
        }
        (Some("Chnl"), Target::Property(p)) if p == "fsel" => match d.get("T   ")? {
            // ⌘-click on a layer mask, or on the composite channel (a luminosity selection).
            D::Reference(t) => match (ref_class(t).as_deref(), target(t)) {
                (Some("Chnl"), Target::Ordinal(o)) => {
                    let channel = match o.as_str() {
                        "Msk" => "mask",
                        "RGB" | "CMYK" | "Lab" => "composite",
                        _ => return None,
                    };
                    Some(step("select.loadSelection", json!({"channel": channel})))
                }
                _ => None,
            },
            // The Magic Wand: a click at a point.
            D::Descriptor(t) if trimmed(&t.class_id) == "Pnt" => magic_wand(d, t, None),
            _ => None,
        },
        // A Solid Color fill layer's colour (double-click its thumbnail).
        (Some("contentLayer"), Target::Ordinal(o)) if o == "Trgt" => {
            let t = get_obj(d, "T   ").filter(|t| t.class_id.is("solidColorLayer"))?;
            let c = get_obj(t, "Clr ").and_then(color_hex)?;
            Some(step("layer.fill.solidColor.set", json!({"color": c})))
        }
        // A Color Lookup's table, embedded in the step as the file's bytes, or Curves loaded from
        // a preset file.
        (Some("AdjL"), Target::Ordinal(o)) if o == "Trgt" => {
            let t = get_obj(d, "T   ")?;
            if t.class_id.is("colorLookup") {
                return Some(step("layer.setAdjustment", lookup_params(t)?));
            }
            if trimmed(&t.class_id) == "Crvs" && t.get("Usng").is_some() {
                return Some(step("layer.setAdjustment", curves_file_params(t)?));
            }
            None
        }
        // A layer style that is a Color Overlay (`SoFi`) alone.
        (Some("Prpr"), Target::Property(p)) if p == "Lefx" => color_overlay(d, r),
        _ => None,
    }
}

/// `select.magicWand` for a click at `pt` (pixels) with the tolerance, anti-alias, contiguous and
/// sample-merged settings `d` carries, combining with the selection as `mode` says.
fn magic_wand(d: &Descriptor, pt: &Descriptor, mode: Option<&str>) -> Option<Mapped> {
    let px_of = |k: &str| match pt.get(k) {
        Some(D::UnitFloat { unit: PX, value }) => Some(*value),
        _ => None,
    };
    let (x, y) = (px_of("Hrzn")?, px_of("Vrtc")?);
    let mut p = json!({"x": x.round(), "y": y.round()});
    if let Some(v) = get_num(d, "Tlrn") {
        p["tolerance"] = json!(v);
    }
    if let Some(v) = get_bool(d, "AntA") {
        p["antiAlias"] = json!(v);
    }
    if let Some(v) = get_bool(d, "Cntg") {
        p["contiguous"] = json!(v);
    }
    if let Some(v) = get_bool(d, "Mrgd") {
        p["sampleAllLayers"] = json!(v);
    }
    if let Some(m) = mode {
        p["mode"] = json!(m);
    }
    Some(step("select.magicWand", p))
}

/// `set` of a layer's style to a Color Overlay alone (`Lefx` with `SoFi`, at 100 % scale), on
/// the target layer or one by name. Other effects would be dropped, so they stay unsupported.
fn color_overlay(d: &Descriptor, r: &[ReferenceItem]) -> Option<Mapped> {
    let mut p = layer_target(r.get(1..).filter(|rest| !rest.is_empty())?)?;
    let t = get_obj(d, "T   ")?;
    if t.items.iter().any(|(k, _)| !matches!(trimmed(k).as_str(), "Scl" | "SoFi")) {
        return None;
    }
    if get_num(t, "Scl ").is_some_and(|s| (s - 100.0).abs() > 1e-9) {
        return None;
    }
    let fill = get_obj(t, "SoFi")?;
    if get_bool(fill, "enab") == Some(false) {
        return None;
    }
    if fill.items.iter().any(|(k, _)| !matches!(trimmed(k).as_str(), "enab" | "present" | "showInDialog" | "Md" | "Opct" | "Clr")) {
        return None;
    }
    p.insert("color".into(), color_hex(get_obj(fill, "Clr ")?)?.into());
    if let Some(o) = get_num(fill, "Opct") {
        p.insert("opacity".into(), o.into());
    }
    if let Some(m) = get_enum(fill, "Md  ") {
        p.insert("blend".into(), blend_label(&m)?.into());
    }
    Some(step("layer.layerStyle.colorOverlay", Value::Object(p)))
}

/// A Color Lookup descriptor's table as `layer.setAdjustment` / `layer.newAdjustmentLayer.
/// colorLookup` params: Photoshop embeds the `.cube` / `.3dl` / `.look` file's bytes
/// (`LUT3DFileData`) with its name, so the look replays without the file.
fn lookup_params(t: &Descriptor) -> Option<Value> {
    if get_enum(t, "lookupType").is_some_and(|k| k != "3DLUT") {
        return None;
    }
    let Some(D::RawData(bytes)) = t.get("LUT3DFileData") else { return None };
    let text = std::str::from_utf8(bytes).ok()?;
    let path = get_text(t, "LUT3DFileName").or_else(|| get_text(t, "Nm  "))?;
    let file = path.rsplit(['/', '\\']).next().filter(|f| !f.is_empty())?.to_string();
    Some(json!({"data": text, "fileName": file}))
}

/// Curves loaded from a `.acv` preset (the Curves dialog's Load Preset, `Usng` = the file): the
/// file's curves, read when the action is loaded so playback doesn't need the file. The
/// composite curve, and red, green and blue where they aren't the identity.
fn curves_file_params(t: &Descriptor) -> Option<Value> {
    let path = path_text(t.get("Usng")?)?;
    let curves = photocraft_psd::acv::parse(&preset_file(&path, "Curves")?).ok()?;
    let points = |c: &photocraft_psd::acv::Curve| json!(c.iter().map(|&(i, o)| json!([i, o])).collect::<Vec<_>>());
    let mut p = json!({"points": points(curves.first()?)});
    for (key, c) in ["red", "green", "blue"].into_iter().zip(curves.iter().skip(1)) {
        if c.as_slice() != [(0, 0), (255, 255)] {
            p[key] = points(c);
        }
    }
    Some(p)
}

/// Largest preset file read for a step (a `.acv` is under a hundred bytes).
const MAX_PRESET_FILE: u64 = 64 << 10;

/// A preset file an action names, read for the step: where it was recorded, else by its name
/// in this computer's Photoshop presets (`<version>/Presets/<folder>`, newest version first): an
/// action recorded on another computer, account or Photoshop version names that one's folder.
#[cfg(not(target_arch = "wasm32"))]
fn preset_file(recorded: &str, folder: &str) -> Option<Vec<u8>> {
    let roots: Vec<std::path::PathBuf> = [
        std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Adobe")),
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support/Adobe")),
    ]
    .into_iter()
    .flatten()
    .collect();
    preset_file_in(recorded, folder, &roots)
}

#[cfg(target_arch = "wasm32")]
fn preset_file(_recorded: &str, _folder: &str) -> Option<Vec<u8>> {
    None
}

/// [`preset_file`] with the folders holding the `Adobe Photoshop <version>` folders given.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn preset_file_in(recorded: &str, folder: &str, roots: &[std::path::PathBuf]) -> Option<Vec<u8>> {
    let read = |p: &std::path::Path| -> Option<Vec<u8>> {
        let meta = std::fs::metadata(p).ok()?;
        (meta.is_file() && meta.len() <= MAX_PRESET_FILE).then(|| std::fs::read(p).ok()).flatten()
    };
    if let Some(b) = read(std::path::Path::new(recorded)) {
        return Some(b);
    }
    let name = recorded.rsplit(['/', '\\']).next().filter(|n| !n.is_empty() && *n != "." && *n != "..")?;
    let mut candidates: Vec<std::path::PathBuf> = roots
        .iter()
        .flat_map(|r| std::fs::read_dir(r).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Adobe Photoshop")))
        .map(|p| p.join("Presets").join(folder).join(name))
        .collect();
    // "Adobe Photoshop 2026" sorts after 2024 and "(Beta)": newest first.
    candidates.sort();
    candidates.iter().rev().find_map(|p| read(p))
}

/// Blending Options › Blend If from a layer's `Blnd` list: per channel, this layer's and the
/// underlying layer's black and white points, each split in two (`[blackLo, blackHi, whiteLo,
/// whiteHi]`, 0..255), as `layer.layerStyle.blendingOptions` takes them.
pub(super) fn blend_if(list: &[D]) -> Option<Value> {
    let mut out = Vec::with_capacity(list.len());
    for v in list {
        let D::Descriptor(e) = v else { return None };
        let channel = match get_ref(e, "Chnl").map(target)? {
            Target::Ordinal(o) => lookup(CHANNELS, &o)?,
            _ => return None,
        };
        let n = |k: &str| get_num(e, k).map(|v| v.round().clamp(0.0, 255.0));
        let this = [n("SrcB")?, n("Srcl")?, n("SrcW")?, n("Srcm")?];
        let under = [n("DstB")?, n("Dstl")?, n("DstW")?, n("Dstt")?];
        out.push(json!({"channel": channel, "thisLayer": this, "underlying": under}));
    }
    Some(Value::Array(out))
}

// ------------------------------------------------------------------ make

/// The `make` forms the table doesn't read (or reads with less of the step); `None` leaves the
/// step to the table.
pub(super) fn make_from(d: &Descriptor) -> Option<Mapped> {
    // Add Layer Mask: `Nw` = the channel class, `At` = the mask, `Usng` = how it starts.
    if let Some(D::Class(c)) = d.get("Nw  ")
        && trimmed(&c.class_id) == "Chnl"
    {
        let unsupported = Some(Mapped::Unsupported { harmless: false });
        let Some(at) = get_ref(d, "At  ") else { return unsupported };
        // A mask copied from another layer (`Usng` = that layer's mask) has no command yet.
        if at.len() != 1 || ref_class(at).as_deref() != Some("Chnl") || !matches!(target(at), Target::Ordinal(o) if o == "Msk") {
            return unsupported;
        }
        let id = match get_enum(d, "Usng").as_deref() {
            Some("RvlA") => "layer.layerMask.revealAll",
            Some("RvlS") => "layer.layerMask.revealSelection",
            Some("HdAl") => "layer.layerMask.hideAll",
            Some("HdSl") => "layer.layerMask.hideSelection",
            _ => return unsupported,
        };
        return Some(none(id));
    }
    let r = get_ref(d, "null")?;
    match ref_class(r).as_deref() {
        Some("AdjL") => adjustment_layer(d),
        Some("Path") => {
            // Make Work Path from the selection, at its tolerance (pixels).
            if get_ref(d, "From").is_some_and(|f| matches!(target(f), Target::Property(p) if p == "fsel")) {
                let mut p = json!({});
                if let Some(t) = get_num(d, "Tlrn") {
                    p["tolerance"] = json!(t);
                }
                return Some(step("select.toWorkPath", p));
            }
            // Layer › Vector Mask › Current Path.
            let at = get_ref(d, "At  ")?;
            let using = get_ref(d, "Usng")?;
            let current = at.len() == 1
                && matches!(target(at), Target::Ordinal(o) if o == "vectorMask")
                && using.len() == 1
                && matches!(target(using), Target::Ordinal(o) if o == "Trgt");
            current.then(|| none("layer.vectorMask.currentPath"))
        }
        // History › New Snapshot.
        Some("SnpS") => {
            let from = match get_enum(d, "Usng").as_deref() {
                Some("FllD") | None => "fullDocument",
                Some("MrgL") => "mergedLayers",
                Some("CrnL") => "currentLayer",
                _ => return None,
            };
            let mut p = json!({"from": from});
            if let Some(n) = get_text(d, "Nm  ") {
                p["name"] = json!(n);
            }
            Some(step("history.newSnapshot", p))
        }
        _ => None,
    }
}

/// A new adjustment layer with the New Layer dialog's name, opacity, mode and clipping, of a
/// class given as a class alone or with empty settings (its defaults), Color Lookup included.
fn adjustment_layer(d: &Descriptor) -> Option<Mapped> {
    let using = get_obj(d, "Usng")?;
    let (class, settings) = match using.get("Type") {
        Some(D::Class(c)) => (trimmed(&c.class_id), None),
        Some(D::Descriptor(t)) => (trimmed(&t.class_id), Some(t)),
        _ => return None,
    };
    let mut v: Vec<(String, Value)> = Vec::new();
    if class == "colorLookup" {
        let params = match settings {
            Some(t) if t.get("LUT3DFileData").is_some() => lookup_params(t)?,
            _ => json!({}),
        };
        v.push(("layer.newAdjustmentLayer.colorLookup".into(), params));
    } else {
        // Photoshop 2024 writes Black & White as `BanW`.
        let a = adjustment_by_class(&class).or_else(|| if class == "BanW" { adjustment_by_class("BlnW") } else { None })?;
        let params = match settings {
            Some(t) if class == "Crvs" && t.get("Usng").is_some() => curves_file_params(t)?,
            // Curves or Levels with no adjustments is the identity, their default.
            Some(t) if t.items.is_empty() && matches!(a.class, "Crvs" | "Lvls") => json!({}),
            Some(t) if get_enum(t, "presetKind").as_deref() != Some("presetKindDefault") => (a.from)(t)?,
            _ => json!({}),
        };
        v.push((format!("layer.newAdjustmentLayer.{}", a.suffix), params));
    }
    let mut props = Map::new();
    if let Some(n) = get_text(using, "Nm  ") {
        props.insert("name".into(), n.into());
    }
    if let Some(o) = get_num(using, "Opct") {
        props.insert("opacity".into(), (o / 100.0).into());
    }
    if let Some(m) = get_enum(using, "Md  ") {
        props.insert("blend".into(), blend_label(&m)?.into());
    }
    if !props.is_empty() {
        v.push(("layer.setProps".into(), Value::Object(props)));
    }
    if get_bool(using, "Grup") == Some(true) {
        v.push(("layer.createClippingMask".into(), json!({})));
    }
    Some(Mapped::Steps(v))
}

// ------------------------------------------------------------------ select, delete

/// The `select` forms the table doesn't read.
pub(super) fn select_from(d: &Descriptor) -> Option<Mapped> {
    let r = get_ref(d, "null")?;
    match (ref_class(r).as_deref(), target(r)) {
        // The vector mask's thumbnail, of the target layer or of one by name.
        (Some("Path"), Target::Ordinal(o)) if o == "vectorMask" => {
            let layer = match r.get(1..) {
                None | Some([]) => Map::new(),
                Some(rest) => layer_target(rest)?,
            };
            let mut v = Vec::new();
            if let Some(n) = layer.get("layer") {
                v.push(("layer.select", json!({"layer": n})));
            }
            v.push(("channel.target", json!({"channel": "vectorMask"})));
            Some(steps(v))
        }
        // A channel by name (an alpha channel such as "Alpha 1").
        (Some("Chnl"), Target::Name(n)) => Some(step("channel.target", json!({"channel": n}))),
        // The work path in the Paths panel: the path steps after it name the path themselves.
        (Some("Path"), Target::Property(p)) if p == "WrPt" => Some(Mapped::Unsupported { harmless: true }),
        _ => None,
    }
}

/// The `delete` forms the table doesn't read.
pub(super) fn delete_from(d: &Descriptor) -> Option<Mapped> {
    let r = get_ref(d, "null")?;
    match (ref_class(r).as_deref(), target(r)) {
        (Some("Path"), Target::Property(p)) if p == "WrPt" => Some(step("path.delete", json!({"name": "work"}))),
        (Some("Path"), Target::Name(n)) => Some(step("path.delete", json!({"name": n}))),
        // The layer mask of the target layer or of a layer by name.
        (Some("Chnl"), Target::Ordinal(o)) if o == "Msk" => {
            let layer = match r.get(1..) {
                None | Some([]) => Map::new(),
                Some(rest) => layer_target(rest)?,
            };
            Some(step("layer.layerMask.delete", Value::Object(layer)))
        }
        (Some("Chnl"), Target::Name(n)) => Some(step("channel.delete", json!({"channel": n}))),
        _ => None,
    }
}

/// Photoshop's effect classes ↔ the effects' names in `layer.setEffectsVisible`.
const EFFECTS: &[(&str, &str)] = &[
    ("DrSh", "Drop Shadow"),
    ("IrSh", "Inner Shadow"),
    ("OrGl", "Outer Glow"),
    ("IrGl", "Inner Glow"),
    ("ebbl", "Bevel & Emboss"),
    ("ChFX", "Satin"),
    ("SoFi", "Color Overlay"),
    ("GrFl", "Gradient Overlay"),
    ("patternFill", "Pattern Overlay"),
    ("FrFX", "Stroke"),
];

/// Hide or Show of one effect, `[class of the effect, the layer]`: the eye on the effect's row.
/// `None` for any other reference (layers are the table's).
pub(super) fn effect_visibility(r: &[ReferenceItem], show: bool) -> Option<(&'static str, Value)> {
    let ReferenceItem::Class(c) = r.first()? else { return None };
    let effect = lookup(EFFECTS, &trimmed(&c.class_id))?;
    let mut p = layer_target(r.get(1..).filter(|rest| !rest.is_empty())?)?;
    // `setEffectsVisible` takes a layer id; a layer by name is selected first by the table's
    // steps, so only the target layer maps.
    if p.contains_key("layer") {
        return None;
    }
    p.insert("effect".into(), effect.into());
    p.insert("visible".into(), show.into());
    Some(("layer.setEffectsVisible", Value::Object(p)))
}

// ------------------------------------------------------------------ events

/// Edit › Auto-Align Layers is recorded as `align` with `Usng` = `ADSContent`. Photoshop's Auto
/// projection is the only one read so far; Vignette Removal has no PhotoCraft equivalent.
pub(super) fn auto_align_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if !matches!(get_enum(d, "Aply").as_deref(), Some("Auto") | None) || get_bool(d, "vignette") == Some(true) {
        return unsupported;
    }
    let mut p = json!({"projection": "auto"});
    if get_bool(d, "radialDistort") == Some(true) {
        p["geometricCorrection"] = json!(true);
    }
    step("edit.autoAlignLayers", p)
}

fn auto_align_to(_: &str, p: &Value) -> Option<Descriptor> {
    if jstr(p, "projection").unwrap_or("auto") != "auto" || p.get("reference").is_some() {
        return None;
    }
    Some(
        desc("null")
            .with("null", target_ref("Lyr "))
            .with("Usng", enm("ADSt", "ADSContent"))
            .with("alignToCanvas", D::Boolean(false))
            .with("Aply", enm("projection", "Auto"))
            .with("vignette", D::Boolean(false))
            .with("radialDistort", D::Boolean(jbool(p, "geometricCorrection").unwrap_or(false))),
    )
}

/// A step on the target layer or one by name (`null` = that layer, or absent).
fn layer_step(d: &Descriptor, id: &str) -> Mapped {
    let p = match get_ref(d, "null") {
        None => Map::new(),
        Some(r) if ref_class(r).as_deref() == Some("Lyr") => match layer_target(r) {
            Some(p) => p,
            None => return Mapped::Unsupported { harmless: false },
        },
        Some(_) => return Mapped::Unsupported { harmless: false },
    };
    step(id, Value::Object(p))
}

fn layer_desc(_: &str, p: &Value) -> Option<Descriptor> {
    Some(desc("null").with("null", layer_ref(p)))
}

/// Color Range's preset ranges (`Clrs`) ↔ `select.colorRange`'s `select`.
const COLOR_RANGES: &[(&str, &str)] = &[
    ("Rds", "reds"),
    ("Ylws", "yellows"),
    ("Grns", "greens"),
    ("Cyns", "cyans"),
    ("Bls", "blues"),
    ("Mgnt", "magentas"),
    ("Hghl", "highlights"),
    ("Mdtn", "midtones"),
    ("Shdw", "shadows"),
    ("OtOf", "outOfGamut"),
    ("skinTone", "skinTones"),
];

/// Select › Color Range: a preset range, or one sampled colour (`Mnm` = `Mxm`, CIE Lab) with
/// its fuzziness. A range sampled over several colours selects a Lab box Photoshop's way, which
/// `select.colorRange` can't express yet.
fn color_range_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    if d.items.iter().any(|(k, _)| !matches!(trimmed(k).as_str(), "Fzns" | "Mnm" | "Mxm" | "colorModel" | "Clrs" | "Invr")) {
        return unsupported;
    }
    let mut p = Map::new();
    if let Some(c) = get_enum(d, "Clrs") {
        let Some(range) = lookup(COLOR_RANGES, &c) else { return unsupported };
        p.insert("select".into(), range.into());
    } else {
        let lab = |k: &str| get_obj(d, k).and_then(|c| Some([get_num(c, "Lmnc")?, get_num(c, "A   ")?, get_num(c, "B   ")?]));
        let (Some(lo), Some(hi)) = (lab("Mnm "), lab("Mxm ")) else { return unsupported };
        if lo != hi {
            return unsupported;
        }
        let Some(color) = get_obj(d, "Mnm ").and_then(color_hex) else { return unsupported };
        p.insert("color".into(), color.into());
    }
    if let Some(f) = get_num(d, "Fzns") {
        p.insert("fuzziness".into(), f.into());
    }
    if let Some(i) = get_bool(d, "Invr") {
        p.insert("invert".into(), i.into());
    }
    step("select.colorRange", Value::Object(p))
}

fn color_range_to(_: &str, p: &Value) -> Option<Descriptor> {
    let known = ["select", "color", "fuzziness", "invert"];
    if p.as_object()?.keys().any(|k| !known.contains(&k.as_str())) {
        return None;
    }
    let mut d = desc("null").with("Fzns", long(jnum(p, "fuzziness").unwrap_or(40.0)));
    match jstr(p, "select").unwrap_or("sampledColors") {
        "sampledColors" => {
            let hex = jstr(p, "color")?;
            let c = color_desc(hex)?;
            let rgb = [get_num(&c, "Rd  ")?, get_num(&c, "Grn ")?, get_num(&c, "Bl  ")?].map(|v| (v / 255.0) as f32);
            let [l, a, b] = photocraft_color::convert::srgb_to_lab(rgb);
            let lab = desc("LbCl").with("Lmnc", D::Double(f64::from(l))).with("A   ", D::Double(f64::from(a))).with("B   ", D::Double(f64::from(b)));
            d = d.with("Mnm ", D::Descriptor(lab.clone())).with("Mxm ", D::Descriptor(lab)).with("colorModel", long(0.0));
        }
        range => d = d.with("Clrs", enm("Clrs", lookup_back(COLOR_RANGES, range)?)),
    }
    if let Some(i) = jbool(p, "invert") {
        d = d.with("Invr", D::Boolean(i));
    }
    Some(d)
}

/// The Camera Raw filter's keys ↔ `filter.cameraRaw`'s params (those seen in Photoshop 2024–2026
/// actions; any other key makes the step unsupported rather than silently skipped).
const CAMERA_RAW: &[(&str, &str)] = &[
    ("CrTx", "texture"),
    ("Shrp", "sharpenAmount"),
    ("ShpR", "sharpenRadius"),
    ("ShpD", "sharpenDetail"),
    ("ShpM", "sharpenMasking"),
    ("LNR", "noiseLuminance"),
    ("LNRD", "noiseLuminanceDetail"),
];
/// Keys that only say which Camera Raw (and process version) wrote the step.
const CAMERA_RAW_INFO: &[&str] = &["CrVe", "PrVN", "PrVe"];

fn camera_raw_from(d: &Descriptor) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let mut p = Map::new();
    for (k, v) in &d.items {
        let k = trimmed(k);
        if CAMERA_RAW_INFO.contains(&k.as_str()) {
            continue;
        }
        // Luminance Contrast at its default changes nothing.
        if k == "LNRC" && num(v) == Some(0.0) {
            continue;
        }
        let (Some(param), Some(x)) = (lookup(CAMERA_RAW, &k), num(v)) else { return unsupported };
        p.insert(param.into(), x.into());
    }
    step("filter.cameraRaw", Value::Object(p))
}

fn camera_raw_to(_: &str, p: &Value) -> Option<Descriptor> {
    let mut d = desc("null").with("CrVe", text("16.4")).with("PrVN", long(6.0)).with("PrVe", long(251_920_384.0));
    for (k, v) in p.as_object()? {
        let key = lookup_back(CAMERA_RAW, k)?;
        d = d.with(&format!("{key:<4}"), long(v.as_f64()?));
    }
    Some(d)
}

/// Diffuse's modes (`Md  `) ↔ `filter.stylize.diffuse`'s.
const DIFFUSE_MODES: &[(&str, &str)] = &[("Nrml", "normal"), ("DrkO", "darkenOnly"), ("LghO", "lightenOnly"), ("anisotropic", "anisotropic")];

fn diffuse_from(d: &Descriptor) -> Mapped {
    let mut p = Map::new();
    if let Some(m) = get_enum(d, "Md  ") {
        let m = if m == "normal" { "Nrml".to_string() } else { m };
        let Some(mode) = lookup(DIFFUSE_MODES, &m) else { return Mapped::Unsupported { harmless: false } };
        p.insert("mode".into(), mode.into());
    }
    if let Some(seed) = get_num(d, "FlRs") {
        p.insert("seed".into(), (seed.rem_euclid(4_294_967_296.0) as u32).into());
    }
    step("filter.stylize.diffuse", Value::Object(p))
}

fn diffuse_to(_: &str, p: &Value) -> Option<Descriptor> {
    let mut d = desc("null").with("Md  ", enm("Md  ", lookup_back(DIFFUSE_MODES, jstr(p, "mode").unwrap_or("normal"))?));
    if let Some(seed) = jnum(p, "seed") {
        d = d.with("FlRs", long(seed));
    }
    Some(d)
}

/// Filter Gallery › Sketch › Photocopy, recorded as its own event since Photoshop CC.
fn photocopy_from(d: &Descriptor) -> Mapped {
    let mut p = Map::new();
    if let Some(v) = get_num(d, "Dtl ") {
        p.insert("detail".into(), v.into());
    }
    if let Some(v) = get_num(d, "Drkn") {
        p.insert("darkness".into(), v.into());
    }
    step("filter.gallery.photocopy", Value::Object(p))
}

/// File › Automate › Fit Image (a plug-in, so its event is the plug-in's id).
fn fit_image_from(d: &Descriptor) -> Mapped {
    let px_of = |k: &str| match d.get(k) {
        Some(D::UnitFloat { unit: PX, value }) => Some(*value),
        _ => None,
    };
    if d.items.iter().any(|(k, _)| !matches!(trimmed(k).as_str(), "Wdth" | "Hght")) {
        return Mapped::Unsupported { harmless: false };
    }
    match (px_of("Wdth"), px_of("Hght")) {
        (Some(w), Some(h)) => step("file.automate.fitImage", json!({"width": w.round(), "height": h.round()})),
        _ => Mapped::Unsupported { harmless: false },
    }
}

fn fit_image_to(_: &str, p: &Value) -> Option<Descriptor> {
    if jbool(p, "dontEnlarge") == Some(true) || p.get("resample").is_some() {
        return None;
    }
    Some(desc("null").with("Wdth", px(jnum(p, "width")?)).with("Hght", px(jnum(p, "height")?)))
}

/// The channel a combination reads: `"transparency"`, `"mask"`, the targeted channel, the
/// composite or a channel by name, with the layer it belongs to when the reference names one.
fn combine_channel(r: &[ReferenceItem]) -> Option<Map<String, Value>> {
    let mut p = match r.get(1..) {
        None | Some([]) => Map::new(),
        Some(rest) => layer_target(rest)?,
    };
    let channel = match (ref_class(r).as_deref(), target(r)) {
        (Some("Chnl"), Target::Ordinal(o)) => match o.as_str() {
            "Trsp" => "transparency".to_string(),
            "Msk" => "mask".to_string(),
            "Trgt" => "targeted".to_string(),
            "RGB" | "CMYK" | "Lab" => "composite".to_string(),
            _ => return None,
        },
        (Some("Chnl"), Target::Name(n)) => n,
        _ => return None,
    };
    p.insert("channel".into(), channel.into());
    Some(p)
}

/// Add To / Subtract From / Intersect With: a shape drawn into the selection (marquee, lasso,
/// Magic Wand click), or a channel's pixels (⌘⇧-, ⌘⌥- and ⌘⌥⇧-click on a thumbnail).
fn combine_from(d: &Descriptor, mode: &str) -> Mapped {
    let unsupported = Mapped::Unsupported { harmless: false };
    let Some(r) = get_ref(d, "null") else { return unsupported };
    let fsel = |v: Option<&D>| matches!(v, Some(D::Reference(t)) if matches!(target(t), Target::Property(p) if p == "fsel"));
    // A shape into the selection.
    if matches!(target(r), Target::Property(p) if p == "fsel") {
        let Some(t) = get_obj(d, "T   ") else { return unsupported };
        let cls = trimmed(&t.class_id);
        if cls == "Rctn" || cls == "Elps" {
            let px_of = |k: &str| match t.get(k) {
                Some(D::UnitFloat { unit, value }) if *unit == PX => Some(*value),
                Some(D::Double(v)) => Some(*v),
                _ => None,
            };
            let (Some(top), Some(left), Some(bottom), Some(right)) = (px_of("Top "), px_of("Left"), px_of("Btom"), px_of("Rght")) else { return unsupported };
            let mut p =
                json!({"x": left.round(), "y": top.round(), "width": (right - left).round().max(0.0), "height": (bottom - top).round().max(0.0), "mode": mode});
            if cls == "Elps" {
                p["ellipse"] = json!(true);
            }
            if let Some(a) = get_bool(d, "AntA") {
                p["antiAlias"] = json!(a);
            }
            return step("select.rect", p);
        }
        if cls == "Pnt" {
            return magic_wand(d, t, Some(mode)).unwrap_or(unsupported);
        }
        return unsupported;
    }
    // A channel combined with the selection (the selection is `T`, `With` or `From`).
    if !(fsel(d.get("T   ")) || fsel(d.get("With")) || fsel(d.get("From"))) {
        return unsupported;
    }
    let Some(mut p) = combine_channel(r) else { return unsupported };
    p.insert("operation".into(), mode.into());
    step("select.loadSelection", Value::Object(p))
}

fn no_export(_: &str, _: &Value) -> Option<Descriptor> {
    None
}

/// The events this module adds; [`super::EVENTS`] is searched first.
pub(super) const EVENTS: &[Event] = &[
    Event {
        code: "GrpL",
        name: "groupEvent",
        label: "Create Clipping Mask",
        ids: &["layer.createClippingMask"],
        from: |d| layer_step(d, "layer.createClippingMask"),
        to: layer_desc,
    },
    Event {
        code: "Ungr",
        name: "ungroup",
        label: "Release Clipping Mask",
        ids: &["layer.releaseClippingMask"],
        from: |d| layer_step(d, "layer.releaseClippingMask"),
        to: layer_desc,
    },
    Event {
        code: "",
        name: "ungroupLayersEvent",
        label: "Ungroup Layers",
        ids: &["layer.ungroupLayers"],
        from: |d| layer_step(d, "layer.ungroupLayers"),
        to: layer_desc,
    },
    Event { code: "ClrR", name: "colorRange", label: "Color Range", ids: &["select.colorRange"], from: color_range_from, to: color_range_to },
    Event { code: "", name: "Adobe Camera Raw Filter", label: "Camera Raw Filter", ids: &["filter.cameraRaw"], from: camera_raw_from, to: camera_raw_to },
    Event { code: "Dfs ", name: "diffuse", label: "Diffuse", ids: &["filter.stylize.diffuse"], from: diffuse_from, to: diffuse_to },
    Event { code: "", name: "photocopy", label: "Photocopy", ids: &[], from: photocopy_from, to: no_export },
    Event {
        code: "",
        name: "3caa3434-cb67-11d1-bc43-0060b0a13dc4",
        label: "Fit Image",
        ids: &["file.automate.fitImage"],
        from: fit_image_from,
        to: fit_image_to,
    },
    Event {
        code: "NTSC",
        name: "NTSC",
        label: "NTSC Colors",
        ids: &["filter.video.ntscColors"],
        from: |_| none("filter.video.ntscColors"),
        to: |_, _| Some(desc("null")),
    },
    Event { code: "AddT", name: "addTo", label: "Add To", ids: &[], from: |d| combine_from(d, "add"), to: no_export },
    Event { code: "", name: "add", label: "Add", ids: &[], from: |d| combine_from(d, "add"), to: no_export },
    Event { code: "SbtF", name: "subtractFrom", label: "Subtract From", ids: &[], from: |d| combine_from(d, "subtract"), to: no_export },
    // `IntW`'s string id is `intersectWith`; Photoshop 2026 writes the code's other name.
    Event { code: "IntW", name: "intersectWith", label: "Intersect With", ids: &[], from: |d| combine_from(d, "intersect"), to: no_export },
    Event { code: "", name: "interfaceIconFrameDimmed", label: "Intersect", ids: &[], from: |d| combine_from(d, "intersect"), to: no_export },
    // Auto-Align imports through the `align` event of the main table; this entry exports it.
    Event { code: "Algn", name: "align", label: "Auto-Align Layers", ids: &["edit.autoAlignLayers"], from: auto_align_from, to: auto_align_to },
];
