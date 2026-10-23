//! Each adjustment type's own glyph, as Photoshop's Layers and Adjustments panels show one per
//! type (a histogram for Levels, a curve for Curves, a balance for Color Balance…). PhotoCraft's
//! own drawings (`assets/icons/adj-*.svg`, see `ATTRIBUTION.md`), tinted with the theme's icon
//! colour like every other icon.

use egui::{Color32, Rect};
use photocraft_doc::Adjustment;

/// The icon for an adjustment kind (`layer.newAdjustmentLayer.<kind>`). Photoshop 2026's Light,
/// Color and vibrance and Clarity and dehaze have theirs ready (`light`, `colorAndVibrance`,
/// `clarityAndDehaze`); unknown kinds get the generic adjustment-layer glyph.
pub fn for_kind(kind: &str) -> &'static str {
    match kind {
        "brightnessContrast" => "adj-brightness-contrast",
        "levels" => "adj-levels",
        "curves" => "adj-curves",
        "exposure" => "adj-exposure",
        "vibrance" => "adj-vibrance",
        "hueSaturation" => "adj-hue-saturation",
        "colorBalance" => "adj-color-balance",
        "blackWhite" => "adj-black-white",
        "photoFilter" => "adj-photo-filter",
        "channelMixer" => "adj-channel-mixer",
        "colorLookup" => "adj-color-lookup",
        "invert" => "adj-invert",
        "posterize" => "adj-posterize",
        "threshold" => "adj-threshold",
        "gradientMap" => "adj-gradient-map",
        "selectiveColor" => "adj-selective-color",
        "grain" => "adj-grain",
        "clarityAndDehaze" => "adj-clarity-dehaze",
        "colorAndVibrance" => "adj-color-vibrance",
        "light" => "adj-light",
        _ => "adjustment-layer",
    }
}

/// The icon for an adjustment layer's content.
pub fn for_adjustment(adj: &Adjustment) -> &'static str {
    match adj {
        // An imported block PhotoCraft can't evaluate: the generic glyph, not Invert's.
        Adjustment::Unsupported { .. } => "adjustment-layer",
        a => for_kind(photocraft_engine::commands::adjustment_kind(a)),
    }
}

/// Paint the adjustment's glyph centred in a Layers panel thumbnail cell.
pub fn paint(ui: &egui::Ui, cell: Rect, adj: &Adjustment, tint: Color32) {
    let size = (cell.width().min(cell.height()) * 0.62).clamp(12.0, 20.0);
    crate::icons::paint(ui, cell, for_adjustment(adj), size, tint);
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [&str; 20] = [
        "brightnessContrast",
        "levels",
        "curves",
        "exposure",
        "vibrance",
        "hueSaturation",
        "colorBalance",
        "blackWhite",
        "photoFilter",
        "channelMixer",
        "colorLookup",
        "invert",
        "posterize",
        "threshold",
        "gradientMap",
        "selectiveColor",
        "grain",
        "clarityAndDehaze",
        "colorAndVibrance",
        "light",
    ];

    /// Every type has its own embedded glyph: no two share one, none falls back.
    #[test]
    fn every_adjustment_type_has_its_own_icon() {
        let names: Vec<&str> = KINDS.iter().map(|k| for_kind(k)).collect();
        for (kind, name) in KINDS.iter().zip(&names) {
            assert!(crate::icons::exists(name), "{kind}: `{name}` is not embedded");
            assert_ne!(*name, "adjustment-layer", "{kind} falls back to the generic glyph");
            assert_eq!(names.iter().filter(|n| *n == name).count(), 1, "{kind} shares `{name}`");
        }
        assert_eq!(for_kind("noSuchKind"), "adjustment-layer");
    }

    /// Each layer content maps through its engine kind; imported blocks get the generic glyph.
    #[test]
    fn adjustment_layers_map_to_their_kind_icon() {
        let mode = photocraft_color::ColorMode::Rgb;
        for kind in KINDS.iter().take(17) {
            let adj = photocraft_engine::adjust_params::default_for(kind, mode).unwrap();
            assert_eq!(for_adjustment(&adj), for_kind(kind), "{kind}");
        }
        let raw = Adjustment::Unsupported { psd_key: "zzzz".into(), raw: Vec::new() };
        assert_eq!(for_adjustment(&raw), "adjustment-layer");
    }

    /// The glyphs are well-formed SVG the icon loader can rasterise (a broken file would draw the
    /// fallback square in the panel).
    #[test]
    fn adjustment_icons_parse() {
        for kind in KINDS {
            let svg = crate::icons::svg_bytes(for_kind(kind)).unwrap();
            let text = String::from_utf8(svg).unwrap();
            assert!(text.contains("<svg") && text.trim_end().ends_with("</svg>"), "{kind}");
            assert!(!text.contains("currentColor"), "{kind}: recoloured for tinting");
            let size = egui::load::SizeHint::Size { width: 24, height: 24, maintain_aspect_ratio: true };
            let image = egui_extras::image::load_svg_bytes_with_size(text.as_bytes(), size, &Default::default()).unwrap_or_else(|e| panic!("{kind}: {e}"));
            let ink = image.pixels.iter().filter(|p| p.a() > 16).count();
            assert!(ink > 40, "{kind}: only {ink} inked pixels at 24 px");
        }
    }
}
