use photocraft_doc::effects::{Effect, FxPaint};
use photocraft_doc::text::{AntiAlias, Caps, CharStyle, Kerning, Orientation, TextAlign, TextDirection, TextRun};
use photocraft_doc::{Document, LayerContent, TextLayer};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

const CONTOUR_PROFILE_ROWS: usize = 64;
const CONTOUR_SCAN_BUDGET: u64 = 16 * 1024 * 1024;
const DEFAULT_TYPE_FIT_MIN_PERCENT: f32 = 75.0;

/// Maximum `.tpl` input accepted by the TypeR importer.
pub const MAX_TPL_FILE_BYTES: usize = photocraft_psd::tpl::MAX_FILE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasurementSystem {
    Metric,
    #[default]
    Optical,
}

impl MeasurementSystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Metric => "metric",
            Self::Optical => "optical",
        }
    }

    pub fn is_optical(&self) -> bool {
        matches!(self, Self::Optical)
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "optical" | "opticalkern" | "visual" => Self::Optical,
            "metric" | "metrics" | "metricskern" | "font" => Self::Metric,
            _ => Self::Optical,
        }
    }
}

pub fn default_text_color() -> [u8; 4] {
    [0, 0, 0, 255]
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStylePreset {
    pub name: String,
    pub folder: Option<String>,
    pub font_family: String,
    pub font_postscript: String,
    pub font_style: String,
    pub font_size: f32,
    pub leading: f32,
    pub tracking: i32,
    pub horizontal_scale: f32,
    pub bold: bool,
    pub italic: bool,
    pub all_caps: bool,
    pub stroke_width: f32,
    pub stroke_color: [u8; 4],
    #[serde(default = "default_text_color")]
    pub text_color: [u8; 4],
    #[serde(default)]
    pub measurement_system: MeasurementSystem,
    #[serde(default = "default_true")]
    pub hanging_punctuation: bool,
    #[serde(default = "default_anti_alias")]
    pub anti_alias: String,
    #[serde(default)]
    pub script_prefixes: Vec<String>,
    /// TypeR's "punctuation-only" size multiplier: 1.0 is off, 2.0/3.0 enlarge a line
    /// such as `?!` and its leading. Ordinary lines are never affected.
    #[serde(default = "default_punctuation_scale")]
    pub punctuation_only_scale: f32,
    /// `horizontal` or `vertical` type.
    #[serde(default = "default_orientation")]
    pub orientation: String,
    /// Paragraph alignment: `left`, `center`, `right`, `justify`, `justifyCenter`,
    /// `justifyRight` or `justifyAll`.
    #[serde(default = "default_alignment")]
    pub alignment: String,
    /// Paragraph direction: `auto`, `ltr` or `rtl`.
    #[serde(default = "default_direction")]
    pub direction: String,
    #[serde(default)]
    pub first_line_indent: f32,
    #[serde(default)]
    pub start_indent: f32,
    #[serde(default)]
    pub end_indent: f32,
    #[serde(default)]
    pub space_before: f32,
    #[serde(default)]
    pub space_after: f32,
    #[serde(default = "default_true")]
    pub hyphenate: bool,
}

fn default_anti_alias() -> String {
    "crisp".to_string()
}

fn default_punctuation_scale() -> f32 {
    1.0
}

fn default_orientation() -> String {
    "horizontal".to_string()
}

fn default_alignment() -> String {
    "center".to_string()
}

fn default_direction() -> String {
    "auto".to_string()
}

fn text_align_name(align: TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
        TextAlign::JustifyLeft => "justify",
        TextAlign::JustifyCenter => "justifyCenter",
        TextAlign::JustifyRight => "justifyRight",
        TextAlign::JustifyAll => "justifyAll",
    }
}

fn text_direction_name(direction: TextDirection) -> &'static str {
    match direction {
        TextDirection::Auto => "auto",
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    }
}

fn orientation_name(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::Horizontal => "horizontal",
        Orientation::Vertical => "vertical",
    }
}

fn normalize_alignment(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => "left",
        "right" | "end" | "rght" => "right",
        "center" | "centre" | "cntr" | "nrml" => "center",
        "justify" | "justifyleft" | "justifyfull" | "jstf" => "justify",
        "justifycenter" => "justifyCenter",
        "justifyright" => "justifyRight",
        "justifyall" | "jsta" => "justifyAll",
        _ => "center",
    }
    .to_string()
}

fn normalize_orientation(value: &str) -> String {
    if value.trim().eq_ignore_ascii_case("vertical") || value.trim().eq_ignore_ascii_case("vrtc") {
        "vertical".to_string()
    } else {
        "horizontal".to_string()
    }
}

fn normalize_direction(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "ltr" | "lefttoright" => "ltr",
        "rtl" | "righttoleft" => "rtl",
        _ => "auto",
    }
    .to_string()
}

impl Default for TextStylePreset {
    fn default() -> Self {
        Self {
            name: "Дефолт".to_string(),
            folder: Some("Манга".to_string()),
            font_family: "Kurai Voice 2".to_string(),
            font_postscript: "KuraiVoice2".to_string(),
            font_style: "Regular".to_string(),
            font_size: 26.0,
            leading: 30.0,
            tracking: 0,
            horizontal_scale: 100.0,
            bold: false,
            italic: false,
            all_caps: true,
            stroke_width: 2.0,
            stroke_color: [255, 255, 255, 255],
            text_color: [0, 0, 0, 255],
            measurement_system: MeasurementSystem::Optical,
            hanging_punctuation: true,
            anti_alias: "crisp".to_string(),
            script_prefixes: Vec::new(),
            punctuation_only_scale: 1.0,
            orientation: default_orientation(),
            alignment: default_alignment(),
            direction: default_direction(),
            first_line_indent: 0.0,
            start_indent: 0.0,
            end_indent: 0.0,
            space_before: 0.0,
            space_after: 0.0,
            hyphenate: true,
        }
    }
}

impl TextStylePreset {
    /// Синхронизирует интерлиньяж (leading) пропорционально изменению кегля шрифта (Line Spacing Sync).
    pub fn sync_leading_with_size(&mut self, new_size: f32) {
        let clamped = new_size.clamp(4.0, 500.0);
        if self.font_size > 0.0 {
            let ratio = clamped / self.font_size;
            self.leading = ((self.leading * ratio) * 10.0).round() / 10.0;
        }
        self.font_size = clamped;
    }

    pub fn from_text_layer(name: &str, text: &TextLayer, effects: &photocraft_doc::Effects) -> (Self, bool) {
        let run = text.char_runs().into_iter().max_by_key(|run| run.len);
        let style = run.as_ref().map(|run| &run.style);
        let size = style.map_or(text.size_pt, |style| style.size_pt);
        let size = if size.is_finite() && size > 0.0 { size.clamp(0.1, 1296.0) } else { 12.0 };
        let paragraph = text.paragraph_runs().into_iter().next();
        let auto_leading = paragraph.as_ref().map_or(1.2, |run| run.style.auto_leading);
        let leading = style
            .and_then(|style| style.leading_pt)
            .filter(|leading| leading.is_finite() && *leading > 0.0)
            .unwrap_or_else(|| {
                if auto_leading.is_finite() && auto_leading > 0.0 {
                    size * auto_leading
                } else {
                    size * 1.2
                }
            });
        let mut preset = Self {
            name: if name.trim().is_empty() { "PSD text layer".into() } else { name.into() },
            folder: None,
            font_family: style.map_or_else(|| text.font_family.clone(), |style| {
                if style.font_family.is_empty() { text.font_family.clone() } else { style.font_family.clone() }
            }),
            font_postscript: style.and_then(|style| style.postscript_name.clone()).unwrap_or_default(),
            font_style: style.map_or_else(String::new, |style| style.font_style.clone()),
            font_size: size,
            leading: if leading.is_finite() { leading.clamp(0.1, 1296.0) } else { size * 1.2 },
            tracking: style.map_or(0, |style| style.tracking.round().clamp(-1000.0, 10000.0) as i32),
            horizontal_scale: style.map_or(100.0, |style| {
                if style.horizontal_scale.is_finite() { (style.horizontal_scale * 100.0).clamp(1.0, 1000.0) } else { 100.0 }
            }),
            bold: style.is_some_and(|style| style.weight >= 600 || style.faux_bold),
            italic: style.is_some_and(|style| style.italic || style.faux_italic),
            all_caps: style.is_some_and(|style| style.caps == Caps::AllCaps),
            stroke_width: 0.0,
            stroke_color: [255, 255, 255, 255],
            text_color: style.map_or_else(|| text.color.to_rgba8(), |style| style.color.to_rgba8()),
            measurement_system: if style.is_some_and(|style| style.kerning == Kerning::Optical) {
                MeasurementSystem::Optical
            } else {
                MeasurementSystem::Metric
            },
            hanging_punctuation: true,
            anti_alias: match text.antialias {
                AntiAlias::None => "none",
                AntiAlias::Sharp => "sharp",
                AntiAlias::Crisp => "crisp",
                AntiAlias::Strong => "strong",
                AntiAlias::Smooth => "smooth",
                AntiAlias::Windows => "windows",
                AntiAlias::WindowsLcd => "windowsLcd",
            }
            .into(),
            script_prefixes: Vec::new(),
            punctuation_only_scale: 1.0,
            orientation: orientation_name(text.orientation).to_string(),
            alignment: paragraph.as_ref().map_or_else(default_alignment, |run| text_align_name(run.style.align).to_string()),
            direction: paragraph.as_ref().map_or_else(default_direction, |run| text_direction_name(run.style.direction).to_string()),
            first_line_indent: paragraph.as_ref().map_or(0.0, |run| run.style.first_line_indent_pt),
            start_indent: paragraph.as_ref().map_or(0.0, |run| run.style.start_indent_pt),
            end_indent: paragraph.as_ref().map_or(0.0, |run| run.style.end_indent_pt),
            space_before: paragraph.as_ref().map_or(0.0, |run| run.style.space_before_pt),
            space_after: paragraph.as_ref().map_or(0.0, |run| run.style.space_after_pt),
            hyphenate: paragraph.as_ref().is_some_and(|run| run.style.hyphenate),
        };
        let mut unsupported_stroke = false;
        if effects.enabled {
            for effect in &effects.items {
                let Effect::Stroke(stroke) = effect else { continue };
                if !stroke.common.enabled {
                    continue;
                }
                match &stroke.paint {
                    FxPaint::Color(color)
                        if stroke.position == photocraft_doc::effects::StrokePosition::Outside
                            && stroke.common.blend == photocraft_doc::BlendMode::Normal
                            && preset.stroke_width == 0.0 =>
                    {
                        preset.stroke_width = if stroke.size.is_finite() { stroke.size.clamp(0.0, 2500.0) } else { 0.0 };
                        preset.stroke_color = color.to_rgba8();
                        let alpha = (f32::from(preset.stroke_color[3]) * stroke.common.opacity.clamp(0.0, 1.0)).round();
                        preset.stroke_color[3] = alpha as u8;
                    }
                    FxPaint::Color(_) | FxPaint::Gradient(_) | FxPaint::Pattern { .. } => unsupported_stroke = true,
                }
            }
        }
        (preset, unsupported_stroke)
    }

    fn same_typography(&self, other: &Self) -> bool {
        self.font_family == other.font_family
            && self.font_postscript == other.font_postscript
            && self.font_style == other.font_style
            && self.font_size == other.font_size
            && self.leading == other.leading
            && self.tracking == other.tracking
            && self.horizontal_scale == other.horizontal_scale
            && self.bold == other.bold
            && self.italic == other.italic
            && self.all_caps == other.all_caps
            && self.stroke_width == other.stroke_width
            && self.stroke_color == other.stroke_color
            && self.text_color == other.text_color
            && self.measurement_system == other.measurement_system
            && self.anti_alias == other.anti_alias
            && self.punctuation_only_scale == other.punctuation_only_scale
            && self.orientation == other.orientation
            && self.alignment == other.alignment
    }

    /// Scales size and leading for a line made only of punctuation, as TypeR's
    /// `punctuationOnlyScale` does. Returns the applied multiplier (1.0 when unchanged).
    pub fn scale_for_punctuation_only(&mut self, text: &str) -> f32 {
        if self.punctuation_only_scale <= 1.0 || !is_punctuation_only_text(text) {
            return 1.0;
        }
        let scale = self.punctuation_only_scale.clamp(1.0, 4.0);
        self.font_size = (self.font_size * scale).clamp(0.1, 1296.0);
        self.leading = (self.leading * scale).clamp(0.1, 1296.0);
        scale
    }
}

