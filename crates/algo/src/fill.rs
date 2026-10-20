//! The Fill tool's areas, after Clip Studio Paint's Fill tool (its behaviour as its manual
//! describes it; no code): a click fill that closes gaps in line art and grows under the lines,
//! and the enclosed-area fills of Enclose and Fill and Paint Unfilled Area.
//!
//! Gaps close with a watershed on the distance to the lines. Pixels at least half the gap away
//! from every line are *open*, and the lines alone split them into areas. The pixels nearer a
//! line then join those areas, every area flooding outward from its open middle in order of
//! distance, so two areas that leak into each other through a gap meet at its narrowest point.
//! A pixel both reach at once is the seam that closes the gap: it belongs to neither area, and a
//! fill covers the seam pixels beside it as it covers a line, so two fills meet without a speck
//! between them.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use photocraft_geom::Rect;

use crate::selection::{Region, edt, region_from_marks};

/// The widest gap Close Gap closes, in pixels.
pub const MAX_GAP: f32 = 100.0;
/// The most Area Scaling grows or shrinks a fill, in pixels.
pub const MAX_SCALE: f32 = 100.0;
/// The largest Paint Unfilled Area brush radius, in pixels.
pub const MAX_RADIUS: f32 = 1000.0;

/// How Area Scaling grows or shrinks a fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scaling {
    /// "To darkest pixel": grows only into the lines, and only while they get darker (further
    /// from the filled colour), so the fill reaches under a line's core but not past it.
    #[default]
    Darkest,
    /// A circle at each edge pixel: round corners.
    Round,
    /// A square at each edge pixel: square corners.
    Square,
}

impl Scaling {
    /// `darkest`, `round` or `square` (also `rectangle`, Clip Studio's name).
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "darkest" => Some(Self::Darkest),
            "round" => Some(Self::Round),
            "square" | "rectangle" => Some(Self::Square),
            _ => None,
        }
    }
}

/// What Enclose and Fill and Paint Unfilled Area fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// Transparent areas (Clip Studio's default): colour flats on an empty layer.
    #[default]
    Transparent,
    /// White and transparent areas: line art drawn on white paper.
    WhiteAndTransparent,
}

impl Target {
    /// `transparent` or `whiteAndTransparent`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "transparent" => Some(Self::Transparent),
            "whiteAndTransparent" => Some(Self::WhiteAndTransparent),
            _ => None,
        }
    }
}

/// The Fill tool's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillOptions {
    /// Color margin: how far a pixel may be from the filled colour, 0..=100 % of the range.
    pub tolerance: f32,
    /// Follow adjacent pixels: only the area around the click (else every similar pixel).
    pub contiguous: bool,
    /// Close gaps in the lines up to this many pixels wide (0: off).
    pub close_gap: f32,
    /// Area Scaling in pixels: positive grows the fill, negative shrinks it.
    pub scale: f32,
    pub scaling: Scaling,
    pub anti_alias: bool,
    /// Refer to image border: the canvas edge closes gaps as a line does.
    pub image_border: bool,
    /// The colour a click must be like, when it goes on from an earlier one (a drag filling each
    /// area it reaches): a seed further from it than the colour margin fills nothing, and the
    /// fill follows this colour rather than the seed's.
    pub match_color: Option<[u8; 4]>,
}

impl Default for FillOptions {
    fn default() -> Self {
        Self { tolerance: 10.0, contiguous: true, close_gap: 0.0, scale: 0.0, scaling: Scaling::Darkest, anti_alias: true, image_border: false, match_color: None }
    }
}

