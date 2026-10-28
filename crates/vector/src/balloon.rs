//! Balloon geometry for manga/comic lettering: editable tails, procedural balloon shapes and
//! mouse-friendly vector linework.
//!
//! Everything here produces plain [`Path`]s, so fill, stroke, Free Transform, history and export
//! keep working exactly as for the existing shape primitives. Nothing is rasterized until the
//! shape layer is composited.
//!
//! * [`BalloonTail`] + [`attach_tail`]: a tail merged with the balloon body by a real polygon
//!   union, so one fill and one stroke outline the whole balloon without a seam.
//! * [`cloud`]: a thought-cloud contour (scalloped bumps along an ellipse or rectangle).
//! * [`spiky`]: a shout/flash balloon with sharp spikes pointing out or in.
//! * [`catmull_rom_path`], [`sai_stroke`], [`tapered_outline`]: click-to-click vector linework in
//!   the Paint Tool SAI spirit — a click adds a node and the segment is smoothed automatically.
//!
//! Point-level editing of the result (Ctrl-drag, Alt-click toggling corners, deleting anchors)
//! already lives in [`crate::edit`], which works on any [`Path`] this module returns.

use std::f64::consts::TAU;

use photocraft_doc::{FillRule, Knot, Path, PathOp, Subpath};
use photocraft_geom::Point;

use crate::flatten::flatten_subpath;

/// A point in path space. Kept as a tuple internally; [`Path`] takes [`Point`] knots.
type P = (f64, f64);

/// Tolerance used when a curved balloon body is flattened for a boolean union (px).
const UNION_TOL: f64 = 0.05;
/// Points this close are treated as the same node when stitching a union boundary.
const JOIN_EPS: f64 = 1e-6;

// ---------------------------------------------------------------------------------------------
// Small tuple geometry helpers (no vector type of our own: `photocraft-geom` has no `Vec2`).
// ---------------------------------------------------------------------------------------------

fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}

fn mul(a: P, s: f64) -> P {
    (a.0 * s, a.1 * s)
}


fn cross(a: P, b: P) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn len(a: P) -> f64 {
    a.0.hypot(a.1)
}

fn dist(a: P, b: P) -> f64 {
    len(sub(a, b))
}

fn norm(a: P) -> P {
    let l = len(a);
    if l < 1e-12 { (0.0, 0.0) } else { (a.0 / l, a.1 / l) }
}

fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn near(a: P, b: P) -> bool {
    dist(a, b) <= JOIN_EPS
}

fn to_point(p: P) -> Point {
    Point::new(p.0, p.1)
}

/// Deterministic pseudo-random value in `0..=1` from a seed and a node index (no RNG
/// dependency, and the same document always renders the same balloon).
fn hash01(seed: u32, index: u32) -> f64 {
    let mut x = seed ^ index.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    f64::from(x) / f64::from(u32::MAX)
}

/// `value` scaled by a deterministic ±`amount` jitter around 1.
fn jittered(value: f64, seed: u32, index: u32, amount: f64) -> f64 {
    value * (1.0 + amount * (hash01(seed, index) - 0.5) * 2.0)
}

// ---------------------------------------------------------------------------------------------
// Polygon union (the "∪" of a balloon body and its tail).
// ---------------------------------------------------------------------------------------------

/// Signed area of a closed polygon; positive is counter-clockwise in y-down path space.
fn signed_area(poly: &[P]) -> f64 {
    let n = poly.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        area += a.0 * b.1 - b.0 * a.1;
    }
    area * 0.5
}

/// Drops repeated and closing points.
fn clean(poly: &[P]) -> Vec<P> {
    let mut out: Vec<P> = Vec::with_capacity(poly.len());
    for &p in poly {
        if out.last().is_none_or(|&q| dist(q, p) > JOIN_EPS) {
            out.push(p);
        }
    }
    while out.len() > 1 && dist(out[0], out[out.len() - 1]) <= JOIN_EPS {
        out.pop();
    }
    out
}

fn oriented(poly: &[P]) -> Vec<P> {
    let mut p = clean(poly);
    if signed_area(&p) < 0.0 {
        p.reverse();
    }
    p
}

/// Ray-casting point-in-polygon test; points exactly on the boundary are unspecified (callers
/// only test midpoints of pieces that do not lie on the other boundary by construction).
fn inside(p: P, poly: &[P]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if (a.1 > p.1) != (b.1 > p.1) {
            let t = (p.1 - a.1) / (b.1 - a.1);
            let x = a.0 + t * (b.0 - a.0);
            if p.0 < x {
                inside = !inside;
            }
        }
    }
    inside
}


