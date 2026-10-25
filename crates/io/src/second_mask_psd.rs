//! PhotoCraft's second layer mask (`Layer::second_mask`) in a PSD. Photoshop has one pixel mask a
//! layer, so the file's layer mask is the two multiplied (what Photoshop and every other reader
//! shows is the layer as PhotoCraft draws it), and a private `PCm2` block keeps both masks as they
//! were, which PhotoCraft reads back. A file saved again by Photoshop loses the block and keeps
//! the multiplied mask.

use std::sync::Arc;

use photocraft_color::SampleType;
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// The private block's key.
pub const KEY: [u8; 4] = *b"PCm2";
const VERSION: u32 = 1;
/// The largest mask area the block decodes (pixels), as a guard against hostile files.
const MAX_PIXELS: u64 = 1 << 31;

/// `doc` ready for PSD export: each layer with a second mask gets the masks' product as its layer
/// mask and the `PCm2` block. `None` when no layer has one (export `doc` itself).
pub fn for_export(doc: &Document) -> Option<Document> {
    fn any(layers: &[Layer]) -> bool {
        layers.iter().any(|l| l.second_mask.is_some() || l.children().is_some_and(any))
    }
    if !any(&doc.layers) {
        return None;
    }
    fn walk(layers: &mut [Layer], canvas: Rect) {
        for l in layers {
            if let LayerContent::Group(g) = &mut l.content {
                walk(&mut g.children, canvas);
            }
            let Some(second) = l.second_mask.take() else { continue };
            l.psd_blocks.retain(|(k, _)| *k != KEY);
            let Some(first) = l.mask.clone() else { continue };
            l.psd_blocks.push((KEY, Arc::new(encode(&first, &second))));
            if second.enabled {
                l.mask = Some(multiplied(&first, &second, canvas));
            }
        }
    }
    let mut out = doc.clone();
    walk(&mut out.layers, doc.bounds());
    Some(out)
}

/// After import: layers with a `PCm2` block get their two masks back (the block is dropped
/// either way, so a save writes it fresh). A block that doesn't decode leaves the file's mask.
pub fn after_import(doc: &mut Document, warnings: &mut Vec<String>) {
    fn walk(layers: &mut [Layer], warnings: &mut Vec<String>) {
        for l in layers {
            if let LayerContent::Group(g) = &mut l.content {
                walk(&mut g.children, warnings);
            }
            let Some(i) = l.psd_blocks.iter().position(|(k, _)| *k == KEY) else { continue };
            let (_, data) = l.psd_blocks.remove(i);
            match decode(&data, l.mask.as_ref().map(|m| m.surface.format().sample)) {
                Some((first, second)) => {
                    l.mask = Some(first);
                    l.second_mask = Some(second);
                }
                None => warnings.push(format!("layer \"{}\": its second mask could not be read; the file's layer mask is kept", l.name)),
            }
        }
    }
    walk(&mut doc.layers, warnings);
}

/// The two masks' effective values (density applied) multiplied, with the first's feather, link
/// and enabled state.
fn multiplied(a: &LayerMask, b: &LayerMask, canvas: Rect) -> LayerMask {
    let eff = |m: &LayerMask, v: f32| 1.0 - m.density * (1.0 - v);
    let default = |m: &LayerMask| eff(m, m.surface.default_pixel().first().copied().unwrap_or(1.0));
    let fmt = a.surface.format();
    let mut surface = Surface::with_default(fmt, &[(default(a) * default(b)).clamp(0.0, 1.0)]);
    let area = union(a.surface.content_bounds(), b.surface.content_bounds()).intersect(&canvas);
    if !area.is_empty() {
        let mut va = Vec::new();
        let mut vb = Vec::new();
        a.values_into(area, &mut va);
        b.values_into(area, &mut vb);
        for (x, y) in va.iter_mut().zip(&vb) {
            *x = (*x * y).clamp(0.0, 1.0);
        }
        surface.write_region(area, &va);
        surface.prune();
    }
    LayerMask { surface, density: 1.0, ..a.clone() }
}

fn union(a: Rect, b: Rect) -> Rect {
    if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    }
}

