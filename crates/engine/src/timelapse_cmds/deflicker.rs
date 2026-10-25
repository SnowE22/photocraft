//! Remove flicker: finds frames that flash in and out (a layer hidden for a moment, a stroke undone
//! right away, a colour tried for a second). A frame, or a burst of 2–3, is left out when it
//! differs from the frames on both sides while those two match each other (TimeSplice's method at
//! sensitivity 200, as the Photoshop Timelapse Recorder does it).
//!
//! It works on 32×32 greyscale thumbnails, cached in `thumbs32.bin` in the recording folder
//! (records of `[frame number: u32 LE][1024 grey bytes]`, the Photoshop plug-in's layout), so each
//! frame is only decoded once.

use std::collections::{BTreeSet, HashMap};

/// Pixels in one thumbnail.
pub const THUMB: usize = 32 * 32;
/// The cache file in a recording folder.
pub const THUMBS_FILE: &str = "thumbs32.bin";
const RECORD: usize = 4 + THUMB;
pub const SENSITIVITY: f64 = 200.0;

/// Average per-pixel difference of two thumbnails.
fn dist(a: &[u8], b: &[u8]) -> f64 {
    let sum: u64 = a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
    sum as f64 / THUMB as f64
}

/// Sorted indexes of the frames to leave out (one thumbnail per frame in video order, `None` when
/// unknown). The first and last frames are never among them; when more than 90 % would go,
/// something is off and nothing is left out.
pub fn find_flicker(thumbs: &[Option<&[u8]>], sensitivity: f64) -> Vec<usize> {
    let n = thumbs.len();
    if n < 3 {
        return Vec::new();
    }
    // TimeSplice's thresholds: T is an absolute floor, R a ratio.
    let t = (30.0 - sensitivity * 0.295).max(0.0);
    let r = (4.0 - sensitivity * 0.029).max(0.7);
    let th = |i: usize| thumbs.get(i).copied().flatten();
    let known = |from: usize, to: usize| (from..=to).all(|i| th(i).is_some());
    let d = |i: usize, j: usize| match (th(i), th(j)) {
        (Some(a), Some(b)) => dist(a, b),
        _ => 0.0,
    };
    let mut out = BTreeSet::new();
    // 1-frame flickers.
    for i in 1..n - 1 {
        if !known(i - 1, i + 1) {
            continue;
        }
        let (prev, next, ctx) = (d(i - 1, i), d(i, i + 1), d(i - 1, i + 1));
        if prev > t && next > t && prev > r * ctx && next > r * ctx {
            out.insert(i);
        }
    }
    // 2-frame bursts: i and i+1 match each other but not i-1 / i+2, which match.
    for i in 1..n.saturating_sub(2) {
        if out.contains(&i) || out.contains(&(i + 1)) || !known(i - 1, i + 2) {
            continue;
        }
        let (din, dout, rest, burst) = (d(i - 1, i), d(i + 1, i + 2), d(i - 1, i + 2), d(i, i + 1));
        if din > t && dout > t && din > r * rest && dout > r * rest && burst < din * 0.5 {
            out.extend([i, i + 1]);
        }
    }
    // 3-frame bursts.
    for i in 1..n.saturating_sub(3) {
        if (i..i + 3).any(|k| out.contains(&k)) || !known(i - 1, i + 3) {
            continue;
        }
        let (din, dout, rest) = (d(i - 1, i), d(i + 2, i + 3), d(i - 1, i + 3));
        let (b1, b2) = (d(i, i + 1), d(i + 1, i + 2));
        if din > t && dout > t && din > r * rest && dout > r * rest && b1 < din * 0.5 && b2 < din * 0.5 {
            out.extend([i, i + 1, i + 2]);
        }
    }
    if out.len() * 10 > n * 9 {
        return Vec::new();
    }
    out.into_iter().collect()
}

/// `thumbs32.bin` bytes → frame number → thumbnail. A torn last record is ignored.
pub fn parse_thumbs(bytes: &[u8]) -> HashMap<u32, Vec<u8>> {
    bytes
        .as_chunks::<RECORD>()
        .0
        .iter()
        .filter_map(|rec| {
            let (n, px) = rec.split_at(4);
            let n = u32::from_le_bytes(n.try_into().ok()?);
            Some((n, px.to_vec()))
        })
        .collect()
}

/// Frame number → thumbnail as `thumbs32.bin` bytes (sorted by frame number).
pub fn serialize_thumbs(map: &HashMap<u32, Vec<u8>>) -> Vec<u8> {
    let mut keys: Vec<u32> = map.keys().copied().collect();
    keys.sort_unstable();
    let mut out = Vec::with_capacity(keys.len() * RECORD);
    for n in keys {
        let Some(t) = map.get(&n).filter(|t| t.len() == THUMB) else { continue };
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(t);
    }
    out
}

/// A 32×32 greyscale thumbnail of tightly packed RGB pixels, each cell the average of the source
/// pixels it covers (Rec. 601 luma, as ffmpeg's `format=gray`).
pub fn thumbnail(rgb: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    if w == 0 || h == 0 || rgb.len() < w.checked_mul(h)?.checked_mul(3)? {
        return None;
    }
    let mut sum = vec![0u64; THUMB];
    let mut count = vec![0u64; THUMB];
    for (y, row) in rgb.chunks_exact(w * 3).take(h).enumerate() {
        let ty = y * 32 / h;
        for (x, p) in row.as_chunks::<3>().0.iter().enumerate() {
            let cell = ty * 32 + x * 32 / w;
            let luma = (299 * u64::from(p[0]) + 587 * u64::from(p[1]) + 114 * u64::from(p[2])) / 1000;
            if let (Some(s), Some(c)) = (sum.get_mut(cell), count.get_mut(cell)) {
                *s += luma;
                *c += 1;
            }
        }
    }
    // A source smaller than 32 px leaves cells empty: they take their nearest filled neighbour's
    // value along the row so the distance stays meaningful.
    let mut out: Vec<u8> = sum.iter().zip(&count).map(|(s, c)| if *c > 0 { (s / c) as u8 } else { 0 }).collect();
    for cell in 1..THUMB {
        if count.get(cell) == Some(&0)
            && let Some(prev) = out.get(cell - 1).copied()
            && let Some(o) = out.get_mut(cell)
        {
            *o = prev;
        }
    }
    Some(out)
}
