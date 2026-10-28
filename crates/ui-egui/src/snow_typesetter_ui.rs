use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    Window, vec2,
};
use photocraft_engine::typesetter::{ScriptLine, TextStylePreset, TypesetterState};
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

/// Point size for style names drawn in their own face on the style cards.
const STYLE_NAME_PT: f32 = 13.0;

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
}

pub fn typesetter_storage_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir).join("typesetter_state.json"));
    }
    // Portable mode: check next to executable or portable.txt
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let local = parent.join("typesetter_state.json");
            if local.exists() || parent.join("portable.txt").exists() {
                return Some(local);
            }
        }
    }
    let cwd_local = std::path::PathBuf::from("typesetter_state.json");
    if cwd_local.exists() {
        return Some(cwd_local);
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
            script_buffer: String::new(),
            script_list_height: 160.0,
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

impl TypesetterUi {
    /// Синхронизирует буфер единого текстового редактора из массива строк реплик.
    pub fn sync_script_buffer_from_state(&mut self) {
        if !self.state.lines.is_empty() {
            self.script_buffer = self.state.lines.iter().map(|l| l.raw.as_str()).collect::<Vec<_>>().join("\n");
        }
    }

    /// Синхронизирует массив строк реплик и метаданных при редактировании единого текстового буфера.
    pub fn sync_state_from_script_buffer(&mut self) {
        let lines: Vec<&str> = self.script_buffer.split('\n').collect();
        let mut new_lines = Vec::with_capacity(lines.len());
        let mut page = 1;
        for (i, line_str) in lines.iter().enumerate() {
            let trimmed = line_str.trim();
            if let Some(p) = photocraft_engine::typesetter::detect_page_header(trimmed) {
                page = p;
            }
            let cleaned = photocraft_engine::typesetter::clean_script_line_with_options(trimmed, &self.state.options);
            let was_applied = self.state.lines.get(i).map(|l| l.is_applied).unwrap_or(false);
            new_lines.push(ScriptLine {
                raw: line_str.to_string(),
                text: if cleaned.is_empty() { line_str.to_string() } else { cleaned },
                is_applied: was_applied,
                page,
            });
        }
        self.state.lines = new_lines;
        if self.state.current_line_idx >= self.state.lines.len() {
            self.state.current_line_idx = self.state.lines.len().saturating_sub(1);
        }
        self.auto_save();
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Вставить сценарий напрямую из буфера обмена.
    pub fn paste_script_from_clipboard(&mut self) {
        let text_opt = arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok());
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

    /// Автоматическое сохранение состояния и пресетов в файл настроек.
    pub fn auto_save(&self) {
        let persisted = TypesetterPersistedState {
            dock_mode: self.dock_mode,
            state: self.state.clone(),
            selected_folder: self.selected_folder.clone(),
            custom_storage_dir: self.custom_storage_dir.clone(),
            favorite_fonts: self.favorite_fonts.clone(),
            is_open: self.is_open,
            script_list_height: self.script_list_height,
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

    /// Автоматическая загрузка сохраненных пресетов и состояния.
    pub fn auto_load(&mut self) {
        if let Some(path) = typesetter_storage_path() {
            if path.exists() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(data) = serde_json::from_str::<TypesetterPersistedState>(&content) {
                        self.dock_mode = data.dock_mode;
                        self.state = data.state;
                        self.selected_folder = data.selected_folder;
                        if !data.favorite_fonts.is_empty() {
                            self.favorite_fonts = data.favorite_fonts;
                        }
                        self.custom_storage_dir = data.custom_storage_dir;
                        self.is_open = data.is_open;
                        self.script_list_height = data.script_list_height;
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
                        if !data.favorite_fonts.is_empty() {
                            self.favorite_fonts = data.favorite_fonts;
                        }
                        self.is_open = data.is_open;
                        if !self.state.presets.is_empty() {
                            return;
                        }
                    }
                }
            }
        }

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
        self.rendered_this_frame = true;
        let mut action = None;
        egui::Frame::NONE
            .inner_margin(egui::Margin { left: 6, right: 6, top: 4, bottom: 4 })
            .show(ui, |ui| {
                self.render_header_toolbar(ui);
                self.render_body(ui, &mut action);
            });
        action
    }

    /// Основная точка отрисовки UI (поддержка свободного окна и плавающего режима).
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<TypesetterAction> {
        if !self.is_open {
            self.rendered_this_frame = false;
            return None;
        }

        let mut action = None;
        let ctx = ui.ctx().clone();
        self.rendered_this_frame = false;

        // Если открыто отдельное окно полного текста — рендерим его параллельно
        if self.show_full_text_window {
            let mut show_win = self.show_full_text_window;
            Window::new(tl!("Manga Script (Full Text)"))
                .open(&mut show_win)
                .default_width(450.0)
                .default_height(550.0)
                .min_width(320.0)
                .min_height(300.0)
                .resizable(true)
                .show(&ctx, |ui| {
                    self.render_full_text_content(ui, &mut action);
                });
            self.show_full_text_window = show_win;
        }

        // Модальное окно настроек конкретного шрифта / стиля
        if let Some(edit_idx) = self.editing_preset_idx {
            let mut close_modal = false;
            let title = if let Some(preset) = self.state.presets.get(edit_idx) {
                format!("{}: {}", tl!("Font Settings"), preset.name)
            } else {
                tl!("Font Settings").to_string()
            };

            Window::new(title)
                .collapsible(false)
                .resizable(true)
                .default_width(360.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    self.render_font_settings_modal(ui, edit_idx, &mut close_modal);
                });

            if close_modal {
                self.editing_preset_idx = None;
                self.auto_save();
            }
        }

        // Модальное окно ввода и редактирования текста сценария
        if self.show_script_editor_modal {
            let mut close_modal = false;
            Window::new(tl!("Script Editor"))
                .collapsible(false)
                .resizable(true)
                .default_width(520.0)
                .default_height(420.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    self.render_script_editor_modal(ui, &mut close_modal);
                });

            if close_modal {
                self.show_script_editor_modal = false;
            }
        }

        action
    }

    /// Верхняя панель вкладок и меню опций/импорта/экспорта в едином стиле Photoshop.
    fn render_header_toolbar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());

        ui.horizontal(|ui| {
            for (tab_val, label_key) in [
                (TypesetterTab::Queue, "Typesetter"),
                (TypesetterTab::FullText, "Script"),
                (TypesetterTab::Settings, "Options"),
            ] {
                let label = tl!(label_key);
                let is_sel = self.current_tab == tab_val;
                if widgets::pill_tab(ui, label, is_sel).clicked() {
                    self.current_tab = tab_val;
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

    /// Вертикальная раскладка TypeR (для боковых доков и плавающего окна).
    fn render_vertical_typer_workspace(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let t = Tokens::get(ui.ctx());
        let total_lines = self.state.lines.len();
        let cur_idx = self.state.current_line_idx;

        // --- 1. ВЕРХНИЙ БЛОК: СТРЕЛКИ ▲/▼, СТАТУС, ВСТАВКА (➔), ТЕКУЩАЯ СТРОКА ---
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
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
            });

            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;

                let line_num_str = if total_lines == 0 { 0 } else { cur_idx + 1 };
                let style_name = self.state.active_preset().map(|p| p.name.clone()).unwrap_or_else(|| tl!("Default").into());
                let font_size = self.state.active_preset().map(|p| p.font_size.round() as i32).unwrap_or(12);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}: {}, {}: {}, {}: {}pt",
                            tl!("line"), line_num_str, tl!("style"), style_name, tl!("size"), font_size
                        ))
                        .color(t.text_dim)
                        .small(),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let insert_btn = egui::Button::new(
                            RichText::new("▶")
                                .color(t.accent_text)
                                .strong(),
                        )
                        .fill(t.accent)
                        .min_size(vec2(26.0, 18.0));

                        if ui.add(insert_btn).on_hover_text(tl!("Insert current line into selection/bubble (Enter)")).clicked() {
                            if let Some(line) = self.state.active_line() {
                                if let Some(preset) = self.state.active_preset().cloned() {
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
                });

                let current_text = self.state.active_line().map(|l| l.text.as_str()).unwrap_or("");
                Frame::NONE
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(170, 175, 185)))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        let display = if current_text.is_empty() {
                            if total_lines == 0 {
                                tl!("(no script loaded — see 'Script' tab)")
                            } else {
                                tl!("(end of script)")
                            }
                        } else {
                            current_text
                        };

                        let text_color = if current_text.is_empty() {
                            Color32::from_rgb(140, 145, 155)
                        } else {
                            Color32::from_rgb(20, 25, 35)
                        };

                        ui.add(
                            egui::Label::new(
                                RichText::new(display)
                                    .color(text_color)
                                    .size(12.0)
                                    .strong(),
                            )
                            .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                    });
            });
        });

        widgets::hairline(ui);
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            let is_narrow = ui.available_width() < 300.0;
            ui.label(RichText::new(format!("{}:", tl!("Script"))).color(t.text_dim).small().strong());

            let paste_text = if is_narrow { "📋".to_string() } else { format!("📋 {}", tl!("Paste")) };
            if ui.small_button(paste_text).on_hover_text(tl!("Paste script text from clipboard (Ctrl+V)")).clicked() {
                self.paste_script_from_clipboard();
            }

            let edit_text = if is_narrow { "📝".to_string() } else { format!("📝 {}", tl!("Editor")) };
            if ui.small_button(edit_text).on_hover_text(tl!("Open script text editor")).clicked() {
                self.open_script_editor();
            }

            if ui.small_button("📥 .txt").on_hover_text(tl!("Load script file (.txt)")).clicked() {
                self.open_script_file_dialog();
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

        let num_cols = if ui.available_width() < 300.0 { 1 } else { 2 };
        self.render_style_presets_grid(ui, action, num_cols);
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
                        if let Some(preset) = self.state.active_preset().cloned() {
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

    /// Тулбар управления стилями (+ Стиль, + Папка, Поиск).
    fn render_preset_toolbar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());
        ui.horizontal(|ui| {
            if ui.small_button(tl!("+ Style")).on_hover_text("Создать новый стиль").clicked() {
                let new_preset = TextStylePreset {
                    name: tl!("New Preset").to_string(),
                    ..TextStylePreset::default()
                };
                self.state.presets.push(new_preset);
                self.editing_preset_idx = Some(self.state.presets.len() - 1);
                self.auto_save();
            }

            if ui.small_button(tl!("+ Folder")).on_hover_text("Создать новую папку стилей").clicked() {
                self.creating_new_folder = true;
                self.new_folder_name_buffer = tl!("New Folder").into();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.style_filter).hint_text("🔍").desired_width(70.0));
            });
        });

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

        let gutter_w = 46.0f32;
        let action_w = 24.0f32;
        let edit_id = egui::Id::new("typer_unified_script_text_edit");
        let active_line_idx = self.state.current_line_idx;
        let mut do_scroll_active = self.scroll_to_active_line;

        // Если буфер пуст и реплик нет — показываем плашку быстрого старта с кнопками
        if self.script_buffer.trim().is_empty() && self.state.lines.is_empty() {
            Frame::NONE
                .fill(Color32::WHITE)
                .stroke(Stroke::new(1.0, Color32::from_rgb(180, 185, 195)))
                .corner_radius(CornerRadius::same(2))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tl!("No script loaded")).color(Color32::from_rgb(120, 125, 135)).small());
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

        ScrollArea::vertical()
            .id_salt("typer_script_unified_scroll")
            .max_height(max_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail_w = ui.available_width().max(180.0);
                // Высота редактора должна следовать за ползунком-сплиттером: без desired_rows
                // TextEdit всегда занимает 4 строки, и изменение script_list_height не видно.
                let row_h = ui.text_style_height(&egui::TextStyle::Body).max(1.0);
                let desired_rows = (((max_height - 8.0) / row_h).floor() as usize).max(1);
                let edit = egui::TextEdit::multiline(&mut self.script_buffer)
                    .id(edit_id)
                    // Рабочий текст сценария читается пропорциональным шрифтом, а не моноширинным.
                    .font(egui::TextStyle::Body)
                    .text_color(Color32::from_rgb(20, 25, 35))
                    // Frame::NONE заменяет margin целиком, поэтому отступы задаём во фрейме:
                    // иначе текст начинается под левым гуттером и первые символы строки скрыты.
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: gutter_w as i8,
                        right: action_w as i8,
                        top: 4,
                        bottom: 4,
                    }))
                    .desired_rows(desired_rows)
                    .desired_width(avail_w);

                // Резервируем форму подложки до отрисовки TextEdit: иначе белая заливка,
                // добавленная после текста, перекрыла бы его в том же слое painter.
                let background_idx = ui.painter().add(egui::Shape::Noop);
                let output = edit.show(ui);
                let rect = output.response.rect;

                // 1. Отрисовка подложки и вертикальных границ колонок
                ui.painter().set(background_idx, egui::Shape::rect_filled(rect, CornerRadius::same(2), Color32::WHITE));

                // Левый гуттер (#e4e6eb)
                let left_gutter_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + gutter_w, rect.bottom()));
                ui.painter().rect_filled(left_gutter_rect, CornerRadius { nw: 2, sw: 2, ne: 0, se: 0 }, Color32::from_rgb(228, 230, 235));
                ui.painter().vline(rect.left() + gutter_w, rect.top()..=rect.bottom(), Stroke::new(1.0, Color32::from_rgb(205, 208, 215)));

                // Правый гуттер (#fafafc)
                let right_gutter_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - action_w, rect.top()), rect.max);
                ui.painter().rect_filled(right_gutter_rect, CornerRadius { ne: 2, se: 2, nw: 0, sw: 0 }, Color32::from_rgb(250, 250, 252));
                ui.painter().vline(rect.right() - action_w, rect.top()..=rect.bottom(), Stroke::new(1.0, Color32::from_rgb(225, 228, 235)));

                // Внешняя аккуратная рамка
                ui.painter().rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, Color32::from_rgb(180, 185, 195)), egui::StrokeKind::Inside);

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

                for (line_idx, start_y, end_y, first_row_y, first_row_h) in logical_lines_geom {
                    let screen_start_y = output.galley_pos.y + start_y;
                    let screen_end_y = output.galley_pos.y + end_y;
                    let screen_center_y = output.galley_pos.y + first_row_y + first_row_h * 0.5;

                    let is_active = line_idx == active_line_idx;
                    let is_applied = self.state.lines.get(line_idx).map(|l| l.is_applied).unwrap_or(false);

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
                        Color32::from_rgb(0, 100, 200)
                    } else if is_applied {
                        Color32::from_rgb(160, 165, 175)
                    } else {
                        Color32::from_rgb(100, 105, 115)
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
                        ui.painter().circle_stroke(target_center, 4.5, Stroke::new(1.3, Color32::from_rgb(0, 120, 215)));
                        ui.painter().circle_filled(target_center, 2.3, Color32::from_rgb(0, 120, 215));
                    } else {
                        let stroke_color = if is_target_hovered {
                            Color32::from_rgb(0, 120, 215)
                        } else if is_applied {
                            Color32::from_rgb(175, 180, 190)
                        } else {
                            Color32::from_rgb(130, 135, 145)
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

                    let btn_bg = if is_action_hovered {
                        Color32::from_rgb(0, 120, 215)
                    } else {
                        Color32::from_rgb(240, 243, 248)
                    };
                    let icon_color = if is_action_hovered {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(120, 125, 135)
                    };
                    let border_color = if is_action_hovered {
                        Color32::from_rgb(0, 100, 190)
                    } else {
                        Color32::from_rgb(205, 210, 220)
                    };

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

                    // В. Горизонтальная пунктирная линия под строкой
                    let dash_stroke = Stroke::new(1.0, Color32::from_rgb(181, 181, 181)); // #b5b5b5
                    draw_dashed_hline(ui.painter(), rect.left(), rect.right(), screen_end_y, dash_stroke);
                }

                if let Some(target_idx) = clicked_target_line {
                    self.state.set_current_line(target_idx);
                    self.scroll_to_active_line = true;
                }

                if let Some(act_idx) = clicked_action_line {
                    if let Some(line) = self.state.lines.get(act_idx) {
                        if let Some(preset) = self.state.active_preset().cloned() {
                            *action = Some(TypesetterAction {
                                text: line.text.clone(),
                                preset,
                            });
                            self.state.set_current_line(act_idx);
                            self.state.advance_line();
                            self.auto_save();
                        }
                    }
                }

                // 3. Отслеживание курсора: при клике или переходе по стрелкам обновляем активную строку
                if let Some(cr) = output.cursor_range {
                    let c_idx = cr.primary.index;
                    let char_count = self.script_buffer.chars().take(c_idx.into()).filter(|&c| c == '\n').count();
                    if char_count != self.state.current_line_idx && char_count < self.state.lines.len() {
                        self.state.set_current_line(char_count);
                    }
                }

                // 4. Синхронизация состояния при правках в текстовом поле
                if output.response.response.changed() {
                    self.sync_state_from_script_buffer();
                }
            });

        self.scroll_to_active_line = false;
    }

    /// Сетка карточек стилей шрифтов по папкам (с адаптивным числом колонок).
    fn render_style_presets_grid(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>, num_cols: usize) {
        let t = Tokens::get(ui.ctx());
        let active_preset_idx = self.state.selected_preset_idx;

        let mut style_to_select = None;
        let mut style_to_edit = None;
        let mut style_to_duplicate = None;
        let mut style_to_apply = None;
        let mut style_to_delete = None;

        let mut folder_names = self.state.folders();
        folder_names.push(tl!("No folder").to_string());

        let filter_lower = self.style_filter.trim().to_lowercase();

        ScrollArea::vertical()
            .id_salt("typer_folders_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
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

                    egui::CollapsingHeader::new(RichText::new(&header_title).strong())
                        .id_salt(format!("folder_collapse_{}", folder_name))
                        .default_open(true)
                        .show(ui, |ui| {
                            let mut draw_card = |col: &mut egui::Ui, p_idx: usize| {
                                let preset = &self.state.presets[p_idx];
                                let is_active = p_idx == active_preset_idx;

                                let (card_bg, card_border, text_color) = if is_active {
                                    (
                                        Color32::from_rgb(0, 120, 215),
                                        Color32::from_rgb(100, 180, 255),
                                        Color32::WHITE,
                                    )
                                } else {
                                    (
                                        t.field,
                                        t.field_border,
                                        t.text,
                                    )
                                };

                                Frame::NONE
                                    .fill(card_bg)
                                    .stroke(Stroke::new(1.0, card_border))
                                    .corner_radius(CornerRadius::same(t.radius_sm as u8))
                                    .inner_margin(Margin::symmetric(4, 3))
                                    .show(col, |ui| {
                                        ui.horizontal(|ui| {
                                            let sel_icon = if is_active { "●" } else { "○" };
                                            let ind_color = if is_active { Color32::WHITE } else { t.text_dim };
                                            if ui.add(egui::Label::new(RichText::new(sel_icon).color(ind_color).small()).sense(Sense::click())).clicked() {
                                                style_to_select = Some(p_idx);
                                            }

                                            let swatch_color = egui::Color32::from_rgba_unmultiplied(
                                                preset.text_color[0],
                                                preset.text_color[1],
                                                preset.text_color[2],
                                                preset.text_color[3],
                                            );
                                            let (swatch_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                            ui.painter().rect_filled(swatch_rect, 2.0, swatch_color);
                                            if preset.stroke_width > 0.0 {
                                                let stroke_c = egui::Color32::from_rgba_unmultiplied(
                                                    preset.stroke_color[0],
                                                    preset.stroke_color[1],
                                                    preset.stroke_color[2],
                                                    preset.stroke_color[3],
                                                );
                                                ui.painter().rect_stroke(swatch_rect, 2.0, egui::Stroke::new(1.0, stroke_c), egui::StrokeKind::Outside);
                                            }

                                            let mut open_edit = false;
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                let edit_btn = egui::Button::new(
                                                    RichText::new("✏")
                                                        .color(if is_active { Color32::WHITE } else { t.text_dim })
                                                        .small(),
                                                )
                                                .frame(false);
                                                if ui.add(edit_btn).on_hover_text("Настройки шрифта и стиля").clicked() {
                                                    open_edit = true;
                                                }

                                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                                    let mut title = RichText::new(&preset.name).color(text_color).small();
                                                    if preset.bold {
                                                        title = title.strong();
                                                    }
                                                    if preset.italic {
                                                        title = title.italics();
                                                    }

                                                    // Имя стиля показываем его собственным шрифтом,
                                                    // если семейство загружено и имя влезает в карточку.
                                                    let avail_w = ui.available_width().max(16.0);
                                                    let name_label = if let Some([w, h]) = crate::font_preview::name_size(ui.ctx(), &preset.font_family, &preset.name, STYLE_NAME_PT)
                                                        .filter(|[w, _]| *w <= avail_w)
                                                    {
                                                        let (rect, resp) = ui.allocate_exact_size(
                                                            vec2(w, h.max(ui.text_style_height(&egui::TextStyle::Body))),
                                                            Sense::click(),
                                                        );
                                                        crate::font_preview::paint_name(ui, &preset.font_family, &preset.name, STYLE_NAME_PT, rect, text_color);
                                                        resp
                                                    } else {
                                                        ui.add(egui::Label::new(title).truncate().sense(Sense::click()))
                                                    };

                                                    let sys_label = if preset.measurement_system.is_optical() { "Оптическая" } else { "Метрическая" };
                                                    let hex_c = format!("#{:02X}{:02X}{:02X}{:02X}", preset.text_color[0], preset.text_color[1], preset.text_color[2], preset.text_color[3]);
                                                    let tip = format!(
                                                        "{}\nШрифт: {}\nКегль: {}pt\nИнтерлиньяж: {}pt\nТрекинг: {}\nЦвет шрифта: {}\nСистема: {}\n\nЛКМ: выбрать стиль\n2xЛКМ: применить и вставить\nПКМ: меню действий",
                                                        preset.name, preset.font_family, preset.font_size, preset.leading, preset.tracking, hex_c, sys_label
                                                    );

                                                    let name_label = name_label.on_hover_text(tip);

                                                    if name_label.clicked() {
                                                        style_to_select = Some(p_idx);
                                                    }
                                                    if name_label.double_clicked() {
                                                        style_to_apply = Some(p_idx);
                                                    }

                                                    name_label.context_menu(|ui| {
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
                                                });
                                            });

                                            if open_edit {
                                                style_to_edit = Some(p_idx);
                                            }
                                        });
                                    });
                                col.add_space(2.0);
                            };

                            if num_cols <= 1 {
                                for &p_idx in &matching_indices {
                                    draw_card(ui, p_idx);
                                }
                            } else {
                                ui.columns(num_cols, |cols| {
                                    for (i, &p_idx) in matching_indices.iter().enumerate() {
                                        let col = &mut cols[i % num_cols];
                                        draw_card(col, p_idx);
                                    }
                                });
                            }
                        });
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
                    if let Some(preset) = self.state.active_preset().cloned() {
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
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    if let Ok(text) = cb.get_text() {
                        if !text.trim().is_empty() {
                            self.script_editor_buffer = text;
                        }
                    }
                }
            }

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
    use photocraft_doc::LayerContent;
    use photocraft_engine::typesetter::MeasurementSystem;
    use serde_json::json;

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
        };
        let json_str = serde_json::to_string(&persisted).unwrap();
        let loaded: TypesetterPersistedState = serde_json::from_str(&json_str).unwrap();
        assert_eq!(loaded.script_list_height, 240.0);
    }
}