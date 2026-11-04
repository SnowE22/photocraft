//! TypeR-compatible Content-Aware Fill presets (phase 4).
//!
//! TypeR Studio's `inpaint.js` fills a masked area with a multi-level Wexler/PatchMatch scheme and
//! weights patch similarity towards structure (gradient) differences by `STRUCTURE = 0.6`, so
//! screentone dots and ink lines survive instead of turning into a blurred patch. PhotoCraft's
//! [`crate::content_aware`] already uses the same Wexler/PatchMatch pyramid; with colour adaptation
//! off it copies real source pixels, which keeps screentone grain and line art exactly rather than
//! approximating them. This module pins that TypeR-style preset for the manga workflow.

use crate::content_aware::{FillOptions, fill};
use crate::inpaint::Adapt;

/// TypeR's weighting of a gradient difference against a tone difference (documented here because
/// the preset below reaches the same goal by copying source pixels exactly).
pub const STRUCTURE_WEIGHT: f32 = 0.6;

/// Content-Aware Fill options for manga lettering removal: exact pixel copies (no colour
/// adaptation, no rotations or resampling), deterministic, which preserves screentone grain and
/// black contour lines.
pub fn typer_options() -> FillOptions {
    FillOptions { color_adaptation: Adapt::NONE, rotations: Vec::new(), scale: false, mirror: false, seed: 1 }
}

/// Fills `hole` from `source` with the TypeR preset; `img` is interleaved `w × h × ch` floats.
pub fn typer_fill(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], source: &[bool]) -> Vec<f32> {
    fill(w, h, ch, img, hole, source, &typer_options())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Screentone (dark dots every 4th pixel on light paper) crossed by a black ink line.
    fn screentone(w: usize, h: usize) -> (Vec<f32>, Vec<bool>) {
        let line = h / 2;
        let img: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if y == line || y == line + 1 {
                    0.03
                } else if x % 4 == 0 && y % 4 == 0 {
                    0.15
                } else {
                    0.92
                }
            })
            .collect();
        let hole: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                (w / 2 - 7..w / 2 + 7).contains(&x) && (line.saturating_sub(5)..line + 7).contains(&y)
            })
            .collect();
        (img, hole)
    }

    #[test]
    fn typer_fill_keeps_screentone_and_the_ink_line() {
        let (w, h) = (72, 64);
        let (img, hole) = screentone(w, h);
        let source = vec![true; w * h];
        let out = typer_fill(w, h, 1, &img, &hole, &source);
        assert_eq!(out.len(), img.len());

        // Only the hole changes; the paper, dots and line around it are untouched.
        for i in 0..w * h {
            if !hole[i] {
                assert_eq!(out[i], img[i], "pixel {i}");
            }
        }
        // The fill is textured, not a flat smudge: it keeps both tone and dots.
        let filled: Vec<f32> = (0..w * h).filter(|i| hole[*i]).map(|i| out[i]).collect();
        assert!(filled.iter().any(|v| *v < 0.4), "no dark dots or ink in the fill");
        assert!(filled.iter().any(|v| *v > 0.7), "the fill has no light paper");

        // The ink line is continued across the hole: every hole column near the line stays dark.
        let line = h / 2;
        let columns: Vec<usize> = (w / 2 - 7..w / 2 + 7).collect();
        let dark = columns
            .iter()
            .filter(|x| {
                let row = line;
                let value = out[row * w + **x];
                value < 0.45
            })
            .count();
        assert!(dark * 2 >= columns.len(), "the line continues through the fill ({dark}/{})", columns.len());
    }

    #[test]
    fn typer_preset_is_exact_copy_without_adaptation() {
        let opts = typer_options();
        assert!(opts.color_adaptation.is_none(), "exact copies keep screentone grain");
        assert_eq!(opts.rotations.len(), 0);
        assert!(!opts.scale);
        assert!(!opts.mirror);
        assert!((STRUCTURE_WEIGHT - 0.6).abs() < f32::EPSILON);
    }
}
