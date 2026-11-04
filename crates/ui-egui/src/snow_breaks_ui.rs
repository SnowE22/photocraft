//! Interactive line-break editor preview for TypeR (phase 2), styled after TypeR Studio's Line
//! Breaks panel: a dark toolbar with the break count, the script line, hover-revealed syllable
//! marks, click to break, Alt+click for a hard cut (no hyphen), dragging a mark to move the break,
//! and a click on the join mark to merge with the next line.

use egui::{Color32, FontId, Margin, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::snow_typer_style::TyperStyle;

/// What the preview asks the panel to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakAction {
    /// Split before this character; `hyphenate` adds the trailing hyphen (Alt turns it off).
    Split { char_index: usize, hyphenate: bool },
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
        out.push(offset + point);
    }
}

/// Splits `line` before `char_index`; `hyphenate` keeps the left part with a trailing hyphen, a
/// hard cut leaves both parts as they are.
pub fn split_line(line: &str, char_index: usize, hyphenate: bool) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    if char_index == 0 || char_index >= chars.len() {
        return (line.to_string(), String::new());
    }
    let left: String = chars[..char_index].iter().collect();
    let right: String = chars[char_index..].iter().collect();
    let left = if hyphenate && !left.trim_end().ends_with('-') { format!("{}-", left.trim_end()) } else { left.trim_end().to_string() };
    (left, right.trim_start().to_string())
}

/// Joins a hyphenated line with the next one, dropping the hyphen (the parts are glued, since a
/// hyphen means the break was inside a word).
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

/// The break point closest to `char_index` (for dragging a mark to another syllable).
pub fn nearest_break(points: &[usize], char_index: usize) -> Option<usize> {
    points.iter().copied().min_by_key(|point| point.abs_diff(char_index))
}

/// Byte offset of `char_index` in `text`.
pub fn byte_at_char(text: &str, char_index: usize) -> Option<usize> {
    text.char_indices().nth(char_index).map(|(byte, _)| byte)
}

const HINT_OPEN: &str = "snow-breaks-hint";
const DRAG_ID: &str = "snow-breaks-drag";
const MARKER_HIT_PX: f32 = 6.0;

