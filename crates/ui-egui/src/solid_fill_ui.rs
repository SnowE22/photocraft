//! The Solid Color Fill thumbnail opens the shared Color Picker. Preview a shallow document
//! snapshot; Cancel changes nothing and OK commits exactly one engine command.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Rect, Stroke, StrokeKind, pos2, vec2};
use photocraft_color::Color;
use photocraft_doc::{DocId, Document, Fill, LayerContent, LayerId};
use photocraft_engine::solid_fill_cmds as cmds;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, color_picker_ui, theme::Tokens};

pub(crate) struct Preview {
    document: DocId,
    revision: u64,
    dialog: u64,
    layer: LayerId,
    color: Color,
    shown: Arc<Document>,
    key: u64,
}

pub(crate) fn open(app: &mut PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let LayerContent::Fill(Fill::Solid(color)) = st.doc.layer(layer)?.content else { return None };
    let params = json!({"layer": layer.0, "document": st.doc.id.0});
    Some(color_picker_ui::open_for_command(app, "Color Picker (Solid Color)", color.to_rgb(), cmds::SET, params))
}

/// Marks the picker Layer › New Fill Layer › Solid Color… opened on the layer it just made: the
/// document's revision right after making it.
const MADE_AT: &str = "__madeAt";

/// Layer › New Fill Layer › Solid Color…, as in Photoshop (user report): the layer is made at once
/// with the foreground colour and the Color Picker opens on it, changing it live; OK keeps it (one
/// history step, with the picked colour) and Cancel takes it away again. Returns the picker and
/// the new layer.
pub(crate) fn open_new(app: &mut PhotocraftApp) -> Option<(u64, LayerId)> {
    let made = app.run("layer.newFillLayer.solidColor", json!({})).ok()?;
    let layer = LayerId(made.get("layer")?.as_u64()?);
    let revision = app.session.active()?.revision;
    let dialog = open(app)?;
    if let Some(d) = app.ui.dialog_mut(dialog) {
        d.fields.insert(MADE_AT.into(), json!(revision));
    }
    Some((dialog, layer))
}

/// OK on the picker [`open_new`] opened: the layer stays, with the picked colour. When nothing
/// else changed the document meanwhile, the layer is made again with that colour, so History
/// keeps one "New Color Fill Layer" step as Photoshop's does; otherwise the colour is an edit.
fn confirm_made(app: &mut PhotocraftApp, f: &Map<String, Value>, made_at: u64) -> Result<Value, String> {
    let params = f.get("__params").ok_or("missing Solid Color target")?;
    let layer = params.get("layer").and_then(Value::as_u64).ok_or("missing Solid Color layer")?;
    let st = app.session.active().ok_or("no document open")?;
    if params.get("document").and_then(Value::as_u64) != Some(st.doc.id.0) {
        return Err("the target document is no longer active".into());
    }
    if f.get("color") == f.get("__orig") {
        return Ok(json!({"layer": layer}));
    }
    if st.revision == made_at && st.doc.layer(LayerId(layer)).is_some() {
        let color = f.get("color").and_then(Value::as_str).ok_or("missing Solid Color colour")?.to_string();
        app.run("edit.undo", json!({}))?;
        return app.run("layer.newFillLayer.solidColor", json!({"color": color}));
    }
    let mut f = f.clone();
    f.remove(MADE_AT);
    confirm(app, &f)
}

