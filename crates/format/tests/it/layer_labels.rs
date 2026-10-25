use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LabelColor, Layer, Size};

#[test]
fn layer_labels_pcraft_roundtrip_all_colors_and_depths() {
    for depth in SampleType::ALL {
        let mut doc = Document::new("Labels", Size::new(2, 2), ColorMode::Rgb, depth);
        for color in LabelColor::ALL {
            let mut child = Layer::raster("Child", doc.pixel_format());
            child.label = color;
            let mut group = Layer::group(color.label(), vec![child]);
            group.label = color;
            doc.layers.push(group);
        }
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), doc, "{depth:?}");
        assert!(photocraft_format::load_from_bytes(&bytes[..bytes.len() / 2]).is_err());
    }
}

/// The Fill tool's reference layers survive a `.pcraft` round trip; a file from before them
/// loads with none.
#[test]
fn reference_layers_pcraft_roundtrip() {
    let mut doc = Document::new("Reference", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    let mut lines = Layer::raster("Lines", doc.pixel_format());
    lines.reference = true;
    let mut group = Layer::group("Inks", vec![lines, Layer::raster("Colour", doc.pixel_format())]);
    group.reference = true;
    doc.layers.push(group);
    doc.layers.push(Layer::raster("Plain", doc.pixel_format()));
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back, doc);
    let flags: Vec<bool> = back.walk().iter().map(|(_, _, l)| l.reference).collect();
    assert_eq!(flags, vec![true, true, false, false]);
}