fn encode(first: &LayerMask, second: &LayerMask) -> Vec<u8> {
    let mut out = VERSION.to_be_bytes().to_vec();
    for m in [first, second] {
        encode_mask(m, &mut out);
    }
    out
}

fn encode_mask(m: &LayerMask, out: &mut Vec<u8>) {
    let sample = m.surface.format().sample;
    out.extend([u8::from(m.enabled), u8::from(m.linked), sample.bytes() as u8]);
    out.extend(m.density.to_be_bytes());
    out.extend(m.feather.to_be_bytes());
    out.extend(m.surface.default_pixel().first().copied().unwrap_or(1.0).to_be_bytes());
    let r = m.surface.content_bounds();
    for v in [r.x0, r.y0, r.x1, r.y1] {
        out.extend(v.to_be_bytes());
    }
    let mut raw = Vec::new();
    if !r.is_empty() {
        for v in m.surface.read_region(r) {
            match sample {
                SampleType::U8 => raw.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
                SampleType::U16 => raw.extend(((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes()),
                SampleType::F32 => raw.extend(v.to_be_bytes()),
            }
        }
    }
    let z = photocraft_psd::compression::zip_compress(&raw);
    out.extend((z.len() as u32).to_be_bytes());
    out.extend(z);
}

/// Reads a block; the masks take `depth` (the file's mask depth) when given. `None` when it is
/// malformed.
fn decode(data: &[u8], depth: Option<SampleType>) -> Option<(LayerMask, LayerMask)> {
    let mut r = Reader { data, at: 0 };
    if r.u32()? != VERSION {
        return None;
    }
    let first = decode_mask(&mut r, depth)?;
    let second = decode_mask(&mut r, depth)?;
    Some((first, second))
}

fn decode_mask(r: &mut Reader, depth: Option<SampleType>) -> Option<LayerMask> {
    let (enabled, linked) = (r.u8()? != 0, r.u8()? != 0);
    let sample = match r.u8()? {
        1 => SampleType::U8,
        2 => SampleType::U16,
        4 => SampleType::F32,
        _ => return None,
    };
    let (density, feather, default) = (r.f32()?, r.f32()?, r.f32()?);
    if ![density, feather, default].iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x0, y0, x1, y1) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
    let len = usize::try_from(r.u32()?).ok()?;
    let z = r.bytes(len)?;
    let mut m = LayerMask { enabled, linked, density: density.clamp(0.0, 1.0), feather: feather.clamp(0.0, 1000.0), ..LayerMask::reveal_all() };
    let fmt = m.surface.format().with_sample(depth.unwrap_or(sample));
    m.surface = Surface::with_default(fmt, &[default.clamp(0.0, 1.0)]);
    if x1 > x0 && y1 > y0 {
        let (w, h) = (u64::from(x1.abs_diff(x0)), u64::from(y1.abs_diff(y0)));
        let n = w.checked_mul(h).filter(|n| *n <= MAX_PIXELS)?;
        let expected = usize::try_from(n.checked_mul(sample.bytes() as u64)?).ok()?;
        let raw = photocraft_psd::compression::zip_decompress(z, expected).ok()?;
        if raw.len() != expected {
            return None;
        }
        let values: Vec<f32> = match sample {
            SampleType::U8 => raw.iter().map(|b| f32::from(*b) / 255.0).collect(),
            SampleType::U16 => raw.as_chunks::<2>().0.iter().map(|b| f32::from(u16::from_be_bytes(*b)) / 65535.0).collect(),
            SampleType::F32 => {
                raw.as_chunks::<4>().0.iter().map(|b| f32::from_be_bytes(*b)).map(|v| if v.is_finite() { v } else { 0.0 }).collect()
            }
        };
        m.surface.write_region(Rect::new(x0, y0, x1, y1), &values);
        m.surface.prune();
    }
    Some(m)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let b = self.data.get(self.at..end)?;
        self.at = end;
        Some(b)
    }
    fn u8(&mut self) -> Option<u8> {
        self.bytes(1)?.first().copied()
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_be_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_be_bytes(self.bytes(4)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests;
