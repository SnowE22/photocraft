use serde::{Deserialize, Serialize};

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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptLine {
    pub raw: String,
    pub text: String,
    pub is_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypesetterState {
    pub lines: Vec<ScriptLine>,
    pub current_line_idx: usize,
    pub presets: Vec<TextStylePreset>,
    pub selected_preset_idx: usize,
}

impl Default for TypesetterState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            current_line_idx: 0,
            presets: vec![
                TextStylePreset::default(),
                TextStylePreset {
                    name: "Крик / SFX".to_string(),
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
        }
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

            // Префикс папки, если стиль находится внутри папки
            let folder_id = item.get("folder").and_then(|v| v.as_str());
            let name = match folder_id.and_then(|id| folders_map.get(id)) {
                Some(folder_name) => format!("[{}] {}", folder_name, raw_name),
                None => raw_name.to_string(),
            };

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

            let leading = text_style
                .and_then(|ts| ts.get("leading"))
                .or_else(|| item.get("leading"))
                .and_then(|v| v.as_f64())
                .unwrap_or((font_size * 1.15) as f64) as f32;

            let tracking = text_style
                .and_then(|ts| ts.get("tracking"))
                .or_else(|| item.get("tracking"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;

            let horizontal_scale = text_style
                .and_then(|ts| ts.get("horizontalScale"))
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0) as f32;

            let bold = text_style
                .and_then(|ts| ts.get("syntheticBold"))
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| font_style.to_lowercase().contains("bold"));

            let italic = text_style
                .and_then(|ts| ts.get("syntheticItalic"))
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| font_style.to_lowercase().contains("italic"));

            let all_caps = text_style
                .and_then(|ts| ts.get("fontCaps"))
                .and_then(|v| v.as_str())
                .map(|caps| caps == "allCaps")
                .unwrap_or(false);

            let stroke_width = if let Some(stroke_val) = item.get("stroke_width").or_else(|| item.get("stroke")) {
                stroke_val.as_f64().unwrap_or(2.0) as f32
            } else {
                2.0
            };

            self.presets.push(TextStylePreset {
                name,
                folder: folder_id.and_then(|id| folders_map.get(id)).cloned(),
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
                stroke_color: [255, 255, 255, 255],
            });
            count += 1;
        }

        if count > 0 {
            self.selected_preset_idx = self.presets.len() - 1;
        }

        Ok(count)
    }

    /// Очищает и загружает сырой текст скрипта манги в очередь реплик.
    pub fn load_script(&mut self, content: &str) {
        self.lines = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|line| {
                let cleaned = clean_script_line(line);
                ScriptLine {
                    raw: line.to_string(),
                    text: cleaned,
                    is_applied: false,
                }
            })
            .collect();
        self.current_line_idx = 0;
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
            self.lines.get(self.current_line_idx)
        } else {
            None
        }
    }

    pub fn active_preset(&self) -> Option<&TextStylePreset> {
        self.presets.get(self.selected_preset_idx)
    }
}

/// Удаляет типичные префиксы реплик сценария: тире, нумерацию и имена персонажей ("HERO: ...").
pub fn clean_script_line(line: &str) -> String {
    let s = line.trim();
    let s = s.strip_prefix("- ").or_else(|| s.strip_prefix("— ")).unwrap_or(s);

    if let Some((_, text)) = s.split_once(':') {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    s.to_string()
}

/// Вычисляет геометрический центр ограничивающего прямоугольника [min_x, min_y, max_x, max_y] баббла.
pub fn calculate_bubble_center(bounds: [f32; 4]) -> [f32; 2] {
    let [min_x, min_y, max_x, max_y] = bounds;
    [min_x + (max_x - min_x) * 0.5, min_y + (max_y - min_y) * 0.5]
}

#[cfg(test)]
mod tests {
    use super::*;

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
}