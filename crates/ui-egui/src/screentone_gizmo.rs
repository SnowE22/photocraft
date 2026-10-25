//! On-canvas Screentone gizmo: while the Screentones panel shows, the pending sheet's
//! geometry lives on the canvas as three handles — the *origin* (a crosshair where the lattice
//! anchors), the *x arm* (its direction is the pattern angle, its length the x spacing) and
//! the *y arm* (its length the y spacing).
//!
//! The protocol:
//! * **Origin**: dragging translates the sheet's phase (`ScreentoneSpec::origin_x/y`, given in
//!   the tone's own rotated frame); `Shift` locks the drag to the dominant document axis.
//! * **X arm**: dragging sets the pattern's `angle` and the arm's length sets `spacingX`;
//!   `Shift` snaps the angle to 15° steps.
//! * **Y arm**: dragging sets `spacingY`.
//!
//! While a drag runs the spec changes live — directly on `ScreentoneUi.spec`, the panel's own
//! state, so the panel and the gizmo edit one truth — and a CPU-rendered preview shows the
//! sheet (the engine's own render path, coverage-gated by the mask a restyle keeps or the
//! selection a fresh sheet inherits). The pointer-up commits through `layer.screentone` bound
//! to the layer the panel's New Layer produced — same layer, same name and mask, one
//! journalled step to undo. `Escape` instead restores the spec the drag started from. A press
//! that moves nothing commits nothing.
//!
//! Cursors: egui 0.36 has no dedicated rotate or size-all cursor, so the origin shows the
//! precision crosshair, the x arm the diagonal resize (an angle to set) and the y arm the
//! four-way scroll arrow of the classic "size all".
//!
//! The engine anchors the lattice at `screentone_cmds::sheet_bounds`' center and phases it by
//! the origin through the tone's own canvas rotation (`ScreentoneSpec::rot_canvas`); the
//! gizmo reads those bounds through the engine (the same rule creation and restyle share, so
//! a drag's preview and its committed render use one anchor) and inverts the same rotation.
//! With the default `(0, 0)` origin the handle sits exactly on the bounds' center — the
//! selection's center while a selection rules the bounds.

use egui::{pos2, vec2};
use photocraft_algo::{FilterParams, ScreentoneSpec};
use photocraft_compose::Buffer;
use photocraft_doc::{DocId, LayerId, Rect};
use photocraft_raster::Surface;
use serde_json::{json, Value};

use crate::{canvas::{ToolEvent, ViewXform}, PhotocraftApp};

/// Screen pixels a handle answers the pointer within (a 10 px grab target, midway between the
/// 8-12 px hand targets Photoshop-class gizmos use).
const HIT_PX: f32 = 10.0;

/// Pixel count beyond which the preview skips rendering (the handles and the sync stay live;
/// a region past ~2.5 MP re-renders too slowly per drag frame, and the engine's committed
/// render takes over).
const PREVIEW_CAP: usize = 2_500_000;

#[derive(Default)]
pub(crate) struct Gizmo {
    /// The sheet layer the panel's New Layer produced — the drags' restyle target. Bound to
    /// one document (`target_doc`, so another document fronts it stale).
    target: Option<LayerId>,
    target_doc: Option<DocId>,
    /// Last active screentone layer whose `spec` was synced into `ScreentoneUi`.
    synced_layer: Option<(DocId, LayerId)>,
    /// The running drag, with the spec as it stood at the press (Escape restores it).
    drag: Option<Drag>,
    /// The preview texture cache (the `ClonePreviewCache` pattern: a keyed CPU surface).
    preview: Option<PreviewCache>,
}

/// Which end of the gizmo a pointer took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Handle {
    /// The origin: the sheet's phase anchor inside the tone's rotation (`Shift` axis-locks).
    Origin,
    /// The x arm: direction sets the angle, distance sets the x spacing (`Shift` 15° snap).
    ArmX,
    /// The y arm: distance sets the y spacing.
    ArmY,
}

/// A grab in flight: the handle kind, where the press happened and the spec as it stood (the
/// fields a drag must not disturb stay in it).
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub(crate) kind: Handle,
    pub(crate) start_doc: [f64; 2],
    pub(crate) spec: ScreentoneSpec,
}

struct PreviewCache {
    key: String,
    rect: Rect,
    texture: egui::TextureHandle,
}

/// The gizmo and its canvas preview live only while a drag runs or the Screentones panel shows
/// with an active procedural screentone layer (`LayerContent::Screentone`). Switching to the
/// panel tab over a raster layer, smart object or empty layer stack never projects an overlay.
pub(crate) fn showing(app: &PhotocraftApp) -> bool {
    if app.screentone_gizmo.drag.is_some() {
        return true;
    }
    crate::screentone_panel::showing(app)
        && app
            .session
            .active()
            .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
            .is_some_and(photocraft_doc::Layer::is_screentone)
}

/// The reference bounds the sheet renders over, exactly as the engine computes them
/// (`screentone_cmds::sheet_bounds`): their center is the lattice's anchor, so the gizmo's
/// geometry reads them through the engine instead of deriving them twice.
fn sheet_bounds(app: &PhotocraftApp) -> Option<Rect> {
    let b = photocraft_engine::screentone_cmds::sheet_bounds(&app.session, target_layer(app).map(|l| l.0));
    (!b.is_empty()).then_some(b)
}