/// True when the text has at least one visible character and every visible character is
/// punctuation. Mirrors TypeR's punctuation-only detection for shouting marks such as `?!`.
pub fn is_punctuation_only_text(text: &str) -> bool {
    let mut saw_punctuation = false;
    for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
        saw_punctuation = true;
        let punctuation = ch.is_ascii_punctuation()
            || matches!(
                ch,
                '¡' | '¿' | '«' | '»' | '–' | '—' | '‘' | '’' | '“' | '”' | '…' | '‽' | '⁇' | '⁈' | '⁉' | '、' | '。' | '〜' | '！' | '？' | '～'
            );
        if !punctuation {
            return false;
        }
    }
    saw_punctuation
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptLine {
    pub raw: String,
    pub text: String,
    pub is_applied: bool,
    pub page: usize,
    #[serde(default)]
    pub matched_preset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypesetterOptions {
    pub auto_page_switch: bool,
    pub line_spacing_sync: bool,
    pub strip_character_names: bool,
    pub strip_dash_prefix: bool,
    pub optical_baseline_factor: f32,
    pub hanging_punctuation: bool,
    pub match_style_prefixes: bool,
    pub match_prefixes_across_folders: bool,
    #[serde(default = "default_type_fit_min_percent")]
    pub minimum_fit_scale_percent: f32,
    /// Paste the current script line as soon as a selection is made.
    #[serde(default)]
    pub paste_on_click: bool,
    /// Fill the balloon on a `TypeR Clean` layer before pasting, hiding the original lettering.
    #[serde(default)]
    pub auto_clean: bool,
}

fn default_type_fit_min_percent() -> f32 {
    DEFAULT_TYPE_FIT_MIN_PERCENT
}

impl Default for TypesetterOptions {
    fn default() -> Self {
        Self {
            auto_page_switch: true,
            line_spacing_sync: true,
            strip_character_names: true,
            strip_dash_prefix: true,
            optical_baseline_factor: 0.35,
            hanging_punctuation: true,
            match_style_prefixes: true,
            match_prefixes_across_folders: false,
            minimum_fit_scale_percent: DEFAULT_TYPE_FIT_MIN_PERCENT,
            paste_on_click: false,
            auto_clean: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypesetterState {
    pub lines: Vec<ScriptLine>,
    pub current_line_idx: usize,
    pub current_page: usize,
    pub presets: Vec<TextStylePreset>,
    pub selected_preset_idx: usize,
    pub options: TypesetterOptions,
}

impl Default for TypesetterState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            current_line_idx: 0,
            current_page: 1,
            presets: vec![
                TextStylePreset::default(),
                TextStylePreset {
                    name: "Крик / SFX".to_string(),
                    folder: Some("Звуки".to_string()),
                    font_family: "CC Wild Words".to_string(),
                    font_postscript: "CCWildWords".to_string(),
                    font_size: 24.0,
                    leading: 26.0,
                    tracking: 20,
                    bold: true,
                    all_caps: true,
                    stroke_width: 4.0,
                    ..TextStylePreset::default()
                },
                TextStylePreset {
                    name: "Мысли".to_string(),
                    folder: Some("Диалоги".to_string()),
                    font_family: "Comic Book".to_string(),
                    font_postscript: "ComicBook".to_string(),
                    font_size: 14.0,
                    leading: 16.0,
                    italic: true,
                    all_caps: false,
                    stroke_width: 0.0,
                    ..TextStylePreset::default()
                },
            ],
            selected_preset_idx: 0,
            options: TypesetterOptions::default(),
        }
    }
}

/// Парсер цвета для форматов TypeR и Photoshop CEP:
/// - Объект RGBA: `{"r": .., "g": .., "b": ..}` или `{"red": .., "green": .., "blue": ..}` (0..255 или 0.0..1.0)
/// - Объект Grayscale: `{"gray": 100}` (0..100% или 0..255)
/// - Строка Hex: `"#RRGGBB"` или `"#RRGGBBAA"`
/// - Массив: `[r, g, b]` или `[r, g, b, a]`
pub fn parse_color_value(val: &serde_json::Value) -> Option<[u8; 4]> {
    match val {
        serde_json::Value::Object(map) => {
            let r_val = map.get("r").or_else(|| map.get("red"));
            let g_val = map.get("g").or_else(|| map.get("green"));
            let b_val = map.get("b").or_else(|| map.get("blue"));

            if let (Some(r), Some(g), Some(b)) = (r_val, g_val, b_val) {
                let to_u8 = |v: &serde_json::Value| -> u8 {
                    if let Some(n) = v.as_u64() {
                        n.min(255) as u8
                    } else if let Some(f) = v.as_f64() {
                        if f <= 1.0 && f > 0.0 {
                            (f * 255.0).round().clamp(0.0, 255.0) as u8
                        } else {
                            f.round().clamp(0.0, 255.0) as u8
                        }
                    } else {
                        0
                    }
                };
                let r_u8 = to_u8(r);
                let g_u8 = to_u8(g);
                let b_u8 = to_u8(b);

                let a_u8 = if let Some(a) = map.get("a").or_else(|| map.get("alpha")) {
                    if let Some(n) = a.as_u64() {
                        n.min(255) as u8
                    } else if let Some(f) = a.as_f64() {
                        if f <= 1.0 {
                            (f * 255.0).round().clamp(0.0, 255.0) as u8
                        } else {
                            f.round().clamp(0.0, 255.0) as u8
                        }
                    } else {
                        255
                    }
                } else if let Some(op) = map.get("opacity").and_then(|v| v.as_f64()) {
                    ((op / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    255
                };

                return Some([r_u8, g_u8, b_u8, a_u8]);
            }

            if let Some(g) = map.get("gray").or_else(|| map.get("grey")).and_then(|v| v.as_f64()) {
                let v = if g <= 1.0 && g > 0.0 {
                    (g * 255.0).round().clamp(0.0, 255.0) as u8
                } else if g <= 100.0 {
                    ((g / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    g.round().clamp(0.0, 255.0) as u8
                };
                let a = if let Some(op) = map.get("opacity").and_then(|v| v.as_f64()) {
                    ((op / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    255
                };
                return Some([v, v, v, a]);
            }

            None
        }
        serde_json::Value::Array(arr) if arr.len() >= 3 => {
            let to_u8 = |v: &serde_json::Value| -> u8 {
                if let Some(n) = v.as_u64() {
                    n.min(255) as u8
                } else if let Some(f) = v.as_f64() {
                    if f <= 1.0 && f > 0.0 {
                        (f * 255.0).round().clamp(0.0, 255.0) as u8
                    } else {
                        f.round().clamp(0.0, 255.0) as u8
                    }
                } else {
                    0
                }
            };
            let r = to_u8(&arr[0]);
            let g = to_u8(&arr[1]);
            let b = to_u8(&arr[2]);
            let a = if arr.len() >= 4 { to_u8(&arr[3]) } else { 255 };
            Some([r, g, b, a])
        }
        serde_json::Value::String(s) => {
            let s = s.trim_start_matches('#');
            if s.len() == 6 {
                let r = u8::from_str_radix(s.get(0..2)?, 16).ok()?;
                let g = u8::from_str_radix(s.get(2..4)?, 16).ok()?;
                let b = u8::from_str_radix(s.get(4..6)?, 16).ok()?;
                Some([r, g, b, 255])
            } else if s.len() == 8 {
                let r = u8::from_str_radix(s.get(0..2)?, 16).ok()?;
                let g = u8::from_str_radix(s.get(2..4)?, 16).ok()?;
                let b = u8::from_str_radix(s.get(4..6)?, 16).ok()?;
                let a = u8::from_str_radix(s.get(6..8)?, 16).ok()?;
                Some([r, g, b, a])
            } else {
                None
            }
        }
        _ => None,
    }
}

fn tpl_descriptor<'a>(descriptor: &'a photocraft_psd::Descriptor, key: &str) -> Option<&'a photocraft_psd::Descriptor> {
    match descriptor.get(key)? {
        photocraft_psd::descriptor::Value::Descriptor(value) | photocraft_psd::descriptor::Value::GlobalObject(value) => Some(value),
        _ => None,
    }
}

fn tpl_text_any(descriptor: &photocraft_psd::Descriptor, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match descriptor.get(key)? {
        photocraft_psd::descriptor::Value::Text(value) => {
            let value = value.to_string_lossy();
            (!value.trim().is_empty()).then_some(value)
        }
        photocraft_psd::descriptor::Value::Enumerated { value, .. } => Some(String::from_utf8_lossy(value.as_bytes()).into_owned()),
        _ => None,
    })
}

fn tpl_number_any(descriptor: &photocraft_psd::Descriptor, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| match descriptor.get(key)? {
        photocraft_psd::descriptor::Value::Double(value) => Some(*value),
        photocraft_psd::descriptor::Value::UnitFloat { value, .. } => Some(*value),
        photocraft_psd::descriptor::Value::Integer(value) => Some(f64::from(*value)),
        photocraft_psd::descriptor::Value::LargeInteger(value) => Some(*value as f64),
        _ => None,
    })
}

fn tpl_bool_any(descriptor: &photocraft_psd::Descriptor, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| match descriptor.get(key)? {
        photocraft_psd::descriptor::Value::Boolean(value) => Some(*value),
        _ => None,
    })
}

fn tpl_rgb(descriptor: &photocraft_psd::Descriptor) -> Option<[u8; 4]> {
    let color = tpl_descriptor(descriptor, "Clr ")?;
    let rgb = tpl_descriptor(color, "RGBC").unwrap_or(color);
    // Photoshop uses `Rd  `/`Grn `/`Bl  ` in the measured files. Some writers emit
    // 0..1 `redFloat`/`greenFloat`/`blueFloat` keys instead; accept both rather than
    // silently dropping the colour.
    let component = |integer_keys: &[&str], float_keys: &[&str]| -> Option<u8> {
        let value = if let Some(value) = tpl_number_any(rgb, float_keys) {
            if value.is_finite() && (0.0..=1.0).contains(&value) { value * 255.0 } else { value }
        } else {
            tpl_number_any(rgb, integer_keys)?
        };
        value.is_finite().then(|| value.round().clamp(0.0, 255.0) as u8)
    };
    Some([
        component(&["Rd  ", "red"], &["redFloat"])?,
        component(&["Grn ", "green"], &["greenFloat"])?,
        component(&["Bl  ", "blue"], &["blueFloat"])?,
        255,
    ])
}

fn tpl_display_name(name: &str) -> String {
    // Adobe's own presets use a localization key: "$$$/Presets/Tools/Key=Display name".
    if name.starts_with("$$$/") {
        name.split_once('=').map_or_else(|| name.to_owned(), |(_, display)| display.to_owned())
    } else {
        name.to_owned()
    }
}

/// What a `.tpl` import actually did, so the UI can report skips instead of silently
/// presenting a partial library as complete.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TplImportReport {
    /// Text presets converted into TypeR styles.
    pub imported: usize,
    /// Presets not imported (other tools, incomplete text data).
    pub skipped: usize,
    /// Subset of `imported`: vertical text presets, imported with vertical orientation.
    pub vertical_imported: usize,
}

impl TypesetterState {
    /// Extracts distinct editable presets from non-empty type layers in the active document.
    /// The count of effects that cannot fit the preset model is returned for an explicit UI warning.
    pub fn scan_document_text_styles(&mut self, doc: &Document) -> (usize, usize) {
        let mut scanned = Vec::<TextStylePreset>::new();
        let mut unsupported_effects = 0usize;
        for (_, _, layer) in doc.walk() {
            let LayerContent::Text(text) = &layer.content else { continue };
            if text.text.trim().is_empty() {
                continue;
            }
            let (preset, unsupported) = TextStylePreset::from_text_layer(&layer.name, text, &layer.effects);
            unsupported_effects += usize::from(unsupported);
            if !self.presets.iter().chain(scanned.iter()).any(|existing| existing.same_typography(&preset)) {
                scanned.push(preset);
            }
        }
        let added = scanned.len();
        if added > 0 {
            let first_added = self.presets.len();
            self.presets.extend(scanned);
            self.selected_preset_idx = first_added;
        }
        (added, unsupported_effects)
    }

    /// Imports Photoshop `.tpl` text-tool presets into a folder named after the source file.
    ///
    /// Text presets are recognised by their `textToolCharacterOptions` descriptor, not by a
    /// hard-coded tool class: Photoshop uses several Type tool classes, and non-text presets
    /// carry no character options. Vertical text presets are imported with vertical
    /// orientation and counted separately. Tool presets store sizes in points, so no
    /// ruler-unit conversion is applied.
    pub fn import_tool_presets_tpl(&mut self, data: &[u8], folder: &str) -> Result<TplImportReport, String> {
        let file = photocraft_psd::tpl::parse(data).map_err(|error| format!("Photoshop TPL: {error}"))?;
        let folder = if folder.trim().is_empty() { "Photoshop TPL" } else { folder.trim() };
        let mut imported = Vec::new();
        let mut report = TplImportReport::default();

        for tool in file.presets {
            let character = tpl_descriptor(&tool.settings, "textToolCharacterOptions")
                .and_then(|options| tpl_descriptor(options, "TxtS").or_else(|| tpl_descriptor(options, "textStyle")));
            let Some(character) = character else {
                report.skipped = report.skipped.saturating_add(1);
                continue;
            };
            let font_family = tpl_text_any(character, &["FntN", "fontName"]).filter(|name| !name.trim().is_empty());
            let font_postscript = tpl_text_any(character, &["fontPostScriptName"]).filter(|name| !name.trim().is_empty());
            let Some(font_size) = tpl_number_any(character, &["Sz  ", "size"]).filter(|value| value.is_finite() && *value > 0.0) else {
                report.skipped = report.skipped.saturating_add(1);
                continue;
            };
            if font_family.is_none() && font_postscript.is_none() {
                report.skipped = report.skipped.saturating_add(1);
                continue;
            }
            let paragraph_options = tpl_descriptor(&tool.settings, "textToolParagraphOptions");
            let paragraph_style = paragraph_options.and_then(|options| tpl_descriptor(options, "paragraphStyle"));
            let orientation = paragraph_options.and_then(|options| tpl_text_any(options, &["Ornt"])).unwrap_or_default();
            let orientation = normalize_orientation(&orientation);

            let font_size = (font_size as f32).clamp(0.1, 1296.0);
            let auto_leading = tpl_bool_any(character, &["autoLeading"]);
            let leading = match tpl_number_any(character, &["Ldng", "leading"]) {
                Some(value) if value.is_finite() && value > 0.0 && auto_leading != Some(true) => (value as f32).clamp(0.1, 1296.0),
                _ => font_size * 1.2,
            };
            let tracking = tpl_number_any(character, &["Trck", "tracking"])
                .filter(|value| value.is_finite())
                .map_or(0, |value| value.round().clamp(-1000.0, 10000.0) as i32);
            let horizontal_scale = tpl_number_any(character, &["HrzS", "horizontalScale"])
                .filter(|value| value.is_finite() && *value > 0.0)
                .map_or(100.0, |value| (value as f32).clamp(1.0, 1000.0));
            let font_style = tpl_text_any(character, &["FntS", "fontStyleName"]).unwrap_or_else(|| "Regular".into());
            let caps = tpl_text_any(character, &["fontCaps"]).unwrap_or_default();
            let kerning = tpl_text_any(character, &["AtKr", "autoKern"]).unwrap_or_default();
            let font_family = font_family.or_else(|| font_postscript.clone()).unwrap_or_default();
            // Measured Photoshop 27.7 values; the old 0..4 guess mapped 2/3/4 to the wrong faces.
            let anti_alias = match tpl_number_any(character, &["AntA"]) {
                Some(value) if value.is_finite() => match value.round() as i64 {
                    0 => "none",
                    1 => "crisp",
                    2 => "strong",
                    8 => "smooth",
                    16 => "sharp",
                    _ => "smooth",
                },
                _ => "smooth",
            };
            let paragraph_number = |key: &str| {
                paragraph_style
                    .and_then(|style| tpl_number_any(style, &[key]))
                    .filter(|value| value.is_finite())
                    .map_or(0.0, |value| value.clamp(-5000.0, 5000.0) as f32)
            };
            let alignment = paragraph_style
                .and_then(|style| tpl_text_any(style, &["Algn", "alignment"]))
                .map_or_else(default_alignment, |value| normalize_alignment(&value));
            let style_lower = font_style.to_ascii_lowercase();
            imported.push(TextStylePreset {
                name: tpl_display_name(&tool.name),
                folder: Some(folder.to_owned()),
                font_postscript: font_postscript.unwrap_or_default(),
                font_family,
                bold: tpl_bool_any(character, &["syntheticBold", "FauxBold"]).unwrap_or(false) || style_lower.contains("bold"),
                italic: tpl_bool_any(character, &["syntheticItalic", "FauxItalic"]).unwrap_or(false)
                    || style_lower.contains("italic")
                    || style_lower.contains("oblique"),
                font_style,
                font_size,
                leading,
                tracking,
                horizontal_scale,
                all_caps: caps.eq_ignore_ascii_case("allcaps") || caps.eq_ignore_ascii_case("all"),
                text_color: tpl_rgb(character).unwrap_or([0, 0, 0, 255]),
                measurement_system: if kerning.to_ascii_lowercase().contains("optical") { MeasurementSystem::Optical } else { MeasurementSystem::Metric },
                stroke_width: 0.0,
                anti_alias: anti_alias.into(),
                orientation: orientation.clone(),
                alignment,
                direction: default_direction(),
                first_line_indent: paragraph_number("firstLineIndent"),
                start_indent: paragraph_number("startIndent"),
                end_indent: paragraph_number("endIndent"),
                space_before: paragraph_number("spaceBefore"),
                space_after: paragraph_number("spaceAfter"),
                hyphenate: paragraph_style
                    .and_then(|style| tpl_bool_any(style, &["hyphenate"]))
                    .unwrap_or(false),
                ..TextStylePreset::default()
            });
            if orientation == "vertical" {
                report.vertical_imported = report.vertical_imported.saturating_add(1);
            }
        }

        if imported.is_empty() {
            return Err(format!("В файле нет поддерживаемых текстовых пресетов (пропущено: {})", report.skipped));
        }
        report.imported = imported.len();
        let first_imported = self.presets.len();
        self.presets.extend(imported);
        self.selected_preset_idx = first_imported;
        self.recompute_script_style_matches();
        Ok(report)
    }

    /// Импортирует стили из JSON-строки (полная поддержка структуры TypeR / Photoshop CEP).
    pub fn import_styles_json(&mut self, json_str: &str) -> Result<usize, String> {
        let val: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| format!("Ошибка парсинга JSON: {}", e))?;

        // 1. Построение словаря папок: id -> название папки
        let mut folders_map = std::collections::HashMap::new();
        if let Some(folders) = val.get("folders").and_then(|f| f.as_array()) {
            for f in folders {
                if let (Some(id), Some(name)) = (
                    f.get("id").and_then(|v| v.as_str()),
                    f.get("name").and_then(|v| v.as_str()),
                ) {
                    folders_map.insert(id.to_string(), name.to_string());
                }
            }
        }

        // 2. Получение массива стилей
        let items = if let Some(arr) = val.as_array() {
            arr.clone()
        } else if let Some(styles) = val.get("styles").and_then(|s| s.as_array()) {
            styles.clone()
        } else {
            return Err("JSON не содержит массив стилей или ключ 'styles'".into());
        };

        let mut count = 0;
        for item in items {
            let raw_name = item.get("name")
                .or_else(|| item.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Новый стиль");

            let folder_id = item.get("folder").and_then(|v| v.as_str());
            let folder_name = folder_id.and_then(|id| folders_map.get(id)).cloned();
            let name = raw_name.to_string();

            // Разрешение вложенного пути TypeR: textProps -> layerText -> textStyleRange[0] -> textStyle
            let text_style = item.pointer("/textProps/layerText/textStyleRange/0/textStyle");

            let font_family = text_style
                .and_then(|ts| ts.get("fontName"))
                .or_else(|| item.get("font_family"))
                .or_else(|| item.get("fontFamily"))
                .and_then(|v| v.as_str())
                .unwrap_or("Kurai Voice 2")
                .to_string();

            let font_postscript = text_style
                .and_then(|ts| ts.get("fontPostScriptName"))
                .and_then(|v| v.as_str())
                .unwrap_or(&font_family)
                .to_string();

            let font_style = text_style
                .and_then(|ts| ts.get("fontStyleName"))
                .and_then(|v| v.as_str())
                .unwrap_or("Regular")
                .to_string();

            let font_size = text_style
                .and_then(|ts| ts.get("size"))
                .or_else(|| item.get("size"))
                .and_then(|v| v.as_f64())
                .unwrap_or(24.0) as f32;

            let auto_leading_pct = item.pointer("/textProps/layerText/paragraphStyleRange/0/paragraphStyle/autoLeadingPercentage")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.2) as f32;

            let leading = text_style
                .and_then(|ts| ts.get("leading"))
                .or_else(|| item.get("leading"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
                .unwrap_or_else(|| (font_size * auto_leading_pct).round());

            let tracking = text_style
                .and_then(|ts| ts.get("tracking"))
                .or_else(|| item.get("tracking"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;

            let horizontal_scale = text_style
                .and_then(|ts| ts.get("horizontalScale"))
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0) as f32;

            let synthetic_bold = text_style
                .and_then(|ts| ts.get("syntheticBold"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let synthetic_italic = text_style
                .and_then(|ts| ts.get("syntheticItalic"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let is_bold_style = font_style.to_lowercase().contains("bold");
            let is_italic_style = font_style.to_lowercase().contains("italic") || font_style.to_lowercase().contains("oblique");

            let bold = synthetic_bold || is_bold_style;
            let italic = synthetic_italic || is_italic_style;

            let all_caps = text_style
                .and_then(|ts| ts.get("fontCaps"))
                .and_then(|v| v.as_str())
                .map(|caps| caps == "allCaps" || caps == "all")
                .unwrap_or(false);

            let (stroke_width, stroke_color) = if let Some(stroke_obj) = item.get("stroke").and_then(|v| v.as_object()) {
                let enabled = stroke_obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                let size = stroke_obj.get("size").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                let width = if enabled { size } else { 0.0 };
                let color = if let Some(c) = stroke_obj.get("color") {
                    parse_color_value(c).unwrap_or([255, 255, 255, 255])
                } else {
                    [255, 255, 255, 255]
                };
                (width, color)
            } else if let Some(stroke_val) = item.get("stroke_width").or_else(|| item.get("stroke")) {
                let width = stroke_val.as_f64().unwrap_or(0.0) as f32;
                let color = if let Some(c) = item.get("strokeColor") {
                    parse_color_value(c).unwrap_or([255, 255, 255, 255])
                } else {
                    [255, 255, 255, 255]
                };
                (width, color)
            } else {
                (0.0, [255, 255, 255, 255])
            };

            let text_color = if let Some(c) = item.get("textColorRgba").or_else(|| item.get("textColor")).or_else(|| item.get("text_color")) {
                parse_color_value(c)
            } else if let Some(c) = text_style.and_then(|ts| ts.get("color")) {
                let mut col = parse_color_value(c);
                if let Some(ref mut col) = col
                    && let Some(op) = text_style.and_then(|ts| ts.get("opacity")).or_else(|| item.get("opacity")).and_then(|v| v.as_f64())
                    && (0.0..=100.0).contains(&op)
                {
                    col[3] = ((op / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                col
            } else if let Some(c) = item.get("color") {
                parse_color_value(c)
            } else {
                None
            }.unwrap_or([0, 0, 0, 255]);

            let measurement_system = if let Some(kern) = text_style.and_then(|ts| ts.get("autoKern")).and_then(|v| v.as_str()) {
                MeasurementSystem::from_str_loose(kern)
            } else if let Some(ms) = item.get("measurementSystem")
                .or_else(|| item.get("measurement_system"))
                .or_else(|| text_style.and_then(|ts| ts.get("measurementSystem")))
                .and_then(|v| v.as_str())
            {
                MeasurementSystem::from_str_loose(ms)
            } else if let Some(kern) = text_style.and_then(|ts| ts.get("kerning")).and_then(|v| v.as_str()) {
                MeasurementSystem::from_str_loose(kern)
            } else {
                MeasurementSystem::Optical
            };

            let para_style = item.get("textProps")
                .and_then(|tp| tp.get("layerText"))
                .and_then(|lt| lt.get("paragraphStyleRange"))
                .and_then(|psr| psr.as_array())
                .and_then(|arr| arr.first())
                .and_then(|ps| ps.get("paragraphStyle"));

            let hanging_punctuation = if let Some(hr) = para_style.and_then(|ps| ps.get("hangingRoman")).and_then(|v| v.as_bool()) {
                hr
            } else {
                item.get("hangingPunctuation").or_else(|| item.get("hanging_punctuation")).and_then(|v| v.as_bool()).unwrap_or(true)
            };

            let anti_alias = if let Some(aa) = item.get("antiAlias").or_else(|| item.get("anti_alias")).and_then(|v| v.as_str()) {
                aa.to_string()
            } else if let Some(aa) = text_style.and_then(|ts| ts.get("antiAlias")).and_then(|v| v.as_str()) {
                aa.to_string()
            } else {
                "crisp".to_string()
            };

            let script_prefixes = item
                .get("scriptPrefixes")
                .or_else(|| item.get("prefixes"))
                .or_else(|| item.get("prefix"))
                .map(|value| match value {
                    serde_json::Value::Array(values) => values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|prefix| !prefix.is_empty())
                        .map(str::to_string)
                        .collect(),
                    serde_json::Value::String(prefix) => prefix
                        .split(',')
                        .map(str::trim)
                        .filter(|prefix| !prefix.is_empty())
                        .map(str::to_string)
                        .collect(),
                    _ => Vec::new(),
                })
                .unwrap_or_default();
            let punctuation_only_scale = item
                .get("punctuationOnlyScale")
                .or_else(|| item.get("punctuation_only_scale"))
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite() && *value > 0.0)
                .map_or(1.0, |value| (value as f32).clamp(1.0, 4.0));
            let orientation = item
                .get("orientation")
                .and_then(serde_json::Value::as_str)
                .or_else(|| item.pointer("/textProps/layerText/orientation").and_then(serde_json::Value::as_str))
                .map_or_else(default_orientation, normalize_orientation);
            let alignment = item
                .get("alignment")
                .or_else(|| item.get("align"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| para_style.and_then(|style| style.get("align")).and_then(serde_json::Value::as_str))
                .map_or_else(default_alignment, normalize_alignment);
            let direction = item
                .get("direction")
                .and_then(serde_json::Value::as_str)
                .or_else(|| para_style.and_then(|style| style.get("direction")).and_then(serde_json::Value::as_str))
                .map_or_else(default_direction, normalize_direction);
            let paragraph_number = |key: &str, fallback: &str| {
                item.get(key)
                    .or_else(|| para_style.and_then(|style| style.get(fallback)))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|value| value.is_finite())
                    .map_or(0.0, |value| value.clamp(-5000.0, 5000.0) as f32)
            };

            self.presets.push(TextStylePreset {
                name,
                folder: folder_name,
                font_family,
                font_postscript,
                font_style,
                font_size,
                leading,
                tracking,
                horizontal_scale,
                bold,
                italic,
                all_caps,
                stroke_width,
                stroke_color,
                text_color,
                measurement_system,
                hanging_punctuation,
                anti_alias,
                script_prefixes,
                punctuation_only_scale,
                orientation,
                alignment,
                direction,
                first_line_indent: paragraph_number("firstLineIndent", "firstLineIndent"),
                start_indent: paragraph_number("startIndent", "startIndent"),
                end_indent: paragraph_number("endIndent", "endIndent"),
                space_before: paragraph_number("spaceBefore", "spaceBefore"),
                space_after: paragraph_number("spaceAfter", "spaceAfter"),
                hyphenate: item
                    .get("hyphenate")
                    .and_then(serde_json::Value::as_bool)
                    .or_else(|| para_style.and_then(|style| style.get("hyphenate")).and_then(serde_json::Value::as_bool))
                    .unwrap_or(false),
            });
            count += 1;
        }

        if count > 0 {
            self.selected_preset_idx = self.presets.len() - 1;
            self.recompute_script_style_matches();
        }

        Ok(count)
    }

    /// Экспортирует стили в JSON-строку формата TypeR (совместимо с Photoshop TypeR).
    pub fn export_styles_json(&self, folder_filter: Option<&str>) -> Result<String, String> {
        let mut folders_map = std::collections::HashMap::new();
        let mut folders_arr = Vec::new();

        let mut folder_id_counter = 1;
        for preset in &self.presets {
            if let Some(folder) = &preset.folder
                && !folders_map.contains_key(folder)
            {
                let id = format!("folder_{}", folder_id_counter);
                folder_id_counter += 1;
                folders_map.insert(folder.clone(), id.clone());
                folders_arr.push(serde_json::json!({
                    "id": id,
                    "name": folder,
                    "order": folders_arr.len()
                }));
            }
        }

        let mut styles_arr = Vec::new();
        for preset in &self.presets {
            if let Some(target_f) = folder_filter
                && preset.folder.as_deref() != Some(target_f)
            {
                continue;
            }

            let folder_id = preset.folder.as_ref().and_then(|f| folders_map.get(f));

            styles_arr.push(serde_json::json!({
                "name": preset.name,
                "folder": folder_id,
                "textProps": {
                    "layerText": {
                        "textStyleRange": [
                            {
                                "textStyle": {
                                    "fontName": preset.font_family,
                                    "fontPostScriptName": preset.font_postscript,
                                    "fontStyleName": preset.font_style,
                                    "size": preset.font_size,
                                    "leading": preset.leading,
                                    "tracking": preset.tracking,
                                    "horizontalScale": preset.horizontal_scale,
                                    "syntheticBold": preset.bold && !preset.font_style.to_lowercase().contains("bold"),
                                    "syntheticItalic": preset.italic && !preset.font_style.to_lowercase().contains("italic"),
                                    "fontCaps": if preset.all_caps { "allCaps" } else { "normal" },
                                    "autoKern": if preset.measurement_system.is_optical() { "opticalKern" } else { "metricsKern" },
                                    "color": {
                                        "red": preset.text_color[0],
                                        "green": preset.text_color[1],
                                        "blue": preset.text_color[2]
                                    },
                                    "opacity": (preset.text_color[3] as f32 / 2.55).round() as u32
                                }
                            }
                        ],
                        "paragraphStyleRange": [
                            {
                                "paragraphStyle": {
                                    "alignment": "center",
                                    "autoLeadingPercentage": 1.2
                                }
                            }
                        ]
                    }
                },
                "stroke": {
                    "enabled": preset.stroke_width > 0.0,
                    "size": preset.stroke_width,
                    "opacity": (preset.stroke_color[3] as f32 / 2.55).round() as u32,
                    "position": "outer",
                    "color": {
                        "r": preset.stroke_color[0],
                        "g": preset.stroke_color[1],
                        "b": preset.stroke_color[2]
                    }
                },
                "stroke_width": preset.stroke_width,
                "strokeColor": preset.stroke_color,
                "textColor": format!("#{:02x}{:02x}{:02x}{:02x}", preset.text_color[0], preset.text_color[1], preset.text_color[2], preset.text_color[3]),
                "textColorRgba": preset.text_color,
                "measurementSystem": preset.measurement_system.as_str(),
                "scriptPrefixes": preset.script_prefixes,
                "punctuationOnlyScale": preset.punctuation_only_scale,
                "orientation": preset.orientation,
                "alignment": preset.alignment,
                "direction": preset.direction,
                "firstLineIndent": preset.first_line_indent,
                "startIndent": preset.start_indent,
                "endIndent": preset.end_indent,
                "spaceBefore": preset.space_before,
                "spaceAfter": preset.space_after,
                "hyphenate": preset.hyphenate
            }));
        }

        let out = serde_json::json!({
            "folders": folders_arr,
            "styles": styles_arr
        });

        serde_json::to_string_pretty(&out)
            .map_err(|e| format!("Ошибка сериализации в JSON: {}", e))
    }

    /// Очищает и загружает сырой текст скрипта манги в очередь реплик с автоопределением страниц.
    pub fn load_script(&mut self, content: &str) {
        self.lines.clear();
        let mut page = 1;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Some(detected_page) = detect_page_header(trimmed) {
                page = detected_page;
                continue;
            }

            if trimmed.starts_with("##") {
                continue;
            }

            if let Some(line) = self.parse_script_line(trimmed, page, false) {
                self.lines.push(line);
            }
        }

        self.current_line_idx = 0;
        self.current_page = self.lines.first().map_or(1, |l| l.page);
        self.activate_current_line_preset();
    }

    fn parse_script_line(&self, raw: &str, page: usize, is_applied: bool) -> Option<ScriptLine> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("##") {
            return None;
        }

        let prefix_source = if self.options.strip_dash_prefix {
            trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("— ")).unwrap_or(trimmed)
        } else {
            trimmed
        };
        let style_match = self.match_script_prefix(prefix_source);
        let text_source = style_match.map_or(trimmed, |(_, text)| text);
        let text = clean_script_line_with_options(text_source, &self.options);
        if text.is_empty() {
            return None;
        }

        Some(ScriptLine {
            raw: trimmed.to_string(),
            text,
            is_applied,
            page,
            matched_preset: style_match.map(|(idx, _)| idx),
        })
    }

    fn match_script_prefix<'a>(&self, line: &'a str) -> Option<(usize, &'a str)> {
        if !self.options.match_style_prefixes {
            return None;
        }
        let (tag, text) = line.split_once(':')?;
        let tag = tag.trim();
        let text = text.trim_start();
        if tag.is_empty() || text.is_empty() {
            return None;
        }

        let active_folder = self.active_preset().and_then(|preset| preset.folder.as_deref());
        self.presets
            .iter()
            .enumerate()
            .filter(|(_, preset)| {
                self.options.match_prefixes_across_folders
                    || preset.folder.as_deref() == active_folder
            })
            .flat_map(|(idx, preset)| {
                preset.script_prefixes.iter().filter_map(move |prefix| {
                    let prefix = prefix.trim().trim_end_matches(':').trim();
                    (!prefix.is_empty() && prefix.eq_ignore_ascii_case(tag)).then_some((idx, text, prefix.len()))
                })
            })
            .max_by_key(|(_, _, prefix_len)| *prefix_len)
            .map(|(idx, text, _)| (idx, text))
    }

    fn activate_current_line_preset(&mut self) {
        if let Some(Some(idx)) = self.lines.get(self.current_line_idx).map(|line| line.matched_preset)
            && idx < self.presets.len()
        {
            self.selected_preset_idx = idx;
        }
    }

    pub fn recompute_script_style_matches(&mut self) {
        let updated: Vec<_> = self
            .lines
            .iter()
            .filter_map(|line| self.parse_script_line(&line.raw, line.page, line.is_applied))
            .collect();
        self.lines = updated;
        if self.current_line_idx >= self.lines.len() {
            self.current_line_idx = self.lines.len().saturating_sub(1);
        }
        self.activate_current_line_preset();
    }

    /// Возвращает список всех уникальных страниц в скрипте.
    pub fn pages(&self) -> Vec<usize> {
        let mut p: Vec<usize> = self.lines.iter().map(|l| l.page).collect();
        p.sort_unstable();
        p.dedup();
        if p.is_empty() {
            vec![1]
        } else {
            p
        }
    }

    /// Возвращает реплики для указанной страницы вместе с их индексами в общем списке.
    pub fn lines_for_page(&self, page: usize) -> Vec<(usize, &ScriptLine)> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.page == page)
            .collect()
    }

    pub fn active_line(&self) -> Option<&ScriptLine> {
        self.lines.get(self.current_line_idx)
    }

    pub fn advance_line(&mut self) -> Option<&ScriptLine> {
        if let Some(line) = self.lines.get_mut(self.current_line_idx) {
            line.is_applied = true;
        }
        if self.current_line_idx + 1 < self.lines.len() {
            self.current_line_idx += 1;
            self.activate_current_line_preset();
            let next = self.lines.get(self.current_line_idx);
            if self.options.auto_page_switch
                && let Some(nl) = next
            {
                self.current_page = nl.page;
            }
            next
        } else {
            None
        }
    }

    /// Moves to the next line without marking the current one as applied: the ▼ button's
    /// navigation, where `advance_line` means "sent to the layer".
    pub fn next_line(&mut self) {
        if self.current_line_idx + 1 < self.lines.len() {
            self.current_line_idx += 1;
            self.activate_current_line_preset();
            if self.options.auto_page_switch
                && let Some(line) = self.lines.get(self.current_line_idx)
            {
                self.current_page = line.page;
            }
        }
    }
    pub fn prev_line(&mut self) {
        if self.current_line_idx > 0 {
            self.current_line_idx -= 1;
            self.activate_current_line_preset();
            if self.options.auto_page_switch
                && let Some(line) = self.lines.get(self.current_line_idx)
            {
                self.current_page = line.page;
            }
        }
    }

    pub fn set_current_line(&mut self, idx: usize) {
        if idx < self.lines.len() {
            self.current_line_idx = idx;
            self.activate_current_line_preset();
            if let Some(line) = self.lines.get(idx) {
                self.current_page = line.page;
            }
        }
    }

    pub fn active_preset(&self) -> Option<&TextStylePreset> {
        self.presets.get(self.selected_preset_idx)
    }

    pub fn preset_for_line(&self, idx: usize) -> Option<&TextStylePreset> {
        self.lines
            .get(idx)
            .and_then(|line| line.matched_preset)
            .and_then(|preset_idx| self.presets.get(preset_idx))
            .or_else(|| self.active_preset())
    }

    pub fn active_preset_mut(&mut self) -> Option<&mut TextStylePreset> {
        self.presets.get_mut(self.selected_preset_idx)
    }

    /// Корректирует кегль активного стиля (с опциональной синхронизацией интерлиньяжа).
    pub fn adjust_active_preset_size(&mut self, delta: f32) {
        let sync = self.options.line_spacing_sync;
        if let Some(preset) = self.active_preset_mut() {
            let target_size = preset.font_size + delta;
            if sync {
                preset.sync_leading_with_size(target_size);
            } else {
                preset.font_size = target_size.clamp(4.0, 500.0);
            }
        }
    }

    /// Возвращает список уникальных названий папок стилей.
    pub fn folders(&self) -> Vec<String> {
        let mut list: Vec<String> = self
            .presets
            .iter()
            .filter_map(|p| p.folder.clone())
            .collect();
        list.sort();
        list.dedup();
        list
    }

    /// Дублирует всю папку со всеми стилями (особенность TypeR: Duplicate Style Folders).
    pub fn duplicate_folder(&mut self, folder: &str, new_folder_name: &str) -> usize {
        let to_duplicate: Vec<TextStylePreset> = self
            .presets
            .iter()
            .filter(|p| p.folder.as_deref() == Some(folder))
            .cloned()
            .collect();

        let count = to_duplicate.len();
        for mut style in to_duplicate {
            style.folder = Some(new_folder_name.to_string());
            self.presets.push(style);
        }
        count
    }

    /// Дублирует выбранный стиль.
    pub fn duplicate_preset(&mut self, idx: usize) -> Option<usize> {
        if let Some(preset) = self.presets.get(idx).cloned() {
            let mut copy = preset;
            copy.name = format!("{} (Копия)", copy.name);
            self.presets.push(copy);
            let new_idx = self.presets.len() - 1;
            self.selected_preset_idx = new_idx;
            Some(new_idx)
        } else {
            None
        }
    }

    /// Удаляет стиль по индексу.
    pub fn delete_preset(&mut self, idx: usize) -> bool {
        if self.presets.len() <= 1 || idx >= self.presets.len() {
            return false;
        }
        self.presets.remove(idx);
        for line in &mut self.lines {
            line.matched_preset = line.matched_preset.and_then(|matched| {
                if matched == idx {
                    None
                } else if matched > idx {
                    Some(matched - 1)
                } else {
                    Some(matched)
                }
            });
        }
        if self.selected_preset_idx >= self.presets.len() {
            self.selected_preset_idx = self.presets.len() - 1;
        }
        true
    }
}

/// Распознает заголовок страницы манги (например: "Page 1", "P02", "Стр. 3", "=== Page 4 ===").
pub fn detect_page_header(line: &str) -> Option<usize> {
    let s = line.trim();
    let s = s.trim_matches(|c| c == '=' || c == '-' || c == '[' || c == ']' || c == '#' || c == '*').trim();
    let lower = s.to_lowercase();

    let prefixes = ["page", "p.", "p", "стр.", "стр", "страница"];
    for prefix in prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let digits: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(num) = digits.parse::<usize>()
                && num > 0
            {
                return Some(num);
            }
        }
    }
    None
}

