//! Pixel helpers for timelapse frames: resizing, letterboxing, the white flash, frame comparison
//! and the work-time badge. Every buffer is tightly packed 8-bit RGB (3 bytes per pixel, rows top
//! to bottom), as the frames are stored.

/// Filter taps for resizing `src` → `dst` samples: a triangle filter whose radius grows when
/// shrinking, so downscales average every source pixel (no shimmering) and upscales are bilinear.
fn taps(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = dst as f64 / src as f64;
    let radius = if scale < 1.0 { 1.0 / scale } else { 1.0 };
    (0..dst)
        .map(|i| {
            let center = (i as f64 + 0.5) / scale;
            let lo = (center - radius).floor().max(0.0) as usize;
            let hi = ((center + radius).ceil() as usize).min(src - 1);
            let mut start = None;
            let mut w = Vec::new();
            for j in lo..=hi {
                let d = (j as f64 + 0.5 - center).abs() / radius;
                if d >= 1.0 {
                    if start.is_some() {
                        break;
                    }
                    continue;
                }
                start.get_or_insert(j);
                w.push((1.0 - d) as f32);
            }
            let start = start.unwrap_or_else(|| {
                w = vec![1.0];
                (center.floor().max(0.0) as usize).min(src - 1)
            });
            let sum: f32 = w.iter().sum();
            if sum > 0.0 {
                for v in &mut w {
                    *v /= sum;
                }
            }
            (start, w)
        })
        .collect()
}

