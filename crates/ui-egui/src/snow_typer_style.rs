//! TypeR Studio's interface tokens, ported from `app.css` (`:root`).
//!
//! The manga typesetter's panel is being moved towards TypeR's own look: these are the colours,
//! metrics and typography the original web UI uses, kept in one place so every TypeR widget draws
//! from the same palette instead of ad-hoc values.

use egui::Color32;

/// Colours and metrics of TypeR's panels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TyperStyle {
    /// `--ui`, panel chrome.
    pub ui: Color32,
    /// `--ui-2`, raised control.
    pub ui_2: Color32,
    /// `--ui-3`, pressed/hover control.
    pub ui_3: Color32,
    /// `--ui-dark`, toolbar and fields.
    pub ui_dark: Color32,
    /// `--ui-darker`, the window backdrop.
    pub ui_darker: Color32,
    /// `--line`, separators and borders.
    pub line: Color32,
    /// `--text`, body text.
    pub text: Color32,
    /// `--muted`, secondary labels.
    pub muted: Color32,
    /// `--blue`, accent and the break caret.
    pub blue: Color32,
    /// `--blue-dark`, accent pressed.
    pub blue_dark: Color32,
    /// `--warn`, the hard-cut/join state.
    pub warn: Color32,
    /// `--tool`: toolbar button size in CSS px.
    pub tool: f32,
    /// `--head`: panel header height in CSS px.
    pub head: f32,
    /// Body font size (`font: 12px/1.35` in the original).
    pub font_body: f32,
    /// Small label size (TypeR uses 10–10.5 px for counters and hints).
    pub font_small: f32,
    /// Line height factor of the original (`1.35`).
    pub line_height: f32,
}

impl Default for TyperStyle {
    fn default() -> Self {
        Self {
            ui: Color32::from_rgb(0x3a, 0x38, 0x38),
            ui_2: Color32::from_rgb(0x45, 0x43, 0x43),
            ui_3: Color32::from_rgb(0x52, 0x50, 0x50),
            ui_dark: Color32::from_rgb(0x2c, 0x2a, 0x2a),
            ui_darker: Color32::from_rgb(0x23, 0x21, 0x21),
            line: Color32::from_rgb(0x1f, 0x1d, 0x1d),
            text: Color32::from_rgb(0xe3, 0xe4, 0xe4),
            muted: Color32::from_rgb(0xa5, 0xa4, 0xa4),
            blue: Color32::from_rgb(0x28, 0x8e, 0xdf),
            blue_dark: Color32::from_rgb(0x19, 0x76, 0xd2),
            warn: Color32::from_rgb(0xe0, 0xa0, 0x30),
            tool: 42.0,
            head: 26.0,
            font_body: 12.0,
            font_small: 10.5,
            line_height: 1.35,
        }
    }
}

impl TyperStyle {
    /// The TypeR palette (independent of the editor theme: TypeR's panels are always dark).
    pub fn get(_ctx: &egui::Context) -> Self {
        Self::default()
    }

    /// Height of the body row for the given font size, following the original 1.35 line height.
    pub fn row_height(&self, size: f32) -> f32 {
        (size * self.line_height).ceil()
    }

    /// A hairline colour for separators between panel rows.
    pub fn separator(&self) -> Color32 {
        self.line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_typer_studios_root_palette() {
        let s = TyperStyle::default();
        assert_eq!(s.ui, Color32::from_rgb(58, 56, 56));
        assert_eq!(s.ui_dark, Color32::from_rgb(44, 42, 42));
        assert_eq!(s.text, Color32::from_rgb(227, 228, 228));
        assert_eq!(s.muted, Color32::from_rgb(165, 164, 164));
        assert_eq!(s.blue, Color32::from_rgb(40, 142, 223));
        assert_eq!(s.warn, Color32::from_rgb(224, 160, 48));
        assert!((s.head - 26.0).abs() < f32::EPSILON);
        assert!((s.line_height - 1.35).abs() < f32::EPSILON);
    }

    #[test]
    fn rows_follow_the_original_line_height() {
        let s = TyperStyle::default();
        assert_eq!(s.row_height(12.0), 17.0);
        assert_eq!(s.row_height(10.5), 15.0);
        assert_eq!(s.separator(), s.line);
    }
}