/// Очищает строку сценария с учетом заданных опций.
pub fn clean_script_line_with_options(line: &str, options: &TypesetterOptions) -> String {
    let mut s = line.trim();

    if options.strip_dash_prefix
        && let Some(stripped) = s.strip_prefix("- ").or_else(|| s.strip_prefix("— "))
    {
        s = stripped.trim();
    }

    if options.strip_character_names
        && let Some((_, text)) = s.split_once(':')
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            s = trimmed;
        }
    }

    s.to_string()
}

/// Удаляет типичные префиксы реплик сценария: тире, нумерацию и имена персонажей ("HERO: ...").
pub fn clean_script_line(line: &str) -> String {
    clean_script_line_with_options(line, &TypesetterOptions::default())
}

pub use crate::snow_knuth_plass::char_width_factor;

/// Рассчитывает автоматический перенос слов (word wrap) для вставки текста в выделенный баббл/форму.
/// Использует типографическую привязку союзов/предлогов («висячка») и солвер Кнута—Пласса
/// для балансировки строк по форме овального или ромбовидного манга-баблона.
pub fn wrap_text_to_bubble(
    text: &str,
    bubble_width: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
) -> String {
    crate::snow_knuth_plass::knuth_plass_bubble_wrap_ext(
        text,
        bubble_width,
        bubble_width * 1.2,
        font_size_pt,
        dpi,
        horizontal_scale_pct,
        true,
    )
}