/// The lattice anchor in document coordinates: the engine anchors the sheet at the render
/// bounds' center and phases the lattice by `origin` through the tone's own canvas rotation,
/// so the document anchor undoes that same rotation (the engine's lattice derives local
/// coordinates with `local_x = rel_x·cos + rel_y·sin` …, and this is its inverse plus the
/// center offset).
fn anchor_doc(spec: &ScreentoneSpec, bounds: Rect) -> [f64; 2] {
    let (sin, cos) = spec.rot_canvas.to_radians().sin_cos();
    let (sin, cos) = (f64::from(sin), f64::from(cos));
    // Exactly the engine's center — the algo takes half of the max-1 sizes in f32
    // (`cx = b.x0 as f32 + bw * 0.5`), so an odd width's half pixel keeps the handle on the
    // true lattice center.
    let center = [
        f64::from(bounds.x0) + f64::from(bounds.width().max(1)) * 0.5,
        f64::from(bounds.y0) + f64::from(bounds.height().max(1)) * 0.5,
    ];
    let (ox, oy) = (f64::from(spec.origin_x), f64::from(spec.origin_y));
    [center[0] + ox * cos - oy * sin, center[1] + ox * sin + oy * cos]
}

/// A document-space offset through the tone's own canvas rotation: doc → local, the same
/// mapping the engine's lattice uses, so a drag's pointer delta lands in the spec's frame.
fn doc_to_local(spec: &ScreentoneSpec, rel: [f64; 2]) -> [f32; 2] {
    let (sin, cos) = spec.rot_canvas.to_radians().sin_cos();
    let (rx, ry) = (rel[0] as f32, rel[1] as f32);
    [rx * cos + ry * sin, -rx * sin + ry * cos]
}

/// The document-space direction an arm points: the tone's own axis through its canvas
/// rotation plus, for the x arm, the pattern's own angle — the same mapping a drag inverts.
fn arm_dir(spec: &ScreentoneSpec, kind: Handle) -> [f64; 2] {
    let deg = match kind {
        Handle::ArmX => spec.angle + spec.rot_canvas,
        Handle::ArmY => spec.angle + spec.rot_canvas + 90.0,
        Handle::Origin => spec.angle,
    };
    let (sin, cos) = f32::to_radians(deg).sin_cos();
    [f64::from(cos), f64::from(sin)]
}

/// The three handles in screen space: the origin at the lattice anchor, the arm ends at one
/// spacing along each axis (the spacings are document px, so the arms scale with the view).
fn handles(spec: &ScreentoneSpec, bounds: Rect, xf: &ViewXform) -> [egui::Pos2; 3] {
    let a = anchor_doc(spec, bounds);
    let at = |dx: [f64; 2], spacing: f64| {
        xf.to_screen((a[0] + dx[0] * spacing) as f32, (a[1] + dx[1] * spacing) as f32)
    };
    let x = arm_dir(spec, Handle::ArmX);
    let y = arm_dir(spec, Handle::ArmY);
    [xf.to_screen(a[0] as f32, a[1] as f32), at(x, spec.spacing_x as f64), at(y, spec.spacing_y as f64)]
}

/// The handle under a document-space pointer point (the distances are screen px through the
/// view, so zoomed, rotated and flipped views all grab correctly) — the nearest within the
/// grab target, or `None` between handles.
fn hit(app: &PhotocraftApp, xf: &ViewXform, at: [f64; 2]) -> Option<Handle> {
    let b = sheet_bounds(app)?;
    let spec = app.ui.screentone_ui.spec.clone();
    let hs = handles(&spec, b, xf);
    let p = xf.to_screen(at[0] as f32, at[1] as f32);
    let mut best: Option<(f32, Handle)> = None;
    for (kind, q) in [(Handle::Origin, hs[0]), (Handle::ArmX, hs[1]), (Handle::ArmY, hs[2])] {
        let d = q.distance(p);
        if d <= HIT_PX && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, kind));
        }
    }
    best.map(|(_, kind)| kind)
}

/// The canvas event handler for the gizmo's drags, before the tools see the events (hooked in
/// `canvas::tool_event` right after `symmetry_ui::pointer`). Only a press that took a handle
/// (and its moves and release) returns true; everything else falls through to the tools.
pub(crate) fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if !showing(app) {
        return false;
    }
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let Some(xf) = ViewXform::active(app) else { return false };
            let Some(kind) = hit(app, &xf, [x, y]) else { return false };
            app.screentone_gizmo.drag = Some(Drag { kind, start_doc: [x, y], spec: app.ui.screentone_ui.spec.clone() });
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let Some(drag) = app.screentone_gizmo.drag.clone() else { return false };
            apply_drag(app, &drag, [x, y], mods.shift);
            true
        }
        ToolEvent::Up { x, y } => {
            let Some(drag) = app.screentone_gizmo.drag.clone() else { return false };
            app.screentone_gizmo.drag = None;
            app.screentone_gizmo.preview = None;
            apply_drag(app, &drag, [x, y], mods.shift);
            commit_drag(app, &drag);
            true
        }
    }
}

/// Maps a pointer move onto the pending spec. The drag re-derives each move from its own
/// snapshot, so moves compose and the drag's fields never collide with anything the panel
/// changes meanwhile.
fn apply_drag(app: &mut PhotocraftApp, drag: &Drag, at: [f64; 2], shift: bool) {
    let Some(b) = sheet_bounds(app) else { return };
    let mut spec = drag.spec.clone();
    match drag.kind {
        Handle::Origin => {
            // `Shift` locks to the dominant *document* axis before the tone's own rotation, so
            // the slide stays on one document direction however the tone is turned.
            let mut d = [at[0] - drag.start_doc[0], at[1] - drag.start_doc[1]];
            if shift && d[0].abs() != d[1].abs() {
                if d[0].abs() < d[1].abs() {
                    d[0] = 0.0;
                } else {
                    d[1] = 0.0;
                }
            }
            let loc = doc_to_local(&drag.spec, d);
            spec.origin_x = (spec.origin_x + loc[0]).clamp(-100_000.0, 100_000.0);
            spec.origin_y = (spec.origin_y + loc[1]).clamp(-100_000.0, 100_000.0);
        }
        Handle::ArmX | Handle::ArmY => {
            let a = anchor_doc(&drag.spec, b);
            let loc = doc_to_local(&drag.spec, [at[0] - a[0], at[1] - a[1]]);
            let dist = (loc[0] * loc[0] + loc[1] * loc[1]).sqrt();
            match drag.kind {
                Handle::ArmX => {
                    let mut deg = loc[1].atan2(loc[0]).to_degrees();
                    if shift {
                        deg = (deg / 15.0).round() * 15.0;
                    }
                    // Normalize into the panel field's −180..180 range.
                    spec.angle = (deg + 180.0).rem_euclid(360.0) - 180.0;
                    spec.spacing_x = dist.clamp(1.0, 2000.0);
                }
                _ => spec.spacing_y = dist.clamp(1.0, 2000.0),
            }
        }
    }
    app.ui.screentone_ui.spec = spec;
}