/// Cancel on the picker [`open_new`] opened takes the new layer away again, as in Photoshop:
/// undone when nothing else changed the document since, else deleted.
pub(crate) fn cancelled(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let Some(made_at) = f.get(MADE_AT).and_then(Value::as_u64) else { return };
    let Some(params) = f.get("__params") else { return };
    let (Some(document), Some(layer)) = (params.get("document").and_then(Value::as_u64), params.get("layer").and_then(Value::as_u64)) else { return };
    let Some(st) = app.session.active().filter(|st| st.doc.id.0 == document && st.doc.layer(LayerId(layer)).is_some()) else { return };
    let r = if st.revision == made_at {
        app.run("edit.undo", json!({}))
    } else {
        // Not through `app.run`: that would ask "Delete layer?" first.
        let r = app.session.execute("layer.delete", json!({"layer": layer})).map_err(|e| e.to_string());
        app.sync_views();
        r
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// A creation picker is distinct from the editor opened by a fill layer's thumbnail.
pub(crate) fn owns_new(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some("layer.newFillLayer.solidColor")
        && f.get("__params").and_then(|p| p.get("document")).and_then(Value::as_u64).is_some()
}

pub(crate) fn confirm_new(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let expected = f.get("__params").and_then(|p| p.get("document")).and_then(Value::as_u64).ok_or("missing Solid Color document")?;
    let st = app.session.active().ok_or("no document open")?;
    if st.doc.id.0 != expected {
        return Err("the target document is no longer active".into());
    }
    let color = f.get("color").and_then(Value::as_str).ok_or("missing Solid Color colour")?;
    app.run("layer.newFillLayer.solidColor", json!({"color": color}))
}

pub(crate) fn owns(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(cmds::SET)
}

pub(crate) fn thumbnail_color(app: &PhotocraftApp, document: DocId, layer: LayerId, original: Color) -> Color {
    app.ui
        .dialogs
        .iter()
        .rev()
        .filter(|d| owns(&d.fields))
        .find_map(|d| {
            let params = d.fields.get("__params")?;
            if params.get("document")?.as_u64()? != document.0 || params.get("layer")?.as_u64()? != layer.0 || d.fields.get("color") == d.fields.get("__orig") {
                return None;
            }
            cmds::color_param(d.fields.get("color")?, original, app.session.documents().iter().find(|st| st.doc.id == document)?.doc.mode).ok()
        })
        .unwrap_or(original)
}

/// No-op OK preserves imported high-precision / non-RGB colour values rather than quantising
/// them to the picker's display hex. A stale document must never receive the edit.
pub(crate) fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if let Some(made_at) = f.get(MADE_AT).and_then(Value::as_u64) {
        return confirm_made(app, f, made_at);
    }
    let mut params = f.get("__params").cloned().ok_or("missing Solid Color target")?;
    let st = app.session.active().ok_or("no document open")?;
    if params.get("document").and_then(Value::as_u64) != Some(st.doc.id.0) {
        return Err("the target document is no longer active".into());
    }
    let layer = LayerId(params.get("layer").and_then(Value::as_u64).ok_or("missing Solid Color layer")?);
    if !st.doc.layer(layer).is_some_and(|l| matches!(l.content, LayerContent::Fill(Fill::Solid(_)))) {
        return Err("the target Solid Color Fill layer no longer exists".into());
    }
    if f.get("color") == f.get("__orig") {
        return Ok(json!({"layer": layer.0, "unchanged": true}));
    }
    let color = f.get("color").cloned().ok_or("missing color")?;
    let object = params.as_object_mut().ok_or("invalid Solid Color target")?;
    object.insert("color".into(), color);
    app.run(cmds::SET, params)
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let dialog = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields));
    let Some(d) = dialog else {
        app.solid_fill_preview = None;
        return None;
    };
    let st = app.session.documents().get(idx)?;
    let params = d.fields.get("__params")?;
    if params.get("document")?.as_u64()? != st.doc.id.0 || d.fields.get("color") == d.fields.get("__orig") {
        return None;
    }
    let layer = LayerId(params.get("layer")?.as_u64()?);
    let LayerContent::Fill(Fill::Solid(original)) = st.doc.layer(layer)?.content else { return None };
    let color = cmds::color_param(d.fields.get("color")?, original, st.doc.mode).ok()?;
    if let Some(p) = &app.solid_fill_preview
        && p.document == st.doc.id
        && p.revision == st.revision
        && p.dialog == d.id
        && p.layer == layer
        && p.color == color
    {
        return Some((p.shown.clone(), p.key));
    }
    let mut doc = (*st.doc).clone();
    doc.layer_mut(layer)?.content = LayerContent::Fill(Fill::Solid(color));
    // Include the dialog identity: cancelling and reopening at the same document revision
    // must not reuse an old preview's pixels in the canvas caches.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (st.doc.id.0, st.revision, d.id, layer.0).hash(&mut hash);
    color.mode.hash(&mut hash);
    for component in color.c.into_iter().chain([color.alpha]) {
        component.to_bits().hash(&mut hash);
    }
    let key = (1 << 36) | (hash.finish() & ((1 << 36) - 1));
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    app.solid_fill_preview = Some(Preview { document: st.doc.id, revision: st.revision, dialog: d.id, layer, color, shown: shown.clone(), key });
    Some((shown, key))
}

/// Clean-room drawing of Photoshop's Solid Color thumbnail: an inset colour sample with the
/// small fill marker underneath, rather than a raster thumbnail covering the whole tile.
pub(crate) fn paint_thumbnail(ui: &egui::Ui, color: Color, rect: Rect) {
    thumbnail_frame(ui, rect, |swatch| {
        let [r, g, b, a] = color.to_rgba8();
        crate::widgets::checker(ui.painter(), swatch, 3.0);
        ui.painter().rect_filled(swatch, 0.0, Color32::from_rgba_unmultiplied(r, g, b, a));
    });
}

/// Shared fill-layer thumbnail frame and marker, drawn around the inset contents.
pub(crate) fn thumbnail_frame(ui: &egui::Ui, rect: Rect, contents: impl FnOnce(Rect)) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    if t.pro {
        // Photoshop's fill thumbnail uses the whole tile: the fill on top and, under a dark line, a
        // light slider bar with a notch in the middle.
        let bar_h = (rect.height() * 0.25).round().max(3.0);
        let swatch = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - bar_h));
        contents(swatch);
        p.rect_filled(Rect::from_min_max(swatch.left_bottom(), pos2(rect.right(), swatch.bottom() + 1.0)), 0.0, Color32::from_gray(20));
        let bar = Rect::from_min_max(pos2(rect.left(), swatch.bottom() + 1.0), rect.max);
        p.rect_filled(bar, 0.0, Color32::from_gray(178));
        let (x, k) = (bar.center().x.round(), (bar.height() * 0.5).round().max(2.0));
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x, bar.bottom() - k - 1.0), pos2(x + k, bar.bottom() - 1.0), pos2(x - k, bar.bottom() - 1.0)],
            Color32::from_gray(40),
            Stroke::NONE,
        ));
        return;
    }
    p.rect_filled(rect, t.radius_sm, t.field);
    let size = rect.size() * 0.7;
    let sample = Rect::from_center_size(rect.center() - vec2(0.0, size.y * 0.08), size);
    let bar_height = (size.y * 0.17).max(2.0);
    let swatch = Rect::from_min_max(sample.min, pos2(sample.right(), sample.bottom() - bar_height));
    contents(swatch);
    p.rect_stroke(sample, 0.0, Stroke::new(1.0, t.text), StrokeKind::Outside);
    let y = swatch.bottom();
    p.line_segment([pos2(sample.left(), y), pos2(sample.right(), y)], Stroke::new(1.0, t.text));
    let x = sample.center().x;
    p.add(egui::Shape::convex_polygon(
        vec![pos2(x, y + 1.0), pos2(x - bar_height * 0.6, sample.bottom()), pos2(x + bar_height * 0.6, sample.bottom())],
        t.text,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests;