/// Вычисляет геометрический центр ограничивающего прямоугольника [min_x, min_y, max_x, max_y] баббла.
pub fn calculate_bubble_center(bounds: [f32; 4]) -> [f32; 2] {
    let [min_x, min_y, max_x, max_y] = bounds;
    [min_x + (max_x - min_x) * 0.5, min_y + (max_y - min_y) * 0.5]
}

fn contour_profile(selection: &Surface, bounds: Rect) -> Result<Vec<(i32, i32, i32)>, String> {
    let bounds = selection.content_bounds().intersect(&bounds);
    if bounds.is_empty() {
        return Ok(Vec::new());
    }

    let width = bounds.width() as usize;
    let height = bounds.height() as usize;
    let channels = selection.channels();
    if channels == 0 || width == 0 || height == 0 {
        return Ok(Vec::new());
    }
    let row_samples = (width as u64).checked_mul(channels as u64).ok_or_else(|| "bubble selection row size overflow".to_string())?;
    if row_samples > CONTOUR_SCAN_BUDGET {
        return Err("bubble selection is too wide to scan safely".into());
    }

    let max_rows = (CONTOUR_SCAN_BUDGET / row_samples.max(1)).clamp(2, CONTOUR_PROFILE_ROWS as u64) as usize;
    let sample_count = height.min(max_rows);
    let center_twice = i64::from(bounds.x0) + i64::from(bounds.x1);
    let mut profile = Vec::new();
    profile.try_reserve_exact(sample_count).map_err(|error| format!("cannot allocate bubble contour profile: {error}"))?;

    let max_row_gap = (height / sample_count.max(1)).saturating_mul(2).max(2) as i32;
    let mut previous_y = None;
    for row in 0..sample_count {
        let y_offset = if sample_count <= 1 { 0 } else { (row as u64 * (height - 1) as u64 / (sample_count - 1) as u64) as usize };
        let y = bounds.y0.saturating_add(y_offset as i32);
        let row_rect = Rect::new(bounds.x0, y, bounds.x1, y.saturating_add(1));
        let samples = selection
            .try_read_region(row_rect)
            .map_err(|error| format!("cannot read bubble selection: {error}"))?;
        let mut start = None;
        let mut best: Option<(usize, usize, i64, usize)> = None;
        let mut significant_spans = 0usize;
        let mut consider_span = |begin: usize, end: usize| {
            let midpoint_twice = 2 * i64::from(bounds.x0) + begin as i64 + end as i64;
            let distance = (midpoint_twice - center_twice).abs();
            let candidate = (begin, end, distance, end - begin);
            if candidate.3 >= width / 12 {
                significant_spans += 1;
            }
            if best.is_none_or(|current| distance < current.2 || (distance == current.2 && candidate.3 > current.3)) {
                best = Some(candidate);
            }
        };
        for x in 0..width {
            let selected = samples.get(x.saturating_mul(channels)).is_some_and(|value| *value >= 0.5);
            match (start, selected) {
                (None, true) => start = Some(x),
                (Some(begin), false) => {
                    consider_span(begin, x);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(begin) = start {
            consider_span(begin, width);
        }

        if significant_spans > 1 {
            return Ok(Vec::new());
        }
        if let Some((x0, x1, _, _)) = best {
            if previous_y.is_some_and(|previous| y.saturating_sub(previous) > max_row_gap) {
                return Ok(Vec::new());
            }
            profile.push((y, bounds.x0.saturating_add(x0 as i32), bounds.x0.saturating_add(x1 as i32)));
            previous_y = Some(y);
        }
    }
    Ok(profile)
}

fn measure_script_word(text: &str, style: &CharStyle, dpi: f32, engine: &mut photocraft_text::TextEngine) -> f32 {
    let layer = TextLayer {
        text: text.to_string(),
        runs: vec![TextRun { len: text.len(), style: style.clone() }],
        ..TextLayer::default()
    };
    engine.layout(&layer, dpi).lines.first().map_or(0.0, |line| (line.x1 - line.x0).max(0.0))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptMarkupCharStyle {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptMarkupWord {
    pub chars: Vec<ScriptMarkupCharStyle>,
}

fn exact_marker_at(text: &str, at: usize, marker: &str) -> bool {
    let Some(end) = at.checked_add(marker.len()) else {
        return false;
    };
    if text.get(at..end) != Some(marker) {
        return false;
    }
    let byte = marker.as_bytes().first().copied();
    let before_is_marker = at.checked_sub(1).and_then(|i| text.as_bytes().get(i)).copied() == byte;
    let after_is_marker = text.as_bytes().get(end).copied() == byte;
    !before_is_marker && !after_is_marker
}

fn has_closing_marker(text: &str, marker: &str, after: usize) -> bool {
    let mut search = after;
    while let Some(relative) = text.get(search..).and_then(|rest| rest.find(marker)) {
        let at = search.saturating_add(relative);
        if exact_marker_at(text, at, marker) && at > after {
            return true;
        }
        search = at.saturating_add(1);
    }
    false
}

/// Removes Markdown `**bold**` / `__bold__` and `*italic*` / `_italic_` markers while retaining
/// the style of each script word, so inserted layers stay natively editable type.
pub fn parse_script_markup(text: &str) -> (String, Vec<ScriptMarkupWord>) {
    let mut output = String::with_capacity(text.len());
    let mut words = Vec::new();
    let (mut bold, mut italic) = (false, false);
    let mut word = ScriptMarkupWord::default();
    let mut at = 0usize;
    while at < text.len() {
        if text.as_bytes().get(at) == Some(&b'\\')
            && let Some(next) = at.checked_add(1).and_then(|index| text.get(index..))
            && let Some(ch) = next.chars().next()
            && matches!(ch, '*' | '_' | '\\')
        {
            output.push(ch);
            word.chars.push(ScriptMarkupCharStyle { bold, italic });
            at += 1 + ch.len_utf8();
            continue;
        }

        let remaining = text.get(at..).unwrap_or_default();
        let marker = if remaining.starts_with("**") {
            Some(("**", true))
        } else if remaining.starts_with("__") {
            Some(("__", true))
        } else if remaining.starts_with('*') && !remaining.starts_with("**") {
            Some(("*", false))
        } else if remaining.starts_with('_') && !remaining.starts_with("__") {
            Some(("_", false))
        } else {
            None
        };
        if let Some((marker, is_bold)) = marker {
            let style_is_active = if is_bold { bold } else { italic };
            if style_is_active || has_closing_marker(text, marker, at + marker.len()) {
                if is_bold {
                    bold = !bold;
                } else {
                    italic = !italic;
                }
                at += marker.len();
                continue;
            }
        }

        let Some(ch) = remaining.chars().next() else {
            break;
        };
        output.push(ch);
        if ch.is_whitespace() {
            if !word.chars.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.chars.push(ScriptMarkupCharStyle { bold, italic });
        }
        at += ch.len_utf8();
    }
    if !word.chars.is_empty() {
        words.push(word);
    }
    (output, words)
}

/// Builds `type.edit` character-run parameters after contour fitting has inserted line breaks.
pub fn script_markup_run_params(text: &str, words: &[ScriptMarkupWord]) -> Vec<serde_json::Value> {
    let output_chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut cursor = 0usize;
    for style_word in words {
        while output_chars.get(cursor).is_some_and(|ch| ch.is_whitespace()) {
            cursor += 1;
        }
        let word_start = cursor;
        while output_chars.get(cursor).is_some_and(|ch| !ch.is_whitespace()) {
            cursor += 1;
        }
        if cursor.saturating_sub(word_start) != style_word.chars.len() {
            return Vec::new();
        }
        let mut start = 0usize;
        while start < style_word.chars.len() {
            let Some(style) = style_word.chars.get(start).copied() else {
                break;
            };
            let mut end = start + 1;
            while end < style_word.chars.len() && style_word.chars.get(end) == Some(&style) {
                end += 1;
            }
            if style.bold || style.italic {
                let mut params = serde_json::json!({
                    "start": word_start + start,
                    "end": word_start + end
                });
                if style.bold {
                    params["fauxBold"] = serde_json::Value::Bool(true);
                }
                if style.italic {
                    params["fauxItalic"] = serde_json::Value::Bool(true);
                }
                runs.push(params);
            }
            start = end;
        }
    }
    if output_chars.get(cursor).is_some_and(|ch| !ch.is_whitespace()) {
        return Vec::new();
    }
    runs
}

fn contour_line_width(profile: &[(i32, i32, i32)], center_x: f32, center_y: f32, half_height: f32) -> Option<f32> {
    let mut width = f32::INFINITY;
    let mut samples = 0usize;
    for &(y, x0, x1) in profile {
        if (y as f32 - center_y).abs() <= half_height {
            let left = center_x - x0 as f32;
            let right = x1 as f32 - center_x;
            width = width.min((left.min(right) * 2.0).max(0.0));
            samples += 1;
        }
    }
    if samples == 0 {
        profile
            .iter()
            .min_by(|a, b| (a.0 as f32 - center_y).abs().total_cmp(&(b.0 as f32 - center_y).abs()))
            .map(|(_, x0, x1)| (2.0 * (center_x - *x0 as f32).min(*x1 as f32 - center_x)).max(0.0))
    } else {
        Some(width)
    }
}

fn wrap_words_for_contour(
    paragraphs: &[Vec<(String, f32)>],
    profile: &[(i32, i32, i32)],
    bounds: Rect,
    line_count: usize,
    font_size_px: f32,
    leading_px: f32,
    space_width: f32,
) -> Option<Vec<String>> {
    if line_count == 0 || profile.is_empty() {
        return None;
    }
    let line_height = leading_px.max(font_size_px);
    let total_height = (line_count.saturating_sub(1) as f32) * line_height + font_size_px;
    if total_height > bounds.height() as f32 {
        return None;
    }
    let center_y = (bounds.y0 as f32 + bounds.y1 as f32) * 0.5;
    let first_center = center_y - (line_count.saturating_sub(1) as f32) * line_height * 0.5;
    let mut widths = Vec::new();
    widths.try_reserve_exact(line_count).ok()?;
    for index in 0..line_count {
        let y = first_center + index as f32 * line_height;
        let width = contour_line_width(profile, (bounds.x0 as f32 + bounds.x1 as f32) * 0.5, y, font_size_px * 0.45)? * 0.88;
        if width < font_size_px * 0.5 {
            return None;
        }
        widths.push(width);
    }

    let mut output = Vec::new();
    output.try_reserve_exact(line_count).ok()?;
    let mut output_line = 0usize;
    for paragraph in paragraphs {
        if paragraph.is_empty() {
            if output_line >= widths.len() {
                return None;
            }
            output.push(String::new());
            output_line += 1;
            continue;
        }

        let mut word_index = 0usize;
        while word_index < paragraph.len() {
            let target = *widths.get(output_line)?;
            let mut used = 0.0f32;
            let mut end = word_index;
            while let Some((_, word_width)) = paragraph.get(end) {
                let next_width = used + *word_width + if end > word_index { space_width } else { 0.0 };
                if next_width > target {
                    break;
                }
                used = next_width;
                end += 1;
            }
            if end == word_index {
                return None;
            }
            end = balanced_contour_line_end(paragraph, word_index, end, &widths, output_line, space_width);
            let mut line = String::new();
            for index in word_index..end {
                let (word, _) = paragraph.get(index)?;
                if index > word_index {
                    line.push(' ');
                }
                line.push_str(word);
            }
            output.push(line);
            output_line += 1;
            word_index = end;
        }
    }
    Some(output)
}

fn balanced_contour_line_end(words: &[(String, f32)], start: usize, greedy_end: usize, line_widths: &[f32], line_index: usize, space_width: f32) -> usize {
    let Some(&target_width) = line_widths.get(line_index) else {
        return greedy_end;
    };
    let Some(&next_width) = line_widths.get(line_index.saturating_add(1)) else {
        return greedy_end;
    };
    if greedy_end <= start.saturating_add(1) || greedy_end >= words.len() || target_width <= 0.0 || next_width <= 0.0 {
        return greedy_end;
    }

    let first_candidate = greedy_end.saturating_sub(12).max(start.saturating_add(1));
    let mut best_end = greedy_end;
    let mut best_cost = f32::INFINITY;
    let mut current_width = 0.0f32;
    for candidate_end in start.saturating_add(1)..=greedy_end {
        let Some((_, word_width)) = words.get(candidate_end.saturating_sub(1)) else {
            break;
        };
        current_width += *word_width;
        if candidate_end > start.saturating_add(1) {
            current_width += space_width;
        }
        if candidate_end < first_candidate {
            continue;
        }

        let mut next_used = 0.0f32;
        let mut next_end = candidate_end;
        while let Some((_, word_width)) = words.get(next_end) {
            let next_width_with_word = next_used + *word_width + if next_end > candidate_end { space_width } else { 0.0 };
            if next_width_with_word > next_width {
                break;
            }
            next_used = next_width_with_word;
            next_end = next_end.saturating_add(1);
        }
        if next_end == candidate_end {
            continue;
        }

        let current_slack = ((target_width - current_width).max(0.0) / target_width).powi(2);
        let next_slack = ((next_width - next_used).max(0.0) / next_width).powi(2);
        let current_weight = if candidate_end == words.len() { 0.4 } else { 1.0 };
        let next_weight = if next_end == words.len() { 0.4 } else { 0.7 };
        let orphan_penalty = if next_end == words.len() && words.len().saturating_sub(candidate_end) == 1 { 0.2 } else { 0.0 };
        let cost = current_slack * current_weight + next_slack * next_weight + orphan_penalty;
        if cost < best_cost {
            best_cost = cost;
            best_end = candidate_end;
        }
    }
    best_end
}

/// One proposed line-breaking of the active TypeR line, shown by TextShapeR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextShapeKind {
    /// Explicit line breaks kept as written, with orphan binders applied.
    Original,
    /// The fitted contour layout at the style's size or the smallest allowed size.
    ContourFit,
    /// The rectangular bubble wrap.
    BubbleWrap,
    /// Words distributed over roughly square line lengths.
    Balanced,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextShapeCandidate {
    pub kind: TextShapeKind,
    pub text: String,
}

fn balanced_plain_wrap(text: &str) -> String {
    let mut output = Vec::new();
    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split(' ').filter(|word| !word.is_empty()).collect();
        if words.len() <= 2 {
            output.push(words.join(" "));
            continue;
        }
        let target_lines = (words.len() as f32).sqrt().ceil().max(1.0) as usize;
        let per_line = words.len().div_ceil(target_lines);
        let mut index = 0usize;
        while index < words.len() {
            let end = (index + per_line).min(words.len());
            output.push(words.get(index..end).unwrap_or_default().join(" "));
            index = end;
        }
    }
    output.join("\n")
}

/// Balanced 2-line wrap: searches the single split point that minimizes character difference.
fn balanced_two_line_wrap(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() <= 2 {
        return words.join(" ");
    }
    let mut best_split = 1;
    let mut min_diff = usize::MAX;
    for i in 1..words.len() {
        let l1 = words[..i].join(" ").chars().count();
        let l2 = words[i..].join(" ").chars().count();
        let diff = l1.abs_diff(l2);
        if diff < min_diff {
            min_diff = diff;
            best_split = i;
        }
    }
    format!("{}\n{}", words[..best_split].join(" "), words[best_split..].join(" "))
}

/// Diamond / Manga speech-bubble wrap: 3 lines where the middle line is wider than top and bottom.
fn diamond_manga_wrap(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() <= 2 {
        return words.join(" ");
    }
    if words.len() == 3 {
        return balanced_two_line_wrap(text);
    }
    let mut best_i = 1;
    let mut best_j = 2;
    let mut best_cost = f32::MAX;

    for i in 1..words.len().saturating_sub(1) {
        for j in (i + 1)..words.len() {
            let len1 = words[..i].join(" ").chars().count();
            let len2 = words[i..j].join(" ").chars().count();
            let len3 = words[j..].join(" ").chars().count();

            // Diamond bonus: middle line wider than both top and bottom
            let diamond_penalty = if len2 >= len1 && len2 >= len3 { 0.0 } else { 120.0 };
            // Symmetry penalty between top and bottom
            let sym_penalty = (len1 as f32 - len3 as f32).abs();
            // Ratio: middle line ideally ~1.2x to 1.5x of edges
            let max_edge = len1.max(len3).max(1) as f32;
            let ratio_cost = ((len2 as f32 / max_edge) - 1.3).abs() * 15.0;

            let cost = diamond_penalty + sym_penalty + ratio_cost;
            if cost < best_cost {
                best_cost = cost;
                best_i = i;
                best_j = j;
            }
        }
    }

    format!(
        "{}\n{}\n{}",
        words[..best_i].join(" "),
        words[best_i..best_j].join(" "),
        words[best_j..].join(" ")
    )
}

/// Balanced 4-line wrap for longer phrases or vertical/narrow bubbles.
fn balanced_four_line_wrap(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() < 4 {
        return balanced_plain_wrap(text);
    }
    let per_line = words.len().div_ceil(4);
    let mut lines = Vec::new();
    let mut idx = 0;
    while idx < words.len() {
        let end = (idx + per_line).min(words.len());
        lines.push(words[idx..end].join(" "));
        idx = end;
    }
    lines.join("\n")
}

/// Proposes line breaks for the active TypeR line. This is a bounded, local TextShapeR:
/// it reuses the existing contour fit, rectangular wrap and a balanced split instead of
/// TypeR's trained lettering model.
/// Автоматически обнаруживает прямоугольные границы баббла вокруг якорной точки (центра текста)
/// на растровой поверхности, трассируя лучи во все 4 стороны до темной границы (luma < 0.45).
pub fn detect_speech_bubble_bounds(
    surface: &Surface,
    anchor_x: i32,
    anchor_y: i32,
    max_radius: i32,
) -> Option<Rect> {
    let bounds = surface.content_bounds();
    if !bounds.contains(anchor_x, anchor_y) {
        return None;
    }
    let get_luma = |x: i32, y: i32| -> f32 {
        let px = surface.pixel(x, y);
        if px.is_empty() {
            return 0.0;
        }
        if px.len() >= 3 {
            0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2]
        } else {
            px[0]
        }
    };
    let mut seed = None;
    for r in [0, 4, 8, 12, 16, 20] {
        for dy in [-r, 0, r] {
            for dx in [-r, 0, r] {
                let sx = anchor_x + dx;
                let sy = anchor_y + dy;
                if bounds.contains(sx, sy) && get_luma(sx, sy) >= 0.50 {
                    seed = Some((sx, sy));
                    break;
                }
            }
            if seed.is_some() { break; }
        }
        if seed.is_some() { break; }
    }
    let (seed_x, seed_y) = seed?;
    let max_r = max_radius.clamp(20, 600);
    let mut min_x = seed_x;
    while min_x > seed_x - max_r && min_x > bounds.x0 {
        if get_luma(min_x - 1, seed_y) < 0.45 {
            break;
        }
        min_x -= 1;
    }
    let mut max_x = seed_x;
    while max_x < seed_x + max_r && max_x < bounds.x1 {
        if get_luma(max_x + 1, seed_y) < 0.45 {
            break;
        }
        max_x += 1;
    }
    let mut min_y = seed_y;
    while min_y > seed_y - max_r && min_y > bounds.y0 {
        if get_luma(seed_x, min_y - 1) < 0.45 {
            break;
        }
        min_y -= 1;
    }
    let mut max_y = seed_y;
    while max_y < seed_y + max_r && max_y < bounds.y1 {
        if get_luma(seed_x, max_y + 1) < 0.45 {
            break;
        }
        max_y += 1;
    }
    let w = max_x - min_x;
    let h = max_y - min_y;
    if w >= 20 && h >= 20 {
        Some(Rect::new(min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

pub fn text_shape_candidates(
    text: &str,
    preset: &TextStylePreset,
    selection: Option<&Surface>,
    document_bounds: Rect,
    dpi: f32,
    sync_leading: bool,
    minimum_fit_scale_percent: f32,
) -> Result<Vec<TextShapeCandidate>, String> {
    if text.trim().is_empty() || text.len() > 16_384 {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let push_candidate = |candidates: &mut Vec<TextShapeCandidate>, kind, text: String| {
        let trimmed = text.trim();
        if !trimmed.is_empty() && !candidates.iter().any(|c: &TextShapeCandidate| c.text == trimmed) {
            candidates.push(TextShapeCandidate { kind, text: trimmed.to_string() });
        }
    };

    // Clean un-broken text for algorithmic generation
    let clean_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let bound_clean = crate::snow_knuth_plass::bind_orphan_prepositions(&clean_text);

    // 1. Original text (preserving existing line breaks if multi-line, otherwise bound clean)
    let original_text = if text.contains('\n') {
        text.lines()
            .map(|l| crate::snow_knuth_plass::bind_orphan_prepositions(l.trim()))
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        bound_clean.clone()
    };
    push_candidate(&mut candidates, TextShapeKind::Original, original_text);

    let (bubble_w, bubble_h, bubble_shape) = if let Some(sel) = selection {
        let b = sel.content_bounds().intersect(&document_bounds);
        let w = b.width().max(1) as f32;
        let h = b.height().max(1) as f32;
        let shape = crate::snow_knuth_plass::BubbleShape::from_aspect_ratio(w, h);
        (Some(w), Some(h), shape)
    } else {
        (None, None, crate::snow_knuth_plass::BubbleShape::RoundOval)
    };

    if let Some(selection) = selection {
        let mut fitted_preset = preset.clone();
        if let Some(fitted) = fit_script_text_to_selection(
            &clean_text,
            &mut fitted_preset,
            selection,
            document_bounds,
            dpi,
            sync_leading,
            minimum_fit_scale_percent,
        )? {
            push_candidate(&mut candidates, TextShapeKind::ContourFit, fitted);
        }
        if let (Some(w), Some(h)) = (bubble_w, bubble_h)
            && w >= 10.0 && h >= 10.0
        {
            push_candidate(
                    &mut candidates,
                    TextShapeKind::BubbleWrap,
                    crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                        &clean_text,
                        w,
                        h,
                        preset.font_size,
                        dpi,
                        preset.horizontal_scale,
                        preset.hanging_punctuation,
                        bubble_shape,
                    ),
                );
        }
    }

    // Предложения с учетом формы баббла
    let words_count = bound_clean.split(' ').filter(|w| !w.is_empty()).count();
    if words_count <= 5 {
        match bubble_shape {
            crate::snow_knuth_plass::BubbleShape::VerticalOval => {
                let bw = bubble_w.unwrap_or(120.0);
                let bh = bubble_h.unwrap_or(240.0);
                push_candidate(
                    &mut candidates,
                    TextShapeKind::Balanced,
                    crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                        &clean_text,
                        bw * 0.90,
                        bh,
                        preset.font_size,
                        dpi,
                        preset.horizontal_scale,
                        preset.hanging_punctuation,
                        crate::snow_knuth_plass::BubbleShape::VerticalOval,
                    ),
                );
                push_candidate(&mut candidates, TextShapeKind::BubbleWrap, diamond_manga_wrap(&bound_clean));
                if words_count >= 4 {
                    push_candidate(&mut candidates, TextShapeKind::Balanced, balanced_four_line_wrap(&bound_clean));
                }
            }
            crate::snow_knuth_plass::BubbleShape::HorizontalOval => {
                push_candidate(&mut candidates, TextShapeKind::BubbleWrap, balanced_two_line_wrap(&bound_clean));
                let bw = bubble_w.unwrap_or(240.0);
                let bh = bubble_h.unwrap_or(120.0);
                push_candidate(
                    &mut candidates,
                    TextShapeKind::Balanced,
                    crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                        &clean_text,
                        bw,
                        bh,
                        preset.font_size,
                        dpi,
                        preset.horizontal_scale,
                        preset.hanging_punctuation,
                        crate::snow_knuth_plass::BubbleShape::HorizontalOval,
                    ),
                );
            }
            crate::snow_knuth_plass::BubbleShape::Rectangle => {
                let bw = bubble_w.unwrap_or(160.0);
                let bh = bubble_h.unwrap_or(160.0);
                push_candidate(
                    &mut candidates,
                    TextShapeKind::Balanced,
                    crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                        &clean_text,
                        bw,
                        bh,
                        preset.font_size,
                        dpi,
                        preset.horizontal_scale,
                        preset.hanging_punctuation,
                        crate::snow_knuth_plass::BubbleShape::Rectangle,
                    ),
                );
                push_candidate(&mut candidates, TextShapeKind::BubbleWrap, balanced_two_line_wrap(&bound_clean));
            }
            _ => {
                if words_count >= 3 {
                    push_candidate(&mut candidates, TextShapeKind::Balanced, diamond_manga_wrap(&bound_clean));
                    push_candidate(&mut candidates, TextShapeKind::BubbleWrap, balanced_two_line_wrap(&bound_clean));
                    if words_count >= 5 {
                        push_candidate(&mut candidates, TextShapeKind::Balanced, balanced_four_line_wrap(&bound_clean));
                    } else {
                        push_candidate(&mut candidates, TextShapeKind::Balanced, balanced_plain_wrap(&bound_clean));
                    }
                } else {
                    push_candidate(&mut candidates, TextShapeKind::Balanced, balanced_plain_wrap(&bound_clean));
                }
            }
        }
    } else {
        // Большой текст (6+ слов): не форсируем 2-3 строки, используем адаптивный перенос по форме
        let bw = bubble_w.unwrap_or(150.0);
        let bh = bubble_h.unwrap_or(300.0);
        push_candidate(
            &mut candidates,
            TextShapeKind::Balanced,
            crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                &clean_text,
                bw * 0.85,
                bh,
                preset.font_size,
                dpi,
                preset.horizontal_scale,
                preset.hanging_punctuation,
                bubble_shape,
            ),
        );
        push_candidate(
            &mut candidates,
            TextShapeKind::Balanced,
            crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                &clean_text,
                bw * 1.15,
                bh,
                preset.font_size,
                dpi,
                preset.horizontal_scale,
                preset.hanging_punctuation,
                bubble_shape,
            ),
        );
    }

    // Если есть длинные слова, предлагаем вариант со слоговыми переносами
    let longest_word = bound_clean.split(' ').map(|w| w.chars().count()).max().unwrap_or(0);
    if longest_word >= 8 {
        let bw = bubble_w.unwrap_or(140.0);
        let dpi_scale = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
        let h_scale = (preset.horizontal_scale / 100.0).clamp(0.1, 5.0);
        let hyphenated = crate::snow_knuth_plass::hyphenate_long_words_with_breaks(
            &bound_clean,
            bw * 0.85,
            preset.font_size * dpi_scale,
            h_scale,
        );
        push_candidate(&mut candidates, TextShapeKind::BubbleWrap, hyphenated);
    }

    if candidates.is_empty() {
        push_candidate(&mut candidates, TextShapeKind::Original, bound_clean);
    }

    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
pub fn text_shape_candidates_with_bubble(
    text: &str,
    preset: &TextStylePreset,
    selection: Option<&Surface>,
    detected_bubble: Option<Rect>,
    document_bounds: Rect,
    dpi: f32,
    sync_leading: bool,
    minimum_fit_scale_percent: f32,
) -> Result<Vec<TextShapeCandidate>, String> {
    if selection.is_none() && let Some(bubble) = detected_bubble {
        let fake_shape = crate::snow_knuth_plass::BubbleShape::from_aspect_ratio(bubble.width() as f32, bubble.height() as f32);
        let mut candidates = Vec::new();
        let clean_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let bound_clean = crate::snow_knuth_plass::bind_orphan_prepositions(&clean_text);
        let original_text = if text.contains('\n') {
            text.lines()
                .map(|l| crate::snow_knuth_plass::bind_orphan_prepositions(l.trim()))
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            bound_clean.clone()
        };
        candidates.push(TextShapeCandidate { kind: TextShapeKind::Original, text: original_text });
        candidates.push(TextShapeCandidate {
            kind: TextShapeKind::BubbleWrap,
            text: crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                &clean_text,
                bubble.width() as f32,
                bubble.height() as f32,
                preset.font_size,
                dpi,
                preset.horizontal_scale,
                preset.hanging_punctuation,
                fake_shape,
            ),
        });
        let bw = bubble.width() as f32;
        let bh = bubble.height() as f32;
        let words_count = bound_clean.split(' ').filter(|w| !w.is_empty()).count();
        if words_count <= 5 {
            match fake_shape {
                crate::snow_knuth_plass::BubbleShape::VerticalOval => {
                    candidates.push(TextShapeCandidate {
                        kind: TextShapeKind::Balanced,
                        text: diamond_manga_wrap(&bound_clean),
                    });
                }
                crate::snow_knuth_plass::BubbleShape::HorizontalOval | crate::snow_knuth_plass::BubbleShape::Rectangle => {
                    candidates.push(TextShapeCandidate {
                        kind: TextShapeKind::Balanced,
                        text: balanced_two_line_wrap(&bound_clean),
                    });
                }
                _ => {
                    candidates.push(TextShapeCandidate {
                        kind: TextShapeKind::Balanced,
                        text: diamond_manga_wrap(&bound_clean),
                    });
                }
            }
        } else {
            // Для длинного текста (6+ слов) предлагаем более плотную и более широкую разбивку
            candidates.push(TextShapeCandidate {
                kind: TextShapeKind::Balanced,
                text: crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                    &clean_text,
                    bw * 0.85,
                    bh,
                    preset.font_size,
                    dpi,
                    preset.horizontal_scale,
                    preset.hanging_punctuation,
                    fake_shape,
                ),
            });
            candidates.push(TextShapeCandidate {
                kind: TextShapeKind::Balanced,
                text: crate::snow_knuth_plass::knuth_plass_bubble_wrap_with_shape(
                    &clean_text,
                    bw * 1.15,
                    bh,
                    preset.font_size,
                    dpi,
                    preset.horizontal_scale,
                    preset.hanging_punctuation,
                    fake_shape,
                ),
            });
        }
        return Ok(candidates);
    }
    text_shape_candidates(text, preset, selection, document_bounds, dpi, sync_leading, minimum_fit_scale_percent)
}

/// Wraps and scales a TypeR line against the selected bubble's sampled contour.
///
/// `None` means the selection is not a suitable bubble or the text cannot fit at the minimum
/// type size; callers may retain their rectangular-bubble fallback. Errors reading the mask are
/// returned so the caller can report them rather than silently pretending the fit succeeded.
pub fn fit_script_text_to_selection(
    text: &str,
    preset: &mut TextStylePreset,
    selection: &Surface,
    document_bounds: Rect,
    dpi: f32,
    sync_leading: bool,
    minimum_fit_scale_percent: f32,
) -> Result<Option<String>, String> {
    if text.trim().is_empty() || text.len() > 16_384 || !preset.font_size.is_finite() || preset.font_size <= 0.0 {
        return Ok(None);
    }
    let mut text = crate::snow_knuth_plass::bind_orphan_prepositions(text);
    let profile = contour_profile(selection, document_bounds)?;
    if profile.len() < 2 {
        return Ok(None);
    }
    let fit_bounds = selection.content_bounds().intersect(&document_bounds);
    if fit_bounds.width() < 8 || fit_bounds.height() < 8 {
        return Ok(None);
    }
    if preset.hyphenate {
        let dpi_scale = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
        let widest_line = profile.iter().map(|sample| (sample.2 - sample.1) as f32).fold(0.0f32, f32::max) * 0.88;
        let h_scale = (preset.horizontal_scale / 100.0).clamp(0.01, 10.0);
        text = crate::snow_knuth_plass::hyphenate_long_words_with_breaks(
            &text,
            widest_line * 0.95,
            preset.font_size * dpi_scale,
            h_scale,
        );
    }

    let paragraphs: Vec<Vec<&str>> = text
        .split('\n')
        .map(|paragraph| paragraph.split(' ').filter(|word| !word.is_empty()).collect())
        .collect();
    let family = if preset.font_family.trim().is_empty() { photocraft_text::fonts::DEFAULT_FAMILY } else { preset.font_family.as_str() };
    let style = CharStyle {
        font_family: family.to_string(),
        font_style: preset.font_style.clone(),
        postscript_name: (!preset.font_postscript.is_empty()).then(|| preset.font_postscript.clone()),
        weight: if preset.bold { 700 } else { 400 },
        italic: preset.italic,
        size_pt: preset.font_size,
        tracking: preset.tracking as f32,
        horizontal_scale: (preset.horizontal_scale / 100.0).clamp(0.01, 10.0),
        faux_bold: preset.bold,
        faux_italic: preset.italic,
        kerning: if preset.measurement_system.is_optical() { Kerning::Optical } else { Kerning::Metrics },
        ..CharStyle::default()
    };
    let dpi_scale = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let (word_widths, space_width) = {
        let mut engine = photocraft_text::shared().lock().unwrap_or_else(|error| error.into_inner());
        let mut cache = std::collections::HashMap::<String, f32>::new();
        for word in paragraphs.iter().flatten() {
            if !cache.contains_key(*word) {
                let width = measure_script_word(word, &style, dpi, &mut engine);
                cache.insert((*word).to_string(), width);
            }
        }
        let measured_space = measure_script_word(" ", &style, dpi, &mut engine);
        let space = if measured_space > 0.0 { measured_space } else { preset.font_size * dpi_scale * style.horizontal_scale * 0.25 };
        (cache, space)
    };
    let paragraphs: Vec<Vec<(String, f32)>> = paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .iter()
                .map(|word| {
                    let width = word_widths.get(*word).copied().unwrap_or(0.0);
                    ((*word).to_string(), width)
                })
                .collect()
        })
        .collect();
    let total_width: f32 = paragraphs.iter().flatten().map(|(_, width)| *width).sum::<f32>()
        + paragraphs.iter().map(|paragraph| paragraph.len().saturating_sub(1)).sum::<usize>() as f32 * space_width;
    if total_width <= 0.0 || !total_width.is_finite() {
        return Ok(None);
    }

    let original_size = preset.font_size;
    let original_leading = if preset.leading > 0.0 { preset.leading } else { original_size * 1.2 };
    let check_size = |size: f32| -> Option<Vec<String>> {
        let ratio = size / original_size;
        let leading_pt = if sync_leading { original_leading * ratio } else { original_leading };
        let font_size_px = size * dpi_scale;
        let leading_px = leading_pt * dpi_scale;
        let available_lines = ((fit_bounds.height() as f32 - font_size_px) / leading_px.max(font_size_px)).floor() as usize + 1;
        if available_lines == 0 || available_lines > 512 {
            return None;
        }
        let scaled_paragraphs: Vec<Vec<(String, f32)>> = paragraphs
            .iter()
            .map(|paragraph| paragraph.iter().map(|(word, width)| (word.clone(), width * ratio)).collect())
            .collect();
        let widest = profile.iter().map(|sample| (sample.2 - sample.1) as f32).fold(0.0f32, f32::max) * 0.88;
        let mut expected = ((total_width * ratio / widest.max(1.0) / 0.68).ceil() as usize).clamp(1, available_lines);
        for _ in 0..12 {
            let lines = wrap_words_for_contour(&scaled_paragraphs, &profile, fit_bounds, expected, font_size_px, leading_px, space_width * ratio)?;
            if lines.len() == expected {
                return Some(lines);
            }
            if lines.len() > available_lines {
                return None;
            }
            expected = lines.len().max(1);
        }
        None
    };

    if let Some(lines) = check_size(original_size) {
        return Ok(Some(lines.join("\n")));
    }
    let minimum_scale = if minimum_fit_scale_percent.is_finite() {
        (minimum_fit_scale_percent / 100.0).clamp(0.1, 1.0)
    } else {
        DEFAULT_TYPE_FIT_MIN_PERCENT / 100.0
    };
    let minimum_size = (original_size * minimum_scale).max(0.1);
    let Some(mut best_lines) = check_size(minimum_size) else {
        return Ok(None);
    };
    let mut low = minimum_size;
    let mut high = original_size;
    for _ in 0..12 {
        let candidate = (low + high) * 0.5;
        if let Some(lines) = check_size(candidate) {
            low = candidate;
            best_lines = lines;
        } else {
            high = candidate;
        }
    }
    let scale = low / original_size;
    preset.font_size = low;
    if sync_leading {
        preset.leading = original_leading * scale;
    }
    Ok(Some(best_lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_tpl_entry(section: &mut Vec<u8>, name: &str, descriptor: photocraft_psd::Descriptor) {
        let units: Vec<u16> = name.encode_utf16().collect();
        section.extend_from_slice(&(units.len() as u32).to_be_bytes());
        for unit in units {
            section.extend_from_slice(&unit.to_be_bytes());
        }
        // Photoshop writes a descriptor version (measured: 16), then one full ActionDescriptor
        // (display name, class id, property count, properties).
        section.extend_from_slice(&16u32.to_be_bytes());
        section.extend_from_slice(&descriptor.to_bytes());
    }

    fn tpl_file(section: Vec<u8>) -> Vec<u8> {
        let mut data = b"8BTP".to_vec();
        data.extend_from_slice(&2u32.to_be_bytes());
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(b"8BIMtptp");
        data.extend_from_slice(&(section.len() as u32).to_be_bytes());
        data.extend_from_slice(&section);
        while !data.len().is_multiple_of(4) {
            data.push(0);
        }
        data
    }

    #[test]
    fn invalid_unicode_hex_colors_are_rejected_without_panicking() {
        assert_eq!(parse_color_value(&serde_json::json!("界界")), None);
        assert_eq!(parse_color_value(&serde_json::json!("1界界1")), None);
    }

    /// ▼ is navigation: it moves to the next line without marking the current one applied.
    #[test]
    fn next_line_navigates_without_applying() {
        let mut state = TypesetterState::default();
        state.load_script("Line one\nLine two\nLine three");
        state.set_current_line(1);
        state.next_line();
        assert_eq!(state.current_line_idx, 2);
        assert!(state.lines.iter().all(|l| !l.is_applied), "navigation must not send the line");
        state.next_line();
        assert_eq!(state.current_line_idx, 2, "stays on the last line");
    }

    #[test]
    fn test_clean_script_line() {
        assert_eq!(clean_script_line("- Привет!"), "Привет!");
        assert_eq!(clean_script_line("ГЕРОЙ: Стой на месте!"), "Стой на месте!");
        assert_eq!(clean_script_line("Обычная строка"), "Обычная строка");
    }

    #[test]
    fn test_typesetter_queue() {
        let mut state = TypesetterState::default();
        state.load_script("Hero: Line 1\n\n- Line 2");
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.active_line().unwrap().text, "Line 1");

        state.advance_line();
        assert_eq!(state.active_line().unwrap().text, "Line 2");
        assert!(state.lines[0].is_applied);
    }

    #[test]
    fn script_prefix_selects_matching_style_and_is_removed() {
        let mut state = TypesetterState::default();
        state.presets[0].script_prefixes = vec!["REG".into()];
        state.load_script("Page 12 : title\nrEg: Dialogue\n## translator note\n- REG: Next");

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines[0].page, 12);
        assert_eq!(state.lines[0].text, "Dialogue");
        assert_eq!(state.lines[0].matched_preset, Some(0));
        assert_eq!(state.lines[1].text, "Next");
        assert_eq!(state.lines[1].matched_preset, Some(0));
        assert_eq!(state.selected_preset_idx, 0);
    }

    #[test]
    fn style_prefix_scope_can_be_extended_across_folders() {
        let mut state = TypesetterState::default();
        state.presets[1].folder = Some("SFX".into());
        state.presets[1].script_prefixes = vec!["SFX".into()];
        state.load_script("SFX: Boom");
        assert_eq!(state.lines[0].matched_preset, None);

        state.options.match_prefixes_across_folders = true;
        state.load_script("SFX: Boom");
        assert_eq!(state.lines[0].matched_preset, Some(1));
        assert_eq!(state.selected_preset_idx, 1);
    }

    #[test]
    fn navigating_to_prefixed_line_activates_its_style() {
        let mut state = TypesetterState::default();
        state.presets[0].script_prefixes = vec!["REG".into()];
        state.presets[1].folder = state.presets[0].folder.clone();
        state.presets[1].script_prefixes = vec!["SFX".into()];
        state.load_script("REG: Hello\nSFX: Boom");
        state.next_line();

        assert_eq!(state.selected_preset_idx, 1);
        state.prev_line();
        assert_eq!(state.selected_preset_idx, 0);
    }

    #[test]
    fn line_preset_uses_prefix_match_even_before_navigation() {
        let mut state = TypesetterState::default();
        state.presets[0].script_prefixes = vec!["REG".into()];
        state.presets[1].folder = state.presets[0].folder.clone();
        state.presets[1].script_prefixes = vec!["SFX".into()];
        state.load_script("REG: Dialogue\nSFX: Boom");

        assert_eq!(state.selected_preset_idx, 0);
        assert_eq!(state.preset_for_line(1).map(|preset| preset.name.as_str()), Some("Крик / SFX"));
    }

    #[test]
    fn imported_style_prefixes_round_trip_and_old_settings_default_cleanly() {
        let mut state = TypesetterState::default();
        let imported = state
            .import_styles_json(r#"{"styles":[{"name":"SFX","prefixes":["SFX","BOOM"]}]}"#)
            .unwrap();
        assert_eq!(imported, 1);
        assert_eq!(state.presets.last().unwrap().script_prefixes, vec!["SFX", "BOOM"]);

        let exported = state.export_styles_json(None).unwrap();
        let exported: serde_json::Value = serde_json::from_str(&exported).unwrap();
        let styles = exported.get("styles").and_then(serde_json::Value::as_array).unwrap();
        let prefixes = styles.last().and_then(|style| style.get("scriptPrefixes"));
        assert_eq!(prefixes, Some(&serde_json::json!(["SFX", "BOOM"])));

        let options: TypesetterOptions = serde_json::from_str(
            r#"{"auto_page_switch":true,"line_spacing_sync":true,"strip_character_names":true,"strip_dash_prefix":true,"optical_baseline_factor":0.35,"hanging_punctuation":true}"#,
        )
        .unwrap();
        assert!(options.match_style_prefixes);
        assert!(!options.match_prefixes_across_folders);
        assert_eq!(options.minimum_fit_scale_percent, DEFAULT_TYPE_FIT_MIN_PERCENT);
    }

    #[test]
    fn test_bubble_center_calculation() {
        let center = calculate_bubble_center([100.0, 100.0, 300.0, 500.0]);
        assert_eq!(center, [200.0, 300.0]);
    }

    #[test]
    fn contour_fit_wraps_to_mask_and_reduces_size_when_needed() {
        let bounds = Rect::new(0, 0, 260, 220);
        let center_x = 130.0f32;
        let center_y = 110.0f32;
        let mut pixels = vec![0.0; bounds.width() as usize * bounds.height() as usize];
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let dx = (x as f32 - center_x) / 128.0;
                let dy = (y as f32 - center_y) / 108.0;
                if dx * dx + dy * dy <= 1.0 {
                    let index = y as usize * bounds.width() as usize + x as usize;
                    if let Some(pixel) = pixels.get_mut(index) {
                        *pixel = 1.0;
                    }
                }
            }
        }
        let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &pixels);
        let text = "This is a longer sample dialogue that needs several carefully balanced lines inside the selected speech bubble";
        let mut preset = TextStylePreset {
            font_family: "Inter".into(),
            font_postscript: String::new(),
            font_size: 24.0,
            leading: 28.0,
            ..TextStylePreset::default()
        };

        let fitted = fit_script_text_to_selection(text, &mut preset, &selection, bounds, 72.0, true, 75.0).unwrap();
        let fitted = fitted.expect("the ellipse selection must fit after reducing the type size");

        assert!(fitted.contains('\n'));
        assert!(preset.font_size < 24.0);
        assert!(preset.font_size >= 18.0);
        let line_lengths: Vec<_> = fitted.lines().map(|line| line.chars().count()).collect();
        assert!(line_lengths.first().zip(line_lengths.get(line_lengths.len() / 2)).is_some_and(|(top, middle)| top < middle));
        let compact = |value: &str| value.chars().filter(|character| !character.is_whitespace()).collect::<String>();
        assert_eq!(compact(&fitted), compact(text));
    }

    #[test]
    fn contour_fit_rejects_an_empty_selection() {
        let selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        let mut preset = TextStylePreset::default();
        assert_eq!(
            fit_script_text_to_selection("Hello", &mut preset, &selection, Rect::new(0, 0, 200, 200), 72.0, true, 75.0).unwrap(),
            None
        );
    }

    #[test]
    fn contour_wrap_balances_adjacent_lines_to_avoid_a_one_word_widow() {
        let words = vec![vec![
            ("one".into(), 20.0),
            ("two".into(), 20.0),
            ("three".into(), 20.0),
            ("four".into(), 20.0),
        ]];
        let profile: Vec<_> = (0..100).map(|y| (y, 0, 100)).collect();

        let lines = wrap_words_for_contour(&words, &profile, Rect::new(0, 0, 100, 100), 2, 10.0, 10.0, 5.0).unwrap();

        assert_eq!(lines, vec!["one two", "three four"]);
    }

    #[test]
    fn script_markdown_becomes_editable_character_runs() {
        let (plain, words) = parse_script_markup("Regular **bold text** and *italic*");
        assert_eq!(plain, "Regular bold text and italic");
        let runs = script_markup_run_params(&plain, &words);
        assert_eq!(
            runs,
            vec![
                serde_json::json!({"start": 8, "end": 12, "fauxBold": true}),
                serde_json::json!({"start": 13, "end": 17, "fauxBold": true}),
                serde_json::json!({"start": 22, "end": 28, "fauxItalic": true})
            ]
        );
    }

    #[test]
    fn script_markdown_preserves_unmatched_and_escaped_markers() {
        let (plain, words) = parse_script_markup(r"Keep \*literal\* and **unclosed");
        assert_eq!(plain, "Keep *literal* and **unclosed");
        assert!(script_markup_run_params(&plain, &words).is_empty());
    }

    #[test]
    fn script_markdown_runs_use_character_offsets_with_unicode_and_wrapped_lines() {
        let (plain, words) = parse_script_markup("普通 **太字**\n*声*");
        assert_eq!(plain, "普通 太字\n声");
        assert_eq!(
            script_markup_run_params(&plain, &words),
            vec![
                serde_json::json!({"start": 3, "end": 5, "fauxBold": true}),
                serde_json::json!({"start": 6, "end": 7, "fauxItalic": true})
            ]
        );
    }

    #[test]
    fn document_style_scan_imports_and_deduplicates_editable_type_styles() {
        let style = CharStyle {
            font_family: "Example Sans".into(),
            postscript_name: Some("ExampleSans-Bold".into()),
            font_style: "Bold".into(),
            weight: 700,
            size_pt: 18.0,
            color: photocraft_color::Color::rgba(0.1, 0.2, 0.3, 1.0),
            tracking: 25.0,
            horizontal_scale: 0.9,
            faux_bold: true,
            kerning: Kerning::Optical,
            ..CharStyle::default()
        };
        let text_layer = TextLayer {
            text: "Sample text".into(),
            runs: vec![TextRun { len: "Sample text".len(), style }],
            antialias: AntiAlias::Sharp,
            ..TextLayer::default()
        };
        let mut doc = Document::new(
            "Style scan",
            photocraft_geom::Size::new(100, 100),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        );
        let mut layer = photocraft_doc::Layer::new("Dialogue", LayerContent::Text(text_layer));
        layer.effects.enabled = true;
        layer.effects.items.push(Effect::Stroke(photocraft_doc::effects::StrokeFx {
            common: photocraft_doc::effects::FxCommon::new(photocraft_doc::BlendMode::Normal, 0.5),
            size: 2.0,
            position: photocraft_doc::effects::StrokePosition::Outside,
            paint: FxPaint::Color(photocraft_color::Color::WHITE),
            overprint: false,
        }));
        doc.layers.push(layer);

        let mut state = TypesetterState::default();
        let (added, unsupported) = state.scan_document_text_styles(&doc);
        assert_eq!((added, unsupported), (1, 0));
        let preset = state.active_preset().unwrap();
        assert_eq!(preset.name, "Dialogue");
        assert_eq!(preset.font_family, "Example Sans");
        assert_eq!(preset.font_postscript, "ExampleSans-Bold");
        assert_eq!(preset.font_size, 18.0);
        assert_eq!(preset.tracking, 25);
        assert_eq!(preset.horizontal_scale, 90.0);
        assert!(preset.bold);
        assert_eq!(preset.text_color, [26, 51, 77, 255]);
        assert_eq!(preset.measurement_system, MeasurementSystem::Optical);
        assert_eq!(preset.anti_alias, "sharp");
        assert_eq!(preset.stroke_width, 2.0);
        assert_eq!(preset.stroke_color, [255, 255, 255, 128]);
        assert_eq!(state.scan_document_text_styles(&doc), (0, 0));
    }

    #[test]
    fn document_style_scan_reports_strokes_not_representable_by_type_styles() {
        let text_layer = TextLayer {
            text: "Outlined".into(),
            runs: vec![TextRun {
                len: "Outlined".len(),
                style: CharStyle { font_family: "Unsupported Stroke Test".into(), ..CharStyle::default() },
            }],
            ..TextLayer::default()
        };
        let mut layer = photocraft_doc::Layer::new("Outlined", LayerContent::Text(text_layer));
        layer.effects.enabled = true;
        layer.effects.items.push(Effect::Stroke(photocraft_doc::effects::StrokeFx {
            common: photocraft_doc::effects::FxCommon::new(photocraft_doc::BlendMode::Normal, 1.0),
            size: 2.0,
            position: photocraft_doc::effects::StrokePosition::Inside,
            paint: FxPaint::Color(photocraft_color::Color::WHITE),
            overprint: false,
        }));
        let mut doc = Document::new(
            "Unsupported stroke",
            photocraft_geom::Size::new(100, 100),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        );
        doc.layers.push(layer);

        assert_eq!(TypesetterState::default().scan_document_text_styles(&doc), (1, 1));
    }

    #[test]
    fn tpl_import_maps_photoshop_text_preset_and_reports_skips() {
        use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value};

        let character = Descriptor::new("TxtS")
            .with("fontPostScriptName", Value::Text(UnicodeString::new_nul("ExampleSans-Bold")))
            .with("FntN", Value::Text(UnicodeString::new_nul("Example Sans")))
            .with("FntS", Value::Text(UnicodeString::new_nul("Bold")))
            .with("Sz  ", Value::UnitFloat { unit: *b"#Pnt", value: 18.0 })
            .with("Ldng", Value::UnitFloat { unit: *b"#Pnt", value: 22.0 })
            .with("autoLeading", Value::Boolean(false))
            .with("Trck", Value::Integer(25))
            .with("HrzS", Value::Double(92.0))
            .with("AntA", Value::Integer(16))
            .with("syntheticItalic", Value::Boolean(true))
            .with("fontCaps", Value::Enumerated { type_id: Id::new("fontCaps"), value: Id::new("AllCaps") })
            .with("AtKr", Value::Enumerated { type_id: Id::new("AtKr"), value: Id::new("opticalKern") })
            .with(
                "Clr ",
                Value::Descriptor(
                    Descriptor::new("RGBC")
                        .with("Rd  ", Value::Double(200.4))
                        .with("Grn ", Value::Double(30.0))
                        .with("Bl  ", Value::Double(10.0)),
                ),
            );
        let type_tool = Descriptor::new("typeCreateOrEditTool")
            .with(
                "textToolCharacterOptions",
                Value::Descriptor(Descriptor::new("textToolCharacterOptions").with("TxtS", Value::Descriptor(character.clone()))),
            )
            .with(
                "textToolParagraphOptions",
                Value::Descriptor(
                    Descriptor::new("textToolParagraphOptions")
                        .with("Ornt", Value::Enumerated { type_id: Id::new("Ornt"), value: Id::new("Hrzn") }),
                ),
            );
        let vertical_tool = Descriptor::new("typeVerticalCreateOrEditTool")
            .with(
                "textToolCharacterOptions",
                Value::Descriptor(Descriptor::new("textToolCharacterOptions").with("TxtS", Value::Descriptor(character))),
            )
            .with(
                "textToolParagraphOptions",
                Value::Descriptor(
                    Descriptor::new("textToolParagraphOptions")
                        .with("Ornt", Value::Enumerated { type_id: Id::new("Ornt"), value: Id::new("Vrtc") }),
                ),
            );

        let mut section = Vec::new();
        section.extend_from_slice(&3u32.to_be_bytes());
        append_tpl_entry(&mut section, "$$$/Presets/Tools/Text_tpl=Dialogue", type_tool);
        append_tpl_entry(&mut section, "Brush", Descriptor::new("paintbrushTool"));
        append_tpl_entry(&mut section, "Vertical", vertical_tool);
        let data = tpl_file(section);

        let mut state = TypesetterState::default();
        let report = state.import_tool_presets_tpl(&data, "Chapter").unwrap();
        assert_eq!(report, TplImportReport { imported: 2, skipped: 1, vertical_imported: 1 });
        let imported = state.presets.iter().find(|preset| preset.name == "Dialogue").unwrap();
        assert_eq!(imported.name, "Dialogue");
        assert_eq!(imported.folder.as_deref(), Some("Chapter"));
        assert_eq!(imported.font_family, "Example Sans");
        assert_eq!(imported.font_postscript, "ExampleSans-Bold");
        assert_eq!(imported.font_style, "Bold");
        assert!(imported.bold);
        assert!(imported.italic);
        assert!(imported.all_caps);
        assert_eq!(imported.font_size, 18.0);
        assert_eq!(imported.leading, 22.0);
        assert_eq!(imported.tracking, 25);
        assert_eq!(imported.horizontal_scale, 92.0);
        assert_eq!(imported.text_color, [200, 30, 10, 255]);
        assert_eq!(imported.anti_alias, "sharp");
        assert_eq!(imported.measurement_system, MeasurementSystem::Optical);
        assert_eq!(imported.stroke_width, 0.0);
        assert!(state.presets.iter().any(|preset| preset.name == "Vertical" && preset.orientation == "vertical"));
    }

    #[test]
    fn tpl_import_reads_float_rgb_and_rejects_incomplete_text_presets() {
        use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value};

        let complete = Descriptor::new("TxtS")
            .with("FntN", Value::Text(UnicodeString::new_nul("Float Colour")))
            .with("Sz  ", Value::UnitFloat { unit: *b"#Pnt", value: 12.0 })
            .with(
                "Clr ",
                Value::Descriptor(
                    Descriptor::new("RGBC")
                        .with("redFloat", Value::Double(0.5))
                        .with("greenFloat", Value::Double(0.0))
                        .with("blueFloat", Value::Double(1.0)),
                ),
            );
        // No font name or size: Photoshop writes these for incomplete presets, and TypeR
        // skips them rather than inventing a default face.
        let incomplete = Descriptor::new("TxtS").with("FntN", Value::Text(UnicodeString::new_nul("Only a name")));
        let make_tool = |character: Descriptor| {
            Descriptor::new("typeCreateOrEditTool").with(
                "textToolCharacterOptions",
                Value::Descriptor(Descriptor::new("textToolCharacterOptions").with("TxtS", Value::Descriptor(character))),
            )
        };

        let mut section = Vec::new();
        section.extend_from_slice(&2u32.to_be_bytes());
        append_tpl_entry(&mut section, "Float", make_tool(complete));
        append_tpl_entry(&mut section, "Incomplete", make_tool(incomplete));

        let mut state = TypesetterState::default();
        let report = state.import_tool_presets_tpl(&tpl_file(section), "Float").unwrap();
        assert_eq!(report, TplImportReport { imported: 1, skipped: 1, vertical_imported: 0 });
        assert_eq!(state.presets.last().unwrap().text_color, [128, 0, 255, 255]);
    }

    #[test]
    fn punctuation_only_lines_scale_size_and_leading() {
        assert!(is_punctuation_only_text("?!"));
        assert!(is_punctuation_only_text("  ?! …  "));
        assert!(!is_punctuation_only_text("?! ok"));
        assert!(!is_punctuation_only_text(""));
        assert!(!is_punctuation_only_text("   "));

        let mut preset = TextStylePreset { font_size: 20.0, leading: 24.0, punctuation_only_scale: 2.0, ..TextStylePreset::default() };
        assert_eq!(preset.scale_for_punctuation_only("?! …"), 2.0);
        assert_eq!((preset.font_size, preset.leading), (40.0, 48.0));

        let mut ordinary = TextStylePreset { font_size: 20.0, leading: 24.0, punctuation_only_scale: 3.0, ..TextStylePreset::default() };
        assert_eq!(ordinary.scale_for_punctuation_only("Wait!"), 1.0);
        assert_eq!((ordinary.font_size, ordinary.leading), (20.0, 24.0));

        let json = serde_json::json!({"styles": [{"name": "Shout", "punctuationOnlyScale": 3}]});
        let mut state = TypesetterState::default();
        state.import_styles_json(&json.to_string()).unwrap();
        assert_eq!(state.presets.last().unwrap().punctuation_only_scale, 3.0);
        let exported = state.export_styles_json(None).unwrap();
        assert!(exported.contains("\"punctuationOnlyScale\": 3.0"));
    }

    #[test]
    fn text_shape_candidates_include_balanced_and_contour_layouts() {
        let bounds = Rect::new(0, 0, 220, 180);
        let mut pixels = vec![0.0; bounds.width() as usize * bounds.height() as usize];
        for y in 0..180 {
            for x in 0..220 {
                let dx = (x as f32 - 110.0) / 105.0;
                let dy = (y as f32 - 90.0) / 85.0;
                if dx * dx + dy * dy <= 1.0
                    && let Some(pixel) = pixels.get_mut(y * 220 + x)
                {
                    *pixel = 1.0;
                }
            }
        }
        let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &pixels);
        let preset = TextStylePreset { font_family: "Inter".into(), font_size: 20.0, leading: 24.0, ..TextStylePreset::default() };

        let candidates = text_shape_candidates(
            "This is a sample line with enough words to need several balanced lines inside a bubble",
            &preset,
            Some(&selection),
            bounds,
            72.0,
            true,
            75.0,
        )
        .unwrap();

        assert!(candidates.iter().any(|candidate| candidate.kind == TextShapeKind::Original));
        assert!(candidates.iter().any(|candidate| candidate.kind == TextShapeKind::Balanced));
        assert!(candidates.iter().any(|candidate| candidate.kind == TextShapeKind::ContourFit));
        let mut texts: Vec<_> = candidates.iter().map(|candidate| candidate.text.as_str()).collect();
        texts.sort_unstable();
        texts.dedup();
        assert_eq!(texts.len(), candidates.len(), "candidates must be unique");
    }

    #[test]
    fn paragraph_and_orientation_settings_round_trip_through_json() {
        let json = serde_json::json!({"styles": [{
            "name": "Narration",
            "orientation": "vertical",
            "alignment": "right",
            "direction": "rtl",
            "firstLineIndent": 12.0,
            "startIndent": 4.0,
            "endIndent": 6.0,
            "spaceBefore": 2.0,
            "spaceAfter": 3.0,
            "hyphenate": true
        }]});
        let mut state = TypesetterState::default();
        state.import_styles_json(&json.to_string()).unwrap();
        let preset = state.presets.last().unwrap();
        assert_eq!(preset.orientation, "vertical");
        assert_eq!(preset.alignment, "right");
        assert_eq!(preset.direction, "rtl");
        assert_eq!((preset.first_line_indent, preset.start_indent, preset.end_indent), (12.0, 4.0, 6.0));
        assert_eq!((preset.space_before, preset.space_after), (2.0, 3.0));
        assert!(preset.hyphenate);

        let exported: serde_json::Value = serde_json::from_str(&state.export_styles_json(None).unwrap()).unwrap();
        let style = exported.get("styles").and_then(serde_json::Value::as_array).and_then(|styles| styles.last()).unwrap();
        assert_eq!(style.get("orientation").and_then(serde_json::Value::as_str), Some("vertical"));
        assert_eq!(style.get("hyphenate").and_then(serde_json::Value::as_bool), Some(true));
    }

    #[test]
    fn tpl_import_rejects_libraries_without_text_presets() {
        use photocraft_psd::descriptor::Descriptor;

        let mut section = 1u32.to_be_bytes().to_vec();
        append_tpl_entry(&mut section, "Brush", Descriptor::new("paintbrushTool"));
        assert!(TypesetterState::default().import_tool_presets_tpl(&tpl_file(section), "Brushes").is_err());
    }

    #[test]
    fn contour_fit_leaves_disconnected_bubbles_to_the_fallback() {
        let bounds = Rect::new(0, 0, 200, 120);
        let mut pixels = vec![0.0; bounds.width() as usize * bounds.height() as usize];
        for y in 20..100 {
            for x in 10..75 {
                if let Some(pixel) = pixels.get_mut(y * bounds.width() as usize + x) {
                    *pixel = 1.0;
                }
            }
            for x in 125..190 {
                if let Some(pixel) = pixels.get_mut(y * bounds.width() as usize + x) {
                    *pixel = 1.0;
                }
            }
        }
        let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &pixels);
        let mut preset = TextStylePreset::default();

        assert_eq!(
            fit_script_text_to_selection("Hello bubble", &mut preset, &selection, bounds, 72.0, true, 75.0).unwrap(),
            None
        );
        assert_eq!(preset.font_size, TextStylePreset::default().font_size);
    }

    #[test]
    fn contour_fit_leaves_vertically_separated_bubbles_to_the_fallback() {
        let bounds = Rect::new(0, 0, 160, 140);
        let mut pixels = vec![0.0; bounds.width() as usize * bounds.height() as usize];
        for y in 5..45 {
            for x in 50..110 {
                if let Some(pixel) = pixels.get_mut(y * bounds.width() as usize + x) {
                    *pixel = 1.0;
                }
            }
        }
        for y in 95..135 {
            for x in 50..110 {
                if let Some(pixel) = pixels.get_mut(y * bounds.width() as usize + x) {
                    *pixel = 1.0;
                }
            }
        }
        let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &pixels);
        let mut preset = TextStylePreset::default();

        assert_eq!(
            fit_script_text_to_selection("Hello bubble", &mut preset, &selection, bounds, 72.0, true, 75.0).unwrap(),
            None
        );
    }

    #[test]
    fn test_page_detection_and_grouping() {
        let script = "Page 1\nHero: Hello!\n\nPage 2\n- Next page line";
        let mut state = TypesetterState::default();
        state.load_script(script);
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines[0].page, 1);
        assert_eq!(state.lines[1].page, 2);

        let p2_lines = state.lines_for_page(2);
        assert_eq!(p2_lines.len(), 1);
        assert_eq!(p2_lines[0].1.text, "Next page line");
    }

    #[test]
    fn test_line_spacing_sync() {
        let mut preset = TextStylePreset {
            font_size: 20.0,
            leading: 24.0,
            ..TextStylePreset::default()
        };
        preset.sync_leading_with_size(30.0);
        assert_eq!(preset.font_size, 30.0);
        assert_eq!(preset.leading, 36.0); // 24 * (30/20) = 36
    }

    #[test]
    fn test_duplicate_folder() {
        let mut state = TypesetterState::default();
        let initial_count = state.presets.len();
        let manga_styles_count = state.presets.iter().filter(|p| p.folder.as_deref() == Some("Манга")).count();
        assert!(manga_styles_count > 0);

        let dup = state.duplicate_folder("Манга", "Манга (Копия)");
        assert_eq!(dup, manga_styles_count);
        assert_eq!(state.presets.len(), initial_count + manga_styles_count);
    }

    #[test]
    fn test_export_and_import_json() {
        let state = TypesetterState::default();
        let json = state.export_styles_json(None).unwrap();
        assert!(json.contains("folders"));
        assert!(json.contains("styles"));

        let mut new_state = TypesetterState {
            presets: Vec::new(),
            ..TypesetterState::default()
        };
        let imported = new_state.import_styles_json(&json).unwrap();
        assert_eq!(imported, state.presets.len());
        for i in 0..state.presets.len() {
            assert_eq!(new_state.presets[i].text_color, state.presets[i].text_color);
            assert_eq!(new_state.presets[i].measurement_system, state.presets[i].measurement_system);
        }
    }

    #[test]
    fn test_import_typer_export_json_sample() {
        let json_data = r#"{
            "folders": [{"name": "Манга", "id": "zbtrk7i9"}],
            "styles": [
                {
                    "name": "Мысли",
                    "folder": "zbtrk7i9",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "Kurai Voice 2",
                                    "fontPostScriptName": "KuraiVoice2-Italic",
                                    "fontStyleName": "Italic",
                                    "size": 26,
                                    "syntheticBold": false,
                                    "syntheticItalic": false,
                                    "fontCaps": "allCaps"
                                }
                            }],
                            "paragraphStyleRange": [{
                                "paragraphStyle": {
                                    "autoLeadingPercentage": 1.2
                                }
                            }]
                        }
                    },
                    "stroke": {"enabled": false, "size": 0, "color": {"r": 255, "g": 255, "b": 255}}
                },
                {
                    "name": "Обычный выделенный",
                    "folder": "zbtrk7i9",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "Kurai Voice 2",
                                    "fontPostScriptName": "KuraiVoice2-Bold",
                                    "fontStyleName": "Bold",
                                    "size": 36,
                                    "syntheticBold": false,
                                    "syntheticItalic": false,
                                    "fontCaps": "allCaps"
                                }
                            }]
                        }
                    }
                },
                {
                    "name": "Жёсткий Крик",
                    "folder": "zbtrk7i9",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "v_CCWiccanSansInt",
                                    "fontPostScriptName": "v_CCWiccanSansInt",
                                    "fontStyleName": "Regular",
                                    "size": 26,
                                    "syntheticBold": true,
                                    "syntheticItalic": true,
                                    "fontCaps": "allCaps"
                                }
                            }]
                        }
                    },
                    "stroke": {"enabled": false, "size": 0}
                }
            ]
        }"#;

        let mut state = TypesetterState {
            presets: Vec::new(),
            ..TypesetterState::default()
        };
        let count = state.import_styles_json(json_data).unwrap();
        assert_eq!(count, 3);

        // Мысли
        assert_eq!(state.presets[0].name, "Мысли");
        assert_eq!(state.presets[0].font_family, "Kurai Voice 2");
        assert_eq!(state.presets[0].font_postscript, "KuraiVoice2-Italic");
        assert_eq!(state.presets[0].font_style, "Italic");
        assert!(!state.presets[0].bold);
        assert!(state.presets[0].italic);
        assert_eq!(state.presets[0].stroke_width, 0.0);
        assert_eq!(state.presets[0].leading, 31.0); // 26 * 1.2 = 31.2 -> 31.0

        // Обычный выделенный
        assert_eq!(state.presets[1].name, "Обычный выделенный");
        assert_eq!(state.presets[1].font_style, "Bold");
        assert!(state.presets[1].bold);
        assert!(!state.presets[1].italic);
        assert_eq!(state.presets[1].leading, 43.0); // 36 * 1.2 = 43.2 -> 43.0

        // Жёсткий Крик
        assert_eq!(state.presets[2].name, "Жёсткий Крик");
        assert!(state.presets[2].bold);
        assert!(state.presets[2].italic);
        assert_eq!(state.presets[2].stroke_width, 0.0);
    }

    #[test]
    fn test_import_real_file_if_present() {
        if let Ok(content) = std::fs::read_to_string(r"d:\Work\umamusume\TypeR_Export.json") {
            let mut state = TypesetterState {
                presets: Vec::new(),
                ..TypesetterState::default()
            };
            let count = state.import_styles_json(&content).unwrap();
            assert!(count > 0);
            for p in &state.presets {
                if p.font_style.to_lowercase().contains("bold") {
                    assert!(p.bold, "Preset '{}' with style '{}' must be bold", p.name, p.font_style);
                }
                if p.font_style.to_lowercase().contains("italic") {
                    assert!(p.italic, "Preset '{}' with style '{}' must be italic", p.name, p.font_style);
                }
            }
        }
    }

    #[test]
    fn test_typer_font_color_and_measurement_system_import_export() {
        let json_data = r##"{
            "folders": [{"name": "Тест", "id": "f1"}],
            "styles": [
                {
                    "name": "RGB Optical",
                    "folder": "f1",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "Inter",
                                    "size": 20,
                                    "color": {"red": 255, "green": 128, "blue": 64},
                                    "opacity": 80,
                                    "autoKern": "opticalKern"
                                }
                            }]
                        }
                    }
                },
                {
                    "name": "Grayscale Metric",
                    "folder": "f1",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "Inter",
                                    "size": 24,
                                    "color": {"gray": 100},
                                    "autoKern": "metricsKern"
                                }
                            }]
                        }
                    }
                },
                {
                    "name": "Hex Metric",
                    "folder": "f1",
                    "textColor": "#112233aa",
                    "measurementSystem": "metric",
                    "textProps": {
                        "layerText": {
                            "textStyleRange": [{
                                "textStyle": {
                                    "fontName": "Inter",
                                    "size": 18
                                }
                            }]
                        }
                    }
                }
            ]
        }"##;

        let mut state = TypesetterState {
            presets: Vec::new(),
            ..TypesetterState::default()
        };
        let count = state.import_styles_json(json_data).unwrap();
        assert_eq!(count, 3);

        // 1. RGB Optical with 80% opacity: [255, 128, 64, 204]
        assert_eq!(state.presets[0].name, "RGB Optical");
        assert_eq!(state.presets[0].text_color[0], 255);
        assert_eq!(state.presets[0].text_color[1], 128);
        assert_eq!(state.presets[0].text_color[2], 64);
        assert_eq!(state.presets[0].text_color[3], 204);
        assert_eq!(state.presets[0].measurement_system, MeasurementSystem::Optical);

        // 2. Grayscale 100% Metric: [255, 255, 255, 255]
        assert_eq!(state.presets[1].name, "Grayscale Metric");
        assert_eq!(state.presets[1].text_color, [255, 255, 255, 255]);
        assert_eq!(state.presets[1].measurement_system, MeasurementSystem::Metric);

        // 3. Hex Metric: #112233aa
        assert_eq!(state.presets[2].name, "Hex Metric");
        assert_eq!(state.presets[2].text_color, [0x11, 0x22, 0x33, 0xaa]);
        assert_eq!(state.presets[2].measurement_system, MeasurementSystem::Metric);

        // Roundtrip export
        let exported = state.export_styles_json(None).unwrap();
        let mut reimported = TypesetterState {
            presets: Vec::new(),
            ..TypesetterState::default()
        };
        reimported.import_styles_json(&exported).unwrap();
        assert_eq!(reimported.presets.len(), 3);
        assert_eq!(reimported.presets[0].text_color, state.presets[0].text_color);
        assert_eq!(reimported.presets[0].measurement_system, MeasurementSystem::Optical);
        assert_eq!(reimported.presets[1].text_color, state.presets[1].text_color);
        assert_eq!(reimported.presets[1].measurement_system, MeasurementSystem::Metric);
        assert_eq!(reimported.presets[2].text_color, state.presets[2].text_color);
        assert_eq!(reimported.presets[2].measurement_system, MeasurementSystem::Metric);
    }

    #[test]
    fn test_wrap_text_to_bubble() {
        let short_text = "Короткий текст";
        // Wide bubble: no wrap
        let res = wrap_text_to_bubble(short_text, 300.0, 14.0, 72.0, 100.0);
        assert_eq!(res, short_text);

        // Long text in narrow bubble: should wrap into multiple lines with \n
        let long_text = "Это длинная реплика персонажа которая должна красиво переноситься внутри баббла";
        let wrapped = wrap_text_to_bubble(long_text, 120.0, 14.0, 72.0, 100.0);
        assert!(wrapped.contains('\n'));
        let lines: Vec<&str> = wrapped.lines().collect();
        assert!(lines.len() >= 2);
        // Words should be preserved
        let words_orig: Vec<&str> = long_text.split_whitespace().collect();
        let words_wrapped: Vec<&str> = wrapped.split_whitespace().collect();
        assert_eq!(words_orig, words_wrapped);
    }
}