/// The pointer-up commit: the pending spec runs `layer.screentone` against the bound sheet —
/// one journalled edit, restyled in place. Without a valid target (the panel has not
/// generated a sheet yet, the layer is gone, another document fronts) or a moved spec there
/// is nothing to write.
fn commit_drag(app: &mut PhotocraftApp, drag: &Drag) {
    if app.ui.screentone_ui.spec == drag.spec {
        return; // nothing moved: no render, no journal entry
    }
    let Some(target) = target_layer(app) else { return };
    let mut p = crate::screentone_panel::params(&app.ui.screentone_ui);
    if let Some(map) = p.as_object_mut() {
        map.insert("layer".into(), json!(target.0));
    }
    let result = crate::preset_panels::run(app, photocraft_engine::screentone_cmds::LAYER, p);
    bind_layer_result(app, photocraft_engine::screentone_cmds::LAYER, &result);
}

/// Synchronizes `ScreentoneUi.spec` and the gizmo's target binding when the user selects an
/// existing `LayerContent::Screentone` layer in the Layers panel.
pub(crate) fn sync_active_layer(app: &mut PhotocraftApp) {
    if app.screentone_gizmo.drag.is_some() {
        return;
    }
    let active_tone = app.session.active().and_then(|st| {
        let id = st.active_layer?;
        let photocraft_doc::LayerContent::Screentone(tone) = &st.doc.layer(id)?.content else {
            return None;
        };
        Some((st.doc.id, id, tone.spec.clone()))
    });
    match active_tone {
        Some((doc_id, id, spec)) => {
            if app.screentone_gizmo.synced_layer != Some((doc_id, id)) {
                app.ui.screentone_ui.spec = spec;
                app.screentone_gizmo.target = Some(id);
                app.screentone_gizmo.target_doc = Some(doc_id);
                app.screentone_gizmo.synced_layer = Some((doc_id, id));
            }
        }
        None => {
            app.screentone_gizmo.synced_layer = None;
        }
    }
}

/// The gizmo's restyle target while it still names an existing live screentone layer in the
/// active document (prefers the active layer when it is a live screentone, or the panel's
/// bound layer in the same document).
fn target_layer(app: &PhotocraftApp) -> Option<LayerId> {
    let st = app.session.active()?;
    if let Some(id) = st.active_layer
        && st.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone)
    {
        return Some(id);
    }
    let gizmo = &app.screentone_gizmo;
    if let Some(id) = gizmo.target
        && gizmo.target_doc == Some(st.doc.id)
        && st.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone)
    {
        return Some(id);
    }
    None
}

/// The panel hands the gizmo the layer a New Layer run produced: that sheet becomes the
/// drags' restyle target (a binding, not an active-layer look-up — the panel knows exactly
/// which sheet it just made; tests call this like the panel does).
pub(crate) fn bind_layer_result(app: &mut PhotocraftApp, command: &str, result: &Option<Value>) {
    if command != photocraft_engine::screentone_cmds::LAYER {
        return;
    }
    let Some(id) = result.as_ref().and_then(|v| v.get("layer")).and_then(Value::as_u64) else { return };
    let Some(st) = app.session.active() else { return };
    let id = LayerId(id);
    if !st.doc.layer(id).is_some_and(photocraft_doc::Layer::is_screentone) {
        return;
    }
    let doc_id = st.doc.id;
    let gizmo = &mut app.screentone_gizmo;
    gizmo.target = Some(id);
    gizmo.target_doc = Some(doc_id);
    gizmo.synced_layer = Some((doc_id, id));
}

/// Detaches the gizmo from any previously bound screentone layer (e.g. after freezing/rasterizing
/// the active screentone layer so the next edit does not target a baked layer).
pub(crate) fn detach(app: &mut PhotocraftApp) {
    let gizmo = &mut app.screentone_gizmo;
    gizmo.target = None;
    gizmo.target_doc = None;
    gizmo.synced_layer = None;
    gizmo.drag = None;
    gizmo.preview = None;
}

/// Escape (hooked in `canvas_view`, where the egui context is at hand) cancels the running
/// drag and restores the spec it started from — no render, no journal entry.
pub(crate) fn escape(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return;
    }
    cancel_drag(app);
}

/// Cancels the running drag and restores its spec (the `Escape` behaviour, testable without
/// input events). A no-op without a running drag.
pub(crate) fn cancel_drag(app: &mut PhotocraftApp) {
    let Some(drag) = app.screentone_gizmo.drag.take() else { return };
    app.screentone_gizmo.preview = None;
    app.ui.screentone_ui.spec = drag.spec;
}

