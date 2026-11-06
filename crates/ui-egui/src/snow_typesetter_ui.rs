use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    Window, vec2,
};
use photocraft_engine::typesetter::{TextShapeCandidate, TextShapeKind, TextStylePreset, TypesetterState};
use serde::{Deserialize, Serialize};

use crate::theme::Tokens;
use crate::widgets;

#[derive(PartialEq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TypesetterDockMode {
    Floating,
    DockRight,
    DockLeft,
    DockBottom,
}

impl Default for TypesetterDockMode {
    fn default() -> Self {
        Self::Floating
    }
}

#[derive(PartialEq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TypesetterTab {
    Queue,
    FullText,
    Styles,
    Settings,
}

pub struct TypesetterAction {
    pub text: String,
    pub preset: TextStylePreset,
}

fn default_true() -> bool {
    true
}

fn default_script_list_height() -> f32 {
    160.0
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn read_tpl_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let max_bytes = photocraft_engine::typesetter::MAX_TPL_FILE_BYTES;
    let limit = max_bytes as u64;
    if file.metadata()?.len() > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Photoshop TPL file exceeds the size limit"));
    }
    let mut reader = std::io::Read::take(file, limit + 1);
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut data)?;
    if data.len() > max_bytes {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Photoshop TPL file exceeds the size limit"));
    }
    Ok(data)
}

/// Point size for style names drawn in their own face on the style cards.
const STYLE_NAME_PT: f32 = 13.0;

/// Extra leading between script rows in the unified editor (logical pixels). egui's multiline
/// `TextEdit` packs rows at the font's own line height, which is too tight for reading a script.
const SCRIPT_LINE_SPACING: f32 = 1.0;
/// A multi-line preview form of a candidate for the TextShapeR cards: preserves line breaks
/// and truncates long lines so a long dialogue cannot break the card layout.
fn preview_short_text(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let total_lines = lines.len();
    let mut out_lines = Vec::new();
    let max_lines = 16;
    for l in lines.into_iter().take(max_lines) {
        if l.chars().count() > 36 {
            let mut short: String = l.chars().take(35).collect();
            short.push('…');
            out_lines.push(short);
        } else {
            out_lines.push(l.to_string());
        }
    }
    if total_lines > max_lines {
        out_lines.push("…".to_string());
    }
    out_lines.join("\n")
}

#[derive(Serialize, Deserialize)]
struct TypesetterPersistedState {
    pub dock_mode: TypesetterDockMode,
    pub state: TypesetterState,
    pub selected_folder: Option<String>,
    #[serde(default)]
    pub custom_storage_dir: Option<std::path::PathBuf>,
    #[serde(default)]
    pub favorite_fonts: Vec<String>,
    #[serde(default = "default_true")]
    pub is_open: bool,
    #[serde(default = "default_script_list_height")]
    pub script_list_height: f32,
    /// The one-time move to the profile store already merged the legacy files.
    #[serde(default)]
    pub legacy_merged: bool,
}

pub fn typesetter_storage_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir).join("typesetter_state.json"));
    }
    // Portable mode is opt-in only (`portable.txt` next to the exe): an existing state file next
    // to whichever exe was launched must NOT move the store, or updating the exe from another
    // folder loses favorites and styles.
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
        && parent.join("portable.txt").exists()
    {
        return Some(parent.join("typesetter_state.json"));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let p = std::path::PathBuf::from(appdata).join("Photocraft");
            let _ = std::fs::create_dir_all(&p);
            return Some(p.join("typesetter_state.json"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let p = std::path::PathBuf::from(home).join("Library/Application Support/Photocraft");
            let _ = std::fs::create_dir_all(&p);
            return Some(p.join("typesetter_state.json"));
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            let p = std::path::PathBuf::from(xdg).join("photocraft");
            let _ = std::fs::create_dir_all(&p);
            return Some(p.join("typesetter_state.json"));
        } else if let Some(home) = std::env::var_os("HOME") {
            let p = std::path::PathBuf::from(home).join(".config/photocraft");
            let _ = std::fs::create_dir_all(&p);
            return Some(p.join("typesetter_state.json"));
        }
    }
    None
}

/// Where older builds kept `typesetter_state.json`: next to the exe (the old portable heuristic),
/// the working directory, and the lower-case Roaming folder. Read once for migration, never
/// written again, so an un-starred font cannot come back from a stale copy.
fn legacy_typesetter_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        paths.push(parent.join("typesetter_state.json"));
    }
    paths.push(std::path::PathBuf::from("typesetter_state.json"));
    if let Some(appdata) = std::env::var_os("APPDATA") {
        paths.push(std::path::PathBuf::from(appdata).join("photocraft").join("typesetter_state.json"));
    }
    paths.retain(|p| Some(p) != typesetter_storage_path().as_ref());
    paths
}
pub struct TypesetterUi {
    pub is_open: bool,
    pub dock_mode: TypesetterDockMode,
    pub current_tab: TypesetterTab,
    pub state: TypesetterState,
    pub script_input_buffer: String,
    pub style_filter: String,
    pub line_filter: String,
    pub full_text_filter: String,
    pub selected_folder: Option<String>,
    pub custom_storage_dir: Option<std::path::PathBuf>,
    pub status_message: Option<String>,
    pub status_error: bool,
    pub filter_current_page_only: bool,
    pub editing_preset_idx: Option<usize>,
    pub duplicating_folder: Option<String>,
    pub duplicate_folder_name_buffer: String,
    pub new_preset_name_buffer: String,
    pub show_full_text_window: bool,
    pub creating_new_folder: bool,
    pub new_folder_name_buffer: String,
    pub show_script_editor_modal: bool,
    pub script_editor_buffer: String,
    pub favorite_fonts: Vec<String>,
    pub rendered_this_frame: bool,
    pub scroll_to_active_line: bool,
    pub script_buffer: String,
    pub script_list_height: f32,
    document_style_scan_requested: bool,
    document_style_file_scan_requested: Option<std::path::PathBuf>,
    text_shape_requested: bool,
    text_shape_candidates: Vec<TextShapeCandidate>,
    text_shape_selected: usize,
    show_text_shape_window: bool,
    text_shape_layer: Option<u64>,
    text_shape_apply_requested: Option<(u64, String)>,
    text_shape_preview_preset: Option<TextStylePreset>,
    multi_bubble_captures: Vec<photocraft_raster::Surface>,
    multi_bubble_capture_requested: bool,
    multi_bubble_fill_requested: bool,
    align_requested: bool,
    /// Style to apply to the text layer already selected on the page.
    apply_style_requested: Option<TextStylePreset>,
    /// Pass number in which the auxiliary windows were last drawn, so a frame that renders both
    /// the docked body and the floating one does not open them twice.
    aux_pass: u64,
    /// Whether the help modal dialog is open.
    pub show_help_modal: bool,
    /// The legacy-state migration already ran (persisted, so removed favorites stay removed).
    legacy_merged: bool,
    /// Script line the editor caret was on last frame; when the current line differs, the caret
    /// follows the line (▲/▼, ➔) instead of dragging the selection back to where it was.
    caret_line: Option<usize>,
    /// Pending auto-save dirty flag.
    save_dirty: bool,
    /// Deadline for debounced auto-save.
    save_dirty_until: Option<std::time::Instant>,
}

impl Default for TypesetterUi {
    fn default() -> Self {
        let mut ui = Self {
            is_open: true,
            dock_mode: TypesetterDockMode::Floating,
            current_tab: TypesetterTab::Queue,
            state: TypesetterState::default(),
            script_input_buffer: String::new(),
            style_filter: String::new(),
            line_filter: String::new(),
            full_text_filter: String::new(),
            selected_folder: None,
            custom_storage_dir: None,
            status_message: None,
            status_error: false,
            filter_current_page_only: true,
            editing_preset_idx: None,
            duplicating_folder: None,
            duplicate_folder_name_buffer: String::new(),
            new_preset_name_buffer: String::new(),
            show_full_text_window: false,
            creating_new_folder: false,
            new_folder_name_buffer: String::new(),
            show_script_editor_modal: false,
            script_editor_buffer: String::new(),
            favorite_fonts: Vec::new(),
            rendered_this_frame: false,
            scroll_to_active_line: false,
            show_help_modal: false,
            script_buffer: String::new(),
            script_list_height: 160.0,
            document_style_scan_requested: false,
            document_style_file_scan_requested: None,
            text_shape_requested: false,
            text_shape_candidates: Vec::new(),
            text_shape_selected: 0,
            show_text_shape_window: false,
            text_shape_layer: None,
            text_shape_apply_requested: None,
            text_shape_preview_preset: None,
            multi_bubble_captures: Vec::new(),
            multi_bubble_capture_requested: false,
            multi_bubble_fill_requested: false,
            align_requested: false,
            apply_style_requested: None,
            aux_pass: u64::MAX,
            legacy_merged: false,
            caret_line: None,
            save_dirty: false,
            save_dirty_until: None,
        };
        ui.auto_load();
        ui
    }
}

fn draw_dashed_hline(painter: &egui::Painter, x_start: f32, x_end: f32, y: f32, stroke: Stroke) {
    let mut x = x_start;
    let dash_len = 3.0;
    let gap_len = 3.0;
    while x < x_end {
        let x2 = (x + dash_len).min(x_end);
        painter.line_segment([egui::pos2(x, y), egui::pos2(x2, y)], stroke);
        x += dash_len + gap_len;
    }
}

fn get_system_clipboard_text() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

impl TypesetterUi {
    /// Синхронизирует буфер единого текстового редактора из массива строк реплик.
    pub fn sync_script_buffer_from_state(&mut self) {
        if !self.state.lines.is_empty() {
            self.script_buffer = self.state.lines.iter().map(|l| l.raw.as_str()).collect::<Vec<_>>().join("\n");
        }
    }

    pub fn take_document_style_scan_request(&mut self) -> bool {
        std::mem::take(&mut self.document_style_scan_requested)
    }

    pub fn take_document_style_file_scan_request(&mut self) -> Option<std::path::PathBuf> {
        self.document_style_file_scan_requested.take()
    }

    pub fn take_text_shape_request(&mut self) -> bool {
        std::mem::take(&mut self.text_shape_requested)
    }

    pub fn set_text_shape_candidates(&mut self, candidates: Vec<TextShapeCandidate>, layer: u64, preset: TextStylePreset) {
        self.set_text_shape_candidates_inline(candidates, layer, preset);
        self.show_text_shape_window = !self.text_shape_candidates.is_empty();
    }

    /// Refresh the inline cards without opening the modal preview window.
    pub fn set_text_shape_candidates_inline(&mut self, candidates: Vec<TextShapeCandidate>, layer: u64, preset: TextStylePreset) {
        self.text_shape_selected = self.text_shape_selected.min(candidates.len().saturating_sub(1));
        self.text_shape_candidates = candidates;
        self.text_shape_layer = if self.text_shape_candidates.is_empty() { None } else { Some(layer) };
        self.text_shape_preview_preset = if self.text_shape_candidates.is_empty() { None } else { Some(preset) };
        if self.text_shape_candidates.is_empty() {
            self.show_text_shape_window = false;
        }
    }

    pub fn take_text_shape_apply_request(&mut self) -> Option<(u64, String)> {
        self.text_shape_apply_requested.take()
    }

    pub fn has_text_shape_candidates(&self) -> bool {
        !self.text_shape_candidates.is_empty() && self.text_shape_layer.is_some()
    }

    pub fn first_text_shape_candidate_text(&self) -> Option<String> {
        self.text_shape_candidates.first().map(|candidate| candidate.text.clone())
    }

    pub fn clear_text_shape_candidates(&mut self) {
        self.text_shape_candidates.clear();
        self.text_shape_layer = None;
        self.text_shape_preview_preset = None;
        self.show_text_shape_window = false;
    }

    fn apply_text_shape_candidate(&mut self, index: usize) {
        let Some(layer) = self.text_shape_layer else { return };
        let Some(candidate) = self.text_shape_candidates.get(index) else { return };
        self.text_shape_apply_requested = Some((layer, candidate.text.clone()));
        self.show_text_shape_window = false;
    }

    pub fn is_blocking_canvas_insertion(&self) -> bool {
        self.editing_preset_idx.is_some()
            || self.current_tab == TypesetterTab::Settings
            || self.show_full_text_window
            || self.show_text_shape_window
            || self.show_script_editor_modal
            || self.show_help_modal
            || self.creating_new_folder
            || self.duplicating_folder.is_some()
    }

    pub fn take_apply_style_request(&mut self) -> Option<TextStylePreset> {
        self.apply_style_requested.take()
    }

    pub fn take_align_request(&mut self) -> bool {
        std::mem::take(&mut self.align_requested)
    }

    pub fn take_multi_bubble_capture_request(&mut self) -> bool {
        std::mem::take(&mut self.multi_bubble_capture_requested)
    }

    pub fn take_multi_bubble_fill_request(&mut self) -> bool {
        std::mem::take(&mut self.multi_bubble_fill_requested)
    }

    pub fn push_multi_bubble_capture(&mut self, selection: photocraft_raster::Surface) {
        self.multi_bubble_captures.push(selection);
    }

    pub fn drain_multi_bubble_captures(&mut self) -> Vec<photocraft_raster::Surface> {
        std::mem::take(&mut self.multi_bubble_captures)
    }

    pub fn multi_bubble_count(&self) -> usize {
        self.multi_bubble_captures.len()
    }

    /// Синхронизирует массив строк реплик и метаданных при редактировании единого текстового буфера.
    pub fn sync_state_from_script_buffer(&mut self) {
        let previous_lines = self.state.lines.clone();
        let current_line_idx = self.state.current_line_idx;
        self.state.load_script(&self.script_buffer);
        for (idx, line) in self.state.lines.iter_mut().enumerate() {
            if let Some(previous) = previous_lines.get(idx)
                && previous.raw == line.raw
            {
                line.is_applied = previous.is_applied;
            }
        }
        if !self.state.lines.is_empty() {
            self.state.set_current_line(current_line_idx.min(self.state.lines.len() - 1));
        } else {
            self.state.set_current_line(0);
        }
        self.auto_save();
    }

    /// Splits the current script line before `char_index` (the break preview click), adding the
    /// hyphen and a line break, then rebuilding the queue from the buffer.
    pub fn split_current_line_at(&mut self, char_index: usize, hyphenate: bool) {
        let Some(line) = self.state.lines.get(self.state.current_line_idx) else { return };
        let start: usize = self.state.lines.iter().take(self.state.current_line_idx).map(|l| l.raw.chars().count() + 1).sum();
        let line_len = line.raw.chars().count();
        let (left, right) = crate::snow_breaks_ui::split_line(&line.raw, char_index, hyphenate);
        if right.is_empty() {
            return;
        }
        let buffer: Vec<char> = self.script_buffer.chars().collect();
        if start + line_len > buffer.len() {
            return;
        }
        let mut updated: Vec<char> = buffer.get(..start).map(<[char]>::to_vec).unwrap_or_default();
        updated.extend(left.chars());
        updated.push(char::from(10));
        updated.extend(right.chars());
        let tail = (start + line_len + 1).min(buffer.len());
        updated.extend(buffer.get(tail..).unwrap_or_default());
        self.script_buffer = updated.into_iter().collect();
        self.sync_state_from_script_buffer();
        self.caret_line = None;
    }

    /// Joins the current line with the next one when it ends with a hyphen.
    pub fn merge_current_line(&mut self) {
        let Some(line) = self.state.lines.get(self.state.current_line_idx) else { return };
        let Some(next) = self.state.lines.get(self.state.current_line_idx + 1) else { return };
        if !line.raw.trim_end().ends_with('-') {
            return;
        }
        let start: usize = self.state.lines.iter().take(self.state.current_line_idx).map(|l| l.raw.chars().count() + 1).sum();
        let line_len = line.raw.chars().count();
        let next_len = next.raw.chars().count();
        let merged = crate::snow_breaks_ui::merge_lines(&line.raw, &next.raw);
        let buffer: Vec<char> = self.script_buffer.chars().collect();
        if start + line_len + 1 + next_len > buffer.len() {
            return;
        }
        let mut updated: Vec<char> = buffer.get(..start).map(<[char]>::to_vec).unwrap_or_default();
        updated.extend(merged.chars());
        updated.extend(buffer.get(start + line_len + 1 + next_len..).unwrap_or_default());
        self.script_buffer = updated.into_iter().collect();
        self.sync_state_from_script_buffer();
        self.caret_line = None;
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Вставить сценарий напрямую из буфера обмена.
    pub fn paste_script_from_clipboard(&mut self) {
        let text_opt = get_system_clipboard_text();
        if let Some(text) = text_opt {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                self.script_input_buffer = text.clone();
                self.script_editor_buffer = text.clone();
                self.state.load_script(&text);
                self.sync_script_buffer_from_state();
                self.auto_save();
                self.status_message = Some(format!("Загружено реплик из буфера: {}", self.state.lines.len()));
                self.status_error = false;
                return;
            }
        }
        // Если буфер пуст или недоступен — открываем окно редактора для ручного ввода
        self.open_script_editor();
        self.status_message = Some("Буфер пуст. Вставьте текст вручную в окно редактора".into());
        self.status_error = false;
    }