/// The Fill tool's click at `seed` (document pixels) on `img`, the referenced pixels over `area`
/// (straight RGBA8, row-major): the pixels within the colour margin of the seed's, around it or
/// all over, with gaps closed and Area Scaling applied. `None` if the seed is outside `area` or
/// nothing would be filled.
pub fn click_region(img: &[[u8; 4]], area: Rect, seed: (i32, i32), o: &FillOptions) -> Option<Region> {
    if !area.contains(seed.0, seed.1) {
        return None;
    }
    let (w, h) = dims(img, area)?;
    let si = (seed.1 - area.y0) as usize * w + (seed.0 - area.x0) as usize;
    let margin = margin(o.tolerance);
    let target = premul(o.match_color.unwrap_or(*img.get(si)?));
    if difference(*img.get(si)?, target) > margin {
        return None;
    }
    let strength = strengths(img, |p| difference(p, target));
    let free: Vec<bool> = crate::par::map(strength.len(), strength.len() >= 65_536, |i| strength.get(i).is_some_and(|s| *s <= margin));
    let gap = finite(o.close_gap, MAX_GAP);
    if !o.contiguous {
        let mut fill = free.clone();
        scale(&mut fill, &strength, &free, w, h, o);
        return to_region(&fill, w, h, area, o.anti_alias);
    }
    // The seed's area before any gap is closed. Closing gaps only splits it, and Area Scaling
    // reaches a few pixels past it, so the rest works on its box and a margin: a click on a page
    // costs what the spot clicked costs, not what the page does.
    let mut spot = vec![false; free.len()];
    let (x0, y0, x1, y1) = flood(&free, &mut spot, w, h, si, true)?;
    let amount = if o.scale.is_finite() { o.scale.clamp(-MAX_SCALE, MAX_SCALE).abs() } else { 0.0 };
    let pad = (gap / 2.0).ceil() as usize + amount.ceil() as usize + 4;
    let (cx0, cy0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let (cx1, cy1) = ((x1 + pad + 1).min(w), (y1 + pad + 1).min(h));
    let (cw, ch) = (cx1 - cx0, cy1 - cy0);
    let crop = |v: &[bool]| -> Vec<bool> { (cy0..cy1).flat_map(|y| v.get(y * w + cx0..y * w + cx1).unwrap_or(&[]).iter().copied()).collect() };
    let strength: Vec<u8> = (cy0..cy1).flat_map(|y| strength.get(y * w + cx0..y * w + cx1).unwrap_or(&[]).iter().copied()).collect();
    let (spot, free) = (crop(&spot), crop(&free));
    let csi = (si / w - cy0) * cw + si % w - cx0;
    let carea = Rect::new(area.x0 + cx0 as i32, area.y0 + cy0 as i32, area.x0 + cx1 as i32, area.y0 + cy1 as i32);
    let mut fill = if gap < 1.0 {
        spot
    } else {
        // Inside the spot, the nearest pixel outside it is always a line pixel, so taking every
        // other pixel for a line changes no distance there: the areas come out as they do over
        // the whole page, and the narrow-spot climb can't step across a thin diagonal line into
        // the next spot.
        let (labels, seed_label) = areas(&spot, cw, ch, gap, o.image_border, Some(csi));
        let l = seed_label?;
        let mut fill: Vec<bool> = labels.iter().map(|v| *v == l).collect();
        cover_seams(&mut fill, &labels, &spot, cw, ch);
        fill
    };
    scale(&mut fill, &strength, &free, cw, ch, o);
    to_region(&fill, cw, ch, carea, o.anti_alias)
}


/// Enclose and Fill / Paint Unfilled Area: fills every area of `target` pixels of `img` (as in
/// [`click_region`]) that lies wholly inside `inside` (the lasso or the painted stroke, a mask
/// over `area`). An area that reaches outside it is left as it is. `contiguous` is not used.
pub fn enclosed_region(img: &[[u8; 4]], area: Rect, inside: &[bool], target: Target, o: &FillOptions) -> Option<Region> {
    let (w, h) = dims(img, area)?;
    if inside.len() != w * h {
        return None;
    }
    let strength = match target {
        Target::Transparent => strengths(img, |p| difference(p, [0; 4])),
        Target::WhiteAndTransparent => strengths(img, darkness),
    };
    let margin = margin(o.tolerance);
    let free: Vec<bool> = strength.iter().map(|s| *s <= margin).collect();
    let gap = finite(o.close_gap, MAX_GAP);
    let labels = if gap < 1.0 { components(&free, w, h) } else { areas(&free, w, h, gap, o.image_border, None).0 };
    let count = labels.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut leaves = vec![false; count];
    for (l, inside) in labels.iter().zip(inside) {
        if *l != 0
            && !inside
            && let Some(v) = leaves.get_mut(*l as usize)
        {
            *v = true;
        }
    }
    let mut fill: Vec<bool> = labels.iter().map(|l| *l != 0 && !leaves.get(*l as usize).copied().unwrap_or(true)).collect();
    if gap >= 1.0 {
        cover_seams(&mut fill, &labels, &free, w, h);
    }
    scale(&mut fill, &strength, &free, w, h, o);
    to_region(&fill, w, h, area, o.anti_alias)
}

/// The pixels of `area` inside the polygon `points` (even-odd, pixel centres): a lasso.
pub fn lasso_mask(points: &[(f32, f32)], area: Rect) -> Vec<bool> {
    crate::selection::polygon(points, area, false).iter().map(|c| *c >= 0.5).collect()
}

/// The pixels of `area` a round brush of `radius` covers along `points`: a painted stroke.
pub fn stroke_mask(points: &[(f32, f32)], radius: f32, area: Rect) -> Vec<bool> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut m = vec![false; w * h];
    let r = finite(radius, MAX_RADIUS).max(0.5);
    let mut stamp = |cx: f32, cy: f32| {
        let (x0, x1) = (((cx - r).floor() as i32).max(area.x0), ((cx + r).ceil() as i32).saturating_add(1).min(area.x1));
        let (y0, y1) = (((cy - r).floor() as i32).max(area.y0), ((cy + r).ceil() as i32).saturating_add(1).min(area.y1));
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r
                    && let Some(v) = m.get_mut((y - area.y0) as usize * w + (x - area.x0) as usize)
                {
                    *v = true;
                }
            }
        }
    };
    // Dabs every half radius along the path (however densely it was sampled), up to a bound.
    let step = (r / 2.0).max(0.5);
    let mut dabs = 0usize;
    let mut prev: Option<(f32, f32)> = None;
    let mut since = 0.0f32;
    for &(x, y) in points.iter().filter(|(x, y)| x.is_finite() && y.is_finite()) {
        let (x, y) = (x.clamp(-1e6, 1e6), y.clamp(-1e6, 1e6));
        let Some((px, py)) = prev else {
            stamp(x, y);
            prev = Some((x, y));
            continue;
        };
        let len = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
        let mut at = step - since;
        let mut last = None;
        while at <= len && dabs < MAX_DABS {
            let t = at / len;
            stamp(px + (x - px) * t, py + (y - py) * t);
            dabs += 1;
            last = Some(at);
            at += step;
        }
        since = last.map_or(since + len, |a| len - a);
        prev = Some((x, y));
    }
    if let Some((x, y)) = prev {
        stamp(x, y);
    }
    m
}