/// Proper interior crossing of two segments (endpoints and collinear overlaps are ignored; the
/// union only needs to split where one boundary truly enters the other).
fn crossing_param(a0: P, a1: P, b0: P, b1: P) -> Option<f64> {
    let r = sub(a1, a0);
    let s = sub(b1, b0);
    let denom = cross(r, s);
    if denom.abs() < 1e-12 {
        return None;
    }
    let qp = sub(b0, a0);
    let ta = cross(qp, s) / denom;
    let tb = cross(qp, r) / denom;
    let inside_segment = |t: f64| t > 1e-9 && t < 1.0 - 1e-9;
    if inside_segment(ta) && inside_segment(tb) { Some(ta) } else { None }
}

/// Splits every edge of `poly` at its crossings with `other`.
fn split_against(poly: &[P], other: &[P]) -> Vec<[P; 2]> {
    let n = poly.len();
    let m = other.len();
    let mut pieces = Vec::new();
    if n < 2 || m < 2 {
        return pieces;
    }
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let mut cuts: Vec<f64> = vec![0.0, 1.0];
        for j in 0..m {
            if let Some(t) = crossing_param(a, b, other[j], other[(j + 1) % m]) {
                cuts.push(t);
            }
        }
        cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        for w in cuts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 > 1e-9 {
                pieces.push([lerp(a, b, t0), lerp(a, b, t1)]);
            }
        }
    }
    pieces
}

/// Chains directed boundary pieces into closed loops by matching their endpoints.
fn stitch(pieces: &[[P; 2]]) -> Vec<Vec<P>> {
    let mut pool: Vec<[P; 2]> = pieces.to_vec();
    let mut loops = Vec::new();
    while let Some(start) = pool.pop() {
        let mut poly = vec![start[0], start[1]];
        loop {
            let end = poly[poly.len() - 1];
            let Some(index) = pool.iter().position(|s| near(s[0], end)) else { break };
            let next = pool.swap_remove(index);
            let next_end = next[1];
            poly.push(next_end);
            if near(next_end, poly[0]) {
                break;
            }
        }
        poly.pop();
        let poly = clean(&poly);
        if poly.len() >= 3 && signed_area(&poly).abs() > JOIN_EPS {
            loops.push(poly);
        }
    }
    loops
}

/// Union of two simple closed polygons. Returns the boundary contours of the union (two contours
/// when the inputs do not touch, one when they overlap or one contains the other). The inputs are
/// treated as sets, so winding direction does not matter.
pub fn union_polygons(a: &[P], b: &[P]) -> Vec<Vec<P>> {
    let a = oriented(a);
    let b = oriented(b);
    if a.len() < 3 {
        return if b.len() >= 3 { vec![b] } else { Vec::new() };
    }
    if b.len() < 3 {
        return vec![a];
    }

    let mut pieces: Vec<[P; 2]> = Vec::new();
    for seg in split_against(&a, &b) {
        if !inside(lerp(seg[0], seg[1], 0.5), &b) {
            pieces.push(seg);
        }
    }
    for seg in split_against(&b, &a) {
        if !inside(lerp(seg[0], seg[1], 0.5), &a) {
            pieces.push(seg);
        }
    }
    if pieces.is_empty() {
        // Identical or fully coincident boundaries: nothing outside either polygon.
        return vec![a];
    }
    let loops = stitch(&pieces);
    if loops.is_empty() { vec![a] } else { loops }
}

/// Whether two closed polygons overlap or one contains the other (used to decide whether a new
/// tail joins an existing contour or becomes its own).
fn overlaps(a: &[P], b: &[P]) -> bool {
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    if inside(a[0], b) || inside(b[0], a) {
        return true;
    }
    for i in 0..a.len() {
        let a0 = a[i];
        let a1 = a[(i + 1) % a.len()];
        for j in 0..b.len() {
            if crossing_param(a0, a1, b[j], b[(j + 1) % b.len()]).is_some() {
                return true;
            }
        }
    }
    false
}

/// Merges one more shape into the evolving set of union contours.
fn merge_shape(contours: &mut Vec<Vec<P>>, shape: Vec<P>) {
    let shape = oriented(&shape);
    if shape.len() < 3 {
        return;
    }
    let mut i = 0;
    while i < contours.len() {
        if overlaps(&contours[i], &shape) {
            let merged = union_polygons(&contours[i], &shape);
            contours.remove(i);
            for (offset, contour) in merged.into_iter().enumerate() {
                contours.insert(i + offset, contour);
            }
            return;
        }
        i += 1;
    }
    contours.push(shape);
}

// ---------------------------------------------------------------------------------------------
// Stage 1: balloon tails and their union with the body.
// ---------------------------------------------------------------------------------------------