    /// Открыть модальное окно редактора сценария.
    pub fn open_script_editor(&mut self) {
        if self.script_editor_buffer.trim().is_empty() && !self.state.lines.is_empty() {
            // Если буфер редактора пуст, подставляем текущие реплики
            self.script_editor_buffer = self.state.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        }
        self.show_script_editor_modal = true;
    }

    /// Диалог выбора .txt файла сценария.
    pub fn open_script_file_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Текстовые файлы (*.txt)", &["txt"])
            .set_title("Выберите файл сценария")
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    self.script_input_buffer = content.clone();
                    self.script_editor_buffer = content.clone();
                    self.state.load_script(&content);
                    self.sync_script_buffer_from_state();
                    self.auto_save();
                    self.status_message = Some(format!("Загружено реплик из файла: {}", self.state.lines.len()));
                    self.status_error = false;
                }
                Err(err) => {
                    self.status_message = Some(format!("Ошибка чтения файла: {}", err));
                    self.status_error = true;
                }
            }
        }
    }

    /// Текущий эффективный путь к файлу пресетов (пользовательская папка или системная).
    pub fn storage_path(&self) -> Option<std::path::PathBuf> {
        if let Some(custom_dir) = &self.custom_storage_dir {
            return Some(custom_dir.join("typesetter_state.json"));
        }
        typesetter_storage_path()
    }

    /// One-time merge of the state files older builds left next to the exe (or in the lower-case
    /// Roaming folder): favorites are unioned and styles imported when the profile store has none.
    /// The flag is persisted, so a font the user un-stars later cannot come back from a stale copy.
    fn migrate_legacy(&mut self) {
        if self.legacy_merged {
            return;
        }
        let mut found = false;
        for path in legacy_typesetter_paths() {
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let Ok(data) = serde_json::from_str::<TypesetterPersistedState>(&content) else { continue };
            found = true;
            for font in data.favorite_fonts {
                if !self.favorite_fonts.contains(&font) {
                    self.favorite_fonts.push(font);
                }
            }
            if self.state.presets.is_empty()
                && let Ok(count) = self.state.import_styles_json(&content)
                && count > 0
            {
                self.state.selected_preset_idx = self.state.selected_preset_idx.min(self.state.presets.len().saturating_sub(1));
            }
        }
        // Only a real source marks the migration done: a run whose exe-adjacent/cwd paths hold
        // nothing (a test binary, a fresh install) retries on the next launch instead of
        // silently declaring the old favorites gone.
        self.legacy_merged = found;
        self.auto_save();
    }
    /// Автоматическое сохранение состояния и пресетов в файл настроек (дебаунс 300 мс).
    pub fn auto_save(&mut self) {
        self.save_dirty = true;
        self.save_dirty_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(300));
    }

    /// Проверяет таймер дебаунса автосохранения и сохраняет при истечении.
    pub fn check_auto_save_debounce(&mut self) {
        if self.save_dirty {
            if let Some(deadline) = self.save_dirty_until {
                if std::time::Instant::now() >= deadline {
                    self.save_now();
                    self.save_dirty = false;
                    self.save_dirty_until = None;
                }
            }
        }
    }

    /// Немедленный сброс отложенного сохранения на диск.
    pub fn flush_auto_save(&mut self) {
        if self.save_dirty {
            self.save_now();
            self.save_dirty = false;
            self.save_dirty_until = None;
        }
    }

    /// Прямая запись состояния в файл на диске.
    pub fn save_now(&self) {
        let persisted = TypesetterPersistedState {
            dock_mode: self.dock_mode,
            state: self.state.clone(),
            selected_folder: self.selected_folder.clone(),
            custom_storage_dir: self.custom_storage_dir.clone(),
            favorite_fonts: self.favorite_fonts.clone(),
            is_open: self.is_open,
            script_list_height: self.script_list_height,
            legacy_merged: self.legacy_merged,
        };
        if let Ok(json_str) = serde_json::to_string_pretty(&persisted) {
            if let Some(custom_dir) = &self.custom_storage_dir {
                let _ = std::fs::create_dir_all(custom_dir);
                let _ = std::fs::write(custom_dir.join("typesetter_state.json"), &json_str);
            }
            if let Some(default_path) = typesetter_storage_path() {
                let _ = std::fs::write(default_path, &json_str);
            }
        }
    }

    /// Moves the script editor's caret to the start of the current line, so a line chosen by the
    /// toolbar arrows, or the next one after sending, is where typing continues.
    fn move_caret_to_current_line(&self, ctx: &egui::Context, id: egui::Id) {
        let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) else { return };
        let start: usize = self.state.lines.iter().take(self.state.current_line_idx).map(|l| l.raw.chars().count() + 1).sum();
        state.cursor.set_char_range(Some(egui::text_selection::CCursorRange::one(egui::text::CCursor::new(start))));
        state.store(ctx, id);
    }
    /// Автоматическая загрузка сохраненных пресетов и состояния.
    pub fn auto_load(&mut self) {
        if let Some(path) = typesetter_storage_path() {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(data) = serde_json::from_str::<TypesetterPersistedState>(&content) {
                        self.dock_mode = data.dock_mode;
                        self.state = data.state;
                        self.selected_folder = data.selected_folder;
                        self.favorite_fonts = data.favorite_fonts;
                        self.custom_storage_dir = data.custom_storage_dir;
                        self.is_open = data.is_open;
                        self.script_list_height = data.script_list_height;
                        self.legacy_merged = data.legacy_merged;
                        self.sync_script_buffer_from_state();
                    } else if let Ok(count) = self.state.import_styles_json(&content) {
                        if count > 0 {
                            return;
                        }
                    }
                }
            }
        }

        if let Some(custom_dir) = &self.custom_storage_dir {
            let custom_path = custom_dir.join("typesetter_state.json");
            if custom_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&custom_path) {
                    if let Ok(data) = serde_json::from_str::<TypesetterPersistedState>(&content) {
                        self.dock_mode = data.dock_mode;
                        self.state = data.state;
                        self.selected_folder = data.selected_folder;
                        self.favorite_fonts = data.favorite_fonts;
                        self.is_open = data.is_open;
                        self.legacy_merged = data.legacy_merged;
                        if !self.state.presets.is_empty() {
                            return;
                        }
                    }
                }
            }
        }

        // One-time move of favorites/styles from the old exe-relative stores.
        self.migrate_legacy();

        // Если файла еще нет, проверяем TypeR_Export.json рядом с exe или в текущей папке
        let export_candidates = [
            std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("TypeR_Export.json"))),
            Some(std::path::PathBuf::from("TypeR_Export.json")),
        ];
        for cand in export_candidates.into_iter().flatten() {
            if cand.exists() {
                if let Ok(content) = std::fs::read_to_string(&cand) {
                    if let Ok(count) = self.state.import_styles_json(&content) {
                        if count > 0 {
                            self.auto_save();
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Отрисовка TypeR при встраивании в контейнер дока (левый, правый или нижний).
    pub fn render_embedded(&mut self, ui: &mut egui::Ui) -> Option<TypesetterAction> {
        self.check_auto_save_debounce();
        self.rendered_this_frame = true;
        let mut action = None;
        let avail_w = ui.available_width();
        egui::Frame::NONE
            .fill(crate::snow_typer_style::TyperStyle::get(ui.ctx()).ui_darker)
            .inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 })
            .show(ui, |ui| {
                ui.set_max_width((avail_w - 16.0).max(10.0));
                self.render_header_toolbar(ui);
                self.render_body(ui, &mut action);
                self.render_type_r_footer(ui);
            });
        // Придоканная панель тоже должна открывать окна стиля/сценария; в плавающем режиме их
        // рисует `show`, а счётчик pass не даст открыть их дважды за кадр.
        let ctx = ui.ctx().clone();
        self.render_aux_windows(&ctx, &mut action);
        action
    }

    /// Основная точка отрисовки UI (поддержка свободного окна и плавающего режима).
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<TypesetterAction> {
        self.check_auto_save_debounce();
        if !self.is_open {
            self.flush_auto_save();
            self.rendered_this_frame = false;
            return None;
        }

        let mut action = None;
        let ctx = ui.ctx().clone();
        self.rendered_this_frame = false;

        self.render_aux_windows(&ctx, &mut action);

        action
    }

    /// Окна-спутники TypeR: полный текст, настройки стиля и редактор сценария.
    ///
    /// Рисуются и в плавающем (`show`), и в придоканном (`render_embedded`) режиме: окна egui
    /// живут на контексте, поэтому вызов один на кадр — иначе панель, которую в доке рисуют и
    /// `show`, и док, создала бы каждое окно дважды.
    fn render_aux_windows(&mut self, ctx: &egui::Context, action: &mut Option<TypesetterAction>) {
        let pass = ctx.cumulative_pass_nr();
        if self.aux_pass == pass {
            return;
        }
        self.aux_pass = pass;
        if self.show_full_text_window {
            let mut show_win = self.show_full_text_window;
            Window::new(tl!("Manga Script (Full Text)"))
                .open(&mut show_win)
                .default_width(450.0)
                .default_height(550.0)
                .min_width(320.0)
                .min_height(300.0)
                .resizable(true)
                .show(ctx, |ui| {
                    self.render_full_text_content(ui, &mut *action);
                });
            self.show_full_text_window = show_win;
        }

        if self.show_text_shape_window {
            let mut open = self.show_text_shape_window;
            let mut apply = None;
            Window::new(tl!("TextShapeR"))
                .id(egui::Id::new("typer_text_shape_window"))
                .open(&mut open)
                .default_width(420.0)
                .resizable(true)
                .show(ctx, |ui| {
                    if self.text_shape_candidates.is_empty() {
                        ui.label(tl!("No suitable line breaks were found"));
                        return;
                    }
                    for index in 0..self.text_shape_candidates.len() {
                        let label = self.text_shape_candidates.get(index).map(|candidate| match candidate.kind {
                            TextShapeKind::Original => tl!("Original line breaks"),
                            TextShapeKind::ContourFit => tl!("Contour fit"),
                            TextShapeKind::BubbleWrap => tl!("Bubble wrap"),
                            TextShapeKind::Balanced => tl!("Balanced"),
                        });
                        if let Some(label) = label {
                            ui.radio_value(&mut self.text_shape_selected, index, label);
                        }
                    }
                    ui.separator();
                    if let Some(candidate) = self.text_shape_candidates.get(self.text_shape_selected) {
                        let text = candidate.text.clone();
                        ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            ui.label(RichText::new(text).monospace());
                        });
                        if ui.button(tl!("Apply")).clicked() {
                            apply = Some(self.text_shape_selected);
                        }
                    }
                });
            self.show_text_shape_window = open;
            if let Some(index) = apply {
                self.apply_text_shape_candidate(index);
            }
        }

        // Модальное окно настроек конкретного шрифта / стиля
        if let Some(edit_idx) = self.editing_preset_idx {
            let mut close_modal = false;
            let title = if let Some(preset) = self.state.presets.get(edit_idx) {
                format!("{}: {}", tl!("Font Settings"), preset.name)
            } else {
                tl!("Font Settings").to_string()
            };

            // Окно должно свободно перетаскиваться: `.anchor()` прибивал его к центру каждый
            // кадр, из-за чего заголовок не тянулся. Вместо этого центрируем только стартовую
            // позицию, а дальнейшие — запоминает egui по стабильному id (иначе id зависел от
            // имени стиля и позиция сбрасывалась при смене стиля).
            Window::new(title)
                .id(egui::Id::new("typer_font_settings_window"))
                .collapsible(false)
                .resizable(true)
                .default_width(360.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    self.render_font_settings_modal(ui, edit_idx, &mut close_modal);
                });

            if close_modal {
                self.editing_preset_idx = None;
                self.auto_save();
            }
        }

        if self.show_help_modal {
            let mut show_help = self.show_help_modal;
            Window::new("Справка TypeR")
                .id(egui::Id::new("typer_help_window"))
                .open(&mut show_help)
                .default_width(340.0)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("Горячие клавиши и подсказки");
                    ui.add_space(6.0);
                    ui.label("• Enter / ⤓ — вставить текущую строку в выделение/баллон");
                    ui.label("• ⇢ на стиле — применить стиль к выделенному тексту на странице");
                    ui.label("• Клик по стилю — выбрать стиль для следующих реплик");
                    ui.label("• Двойной клик по стилю — применить стиль и вставить строку");
                    ui.label("• ✏ — открыть детальные настройки шрифта и стиля");
                    ui.label("• ⎘ — дублировать стиль");
                    ui.label("• Вставка по клику — авто-вставка строки при создании выделения");
                    ui.label("• Авто-клин — очистка баллона на слое TypeR Clean перед вставкой");
                    ui.add_space(8.0);
                });
            self.show_help_modal = show_help;
        }

        // Модальное окно ввода и редактирования текста сценария
        if self.show_script_editor_modal {
            let mut close_modal = false;
            Window::new(tl!("Script Editor"))
                .id(egui::Id::new("typer_script_editor_window"))
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .default_height(420.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    self.render_script_editor_modal(ui, &mut close_modal);
                });

            if close_modal {
                self.show_script_editor_modal = false;
            }
        }
    }

    /// Верхняя панель вкладок и меню опций/импорта/экспорта в едином стиле Photoshop.
    fn render_header_toolbar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());

        ui.horizontal(|ui| {
            ui.label(RichText::new("TypeR").color(crate::snow_typer_style::TyperStyle::get(ui.ctx()).muted).size(11.0));
            if self.current_tab != TypesetterTab::Queue {
                if ui.small_button("◀").on_hover_text(tl!("Back to TypeR")).clicked() {
                    self.current_tab = TypesetterTab::Queue;
                }
            }

            // Меню панели и переключатель страниц главы
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("···", |ui| {
                    widgets::style_spectrum_popup_menu(ui);

                    ui.label(RichText::new(tl!("Manga Workflow")).size(11.0).color(t.text_dim).strong());
                    if ui.button(tl!("Chapter Pages…")).clicked() {
                        ui.close();
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("photocraft_toggle_pages_cmd"), true));
                    }
                    if ui.button(tl!("Split Left Dock (Pages + TypeR)…")).clicked() {
                        ui.close();
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("photocraft_toggle_split_dock_cmd"), true));
                    }

                    ui.separator();
                    ui.label(RichText::new(tl!("Script")).size(11.0).color(t.text_dim).strong());

                    if ui.button(tl!("Paste / Edit script…")).clicked() {
                        ui.close();
                        self.open_script_editor();
                    }

                    if ui.button(tl!("Load script file (.txt)…")).clicked() {
                        ui.close();
                        self.open_script_file_dialog();
                    }

                    if !self.state.lines.is_empty() {
                        if ui.button(tl!("Clear script")).clicked() {
                            ui.close();
                            self.state.lines.clear();
                            self.state.current_line_idx = 0;
                            self.auto_save();
                        }
                    }

                    ui.separator();
                    ui.label(RichText::new(tl!("TypeR Styles")).size(11.0).color(t.text_dim).strong());

                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tl!("Import TypeR JSON…")).clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TypeR JSON (*.json)", &["json"])
                            .set_title(tl!("Choose TypeR styles file"))
                            .pick_file()
                        {
                            match std::fs::read_to_string(&path) {
                                Ok(content) => match self.state.import_styles_json(&content) {
                                    Ok(count) => {
                                        self.status_message = Some(format!("{}: {}", tl!("Loaded styles"), count));
                                        self.status_error = false;
                                        self.auto_save();
                                    }
                                    Err(err) => {
                                        self.status_message = Some(format!("{}: {}", tl!("Import error"), err));
                                        self.status_error = true;
                                    }
                                },
                                Err(err) => {
                                    self.status_message = Some(format!("{}: {}", tl!("Read error"), err));
                                    self.status_error = true;
                                }
                            }
                        }
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tl!("Import Photoshop TPL…")).clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Photoshop Tool Presets (*.tpl)", &["tpl"])
                            .set_title(tl!("Choose Photoshop tool presets"))
                            .pick_file()
                        {
                            match read_tpl_file(&path) {
                                Ok(data) => {
                                    let folder = path.file_stem().and_then(|name| name.to_str()).unwrap_or("Photoshop TPL");
                                    match self.state.import_tool_presets_tpl(&data, folder) {
                                        Ok(report) => {
                                            self.status_message = Some(crate::i18n::fmt(
                                                tl!("Loaded {count} text styles ({vertical} vertical); skipped {skipped} presets"),
                                                &[
                                                    ("count", &report.imported.to_string()),
                                                    ("skipped", &report.skipped.to_string()),
                                                    ("vertical", &report.vertical_imported.to_string()),
                                                ],
                                            ));
                                            self.status_error = false;
                                            self.auto_save();
                                        }
                                        Err(error) => {
                                            self.status_message = Some(format!("{}: {}", tl!("Import error"), error));
                                            self.status_error = true;
                                        }
                                    }
                                }
                                Err(error) => {
                                    self.status_message = Some(format!("{}: {}", tl!("Read error"), error));
                                    self.status_error = true;
                                }
                            }
                        }
                    }

                    if ui.button(tl!("TextShapeR…")).clicked() {
                        ui.close();
                        self.text_shape_requested = true;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tl!("Export TypeR JSON…")).clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TypeR JSON (*.json)", &["json"])
                            .set_title(tl!("Export TypeR styles"))
                            .save_file()
                        {
                            match self.state.export_styles_json(self.selected_folder.as_deref()) {
                                Ok(json_content) => match std::fs::write(&path, json_content) {
                                    Ok(_) => {
                                        self.status_message = Some(tl!("Styles successfully exported").into());
                                        self.status_error = false;
                                    }
                                    Err(err) => {
                                        self.status_message = Some(format!("{}: {}", tl!("Write error"), err));
                                        self.status_error = true;
                                    }
                                },
                                Err(err) => {
                                    self.status_message = Some(format!("{}: {}", tl!("Export error"), err));
                                    self.status_error = true;
                                }
                            }
                        }
                    }

                    if ui.button(tl!("Scan text layers in current document")).clicked() {
                        ui.close();
                        self.document_style_scan_requested = true;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tl!("Scan PSD file for text styles…")).clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Photoshop documents (*.psd, *.psb)", &["psd", "psb"])
                            .set_title(tl!("Choose a Photoshop document to scan"))
                            .pick_file()
                        {
                            self.document_style_file_scan_requested = Some(path);
                        }
                    }
                });

                if ui.button(RichText::new("📄").size(11.0))
                    .on_hover_text(tl!("Toggle Chapter Pages panel"))
                    .clicked()
                {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("photocraft_toggle_pages_cmd"), true));
                }
            });
        });

        // Статусное сообщение
        if let Some(msg) = &self.status_message {
            let color = if self.status_error {
                t.danger
            } else {
                t.accent
            };
            ui.add_space(2.0);
            ui.label(RichText::new(msg).color(color).small());
        }

        ui.add_space(2.0);
    }

    /// TypeR's footer rail: workspace, full script and settings, plus queue progress.
    fn render_type_r_footer(&mut self, ui: &mut egui::Ui) {
        let ty = crate::snow_typer_style::TyperStyle::get(ui.ctx());
        let avail_w = ui.available_width();
        egui::Frame::NONE
            .fill(ty.ui_dark)
            .inner_margin(Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.set_max_width((avail_w - 8.0).max(10.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if ui.link(RichText::new("Справка").color(ty.muted).size(11.0)).on_hover_text("Открыть справку и горячие клавиши TypeR").clicked() {
                        self.show_help_modal = true;
                    }
                    if ui.link(RichText::new("Настройки").color(ty.muted).size(11.0)).on_hover_text("Переключить вкладку настроек тайпсеттера").clicked() {
                        self.current_tab = if self.current_tab == TypesetterTab::Settings {
                            TypesetterTab::Queue
                        } else {
                            TypesetterTab::Settings
                        };
                    }
                    if ui.link(RichText::new("Каталог").color(ty.muted).size(11.0)).on_hover_text("Загрузить файл сценария (.txt)").clicked() {
                        self.open_script_file_dialog();
                    }
                    let mb_on = self.multi_bubble_count() > 0;
                    let dot_color = if mb_on { Color32::from_rgb(52, 199, 89) } else { Color32::from_rgb(107, 107, 107) };
                    let mb_text = if mb_on { format!("MB ({})", self.multi_bubble_count()) } else { "MB".into() };
                    ui.label(RichText::new(mb_text).color(ty.muted).size(11.0));
                    let (dot_rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 3.5, dot_color);
                });
            });
    }

    /// Тело активной вкладки тайпсеттера.
    fn render_body(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        match self.current_tab {
            TypesetterTab::Queue | TypesetterTab::Styles => self.render_unified_typer_workspace(ui, action),
            TypesetterTab::FullText => self.render_full_text_content(ui, action),
            TypesetterTab::Settings => self.render_settings_tab(ui),
        }
    }

    /// Единая рабочая область TypeR: адаптивный выбор между горизонтальной
    /// раскладкой (для Нижнего дока) и вертикальной раскладкой (для Левого/Правого доков).
    fn render_unified_typer_workspace(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        if ui.available_height() <= 280.0 && ui.available_width() >= 450.0 {
            self.render_horizontal_typer_workspace(ui, action);
        } else {
            self.render_vertical_typer_workspace(ui, action);
        }
    }

    /// TypeR's two action rows: paste/align/size, then TextShapeR or page navigation.
    fn render_type_r_action_bar(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let ty = crate::snow_typer_style::TyperStyle::get(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let avail = ui.available_width();
            let stepper_w = 68.0f32;
            let gap = 6.0f32;
            let rem = (avail - stepper_w - gap * 2.0).max(10.0);
            let insert_w = (rem * 0.58).floor().max(10.0);
            let align_w = (rem - insert_w).floor().max(10.0);

            let insert = egui::Button::new(RichText::new(format!("⤓ {}", tl!("Insert"))).color(Color32::WHITE).size(12.0).strong())
                .truncate()
                .fill(ty.blue)
                .corner_radius(2.0);
            if ui.add_sized(vec2(insert_w, 26.0), insert).on_hover_text(tl!("Insert current line into selection/bubble (Enter)")).clicked() {
                self.send_active_line(action);
            }

            let align = egui::Button::new(RichText::new(format!("⊹ {}", tl!("Align"))).size(12.0).color(ty.text))
                .truncate()
                .fill(ty.ui)
                .corner_radius(2.0);
            if ui.add_sized(vec2(align_w, 26.0), align).clicked() {
                self.align_requested = true;
            }

            // Stepper: [-] [1 px] [+] (flex-shrink: 0, 68px width)
            let (stepper_rect, _) = ui.allocate_exact_size(vec2(stepper_w, 26.0), Sense::hover());
            let btn_w = 18.0f32;
            let label_w = (stepper_w - btn_w * 2.0).max(10.0);

            let minus_rect = egui::Rect::from_min_size(stepper_rect.min, vec2(btn_w, 26.0));
            let minus_resp = ui.interact(minus_rect, ui.id().with("stepper_minus"), Sense::click())
                .on_hover_text(tl!("Decrease text size"));
            if minus_resp.clicked() {
                self.state.adjust_active_preset_size(-1.0);
                self.auto_save();
            }
            let minus_bg = if minus_resp.hovered() { ty.ui_dark } else { ty.ui };
            ui.painter().rect_filled(minus_rect, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, minus_bg);
            ui.painter().text(minus_rect.center(), egui::Align2::CENTER_CENTER, "−", egui::FontId::proportional(13.0), ty.text);

            let label_rect = egui::Rect::from_min_size(egui::pos2(minus_rect.max.x, stepper_rect.min.y), vec2(label_w, 26.0));
            ui.painter().rect_filled(label_rect, 0.0, ty.ui_dark);
            ui.painter().rect_stroke(label_rect, 0.0, Stroke::new(1.0, ty.line), egui::StrokeKind::Inside);
            ui.painter().text(label_rect.center(), egui::Align2::CENTER_CENTER, "1 px", egui::FontId::proportional(11.0), ty.text);

            let plus_rect = egui::Rect::from_min_size(egui::pos2(label_rect.max.x, stepper_rect.min.y), vec2(btn_w, 26.0));
            let plus_resp = ui.interact(plus_rect, ui.id().with("stepper_plus"), Sense::click())
                .on_hover_text(tl!("Increase text size"));
            if plus_resp.clicked() {
                self.state.adjust_active_preset_size(1.0);
                self.auto_save();
            }
            let plus_bg = if plus_resp.hovered() { ty.ui_dark } else { ty.ui };
            ui.painter().rect_filled(plus_rect, CornerRadius { ne: 2, se: 2, nw: 0, sw: 0 }, plus_bg);
            ui.painter().text(plus_rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(13.0), ty.text);
        });

        if self.text_shape_layer.is_some() && !self.text_shape_candidates.is_empty() {
            ui.horizontal(|ui| {
                ui.menu_button(RichText::new(format!("{}  ⌄", tl!("TextShapeR"))).color(ty.text).size(11.5), |ui| {
                    if ui.button(tl!("TextShapeR")).clicked() {
                        ui.close();
                        self.text_shape_requested = true;
                    }
                });
                let _ = ui.add(egui::Button::new("☆").frame(false));
                let _ = ui.add(egui::Button::new("▦").frame(false));
                let _ = ui.add(egui::Button::new("▤").frame(false));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let pages = self.state.pages();
                    let current = self.state.current_page;
                    let index = pages.iter().position(|page| *page == current).unwrap_or(0);
                    if ui.add(egui::Button::new("▶").frame(false)).on_hover_text(tl!("Next page")).clicked() {
                        if let Some(next) = pages.get(index + 1).copied() {
                            self.set_current_page(next);
                        }
                    }
                    ui.label(RichText::new(format!("{} / {}", index + 1, pages.len().max(1))).color(ty.muted).size(11.0));
                    if ui.add(egui::Button::new("◀").frame(false)).on_hover_text(tl!("Previous page")).clicked() {
                        if let Some(next) = index.checked_sub(1).and_then(|i| pages.get(i)).copied() {
                            self.set_current_page(next);
                        }
                    }
                });
            });
        }
    }

    fn render_active_line_preview_box(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let ty = crate::snow_typer_style::TyperStyle::get(ui.ctx());
        let cur_idx = self.state.current_line_idx;
        let total_lines = self.state.lines.len();
        let cur_text = self.state.active_line().map(|l| l.text.clone()).unwrap_or_default();
        let style_name = self.state.active_preset().map(|p| p.name.clone()).unwrap_or_else(|| tl!("Default").into());

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                let up = egui::Button::new("↑").fill(ty.ui).min_size(vec2(22.0, 18.0));
                if ui.add(up).on_hover_text(tl!("Previous line (Up)")).clicked() {
                    self.state.prev_line();
                    self.scroll_to_active_line = true;
                    self.auto_save();
                }
                let down = egui::Button::new("↓").fill(ty.ui).min_size(vec2(22.0, 18.0));
                if ui.add(down).on_hover_text(tl!("Next line (Down)")).clicked() {
                    self.state.advance_line();
                    self.scroll_to_active_line = true;
                    self.auto_save();
                }
            });

            let outer_w = (ui.available_width() - 2.0).max(10.0);
            let inner_w = (outer_w - 14.0).max(10.0);
            ui.allocate_ui_with_layout(vec2(outer_w, 40.0), Layout::top_down(Align::Min), |ui| {
                Frame::NONE
                    .fill(ty.ui_dark)
                    .stroke(Stroke::new(1.0, ty.line))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.set_max_width(inner_w);
                        ui.horizontal(|ui| {
                            let line_num_str = if total_lines == 0 { 0 } else { cur_idx + 1 };
                            let title_text = format!("строка: {}, стиль: {}, масштаб: 100 %", line_num_str, style_name);
                            let meta_w = (inner_w - 24.0).max(10.0);
                            ui.allocate_ui_with_layout(vec2(meta_w, 16.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.add(egui::Label::new(
                                    RichText::new(title_text)
                                        .color(ty.muted)
                                        .size(10.5),
                                ).truncate());
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let send_btn = egui::Button::new(RichText::new("⇢").color(ty.text).size(12.0)).frame(false);
                                if ui.add(send_btn).on_hover_text("Вставить строку на холст").clicked() {
                                    self.send_active_line(action);
                                }
                            });
                        });
                        ui.add_space(2.0);
                        let display_text = if cur_text.is_empty() {
                            if total_lines == 0 { "(сценарий не загружен)" } else { "(пустая строка)" }
                        } else {
                            &cur_text
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(display_text)
                                    .color(Color32::WHITE)
                                    .strong()
                                    .size(11.5),
                            )
                            .truncate(),
                        );
                    });
            });
        });
    }

    fn send_active_line(&mut self, action: &mut Option<TypesetterAction>) {
        if let Some(line) = self.state.active_line() {
            if let Some(preset) = self.state.preset_for_line(self.state.current_line_idx).cloned() {
                *action = Some(TypesetterAction { text: line.text.clone(), preset });
                self.state.advance_line();
                self.scroll_to_active_line = true;
                self.auto_save();
            }
        }
    }

    fn set_current_page(&mut self, page: usize) {
        self.state.current_page = page;
        if let Some((index, _)) = self.state.lines_for_page(page).first() {
            self.state.set_current_line(*index);
        }
        self.scroll_to_active_line = true;
        self.auto_save();
    }

    fn render_type_r_preview_cards(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        if !self.text_shape_candidates.is_empty() && self.text_shape_layer.is_some() {
            let candidates = self.text_shape_candidates.clone();
            ui.columns(3, |columns| {
                for column in 0..3 {
                    let Some(candidate) = candidates.get(column) else { continue };
                    let selected = column == self.text_shape_selected;
                    Frame::NONE
                        .fill(if selected { t.accent_soft } else { t.field })
                        .stroke(Stroke::new(1.0, if selected { t.accent } else { t.field_border }))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(6, 5))
                        .show(&mut columns[column], |ui| {
                            ui.set_min_height(78.0);
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                ui.label(RichText::new(format!("{}", column + 1)).color(t.text_dim).small());
                            });
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 62.0),
                                Sense::click(),
                            );
                            let short = preview_short_text(&candidate.text);
                            let font_family = self.text_shape_preview_preset.as_ref().map(|p| p.font_family.as_str()).unwrap_or("");
                            let line_count = short.lines().count().max(1) as f32;
                            let base_font_size = self.text_shape_preview_preset.as_ref().map_or(13.0, |p| p.font_size.min(18.0));
                            let font_size = base_font_size.min((54.0 / (line_count * 1.15)).max(5.5));
                            crate::font_preview::paint_centered(
                                ui,
                                font_family,
                                &short,
                                font_size,
                                rect,
                                t.text,
                            );
                            if response.clicked() {
                                self.text_shape_selected = column;
                                self.apply_text_shape_candidate(column);
                            }
                        });
                }
            });
            ui.add_space(2.0);
            return;
        }
        // The three fields belong to TextShapeR, not to the script queue. With no selected
        // text layer they stay empty instead of showing typer lines.
        ui.columns(3, |columns| {
            for column in 0..3 {
                Frame::NONE
                    .fill(t.field)
                    .stroke(Stroke::new(1.0, t.field_border))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Margin::symmetric(6, 5))
                    .show(&mut columns[column], |ui| {
                        ui.set_min_height(78.0);
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.label(RichText::new(format!("{}", column + 1)).color(t.text_dim).small());
                        });
                        ui.label(RichText::new(tl!("Select a text layer on the page")).color(t.text_faint).small());
                    });
            }
        });
        ui.add_space(2.0);
    }

    /// Вертикальная раскладка TypeR (для боковых доков и плавающего окна).
    fn render_vertical_typer_workspace(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let t = Tokens::get(ui.ctx());
        let total_lines = self.state.lines.len();
        let cur_idx = self.state.current_line_idx;

        self.render_type_r_action_bar(ui, action);
        if self.text_shape_layer.is_some() && !self.text_shape_candidates.is_empty() {
            self.render_type_r_preview_cards(ui);
        } else {
            self.render_active_line_preview_box(ui, action);
        }

        widgets::hairline(ui);
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            if ui.small_button("📋").on_hover_text(tl!("Paste script text from clipboard (Ctrl+V)")).clicked() {
                self.paste_script_from_clipboard();
            }
            if ui.small_button("📝").on_hover_text(tl!("Open script text editor")).clicked() {
                self.open_script_editor();
            }
            if ui.small_button("📥").on_hover_text(tl!("Load script file (.txt)")).clicked() {
                self.open_script_file_dialog();
            }
            if ui.small_button("⚙").on_hover_text(tl!("Options")).clicked() {
                self.current_tab = TypesetterTab::Settings;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if total_lines > 0 {
                    ui.label(
                        RichText::new(format!("{}/{}", if total_lines == 0 { 0 } else { cur_idx + 1 }, total_lines))
                            .color(t.text_dim)
                            .small(),
                    );
                }
            });
        });
        ui.add_space(2.0);

        let script_list_height = self.script_list_height;
        self.render_script_list(ui, action, script_list_height);

        // Splitter / ползунок изменения высоты окна сценария
        let (res_rect, res_resp) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::drag());
        let is_res_hover = res_resp.hovered() || res_resp.dragged();
        if is_res_hover {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if res_resp.dragged() {
            self.script_list_height = (self.script_list_height + res_resp.drag_delta().y).clamp(80.0, 600.0);
            self.auto_save();
        }
        let splitter_line_col = if is_res_hover {
            t.accent
        } else {
            Color32::from_rgb(205, 210, 220)
        };
        ui.painter().hline(res_rect.x_range(), res_rect.center().y, Stroke::new(1.0, splitter_line_col));
        let cx = res_rect.center().x;
        let cy = res_rect.center().y;
        for offset in [-12.0, -6.0, 0.0, 6.0, 12.0] {
            ui.painter().circle_filled(egui::pos2(cx + offset, cy), 1.2, splitter_line_col);
        }
        ui.add_space(2.0);

        self.render_preset_toolbar(ui);

        let num_cols = if ui.available_width() < 240.0 { 1 } else { 2 };
        self.render_style_presets_grid(ui, action, num_cols);

        // TypeR's two wide style buttons (at the bottom of style tree, matching TypeR Studio).
        ui.add_space(3.0);
        let ty = crate::snow_typer_style::TyperStyle::get(ui.ctx());
        let gap = 6.0f32;
        let avail_w = ui.available_width();
        let btn_w = ((avail_w - gap) * 0.5).floor().max(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let folder = egui::Button::new(RichText::new("＋ Добавить папку").size(11.0).color(ty.text))
                .truncate()
                .fill(ty.ui)
                .corner_radius(2.0);
            if ui.add_sized(vec2(btn_w, 24.0), folder).clicked() {
                self.creating_new_folder = true;
                self.new_folder_name_buffer = tl!("New Folder").into();
            }
            let style = egui::Button::new(RichText::new("＋ Добавить стиль").size(11.0).color(ty.text))
                .truncate()
                .fill(ty.ui)
                .corner_radius(2.0);
            if ui.add_sized(vec2(btn_w, 24.0), style).clicked() {
                self.state.presets.push(TextStylePreset { name: tl!("New Preset").to_string(), ..TextStylePreset::default() });
                self.editing_preset_idx = Some(self.state.presets.len().saturating_sub(1));
                self.auto_save();
            }
        });
    }

    /// Горизонтальная раскладка TypeR (специально для Нижнего дока).
    fn render_horizontal_typer_workspace(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let t = Tokens::get(ui.ctx());
        let total_lines = self.state.lines.len();
        let cur_idx = self.state.current_line_idx;

        ui.horizontal(|ui| {
            // --- 1. Левая колонка: Сценарий (список реплик) ---
            let left_w = 250.0f32.min(ui.available_width() * 0.32);
            ui.allocate_ui_with_layout(vec2(left_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}:", tl!("Script"))).color(t.text_dim).small().strong());
                    if ui.small_button("📋").on_hover_text(tl!("Paste script from clipboard (Ctrl+V)")).clicked() {
                        self.paste_script_from_clipboard();
                    }
                    if ui.small_button("📝").on_hover_text(tl!("Open script text editor")).clicked() {
                        self.open_script_editor();
                    }
                    if ui.small_button("📥").on_hover_text(tl!("Load script file (.txt)")).clicked() {
                        self.open_script_file_dialog();
                    }
                    if total_lines > 0 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{}/{}", cur_idx + 1, total_lines)).color(t.text_dim).small());
                        });
                    }
                });

                let list_h = (ui.available_height() - 4.0).max(60.0);
                self.render_script_list(ui, action, list_h);
            });

            ui.separator();

            // --- 2. Средняя колонка: Текущая реплика и быстрая вставка ---
            let mid_w = 230.0f32.min(ui.available_width() * 0.35);
            ui.allocate_ui_with_layout(vec2(mid_w, ui.available_height()), Layout::top_down(Align::Min), |ui| {
                ui.horizontal(|ui| {
                    let up_btn = egui::Button::new("▲").min_size(vec2(22.0, 16.0));
                    if ui.add(up_btn).on_hover_text(tl!("Previous line (↑)")).clicked() {
                        self.state.prev_line();
                        self.scroll_to_active_line = true;
                    }
                    let down_btn = egui::Button::new("▼").min_size(vec2(22.0, 16.0));
                    if ui.add(down_btn).on_hover_text(tl!("Next line (↓)")).clicked() {
                        self.state.advance_line();
                        self.scroll_to_active_line = true;
                    }

                    let line_num_str = if total_lines == 0 { 0 } else { cur_idx + 1 };
                    let style_name = self.state.active_preset().map(|p| p.name.clone()).unwrap_or_else(|| tl!("Default").into());
                    let font_size = self.state.active_preset().map(|p| p.font_size.round() as i32).unwrap_or(12);

                    ui.label(
                        RichText::new(format!("{}: {} ({}pt)", line_num_str, style_name, font_size))
                            .color(t.text_dim)
                            .small(),
                    );
                });

                ui.horizontal(|ui| {
                    if ui.small_button(tl!("Capture bubble")).on_hover_text(tl!("Capture the current selection for multi-bubble fill")).clicked() {
                        self.multi_bubble_capture_requested = true;
                    }
                    let count = self.multi_bubble_count();
                    let fill = egui::Button::new(format!("{} ({count})", tl!("Fill bubbles")));
                    if ui.add_enabled(count > 0, fill).clicked() {
                        self.multi_bubble_fill_requested = true;
                    }
                    if count > 0 && ui.small_button("✖").on_hover_text(tl!("Clear captured bubbles")).clicked() {
                        self.multi_bubble_captures.clear();
                    }
                });

                let current_text = self.state.active_line().map(|l| l.text.as_str()).unwrap_or("");
                let box_h = (ui.available_height() - 32.0).max(40.0);
                Frame::NONE
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(170, 175, 185)))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        let display = if current_text.is_empty() {
                            if total_lines == 0 { "(сценарий не загружен)" } else { tl!("(end of script)") }
                        } else {
                            current_text
                        };
                        let text_color = if current_text.is_empty() {
                            Color32::from_rgb(140, 145, 155)
                        } else {
                            Color32::from_rgb(20, 25, 35)
                        };
                        ScrollArea::vertical()
                            .id_salt("horizontal_current_text_scroll")
                            .max_height(box_h)
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(display).color(text_color).size(12.0).strong()).wrap_mode(egui::TextWrapMode::Wrap));
                            });
                    });

                let insert_btn = egui::Button::new(RichText::new("▶ Вставить (Enter)").color(t.accent_text).strong())
                    .fill(t.accent)
                    .min_size(vec2(ui.available_width(), 24.0));
                if ui.add(insert_btn).clicked() {
                    if let Some(line) = self.state.active_line() {
                        if let Some(preset) = self.state.preset_for_line(self.state.current_line_idx).cloned() {
                            *action = Some(TypesetterAction {
                                text: line.text.clone(),
                                preset,
                            });
                            self.state.advance_line();
                            self.scroll_to_active_line = true;
                            self.auto_save();
                        }
                    }
                }
            });

            ui.separator();

            // --- 3. Правая колонка: Стили и пресеты ---
            ui.allocate_ui_with_layout(vec2(ui.available_width(), ui.available_height()), Layout::top_down(Align::Min), |ui| {
                self.render_preset_toolbar(ui);
                let num_cols = ((ui.available_width() / 150.0).floor() as usize).max(1);
                self.render_style_presets_grid(ui, action, num_cols);
            });
        });
    }

    /// Тулбар управления стилями (Поиск).
    fn render_preset_toolbar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        let avail_w = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(avail_w, 22.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.add_space(2.0);
                let filter_w = 70.0f32.min((avail_w - 6.0).max(30.0));
                ui.add_sized(
                    vec2(filter_w, 20.0),
                    egui::TextEdit::singleline(&mut self.style_filter)
                        .hint_text("🔍")
                        .desired_width(filter_w),
                );
            },
        );

        if self.creating_new_folder {
            Frame::NONE
                .fill(t.field)
                .stroke(Stroke::new(1.0, t.field_border))
                .corner_radius(CornerRadius::same(t.radius_sm as u8))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{}:", tl!("Folder name"))).color(t.text).small());
                        ui.text_edit_singleline(&mut self.new_folder_name_buffer);
                        if ui.button("OK").clicked() {
                            let name = self.new_folder_name_buffer.trim().to_string();
                            if !name.is_empty() {
                                self.selected_folder = Some(name);
                            }
                            self.creating_new_folder = false;
                        }
                        if ui.button(tl!("Cancel")).clicked() {
                            self.creating_new_folder = false;
                        }
                    });
                });
        }

        if let Some(src_folder) = self.duplicating_folder.clone() {
            Frame::NONE
                .fill(t.field)
                .stroke(Stroke::new(1.0, t.field_border))
                .corner_radius(CornerRadius::same(t.radius_sm as u8))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("{}: '{}'", tl!("Duplicate folder"), src_folder)).color(t.text).strong().small());
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.duplicate_folder_name_buffer);
                        if ui.button("OK").clicked() {
                            let count = self.state.duplicate_folder(&src_folder, &self.duplicate_folder_name_buffer);
                            self.status_message = Some(format!("{}: {}", tl!("Folder duplicated"), count));
                            self.duplicating_folder = None;
                            self.auto_save();
                        }
                        if ui.button(tl!("Cancel")).clicked() {
                            self.duplicating_folder = None;
                        }
                    });
                });
        }
    }

    /// Полноценный единый текстовый редактор сценария TypeR с гуттерами строк,
    /// сквозным выделением текста мышью, свободным слиянием (Backspace) и переносом (Enter).
    fn render_script_list(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>, max_height: f32) {
        if self.script_buffer.is_empty() && !self.state.lines.is_empty() {
            self.sync_script_buffer_from_state();
        }

        let t = Tokens::get(ui.ctx());
        let gutter_w = 46.0f32;
        let action_w = 22.0f32;
        let right_pad = 6.0f32;
        let edit_id = egui::Id::new("typer_unified_script_text_edit");
        let active_line_idx = self.state.current_line_idx;
        let mut do_scroll_active = self.scroll_to_active_line;

        // Если буфер пуст и реплик нет — показываем плашку быстрого старта с кнопками
        if self.script_buffer.trim().is_empty() && self.state.lines.is_empty() {
            Frame::NONE
                .fill(t.field)
                .stroke(Stroke::new(1.0, t.field_border))
                .corner_radius(CornerRadius::same(2))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tl!("No script loaded")).color(t.text_dim).small());
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(RichText::new(format!("📋 {}", tl!("Paste"))).strong()).on_hover_text(tl!("Paste script from clipboard (Ctrl+V)")).clicked() {
                                self.paste_script_from_clipboard();
                            }
                            if ui.button(format!("📥 {}", tl!("Load .txt"))).on_hover_text(tl!("Load script file (.txt)")).clicked() {
                                self.open_script_file_dialog();
                            }
                            if ui.button(format!("📝 {}", tl!("Editor"))).on_hover_text(tl!("Enter script manually")).clicked() {
                                self.open_script_editor();
                            }
                        });
                        ui.add_space(8.0);
                    });
                });
            return;
        }


        // Phase 2: the line-break preview above the editor; hovering shows syllable markers, a
        // click splits the line, a click on the join mark merges it with the next one.
        let break_idx = self.state.current_line_idx;
        if let Some(raw) = self.state.lines.get(break_idx).map(|l| l.raw.clone()) {
            let next_raw = self.state.lines.get(break_idx + 1).map(|l| l.raw.clone());
            match crate::snow_breaks_ui::show(ui, &raw, next_raw.as_deref()) {
                Some(crate::snow_breaks_ui::BreakAction::Split { char_index, hyphenate }) => self.split_current_line_at(char_index, hyphenate),
                Some(crate::snow_breaks_ui::BreakAction::Merge) => self.merge_current_line(),
                None => {}
            }
        }

        let programmatic_line = self.caret_line != Some(self.state.current_line_idx);

        ScrollArea::vertical()
            .max_height(max_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let styles = ui.style_mut();
                styles.text_styles.insert(egui::TextStyle::Body, egui::FontId::monospace(12.0));
                ui.visuals_mut().selection.bg_fill = crate::snow_typer_style::TyperStyle::get(ui.ctx()).blue;
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::WHITE;
                let avail_w = ui.available_width().max(10.0);
                ui.set_max_width(avail_w);
                ui.spacing_mut().extra_text_line_spacing = SCRIPT_LINE_SPACING;
                let row_h = (ui.text_style_height(&egui::TextStyle::Body) + SCRIPT_LINE_SPACING).max(1.0);
                let desired_rows = (((max_height - 8.0) / row_h).floor() as usize).max(1);
                let edit = egui::TextEdit::multiline(&mut self.script_buffer)
                    .id(edit_id)
                    .font(egui::TextStyle::Body)
                    .text_color(Color32::from_rgb(18, 18, 18))
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: gutter_w as i8,
                        right: (action_w + right_pad) as i8,
                        top: 4,
                        bottom: 4,
                    }))
                    .desired_rows(desired_rows)
                    .desired_width(avail_w);

                let background_idx = ui.painter().add(egui::Shape::Noop);
                let line_highlights: Vec<egui::layers::ShapeIdx> =
                    (0..self.state.lines.len()).map(|_| ui.painter().add(egui::Shape::Noop)).collect();
                let output = edit.show(ui);
                let rect = output.response.rect;

                // 1. Отрисовка подложки и вертикальных границ колонок: чистый белый фон, светлый гуттер
                ui.painter().set(background_idx, egui::Shape::rect_filled(rect, CornerRadius::same(2), Color32::WHITE));

                // Left gutter (светло-серая колонка нумерации)
                let left_gutter_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
                ui.painter().rect_filled(left_gutter_rect, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, Color32::from_rgb(228, 230, 235));
                ui.painter().vline(rect.left() + gutter_w, rect.top()..=rect.bottom(), Stroke::new(1.0, Color32::from_rgb(205, 208, 215)));

                // Right action gutter (колонка кнопки отправки на холст)
                let right_gutter_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - action_w, rect.top()), rect.max);
                ui.painter().rect_filled(right_gutter_rect, CornerRadius { ne: 2, se: 2, nw: 0, sw: 0 }, Color32::from_rgb(245, 245, 247));
                ui.painter().vline(rect.right() - action_w, rect.top()..=rect.bottom(), Stroke::new(1.0, Color32::from_rgb(220, 222, 228)));

                ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, Color32::from_rgb(195, 198, 205)), egui::StrokeKind::Inside);

                // 2. Расчет геометрии логических строк по строкам галлеи (PlacedRow)
                let mut logical_line_idx = 0;
                let mut line_start_y = 0.0f32;
                let mut line_first_row_y = 0.0f32;
                let mut line_first_row_h = 16.0f32;
                let mut line_has_start = false;

                let rows_count = output.galley.rows.len();
                let mut logical_lines_geom = Vec::new();

                for (r_idx, placed_row) in output.galley.rows.iter().enumerate() {
                    let row_y = placed_row.pos.y;
                    let row_h = placed_row.row.size.y;
                    if !line_has_start {
                        line_start_y = row_y;
                        line_first_row_y = row_y;
                        line_first_row_h = row_h;
                        line_has_start = true;
                    }

                    if placed_row.ends_with_newline || r_idx + 1 == rows_count {
                        let line_end_y = row_y + row_h;
                        logical_lines_geom.push((
                            logical_line_idx,
                            line_start_y,
                            line_end_y,
                            line_first_row_y,
                            line_first_row_h,
                        ));
                        logical_line_idx += 1;
                        line_has_start = false;
                    }
                }

                let mouse_pos = ui.input(|i| i.pointer.hover_pos());
                let primary_clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
                let mut clicked_target_line = None;
                let mut clicked_action_line = None;
                // A line was just sent to the layer: its next line becomes current, and the caret
                // must follow it instead of snapping the selection back to where the click was.
                let mut line_sent = false;

                for (line_idx, start_y, end_y, first_row_y, first_row_h) in logical_lines_geom {
                    let screen_start_y = output.galley_pos.y + start_y;
                    let screen_end_y = output.galley_pos.y + end_y;
                    let screen_center_y = output.galley_pos.y + first_row_y + first_row_h * 0.5;

                    let is_active = line_idx == active_line_idx;
                    let is_applied = self.state.lines.get(line_idx).map(|l| l.is_applied).unwrap_or(false);
                    if is_active
                        && let Some(shape) = line_highlights.get(line_idx)
                    {
                        ui.painter().set(
                            *shape,
                            egui::Shape::rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(rect.left(), screen_start_y),
                                    egui::pos2(rect.right() - action_w, screen_end_y),
                                ),
                                CornerRadius::ZERO,
                                Color32::from_rgba_unmultiplied(40, 142, 223, 40),
                            ),
                        );
                    }

                    // Скролл к активной строке при программной смене (стрелки ▲/▼, Enter)
                    if is_active && do_scroll_active {
                        let active_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), screen_start_y),
                            egui::pos2(rect.right(), screen_end_y),
                        );
                        ui.scroll_to_rect(active_rect, Some(Align::Center));
                        do_scroll_active = false;
                    }

                    // А. Левый гуттер: номер строки
                    let num_color = if is_active {
                        Color32::WHITE
                    } else if is_applied {
                        t.text_faint
                    } else {
                        t.text_dim
                    };
                    ui.painter().text(
                        egui::pos2(rect.left() + 22.0, screen_center_y),
                        egui::Align2::RIGHT_CENTER,
                        format!("{}", line_idx + 1),
                        egui::FontId::monospace(10.5),
                        num_color,
                    );

                    // Мишень / иконка статуса
                    let target_center = egui::pos2(rect.left() + 34.0, screen_center_y);
                    let target_rect = egui::Rect::from_center_size(target_center, vec2(16.0, 16.0));
                    let is_target_hovered = mouse_pos.is_some_and(|p| target_rect.contains(p));

                    if is_active {
                        ui.painter().circle_stroke(target_center, 4.5, Stroke::new(1.3, Color32::WHITE));
                        ui.painter().circle_filled(target_center, 2.3, Color32::WHITE);
                    } else {
                        let stroke_color = if is_target_hovered {
                            t.accent
                        } else if is_applied {
                            t.text_faint
                        } else {
                            t.text_dim
                        };
                        ui.painter().circle_stroke(target_center, 4.5, Stroke::new(1.2, stroke_color));
                        ui.painter().circle_stroke(target_center, 1.5, Stroke::new(1.0, stroke_color));
                    }

                    if is_target_hovered && primary_clicked {
                        clicked_target_line = Some(line_idx);
                    }

                    // Б. Правый гуттер: кнопка быстрой отправки строки на холст (->)
                    let action_center = egui::pos2(rect.right() - 12.0, screen_center_y);
                    let action_rect = egui::Rect::from_center_size(action_center, vec2(18.0, 18.0));
                    let is_action_hovered = mouse_pos.is_some_and(|p| action_rect.contains(p));

                    let btn_bg = if is_action_hovered { t.accent } else { t.hover };
                    let icon_color = if is_action_hovered { t.accent_text } else { t.text_dim };
                    let border_color = if is_action_hovered { t.accent } else { t.separator };

                    ui.painter().circle_filled(action_center, 7.5, btn_bg);
                    ui.painter().circle_stroke(action_center, 7.5, Stroke::new(1.0, border_color));

                    let arrow_stroke = Stroke::new(1.4, icon_color);
                    let ax = action_center.x;
                    let ay = action_center.y;
                    ui.painter().line_segment([egui::pos2(ax - 3.5, ay), egui::pos2(ax + 2.5, ay)], arrow_stroke);
                    ui.painter().line_segment([egui::pos2(ax + 0.5, ay - 2.5), egui::pos2(ax + 3.0, ay)], arrow_stroke);
                    ui.painter().line_segment([egui::pos2(ax + 0.5, ay + 2.5), egui::pos2(ax + 3.0, ay)], arrow_stroke);

                    if is_action_hovered && primary_clicked {
                        clicked_action_line = Some(line_idx);
                    }

                    // В. Горизонтальная пунктирная линия под строкой (1px dashed #b5b5b5)
                    let dash_stroke = Stroke::new(1.0, Color32::from_rgb(181, 181, 181));
                    draw_dashed_hline(ui.painter(), rect.left(), rect.right(), screen_end_y, dash_stroke);
                }

                if let Some(target_idx) = clicked_target_line {
                    self.state.set_current_line(target_idx);
                    self.scroll_to_active_line = true;
                }

                if let Some(act_idx) = clicked_action_line {
                    if let Some(text) = self.state.lines.get(act_idx).map(|line| line.text.clone()) {
                        if let Some(preset) = self.state.preset_for_line(act_idx).cloned() {
                            *action = Some(TypesetterAction {
                                text,
                                preset,
                            });
                            self.state.set_current_line(act_idx);
                            self.state.advance_line();
                            line_sent = true;
                            // Keep the caret on the line that just became current.
                            self.move_caret_to_current_line(ui.ctx(), edit_id);
                            self.caret_line = Some(self.state.current_line_idx);
                        }
                    }
                }

                // 3. Отслеживание курсора: при клике или переходе по стрелкам обновляем активную строку
                if programmatic_line && !line_sent {
                    self.move_caret_to_current_line(ui.ctx(), edit_id);
                    self.caret_line = Some(self.state.current_line_idx);
                }

                if !line_sent && !programmatic_line && let Some(cr) = output.cursor_range {
                    let c_idx = cr.primary.index;
                    let char_count = self.script_buffer.chars().take(c_idx.into()).filter(|&c| c == '\n').count();
                    if char_count != self.state.current_line_idx && char_count < self.state.lines.len() {
                        self.state.set_current_line(char_count);
                        self.caret_line = Some(char_count);
                    }
                }

                // 4. Синхронизация состояния при правках в текстовом поле
                if output.response.response.changed() {
                    self.sync_state_from_script_buffer();
                }
            });

        self.scroll_to_active_line = false;
    }

