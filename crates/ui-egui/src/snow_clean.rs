//! Auto-clean support for TypeR paste-on-click (phase 3): the balloon under the selection is
//! filled with a clean white or black before the translation is placed over it.
//!
//! The fill colour follows the artwork: a bright balloon gets white, a dark one black, so the
//! original Japanese/Korean lettering is hidden without a grey box. The result goes to a layer
//! named `TypeR Clean`, which keeps the clean-up editable and separate from the text.

use photocraft_color::ColorMode;
use photocraft_raster::Surface;

/// Layer that receives the clean fill.
pub const CLEAN_LAYER: &str = "TypeR Clean";

/// White or black for a mean luminance (0..=255): bright areas get white, dark ones black.
pub fn color_for_luminance(luminance: f32) -> [u8; 3] {
    if luminance >= 118.0 { [255, 255, 255] } else { [0, 0, 0] }
}

/// The filter factor that samples a selected area with at most `budget` probes.
pub fn sample_step(width: u32, height: u32, budget: u32) -> u32 {
    if width == 0 || height == 0 || budget == 0 {
        return 1;
    }
    let area = u64::from(width) * u64::from(height);
    let step = (area as f64 / f64::from(budget)).sqrt().ceil() as u32;
    step.max(1)
}

fn luminance_of([r, g, b, a]: [u8; 4]) -> Option<f32> {
    if a == 0 {
        return None;
    }
    Some(0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b))
}

fn pixel(surface: &Surface, x: i32, y: i32) -> Option<[u8; 4]> {
    let mut value = [[0u8; 4]; 1];
    surface.read_rgba8_into(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &mut value);
    Some(value[0])
}

/// Mean luminance of `layer` inside the selected area; `None` when nothing is selected or the
/// layer has no pixels there.
pub fn mean_luminance(layer: Option<&Surface>, selection: &Surface) -> Option<f32> {
    let area = selection.content_bounds();
    if area.is_empty() {
        return None;
    }
    let layer_area = layer.map(|surface| surface.content_bounds()).unwrap_or(area);
    let x0 = area.x0.max(layer_area.x0);
    let y0 = area.y0.max(layer_area.y0);
    let x1 = area.x1.min(layer_area.x1);
    let y1 = area.y1.min(layer_area.y1);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let step = sample_step((x1 - x0) as u32, (y1 - y0) as u32, 4096) as i32;
    let mut sum = 0.0f64;
    let mut count = 0u32;
    for y in (y0..y1).step_by(step.max(1) as usize) {
        for x in (x0..x1).step_by(step.max(1) as usize) {
            let Some(selected) = pixel(selection, x, y) else { continue };
            if selected[3] < 128 {
                continue;
            }
            let Some(source) = layer.and_then(|surface| pixel(surface, x, y)) else { continue };
            if let Some(luminance) = luminance_of(source) {
                sum += f64::from(luminance);
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        None
    } else {
        Some((sum / f64::from(count)) as f32)
    }
}

/// The clean fill colour for the selected balloon: white on bright art, black on dark art, white
/// when the layer cannot be sampled (an empty or vector layer under the selection).
pub fn clean_fill_color(layer: Option<&Surface>, selection: &Surface) -> [u8; 3] {
    mean_luminance(layer, selection).map(color_for_luminance).unwrap_or([255, 255, 255])
}

/// A grayscale or RGBA surface is a pixel layer; shapes, text and groups have no pixels of their
/// own to sample, and are cleaned with white.
pub fn is_sampleable(surface: &Surface) -> bool {
    !matches!(surface.format().mode, ColorMode::Cmyk | ColorMode::Lab)
}

/// A cheap fingerprint of a selection mask (bounds plus a 32×32 sample of its alpha), so the
/// paste-on-click hook fires once per new selection instead of every frame.
pub fn selection_fingerprint(selection: &Surface) -> u64 {
    let area = selection.content_bounds();
    if area.is_empty() {
        return 0;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mix = |hash: &mut u64, value: u64| {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(&mut hash, area.x0 as u64);
    mix(&mut hash, area.y0 as u64);
    mix(&mut hash, area.x1 as u64);
    mix(&mut hash, area.y1 as u64);
    let step_x = ((area.x1 - area.x0) / 32).max(1);
    let step_y = ((area.y1 - area.y0) / 32).max(1);
    for y in (area.y0..area.y1).step_by(step_y as usize) {
        for x in (area.x0..area.x1).step_by(step_x as usize) {
            if let Some(px) = pixel(selection, x, y) {
                mix(&mut hash, u64::from(px[3]));
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bright_art_gets_white_and_dark_art_black() {
        assert_eq!(color_for_luminance(255.0), [255, 255, 255]);
        assert_eq!(color_for_luminance(118.0), [255, 255, 255]);
        assert_eq!(color_for_luminance(117.9), [0, 0, 0]);
        assert_eq!(color_for_luminance(0.0), [0, 0, 0]);
    }

    #[test]
    fn sampling_stays_within_the_budget() {
        assert_eq!(sample_step(0, 100, 100), 1);
        assert_eq!(sample_step(100, 100, 0), 1);
        assert_eq!(sample_step(10, 10, 4096), 1);
        let step = sample_step(4000, 4000, 4096);
        assert!(step >= 62, "{step}");
    }

    #[test]
    fn unsampleable_surfaces_are_still_cleanable() {
        // A surface whose format is CMYK is rejected; the rest may be sampled.
        let surface = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Rgb, photocraft_color::SampleType::U8, true));
        assert!(is_sampleable(&surface));
        let cmyk = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Cmyk, photocraft_color::SampleType::U8, true));
        assert!(!is_sampleable(&cmyk));
        assert_eq!(clean_fill_color(Some(&cmyk), &cmyk), [255, 255, 255]);
    }
}