/// Draws the breaks preview and returns the requested action, styled like TypeR's panel.
pub fn show(ui: &mut egui::Ui, line: &str, next: Option<&str>) -> Option<BreakAction> {
    let style = TyperStyle::get(ui.ctx());
    let points = line_break_points(line);
    let joinable = line.trim_end().ends_with('-') && next.is_some();
    let mut action = None;
    egui::Frame::NONE
        .fill(style.ui_dark)
        .stroke(Stroke::new(1.0, style.line))
        .corner_radius(2.0)
        .inner_margin(Margin::symmetric(6, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{}", points.len())).color(style.muted).size(10.5).strong());
                ui.label(egui::RichText::new("переносов").color(style.muted).size(10.5));
                if joinable {
                    ui.label(egui::RichText::new("· клик по − склеит").color(style.warn).size(10.5));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut hint = ui.ctx().data_mut(|d| d.get_temp::<bool>(egui::Id::new(HINT_OPEN)).unwrap_or(false));
                    if ui
                        .add(egui::Button::new(egui::RichText::new("?").size(10.5).color(if hint { Color32::WHITE } else { style.muted })).frame(false))
                        .clicked()
                    {
                        hint = !hint;
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(HINT_OPEN), hint));
                    }
                    if hint {
                        ui.label(
                            egui::RichText::new("клик — дефис · Alt+клик — без дефиса · тяните маркер — двигать")
                                .color(style.muted)
                                .size(10.0),
                        );
                    }
                });
            });
            let font = FontId::proportional(13.0);
            let galley = ui.painter().layout_no_wrap(line.to_owned(), font, style.text);
            let width = ui.available_width().max(60.0);
            let height = galley.size().y.max(18.0) + 4.0;
            let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
            let text_rect = Rect::from_min_size(rect.min + vec2(2.0, 2.0), galley.size());
            ui.painter().galley(text_rect.min, galley.clone(), style.text);
            let x_of = |index: usize| galley.pos_from_cursor(egui::text::CCursor::new(index)).left() + text_rect.min.x;
            let pointer = response.hover_pos();
            let drag_key = egui::Id::new(DRAG_ID);
            let dragging: Option<usize> = ui.ctx().data(|d| d.get_temp(drag_key));
            if response.drag_started()
                && let Some(pos) = pointer
            {
                let hit = points.iter().copied().find(|point| (x_of(*point) - pos.x).abs() <= MARKER_HIT_PX);
                if let Some(point) = hit {
                    ui.ctx().data_mut(|d| d.insert_temp(drag_key, point));
                }
            }
            let active = dragging.or_else(|| {
                pointer.and_then(|pos| points.iter().copied().find(|point| (x_of(*point) - pos.x).abs() <= MARKER_HIT_PX))
            });
            if response.hovered() || dragging.is_some() {
                for &point in &points {
                    let hot = Some(point) == active;
                    let color = if hot { style.blue } else { style.muted };
                    ui.painter().circle_filled(pos2(x_of(point), text_rect.bottom() + 1.0), if hot { 2.6 } else { 2.0 }, color);
                }
            }
            // Dragging a mark: the break follows the pointer to the nearest syllable.
            let mut release_point = None;
            if dragging.is_some() {
                if let Some(pos) = pointer {
                    let cursor = galley.cursor_from_pos(pos - text_rect.min).index;
                    let cursor: usize = cursor.into();
                    release_point = nearest_break(&points, cursor);
                    if let Some(point) = release_point {
                        let x = x_of(point);
                        ui.painter().line_segment([pos2(x, text_rect.top()), pos2(x, text_rect.bottom())], Stroke::new(1.5, style.blue));
                    }
                }
                if response.drag_stopped() {
                    ui.ctx().data_mut(|d| d.remove_temp::<usize>(drag_key));
                    if let Some(point) = release_point.or(dragging) {
                        action = Some(BreakAction::Split { char_index: point, hyphenate: true });
                    }
                }
            } else if response.clicked() {
                let alt = ui.input(|i| i.modifiers.alt);
                if joinable && pointer.is_some_and(|pos| pos.x >= text_rect.right() - 2.0) {
                    action = Some(BreakAction::Merge);
                } else if let Some(point) = active {
                    action = Some(BreakAction::Split { char_index: point, hyphenate: !alt });
                }
            }
            if joinable {
                let x = text_rect.right() + 5.0;
                ui.painter().line_segment(
                    [pos2(x - 3.0, text_rect.center().y), pos2(x + 3.0, text_rect.center().y)],
                    Stroke::new(1.5, style.warn),
                );
            }
        });
    action
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
    fn split_with_and_without_hyphen_round_trips() {
        let (left, right) = split_line("НЕОБХОДИМОСТЬ", 5, true);
        assert_eq!(left, "НЕОБХ-");
        assert_eq!(right, "ОДИМОСТЬ");
        assert_eq!(merge_lines(&left, &right), "НЕОБХОДИМОСТЬ");
        let (hard_left, hard_right) = split_line("NEEDLESS", 4, false);
        assert_eq!(hard_left, "NEED");
        assert_eq!(hard_right, "LESS");
        assert_eq!(merge_lines(&hard_left, &hard_right), "NEED LESS");
    }

    #[test]
    fn merge_drops_the_hyphen_and_keeps_words_apart() {
        assert_eq!(merge_lines("необхо-", "димость"), "необходимость");
        assert_eq!(merge_lines("слово-", ""), "слово");
        assert_eq!(merge_lines("слово", "дальше"), "слово дальше");
    }

    #[test]
    fn nearest_break_finds_the_closest_syllable() {
        let points = [3, 6, 9, 12];
        assert_eq!(nearest_break(&points, 7), Some(6));
        assert_eq!(nearest_break(&points, 11), Some(12));
        assert_eq!(nearest_break(&[], 4), None);
    }

    #[test]
    fn byte_offsets_follow_char_boundaries() {
        let line = "НЕОБХОДИМОСТЬ";
        let byte = byte_at_char(line, 5).expect("inside");
        assert!(line.is_char_boundary(byte));
        assert_eq!(byte, 10);
    }
}