fn paint_pencil_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let p_tip = egui::pos2(rect.min.x + 2.5, rect.max.y - 2.5);
    let p_top_left = egui::pos2(rect.max.x - 4.5, rect.min.y + 2.5);
    let p_top_right = egui::pos2(rect.max.x - 2.5, rect.min.y + 4.5);
    let p_bot_right = egui::pos2(rect.min.x + 4.5, rect.max.y - 2.5);
    let p_bot_left = egui::pos2(rect.min.x + 2.5, rect.max.y - 4.5);
    painter.line_segment([p_bot_left, p_top_left], stroke);
    painter.line_segment([p_bot_right, p_top_right], stroke);
    painter.line_segment([p_top_left, p_top_right], stroke);
    painter.line_segment([p_bot_left, p_tip], stroke);
    painter.line_segment([p_bot_right, p_tip], stroke);
}

fn paint_duplicate_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32, fill_bg: Color32) {
    let stroke = Stroke::new(1.0, color);
    let r1 = egui::Rect::from_min_size(egui::pos2(rect.min.x + 3.0, rect.min.y + 1.0), egui::vec2(8.0, 8.0));
    painter.rect_stroke(r1, 1.0, stroke, egui::StrokeKind::Inside);
    let r2 = egui::Rect::from_min_size(egui::pos2(rect.min.x + 1.0, rect.min.y + 4.0), egui::vec2(8.0, 8.0));
    painter.rect_filled(r2, 1.0, fill_bg);
    painter.rect_stroke(r2, 1.0, stroke, egui::StrokeKind::Inside);
}