/// The most dabs [`stroke_mask`] paints.
const MAX_DABS: usize = 200_000;

/// `area`'s size when `img` covers it.
fn dims(img: &[[u8; 4]], area: Rect) -> Option<(usize, usize)> {
    let (w, h) = (usize::try_from(area.width()).ok()?, usize::try_from(area.height()).ok()?);
    (w > 0 && h > 0 && w.checked_mul(h)? == img.len()).then_some((w, h))
}

/// `v` clamped to `0..=max`; NaN and infinities count as 0.
fn finite(v: f32, max: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, max) } else { 0.0 }
}

/// The colour margin in 8-bit levels.
fn margin(tolerance: f32) -> u8 {
    (finite(tolerance, 100.0) * 2.55).round() as u8
}

fn premul(p: [u8; 4]) -> [i32; 4] {
    let a = i32::from(p[3]);
    let m = |c: u8| (i32::from(c) * a + 127) / 255;
    [m(p[0]), m(p[1]), m(p[2]), a]
}

/// How far `p` is from the premultiplied colour `t`, 0..=255: colours hidden under transparency
/// don't count, and a faint pixel is near transparent whatever its colour.
fn difference(p: [u8; 4], t: [i32; 4]) -> u8 {
    let q = premul(p);
    q.iter().zip(t).map(|(a, b)| (a - b).unsigned_abs()).max().unwrap_or(0).min(255) as u8
}

