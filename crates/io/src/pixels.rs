//! Conversions between PSD planar big-endian channel data and interleaved
//! native-endian surface bytes.

use photocraft_color::SampleType;

/// PSD depth (bits) for a sample type.
pub fn psd_depth(s: SampleType) -> u16 {
    match s {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        SampleType::F32 => 32,
    }
}

/// Sample type for a PSD depth (1-bit is expanded to 8).
pub fn sample_for_depth(depth: u16) -> SampleType {
    match depth {
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => SampleType::U8,
    }
}

fn invert_sample(bytes: &mut [u8], s: SampleType) {
    match s {
        SampleType::U8 => bytes[0] = 255 - bytes[0],
        SampleType::U16 => {
            let v = u16::from_ne_bytes([bytes[0], bytes[1]]);
            bytes.copy_from_slice(&(65535 - v).to_ne_bytes());
        }
        SampleType::F32 => {
            let v = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            bytes.copy_from_slice(&(1.0 - v).to_ne_bytes());
        }
    }
}

/// Interleaves planes (big-endian samples, `n` pixels each). `None` planes
/// are filled with `fill[c]` (a native-endian encoded sample). Channels whose
/// `invert[c]` is set are inverted (PSD stores CMYK as `max - ink`).
pub fn interleave(planes: &[Option<&[u8]>], fill: &[Vec<u8>], n: usize, s: SampleType, invert: &[bool]) -> Vec<u8> {
    let bps = s.bytes();
    let ch = planes.len();
    let row = ch * bps;
    let mut out = vec![0u8; n * row];
    if out.is_empty() {
        return out;
    }
    let bs = bands(n);
    let step = bs.first().map_or(n, |b| b.len()) * row;
    par_map(bs.into_iter().zip(out.chunks_mut(step)).collect(), |(band, part)| {
        for (c, plane) in planes.iter().enumerate() {
            let src = plane.and_then(|p| p.get(band.start * bps..)).unwrap_or_default();
            let inv = src.len() >= bps && invert[c];
            let fill = fill.get(c).map(Vec::as_slice).unwrap_or_default();
            match s {
                SampleType::U8 => write_channel(part, ch, c, src, fill, |[v]| [if inv { 255 - v } else { v }]),
                SampleType::U16 => write_channel(part, ch, c, src, fill, |v| {
                    let v = u16::from_be_bytes(v);
                    (if inv { 65535 - v } else { v }).to_ne_bytes()
                }),
                SampleType::F32 => write_channel(part, ch, c, src, fill, |v| {
                    // Preserve float bits when no inversion is requested.
                    if inv { (1.0 - f32::from_be_bytes(v)).to_ne_bytes() } else { u32::from_be_bytes(v).to_ne_bytes() }
                }),
            }
        }
    });
    out
}