fn paint_apply_layer_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let c = rect.center();
    painter.circle_stroke(c, 5.0, stroke);
    let p_tip = egui::pos2(c.x + 2.5, c.y);
    let p_start = egui::pos2(c.x - 2.5, c.y);
    painter.line_segment([p_start, p_tip], stroke);
    painter.line_segment([egui::pos2(c.x + 0.5, c.y - 2.0), p_tip], stroke);
    painter.line_segment([egui::pos2(c.x + 0.5, c.y + 2.0), p_tip], stroke);
}

    /// Сетка карточек стилей шрифтов по папкам (с адаптивным числом колонок).
    fn render_style_presets_grid(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>, num_cols: usize) {
        let t = Tokens::get(ui.ctx());
        let ty = crate::snow_typer_style::TyperStyle::get(ui.ctx());
        let active_preset_idx = self.state.selected_preset_idx;

        let mut style_to_select = None;
        let mut style_to_edit = None;
        let mut style_to_duplicate = None;
        let mut style_to_apply = None;
        let mut style_to_delete = None;
        let mut style_to_layer = None;

        let mut folder_names = self.state.folders();
        folder_names.push(tl!("No folder").to_string());

        let filter_lower = self.style_filter.trim().to_lowercase();

        let grid_h = (ui.available_height() - 28.0).max(48.0);
        ScrollArea::vertical()
            .id_salt("typer_folders_scroll")
            .max_height(grid_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().indent = 0.0;
                ui.spacing_mut().item_spacing.x = 4.0;

                for folder_name in &folder_names {
                    let is_unassigned = folder_name == tl!("No folder");

                    let matching_indices: Vec<usize> = self.state.presets
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            let in_folder = if is_unassigned {
                                p.folder.is_none() || p.folder.as_deref() == Some("")
                            } else {
                                p.folder.as_deref() == Some(folder_name.as_str())
                            };
                            let matches_filter = filter_lower.is_empty() || p.name.to_lowercase().contains(&filter_lower);
                            in_folder && matches_filter
                        })
                        .map(|(idx, _)| idx)
                        .collect();

                    if matching_indices.is_empty() && (is_unassigned || !filter_lower.is_empty()) {
                        continue;
                    }

                    let header_title = format!("{} ({})", folder_name, matching_indices.len());
                    let folder_id = ui.id().with(("folder_open", folder_name));
                    let mut is_open = ui.data_mut(|d| d.get_temp::<bool>(folder_id).unwrap_or(true));

                    // Category header: padding-left: 4px, gap: 6px
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.spacing_mut().item_spacing.x = 6.0;

                        let tri_icon = if is_open { "▼" } else { "▶" };
                        let tri_resp = ui.add(
                            egui::Label::new(RichText::new(tri_icon).size(10.0).color(ty.muted))
                                .sense(Sense::click()),
                        );
                        let title_resp = ui.add(
                            egui::Label::new(RichText::new(&header_title).strong().size(12.0).color(ty.text))
                                .sense(Sense::click()),
                        );

                        if tri_resp.clicked() || title_resp.clicked() {
                            is_open = !is_open;
                            ui.data_mut(|d| d.insert_temp(folder_id, is_open));
                        }
                    });

                    if is_open {
                        ui.add_space(2.0);
                        let gap = 6.0f32;
                        let avail_w = ui.available_width();
                        let cols_count = if avail_w < 240.0 { 1 } else { num_cols.max(1) };
                        let col_w = if cols_count == 1 {
                            avail_w
                        } else {
                            ((avail_w - gap) * 0.5).floor().max(10.0)
                        };

                        let mut draw_card = |col: &mut egui::Ui, p_idx: usize, card_w: f32| {
                            let preset = &self.state.presets[p_idx];
                            let is_active = p_idx == active_preset_idx;

                            let (card_bg, card_border, text_color) = if is_active {
                                (ty.blue, ty.blue_dark, Color32::WHITE)
                            } else {
                                (ty.ui, ty.line, ty.text)
                            };

                            let card_h = 24.0f32;
                            let (card_rect, _) = col.allocate_exact_size(vec2(card_w, card_h), Sense::hover());

                            col.painter().rect_filled(card_rect, CornerRadius::same(t.radius_sm as u8), card_bg);
                            col.painter().rect_stroke(card_rect, CornerRadius::same(t.radius_sm as u8), Stroke::new(1.0, card_border), egui::StrokeKind::Inside);

                            // 1. Right action buttons (strictly anchored inside the right edge of the card)
                            let icon_default_color = if is_active { Color32::WHITE } else { t.text_dim };
                            let btn_sz = 14.0f32;
                            let right_pad = 4.0f32;
                            let btn_gap = 2.0f32;

                            // Apply to layer (rightmost)
                            let apply_rect = egui::Rect::from_center_size(
                                egui::pos2(card_rect.right() - right_pad - btn_sz * 0.5, card_rect.center().y),
                                vec2(btn_sz, btn_sz),
                            );
                            let apply_resp = col.interact(apply_rect, col.id().with(("typer_card_apply", p_idx)), Sense::click())
                                .on_hover_text("Применить стиль к выделенному слою на странице");
                            if apply_resp.clicked() || apply_resp.double_clicked() {
                                style_to_layer = Some(p_idx);
                                style_to_apply = None;
                            }
                            let apply_col = if apply_resp.hovered() { Color32::WHITE } else { icon_default_color };
                            Self::paint_apply_layer_icon(col.painter(), apply_rect, apply_col);

                            // Duplicate (middle)
                            let dup_rect = egui::Rect::from_center_size(
                                egui::pos2(card_rect.right() - right_pad - btn_sz * 1.5 - btn_gap, card_rect.center().y),
                                vec2(btn_sz, btn_sz),
                            );
                            let dup_resp = col.interact(dup_rect, col.id().with(("typer_card_dup", p_idx)), Sense::click())
                                .on_hover_text("Дублировать стиль");
                            if dup_resp.clicked() {
                                style_to_duplicate = Some(p_idx);
                            }
                            let dup_col = if dup_resp.hovered() { Color32::WHITE } else { icon_default_color };
                            Self::paint_duplicate_icon(col.painter(), dup_rect, dup_col, card_bg);

                            // Edit (left of the 3 buttons)
                            let edit_rect = egui::Rect::from_center_size(
                                egui::pos2(card_rect.right() - right_pad - btn_sz * 2.5 - btn_gap * 2.0, card_rect.center().y),
                                vec2(btn_sz, btn_sz),
                            );
                            let edit_resp = col.interact(edit_rect, col.id().with(("typer_card_edit", p_idx)), Sense::click())
                                .on_hover_text("Настройки шрифта и стиля");
                            let mut open_edit = false;
                            if edit_resp.clicked() {
                                open_edit = true;
                            }
                            let edit_col = if edit_resp.hovered() { Color32::WHITE } else { icon_default_color };
                            Self::paint_pencil_icon(col.painter(), edit_rect, edit_col);

                            // 2. Left indicator & swatch
                            let sel_rect = egui::Rect::from_center_size(
                                egui::pos2(card_rect.left() + 9.0, card_rect.center().y),
                                vec2(10.0, 10.0),
                            );
                            let sel_resp = col.interact(sel_rect, col.id().with(("typer_card_sel", p_idx)), Sense::click());
                            if sel_resp.clicked() {
                                style_to_select = Some(p_idx);
                            }
                            let sel_icon = if is_active { "●" } else { "○" };
                            let ind_color = if is_active { Color32::WHITE } else { t.text_dim };
                            col.painter().text(sel_rect.center(), egui::Align2::CENTER_CENTER, sel_icon, egui::FontId::proportional(11.0), ind_color);

                            let swatch_rect = egui::Rect::from_center_size(
                                egui::pos2(card_rect.left() + 20.0, card_rect.center().y),
                                vec2(8.0, 8.0),
                            );
                            let swatch_color = egui::Color32::from_rgba_unmultiplied(
                                preset.text_color[0],
                                preset.text_color[1],
                                preset.text_color[2],
                                preset.text_color[3],
                            );
                            col.painter().rect_filled(swatch_rect, 2.0, swatch_color);
                            if preset.stroke_width > 0.0 {
                                let stroke_c = egui::Color32::from_rgba_unmultiplied(
                                    preset.stroke_color[0],
                                    preset.stroke_color[1],
                                    preset.stroke_color[2],
                                    preset.stroke_color[3],
                                );
                                col.painter().rect_stroke(swatch_rect, 2.0, egui::Stroke::new(1.0, stroke_c), egui::StrokeKind::Outside);
                            }

                            // 3. Middle text: starts after swatch (x = left + 27.0) and ends before edit button (x = right - 54.0)
                            let text_x0 = card_rect.left() + 27.0;
                            let text_x1 = (card_rect.right() - 54.0).max(text_x0 + 4.0);
                            let text_rect = egui::Rect::from_min_max(
                                egui::pos2(text_x0, card_rect.top()),
                                egui::pos2(text_x1, card_rect.bottom()),
                            );
                            let text_avail = text_rect.width();

                            if let Some([w, h]) = crate::font_preview::name_size(col.ctx(), &preset.font_family, &preset.name, STYLE_NAME_PT, preset.bold, preset.italic)
                                .filter(|[w, _]| *w <= text_avail)
                            {
                                let target = egui::Rect::from_min_size(
                                    egui::pos2(text_rect.left(), (card_rect.center().y - h * 0.5).round()),
                                    vec2(w, h),
                                );
                                crate::font_preview::paint_name(col, &preset.font_family, &preset.name, STYLE_NAME_PT, target, text_color, preset.bold, preset.italic);
                            } else {
                                let mut job = egui::text::LayoutJob::simple_singleline(
                                    preset.name.clone(),
                                    egui::FontId::proportional(11.0),
                                    text_color,
                                );
                                job.wrap.max_width = text_avail;
                                job.wrap.break_anywhere = false;
                                let galley = col.painter().layout_job(job);
                                col.painter().galley(
                                    egui::pos2(text_rect.left(), (card_rect.center().y - galley.size().y * 0.5).round()),
                                    galley,
                                    text_color,
                                );
                            }

                            // 4. Main card interaction (click = select, 2xclick = apply and insert, context menu = actions)
                            let card_body_rect = egui::Rect::from_min_max(
                                egui::pos2(card_rect.left() + 15.0, card_rect.top()),
                                egui::pos2(card_rect.right() - 54.0, card_rect.bottom()),
                            );
                            let card_interact_resp = col.interact(card_body_rect, col.id().with(("typer_card_click", p_idx)), Sense::click());
                            if card_interact_resp.clicked() {
                                style_to_select = Some(p_idx);
                            }
                            if card_interact_resp.double_clicked() {
                                style_to_apply = Some(p_idx);
                            }

                            let sys_label = if preset.measurement_system.is_optical() { "Оптическая" } else { "Метрическая" };
                            let hex_c = format!("#{:02X}{:02X}{:02X}{:02X}", preset.text_color[0], preset.text_color[1], preset.text_color[2], preset.text_color[3]);
                            let tip = format!(
                                "{}\nШрифт: {}\nКегль: {}pt\nИнтерлиньяж: {}pt\nТрекинг: {}\nЦвет шрифта: {}\nСистема: {}\n\nЛКМ: выбрать стиль\n2xЛКМ: применить и вставить\nПКМ: меню действий",
                                preset.name, preset.font_family, preset.font_size, preset.leading, preset.tracking, hex_c, sys_label
                            );

                            card_interact_resp.on_hover_text(tip).context_menu(|ui| {
                                if ui.button("✏ Редактировать стиль...").clicked() {
                                    open_edit = true;
                                    ui.close();
                                }
                                if ui.button("+ Дублировать стиль").clicked() {
                                    style_to_duplicate = Some(p_idx);
                                    ui.close();
                                }
                                if ui.button("▶ Применить и вставить").clicked() {
                                    style_to_apply = Some(p_idx);
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button(RichText::new("🗑 Удалить стиль").color(t.danger)).clicked() {
                                    style_to_delete = Some(p_idx);
                                    ui.close();
                                }
                            });

                            if open_edit {
                                style_to_edit = Some(p_idx);
                            }
                        };

                        for chunk in matching_indices.chunks(cols_count) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for &p_idx in chunk {
                                    draw_card(ui, p_idx, col_w);
                                }
                            });
                            ui.add_space(3.0);
                        }
                    }
                    ui.add_space(4.0);
                }
            });

        if let Some(idx) = style_to_select {
            self.state.selected_preset_idx = idx;
            self.auto_save();
        }

        if let Some(idx) = style_to_edit {
            self.editing_preset_idx = Some(idx);
        }

        if let Some(idx) = style_to_duplicate {
            self.state.duplicate_preset(idx);
            self.auto_save();
        }

        if let Some(idx) = style_to_delete {
            if idx < self.state.presets.len() {
                self.state.presets.remove(idx);
                if self.state.selected_preset_idx >= self.state.presets.len() {
                    self.state.selected_preset_idx = self.state.presets.len().saturating_sub(1);
                }
                self.auto_save();
            }
        }

        if let Some(idx) = style_to_layer {
            self.state.selected_preset_idx = idx;
            *action = None;
            if let Some(preset) = self.state.presets.get(idx).cloned() {
                self.apply_style_requested = Some(preset);
            }
        }

        if let Some(idx) = style_to_apply {
            self.state.selected_preset_idx = idx;
            if let Some(line) = self.state.active_line() {
                if let Some(preset) = self.state.presets.get(idx).cloned() {
                    *action = Some(TypesetterAction {
                        text: line.text.clone(),
                        preset,
                    });
                    self.state.advance_line();
                    self.scroll_to_active_line = true;
                }
            }
            self.auto_save();
        }
    }
    /// Вкладка "Текст": окно всего текста с листанием наперёд и полным просмотром.
    fn render_full_text_content(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let t = Tokens::get(ui.ctx());
        let pages = self.state.pages();
        let current_page = self.state.current_page;
        let total_lines = self.state.lines.len();
        let applied_count = self.state.lines.iter().filter(|l| l.is_applied).count();
        let pending_count = total_lines.saturating_sub(applied_count);

        // Верхняя панель действий со сценарием (адаптивная к ширине)
        ui.horizontal(|ui| {
            let avail_w = ui.available_width();
            let is_narrow = avail_w < 300.0;

            let paste_btn_text = if is_narrow { "📋".to_string() } else { format!("📋 {}", tl!("Paste")) };
            if ui.button(RichText::new(paste_btn_text).strong()).on_hover_text(tl!("Paste script text from clipboard (Ctrl+V)")).clicked() {
                self.paste_script_from_clipboard();
            }

            let editor_btn_text = if is_narrow { "📝".to_string() } else { format!("📝 {}", tl!("Editor")) };
            if ui.button(RichText::new(editor_btn_text).strong()).on_hover_text(tl!("Open script text editor")).clicked() {
                self.open_script_editor();
            }

            let file_btn_text = if is_narrow { "📥 .txt".to_string() } else { format!("📥 {}", tl!("Open .txt")) };
            if ui.button(file_btn_text).on_hover_text(tl!("Load script file (.txt)")).clicked() {
                self.open_script_file_dialog();
            }

            if total_lines > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗑").on_hover_text(tl!("Clear entire script")).clicked() {
                        self.state.lines.clear();
                        self.state.current_line_idx = 0;
                        self.auto_save();
                    }
                });
            }
        });
        widgets::hairline(ui);
        ui.add_space(2.0);

        // Панель управления текстом
        ui.horizontal(|ui| {
            if ui.small_button("◀").on_hover_text(tl!("Previous page")).clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos > 0 {
                        self.state.current_page = pages[pos - 1];
                    }
                }
            }

            ui.label(RichText::new(format!("{} {}", tl!("Page"), current_page)).strong());

            if ui.small_button("▶").on_hover_text(tl!("Next page")).clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos + 1 < pages.len() {
                        self.state.current_page = pages[pos + 1];
                    }
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let detach_icon = if self.show_full_text_window { "🗖" } else { "🗗" };
                if ui.small_button(detach_icon).on_hover_text(tl!("Open script in separate window")).clicked() {
                    self.show_full_text_window = !self.show_full_text_window;
                }

                if ui.small_button("🎯").on_hover_text(tl!("Go to current line/page")).clicked() {
                    if let Some(cur) = self.state.lines.get(self.state.current_line_idx) {
                        self.state.current_page = cur.page;
                    }
                }

                ui.checkbox(&mut self.filter_current_page_only, tl!("Page"))
                    .on_hover_text(tl!("Show current page dialogues only"));
            });
        });

        // Поиск и статистика
        ui.horizontal(|ui| {
            let search_w = (ui.available_width() - 95.0).max(80.0);
            ui.add(egui::TextEdit::singleline(&mut self.full_text_filter).hint_text(format!("🔍 {}", tl!("Search…"))).desired_width(search_w));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{}/{} ({}: {})", applied_count, total_lines, tl!("left"), pending_count))
                        .weak()
                        .small(),
                );
            });
        });

        ui.separator();

        if total_lines == 0 {
            ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(tl!("No script loaded")).strong().color(t.text_dim));
                    ui.add_space(10.0);
                    let avail_w = ui.available_width();
                    if avail_w >= 360.0 {
                        ui.horizontal(|ui| {
                            if ui.button(RichText::new(format!("📋 {}", tl!("Paste from Clipboard"))).strong()).on_hover_text(tl!("Paste script from clipboard (Ctrl+V)")).clicked() {
                                self.paste_script_from_clipboard();
                            }
                            if ui.button(format!("📥 {}", tl!("Load .txt"))).on_hover_text(tl!("Load script file (.txt)")).clicked() {
                                self.open_script_file_dialog();
                            }
                            if ui.button(format!("📝 {}", tl!("Enter in Editor"))).on_hover_text(tl!("Enter script manually")).clicked() {
                                self.open_script_editor();
                            }
                        });
                    } else {
                        // Адаптивный вертикальный стек кнопок во всю ширину
                        ui.spacing_mut().item_spacing.y = 6.0;
                        let btn_w = (avail_w - 20.0).max(180.0);
                        if ui.add_sized([btn_w, 28.0], egui::Button::new(RichText::new(format!("📋 {}", tl!("Paste from Clipboard"))).strong())).on_hover_text(tl!("Paste script from clipboard (Ctrl+V)")).clicked() {
                            self.paste_script_from_clipboard();
                        }
                        if ui.add_sized([btn_w, 28.0], egui::Button::new(format!("📥 {}", tl!("Load .txt")))).on_hover_text(tl!("Load script file (.txt)")).clicked() {
                            self.open_script_file_dialog();
                        }
                        if ui.add_sized([btn_w, 28.0], egui::Button::new(format!("📝 {}", tl!("Enter in Editor")))).on_hover_text(tl!("Enter script manually")).clicked() {
                            self.open_script_editor();
                        }
                    }
                    ui.add_space(16.0);
                });
            });
            return;
        }

        // Список всего текста скроллируемый
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let filter = self.full_text_filter.to_lowercase();
                let mut line_to_set = None;
                let mut line_to_insert = None;
                let mut text_changed = false;
                let mut should_clear_scroll = false;
                let do_scroll = self.scroll_to_active_line;
                let cur_line_idx = self.state.current_line_idx;
                let mut last_rendered_page = 0;

                for (idx, line) in self.state.lines.iter_mut().enumerate() {
                    if self.filter_current_page_only && line.page != self.state.current_page {
                        continue;
                    }
                    if !filter.is_empty() && !line.text.to_lowercase().contains(&filter) {
                        continue;
                    }

                    // Разделитель страниц
                    if line.page != last_rendered_page {
                        last_rendered_page = line.page;
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("─── {} {} ───", tl!("Page"), line.page)).color(t.text_dim).strong().small());
                        });
                        ui.add_space(2.0);
                    }

                    let is_active = idx == cur_line_idx;
                    let (card_bg, card_border, text_color, num_color) = if is_active {
                        (
                            Color32::from_rgb(204, 232, 255),
                            Color32::from_rgb(0, 120, 215),
                            Color32::from_rgb(10, 25, 50),
                            Color32::from_rgb(0, 90, 180),
                        )
                    } else if line.is_applied {
                        (
                            Color32::from_rgb(250, 250, 252),
                            Color32::TRANSPARENT,
                            Color32::from_rgb(150, 155, 165),
                            Color32::from_rgb(170, 175, 185),
                        )
                    } else {
                        (
                            Color32::WHITE,
                            Color32::TRANSPARENT,
                            Color32::from_rgb(20, 25, 35),
                            Color32::from_rgb(110, 115, 125),
                        )
                    };

                    let card_frame = Frame::NONE
                        .fill(card_bg)
                        .stroke(Stroke::new(if is_active { 1.5 } else { 0.0 }, card_border))
                        .corner_radius(CornerRadius::same(3))
                        .inner_margin(Margin::symmetric(6, 4));

                    let row_resp = card_frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{:2}", idx + 1)).color(num_color).monospace().small());

                            let target_icon = if is_active { "●" } else { "◎" };
                            if ui.small_button(target_icon).on_hover_text(tl!("Make this line current")).clicked() {
                                line_to_set = Some((idx, true));
                            }

                            let mut do_insert = false;
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let (btn_text, btn_fill) = if line.is_applied {
                                    ("✓", Color32::from_rgb(46, 125, 50))
                                } else {
                                    ("▶", Color32::from_rgb(0, 120, 215))
                                };
                                let insert_btn = egui::Button::new(
                                    RichText::new(btn_text).color(Color32::WHITE).strong()
                                )
                                .fill(btn_fill)
                                .min_size(vec2(22.0, 16.0));

                                if ui.add(insert_btn).on_hover_text(tl!("Insert this line into bubble (Enter)")).clicked() {
                                    do_insert = true;
                                }

                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let edit = egui::TextEdit::singleline(&mut line.text)
                                        .font(egui::TextStyle::Body)
                                        .text_color(text_color)
                                        .frame(egui::Frame::NONE)
                                        .id_salt(egui::Id::new("typer_fulltext_line").with(idx));
                                    let edit_resp = ui.add(edit);
                                    if edit_resp.gained_focus() || edit_resp.clicked() {
                                        line_to_set = Some((idx, false));
                                    }
                                    if edit_resp.changed() {
                                        text_changed = true;
                                    }
                                });
                            });

                            if do_insert {
                                line_to_insert = Some((idx, line.clone()));
                            }
                        });
                    });

                    if is_active && do_scroll {
                        row_resp.response.scroll_to_me(Some(Align::Center));
                        should_clear_scroll = true;
                    }

                    ui.add_space(2.0);
                }

                if should_clear_scroll {
                    self.scroll_to_active_line = false;
                }

                if let Some((idx, req_scroll)) = line_to_set {
                    self.state.set_current_line(idx);
                    if req_scroll {
                        self.scroll_to_active_line = true;
                    }
                }

                if let Some((idx, line)) = line_to_insert {
                    if let Some(preset) = self.state.preset_for_line(idx).cloned() {
                        *action = Some(TypesetterAction {
                            text: line.text.clone(),
                            preset,
                        });
                        self.state.set_current_line(idx + 1);
                        self.scroll_to_active_line = true;
                        self.auto_save();
                    }
                }

                if text_changed {
                    self.auto_save();
                }
            });
    }

    /// Модальное окно для прямого ввода, вставки (Ctrl+V) и редактирования текста сценария.
    fn render_script_editor_modal(&mut self, ui: &mut egui::Ui, close_modal: &mut bool) {
        let t = Tokens::get(ui.ctx());

        ui.horizontal(|ui| {
            if ui.button(format!("📋 {}", tl!("Paste from Clipboard (Ctrl+V)"))).on_hover_text(tl!("Paste copied text from clipboard")).clicked() {
                if let Some(text) = get_system_clipboard_text() {
                    if !text.trim().is_empty() {
                        self.script_editor_buffer = text;
                    }
                }
            }

            #[cfg(not(target_arch = "wasm32"))]
            if ui.button(format!("📥 {}", tl!("Load file (.txt)…"))).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Текстовые файлы (*.txt)", &["txt"])
                    .set_title(tl!("Choose script file"))
                    .pick_file()
                {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        self.script_editor_buffer = content;
                    }
                }
            }

            if ui.button(tl!("Clear")).clicked() {
                self.script_editor_buffer.clear();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let line_count = self.script_editor_buffer.lines().filter(|l| !l.trim().is_empty()).count();
                ui.label(RichText::new(format!("{}: {}", tl!("Lines"), line_count)).color(t.text_dim).small());
            });
        });

        ui.add_space(4.0);
        ui.label(
            RichText::new(tl!("Each line becomes a separate dialogue entry. [Page 1] tags split pages."))
                .color(t.text_dim)
                .small(),
        );
        ui.add_space(4.0);

        let available_h = ui.available_height() - 42.0;
        egui::ScrollArea::vertical()
            .max_height(available_h.max(160.0))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.script_editor_buffer)
                        .hint_text(tl!("Paste script text here (Ctrl+V) or write dialogues…"))
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .lock_focus(true),
                );
            });

        ui.add_space(6.0);
        widgets::hairline(ui);
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let has_text = !self.script_editor_buffer.trim().is_empty();
            let apply_btn = egui::Button::new(
                RichText::new(format!("✓ {}", tl!("Load into typesetter"))).color(t.accent_text).strong()
            )
            .fill(t.accent);

            if ui.add_enabled(has_text, apply_btn).clicked() {
                self.script_input_buffer = self.script_editor_buffer.clone();
                self.state.load_script(&self.script_editor_buffer);
                self.sync_script_buffer_from_state();
                self.auto_save();
                self.status_message = Some(format!("{}: {}", tl!("Loaded dialogues"), self.state.lines.len()));
                self.status_error = false;
                *close_modal = true;
            }

            if ui.button(tl!("Cancel")).clicked() {
                *close_modal = true;
            }
        });
    }

    /// Модальное окно редактирования настроек конкретного шрифта и стиля.
    fn render_font_settings_modal(&mut self, ui: &mut egui::Ui, idx: usize, close_modal: &mut bool) {
        let available_folders = self.state.folders();
        let mut to_delete = false;
        let mut to_duplicate = false;
        let mut changed = false;
        let mut toggle_fav_font: Option<String> = None;
        let mut toggle_sync = false;
        let sync_leading = self.state.options.line_spacing_sync;

        if let Some(preset) = self.state.presets.get_mut(idx) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}:", tl!("Style name"))).strong());
                if ui.text_edit_singleline(&mut preset.name).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Folder")));
                let mut folder_str = preset.folder.clone().unwrap_or_default();
                egui::ComboBox::from_id_salt("edit_preset_folder_combo")
                    .selected_text(if folder_str.is_empty() { tl!("No folder") } else { &folder_str })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(folder_str.is_empty(), tl!("No folder")).clicked() {
                            preset.folder = None;
                            changed = true;
                        }
                        for f in &available_folders {
                            if ui.selectable_label(&folder_str == f, f).clicked() {
                                preset.folder = Some(f.clone());
                                changed = true;
                            }
                        }
                    });
                if ui.text_edit_singleline(&mut folder_str).changed() {
                    preset.folder = if folder_str.trim().is_empty() {
                        None
                    } else {
                        Some(folder_str.trim().to_string())
                    };
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Script style prefixes")));
                let mut prefixes = preset.script_prefixes.join(", ");
                if ui.text_edit_singleline(&mut prefixes).changed() {
                    preset.script_prefixes = prefixes
                        .split(',')
                        .map(str::trim)
                        .filter(|prefix| !prefix.is_empty())
                        .map(str::to_string)
                        .collect();
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Punctuation-only size")));
                for (label, value) in [("1×", 1.0f32), ("2×", 2.0), ("3×", 3.0)] {
                    if ui.selectable_label((preset.punctuation_only_scale - value).abs() < 0.01, label).clicked() {
                        preset.punctuation_only_scale = value;
                        changed = true;
                    }
                }
            })
            .response
            .on_hover_text(tl!("Enlarge lines made only of punctuation, such as ?!"));

            ui.separator();
            ui.label(RichText::new(tl!("Paragraph")).strong());
            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Orientation")));
                egui::ComboBox::from_id_salt("typer_style_orientation")
                    .selected_text(if preset.orientation == "vertical" { tl!("Vertical") } else { tl!("Horizontal") })
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut preset.orientation, "horizontal".to_string(), tl!("Horizontal")).clicked() {
                            changed = true;
                        }
                        if ui.selectable_value(&mut preset.orientation, "vertical".to_string(), tl!("Vertical")).clicked() {
                            changed = true;
                        }
                    });
                ui.label(format!("{}:", tl!("Alignment")));
                let alignment_label = match preset.alignment.as_str() {
                    "left" => tl!("Left"),
                    "right" => tl!("Right"),
                    "justify" => tl!("Justify"),
                    "justifyCenter" => tl!("Justify last centered"),
                    "justifyRight" => tl!("Justify last right"),
                    "justifyAll" => tl!("Justify all"),
                    _ => tl!("Center"),
                };
                egui::ComboBox::from_id_salt("typer_style_alignment")
                    .selected_text(alignment_label)
                    .show_ui(ui, |ui| {
                        for (label, value) in [
                            (tl!("Left"), "left"),
                            (tl!("Center"), "center"),
                            (tl!("Right"), "right"),
                            (tl!("Justify"), "justify"),
                            (tl!("Justify last centered"), "justifyCenter"),
                            (tl!("Justify last right"), "justifyRight"),
                            (tl!("Justify all"), "justifyAll"),
                        ] {
                            if ui.selectable_value(&mut preset.alignment, value.to_string(), label).clicked() {
                                changed = true;
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                for (label, value) in [
                    (tl!("First Line Indent"), &mut preset.first_line_indent),
                    (tl!("Indent Left"), &mut preset.start_indent),
                    (tl!("Indent Right"), &mut preset.end_indent),
                ] {
                    ui.label(label);
                    if ui.add(egui::DragValue::new(value).speed(0.5).suffix(" pt").range(-500.0..=500.0)).changed() {
                        changed = true;
                    }
                }
            });
            ui.horizontal(|ui| {
                for (label, value) in [
                    (tl!("Space Before"), &mut preset.space_before),
                    (tl!("Space After"), &mut preset.space_after),
                ] {
                    ui.label(label);
                    if ui.add(egui::DragValue::new(value).speed(0.5).suffix(" pt").range(-500.0..=500.0)).changed() {
                        changed = true;
                    }
                }
                if ui.checkbox(&mut preset.hyphenate, tl!("Hyphenate")).changed() {
                    changed = true;
                }
            });

            ui.separator();

            ui.label(RichText::new(tl!("Typography and Font")).strong());
            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Font")));
                // Quick star toggle for current font
                let is_fav = crate::type_tool::is_favorite_font(ui.ctx(), &preset.font_family);
                let star_text = if is_fav { "★" } else { "☆" };
                let star_color = if is_fav { egui::Color32::from_rgb(240, 190, 50) } else { ui.visuals().weak_text_color() };
                if ui.button(RichText::new(star_text).color(star_color)).on_hover_text(tl!("Favorite font")).clicked() {
                    toggle_fav_font = Some(preset.font_family.clone());
                }

                // Searchable font dropdown with pinned Favorites at the top!
                let prev_fam = preset.font_family.clone();
                let families = crate::type_tool::families();
                if crate::type_tool::family_picker_in(ui, "typesetter-preset-font", &mut preset.font_family, 180.0, &families, photocraft_engine::prefs::FontPreview::Medium, &[]) {
                    changed = true;
                    if preset.font_family != prev_fam {
                        if let Some(engine) = photocraft_text::try_shared() {
                            let faces = engine.lock().unwrap_or_else(std::sync::PoisonError::into_inner).fonts.faces(&preset.font_family);
                            if let Some(first_face) = faces.first() {
                                if let Some(ps) = &first_face.postscript_name {
                                    preset.font_postscript = ps.clone();
                                }
                                preset.font_style = first_face.style.clone();
                            }
                        }
                    }
                }
                if ui.text_edit_singleline(&mut preset.font_family).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("PostScript:");
                if ui.text_edit_singleline(&mut preset.font_postscript).changed() {
                    changed = true;
                }
                ui.label(format!("{}:", tl!("Style")));
                if ui.text_edit_singleline(&mut preset.font_style).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                if ui.checkbox(&mut preset.bold, tl!("Bold")).changed() {
                    if preset.bold && !preset.font_style.to_lowercase().contains("bold") {
                        preset.font_style = if preset.italic { "Bold Italic".into() } else { "Bold".into() };
                    } else if !preset.bold && preset.font_style.to_lowercase().contains("bold") {
                        preset.font_style = if preset.italic { "Italic".into() } else { "Regular".into() };
                    }
                    changed = true;
                }
                if ui.checkbox(&mut preset.italic, tl!("Italic")).changed() {
                    if preset.italic && !preset.font_style.to_lowercase().contains("italic") {
                        preset.font_style = if preset.bold { "Bold Italic".into() } else { "Italic".into() };
                    } else if !preset.italic && preset.font_style.to_lowercase().contains("italic") {
                        preset.font_style = if preset.bold { "Bold".into() } else { "Regular".into() };
                    }
                    changed = true;
                }
                if ui.checkbox(&mut preset.all_caps, "ALL CAPS").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut preset.hanging_punctuation, tl!("Компенсация знаков"))
                    .on_hover_text("Оптическая компенсация знаков в строках (висячая пунктуация для кавычек, тире, многоточий и точек)")
                    .changed()
                {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Anti-aliasing")));
                let cur_aa = preset.anti_alias.clone();
                let aa_labels = [
                    ("crisp", "Чёткий"),
                    ("sharp", "Резкий"),
                    ("smooth", "Плавный"),
                    ("strong", "Жирный"),
                    ("none", "Нет"),
                ];
                let display_label = aa_labels.iter().find(|(k, _)| *k == cur_aa).map(|(_, l)| *l).unwrap_or(cur_aa.as_str());
                egui::ComboBox::from_id_salt("style_modal_aa")
                    .selected_text(display_label)
                    .show_ui(ui, |ui| {
                        for (k, label) in aa_labels {
                            if ui.selectable_label(cur_aa == k, label).clicked() {
                                preset.anti_alias = k.to_string();
                                changed = true;
                            }
                        }
                    });
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Color")));
                let mut text_c = egui::Color32::from_rgba_unmultiplied(
                    preset.text_color[0],
                    preset.text_color[1],
                    preset.text_color[2],
                    preset.text_color[3],
                );
                if egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut text_c,
                    egui::color_picker::Alpha::BlendOrAdditive,
                ).changed() {
                    preset.text_color = [text_c.r(), text_c.g(), text_c.b(), text_c.a()];
                    changed = true;
                }
                ui.label(format!("#{:02X}{:02X}{:02X}{:02X}", preset.text_color[0], preset.text_color[1], preset.text_color[2], preset.text_color[3]));
            });

            ui.separator();

            ui.label(RichText::new(tl!("Typography Size (Line Spacing Sync)")).strong());
            ui.horizontal(|ui| {
                ui.label("Система:");
                let is_optical = preset.measurement_system.is_optical();
                if ui.selectable_label(!is_optical, tl!("Metrics")).on_hover_text("Метрическая система: стандартные метрики шрифта и базовая линия").clicked() {
                    preset.measurement_system = photocraft_engine::typesetter::MeasurementSystem::Metric;
                    changed = true;
                }
                if ui.selectable_label(is_optical, tl!("Optical")).on_hover_text("Оптическая система: визуальный оптический кернинг и центровка баббла").clicked() {
                    preset.measurement_system = photocraft_engine::typesetter::MeasurementSystem::Optical;
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Size (pt)")));
                let mut size = preset.font_size;
                if ui.add(egui::DragValue::new(&mut size).speed(0.5).range(4.0..=200.0)).changed() {
                    if sync_leading {
                        preset.sync_leading_with_size(size);
                    } else {
                        preset.font_size = size;
                    }
                    changed = true;
                }

                if ui.small_button("A-").clicked() {
                    let new_sz = (preset.font_size - 1.0).max(4.0);
                    if sync_leading {
                        preset.sync_leading_with_size(new_sz);
                    } else {
                        preset.font_size = new_sz;
                    }
                    changed = true;
                }
                if ui.small_button("A+").clicked() {
                    let new_sz = preset.font_size + 1.0;
                    if self.state.options.line_spacing_sync {
                        preset.sync_leading_with_size(new_sz);
                    } else {
                        preset.font_size = new_sz;
                    }
                    changed = true;
                }

                let link_icon = if sync_leading { "🔗" } else { "⛓" };
                let link_tip = if sync_leading {
                    "Связка кегля и интерлиньяжа активна (Leading меняется пропорционально)"
                } else {
                    "Связка кегля и интерлиньяжа отключена (Leading независим)"
                };
                if ui.selectable_label(sync_leading, link_icon).on_hover_text(link_tip).clicked() {
                    toggle_sync = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Leading")));
                if ui.add(egui::DragValue::new(&mut preset.leading).speed(0.5).range(4.0..=300.0)).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Tracking")));
                if ui.add(egui::DragValue::new(&mut preset.tracking).speed(1.0).range(-200..=1000)).changed() {
                    changed = true;
                }
                ui.label(format!("{}:", tl!("Horizontal scale (%)")));
                if ui.add(egui::DragValue::new(&mut preset.horizontal_scale).speed(1.0).range(20.0..=300.0)).changed() {
                    changed = true;
                }
            });

            ui.separator();

            ui.label(RichText::new(tl!("Text Outline")).strong());
            ui.horizontal(|ui| {
                ui.label(format!("{}:", tl!("Thickness (px)")));
                if ui.add(egui::DragValue::new(&mut preset.stroke_width).speed(0.2).range(0.0..=30.0)).changed() {
                    changed = true;
                }
                let desc = if preset.stroke_width > 0.0 { tl!("Enabled") } else { tl!("No outline") };
                ui.label(RichText::new(desc).small().weak());
            });

            if preset.stroke_width > 0.0 {
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", tl!("Color")));
                    let mut stroke_c = egui::Color32::from_rgba_unmultiplied(
                        preset.stroke_color[0],
                        preset.stroke_color[1],
                        preset.stroke_color[2],
                        preset.stroke_color[3],
                    );
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut stroke_c,
                        egui::color_picker::Alpha::BlendOrAdditive,
                    ).changed() {
                        preset.stroke_color = [stroke_c.r(), stroke_c.g(), stroke_c.b(), stroke_c.a()];
                        changed = true;
                    }
                    ui.label(format!("#{:02X}{:02X}{:02X}{:02X}", preset.stroke_color[0], preset.stroke_color[1], preset.stroke_color[2], preset.stroke_color[3]));
                });
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button(tl!("Duplicate")).on_hover_text(tl!("Duplicate this style")).clicked() {
                    to_duplicate = true;
                }
                if ui.button(tl!("Delete")).on_hover_text(tl!("Delete this style")).clicked() {
                    to_delete = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(tl!("Done")).clicked() {
                        *close_modal = true;
                    }
                });
            });
        }

        if toggle_sync {
            self.state.options.line_spacing_sync = !self.state.options.line_spacing_sync;
            self.auto_save();
        }

        if let Some(font) = toggle_fav_font {
            let now_fav = crate::type_tool::toggle_favorite_font(ui.ctx(), &font);
            if now_fav {
                if !self.favorite_fonts.contains(&font) {
                    self.favorite_fonts.push(font);
                }
            } else {
                self.favorite_fonts.retain(|f| f != &font);
            }
            changed = true;
        }

        if to_duplicate {
            self.state.duplicate_preset(idx);
            *close_modal = true;
        } else if to_delete {
            self.state.delete_preset(idx);
            *close_modal = true;
        }

        if changed {
            self.state.recompute_script_style_matches();
            self.auto_save();
        }
    }

    /// Вкладка "Опции": общие настройки тайпсеттера, привязки и сохранение.
    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());

        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.label(RichText::new(tl!("TYPESETTER BEHAVIOR")).size(11.0).color(t.text_dim).strong());

            if ui.checkbox(
                &mut self.state.options.auto_page_switch,
                tl!("Auto-advance to next page"),
            ).on_hover_text(tl!("Automatically advance script to the next page")).changed() {
                self.auto_save();
            }

            if ui.checkbox(
                &mut self.state.options.line_spacing_sync,
                tl!("Sync line spacing"),
            ).on_hover_text(tl!("Sync line spacing with font size")).changed() {
                self.auto_save();
            }

            ui.label(tl!("Minimum size when fitting to a bubble"));
            if ui.add(
                egui::Slider::new(&mut self.state.options.minimum_fit_scale_percent, 50.0..=100.0)
                    .suffix("%")
                    .max_decimals(0),
            ).on_hover_text(tl!("Keep the original size when possible; shrink only to this percentage")).changed() {
                self.auto_save();
            }

            if ui.checkbox(
                &mut self.state.options.strip_character_names,
                tl!("Strip character names"),
            ).on_hover_text(tl!("Strip character names at start of line ('HERO: Text')")).changed() {
                self.auto_save();
            }

            if ui.checkbox(
                &mut self.state.options.strip_dash_prefix,
                tl!("Strip leading dashes"),
            ).on_hover_text(tl!("Strip leading dashes ('- Text', '— Text')")).changed() {
                self.auto_save();
            }

            let mut matching_changed = ui.checkbox(
                &mut self.state.options.match_style_prefixes,
                tl!("Auto-select style from script prefixes"),
            ).changed();
            matching_changed |= ui.checkbox(
                &mut self.state.options.match_prefixes_across_folders,
                tl!("Match prefixes across all style folders"),
            ).changed();
            if matching_changed {
                self.state.recompute_script_style_matches();
                self.auto_save();
            }

            if ui.checkbox(
                &mut self.state.options.paste_on_click,
                tl!("Paste current line when a selection is made"),
            ).changed() {
                self.auto_save();
            }

            if ui.checkbox(
                &mut self.state.options.hanging_punctuation,
                tl!("Компенсация знаков в строках"),
            ).on_hover_text(tl!("Оптическая компенсация и вынос знаков препинания за границы строк (висячая пунктуация)")).changed() {
                self.auto_save();
            }

            widgets::hairline(ui);
            ui.add_space(4.0);

            ui.label(RichText::new(tl!("GEOMETRY & CENTERING")).size(11.0).color(t.text_dim).strong());

            let avail_w = ui.available_width();
            if avail_w < 340.0 {
                ui.label(RichText::new(tl!("Baseline shift:")).color(t.text));
                let slider_w = (ui.available_width() - 8.0).max(120.0);
                if ui.add_sized(
                    [slider_w, 20.0],
                    egui::Slider::new(&mut self.state.options.optical_baseline_factor, 0.0..=0.6)
                        .show_value(true)
                        .suffix(" ×")
                        .max_decimals(3),
                ).changed() {
                    self.auto_save();
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tl!("Baseline shift:")).color(t.text));
                    let slider_w = (ui.available_width() - 8.0).max(120.0);
                    if ui.add_sized(
                        [slider_w, 20.0],
                        egui::Slider::new(&mut self.state.options.optical_baseline_factor, 0.0..=0.6)
                            .show_value(true)
                            .suffix(" ×")
                            .max_decimals(3),
                    ).changed() {
                        self.auto_save();
                    }
                });
            }
            ui.label(
                RichText::new(tl!("Shifting the baseline down ensures precise visual vertical centering inside bubbles."))
                    .color(t.text_faint)
                    .small(),
            );

            widgets::hairline(ui);
            ui.add_space(4.0);

            ui.label(RichText::new(tl!("PRESET STORAGE")).size(11.0).color(t.text_dim).strong());
            if let Some(path) = self.storage_path() {
                let dir_str = path.parent().map(|p| p.display().to_string()).unwrap_or_else(|| ".".into());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}:", tl!("Folder"))).color(t.text_dim).small());
                    ui.add(
                        egui::Label::new(
                            RichText::new(&dir_str)
                                .color(t.text_faint)
                                .small(),
                        ).truncate(),
                    ).on_hover_text(&dir_str);
                });
                let file_str = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "typesetter_state.json".into());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}:", tl!("File"))).color(t.text_dim).small());
                    ui.add(
                        egui::Label::new(
                            RichText::new(&file_str)
                                .color(t.text_faint)
                                .small(),
                        ).truncate(),
                    ).on_hover_text(path.display().to_string());
                });
            }

            ui.horizontal_wrapped(|ui| {
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button(tl!("Choose folder…")).on_hover_text(tl!("Choose a custom directory to store typesetter presets and state")).clicked() {
                    let mut dlg = rfd::FileDialog::new().set_title(tl!("Choose presets folder"));
                    if let Some(curr) = self.storage_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                        dlg = dlg.set_directory(&curr);
                    }
                    if let Some(folder) = dlg.pick_folder() {
                        self.custom_storage_dir = Some(folder.clone());
                        self.auto_save();
                        self.status_message = Some(format!("{}: {}", tl!("Storage folder set"), folder.display()));
                        self.status_error = false;
                    }
                }
                if ui.button(tl!("Open folder")).on_hover_text(tl!("Open presets folder in file manager")).clicked() {
                    if let Some(path) = self.storage_path() {
                        if let Some(dir) = path.parent() {
                            #[cfg(windows)]
                            {
                                let _ = std::process::Command::new("explorer").arg(dir).spawn();
                            }
                            #[cfg(target_os = "macos")]
                            {
                                let _ = std::process::Command::new("open").arg(dir).spawn();
                            }
                            #[cfg(not(any(windows, target_os = "macos")))]
                            {
                                let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
                            }
                        }
                    }
                }
                if ui.button(tl!("Save now")).clicked() {
                    self.auto_save();
                    self.status_message = Some(tl!("Presets saved").into());
                    self.status_error = false;
                }
                if ui.button(tl!("Load from file")).clicked() {
                    self.auto_load();
                    self.status_message = Some(tl!("Presets loaded from file").into());
                    self.status_error = false;
                }
                if self.custom_storage_dir.is_some() {
                    if ui.button(tl!("Reset folder")).on_hover_text(tl!("Reset presets storage to default AppData directory")).clicked() {
                        self.custom_storage_dir = None;
                        self.auto_save();
                        self.status_message = Some(tl!("Storage folder reset to default").into());
                        self.status_error = false;
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tpl_file_reader_rejects_oversized_files() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(photocraft_engine::typesetter::MAX_TPL_FILE_BYTES as u64 + 1).unwrap();

        let error = read_tpl_file(file.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
    use photocraft_doc::LayerContent;
    use photocraft_engine::typesetter::MeasurementSystem;
    use serde_json::json;

    #[test]
    fn document_style_scan_request_is_consumed_once() {
        let mut ui = TypesetterUi { document_style_scan_requested: true, ..TypesetterUi::default() };
        assert!(ui.take_document_style_scan_request());
        assert!(!ui.take_document_style_scan_request());
    }

    #[test]
    fn document_style_file_scan_request_is_taken_once() {
        let path = std::path::PathBuf::from("sample.psd");
        let mut ui = TypesetterUi { document_style_file_scan_requested: Some(path.clone()), ..TypesetterUi::default() };
        assert_eq!(ui.take_document_style_file_scan_request(), Some(path));
        assert_eq!(ui.take_document_style_file_scan_request(), None);
    }

    #[test]
    fn typesetter_action_applies_font_color_and_measurement_system() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();

        // 1. Action with Optical measurement system and custom RGBA text color
        let action_optical = TypesetterAction {
            text: "Optical Text".into(),
            preset: TextStylePreset {
                name: "Test Optical".into(),
                font_size: 24.0,
                text_color: [255, 100, 50, 200],
                measurement_system: MeasurementSystem::Optical,
                ..TextStylePreset::default()
            },
        };

        app.handle_typesetter_action(action_optical);

        let st = app.session.active().unwrap();
        let layer_id = st.active_layer.unwrap();
        let layer = st.doc.layer(layer_id).unwrap();
        if let LayerContent::Text(t) = &layer.content {
            assert_eq!(t.text, "OPTICAL TEXT"); // all_caps default is true
            let style = &t.runs[0].style;
            assert_eq!(style.kerning, photocraft_doc::text::Kerning::Optical);
            let rgb = style.color.to_rgb();
            assert!((rgb[0] - (255.0 / 255.0)).abs() < 0.02);
            assert!((rgb[1] - (100.0 / 255.0)).abs() < 0.02);
            assert!((rgb[2] - (50.0 / 255.0)).abs() < 0.02);
            assert!((style.color.alpha - (200.0 / 255.0)).abs() < 0.02);
        } else {
            panic!("Expected text layer");
        }

        // 2. Action with Metric measurement system
        let action_metric = TypesetterAction {
            text: "Metric Text".into(),
            preset: TextStylePreset {
                name: "Test Metric".into(),
                font_size: 20.0,
                text_color: [0, 128, 255, 255],
                measurement_system: MeasurementSystem::Metric,
                all_caps: false,
                ..TextStylePreset::default()
            },
        };

        app.handle_typesetter_action(action_metric);

        let st = app.session.active().unwrap();
        let layer_id = st.active_layer.unwrap();
        let layer = st.doc.layer(layer_id).unwrap();
        if let LayerContent::Text(t) = &layer.content {
            assert_eq!(t.text, "Metric Text");
            let style = &t.runs[0].style;
            assert_eq!(style.kerning, photocraft_doc::text::Kerning::Metrics);
            let rgb = style.color.to_rgb();
            assert!((rgb[0] - 0.0).abs() < 0.02);
            assert!((rgb[1] - (128.0 / 255.0)).abs() < 0.02);
            assert!((rgb[2] - 1.0).abs() < 0.02);
        } else {
            panic!("Expected text layer");
        }
    }

    #[test]
    fn typesetter_action_fits_text_to_selection_contour() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 260, "height": 220})).unwrap();
        let bounds = photocraft_geom::Rect::new(0, 0, 260, 220);
        let mut pixels = vec![0.0; bounds.width() as usize * bounds.height() as usize];
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let dx = (x as f32 - 130.0) / 128.0;
                let dy = (y as f32 - 110.0) / 108.0;
                if dx * dx + dy * dy <= 1.0
                    && let Some(pixel) = pixels.get_mut(y as usize * bounds.width() as usize + x as usize)
                {
                    *pixel = 1.0;
                }
            }
        }
        let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &pixels);
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(selection);

        app.handle_typesetter_action(TypesetterAction {
            text: "This is a longer sample dialogue that needs several carefully balanced lines inside the selected speech bubble".into(),
            preset: TextStylePreset {
                font_family: "Inter".into(),
                font_postscript: String::new(),
                font_size: 24.0,
                leading: 28.0,
                ..TextStylePreset::default()
            },
        });

        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let LayerContent::Text(text) = &layer.content else {
            panic!("Expected text layer");
        };
        assert!(text.text.contains('\n'));
        assert!(text.runs[0].style.size_pt < 24.0);
    }

    #[test]
    fn typesetter_action_enlarges_punctuation_only_lines() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.handle_typesetter_action(TypesetterAction {
            text: "?!".into(),
            preset: TextStylePreset { font_size: 20.0, leading: 24.0, punctuation_only_scale: 2.0, ..TextStylePreset::default() },
        });

        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let LayerContent::Text(text) = &layer.content else {
            panic!("Expected text layer");
        };
        assert_eq!(text.text, "?!");
        assert_eq!(text.runs[0].style.size_pt, 40.0);
    }

    fn rect_selection(x0: i32, y0: i32, width: i32, height: i32) -> photocraft_raster::Surface {
        let bounds = photocraft_geom::Rect::new(x0, y0, x0 + width, y0 + height);
        let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        selection.write_region(bounds, &vec![1.0; width as usize * height as usize]);
        selection
    }

    #[test]
    fn paste_on_click_pastes_the_active_line_when_the_selection_changes() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.typesetter.state.options.paste_on_click = true;
        app.typesetter.state.load_script("Clicked line");
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(10, 10, 80, 80));
        // The first frame only arms the toggle.
        app.maybe_typesetter_paste_on_click();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(20, 20, 90, 90));
        app.maybe_typesetter_paste_on_click();

        let st = app.session.active().unwrap();
        assert!(st.doc.walk().into_iter().any(|(_, _, layer)| matches!(&layer.content, LayerContent::Text(text) if {
            let normalized: String = text.text.chars().filter(|ch| ch.is_alphanumeric()).collect();
            normalized.to_uppercase().contains("CLICKED")
        })));
    }

    #[test]
    fn multi_bubble_capture_fills_consecutive_lines() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.typesetter.state.load_script("First chamber\nSecond chamber");
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(10, 10, 100, 80));
        app.capture_typesetter_bubble();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(200, 200, 100, 80));
        app.capture_typesetter_bubble();
        assert_eq!(app.typesetter.multi_bubble_count(), 2);

        app.fill_typesetter_bubbles();

        let st = app.session.active().unwrap();
        let texts: Vec<String> = st.doc.walk().into_iter().filter_map(|(_, _, layer)| match &layer.content {
            LayerContent::Text(text) => Some(text.text.clone()),
            _ => None,
        }).collect();
        assert!(texts.iter().any(|text| text.contains("FIRST")));
        assert!(texts.iter().any(|text| text.contains("SECOND")));
    }

    #[test]
    fn text_shape_reads_and_updates_the_selected_text_layer() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app
            .run("type.create", json!({"x": 20, "y": 60, "text": "A selected text layer with enough words to break", "size": 24}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        app.session.active_mut().unwrap().active_layer = Some(photocraft_doc::LayerId(id));

        app.typesetter.state.load_script("Queue line that must not appear");
        app.refresh_typesetter_text_shape(true);
        assert!(app.typesetter.has_text_shape_candidates(), "the selected text layer must drive TextShapeR");
        assert!(
            app.typesetter
                .first_text_shape_candidate_text()
                .is_some_and(|text| text.contains("selected text layer")),
            "the queue must not populate the TextShapeR fields"
        );

        app.apply_typesetter_text_shape(id, "A selected text layer\nwith enough words to break".into());
        let layer = app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap();
        let LayerContent::Text(text) = &layer.content else {
            panic!("Expected text layer");
        };
        assert!(text.text.contains('\n'));
        assert!(app.typesetter.has_text_shape_candidates(), "candidates remain interactive after applying one");
    }

    #[test]
    fn typesetter_action_applies_paragraph_settings() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.handle_typesetter_action(TypesetterAction {
            text: "First line\nSecond line".into(),
            preset: TextStylePreset {
                alignment: "right".into(),
                first_line_indent: 12.0,
                start_indent: 4.0,
                space_after: 3.0,
                hyphenate: true,
                ..TextStylePreset::default()
            },
        });

        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let LayerContent::Text(text) = &layer.content else {
            panic!("Expected text layer");
        };
        let paragraph = text.paragraphs.first().expect("a paragraph run");
        assert_eq!(paragraph.style.align, photocraft_doc::text::TextAlign::Right);
        assert_eq!(paragraph.style.first_line_indent_pt, 12.0);
        assert_eq!(paragraph.style.start_indent_pt, 4.0);
        assert_eq!(paragraph.style.space_after_pt, 3.0);
        assert!(paragraph.style.hyphenate);
    }

    #[test]
    fn typesetter_action_keeps_script_markdown_editable() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.handle_typesetter_action(TypesetterAction {
            text: "Regular **bold** and *italic*".into(),
            preset: TextStylePreset { all_caps: false, ..TextStylePreset::default() },
        });

        let st = app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let LayerContent::Text(text) = &layer.content else {
            panic!("Expected text layer");
        };
        assert_eq!(text.text, "Regular bold and italic");
        let bold = text.runs.iter().find(|run| run.len > 0 && run.style.faux_bold);
        let italic = text.runs.iter().find(|run| run.len > 0 && run.style.faux_italic);
        assert_eq!(bold.map(|run| run.len), Some(4));
        assert_eq!(italic.map(|run| run.len), Some(6));
    }

    /// The style-settings window must be draggable. It used to be `.anchor`ed to the screen
    /// centre, which re-pinned it there every frame, so its title bar could not be dragged.
    #[test]
    fn font_settings_window_can_be_dragged() {
        use egui_kittest::Harness;

        let mut typer = TypesetterUi::default();
        if typer.state.presets.is_empty() {
            typer.state.presets.push(TextStylePreset::default());
        }
        typer.is_open = true;
        typer.editing_preset_idx = Some(0);
        let mut frames = 0u32;
        // Tall enough for the whole style dialog: the test drags it upward, and a window
        // taller than the harness cannot move up by the asserted distance.
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(
            move |ui, typer: &mut TypesetterUi| {
                crate::PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ALL[0]);
                frames += 1;
                if frames < 2 {
                    // The named font families reach the context one pass after installation.
                    return;
                }
                typer.show(ui);
            },
            typer,
        );
        h.run_steps(3);
        let id = egui::Id::new("typer_font_settings_window");
        let before = h.ctx.memory(|m| m.area_rect(id)).expect("the style window is shown");
        let grab = egui::pos2(before.center().x, before.top() + 10.0);
        let drop = grab + egui::vec2(-80.0, -60.0);
        h.hover_at(grab);
        h.drag_at(grab);
        h.run_steps(1);
        h.hover_at(drop);
        h.run_steps(1);
        h.drop_at(drop);
        h.run_steps(2);
        let after = h.ctx.memory(|m| m.area_rect(id)).expect("the style window is still shown");
        assert!(after.top() < before.top() - 20.0, "the title drag must move the window up: {before:?} -> {after:?}");
        assert!(after.left() < before.left() - 20.0, "and sideways: {before:?} -> {after:?}");
    }

    /// The docked panel body is drawn by `render_embedded`; it must still open the style dialog,
    /// which used to be reachable only through the floating `show` path.
    #[test]
    fn docked_panel_opens_the_style_dialog() {
        use egui_kittest::Harness;

        let mut typer = TypesetterUi::default();
        if typer.state.presets.is_empty() {
            typer.state.presets.push(TextStylePreset::default());
        }
        typer.is_open = true;
        typer.editing_preset_idx = Some(0);
        let mut frames = 0u32;
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
            move |ui, typer: &mut TypesetterUi| {
                crate::PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ALL[0]);
                frames += 1;
                if frames < 2 {
                    return;
                }
                let _ = typer.render_embedded(ui);
            },
            typer,
        );
        h.run_steps(3);
        let id = egui::Id::new("typer_font_settings_window");
        assert!(h.ctx.memory(|m| m.area_rect(id)).is_some(), "the docked panel opens the style dialog");
    }
    /// Favorites and the one-time migration flag survive the state file round trip, including an
    /// empty list (un-starring every font must persist as empty, not resurrect old favorites).
    #[test]
    fn favorites_and_the_migration_flag_round_trip() {
        let round = |favorites: Vec<String>, merged: bool| {
            let persisted = TypesetterPersistedState {
                dock_mode: TypesetterDockMode::Floating,
                state: TypesetterState::default(),
                selected_folder: None,
                custom_storage_dir: None,
                favorite_fonts: favorites,
                is_open: true,
                script_list_height: 160.0,
                legacy_merged: merged,
            };
            let json = serde_json::to_string(&persisted).unwrap();
            serde_json::from_str::<TypesetterPersistedState>(&json).unwrap()
        };
        let kept = round(vec!["Moritat".into(), "Inter".into()], true);
        assert_eq!(kept.favorite_fonts, ["Moritat", "Inter"]);
        assert!(kept.legacy_merged);
        let cleared = round(Vec::new(), true);
        assert!(cleared.favorite_fonts.is_empty());
    }

    /// The store lives in the user profile, not next to whichever exe was launched: updating the
    /// exe from another folder must keep favorites.
    #[test]
    fn the_state_store_is_exe_independent() {
        let path = typesetter_storage_path().expect("a profile store");
        assert!(path.ends_with("typesetter_state.json"));
        #[cfg(target_os = "windows")]
        assert!(path.to_string_lossy().contains("AppData"), "{path:?}");
        assert!(!path.to_string_lossy().contains("target"), "{path:?}");
    }

    #[test]
    fn test_unified_script_editor_sync_and_operations() {
        let mut ui = TypesetterUi::default();
        // 1. Initial script load
        ui.state.load_script("Линия 1: Привет!\nЛиния 2: Мир!\nЛиния 3: Тайпер!");
        ui.sync_script_buffer_from_state();
        assert_eq!(ui.state.lines.len(), 3);
        assert_eq!(ui.script_buffer, "Линия 1: Привет!\nЛиния 2: Мир!\nЛиния 3: Тайпер!");

        // 2. User presses Backspace at start of line 2 (merging line 1 and line 2)
        ui.script_buffer = "Линия 1: Привет! Линия 2: Мир!\nЛиния 3: Тайпер!".to_string();
        ui.sync_state_from_script_buffer();
        assert_eq!(ui.state.lines.len(), 2);
        assert_eq!(ui.state.lines[0].raw, "Линия 1: Привет! Линия 2: Мир!");
        assert_eq!(ui.state.lines[0].text, "Привет! Линия 2: Мир!");
        assert_eq!(ui.state.lines[1].raw, "Линия 3: Тайпер!");
        assert_eq!(ui.state.lines[1].text, "Тайпер!");

        // 3. User presses Enter in the middle of line 1 (splitting into new line)
        ui.script_buffer = "Линия 1: Привет!\nНовая строка!\nЛиния 3: Тайпер!".to_string();
        ui.sync_state_from_script_buffer();
        assert_eq!(ui.state.lines.len(), 3);
        assert_eq!(ui.state.lines[1].text, "Новая строка!");
        assert_eq!(ui.state.lines[2].text, "Тайпер!");

        // 4. Height persistence
        ui.script_list_height = 240.0;
        let persisted = TypesetterPersistedState {
            dock_mode: ui.dock_mode,
            state: ui.state.clone(),
            selected_folder: ui.selected_folder.clone(),
            custom_storage_dir: ui.custom_storage_dir.clone(),
            favorite_fonts: ui.favorite_fonts.clone(),
            is_open: ui.is_open,
            script_list_height: ui.script_list_height,
            legacy_merged: true,
        };
        let json_str = serde_json::to_string(&persisted).unwrap();
        let loaded: TypesetterPersistedState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(loaded.script_list_height, 240.0);
    }
}