/// While a gizmo handle drag is in flight (`drag.is_some()`), hides the target screentone layer
/// in the canvas composite so the live `preview` mesh replaces it cleanly without double
/// Multiply blending.
pub(crate) fn display_doc(app: &PhotocraftApp, idx: usize) -> Option<(std::sync::Arc<photocraft_doc::Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    app.screentone_gizmo.drag.as_ref()?;
    let target = target_layer(app)?;
    let st = app.session.documents().get(idx)?;
    let shown = crate::distort_ui::without_layer(&st.doc, target);
    let key = (1 << 49) ^ st.revision.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ target.0;
    Some((shown, key))
}

/// Refreshes the live preview each canvas frame (the `&mut` site in `canvas_view`, ahead of
/// `draw_drag_preview`), so the mesh the later draw pass projects holds this frame's texture.
/// The preview texture is built **only** during an active handle drag (`drag.is_some()`); when
/// idle, the document's own `LayerContent::Screentone` layer renders through `photocraft-compose`
/// without any duplicate overlay mesh.
pub(crate) fn refresh_preview(app: &mut PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, output: Option<u32>) {
    sync_active_layer(app);
    if !showing(app) || app.screentone_gizmo.drag.is_none() {
        app.screentone_gizmo.preview = None;
        return;
    }
    let Some(b) = sheet_bounds(app) else {
        app.screentone_gizmo.preview = None;
        return;
    };
    let Some(region) = preview_rect(xf, b) else {
        app.screentone_gizmo.preview = None;
        return;
    };
    let Some(st) = app.session.active() else { return };
    // The display the preview transforms its colors for (None: the plain sRGB pipeline).
    let (display, display_key) = crate::canvas::canvas_display(app, &st.doc, output);
    let target = target_layer(app);
    let coverage = target
        .and_then(|id| st.doc.layer(id))
        .and_then(|l| l.mask.as_ref())
        .filter(|m| m.enabled)
        .map(|m| m.surface.clone())
        .or_else(|| st.doc.selection.clone());
    let spec = app.ui.screentone_ui.spec.clone();
    // The engine restyles in the layer's own surface format; the sheet the panel made (and a
    // fresh creation) would render in the document's, so the preview prefers the layer's.
    let fmt = target
        .and_then(|id| st.doc.layer(id))
        .and_then(|l| l.surface())
        .map(|s| s.format())
        .unwrap_or_else(|| st.doc.pixel_format());
    let key = format!(
        "{}|{}|{:?}|{}|{}|{}|{}|{}",
        st.doc.id.0,
        st.revision,
        target,
        format!("{spec:?}"),
        format!("{b:?}"),
        format!("{region:?}"),
        app.session.active_index().map_or(u64::MAX, |i| i as u64),
        display_key,
    );
    if app.screentone_gizmo.preview.as_ref().is_some_and(|c| c.key == key) {
        return;
    }
    let params = FilterParams::Screentone { spec: spec_with(spec) };
    let buf = render_preview(fmt, region, b, &params, coverage.as_ref().map(|c| (c, region)));
    let image = crate::canvas::display_image(display.as_deref(), &buf);
    match app.screentone_gizmo.preview.as_mut() {
        Some(cache) => {
            cache.texture.set(image, crate::canvas::canvas_texture_options(1));
            cache.key = key;
            cache.rect = region;
        }
        None => app.screentone_gizmo.preview = Some(PreviewCache {
            key,
            rect: region,
            texture: ctx.load_texture("screentone-preview", image, crate::canvas::canvas_texture_options(1)),
        }),
    }
}

/// `useSourceLuminance` off: the preview shows a procedural sheet — the same thing the engine
/// renders on generation and restyle (the gizmo's sheet is always procedural, whatever the
/// panel's modulate checkbox holds for filter runs).
fn spec_with(mut spec: ScreentoneSpec) -> ScreentoneSpec {
    spec.use_source_luminance = false;
    spec.mode = photocraft_algo::ScreentoneMode::Sheet;
    spec
}

/// One tone render for `region` over the engine's `bounds`, by the engine's own path
/// (`FilterParams::Screentone` procedural), coverage-multiplied by a gating surface when one
/// arrives (a mask's, or a selection's): every channel scales with the coverage, and pixels
/// outside `bounds` or where coverage is zero have strictly 0.0 alpha.
fn render_preview(fmt: photocraft_color::PixelFormat, region: Rect, bounds: Rect, params: &FilterParams, coverage: Option<(&Surface, Rect)>) -> Buffer {
    let mut buf = Buffer::transparent(region);
    let clip = region.intersect(&bounds);
    if clip.is_empty() {
        return buf;
    }
    let rendered = photocraft_algo::apply(&Surface::new(fmt), params, region, bounds, None);
    let channels = rendered.format().channels();
    let flat = rendered.read_region(region);
    let cover: Option<(usize, Vec<f32>)> = coverage.map(|(c, r)| (c.format().channels(), c.read_region(r)));
    let w = region.width() as usize;
    buf.px = flat
        .chunks_exact(channels)
        .enumerate()
        .map(|(i, v)| {
            let x = region.x0 + (i % w) as i32;
            let y = region.y0 + (i / w) as i32;
            if !bounds.contains(x, y) {
                return [0.0; 4];
            }
            let sample = |k: usize| v.get(k).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            let mut px = [
                sample(0),
                if channels >= 3 { sample(1) } else { sample(0) },
                if channels >= 3 { sample(2) } else { sample(0) },
                if channels >= 4 { sample(3) } else { 1.0 },
            ];
            if let Some((mch, mflat)) = cover.as_ref() {
                let cov = mflat.get(i * mch).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                if cov <= 1e-6 {
                    return [0.0; 4];
                }
                px[0] *= cov;
                px[1] *= cov;
                px[2] *= cov;
                px[3] *= cov;
            }
            px
        })
        .collect();
    buf
}