/// Prebuilt tail styles, mirroring the common manga tail vocabulary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TailPreset {
    /// Straight tapered wedge (the everyday speech tail).
    #[default]
    Wedge,
    /// Wedge whose sides bow outward, for a softer, hand-drawn look.
    Arc,
    /// Two thin tails leaving the same spot (one speaker speaking two lines, or emphasis).
    Double,
    /// A trail of shrinking circles: the thought-balloon tail.
    Thought,
}

impl TailPreset {
    /// Stable id for UI presets and the control protocol.
    pub const fn id(self) -> &'static str {
        match self {
            TailPreset::Wedge => "wedge",
            TailPreset::Arc => "arc",
            TailPreset::Double => "double",
            TailPreset::Thought => "thought",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "wedge" => Some(TailPreset::Wedge),
            "arc" => Some(TailPreset::Arc),
            "double" => Some(TailPreset::Double),
            "thought" => Some(TailPreset::Thought),
            _ => None,
        }
    }

    /// The offset of the two attachment points relative to the tail axis, in multiples of the
    /// base width: the tail's mouth on the balloon contour.
    pub const fn mouth(self) -> f64 {
        match self {
            TailPreset::Wedge | TailPreset::Arc => 0.5,
            TailPreset::Double => 0.35,
            TailPreset::Thought => 0.0,
        }
    }
}

/// A tail dragged from a balloon body towards the speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BalloonTail {
    pub preset: TailPreset,
    /// The two points where the tail meets the balloon contour.
    pub base: [P; 2],
    /// Where the tail points (the character or the words).
    pub tip: P,
    /// Side bulge, `0..=1`; ignored by [`TailPreset::Thought`].
    pub curve: f64,
    /// Thought preset: number of bubbles in the trail.
    pub bubbles: u32,
    /// Thought preset: radius of the first (largest) bubble in px.
    pub bubble_radius: f64,
}

impl BalloonTail {
    /// A wedge tail between two attachment points and a tip.
    pub fn wedge(base: [P; 2], tip: P) -> Self {
        Self { preset: TailPreset::Wedge, base, tip, curve: 0.12, bubbles: 3, bubble_radius: 6.0 }
    }

    /// An arc tail between two attachment points and a tip.
    pub fn arc(base: [P; 2], tip: P) -> Self {
        Self { preset: TailPreset::Arc, base, tip, curve: 0.45, bubbles: 3, bubble_radius: 6.0 }
    }

    /// A thought-bubble trail leaving `base` towards `tip`.
    pub fn thought(base: [P; 2], tip: P) -> Self {
        Self { preset: TailPreset::Thought, base, tip, curve: 0.0, bubbles: 3, bubble_radius: 6.0 }
    }

    /// One tail built from a drag: the mouth is perpendicular to the drag direction at `start`,
    /// `mouth_width` wide, and the tail points towards `tip`.
    pub fn from_drag(preset: TailPreset, start: P, tip: P, mouth_width: f64) -> Self {
        let axis = norm(sub(tip, start));
        let side = (-axis.1, axis.0);
        let half = mouth_width.abs() * 0.5;
        let base = [sub(start, mul(side, half)), add(start, mul(side, half))];
        let curve = match preset {
            TailPreset::Wedge => 0.12,
            TailPreset::Arc => 0.45,
            TailPreset::Double | TailPreset::Thought => 0.0,
        };
        Self { preset, base, tip, curve, bubbles: 3, bubble_radius: 6.0 }
    }
}


/// Shape of one tail polygon: sampled quadratic sides bowed away from the tail's centroid.
fn tail_contour(b0: P, b1: P, tip: P, bulge: f64) -> Vec<P> {
    let steps = 14usize;
    let centroid = mul(add(add(b0, b1), tip), 1.0 / 3.0);
    let mut pts = vec![b0];
    // b0 -> tip
    pts.extend(quad_side(b0, tip, bulge, steps, centroid).into_iter().skip(1));
    // tip -> b1 (the sample already contains the tip)
    pts.extend(quad_side(tip, b1, bulge, steps, centroid).into_iter().skip(1));
    pts
}

/// Quadratic Bézier from `s` to `e` with a control point pushed away from `centroid`.
fn quad_side(s: P, e: P, bulge: f64, steps: usize, centroid: P) -> Vec<P> {
    let mid = lerp(s, e, 0.5);
    let mut away = norm(sub(mid, centroid));
    if away == (0.0, 0.0) {
        let d = norm(sub(e, s));
        away = (-d.1, d.0);
    }
    let ctrl = add(mid, mul(away, bulge.clamp(0.0, 1.0) * dist(s, e) * 0.5));
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let mt = 1.0 - t;
            add(add(mul(s, mt * mt), mul(ctrl, 2.0 * mt * t)), mul(e, t * t))
        })
        .collect()
}

