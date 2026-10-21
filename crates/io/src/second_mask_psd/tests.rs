use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerMask, Size};
use photocraft_geom::Rect;

/// A 40×20 document whose layer has a left-half second mask over a top-half layer mask.
fn doc(depth: SampleType) -> Document {
    let mut d = Document::new("Two masks", Size::new(40, 20), ColorMode::Rgb, depth);
    let mut l = Layer::raster("Red", d.pixel_format());
    if let Some(s) = l.surface_mut() {
        s.fill_rect(Rect::new(0, 0, 40, 20), &[1.0, 0.0, 0.0, 1.0]);
    }
    let gray = PixelFormat { sample: depth, ..PixelFormat::GRAY8 };
    let mut first = LayerMask { surface: photocraft_raster::Surface::with_default(gray, &[0.0]), ..LayerMask::reveal_all() };
    first.surface.fill_rect(Rect::new(0, 0, 40, 10), &[1.0]);
    let mut second = LayerMask { surface: photocraft_raster::Surface::with_default(gray, &[0.0]), density: 0.75, ..LayerMask::reveal_all() };
    second.surface.fill_rect(Rect::new(0, 0, 20, 20), &[1.0]);
    l.mask = Some(first);
    l.second_mask = Some(second);
    d.layers.push(l);
    d
}

fn bytes(d: &Document) -> Vec<u8> {
    crate::psd_export::document_to_psd(d).to_bytes().unwrap()
}

/// Both masks come back from a PhotoCraft PSD exactly, and the file's own layer mask (what
/// Photoshop shows) is their product.
#[test]
fn both_masks_survive_a_psd_and_photoshop_sees_their_product() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = doc(depth);
        let data = bytes(&d);
        let back = crate::import("two.psd", &data).unwrap().document;
        let (a, b) = (&d.layers[0], &back.layers[back.layers.len() - 1]);
        let (m1, m2) = (b.mask.as_ref().unwrap(), b.second_mask.as_ref().unwrap());
        for (x, y) in [(5, 5), (30, 5), (5, 15), (30, 15)] {
            assert!((m1.value(x, y) - a.mask.as_ref().unwrap().value(x, y)).abs() < 1e-3, "{depth:?} first at {x},{y}");
            assert!((m2.value(x, y) - a.second_mask.as_ref().unwrap().value(x, y)).abs() < 1e-3, "{depth:?} second at {x},{y}");
        }
        assert!(b.psd_blocks.iter().all(|(k, _)| *k != super::KEY), "the block is not kept");
        // Without the block (a reader that ignores it): the product.
        let only = super::for_export(&d).unwrap();
        let shown = only.layers[0].mask.as_ref().unwrap();
        assert!((shown.value(5, 5) - 1.0).abs() < 1e-3 && (shown.value(30, 5) - 0.25).abs() < 2e-2 && shown.value(5, 15) < 1e-3);
        assert!(only.layers[0].second_mask.is_none());
    }
}

/// A damaged block leaves the file's (multiplied) mask and a warning, never a panic.
#[test]
fn a_damaged_block_keeps_the_files_mask() {
    let first = LayerMask::reveal_all();
    let good = super::encode(&first, &first);
    for cut in [0, 3, 4, 10, good.len() - 1] {
        assert!(super::decode(&good[..cut], None).is_none(), "{cut}");
    }
    let mut d = doc(SampleType::U8);
    d.layers[0].psd_blocks.push((super::KEY, std::sync::Arc::new(vec![0, 0, 0, 1, 9])));
    d.layers[0].second_mask = None;
    let mut warnings = Vec::new();
    super::after_import(&mut d, &mut warnings);
    assert_eq!(warnings.len(), 1);
    assert!(d.layers[0].second_mask.is_none() && d.layers[0].mask.is_some() && d.layers[0].psd_blocks.is_empty());
    // A huge claimed area is refused before inflating.
    let mut huge = 1u32.to_be_bytes().to_vec();
    huge.extend([1, 1, 1]);
    huge.extend([1f32.to_be_bytes(), 0f32.to_be_bytes(), 1f32.to_be_bytes()].concat());
    huge.extend([0i32, 0, 1_000_000, 1_000_000].iter().flat_map(|v| v.to_be_bytes()));
    huge.extend(0u32.to_be_bytes());
    assert!(super::decode(&huge, None).is_none());
}