/// The sheet region the preview renders: the visible document area (the view's corners in
/// document space, boxed — a rotated view boxes wider than it shows) cut down by the sheet's
/// bounds. `None` when nothing is visible, or the region would exceed the render cap.
fn preview_rect(xf: &ViewXform, sheet: Rect) -> Option<Rect> {
    let corners = [xf.rect.left_top(), xf.rect.right_top(), xf.rect.right_bottom(), xf.rect.left_bottom()];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, -f64::MAX, -f64::MAX);
    for c in corners {
        let p = xf.to_doc(c);
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    // Whole pixels inside the sheet only.
    let r = Rect::new(
        x0.max(f64::from(sheet.x0)).floor() as i32,
        y0.max(f64::from(sheet.y0)).floor() as i32,
        x1.min(f64::from(sheet.x1)).ceil() as i32,
        y1.min(f64::from(sheet.y1)).ceil() as i32,
    );
    if r.x1 < r.x0 || r.y1 < r.y0 {
        return None;
    }
    (r.width() as usize * r.height() as usize <= PREVIEW_CAP).then_some(r)
}

/// The gizmo overlay: when idle (`drag.is_none()`), draws **only** the handle wireframe (the
/// origin crosshair and the two arms with end rings), leaving pixel rendering to the layer's own
/// `photocraft-compose` composite. Only during an active handle drag (`drag.is_some()`) is the
/// temporary preview mesh drawn under the handles.
pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if !showing(app) {
        return;
    }
    if app.screentone_gizmo.drag.is_some()
        && let Some(cache) = &app.screentone_gizmo.preview
    {
        let mut mesh = egui::Mesh::with_texture(cache.texture.id());
        let r = cache.rect;
        let vertex = |x: f64, y: f64| egui::epaint::Vertex {
            pos: xf.to_screen(x as f32, y as f32),
            // UVs on the region's doc rect: the mesh projects through rotated, flipped and
            // zoomed views with the tone staying correct.
            uv: pos2(((x - f64::from(r.x0)) / f64::from(r.width())) as f32, ((y - f64::from(r.y0)) / f64::from(r.height())) as f32),
            color: egui::Color32::WHITE,
        };
        for (x, y) in [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)] {
            mesh.vertices.push(vertex(f64::from(x), f64::from(y)));
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(mesh));
    }
    let Some(b) = sheet_bounds(app) else { return };
    let spec = app.ui.screentone_ui.spec.clone();
    let hs = handles(&spec, b, xf);
    let tokens = crate::theme::Tokens::get(painter.ctx());
    let stroke = egui::Stroke::new(1.5, tokens.accent);
    // The origin: a crosshair with a ring around the grab point; the arms with small end
    // rings on their ends.
    painter.line_segment([hs[0] - vec2(10.0, 0.0), hs[0] + vec2(10.0, 0.0)], stroke);
    painter.line_segment([hs[0] - vec2(0.0, 10.0), hs[0] + vec2(0.0, 10.0)], stroke);
    painter.circle_stroke(hs[0], 5.0, stroke);
    painter.line_segment([hs[0], hs[1]], stroke);
    painter.line_segment([hs[0], hs[2]], stroke);
    for q in [hs[1], hs[2]] {
        painter.circle_stroke(q, 4.0, stroke);
    }
}

/// The cursor over the canvas while the gizmo lives (hooked in `canvas_view`'s cursor chain,
/// ahead of the selection cursor): the hovered handle's stand-in, or the running drag's.
pub(crate) fn hover_cursor(app: &PhotocraftApp, xf: &ViewXform, at: [f64; 2]) -> Option<egui::CursorIcon> {
    if !showing(app) {
        return None;
    }
    let kind = app.screentone_gizmo.drag.as_ref().map(|d| d.kind).or_else(|| hit(app, xf, at))?;
    Some(cursor_of(kind))
}