fn circle_contour(center: P, radius: f64, steps: usize) -> Vec<P> {
    (0..steps)
        .map(|i| {
            let a = TAU * i as f64 / steps as f64;
            (center.0 + radius * a.cos(), center.1 + radius * a.sin())
        })
        .collect()
}

/// Flattens the closed subpaths of `path` into polygons (open subpaths are ignored).
fn closed_contours(path: &Path) -> Vec<Vec<P>> {
    let mut out = Vec::new();
    for subpath in &path.subpaths {
        if !subpath.closed || subpath.knots.len() < 3 {
            continue;
        }
        let flat = flatten_subpath(subpath, UNION_TOL);
        let poly = clean(&flat.pts);
        if poly.len() >= 3 {
            out.push(poly);
        }
    }
    out
}

/// Pushes a point towards `toward` by `amount` (used to bury the tail mouth inside the body so the
/// union cuts it away cleanly instead of leaving a seam on the contour).
fn push_inside(p: P, toward: P, amount: f64) -> P {
    let d = norm(sub(toward, p));
    if d == (0.0, 0.0) { p } else { add(p, mul(d, amount)) }
}

fn centroid(poly: &[P]) -> P {
    let n = poly.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let mut c = (0.0, 0.0);
    for &p in poly {
        c = add(c, p);
    }
    mul(c, 1.0 / n as f64)
}

/// Contours of the tail alone (one for wedge/arc, two for double, one per bubble for thought).
fn tail_shapes(tail: &BalloonTail, mouth_inset: f64) -> Vec<Vec<P>> {
    let mut base = tail.base;
    if tail.preset == TailPreset::Thought {
        // Bubbles leave the midpoint of the mouth and shrink towards the tip.
        let start = lerp(base[0], base[1], 0.5);
        let count = tail.bubbles.clamp(1, 12) as usize;
        let axis = norm(sub(tail.tip, start));
        let radius0 = tail.bubble_radius.max(0.5);
        return (0..count)
            .map(|i| {
                let t = (i + 1) as f64 / (count + 1) as f64;
                let center = add(lerp(start, tail.tip, t), mul(axis, radius0 * 0.6));
                let radius = radius0 * (1.0 - 0.55 * t);
                circle_contour(center, radius, 32)
            })
            .collect();
    }
    if mouth_inset > 0.0 {
        let body_center = lerp(base[0], base[1], 0.5);
        let away = norm(sub(body_center, tail.tip));
        base = [add(base[0], mul(away, mouth_inset)), add(base[1], mul(away, mouth_inset))];
    }
    match tail.preset {
        TailPreset::Double => {
            let axis = norm(sub(tail.tip, lerp(base[0], base[1], 0.5)));
            let side = (-axis.1, axis.0);
            let spread = dist(base[0], base[1]).max(1.0) * 0.3;
            let tip_a = add(tail.tip, mul(side, spread));
            let tip_b = sub(tail.tip, mul(side, spread));
            vec![
                tail_contour(base[0], lerp(base[0], base[1], 0.5), tip_a, tail.curve),
                tail_contour(lerp(base[0], base[1], 0.5), base[1], tip_b, tail.curve),
            ]
        }
        _ => vec![tail_contour(base[0], base[1], tail.tip, tail.curve)],
    }
}

/// Attaches `tail` to `body` and returns the merged path: one contour (or several, for thought
/// bubbles), one fill, one stroke, no line where the tail meets the body.
///
/// The body's curves are flattened at [`UNION_TOL`] px for the boolean step — visually
/// indistinguishable at screen and print resolutions — and the merged result is a normal editable
/// path, so Free Transform, Direct Selection and PSD/`.pcraft` export are unaffected.
pub fn attach_tail(body: &Path, tail: &BalloonTail) -> Path {
    let mut contours = closed_contours(body);
    let open: Vec<Subpath> = body.subpaths.iter().filter(|s| !s.closed).cloned().collect();
    let shape_poly = contours.first().map(|c| centroid(c));
    let mouth_inset = shape_poly.map(|c| (dist(c, tail.tip) * 0.02).clamp(0.25, 8.0)).unwrap_or(0.0);
    let body_center = shape_poly.unwrap_or_else(|| lerp(tail.base[0], tail.base[1], 0.5));

    let mut base = tail.base;
    if tail.preset != TailPreset::Thought && mouth_inset > 0.0 {
        base = [push_inside(base[0], body_center, mouth_inset), push_inside(base[1], body_center, mouth_inset)];
    }
    let mut tail = *tail;
    tail.base = base;

    if contours.is_empty() {
        // No body to merge with: return the tail alone.
        let mut path = Path::new(Vec::new());
        for shape in tail_shapes(&tail, 0.0) {
            path.subpaths.push(Subpath::polygon(&shape));
        }
        path.subpaths.extend(open);
        return path;
    }

    let mut merged: Vec<Vec<P>> = Vec::new();
    if let Some(first) = contours.first().cloned() {
        merged.push(first);
    }
    for shape in tail_shapes(&tail, mouth_inset) {
        merge_shape(&mut merged, shape);
    }
    // Contours are one connected balloon; keep any extra body contours as they were.
    for contour in contours.drain(1..) {
        merged.push(contour);
    }

    let mut path = Path::new(Vec::new());
    for contour in merged {
        if contour.len() >= 3 {
            path.subpaths.push(Subpath::polygon(&contour));
        }
    }
    path.subpaths.extend(open);
    path.fill_rule = FillRule::NonZero;
    path
}