/// How dark `p` looks on white paper, 0..=255: how far it is from both white and transparent.
fn darkness(p: [u8; 4]) -> u8 {
    let q = premul(p);
    q[..3].iter().map(|c| q[3] - c).max().unwrap_or(0).clamp(0, 255) as u8
}

fn strengths(img: &[[u8; 4]], f: impl Fn([u8; 4]) -> u8 + Sync + Send) -> Vec<u8> {
    crate::par::map(img.len(), img.len() >= 65_536, |i| img.get(i).map_or(255, |p| f(*p)))
}

/// The 4-neighbours of pixel `i` of a `w` × `h` plane.
fn neighbours(i: usize, w: usize, h: usize) -> [Option<usize>; 4] {
    let (x, y) = (i % w, i / w);
    [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
}

/// Span flood of the 4-connected `open` pixels around `start` that have no label yet (the
/// default value), giving them `label`. The box it filled, inclusive, or `None` when `start`
/// wasn't open.
fn flood<L: Copy + Default + PartialEq>(open: &[bool], labels: &mut [L], w: usize, h: usize, start: usize, label: L) -> Option<(usize, usize, usize, usize)> {
    let ok = |labels: &[L], i: usize| open.get(i).copied().unwrap_or(false) && labels.get(i) == Some(&L::default());
    let mut stack = vec![(start % w, start / w)];
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    while let Some((x, y)) = stack.pop() {
        let row = y * w;
        if !ok(labels, row + x) {
            continue;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && ok(labels, row + l - 1) {
            l -= 1;
        }
        while r + 1 < w && ok(labels, row + r + 1) {
            r += 1;
        }
        if let Some(run) = labels.get_mut(row + l..=row + r) {
            run.fill(label);
        }
        bbox = Some(bbox.map_or((l, y, r, y), |(x0, y0, x1, y1)| (x0.min(l), y0.min(y), x1.max(r), y1.max(y))));
        for ny in [y.wrapping_sub(1), y + 1] {
            if ny >= h {
                continue;
            }
            let mut in_run = false;
            for nx in l..=r {
                let here = ok(labels, ny * w + nx);
                if here && !in_run {
                    stack.push((nx, ny));
                }
                in_run = here;
            }
        }
    }
    bbox
}

/// The 4-connected components of `open`, labelled from 1 (0 elsewhere).
fn components(open: &[bool], w: usize, h: usize) -> Vec<u32> {
    let mut labels = vec![0u32; open.len()];
    let mut next = 1u32;
    for i in 0..open.len() {
        if open.get(i) == Some(&true) && labels.get(i) == Some(&0) {
            flood(open, &mut labels, w, h, i, next);
            next = next.saturating_add(1).min(u32::MAX - 1);
        }
    }
    labels
}

/// From pixel `i`, up the distance map to a local top: away from the lines into the middle of
/// the spot `i` is in.
fn climb(dist: &[f32], free: &[bool], w: usize, h: usize, mut i: usize) -> usize {
    let d = |j: usize| dist.get(j).copied().unwrap_or(0.0);
    loop {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let mut best = i;
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if free.get(j) == Some(&true) && d(j) > d(best) {
                best = j;
            }
        }
        if best == i {
            return i;
        }
        i = best;
    }
}

/// The areas of `free` pixels, split by the lines (the other pixels) with gaps up to `gap`
/// pixels closed (see the module docs): labels from 1, 0 on lines and seams. With `seed`, also
/// the label of the seed's area. A seed in a spot narrower than the gap lowers the gap to fit,
/// so a click always fills the spot it lands in.
fn areas(free: &[bool], w: usize, h: usize, gap: f32, border: bool, seed: Option<usize>) -> (Vec<u32>, Option<u32>) {
    if border {
        return watershed(free, w, h, gap, seed);
    }
    // The edge is not a line: the canvas carries on blank beyond it, far enough for any gap
    // between a line and the edge to stay open.
    let pad = (gap / 2.0).ceil() as usize + 2;
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let mut padded = vec![true; pw * ph];
    for (y, row) in free.chunks_exact(w).enumerate() {
        let at = (y + pad) * pw + pad;
        if let Some(dst) = padded.get_mut(at..at + w) {
            dst.copy_from_slice(row);
        }
    }
    let seed = seed.map(|s| (s / w + pad) * pw + s % w + pad);
    let (labels, seed_label) = watershed(&padded, pw, ph, gap, seed);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let at = (y + pad) * pw + pad;
        out.extend_from_slice(labels.get(at..at + w).unwrap_or(&[]));
    }
    out.resize(w * h, 0);
    (out, seed_label)
}