fn cursor_of(kind: Handle) -> egui::CursorIcon {
    // egui 0.36 has no dedicated rotate or size-all cursor: the origin takes the precision
    // crosshair, the x arm the diagonal resize (an angle to set) and the y arm the four-way
    // arrow of the classic "size all".
    match kind {
        Handle::Origin => egui::CursorIcon::Crosshair,
        Handle::ArmX => egui::CursorIcon::ResizeNeSw,
        Handle::ArmY => egui::CursorIcon::AllScroll,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LAYER: &str = photocraft_engine::screentone_cmds::LAYER;

    fn app(depth: u64) -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 96, "height": 72, "depth": depth})).unwrap();
        app.sync_views();
        // The gizmo lives while the Screentones tab shows.
        app.ui.panels.color = true;
        app.ui.dock_tabs.color = crate::screentone_panel::COLOR_TAB;
        (app, ctx)
    }

    /// Generates a sheet through the panel's command path and binds the gizmo to it, the way
    /// the panel's New Layer button does — pending panel values first, since the gizmo edits
    /// the panel's own spec: the json stands in for the fields the panel holds (the engine's
    /// defaults: angle 45°, spacings 8, origin 0).
    fn generate(app: &mut PhotocraftApp, p: Value) -> u64 {
        let s = &mut app.ui.screentone_ui.spec;
        s.angle = p.get("angle").and_then(Value::as_f64).unwrap_or(45.0) as f32;
        s.spacing_x = p.get("spacingX").and_then(Value::as_f64).unwrap_or(8.0) as f32;
        s.spacing_y = p.get("spacingY").and_then(Value::as_f64).unwrap_or(8.0) as f32;
        s.origin_x = p.get("originX").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        s.origin_y = p.get("originY").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let r = app.run(LAYER, p).unwrap();
        bind_layer_result(app, LAYER, &Some(r.clone()));
        r["layer"].as_u64().unwrap()
    }

    /// Drives a full press-move-release at document coordinates through the canvas's own
    /// event pipeline.
    fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], shift: bool) {
        let mut mods = egui::Modifiers::default();
        mods.shift = shift;
        crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
        crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
        crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
    }

    /// A plain 2:1 view with the document centered.
    fn xf() -> ViewXform {
        ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(192.0, 144.0)),
            zoom: 2.0,
            center: [48.0, 36.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        }
    }

    fn history(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.entries().len()
    }

    fn layer_surface(app: &PhotocraftApp, id: u64) -> Vec<f32> {
        app.session
            .active()
            .unwrap()
            .doc
            .layer(LayerId(id))
            .unwrap()
            .surface()
            .unwrap()
            .read_region(Rect::new(0, 0, 96, 72))
    }

    #[test]
    fn the_anchor_starts_at_the_selection_center() {
        let (mut app, _ctx) = app(8);
        app.run("select.rect", json!({"x": 32, "y": 16, "width": 40, "height": 40})).unwrap();
        let sheet = sheet_bounds(&app).unwrap();
        let a = anchor_doc(&app.ui.screentone_ui.spec, sheet);
        assert_eq!(a, [52.0, 36.0], "the origin handle defaults to the selection's center");
    }

    #[test]
    fn a_drag_sets_angle_and_spacing_commits_one_undo() {
        let (mut app, _ctx) = app(8);
        let id = generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8, "angle": 0}));
        let count = history(&app);
        let before = layer_surface(&app, id);
        // The x arm (angle 0 by default) ends at the anchor + (spacing_x, 0) on the canvas
        // center: (48 + 8, 36).
        drag(&mut app, [56.0, 36.0], [68.0, 20.0], false);
        assert!(app.screentone_gizmo.drag.is_none(), "the drag cleared on release");
        assert_ne!(app.ui.screentone_ui.spec.angle, 0.0, "the arm's direction set the angle");
        assert_ne!(app.ui.screentone_ui.spec.spacing_x, 8.0, "the arm's length set the x spacing");
        assert_eq!(app.ui.screentone_ui.spec.spacing_y, 8.0, "the y arm stayed alone");
        assert_eq!(history(&app), count + 1, "one drag commits exactly one history step");
        assert_ne!(layer_surface(&app, id), before, "the restyle changed the sheet's pixels");
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.layer(LayerId(id)).is_some(), "undo keeps the layer");
        assert_eq!(layer_surface(&app, id), before, "undo returns the generated sheet");
    }

    #[test]
    fn shift_snaps_the_angle_and_locks_the_origin_axis() {
        let (mut app, _ctx) = app(8);
        let id = generate(&mut app, json!({"patternType": "dots", "spacingX": 12, "spacingY": 12, "angle": 0}));
        assert_eq!(app.screentone_gizmo.target, Some(LayerId(id)));
        // The origin's axis lock: from the anchor, only the dominant document axis survives.
        drag(&mut app, [48.0, 36.0], [74.0, 20.0], true);
        let spec = &app.ui.screentone_ui.spec;
        assert!((spec.origin_x - 26.0).abs() < 1e-4, "the x axis dominates: {}", spec.origin_x);
        assert_eq!(spec.origin_y, 0.0, "the y component dropped on shift");
        // The angle snap: a throw at about −36° (rel (22, −16) from the phase-shifted anchor)
        // lands on the −30° step. The pending spec carries the origin's phase (26 on the
        // local x from the axis lock above); the arm end sits at that anchor + 12 along the
        // angle axis.
        drag(&mut app, [86.0, 36.0], [96.0, 20.0], true);
        let snapped = app.ui.screentone_ui.spec.angle;
        assert!((snapped + 30.0).abs() < 1e-3, "the throw landed on the −30° step: {snapped}");
    }

    #[test]
    fn escape_restores_the_spec_the_drag_started_from() {
        let (mut app, _ctx) = app(8);
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8}));
        let snapshot = app.ui.screentone_ui.spec.clone();
        let count = history(&app);
        app.screentone_gizmo.drag = Some(Drag { kind: Handle::Origin, start_doc: [48.0, 36.0], spec: snapshot.clone() });
        app.ui.screentone_ui.spec.angle = -120.0;
        cancel_drag(&mut app);
        assert_eq!(app.ui.screentone_ui.spec, snapshot, "the drag restored its snapshot");
        assert!(app.screentone_gizmo.drag.is_none(), "the drag cleared");
        assert_eq!(history(&app), count, "a cancelled drag writes nothing");
    }

    #[test]
    fn the_commit_restyles_the_bound_sheet_in_place() {
        let (mut app, _ctx) = app(8);
        let id = generate(&mut app, json!({"patternType": "checker", "spacingX": 10, "spacingY": 10, "angle": 0, "name": "Mesh"}));
        let name = app.session.active().unwrap().doc.layer(LayerId(id)).unwrap().name.clone();
        let before = layer_surface(&app, id);
        // Press on the arm end (the anchor + (10, 0)) and slide it right.
        drag(&mut app, [58.0, 36.0], [66.0, 36.0], false);
        let st = app.session.active().unwrap();
        let l = st.doc.layer(LayerId(id)).unwrap();
        assert_eq!(l.name, name, "the sheet keeps its name");
        assert!(l.mask.is_none(), "an unmasked generation stays unmasked");
        assert_ne!(layer_surface(&app, id), before, "the sheet's pixels changed with the restyle");
    }

    #[test]
    fn a_still_press_commits_nothing() {
        let (mut app, _ctx) = app(8);
        let id = generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8, "angle": 0}));
        let count = history(&app);
        // Take the arm end and release it in place: the spec did not change, so nothing runs.
        drag(&mut app, [56.0, 36.0], [56.0, 36.0], false);
        assert_eq!(history(&app), count, "a still press writes no history");
        assert_eq!(app.screentone_gizmo.target, Some(LayerId(id)), "the binding survives idle presses");
    }

    #[test]
    fn the_target_dies_with_the_document() {
        let (mut app, _ctx) = app(8);
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8}));
        assert!(target_layer(&app).is_some(), "the fresh sheet binds");
        app.ui.screentone_ui.spec.angle = 15.0;
        // Start an origin drag on the bound screentone layer, then switch documents before release.
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 48.0, y: 36.0, pressure: 1.0 }, egui::Modifiers::default());
        assert!(app.screentone_gizmo.drag.is_some(), "origin drag started");
        app.run("file.new", json!({"width": 64, "height": 48, "depth": 8})).unwrap();
        app.sync_views();
        app.ui.panels.color = true;
        app.ui.dock_tabs.color = crate::screentone_panel::COLOR_TAB;
        assert!(target_layer(&app).is_none(), "another document drops the stale binding");
        // Finishing the drag with a stale target commits nothing, and hides the gizmo on release.
        let count = history(&app);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 56.0, y: 36.0, pressure: 1.0 }, egui::Modifiers::default());
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 56.0, y: 36.0 }, egui::Modifiers::default());
        assert_eq!(app.ui.screentone_ui.spec.angle, 15.0, "an origin drag leaves the angle alone");
        assert_eq!(history(&app), count, "a stale target commits nothing");
        assert!(!showing(&app), "without a screentone layer on the new document, the gizmo hides");
    }

    #[test]
    fn the_gizmo_grabs_through_the_view() {
        let (mut app, _ctx) = app(8);
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8, "angle": 0}));
        // Zoom 4 and a 90° camera rotation: the document-space handle points still answer,
        // through the real projector, because the grab distances stay screen px.
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(384.0, 288.0)),
            zoom: 4.0,
            center: [48.0, 36.0],
            flip: false,
            rotation: 90.0,
            aspect: 1.0,
        };
        assert_eq!(hit(&app, &xf, [48.0, 36.0]), Some(Handle::Origin), "the origin answers on a rotated zoomed view");
        assert_eq!(hit(&app, &xf, [56.0, 36.0]), Some(Handle::ArmX), "the x arm answers through rotation");
        // Mid-arm space is no handle: the press falls through to the tools.
        assert_eq!(hit(&app, &xf, [52.0, 36.0]), None, "mid-arm space falls through");
    }

    #[test]
    fn the_panel_and_the_gizmo_edit_one_spec() {
        let (mut app, _ctx) = app(8);
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8}));
        drag(&mut app, [48.0, 36.0], [54.0, 36.0], false);
        // The panel's serialized params carry the drag's values: one spec, two editors.
        let p = crate::screentone_panel::params(&app.ui.screentone_ui);
        assert_eq!(p["originX"].as_f64().unwrap(), 6.0, "the gizmo's origin reached the panel params");
        assert!(p["layer"].is_null(), "the params stay command-agnostic until the commit");
    }

    #[test]
    fn the_preview_matches_the_sheet_bounds() {
        let (mut app, ctx) = app(8);
        // A fresh unmasked sheet covers the whole canvas (the engine's sheet-bounds rule:
        // no mask, no selection). While idle (`drag.is_none()`), no duplicate preview texture is
        // built; starting a handle drag builds the preview over the sheet's bounds, and releasing
        // clears it again.
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8, "angle": 0}));
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_none(), "no duplicate preview mesh while idle");

        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 48.0, y: 36.0, pressure: 1.0 }, egui::Modifiers::default());
        refresh_preview(&mut app, &ctx, &xf(), None);
        let cache = app.screentone_gizmo.preview.as_ref().expect("the preview rendered during drag");
        assert_eq!(cache.rect, Rect::new(0, 0, 96, 72), "the region is the sheet's bounds");
        assert!(cache.texture.size()[0] > 0 && cache.texture.size()[1] > 0, "the texture has content");

        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 48.0, y: 36.0 }, egui::Modifiers::default());
        assert!(app.screentone_gizmo.preview.is_none(), "releasing the drag clears the preview mesh");
    }

    #[test]
    fn the_preview_multiplies_the_coverage() {
        let (mut app, ctx) = app(8);
        app.run("select.rect", json!({"x": 32, "y": 16, "width": 40, "height": 40})).unwrap();
        generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8}));
        // Interior selection mask restricts `sheet_bounds` and the live drag preview strictly to
        // the selection rect (32..72, 16..56), never leaking across the whole 96×72 canvas.
        assert_eq!(sheet_bounds(&app), Some(Rect::new(32, 16, 72, 56)));
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 52.0, y: 36.0, pressure: 1.0 }, egui::Modifiers::default());
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert_eq!(app.screentone_gizmo.preview.as_ref().unwrap().rect, Rect::new(32, 16, 72, 56));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 52.0, y: 36.0 }, egui::Modifiers::default());

        let (region, fmt) = {
            let st = app.session.active().unwrap();
            (st.doc.bounds(), st.doc.pixel_format())
        };
        let p = crate::screentone_panel::params(&app.ui.screentone_ui);
        let spec = photocraft_engine::screentone_cmds::spec(&p);
        let params = FilterParams::Screentone { spec: spec_with(spec) };
        let raw = render_preview(fmt, region, region, &params, None);
        assert!(raw.px.iter().any(|px| px[3] > 0.0), "the raw sheet has ink");
        // A mask hiding the sheet's left half (absolute document coordinates) gates the tone
        // off there; the right half keeps the raw sheet, channel for channel.
        let mut mask = photocraft_doc::LayerMask::reveal_all();
        let w = region.width();
        let hidden = Rect::new(region.x0, region.y0, region.x0 + (w / 2) as i32, region.y1);
        mask.surface.write_region(hidden, &vec![0.0; w as usize / 2 * region.height() as usize]);
        let gated = render_preview(fmt, region, region, &params, Some((&mask.surface, region)));
        let mid_y = region.y0 + (region.height() / 2) as i32;
        assert_eq!(gated.get(region.x0 + 1, mid_y)[3], 0.0, "the hidden half shows nothing");
        assert_eq!(gated.get(region.x0 + (w / 2) as i32 + 1, mid_y), raw.get(region.x0 + (w / 2) as i32 + 1, mid_y), "the revealed half matches the raw sheet");
    }

    #[test]
    fn the_preview_region_respects_the_cap() {
        let (mut app, _ctx) = app(8);
        let r = preview_rect(&xf(), sheet_bounds(&app).unwrap()).unwrap();
        assert!(r.width() as usize * r.height() as usize <= PREVIEW_CAP, "the small sheet fits");
        // A 3000 × 2400 sheet (7.2 MP) passes the cap: no region, no texture.
        app.run("file.new", json!({"width": 3000, "height": 2400, "depth": 8})).unwrap();
        app.sync_views();
        app.ui.panels.color = true;
        app.ui.dock_tabs.color = crate::screentone_panel::COLOR_TAB;
        let sheet = sheet_bounds(&app).unwrap();
        let big = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(3000.0, 2400.0)),
            zoom: 1.0,
            center: [1500.0, 1200.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        assert_eq!(preview_rect(&big, sheet), None, "the hypersheet region is skipped");
    }

    #[test]
    fn test_gizmo_showing_only_when_screentone_layer_active() {
        let (mut app, ctx) = app(8);
        // Simply opening the Screentones tab on a raster layer must NOT show the gizmo or preview.
        assert!(!showing(&app), "raster Background must not activate the gizmo");
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_none(), "no overlay preview on a raster layer");

        // Converting a pixel layer to a Smart Object (even with a screentone Smart Filter) keeps
        // the gizmo hidden: smart objects are not procedural screentone layers.
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run(photocraft_engine::screentone_cmds::FILTER, json!({"patternType": "dots", "useSourceLuminance": true})).unwrap();
        assert!(!showing(&app), "Smart Object with screentone smart filter must not activate the gizmo");
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_none(), "no overlay preview on a Smart Object");

        // Creating a live Screentone layer makes it active and reveals the gizmo handles,
        // without projecting a duplicate preview mesh while idle.
        let tone_id = generate(&mut app, json!({"patternType": "dots", "spacingX": 8, "spacingY": 8, "angle": 0}));
        assert!(showing(&app), "active Screentone layer activates the gizmo handles");
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_none(), "idle Screentone layer uses the compositor without a duplicate preview mesh");
        assert!(display_doc(&app, 0).is_none(), "compositor shows the real Screentone layer while idle");

        // During an active handle drag, the preview mesh activates and `display_doc` hides the
        // underlying layer to prevent double Multiply rendering.
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 48.0, y: 36.0, pressure: 1.0 }, egui::Modifiers::default());
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_some(), "preview mesh renders during active drag");
        let (hidden_doc, _) = display_doc(&app, 0).expect("display_doc hides original layer during drag");
        assert!(!hidden_doc.layer(LayerId(tone_id)).unwrap().visible, "target layer is hidden in composite during drag");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 48.0, y: 36.0 }, egui::Modifiers::default());
        assert!(app.screentone_gizmo.preview.is_none(), "preview clears on drag release");

        // Rasterizing the screentone layer turns it into a plain pixel layer and hides the gizmo.
        app.run("layer.rasterize.layer", json!({"layer": tone_id})).unwrap();
        assert!(!showing(&app), "rasterized layer no longer shows the gizmo");
        refresh_preview(&mut app, &ctx, &xf(), None);
        assert!(app.screentone_gizmo.preview.is_none(), "preview remains None once rasterized");
    }

    #[test]
    fn switching_active_screentone_layer_syncs_panel_spec() {
        let (mut app, _ctx) = app(8);
        let id1 = generate(&mut app, json!({"patternType": "dots", "dotSize": 5.0, "spacingX": 10.0, "spacingY": 10.0, "angle": 15.0}));
        let id2 = generate(&mut app, json!({"patternType": "lines", "dotSize": 9.0, "spacingX": 20.0, "spacingY": 20.0, "angle": 60.0}));
        assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(id2)));

        // Select the first screentone layer in the Layers panel and sync.
        app.session.select_layer(LayerId(id1)).unwrap();
        sync_active_layer(&mut app);
        assert_eq!(app.ui.screentone_ui.spec.dot_size, 5.0);
        assert_eq!(app.ui.screentone_ui.spec.spacing_x, 10.0);
        assert_eq!(app.ui.screentone_ui.spec.angle, 15.0);
        assert_eq!(target_layer(&app), Some(LayerId(id1)));

        // Editing the panel spec updates the active screentone layer in place.
        app.ui.screentone_ui.spec.dot_size = 11.0;
        app.ui.screentone_ui.spec.spacing_x = 18.0;
        assert!(crate::screentone_panel::sync_spec_to_active_layer(&mut app));
        let st = app.session.active().unwrap();
        let photocraft_doc::LayerContent::Screentone(tone1) = &st.doc.layer(LayerId(id1)).unwrap().content else {
            panic!("expected Screentone layer");
        };
        assert_eq!(tone1.spec.dot_size, 11.0);
        assert_eq!(tone1.spec.spacing_x, 18.0);
    }
}