// ---------------------------------------------------------------------------------------------
// Stage 2: procedural balloons.
// ---------------------------------------------------------------------------------------------

/// Parameters of a thought cloud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudParams {
    /// Number of scalloped bumps around the contour (3..=64).
    pub bumps: u32,
    /// Bump height as a fraction of the balloon's smaller radius (0..=1).
    pub amplitude: f64,
    /// Per-bump size variation, 0 = perfectly regular (0..=1).
    pub jitter: f64,
    /// Seed of the deterministic jitter.
    pub seed: u32,
}

impl Default for CloudParams {
    fn default() -> Self {
        Self { bumps: 9, amplitude: 0.18, jitter: 0.35, seed: 1 }
    }
}

/// Parameters of a spiky shout/flash balloon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpikeParams {
    /// Number of spikes (3..=128).
    pub spikes: u32,
    /// Spike length as a fraction of the balloon radius (0..=2).
    pub length: f64,
    /// Per-spike length variation (0..=1).
    pub jitter: f64,
    /// Spikes point towards the centre (a burst-in outline) instead of outwards.
    pub inward: bool,
    /// Smooth the spikes into rounded rays instead of sharp corners.
    pub smooth: bool,
    pub seed: u32,
}

impl Default for SpikeParams {
    fn default() -> Self {
        Self { spikes: 12, length: 0.45, jitter: 0.35, inward: false, smooth: false, seed: 1 }
    }
}

fn rect_ellipse(rect: [f64; 4]) -> (P, f64, f64) {
    let [x, y, w, h] = rect;
    let (cx, cy) = (x + w * 0.5, y + h * 0.5);
    ((cx, cy), (w.abs() * 0.5).max(0.5), (h.abs() * 0.5).max(0.5))
}

fn ellipse_point(center: P, rx: f64, ry: f64, angle: f64) -> P {
    (center.0 + rx * angle.cos(), center.1 + ry * angle.sin())
}

/// Outward unit normal of the ellipse at `angle`.
fn ellipse_normal(rx: f64, ry: f64, angle: f64) -> P {
    norm((angle.cos() / rx, angle.sin() / ry))
}

/// Uniform Catmull-Rom spline through `points`, emitted as cubic Bézier knots.
///
/// `tension` 1 is the standard spline, 0 retracts the handles into straight segments.
pub fn catmull_rom_knots(points: &[P], closed: bool, tension: f64) -> Vec<Knot> {
    let n = points.len();
    let tension = tension.clamp(0.0, 1.0);
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![Knot::corner(points[0].0, points[0].1)];
    }
    let tangent = |i: usize| -> P {
        let prev = if i == 0 {
            if closed { points[n - 1] } else { points[0] }
        } else {
            points[i - 1]
        };
        let next = if i + 1 >= n {
            if closed { points[0] } else { points[n - 1] }
        } else {
            points[i + 1]
        };
        mul(sub(next, prev), 0.5 * tension)
    };
    let mut knots = Vec::with_capacity(n);
    for i in 0..n {
        let anchor = points[i];
        let t = tangent(i);
        let c1 = add(anchor, mul(t, 1.0 / 3.0));
        let c2 = sub(anchor, mul(t, 1.0 / 3.0));
        knots.push(Knot::smooth(to_point(anchor), to_point(c2), to_point(c1)));
    }
    knots
}

/// Builds a closed [`Path`] from one contour, smoothing corners with a Catmull-Rom spline.
fn smooth_path(points: &[P]) -> Path {
    if points.len() < 3 {
        return Path::default();
    }
    let knots = catmull_rom_knots(points, true, 1.0);
    Path::new(vec![Subpath { closed: true, knots, op: PathOp::Combine }])
}

