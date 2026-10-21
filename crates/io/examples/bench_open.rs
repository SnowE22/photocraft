//! Where opening a PSD goes: parsing the file, making the document, the whole import.
//!
//! ```sh
//! cargo run --release -p photocraft-io --example bench_open -- file.psd [reps]
//! ```

use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    let path = std::env::args().nth(1).expect("give a file");
    let reps: usize = std::env::args().nth(2).and_then(|r| r.parse().ok()).unwrap_or(3);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    for _ in 0..reps {
        let t = Instant::now();
        let file = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let parse = ms(t);
        println!("  {} channels in the header", file.header.channels);
        let t = Instant::now();
        let (doc, _) = photocraft_io::psd_to_document(&file);
        let convert = ms(t);
        let t = Instant::now();
        let r = photocraft_io::import(&path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let all = ms(t);
        let mut kinds = std::collections::BTreeMap::new();
        for (_, _, l) in r.document.walk() {
            let k = match &l.content {
                photocraft_doc::LayerContent::Raster(_) => "pixel",
                photocraft_doc::LayerContent::Text(_) => "type",
                photocraft_doc::LayerContent::Shape(_) => "shape",
                photocraft_doc::LayerContent::Smart(_) => "smart",
                photocraft_doc::LayerContent::Group(_) => "group",
                photocraft_doc::LayerContent::Adjustment(_) => "adjustment",
                photocraft_doc::LayerContent::Fill(_) => "fill",
            };
            *kinds.entry(k).or_insert(0) += 1;
            if photocraft_compose::effects::has_effects(l) {
                *kinds.entry("with effects").or_insert(0) += 1;
            }
            if l.mask.is_some() {
                *kinds.entry("with masks").or_insert(0) += 1;
            }
        }
        println!("  {kinds:?}");
        let mut tiles = std::collections::BTreeMap::new();
        for (_, _, l) in r.document.walk() {
            let k = match &l.content {
                photocraft_doc::LayerContent::Raster(_) => "pixel",
                photocraft_doc::LayerContent::Smart(_) => "smart",
                _ => "other",
            };
            if let Some(s) = l.surface() {
                *tiles.entry(k).or_insert(0usize) += s.tile_count();
            }
            if let Some(c) = &l.fill_cache {
                *tiles.entry("fill cache").or_insert(0usize) += c.surface.tile_count();
                let uniq: std::collections::HashSet<*const photocraft_raster::Tile> = c.surface.tiles().map(|(_, t)| std::sync::Arc::as_ptr(t)).collect();
                *tiles.entry("fill cache distinct").or_insert(0usize) += uniq.len();
            }
            if let Some(m) = &l.mask {
                *tiles.entry("mask").or_insert(0usize) += m.surface.tile_count();
            }
        }
        println!("  tiles {tiles:?}");
        println!("parse {parse:7.1} ms   to document {convert:7.1} ms   import {all:7.1} ms   ({} layers)", doc.walk().len().max(r.document.walk().len()));
    }
}