/// [`areas`] with the canvas edge as a line.
fn watershed(free: &[bool], w: usize, h: usize, gap: f32, seed: Option<usize>) -> (Vec<u32>, Option<u32>) {
    let n = free.len();
    let lines: Vec<bool> = free.iter().map(|f| !f).collect();
    let dist = edt(&lines, w, h);
    if dist.len() != n {
        let labels = components(free, w, h);
        let l = seed.and_then(|s| labels.get(s).copied()).filter(|l| *l != 0);
        return (labels, l);
    }
    // A gap of `gap` pixels between two line ends has its middle (gap + 1) / 2 from them.
    let mut r = (gap + 2.0) / 2.0;
    let top = seed.filter(|s| free.get(*s) == Some(&true)).map(|s| climb(&dist, free, w, h, s));
    if let Some(t) = top {
        r = r.min(dist.get(t).copied().unwrap_or(r));
    }
    let open: Vec<bool> = free.iter().zip(&dist).map(|(f, d)| *f && *d >= r).collect();
    let mut labels = components(&open, w, h);
    drop(open);
    // Each area's deepest point and how big its deep part is. Where two areas meet, the
    // shallower one is kept apart only if it rises well above the meeting point and its deep
    // part is bigger than a few gaps: a real spot behind a gap. Along a stroke that tapers to a
    // point, jagged edges widen it past the gap now and then; those bits are small or barely rise,
    // and join the area they belong to, so the fill runs on to the tip.
    let count = labels.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut peak = vec![0f32; count];
    let mut size = vec![0usize; count];
    for (l, d) in labels.iter().zip(&dist) {
        if let (Some(p), Some(n)) = (peak.get_mut(*l as usize), size.get_mut(*l as usize)) {
            *p = p.max(*d);
            *n += 1;
        }
    }
    let small = (8.0 * r * r) as usize;
    let mut parent: Vec<u32> = (0..count as u32).collect();
    let rise = (r * 0.4).max(1.0);
    const SEAM: u32 = u32::MAX;
    let key = |d: f32| (d.clamp(0.0, 1e6) * 64.0) as u32;
    let labelled = |labels: &[u32], j: usize| labels.get(j).is_some_and(|l| *l != 0 && *l != SEAM);
    let mut queued = vec![false; n];
    let mut heap: BinaryHeap<(u32, Reverse<u64>, usize)> = BinaryHeap::new();
    let mut seq = 0u64;
    for i in 0..n {
        if free.get(i) == Some(&true) && labels.get(i) == Some(&0) && neighbours(i, w, h).into_iter().flatten().any(|j| labelled(&labels, j)) {
            if let Some(q) = queued.get_mut(i) {
                *q = true;
            }
            heap.push((key(dist.get(i).copied().unwrap_or(0.0)), Reverse(seq), i));
            seq += 1;
        }
    }
    // Farthest from the lines first; equally far, first come first served.
    while let Some((_, _, i)) = heap.pop() {
        let here = dist.get(i).copied().unwrap_or(0.0);
        let mut met: Vec<u32> = Vec::new();
        for j in neighbours(i, w, h).into_iter().flatten() {
            if labelled(&labels, j) {
                let root = find(&mut parent, labels.get(j).copied().unwrap_or(0));
                if !met.contains(&root) {
                    met.push(root);
                }
            }
        }
        if met.is_empty() {
            continue;
        }
        // The deepest area takes in the ones that barely rise above this meeting point.
        met.sort_by(|a, b| peak.get(*b as usize).copied().unwrap_or(0.0).total_cmp(&peak.get(*a as usize).copied().unwrap_or(0.0)));
        let got = met[0];
        let mut seam = false;
        for &other in &met[1..] {
            let (o, g) = (other as usize, got as usize);
            if peak.get(o).copied().unwrap_or(0.0) - here < rise || size.get(o).copied().unwrap_or(0) < small {
                if let Some(p) = parent.get_mut(o) {
                    *p = got;
                }
                let (po, so) = (peak.get(o).copied().unwrap_or(0.0), size.get(o).copied().unwrap_or(0));
                if let (Some(p), Some(n)) = (peak.get_mut(g), size.get_mut(g)) {
                    *p = p.max(po);
                    *n += so;
                }
            } else {
                seam = true;
            }
        }
        if let Some(v) = labels.get_mut(i) {
            *v = if seam { SEAM } else { got };
        }
        if seam {
            continue;
        }
        for j in neighbours(i, w, h).into_iter().flatten() {
            if free.get(j) == Some(&true) && labels.get(j) == Some(&0) && queued.get(j) == Some(&false) {
                if let Some(q) = queued.get_mut(j) {
                    *q = true;
                }
                heap.push((key(dist.get(j).copied().unwrap_or(0.0)), Reverse(seq), j));
                seq += 1;
            }
        }
    }
    for l in &mut labels {
        *l = if *l == SEAM { 0 } else { find(&mut parent, *l) };
    }
    let seed_label = top.and_then(|t| labels.get(t).copied()).filter(|l| *l != 0);
    (labels, seed_label)
}