/// A thought-cloud balloon inscribed in `rect` (`[x, y, w, h]`), with `bumps` scalloped bumps.
pub fn cloud(rect: [f64; 4], params: CloudParams) -> Path {
    let (center, rx, ry) = rect_ellipse(rect);
    let bumps = params.bumps.clamp(3, 64);
    let amplitude = params.amplitude.clamp(0.0, 1.0);
    let jitter = params.jitter.clamp(0.0, 1.0);
    let steps = 6usize;
    let mut points: Vec<P> = Vec::new();
    for bump in 0..bumps {
        let a0 = TAU * bump as f64 / bumps as f64;
        let a1 = TAU * (bump + 1) as f64 / bumps as f64;
        let p0 = ellipse_point(center, rx, ry, a0);
        let p1 = ellipse_point(center, rx, ry, a1);
        let am = (a0 + a1) * 0.5;
        let outward = ellipse_normal(rx, ry, am);
        let scale = rx.min(ry);
        let height = jittered(scale * amplitude, params.seed, bump, jitter).max(0.0);
        for step in 0..steps {
            let t = step as f64 / steps as f64;
            let base = lerp(p0, p1, t);
            let bulge = (std::f64::consts::PI * t).sin() * height;
            points.push(add(base, mul(outward, bulge)));
        }
    }
    smooth_path(&points)
}

/// A spiky shout/flash balloon inscribed in `rect`.
///
/// Every spike alternates between a valley on the base ellipse and a tip pushed out by
/// `length` (or pulled in when `inward`), with deterministic per-spike jitter.
pub fn spiky(rect: [f64; 4], params: SpikeParams) -> Path {
    let (center, rx, ry) = rect_ellipse(rect);
    let spikes = params.spikes.clamp(3, 128);
    let length = params.length.clamp(0.0, 2.0);
    let jitter = params.jitter.clamp(0.0, 1.0);
    let mut points: Vec<P> = Vec::with_capacity(spikes as usize * 2);
    for i in 0..spikes {
        let a0 = TAU * i as f64 / spikes as f64;
        let am = a0 + TAU * 0.5 / spikes as f64;
        let valley = ellipse_point(center, rx, ry, a0);
        points.push(valley);
        let outward = ellipse_normal(rx, ry, am);
        let tip = ellipse_point(center, rx, ry, am);
        let reach = jittered(length, params.seed, i, jitter);
        let signed = if params.inward { -reach * 0.6 } else { reach };
        let factor = rx.min(ry) * signed;
        points.push(add(tip, mul(outward, factor)));
    }
    if params.smooth {
        smooth_path(&points)
    } else if points.len() >= 3 {
        Path::new(vec![Subpath::polygon(&points)])
    } else {
        Path::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Stage 3: SAI-style click-to-click linework.
// ---------------------------------------------------------------------------------------------

/// One click-placed node of a vector stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SaiPoint {
    pub p: P,
    /// Simulated pen pressure, `0..=1`; 1 is a full-width point.
    pub pressure: f32,
    /// Sharp corner instead of a smoothed node (Alt-click toggles this in the UI).
    pub corner: bool,
}

impl SaiPoint {
    pub fn new(p: P) -> Self {
        Self { p, pressure: 1.0, corner: false }
    }
}

/// A smooth path through click-placed points (Catmull-Rom → cubic Béziers), open by default.
///
/// Corner points keep sharp knots with retracted handles; the others become smooth knots, which
/// is exactly what SAI-style click drawing produces and what Direct Selection can keep editing.
pub fn sai_stroke(points: &[SaiPoint], closed: bool) -> Path {
    if points.len() < 2 {
        return Path::default();
    }
    let positions: Vec<P> = points.iter().map(|p| p.p).collect();
    let smooth = catmull_rom_knots(&positions, closed, 1.0);
    let knots = smooth
        .into_iter()
        .zip(points.iter())
        .map(|(knot, point)| {
            if point.corner {
                Knot { anchor: knot.anchor, in_ctrl: knot.anchor, out_ctrl: knot.anchor, smooth: false }
            } else {
                knot
            }
        })
        .collect();
    Path::new(vec![Subpath { closed, knots, op: PathOp::Combine }])
}

/// Uniform-width spline through `points` (no pressure).
pub fn catmull_rom_path(points: &[P], closed: bool) -> Path {
    if points.len() < 2 {
        return Path::default();
    }
    let knots = catmull_rom_knots(points, closed, 1.0);
    Path::new(vec![Subpath { closed, knots, op: PathOp::Combine }])
}

/// Width of a tapered line at `pressure`, never below `min_ratio` of the base width.
fn width_at(base_width: f64, min_ratio: f64, pressure: f32) -> f64 {
    let ratio = f64::from(pressure.clamp(0.0, 1.0)).clamp(min_ratio.clamp(0.0, 1.0), 1.0);
    (base_width * ratio).max(0.05)
}

/// A filled polygon approximating a variable-width stroke through `points` — the "weight per
/// node" of the SAI-style tool, used for ink lines with sharp tapers.
///
/// The polygon is built from the left and right offsets of the sampled centreline plus round
/// caps, so a pressure of 0 at the ends naturally gives pointed stroke tips.
pub fn tapered_outline(points: &[SaiPoint], base_width: f64, min_ratio: f64) -> Vec<P> {
    let pts: Vec<P> = points.iter().map(|p| p.p).collect();
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let base_width = base_width.abs().max(0.05);
    let mut left: Vec<P> = Vec::with_capacity(n);
    let mut right: Vec<P> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = if i == 0 {
            if n > 1 { pts[0] } else { pts[0] }
        } else {
            pts[i - 1]
        };
        let next = if i + 1 >= n { pts[n - 1] } else { pts[i + 1] };
        let mut dir = norm(sub(next, prev));
        if dir == (0.0, 0.0) {
            dir = norm(sub(pts[(i + 1).min(n - 1)], pts[i.saturating_sub(1)]));
        }
        if dir == (0.0, 0.0) {
            dir = (1.0, 0.0);
        }
        let normal = (-dir.1, dir.0);
        let half = width_at(base_width, min_ratio, points[i].pressure) * 0.5;
        left.push(add(pts[i], mul(normal, half)));
        right.push(sub(pts[i], mul(normal, half)));
    }
    let mut poly: Vec<P> = Vec::with_capacity(n * 2 + 16);
    poly.extend(left.iter().copied());
    // Round cap at the last point.
    let cap_dir = norm(sub(pts[n - 1], pts[n - 2]));
    let cap_normal = (-cap_dir.1, cap_dir.0);
    let cap_half = width_at(base_width, min_ratio, points[n - 1].pressure) * 0.5;
    for i in 1..8 {
        let angle = std::f64::consts::PI * i as f64 / 8.0;
        let offset = add(mul(cap_normal, cap_half * angle.cos()), mul(cap_dir, cap_half * angle.sin()));
        poly.push(add(pts[n - 1], offset));
    }
    poly.extend(right.iter().rev().copied());
    // Round cap at the first point.
    let start_dir = norm(sub(pts[0], pts[1]));
    let start_normal = (-start_dir.1, start_dir.0);
    let start_half = width_at(base_width, min_ratio, points[0].pressure) * 0.5;
    for i in 1..8 {
        let angle = std::f64::consts::PI * i as f64 / 8.0;
        let offset = add(mul(start_normal, start_half * angle.cos()), mul(start_dir, start_half * angle.sin()));
        poly.push(add(pts[0], offset));
    }
    clean(&poly)
}