/// Resize RGB `src` (`sw`×`sh`) to `dw`×`dh`. A buffer of the wrong length, or a zero size, gives
/// a black frame rather than a panic.
pub fn resize(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    if sw == dw && sh == dh && src.len() == sw * sh * 3 {
        return src.to_vec();
    }
    let mut out = vec![0u8; dw * dh * 3];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * 3 {
        return out;
    }
    let (tx, ty) = (taps(sw, dw), taps(sh, dh));
    // Horizontal pass into float rows `dw` wide.
    let mut tmp = vec![0f32; dw * sh * 3];
    for (row, trow) in src.chunks_exact(sw * 3).zip(tmp.chunks_exact_mut(dw * 3)) {
        for ((start, w), o) in tx.iter().zip(trow.as_chunks_mut::<3>().0) {
            let (mut r, mut g, mut b) = (0f32, 0f32, 0f32);
            for (k, wt) in w.iter().enumerate() {
                if let Some(p) = row.get((start + k) * 3..(start + k) * 3 + 3) {
                    r += f32::from(p[0]) * wt;
                    g += f32::from(p[1]) * wt;
                    b += f32::from(p[2]) * wt;
                }
            }
            *o = [r, g, b];
        }
    }
    // Vertical pass.
    let stride = dw * 3;
    for ((start, w), orow) in ty.iter().zip(out.chunks_exact_mut(stride)) {
        for (x, o) in orow.iter_mut().enumerate() {
            let mut acc = 0f32;
            for (k, wt) in w.iter().enumerate() {
                acc += tmp.get((start + k) * stride + x).copied().unwrap_or(0.0) * wt;
            }
            *o = (acc + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Fit `src` inside a `dw`×`dh` frame keeping its shape, centred on `bg`. Used when the canvas was
/// cropped or resized during the recording.
pub fn letterbox(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize, bg: [u8; 3]) -> Vec<u8> {
    if sw == dw && sh == dh && src.len() == sw * sh * 3 {
        return src.to_vec();
    }
    let mut out: Vec<u8> = std::iter::repeat_n(bg, dw * dh).flatten().collect();
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return out;
    }
    let s = (dw as f64 / sw as f64).min(dh as f64 / sh as f64);
    let tw = ((sw as f64 * s).round() as usize).clamp(1, dw);
    let th = ((sh as f64 * s).round() as usize).clamp(1, dh);
    let scaled = resize(src, sw, sh, tw, th);
    let (ox, oy) = ((dw - tw) / 2, (dh - th) / 2);
    for (y, row) in scaled.chunks_exact(tw * 3).enumerate() {
        let at = ((oy + y) * dw + ox) * 3;
        if let Some(dst) = out.get_mut(at..at + tw * 3) {
            dst.copy_from_slice(row);
        }
    }
    out
}

/// Mix the frame toward white (0 = unchanged, 1 = all white).
pub fn whiten(rgb: &mut [u8], amount: f32) {
    if amount <= 0.0 || !amount.is_finite() {
        return;
    }
    let k = amount.min(1.0);
    for v in rgb {
        *v = (f32::from(*v) + (255.0 - f32::from(*v)) * k + 0.5).min(255.0) as u8;
    }
}

/// Pixels where any channel differs by more than `tol`; counting stops past `limit`.
pub fn count_changed(a: &[u8], b: &[u8], tol: u8, limit: usize) -> usize {
    let mut n = 0;
    for (p, q) in a.as_chunks::<3>().0.iter().zip(b.as_chunks::<3>().0) {
        if p.iter().zip(q).any(|(x, y)| x.abs_diff(*y) > tol) {
            n += 1;
            if n > limit {
                break;
            }
        }
    }
    n
}

/// "2h 05m" / "45m".
pub fn format_work_label(ms: u64) -> String {
    let total_min = ms / 60_000;
    let (h, m) = (total_min / 60, total_min % 60);
    if h > 0 { format!("{h}h {m:02}m") } else { format!("{m}m") }
}

// ---- work-time badge ---------------------------------------------------------------------------
// A small single-stroke vector font (digits, "h", "m", ":") plus a clock icon, rasterised with
// anti-aliasing, so the overlay needs no font files. Glyphs live in a box 1 unit tall (y down).

type Line = Vec<(f64, f64)>;

fn arc(cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, steps: usize) -> Line {
    (0..=steps)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f64 / steps as f64).to_radians();
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

fn join(mut a: Line, b: &[(f64, f64)]) -> Line {
    a.extend_from_slice(b);
    a
}

/// Every digit gets the same width so the counter doesn't jiggle.
const DIGIT_ADV: f64 = 0.66;

struct Glyph {
    adv: f64,
    lines: Vec<Line>,
    shift: f64,
}

fn glyph(adv: f64, lines: Vec<Line>) -> Glyph {
    let xs = lines.iter().flatten().map(|p| p.0);
    let (min, max) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
    // Centre the ink in its advance.
    let shift = if lines.is_empty() { 0.0 } else { (adv - (max - min)) / 2.0 - min };
    Glyph { adv, lines, shift }
}

fn glyph_for(ch: char) -> Glyph {
    let d = DIGIT_ADV;
    match ch {
        '0' => glyph(d, vec![join(join(arc(0.28, 0.28, 0.26, 0.26, 180.0, 360.0, 12), &arc(0.28, 0.72, 0.26, 0.26, 0.0, 180.0, 12)), &[(0.02, 0.28)])]),
        '1' => glyph(d, vec![vec![(0.12, 0.2), (0.33, 0.02), (0.33, 0.98)]]),
        '2' => glyph(d, vec![join(arc(0.28, 0.28, 0.25, 0.26, 200.0, 375.0, 14), &[(0.03, 0.98), (0.55, 0.98)])]),
        '3' => glyph(d, vec![arc(0.27, 0.26, 0.24, 0.24, 200.0, 450.0, 16), arc(0.27, 0.735, 0.27, 0.245, 270.0, 520.0, 16)]),
        '4' => glyph(d, vec![vec![(0.42, 0.98), (0.42, 0.02), (0.02, 0.68), (0.56, 0.68)]]),
        '5' => glyph(d, vec![join(vec![(0.52, 0.02), (0.1, 0.02), (0.075, 0.5)], &arc(0.29, 0.7, 0.26, 0.28, 215.0, 520.0, 18))]),
        '6' => glyph(d, vec![vec![(0.47, 0.02), (0.09, 0.56)], arc(0.29, 0.72, 0.26, 0.26, 0.0, 360.0, 24)]),
        '7' => glyph(d, vec![vec![(0.03, 0.02), (0.55, 0.02), (0.2, 0.98)]]),
        '8' => glyph(d, vec![arc(0.28, 0.265, 0.22, 0.235, 0.0, 360.0, 20), arc(0.28, 0.73, 0.26, 0.25, 0.0, 360.0, 22)]),
        '9' => glyph(d, vec![arc(0.27, 0.28, 0.26, 0.26, 0.0, 360.0, 24), vec![(0.47, 0.44), (0.09, 0.98)]]),
        ':' => glyph(0.3, vec![vec![(0.1, 0.34)], vec![(0.1, 0.8)]]),
        'h' => glyph(0.62, vec![vec![(0.05, 0.0), (0.05, 0.98)], join(arc(0.27, 0.62, 0.22, 0.22, 180.0, 360.0, 12), &[(0.49, 0.98)])]),
        'm' => glyph(
            0.8,
            vec![
                vec![(0.05, 0.4), (0.05, 0.98)],
                join(arc(0.2, 0.58, 0.15, 0.17, 180.0, 360.0, 10), &[(0.35, 0.98)]),
                join(arc(0.5, 0.58, 0.15, 0.17, 180.0, 360.0, 10), &[(0.65, 0.98)]),
            ],
        ),
        _ => glyph(0.32, Vec::new()),
    }
}

fn clock() -> Vec<Line> {
    vec![arc(0.5, 0.5, 0.44, 0.44, 0.0, 360.0, 32), vec![(0.5, 0.22), (0.5, 0.5), (0.7, 0.62)]]
}

type Segment = [f64; 4];

fn push_segments(out: &mut Vec<Segment>, lines: &[Line], ox: f64, oy: f64, scale: f64) {
    for line in lines {
        let pts: Vec<(f64, f64)> = if line.len() == 1 { vec![line[0], line[0]] } else { line.clone() };
        for w in pts.windows(2) {
            out.push([ox + w[0].0 * scale, oy + w[0].1 * scale, ox + w[1].0 * scale, oy + w[1].1 * scale]);
        }
    }
}

/// Paint round-capped strokes into a coverage map (0..1), keeping the max so overlaps don't add up.
fn stroke_segments(cov: &mut [f32], w: usize, h: usize, segs: &[Segment], half: f64) {
    for &[x1, y1, x2, y2] in segs {
        let min_x = (x1.min(x2) - half - 1.0).floor().max(0.0) as usize;
        let max_x = ((x1.max(x2) + half + 1.0).ceil().max(0.0) as usize).min(w.saturating_sub(1));
        let min_y = (y1.min(y2) - half - 1.0).floor().max(0.0) as usize;
        let max_y = ((y1.max(y2) + half + 1.0).ceil().max(0.0) as usize).min(h.saturating_sub(1));
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len2 = dx * dx + dy * dy;
        for y in min_y..=max_y {
            let py = y as f64 + 0.5;
            for x in min_x..=max_x {
                let px = x as f64 + 0.5;
                let t = if len2 > 0.0 { (((px - x1) * dx + (py - y1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (ex, ey) = (px - (x1 + t * dx), py - (y1 + t * dy));
                let c = half + 0.5 - (ex * ex + ey * ey).sqrt();
                if c > 0.0
                    && let Some(v) = cov.get_mut(y * w + x)
                {
                    *v = v.max(c.min(1.0) as f32);
                }
            }
        }
    }
}

/// Opacity of the badge's dark pill.
const BADGE_BG_ALPHA: f32 = 0.55;

/// A badge sprite: pill background plus clock icon and label, sized for a frame `frame_h` tall.
pub struct Badge {
    pub w: usize,
    pub h: usize,
    fg: Vec<f32>,
    bg: Vec<f32>,
}

/// Build the badge for `label` ("2h 05m").
pub fn make_badge(label: &str, frame_h: usize) -> Badge {
    let cap = ((frame_h as f64 * 0.03).round()).clamp(12.0, 110.0);
    let half = cap * 0.07;
    let icon = cap * 1.1;
    let gap = cap * 0.42;
    let glyphs: Vec<Glyph> = label.chars().map(glyph_for).collect();
    let text_w: f64 = glyphs.iter().map(|g| g.adv).sum();
    let (pad_x, pad_y) = ((cap * 0.6).round(), (cap * 0.5).round());
    let w = (icon + gap + text_w * cap + 2.0 * pad_x).ceil() as usize;
    let h = (cap + 2.0 * pad_y).ceil() as usize;

    let mut icon_segs = Vec::new();
    push_segments(&mut icon_segs, &clock(), pad_x, pad_y + (cap - icon) / 2.0, icon);
    let mut text_segs = Vec::new();
    let mut x = pad_x + icon + gap;
    for g in &glyphs {
        push_segments(&mut text_segs, &g.lines, x + g.shift * cap, pad_y, cap);
        x += g.adv * cap;
    }
    let mut fg = vec![0f32; w * h];
    stroke_segments(&mut fg, w, h, &icon_segs, half * 0.85);
    stroke_segments(&mut fg, w, h, &text_segs, half);

    // Capsule-shaped background with a soft 1 px edge.
    let mut bg = vec![0f32; w * h];
    let r = h as f64 / 2.0;
    let (ax, bx) = (r, w as f64 - r);
    for (y, row) in bg.chunks_exact_mut(w.max(1)).enumerate() {
        let dy = y as f64 + 0.5 - r;
        for (xx, v) in row.iter_mut().enumerate() {
            let px = xx as f64 + 0.5;
            let cx = px.clamp(ax.min(bx), bx.max(ax));
            let d = ((px - cx) * (px - cx) + dy * dy).sqrt() - r;
            *v = (0.5 - d).clamp(0.0, 1.0) as f32;
        }
    }
    Badge { w, h, fg, bg }
}

/// Where the badge sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "tl" => Self::TopLeft,
            "tr" => Self::TopRight,
            "bl" => Self::BottomLeft,
            "br" => Self::BottomRight,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::TopLeft => "tl",
            Self::TopRight => "tr",
            Self::BottomLeft => "bl",
            Self::BottomRight => "br",
        }
    }
}

/// Draw `badge` into the `fw`×`fh` RGB frame at `corner`.
pub fn draw_badge(rgb: &mut [u8], fw: usize, fh: usize, badge: &Badge, corner: Corner) {
    let m = (fw.min(fh) as f64 * 0.035).round() as i64;
    let left = matches!(corner, Corner::TopLeft | Corner::BottomLeft);
    let top = matches!(corner, Corner::TopLeft | Corner::TopRight);
    let x0 = if left { m } else { fw as i64 - m - badge.w as i64 };
    let y0 = if top { m } else { fh as i64 - m - badge.h as i64 };
    for y in 0..badge.h {
        let fy = y0 + y as i64;
        if fy < 0 || fy >= fh as i64 {
            continue;
        }
        for x in 0..badge.w {
            let fx = x0 + x as i64;
            if fx < 0 || fx >= fw as i64 {
                continue;
            }
            let i = y * badge.w + x;
            let a = badge.bg.get(i).copied().unwrap_or(0.0) * BADGE_BG_ALPHA;
            let t = badge.fg.get(i).copied().unwrap_or(0.0);
            if a == 0.0 && t == 0.0 {
                continue;
            }
            let p = (fy as usize * fw + fx as usize) * 3;
            if let Some(px) = rgb.get_mut(p..p + 3) {
                for c in px {
                    let v = f32::from(*c) * (1.0 - a);
                    *c = (v * (1.0 - t) + 255.0 * t + 0.5).min(255.0) as u8;
                }
            }
        }
    }
}