/// The area label `l` was merged into (union-find, with path halving).
fn find(parent: &mut [u32], mut l: u32) -> u32 {
    while let Some(&p) = parent.get(l as usize) {
        if p == l {
            break;
        }
        let gp = parent.get(p as usize).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(l as usize) {
            *slot = gp;
        }
        l = gp;
    }
    l
}

/// Adds to `fill` the seams beside it: the pixels within the margin that no area got (see
/// [`areas`]), which close a gap the way a line would.
fn cover_seams(fill: &mut [bool], labels: &[u32], free: &[bool], w: usize, h: usize) {
    let seams: Vec<usize> = (0..fill.len())
        .filter(|&i| free.get(i) == Some(&true) && labels.get(i) == Some(&0) && neighbours(i, w, h).into_iter().flatten().any(|j| fill.get(j) == Some(&true)))
        .collect();
    for i in seams {
        if let Some(f) = fill.get_mut(i) {
            *f = true;
        }
    }
}

/// Area Scaling of `fill` by `o.scale` pixels.
fn scale(fill: &mut [bool], strength: &[u8], free: &[bool], w: usize, h: usize, o: &FillOptions) {
    let amount = if o.scale.is_finite() { o.scale.clamp(-MAX_SCALE, MAX_SCALE) } else { 0.0 };
    if amount.abs() < 0.5 || !fill.iter().any(|f| *f) {
        return;
    }
    if amount < 0.0 {
        let outside: Vec<bool> = fill.iter().map(|f| !f).collect();
        let d = if o.scaling == Scaling::Square { chessboard(&outside, w, h) } else { edt(&outside, w, h) };
        for (f, d) in fill.iter_mut().zip(&d) {
            if *d <= -amount {
                *f = false;
            }
        }
        return;
    }
    match o.scaling {
        Scaling::Round | Scaling::Square => {
            let d = if o.scaling == Scaling::Square { chessboard(fill, w, h) } else { edt(fill, w, h) };
            for (f, d) in fill.iter_mut().zip(&d) {
                if *d <= amount {
                    *f = true;
                }
            }
        }
        Scaling::Darkest => grow_to_darkest(fill, strength, free, w, h, amount),
    }
}