/// A path for the tapered stroke (a filled outline, ready for a shape layer with no stroke).
pub fn tapered_path(points: &[SaiPoint], base_width: f64, min_ratio: f64) -> Path {
    let outline = tapered_outline(points, base_width, min_ratio);
    if outline.len() < 3 {
        Path::default()
    } else {
        Path::new(vec![Subpath::polygon(&outline)])
    }
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Vec<P> {
        vec![(x, y), (x + s, y), (x + s, y + s), (x, y + s)]
    }

    fn area(contours: &[Vec<P>]) -> f64 {
        contours.iter().map(|c| signed_area(c).abs()).sum()
    }

    #[test]
    fn union_of_two_overlapping_squares_is_one_contour() {
        let a = square(0.0, 0.0, 10.0);
        let b = square(5.0, 5.0, 10.0);
        let merged = union_polygons(&a, &b);
        assert_eq!(merged.len(), 1, "overlapping squares join into one contour");
        // 10×10 + 10×10 − 5×5 overlap = 175.
        assert!((area(&merged) - 175.0).abs() < 0.01, "area {}", area(&merged));
    }

    #[test]
    fn union_keeps_disjoint_shapes_and_containment() {
        let a = square(0.0, 0.0, 10.0);
        let b = square(40.0, 0.0, 10.0);
        assert_eq!(union_polygons(&a, &b).len(), 2, "disjoint squares stay apart");
        let inner = square(2.0, 2.0, 3.0);
        let merged = union_polygons(&a, &inner);
        assert_eq!(merged.len(), 1);
        assert!((area(&merged) - 100.0).abs() < 0.01, "contained shape adds nothing");
    }

    #[test]
    fn a_wedge_tail_is_merged_into_the_balloon_outline() {
        let body = crate::shapes::ellipse(0.0, 0.0, 100.0, 80.0);
        let tail = BalloonTail::wedge([(50.0, 80.0), (62.0, 80.0)], (80.0, 150.0));
        let merged = attach_tail(&body, &tail);
        assert_eq!(merged.subpaths.len(), 1, "the tail and body share one contour");
        assert!(merged.subpaths[0].closed);
        // The union keeps the body and adds the tail's area.
        let polygons = closed_contours(&merged);
        assert!((area(&polygons) - std::f64::consts::PI * 50.0 * 40.0).abs() > 100.0, "the tail adds area");
        // Every knot is finite.
        for knot in &merged.subpaths[0].knots {
            assert!(knot.anchor.x.is_finite() && knot.anchor.y.is_finite());
        }
    }

    #[test]
    fn a_thought_tail_makes_separate_bubbles() {
        let body = crate::shapes::ellipse(0.0, 0.0, 100.0, 80.0);
        let tail = BalloonTail::thought([(50.0, 78.0), (56.0, 78.0)], (90.0, 170.0));
        let merged = attach_tail(&body, &tail);
        assert!(merged.subpaths.len() >= 2, "body plus bubbles: {}", merged.subpaths.len());
    }

    #[test]
    fn cloud_is_closed_and_bounded_by_its_rect() {
        let path = cloud([0.0, 0.0, 200.0, 120.0], CloudParams::default());
        assert_eq!(path.subpaths.len(), 1);
        assert!(path.subpaths[0].closed);
        assert!(path.subpaths[0].knots.len() >= 9 * 6, "one sample run per bump");
        // The bumps bulge out of the inscribed ellipse, by at most the jittered amplitude.
        let pad = 60.0 * 0.18 * 1.35 + 1.0;
        for knot in &path.subpaths[0].knots {
            assert!((-pad..=200.0 + pad).contains(&knot.anchor.x), "x {}", knot.anchor.x);
            assert!((-pad..=120.0 + pad).contains(&knot.anchor.y), "y {}", knot.anchor.y);
        }
    }

    #[test]
    fn spiky_alternates_tips_and_valleys_deterministically() {
        let params = SpikeParams { spikes: 10, ..SpikeParams::default() };
        let a = spiky([0.0, 0.0, 100.0, 100.0], params);
        let b = spiky([0.0, 0.0, 100.0, 100.0], params);
        assert_eq!(a, b, "the same params and seed draw the same balloon");
        assert_eq!(a.subpaths[0].knots.len(), 20, "one valley and one tip per spike");
        let c = spiky([0.0, 0.0, 100.0, 100.0], SpikeParams { seed: 7, ..params });
        assert_ne!(a, c, "the seed changes the jitter");
    }

    #[test]
    fn sai_stroke_smooths_nodes_and_keeps_corners_sharp() {
        let points = [
            SaiPoint::new((0.0, 0.0)),
            SaiPoint::new((50.0, 40.0)),
            SaiPoint { p: (100.0, 0.0), pressure: 1.0, corner: true },
        ];
        let path = sai_stroke(&points, false);
        assert_eq!(path.subpaths.len(), 1);
        assert_eq!(path.subpaths[0].knots.len(), 3);
        assert!(path.subpaths[0].knots[1].smooth, "middle node is smoothed");
        assert!(!path.subpaths[0].knots[2].smooth, "corner node stays sharp");
        assert!(!path.subpaths[0].closed);
    }

    #[test]
    fn tapered_outline_tapers_with_pressure() {
        let points = [
            SaiPoint { p: (0.0, 0.0), pressure: 0.0, corner: false },
            SaiPoint { p: (50.0, 0.0), pressure: 1.0, corner: false },
            SaiPoint { p: (100.0, 0.0), pressure: 0.0, corner: false },
        ];
        let outline = tapered_outline(&points, 10.0, 0.0);
        assert!(outline.len() > 6);
        assert!(outline.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
        // The full-pressure middle is thicker than the tapered ends.
        let max_y = outline.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        assert!(max_y > 4.0, "middle width reaches half the base width: {max_y}");
        let empty = tapered_outline(&points[..1], 10.0, 0.0);
        assert!(empty.is_empty(), "a single point has no stroke");
    }

    #[test]
    fn presets_round_trip_through_their_ids() {
        for preset in [TailPreset::Wedge, TailPreset::Arc, TailPreset::Double, TailPreset::Thought] {
            assert_eq!(TailPreset::from_id(preset.id()), Some(preset));
        }
        assert_eq!(TailPreset::from_id("nope"), None);
    }
}
