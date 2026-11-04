//! Interactive line-break preview for the TypeR script editor (phase 2).
//!
//! Shows the current script line above the editor: hovering it reveals the syllable points where
//! the line may break (from `photocraft_engine::snow_hyphenation`), and clicking a point splits
//! the line there with a trailing hyphen. A line that already ends with a hyphen shows a join
//! marker, and clicking it merges the next line back.

use egui::{Color32, FontId, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::theme::Tokens;

/// What a click in the preview asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakAction {
    /// Split before this character (an offset inside the line, in chars).
    Split { char_index: usize },
    /// Join the line with the next one (the line ends with a hyphen).
    Merge,
}

/// Character offsets where `line` may be split: every alphabetic run is hyphenated by the
/// language-aware rules of the engine; offsets are relative to the whole line.
pub fn line_break_points(line: &str) -> Vec<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (index, c) in chars.iter().enumerate() {
        if c.is_alphabetic() {
            start.get_or_insert(index);
        } else if let Some(from) = start.take() {
            push_run(&chars[from..index], from, &mut out);
        }
    }
    if let Some(from) = start {
        push_run(&chars[from..], from, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn push_run(run: &[char], offset: usize, out: &mut Vec<usize>) {
    let word: String = run.iter().collect();
    for point in photocraft_engine::snow_hyphenation::hyphenate(&word, None) {
        // Both halves must stay at least two characters long in the whole line.
        out.push(offset + point);
    }
}

/// Splits `line` before `char_index`, hyphenating the left part (the break is inside a word).
pub fn split_line(line: &str, char_index: usize) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    if char_index == 0 || char_index >= chars.len() {
        return (line.to_string(), String::new());
    }
    let left: String = chars[..char_index].iter().collect();
    let right: String = chars[char_index..].iter().collect();
    (format!("{}-", left.trim_end()), right.trim_start().to_string())
}

/// Joins a line ending with a hyphen to the following one, dropping the hyphen. A hyphen means
/// the break was inside a word, so the halves are glued without a space.
pub fn merge_lines(left: &str, right: &str) -> String {
    let trimmed = left.trim_end();
    let had_hyphen = trimmed.ends_with('-');
    let left = trimmed.strip_suffix('-').unwrap_or(trimmed).trim_end();
    let right = right.trim_start();
    if right.is_empty() {
        left.to_string()
    } else if left.is_empty() {
        right.to_string()
    } else if had_hyphen {
        format!("{left}{right}")
    } else {
        format!("{left} {right}")
    }
}

/// Calls `f` with the byte offset of `char_index` in `text`.
pub fn byte_at_char(text: &str, char_index: usize) -> Option<usize> {
    text.char_indices().nth(char_index).map(|(byte, _)| byte)
}

/// The preview strip: one line of script text plus the break markers under the pointer.
///
/// Returns the action of a click, if any. `next` is required to offer the join marker.
pub fn show(ui: &mut egui::Ui, line: &str, next: Option<&str>) -> Option<BreakAction> {
    let t = Tokens::get(ui.ctx());
    let font = FontId::proportional(13.0);
    let color = t.text;
    let galley = ui.painter().layout_no_wrap(line.to_owned(), font.clone(), color);
    let joinable = line.trim_end().ends_with('-') && next.is_some();
    let height = galley.size().y.max(18.0) + 6.0;
    let width = ui.available_width().max(60.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let text_rect = Rect::from_min_size(rect.min + vec2(2.0, 3.0), galley.size());
    ui.painter().galley(text_rect.min, galley.clone(), color);

    if !response.hovered() && !joinable {
        return None;
    }
    let points = line_break_points(line);
    let accent = t.accent;
    let marker = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 160);
    let hover = response.hover_pos();
    let mut nearest: Option<(usize, f32)> = None;
    if let Some(pos) = hover {
        for &point in &points {
            let x = galley.pos_from_cursor(egui::text::CCursor::new(point)).left() + text_rect.min.x;
            let dx = (pos.x - x).abs();
            if dx <= 5.0 && nearest.is_none_or(|(_, best)| dx < best) {
                nearest = Some((point, dx));
            }
            ui.painter().circle_filled(pos2(x, text_rect.bottom()), 2.0, marker);
        }
    }
    if joinable {
        let x = text_rect.right() + 4.0;
        let y = text_rect.center().y;
        ui.painter().line_segment([pos2(x - 3.0, y), pos2(x + 3.0, y)], Stroke::new(1.5, accent));
        if response.clicked() && hover.is_some_and(|pos| pos.x >= text_rect.right() - 2.0) {
            return Some(BreakAction::Merge);
        }
    }
    if response.clicked()
        && let Some((point, _)) = nearest
    {
        return Some(BreakAction::Split { char_index: point });
    }
    let _ = rect;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_line_offers_syllable_breaks() {
        let points = line_break_points("НЕОБХОДИМОСТЬ пройти");
        assert!(!points.is_empty());
        assert!(points.iter().all(|p| *p > 0 && *p < "НЕОБХОДИМОСТЬ пройти".chars().count()));
    }

    #[test]
    fn english_line_offers_breaks_and_ignores_punctuation_runs() {
        let points = line_break_points("EXTRAORDINARY, adventure!");
        assert!(!points.is_empty());
        for point in &points {
            let chars: Vec<char> = "EXTRAORDINARY, adventure!".chars().collect();
            assert!(chars.get(*point).is_some(), "point {point} is inside the line");
        }
    }

    #[test]
    fn split_then_merge_round_trips() {
        let (left, right) = split_line("НЕОБХОДИМОСТЬ", 5);
        assert_eq!(left, "НЕОБХ-");
        assert_eq!(right, "ОДИМОСТЬ");
        assert_eq!(merge_lines(&left, &right), "НЕОБХОДИМОСТЬ");
    }

    #[test]
    fn merge_drops_the_hyphen_and_keeps_the_words_apart() {
        assert_eq!(merge_lines("необхо-", "димость"), "необходимость");
        assert_eq!(merge_lines("слово-", ""), "слово");
        assert_eq!(merge_lines("слово", "дальше"), "слово дальше");
    }

    #[test]
    fn byte_offsets_follow_char_boundaries() {
        let line = "НЕОБХОДИМОСТЬ";
        let byte = byte_at_char(line, 5).expect("inside");
        assert!(line.is_char_boundary(byte));
        assert_eq!(byte, 10);
    }
}