/// "To darkest pixel": grows `fill` up to `amount` pixels into the lines (pixels not `free`),
/// stepping only to pixels at least as strong (as far from the filled colour) as the last, and
/// never on from a pixel at full strength: the fill tucks under a line up to its darkest pixels.
fn grow_to_darkest(fill: &mut [bool], strength: &[u8], free: &[bool], w: usize, h: usize, amount: f32) {
    let n = fill.len();
    let mut cost = vec![f32::INFINITY; n];
    let key = |c: f32| Reverse((c * 1024.0) as u32);
    let mut heap: BinaryHeap<(Reverse<u32>, usize)> = BinaryHeap::new();
    // Only filled pixels beside a line pixel can step anywhere: the rest start the queue for
    // nothing (a whole page of them took seconds).
    let at_line = |i: usize| {
        let (x, y) = (i % w, i / w);
        (y.saturating_sub(1)..(y + 2).min(h)).any(|ny| (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| free.get(ny * w + nx) == Some(&false) && fill.get(ny * w + nx) == Some(&false)))
    };
    for (i, f) in fill.iter().enumerate() {
        if *f && let Some(c) = cost.get_mut(i) {
            *c = 0.0;
            if at_line(i) {
                heap.push((key(0.0), i));
            }
        }
    }
    let s = |j: usize| strength.get(j).copied().unwrap_or(0);
    while let Some((_, i)) = heap.pop() {
        let here = cost.get(i).copied().unwrap_or(f32::INFINITY);
        let from_line = !fill.get(i).copied().unwrap_or(false);
        if from_line && s(i) == u8::MAX {
            continue;
        }
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if free.get(j) != Some(&false) || fill.get(j) == Some(&true) || (from_line && s(j) < s(i)) {
                continue;
            }
            let step = if dx != 0 && dy != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            let c = here + step;
            if c <= amount + 1e-3
                && let Some(old) = cost.get_mut(j)
                && c < *old
            {
                *old = c;
                heap.push((key(c), j));
            }
        }
    }
    for (f, c) in fill.iter_mut().zip(&cost) {
        if c.is_finite() {
            *f = true;
        }
    }
}

/// Chessboard distance to the nearest `true` pixel (two chamfer passes; 1e10 with none).
fn chessboard(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut d: Vec<f32> = inside.iter().map(|v| if *v { 0.0 } else { 1e10 }).collect();
    let at = |d: &[f32], x: i64, y: i64| {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 { 1e10 } else { d.get(y as usize * w + x as usize).copied().unwrap_or(1e10) }
    };
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let v = [(-1, 0), (-1, -1), (0, -1), (1, -1)].iter().map(|(dx, dy)| at(&d, x + dx, y + dy) + 1.0).fold(at(&d, x, y), f32::min);
            if let Some(c) = d.get_mut(y as usize * w + x as usize) {
                *c = v;
            }
        }
    }
    for y in (0..h as i64).rev() {
        for x in (0..w as i64).rev() {
            let v = [(1, 0), (1, 1), (0, 1), (-1, 1)].iter().map(|(dx, dy)| at(&d, x + dx, y + dy) + 1.0).fold(at(&d, x, y), f32::min);
            if let Some(c) = d.get_mut(y as usize * w + x as usize) {
                *c = v;
            }
        }
    }
    d
}

fn to_region(fill: &[bool], w: usize, h: usize, area: Rect, anti_alias: bool) -> Option<Region> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (y, row) in fill.chunks_exact(w).enumerate().take(h) {
        if let (Some(a), Some(b)) = (row.iter().position(|f| *f), row.iter().rposition(|f| *f)) {
            (x0, x1, y0, y1) = (x0.min(a), x1.max(b), y0.min(y), y1.max(y));
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    let marks: Vec<u8> = fill.iter().map(|f| if *f { 255 } else { 0 }).collect();
    Some(region_from_marks(&marks, w, h, area, (x0, y0, x1, y1), anti_alias))
}

#[cfg(test)]
mod tests;