/// [`interleave`] straight into the tiles of `s` over `r` (the planes hold `r`'s pixels row by
/// row), a band of tiles per core, leaving out tiles that hold only the default pixel as
/// [`photocraft_raster::Surface::prune`] would: no interleaved copy of the whole layer on the way
/// (with the copy into tiles, that was most of the time a PSD with many big layers took to open).
/// Tiles of one single pixel value share one copy (copy-on-write, as a solid fill's do): the
/// stored rendering of a Color Fill layer is mostly such tiles, ~28 MB a layer on a 7 MP canvas.
/// Without `parallel`, one thread does it all (when many layers are decoded at once).
pub fn write_planes(s: &mut photocraft_raster::Surface, r: photocraft_geom::Rect, planes: &[Option<&[u8]>], fill: &[Vec<u8>], invert: &[bool], parallel: bool) {
    use photocraft_geom::TILE_SIZE;
    use std::sync::Arc;
    let st = s.format().sample;
    let bps = st.bytes();
    let bpp = planes.len() * bps;
    if r.is_empty() || bpp != s.format().bytes_per_pixel() {
        return;
    }
    let w = r.width() as usize;
    let coords: Vec<photocraft_geom::TileCoord> = r.tiles().collect();
    let dp = s.default_bytes().to_vec();
    let base = &*s;
    let parts = if parallel { bands(coords.len()) } else { std::iter::once(0..coords.len()).collect() };
    let made: Vec<Vec<(photocraft_geom::TileCoord, Arc<photocraft_raster::Tile>, bool)>> = par_map(parts, |band| {
        let mut out = Vec::new();
        for &tc in coords.get(band).unwrap_or_default() {
            let Ok(mut t) = base.try_default_tile() else { continue };
            let tr = tc.rect().intersect(&r);
            let data = t.bytes_mut();
            let span = tr.width() as usize;
            for y in tr.y0..tr.y1 {
                let at = ((y - tc.ty * TILE_SIZE) as usize * TILE_SIZE as usize + (tr.x0 - tc.tx * TILE_SIZE) as usize) * bpp;
                let Some(dst) = data.get_mut(at..at + span * bpp) else { continue };
                let p0 = (y - r.y0) as usize * w + (tr.x0 - r.x0) as usize;
                for (c, plane) in planes.iter().enumerate() {
                    let inv = invert.get(c).copied().unwrap_or(false);
                    match plane.and_then(|pl| pl.get(p0 * bps..(p0 + span) * bps)) {
                        Some(src) if bps == 1 => {
                            for (d, v) in dst.chunks_exact_mut(bpp).zip(src) {
                                d[c] = if inv { 255 - v } else { *v };
                            }
                        }
                        Some(src) => {
                            for (d, v) in dst.chunks_exact_mut(bpp).zip(src.chunks_exact(bps)) {
                                let o = &mut d[c * bps..(c + 1) * bps];
                                if cfg!(target_endian = "big") {
                                    o.copy_from_slice(v);
                                } else {
                                    for (k, b) in o.iter_mut().enumerate() {
                                        *b = v[bps - 1 - k];
                                    }
                                }
                                if inv {
                                    invert_sample(o, st);
                                }
                            }
                        }
                        // A missing plane, or one too short for this row: what it has, then fill.
                        None => {
                            for (i, d) in dst.chunks_exact_mut(bpp).enumerate() {
                                let o = &mut d[c * bps..(c + 1) * bps];
                                match plane.and_then(|pl| pl.get((p0 + i) * bps..(p0 + i + 1) * bps)) {
                                    Some(v) => {
                                        if cfg!(target_endian = "big") {
                                            o.copy_from_slice(v);
                                        } else {
                                            for (k, b) in o.iter_mut().enumerate() {
                                                *b = v[bps - 1 - k];
                                            }
                                        }
                                        if inv {
                                            invert_sample(o, st);
                                        }
                                    }
                                    None => {
                                        if let Some(f) = fill.get(c).filter(|f| f.len() == bps) {
                                            o.copy_from_slice(f);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut px = data.chunks_exact(bpp);
            let first = px.next().unwrap_or_default();
            let uniform = px.all(|p| p == first);
            if !uniform || first != dp.as_slice() {
                out.push((tc, Arc::new(t), uniform));
            }
        }
        out
    });
    // One tile per value for the uniform ones (bands found theirs apart).
    let mut solid: Vec<Arc<photocraft_raster::Tile>> = Vec::new();
    let tiles = made.into_iter().flatten().map(|(tc, t, uniform)| {
        if !uniform {
            return (tc, t);
        }
        let value = t.bytes().get(..bpp);
        match solid.iter().find(|s| s.bytes().get(..bpp) == value) {
            Some(s) => (tc, Arc::clone(s)),
            None => {
                solid.push(Arc::clone(&t));
                (tc, t)
            }
        }
    });
    let tiles: Vec<_> = tiles.collect();
    s.put_tiles(tiles);
}

fn write_channel<const N: usize>(out: &mut [u8], ch: usize, c: usize, src: &[u8], fill: &[u8], decode: impl Fn([u8; N]) -> [u8; N]) {
    let mut samples = src.as_chunks::<N>().0.iter();
    for pixel in out.chunks_exact_mut(ch * N) {
        let dst = &mut pixel[c * N..(c + 1) * N];
        match samples.next() {
            Some(sample) => dst.copy_from_slice(&decode(*sample)),
            None => dst.copy_from_slice(fill),
        }
    }
}

/// Splits interleaved native-endian bytes into big-endian planes.
pub fn deinterleave(bytes: &[u8], ch: usize, s: SampleType, invert: &[bool]) -> Vec<Vec<u8>> {
    let bps = s.bytes();
    let n = bytes.len() / (ch * bps).max(1);
    if s == SampleType::U8 {
        // Fast path: one plane per thread, no per-sample copies.
        return par_map((0..ch).collect(), |c| {
            let it = bytes.iter().skip(c).step_by(ch).take(n);
            if invert[c] { it.map(|v| 255 - v).collect() } else { it.copied().collect() }
        });
    }
    // One band of rows per core, appended in band order into pre-grown planes.
    let row = ch * bps;
    let bs = bands(n);
    let parts: Vec<Vec<Vec<u8>>> = par_map(bs.clone(), |band| {
        let mut planes = vec![Vec::with_capacity(band.len() * bps); ch];
        let mut tmp = [0u8; 4];
        for p in band {
            for (c, plane) in planes.iter_mut().enumerate() {
                let src = &bytes[p * row + c * bps..p * row + (c + 1) * bps];
                tmp[..bps].copy_from_slice(src);
                if invert[c] {
                    invert_sample(&mut tmp[..bps], s);
                }
                if cfg!(target_endian = "big") {
                    plane.extend_from_slice(&tmp[..bps]);
                } else {
                    plane.extend(tmp[..bps].iter().rev());
                }
            }
        }
        planes
    });
    let mut planes: Vec<Vec<u8>> = (0..ch).map(|_| Vec::with_capacity(n * bps)).collect();
    for part in parts {
        for (c, plane) in planes.iter_mut().enumerate() {
            plane.extend_from_slice(&part[c]);
        }
    }
    planes
}

/// Native-endian encoding of the maximum sample value.
pub fn max_sample(s: SampleType) -> Vec<u8> {
    match s {
        SampleType::U8 => vec![255],
        SampleType::U16 => 65535u16.to_ne_bytes().to_vec(),
        SampleType::F32 => 1.0f32.to_ne_bytes().to_vec(),
    }
}

/// Top of Photoshop's 16-bit Lab a*/b* scale: it stores `32768 + 256·a` (0..65280 spans −128..127,
/// fitted on psd-tools `4x4_16bit_lab`: the merged image holds a stop's `LbCl` a* = 52.29 as 46154),
/// while documents keep a*/b* on the 8-bit scale `(a + 128) / 255` at every depth.
pub const LAB16_CHROMA_MAX: f32 = 65280.0;

/// Rescales the a*/b* channels (1 and 2) of interleaved native-endian 16-bit Lab pixels with
/// `ch` channels: PSD → document when `to_doc`, else document → PSD. Round-trips exactly.
pub fn lab16_chroma(bytes: &mut [u8], ch: usize, to_doc: bool) {
    if ch < 3 {
        return;
    }
    let k = if to_doc { 65535.0 / f64::from(LAB16_CHROMA_MAX) } else { f64::from(LAB16_CHROMA_MAX) / 65535.0 };
    for px in bytes.chunks_exact_mut(ch * 2) {
        for c in 1..3 {
            let v = u16::from_ne_bytes([px[2 * c], px[2 * c + 1]]);
            let w = (f64::from(v) * k).round().min(65535.0) as u16;
            px[2 * c..2 * c + 2].copy_from_slice(&w.to_ne_bytes());
        }
    }
}

/// Native-endian encoding of zero.
pub fn zero_sample(s: SampleType) -> Vec<u8> {
    vec![0; s.bytes()]
}

/// Quantizes a normalized value to big-endian bytes.
pub fn encode_be(v: f32, s: SampleType, out: &mut Vec<u8>) {
    match s {
        SampleType::U8 => out.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8),
        SampleType::U16 => out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).to_be_bytes()),
        SampleType::F32 => out.extend_from_slice(&v.to_be_bytes()),
    }
}

/// Decodes a big-endian sample at `index` to a normalized value.
#[cfg(test)]
pub fn decode_be(bytes: &[u8], index: usize, s: SampleType) -> f32 {
    match s {
        SampleType::U8 => f32::from(bytes[index]) / 255.0,
        SampleType::U16 => f32::from(u16::from_be_bytes([bytes[index * 2], bytes[index * 2 + 1]])) / 65535.0,
        SampleType::F32 => {
            let o = index * 4;
            f32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        }
    }
}

/// Photoshop stores the merged image of a transparent document matted
/// against white: `m = c·a + w·(1 − a)` per stored channel, where `w` is the
/// model's white. These convert between straight and matted values.
pub fn matte(c: f32, a: f32, white: f32) -> f32 {
    c * a + white * (1.0 - a)
}

/// Inverse of [`matte`] (returns `white` where alpha is 0).
/// Keep HDR and negative samples; integer encoders own normalized-range clipping.
pub fn unmatte(m: f32, a: f32, white: f32) -> f32 {
    if a <= 0.0 { white } else { (m - white * (1.0 - a)) / a }
}

/// `items.map(f)` on scoped threads (one per item; callers pass a handful of channels or
/// bands), in order; sequential on wasm or for a single item.
pub fn par_map<T: Send, R: Send>(items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Vec<R> {
    if cfg!(target_arch = "wasm32") || items.len() < 2 {
        return items.into_iter().map(f).collect();
    }
    std::thread::scope(|sc| {
        let f = &f;
        let hs: Vec<_> = items.into_iter().map(|t| sc.spawn(move || f(t))).collect();
        // Join every worker first (an unjoined panicked thread would make the scope panic),
        // then hand a worker's panic to the caller exactly as the sequential path would, so
        // the shell's last-resort guard reports it instead of the scope aborting the join.
        let joined: Vec<_> = hs.into_iter().map(|h| h.join()).collect();
        joined.into_iter().map(|r| r.unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
    })
}

/// `0..n` split into about one band per core.
pub fn bands(n: usize) -> Vec<std::ops::Range<usize>> {
    let k = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |v| v.get()).clamp(1, 32) };
    let step = n.div_ceil(k).max(1);
    (0..n).step_by(step).map(|a| a..(a + step).min(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn previous_interleave(planes: &[Option<&[u8]>], fill: &[Vec<u8>], n: usize, s: SampleType, invert: &[bool]) -> Vec<u8> {
        let bps = s.bytes();
        let ch = planes.len();
        let row = ch * bps;
        let mut out = vec![0u8; n * row];
        // One band of rows per core, written straight into its slice of the shared output: the
        // per-pixel channel loop stays, but a band holds ~1/cores of a 20 MP image's iterations
        // instead of all of them.
        let bs = bands(n);
        let parts: Vec<Vec<u8>> = par_map(bs.clone(), |band| {
            let mut part = vec![0u8; band.len() * row];
            for (i, p) in band.clone().enumerate() {
                for (c, plane) in planes.iter().enumerate() {
                    let dst = &mut part[(i * ch + c) * bps..(i * ch + c + 1) * bps];
                    match plane {
                        Some(pl) if pl.len() >= (p + 1) * bps => {
                            let src = &pl[p * bps..(p + 1) * bps];
                            for k in 0..bps {
                                dst[k] = src[bps - 1 - k];
                            }
                            if cfg!(target_endian = "big") {
                                dst.copy_from_slice(src);
                            }
                            if invert[c] {
                                invert_sample(dst, s);
                            }
                        }
                        _ => dst.copy_from_slice(&fill[c]),
                    }
                }
            }
            part
        });
        for (band, part) in bs.into_iter().zip(parts) {
            out[band.start * row..band.end * row].copy_from_slice(&part);
        }
        out
    }

    #[test]
    fn banded_shuffles_match_a_sequential_reference() {
        fn seq_deinterleave(bytes: &[u8], ch: usize, s: SampleType, invert: &[bool]) -> Vec<Vec<u8>> {
            let bps = s.bytes();
            let n = bytes.len() / (ch * bps).max(1);
            let mut planes = vec![Vec::with_capacity(n * bps); ch];
            let mut tmp = [0u8; 4];
            for i in 0..n {
                for (c, plane) in planes.iter_mut().enumerate() {
                    let src = &bytes[(i * ch + c) * bps..(i * ch + c + 1) * bps];
                    tmp[..bps].copy_from_slice(src);
                    if invert[c] {
                        invert_sample(&mut tmp[..bps], s);
                    }
                    plane.extend(tmp[..bps].iter().rev());
                }
            }
            planes
        }
        // Odd pixel counts cross band boundaries; a missing plane exercises the fill path.
        for n in [0usize, 1, 7, 64, 1000] {
            for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let bps = s.bytes();
                let planes: Vec<Vec<u8>> = (0..4).map(|c| (0..n * bps).map(|i| (i * 13 + c * 5) as u8).collect()).collect();
                let mut refs: Vec<Option<&[u8]>> = planes.iter().map(|p| Some(&p[..])).collect();
                refs[2] = None;
                refs[3] = Some(&planes[3][..(n * bps).saturating_sub(1)]);
                let fill: Vec<Vec<u8>> = (0..4).map(|c| vec![c as u8 * 40; bps]).collect();
                let invert = [false, true, false, true];
                let got = interleave(&refs, &fill, n, s, &invert);
                let want = previous_interleave(&refs, &fill, n, s, &invert);
                assert_eq!(got, want, "interleave n={n}");
                let planes_back = deinterleave(&got, 4, s, &invert);
                let want_back = seq_deinterleave(&got, 4, s, &invert);
                assert_eq!(planes_back, want_back, "deinterleave n={n}");
            }
        }
    }

    #[test]
    fn interleave_roundtrip_all_depths() {
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bps = s.bytes();
            let planes: Vec<Vec<u8>> = (0..3).map(|c| (0..4 * bps).map(|i| (i * 7 + c * 3) as u8).collect()).collect();
            let refs: Vec<Option<&[u8]>> = planes.iter().map(|p| Some(&p[..])).collect();
            let fill = vec![zero_sample(s); 3];
            let inv = [false, true, false];
            let il = interleave(&refs, &fill, 4, s, &inv);
            assert_eq!(interleave(&refs, &[], 4, s, &inv), il);
            let back = deinterleave(&il, 3, s, &inv);
            if s == SampleType::F32 {
                // 1 - (1 - v) may not be exact for floats; only check non-inverted planes.
                assert_eq!(back[0], planes[0]);
                assert_eq!(back[2], planes[2]);
            } else {
                assert_eq!(back, planes);
            }
        }
    }

    /// Planes written straight into tiles read exactly as interleaving them and writing that
    /// did, at every depth, for rects off the tile grid, missing, short and inverted planes, with
    /// a default pixel; tiles of one value share a copy, and none of only the default is kept.
    #[test]
    fn planes_written_into_tiles_read_as_the_interleaved_copy() {
        use photocraft_color::{ColorMode, PixelFormat};
        use photocraft_geom::Rect;
        use photocraft_raster::Surface;
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bps = s.bytes();
            let fmt = PixelFormat::new(ColorMode::Rgb, s, true);
            for (r, parallel) in [(Rect::new(-37, 5, 600, 530), true), (Rect::new(250, 250, 263, 777), false), (Rect::new(0, 0, 512, 256), true)] {
                let n = r.width() as usize * r.height() as usize;
                // Channel 0 a pattern, 1 missing, 2 cut short, alpha opaque on the left half.
                let p0: Vec<u8> = (0..n * bps).map(|i| ((i * 31) % 251) as u8).collect();
                let p2: Vec<u8> = (0..(n / 2) * bps).map(|i| ((i * 17) % 253) as u8).collect();
                let w = r.width() as usize;
                // Planes are big-endian; the samples are native.
                let pa: Vec<u8> = (0..n)
                    .flat_map(|i| {
                        let mut b = if i % w < w / 2 { max_sample(s) } else { zero_sample(s) };
                        if cfg!(target_endian = "little") {
                            b.reverse();
                        }
                        b
                    })
                    .collect();
                let planes = [Some(&p0[..]), None, Some(&p2[..]), Some(&pa[..])];
                let fill = vec![zero_sample(s), max_sample(s), zero_sample(s), max_sample(s)];
                let invert = [false, true, true, false];
                let mut want = Surface::new(fmt);
                want.write_interleaved(r, &interleave(&planes, &fill, n, s, &invert));
                want.prune();
                let mut got = Surface::new(fmt);
                write_planes(&mut got, r, &planes, &fill, &invert, parallel);
                assert_eq!(got.to_interleaved(r.inflate(3)), want.to_interleaved(r.inflate(3)), "{s:?} {r:?}");
                assert_eq!(got.tile_count(), want.tile_count(), "{s:?} {r:?}: the same tiles kept");
            }
            // A solid plane shares one tile; one of only the default pixel keeps none.
            let r = Rect::new(0, 0, 1024, 512);
            let n = 1024 * 512;
            let one: Vec<u8> = vec![0x40; n * bps];
            let mut solid = Surface::new(PixelFormat::new(ColorMode::Grayscale, s, false));
            write_planes(&mut solid, r, &[Some(&one)], &[zero_sample(s)], &[false], true);
            let first = solid.tiles().next().map(|(_, t)| t.clone()).expect("a tile");
            assert!(solid.tile_count() == 8 && solid.tiles().all(|(_, t)| std::sync::Arc::ptr_eq(t, &first)), "{s:?}: one shared tile");
            let mut blank = Surface::new(PixelFormat::new(ColorMode::Grayscale, s, false));
            write_planes(&mut blank, r, &[Some(&vec![0; n * bps])], &[zero_sample(s)], &[false], true);
            assert_eq!(blank.tile_count(), 0, "{s:?}");
        }
    }

    #[test]
    fn missing_planes_use_fill() {
        let il = interleave(&[None, Some(&[5, 6])], &[vec![9], vec![0]], 2, SampleType::U8, &[false, false]);
        assert_eq!(il, vec![9, 5, 9, 6]);
        for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for n in [1, 1000] {
                let fill = vec![vec![9; s.bytes()]; 2];
                assert_eq!(interleave(&[None, Some(&[1, 2, 3][..s.bytes() - 1])], &fill, n, s, &[]), vec![9; n * 2 * s.bytes()]);
            }
        }
    }

    #[test]
    fn lab16_chroma_scale_round_trips() {
        // Two pixels of three channels: L is untouched, a*/b* rescale, and every PSD value
        // comes back exactly.
        let mut all = Vec::new();
        for v in 0..=u16::MAX {
            all.extend_from_slice(&v.to_ne_bytes());
            all.extend_from_slice(&v.to_ne_bytes());
            all.extend_from_slice(&v.to_ne_bytes());
        }
        let orig = all.clone();
        lab16_chroma(&mut all, 3, true);
        let px = |b: &[u8], i: usize, c: usize| u16::from_ne_bytes([b[(i * 3 + c) * 2], b[(i * 3 + c) * 2 + 1]]);
        // Neutral (32768 in PSD) is 128 / 255 of the document scale; 65280 (a* = 127) is the top.
        assert_eq!(px(&all, 32768, 0), 32768);
        assert_eq!(px(&all, 32768, 1), (32768.0f64 * 65535.0 / 65280.0).round() as u16);
        assert_eq!(px(&all, 65280, 2), 65535);
        lab16_chroma(&mut all, 3, false);
        for i in 0..=65280usize {
            assert_eq!(px(&all, i, 1), px(&orig, i, 1), "{i}");
        }
        // Short or odd buffers are left alone.
        let mut short = vec![1u8, 2, 3];
        lab16_chroma(&mut short, 3, true);
        lab16_chroma(&mut short, 1, true);
        assert_eq!(short, [1, 2, 3]);
    }

    #[test]
    fn be_codec() {
        let mut v = Vec::new();
        encode_be(1.0, SampleType::U16, &mut v);
        assert_eq!(v, vec![0xff, 0xff]);
        assert_eq!(decode_be(&v, 0, SampleType::U16), 1.0);
        let mut f = Vec::new();
        encode_be(2.5, SampleType::F32, &mut f);
        assert_eq!(decode_be(&f, 0, SampleType::F32), 2.5);
    }
}

#[cfg(test)]
mod measurement {
    use super::*;
    #[test]
    #[ignore = "manual 24 MP release benchmark"]
    fn benchmark_interleave() {
        use std::{hint::black_box, time::Instant};
        const N: usize = 24_000_000;
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let bps = sample.bytes();
            let planes: Vec<Vec<u8>> = (0..4).map(|c| (0..N * bps).map(|i| (i * 13 + c * 5) as u8).collect()).collect();
            let refs: Vec<_> = planes.iter().map(|p| Some(p.as_slice())).collect();
            let fill = vec![vec![0; bps]; 4];
            let invert = [false, true, false, true];
            assert_eq!(tests::previous_interleave(&refs, &fill, N, sample, &invert), interleave(&refs, &fill, N, sample, &invert));
            for trial in 0..6 {
                let mut paths = [("main", tests::previous_interleave as fn(_, _, _, _, _) -> _), ("split", interleave)];
                if trial % 2 == 1 {
                    paths.reverse();
                }
                for (name, path) in paths {
                    let t = Instant::now();
                    let out = black_box(path)(&refs, &fill, N, sample, &invert);
                    let elapsed = t.elapsed().as_secs_f64() * 1000.0;
                    black_box(&out);
                    println!("interleave,{sample:?},{trial},{name},{elapsed:.6}");
                }
            }
        }
    }
}
