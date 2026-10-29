use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeasurementSystem {
    Metric,
    Optical,
}

impl Default for MeasurementSystem {
    fn default() -> Self {
        Self::Optical
    }
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
}

fn default_anti_alias() -> String {
    "crisp".to_string()
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptLine {
    pub raw: String,
    pub text: String,
    pub is_applied: bool,
    pub page: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypesetterOptions {
    pub auto_page_switch: bool,
    pub line_spacing_sync: bool,
    pub strip_character_names: bool,
    pub strip_dash_prefix: bool,
    pub optical_baseline_factor: f32,
    pub hanging_punctuation: bool,
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
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some([r, g, b, 255])
            } else if s.len() == 8 {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                let a = u8::from_str_radix(&s[6..8], 16).ok()?;
                Some([r, g, b, a])
            } else {
                None
            }
        }
        _ => None,
    }
}

impl TypesetterState {
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
                if let Some(ref mut col) = col {
                    if let Some(op) = text_style.and_then(|ts| ts.get("opacity")).or_else(|| item.get("opacity")).and_then(|v| v.as_f64()) {
                        if (0.0..=100.0).contains(&op) {
                            col[3] = ((op / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
                        }
                    }
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
            } else if let Some(hp) = item.get("hangingPunctuation").or_else(|| item.get("hanging_punctuation")).and_then(|v| v.as_bool()) {
                hp
            } else {
                true
            };

            let anti_alias = if let Some(aa) = item.get("antiAlias").or_else(|| item.get("anti_alias")).and_then(|v| v.as_str()) {
                aa.to_string()
            } else if let Some(aa) = text_style.and_then(|ts| ts.get("antiAlias")).and_then(|v| v.as_str()) {
                aa.to_string()
            } else {
                "crisp".to_string()
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
            });
            count += 1;
        }

        if count > 0 {
            self.selected_preset_idx = self.presets.len() - 1;
        }

        Ok(count)
    }

    /// Экспортирует стили в JSON-строку формата TypeR (совместимо с Photoshop TypeR).
    pub fn export_styles_json(&self, folder_filter: Option<&str>) -> Result<String, String> {
        let mut folders_map = std::collections::HashMap::new();
        let mut folders_arr = Vec::new();

        let mut folder_id_counter = 1;
        for preset in &self.presets {
            if let Some(folder) = &preset.folder {
                if !folders_map.contains_key(folder) {
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
        }

        let mut styles_arr = Vec::new();
        for preset in &self.presets {
            if let Some(target_f) = folder_filter {
                if preset.folder.as_deref() != Some(target_f) {
                    continue;
                }
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
                "measurementSystem": preset.measurement_system.as_str()
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

            let cleaned = clean_script_line_with_options(trimmed, &self.options);
            if !cleaned.is_empty() {
                self.lines.push(ScriptLine {
                    raw: trimmed.to_string(),
                    text: cleaned,
                    is_applied: false,
                    page,
                });
            }
        }

        self.current_line_idx = 0;
        self.current_page = self.lines.first().map_or(1, |l| l.page);
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
            let next = self.lines.get(self.current_line_idx);
            if self.options.auto_page_switch {
                if let Some(nl) = next {
                    self.current_page = nl.page;
                }
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
            if self.options.auto_page_switch {
                if let Some(line) = self.lines.get(self.current_line_idx) {
                    self.current_page = line.page;
                }
            }
        }
    }

    pub fn set_current_line(&mut self, idx: usize) {
        if idx < self.lines.len() {
            self.current_line_idx = idx;
            if let Some(line) = self.lines.get(idx) {
                self.current_page = line.page;
            }
        }
    }

    pub fn active_preset(&self) -> Option<&TextStylePreset> {
        self.presets.get(self.selected_preset_idx)
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
            if let Ok(num) = digits.parse::<usize>() {
                if num > 0 {
                    return Some(num);
                }
            }
        }
    }
    None
}

/// Очищает строку сценария с учетом заданных опций.
pub fn clean_script_line_with_options(line: &str, options: &TypesetterOptions) -> String {
    let mut s = line.trim();

    if options.strip_dash_prefix {
        if let Some(stripped) = s.strip_prefix("- ").or_else(|| s.strip_prefix("— ")) {
            s = stripped.trim();
        }
    }

    if options.strip_character_names {
        if let Some((_, text)) = s.split_once(':') {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                s = trimmed;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_bubble_center_calculation() {
        let center = calculate_bubble_center([100.0, 100.0, 300.0, 500.0]);
        assert_eq!(center, [200.0, 300.0]);
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
