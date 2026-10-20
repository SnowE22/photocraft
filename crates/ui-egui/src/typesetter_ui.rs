use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    Window, vec2,
};
use photocraft_engine::typesetter::{TextStylePreset, TypesetterState};
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

#[derive(Serialize, Deserialize)]
struct TypesetterPersistedState {
    pub dock_mode: TypesetterDockMode,
    pub state: TypesetterState,
    pub selected_folder: Option<String>,
}

pub fn typesetter_storage_path() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir).join("typesetter_state.json"));
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
        };
        ui.auto_load();
        ui
    }
}

impl TypesetterUi {
    pub fn new() -> Self {
        Self::default()
    }

    /// Автоматическое сохранение состояния и пресетов в файл настроек.
    pub fn auto_save(&self) {
        if let Some(path) = typesetter_storage_path() {
            let persisted = TypesetterPersistedState {
                dock_mode: self.dock_mode,
                state: self.state.clone(),
                selected_folder: self.selected_folder.clone(),
            };
            if let Ok(json_str) = serde_json::to_string_pretty(&persisted) {
                let _ = std::fs::write(path, json_str);
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
                        if !self.state.presets.is_empty() {
                            return;
                        }
                    } else if let Ok(count) = self.state.import_styles_json(&content) {
                        if count > 0 {
                            return;
                        }
                    }
                }
            }
        }

        // Если файла еще нет, пробуем подгрузить файл TypeR_Export.json, если он доступен на машине
        if let Ok(content) = std::fs::read_to_string(r"d:\Work\umamusume\TypeR_Export.json") {
            if let Ok(count) = self.state.import_styles_json(&content) {
                if count > 0 {
                    self.auto_save();
                }
            }
        }
    }

    /// Основная точка отрисовки UI (поддержка свободного окна и закрепления в любой части интерфейса).
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<TypesetterAction> {
        if !self.is_open {
            return None;
        }

        let mut action = None;
        let mut open_flag = self.is_open;
        let ctx = ui.ctx().clone();
        let t = Tokens::get(&ctx);

        let dock_frame = egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.card_border))
            .inner_margin(Margin::same(8));

        match self.dock_mode {
            TypesetterDockMode::Floating => {
                Window::new("TypeR Manga Typesetter")
                    .open(&mut open_flag)
                    .default_width(340.0)
                    .default_height(600.0)
                    .min_width(260.0)
                    .max_width(480.0)
                    .min_height(350.0)
                    .collapsible(false)
                    .resizable(true)
                    .frame(
                        egui::Frame::NONE
                            .fill(t.card)
                            .stroke(Stroke::new(1.0, t.card_border))
                            .corner_radius(CornerRadius::same(t.radius_sm as u8))
                            .inner_margin(Margin::same(8)),
                    )
                    .show(&ctx, |ui| {
                        self.render_header_toolbar(ui);
                        self.render_body(ui, &mut action);
                    });
            }
            TypesetterDockMode::DockRight => {
                egui::Panel::right("typesetter_dock_right")
                    .resizable(true)
                    .default_size(340.0)
                    .size_range(260.0..=480.0)
                    .frame(dock_frame)
                    .show(ui, |ui| {
                        self.render_dock_title_bar(ui, &mut open_flag);
                        self.render_header_toolbar(ui);
                        self.render_body(ui, &mut action);
                    });
            }
            TypesetterDockMode::DockLeft => {
                egui::Panel::left("typesetter_dock_left")
                    .resizable(true)
                    .default_size(340.0)
                    .size_range(260.0..=480.0)
                    .frame(dock_frame)
                    .show(ui, |ui| {
                        self.render_dock_title_bar(ui, &mut open_flag);
                        self.render_header_toolbar(ui);
                        self.render_body(ui, &mut action);
                    });
            }
            TypesetterDockMode::DockBottom => {
                egui::Panel::bottom("typesetter_dock_bottom")
                    .resizable(true)
                    .default_size(240.0)
                    .size_range(160.0..=340.0)
                    .frame(dock_frame)
                    .show(ui, |ui| {
                        self.render_dock_title_bar(ui, &mut open_flag);
                        self.render_header_toolbar(ui);
                        self.render_body(ui, &mut action);
                    });
            }
        }

        // Если открыто отдельное окно полного текста — рендерим его параллельно
        if self.show_full_text_window {
            let mut show_win = self.show_full_text_window;
            Window::new("📜 Сценарий манги (Полный текст)")
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
                format!("⚙ Настройки шрифта: {}", preset.name)
            } else {
                "⚙ Настройки шрифта".to_string()
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

        self.is_open = open_flag;
        action
    }

    /// Заголовок закрепленной панели с кнопками управления режимом докинга (как в Photoshop).
    fn render_dock_title_bar(&mut self, ui: &mut egui::Ui, open_flag: &mut bool) {
        let t = Tokens::get(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Тайпсеттер").color(t.text).strong().size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("×").on_hover_text("Закрыть панель").clicked() {
                    *open_flag = false;
                }
                if ui.small_button("Плав.").on_hover_text("Открепить в плавающее окно").clicked() {
                    self.dock_mode = TypesetterDockMode::Floating;
                    self.auto_save();
                }
            });
        });
        widgets::hairline(ui);
        ui.add_space(2.0);
    }

    /// Верхняя панель вкладок и меню опций/импорта/экспорта в едином стиле Photoshop.
    fn render_header_toolbar(&mut self, ui: &mut egui::Ui) {
        let t = Tokens::get(ui.ctx());

        ui.horizontal(|ui| {
            for (tab_val, label) in [
                (TypesetterTab::Queue, "Тайпсеттер"),
                (TypesetterTab::FullText, "Сценарий"),
                (TypesetterTab::Settings, "Опции"),
            ] {
                let is_sel = self.current_tab == tab_val;
                if widgets::pill_tab(ui, label, is_sel).clicked() {
                    self.current_tab = tab_val;
                }
            }

            // Меню панели (троеточие как во всех панелях Photoshop)
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("···", |ui| {
                    widgets::style_spectrum_popup_menu(ui);

                    ui.label(RichText::new("Стили TypeR").size(11.0).color(t.text_dim).strong());

                    if ui.button("Импорт TypeR JSON...").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TypeR JSON (*.json)", &["json"])
                            .set_title("Выберите файл стилей TypeR")
                            .pick_file()
                        {
                            match std::fs::read_to_string(&path) {
                                Ok(content) => match self.state.import_styles_json(&content) {
                                    Ok(count) => {
                                        self.status_message = Some(format!("Загружено стилей: {}", count));
                                        self.status_error = false;
                                        self.auto_save();
                                    }
                                    Err(err) => {
                                        self.status_message = Some(format!("Ошибка: {}", err));
                                        self.status_error = true;
                                    }
                                },
                                Err(err) => {
                                    self.status_message = Some(format!("Ошибка чтения: {}", err));
                                    self.status_error = true;
                                }
                            }
                        }
                    }

                    if ui.button("Экспорт TypeR JSON...").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TypeR JSON (*.json)", &["json"])
                            .set_title("Экспорт стилей TypeR")
                            .save_file()
                        {
                            match self.state.export_styles_json(self.selected_folder.as_deref()) {
                                Ok(json_content) => match std::fs::write(&path, json_content) {
                                    Ok(_) => {
                                        self.status_message = Some("Стили успешно экспортированы".into());
                                        self.status_error = false;
                                    }
                                    Err(err) => {
                                        self.status_message = Some(format!("Ошибка записи: {}", err));
                                        self.status_error = true;
                                    }
                                },
                                Err(err) => {
                                    self.status_message = Some(format!("Ошибка экспорта: {}", err));
                                    self.status_error = true;
                                }
                            }
                        }
                    }

                    ui.separator();
                    ui.label(RichText::new("Закрепление").size(11.0).color(t.text_dim).strong());

                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::Floating, "Плавающее окно").clicked() {
                        self.dock_mode = TypesetterDockMode::Floating;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockRight, "Закрепить справа").clicked() {
                        self.dock_mode = TypesetterDockMode::DockRight;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockLeft, "Закрепить слева").clicked() {
                        self.dock_mode = TypesetterDockMode::DockLeft;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockBottom, "Закрепить снизу").clicked() {
                        self.dock_mode = TypesetterDockMode::DockBottom;
                        self.auto_save();
                        ui.close();
                    }
                });
            });
        });

        // Статусное сообщение
        if let Some(msg) = &self.status_message {
            let color = if self.status_error {
                t.danger
            } else {
                t.accent
            };
            ui.label(RichText::new(msg).color(color).small());
        }

        widgets::hairline(ui);
        ui.add_space(4.0);
    }

    /// Тело активной вкладки тайпсеттера.
    fn render_body(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        match self.current_tab {
            TypesetterTab::Queue | TypesetterTab::Styles => self.render_unified_typer_workspace(ui, action),
            TypesetterTab::FullText => self.render_full_text_content(ui, action),
            TypesetterTab::Settings => self.render_settings_tab(ui),
        }
    }

    /// Единая рабочая область TypeR (как в плагине Photoshop):
    /// - Верх: управление текущей строкой (▲ / ▼), статус (строка, стиль, кегль), быстрая вставка (➔) и поле фразы.
    /// - Середина: полный список строк со скроллом, нумерацией, кнопкой прицела (◎) и быстрой вставкой (➔).
    /// - Низ: сворачиваемые папки (Манга, Манхва, Без папки) с 2-колоночной сеткой стилей шрифтов.
    fn render_unified_typer_workspace(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let t = Tokens::get(ui.ctx());
        let total_lines = self.state.lines.len();
        let cur_idx = self.state.current_line_idx;

        // --- 1. ВЕРХНИЙ БЛОК: СТРЕЛКИ ▲/▼, СТАТУС, ВСТАВКА (➔), ТЕКУЩАЯ СТРОКА ---
        ui.horizontal(|ui| {
            // Вертикальные кнопки стрелок ▲ и ▼
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let up_btn = egui::Button::new("▲").min_size(vec2(22.0, 16.0));
                if ui.add(up_btn).on_hover_text("Предыдущая строка (↑)").clicked() {
                    self.state.prev_line();
                }
                let down_btn = egui::Button::new("▼").min_size(vec2(22.0, 16.0));
                if ui.add(down_btn).on_hover_text("Следующая строка (↓)").clicked() {
                    self.state.advance_line();
                }
            });

            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;

                // Строка статуса: строка: X, стиль: Y, масштаб: Z pt
                let line_num_str = if total_lines == 0 { 0 } else { cur_idx + 1 };
                let style_name = self.state.active_preset().map(|p| p.name.clone()).unwrap_or_else(|| "Дефолт".into());
                let font_size = self.state.active_preset().map(|p| p.font_size.round() as i32).unwrap_or(12);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "строка: {}, стиль: {}, кегль: {}pt",
                            line_num_str, style_name, font_size
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

                        if ui.add(insert_btn).on_hover_text("Вставить текущую строку в выделение/баббл (Enter)").clicked() {
                            if let Some(line) = self.state.active_line() {
                                if let Some(preset) = self.state.active_preset().cloned() {
                                    *action = Some(TypesetterAction {
                                        text: line.text.clone(),
                                        preset,
                                    });
                                    self.state.advance_line();
                                    self.auto_save();
                                }
                            }
                        }
                    });
                });

                // Поле предпросмотра текущей фразы (белое контрастное окошко с построчным отображением как в TypeR)
                let current_text = self.state.active_line().map(|l| l.text.as_str()).unwrap_or("");
                Frame::NONE
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(170, 175, 185)))
                    .corner_radius(CornerRadius::same(3))
                    .inner_margin(Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        let avail_w = ui.available_width();
                        ui.set_width(avail_w);
                        let display = if current_text.is_empty() {
                            if total_lines == 0 {
                                "(сценарий не загружен — вкладка 'Сценарий')"
                            } else {
                                "(конец сценария)"
                            }
                        } else {
                            current_text
                        };

                        let text_color = if current_text.is_empty() {
                            Color32::from_rgb(140, 145, 155)
                        } else {
                            Color32::from_rgb(20, 25, 35)
                        };

                        ScrollArea::vertical()
                            .id_salt("current_phrase_box_scroll")
                            .max_height(60.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.set_width(avail_w);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(display)
                                            .color(text_color)
                                            .size(12.5)
                                            .strong(),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                    });
            });
        });

        widgets::hairline(ui);
        ui.add_space(2.0);

        // --- 2. СРЕДНИЙ БЛОК: СПИСОК РЕПЛИК С НУМЕРАЦИЕЙ, ПРИЦЕЛОМ (◎) И БЫСТРОЙ ВСТАВКОЙ (➔) ---
        let mut line_to_set = None;
        let mut line_to_insert = None;

        let script_list_height = 145.0;
        ScrollArea::vertical()
            .id_salt("typer_script_scroll")
            .max_height(script_list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if total_lines == 0 {
                    ui.vertical_centered(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new("Сценарий не загружен").color(t.text_faint).small());
                        if ui.button("📥 Загрузить файл сценария (.txt)...").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Текстовые файлы (*.txt)", &["txt"])
                                .set_title("Выберите файл сценария")
                                .pick_file()
                            {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    self.script_input_buffer = content.clone();
                                    self.state.load_script(&content);
                                    self.auto_save();
                                }
                            }
                        }
                    });
                } else {
                    for (idx, line) in self.state.lines.iter().enumerate() {
                        let is_active = idx == self.state.current_line_idx;

                        let row_frame = if is_active {
                            // Подсветка активной строки как в TypeR: светлый акцентный фон
                            Frame::NONE
                                .fill(Color32::from_rgb(160, 200, 240))
                                .corner_radius(CornerRadius::same(2))
                                .inner_margin(Margin::symmetric(4, 2))
                        } else {
                            Frame::NONE
                                .fill(Color32::TRANSPARENT)
                                .inner_margin(Margin::symmetric(4, 2))
                        };

                        let row_resp = row_frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let avail = ui.available_width();
                                let num_w = 20.0;
                                let target_w = 22.0;
                                let btn_w = 24.0;
                                let spacing = 4.0;
                                ui.spacing_mut().item_spacing.x = spacing;

                                let text_w = (avail - num_w - target_w - btn_w - spacing * 4.0).max(40.0);

                                // 1. Номер строки
                                let num_color = if is_active {
                                    Color32::from_rgb(20, 25, 35)
                                } else {
                                    t.text_dim
                                };
                                ui.add_sized(
                                    vec2(num_w, 18.0),
                                    egui::Label::new(RichText::new(format!("{:2}", idx + 1)).color(num_color).monospace().small()),
                                );

                                // 2. Иконка прицела: ● если активна, ◎ если неактивна
                                let target_icon = if is_active { "●" } else { "◎" };
                                if ui.small_button(target_icon).on_hover_text("Сделать эту строку текущей").clicked() {
                                    line_to_set = Some(idx);
                                }

                                // 3. Текст строки — выравнивание ВЛЕВО (Left-aligned)
                                let text_color = if is_active {
                                    Color32::from_rgb(15, 20, 30)
                                } else if line.is_applied {
                                    t.text_faint
                                } else {
                                    t.text
                                };
                                let text_label = egui::Label::new(
                                    RichText::new(&line.text)
                                        .color(text_color)
                                        .strong(),
                                ).truncate();
                                let text_resp = ui.add_sized(vec2(text_w, 18.0), text_label.sense(Sense::click()));
                                if text_resp.clicked() {
                                    line_to_set = Some(idx);
                                }

                                // 4. Кнопка быстрой вставки ▶ на правом краю
                                let btn_fill = if is_active { Color32::from_rgb(120, 180, 240) } else { t.accent };
                                let btn_text = if is_active { Color32::from_rgb(20, 25, 35) } else { t.accent_text };
                                let insert_btn = egui::Button::new(
                                    RichText::new("▶").color(btn_text).strong()
                                )
                                .fill(btn_fill)
                                .min_size(vec2(22.0, 16.0));

                                if ui.add(insert_btn).on_hover_text("Вставить эту строку в баббл").clicked() {
                                    line_to_insert = Some((idx, line.clone()));
                                }
                            });
                        });

                        if is_active {
                            row_resp.response.scroll_to_me(Some(Align::Center));
                        }

                        // Разделитель строк
                        widgets::hairline(ui);
                    }
                }
            });

        if let Some(idx) = line_to_set {
            self.state.set_current_line(idx);
        }

        if let Some((idx, line)) = line_to_insert {
            if let Some(preset) = self.state.active_preset().cloned() {
                *action = Some(TypesetterAction {
                    text: line.text.clone(),
                    preset,
                });
                self.state.set_current_line(idx + 1);
                self.auto_save();
            }
        }

        widgets::hairline(ui);
        ui.add_space(3.0);

        // --- 3. НИЖНИЙ БЛОК: СВОРАЧИВАЕМЫЕ ПАПКИ СО СТИЛЯМИ И 2-КОЛОНОЧНОЙ СЕТКОЙ ---
        let mut style_to_select = None;
        let mut style_to_edit = None;
        let mut style_to_duplicate = None;
        let mut style_to_apply = None;
        let mut style_to_delete = None;

        let active_preset_idx = self.state.selected_preset_idx;

        // Панель быстрого управления: создание стиля / папки
        ui.horizontal(|ui| {
            if ui.small_button("+ Стиль").on_hover_text("Создать новый стиль").clicked() {
                let new_preset = TextStylePreset {
                    name: "Новый стиль".to_string(),
                    ..TextStylePreset::default()
                };
                self.state.presets.push(new_preset);
                self.editing_preset_idx = Some(self.state.presets.len() - 1);
                self.auto_save();
            }

            if ui.small_button("+ Папка").on_hover_text("Создать новую папку стилей").clicked() {
                self.creating_new_folder = true;
                self.new_folder_name_buffer = "Новая папка".into();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.style_filter).hint_text("🔍").desired_width(70.0));
            });
        });

        // Создание новой папки
        if self.creating_new_folder {
            Frame::NONE
                .fill(t.field)
                .stroke(Stroke::new(1.0, t.field_border))
                .corner_radius(CornerRadius::same(t.radius_sm as u8))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Имя папки:").color(t.text).small());
                        ui.text_edit_singleline(&mut self.new_folder_name_buffer);
                        if ui.button("OK").clicked() {
                            let name = self.new_folder_name_buffer.trim().to_string();
                            if !name.is_empty() {
                                self.selected_folder = Some(name);
                            }
                            self.creating_new_folder = false;
                        }
                        if ui.button("Отмена").clicked() {
                            self.creating_new_folder = false;
                        }
                    });
                });
        }

        // Дублирование папки
        if let Some(src_folder) = self.duplicating_folder.clone() {
            Frame::NONE
                .fill(t.field)
                .stroke(Stroke::new(1.0, t.field_border))
                .corner_radius(CornerRadius::same(t.radius_sm as u8))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("Дублировать папку '{}':", src_folder)).color(t.text).strong().small());
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut self.duplicate_folder_name_buffer);
                        if ui.button("OK").clicked() {
                            let count = self.state.duplicate_folder(&src_folder, &self.duplicate_folder_name_buffer);
                            self.status_message = Some(format!("Папка продублирована ({} стилей)", count));
                            self.duplicating_folder = None;
                            self.auto_save();
                        }
                        if ui.button("Отмена").clicked() {
                            self.duplicating_folder = None;
                        }
                    });
                });
        }

        // Собираем список папок
        let mut folder_names = self.state.folders();
        folder_names.push("Без папки".to_string());

        ScrollArea::vertical()
            .id_salt("typer_folders_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for folder_name in &folder_names {
                    let is_unassigned = folder_name == "Без папки";

                    let matching_indices: Vec<usize> = self.state.presets
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            if is_unassigned {
                                p.folder.is_none() || p.folder.as_deref() == Some("")
                            } else {
                                p.folder.as_deref() == Some(folder_name.as_str())
                            }
                        })
                        .map(|(idx, _)| idx)
                        .collect();

                    if matching_indices.is_empty() && is_unassigned {
                        continue;
                    }

                    let header_title = format!("{} ({})", folder_name, matching_indices.len());

                    egui::CollapsingHeader::new(RichText::new(&header_title).strong())
                        .id_salt(format!("folder_collapse_{}", folder_name))
                        .default_open(true)
                        .show(ui, |ui| {
                            // 2-колоночная сетка стилей внутри папки
                            let avail_w = (ui.available_width() - 8.0).max(100.0);
                            let spacing = 4.0;
                            let col_w = ((avail_w - spacing) * 0.5).floor().max(80.0);

                            for pair in matching_indices.chunks(2) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = spacing;

                                    for &p_idx in pair {
                                        let preset = &self.state.presets[p_idx];
                                        let is_active = p_idx == active_preset_idx;

                                        // Если в папке всего 1 стиль — растягиваем его на всю ширину
                                        let is_single = matching_indices.len() == 1;
                                        let card_w = if is_single { avail_w } else { col_w };
                                        let card_inner_w = (card_w - 8.0).max(40.0);

                                        let (card_bg, card_border, text_color) = if is_active {
                                            (
                                                Color32::from_rgb(0, 120, 215), // TypeR active blue
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
                                            .show(ui, |ui| {
                                                ui.set_width(card_inner_w);
                                                ui.horizontal(|ui| {
                                                    ui.spacing_mut().item_spacing.x = 3.0;

                                                    // 1. Левый индикатор выбора (● / ○)
                                                    let sel_icon = if is_active { "●" } else { "○" };
                                                    let ind_color = if is_active { Color32::WHITE } else { t.text_dim };
                                                    if ui.add(egui::Label::new(RichText::new(sel_icon).color(ind_color).small()).sense(Sense::click())).clicked() {
                                                        style_to_select = Some(p_idx);
                                                    }

                                                    // 2. Название стиля — выравнивание влево, строго ограниченная ширина с обрезанием
                                                    let label_w = (card_inner_w - 36.0).max(20.0);

                                                    let mut title = RichText::new(&preset.name).color(text_color).small();
                                                    if preset.bold {
                                                        title = title.strong();
                                                    }
                                                    if preset.italic {
                                                        title = title.italics();
                                                    }

                                                    let tip = format!(
                                                        "{}\nШрифт: {}\nКегль: {}pt\nИнтерлиньяж: {}pt\nТрекинг: {}\n\nЛКМ: выбрать стиль\n2xЛКМ: применить и вставить\nПКМ: меню действий",
                                                        preset.name, preset.font_family, preset.font_size, preset.leading, preset.tracking
                                                    );

                                                    let name_resp = ui.add_sized(
                                                        vec2(label_w, 18.0),
                                                        egui::Label::new(title).truncate().sense(Sense::click()),
                                                    ).on_hover_text(tip);

                                                    if name_resp.clicked() {
                                                        style_to_select = Some(p_idx);
                                                    }
                                                    if name_resp.double_clicked() {
                                                        style_to_apply = Some(p_idx);
                                                    }

                                                    name_resp.context_menu(|ui| {
                                                        if ui.button("✏ Редактировать стиль...").clicked() {
                                                            style_to_edit = Some(p_idx);
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

                                                    // 3. Правая кнопка ✏ (настройки) — всегда на своем месте у правого края
                                                    let edit_btn = egui::Button::new(
                                                        RichText::new("✏")
                                                            .color(if is_active { Color32::WHITE } else { t.text_dim })
                                                            .small(),
                                                    )
                                                    .frame(false)
                                                    .min_size(vec2(16.0, 16.0));

                                                    if ui.add(edit_btn).on_hover_text("Настройки шрифта и стиля").clicked() {
                                                        style_to_edit = Some(p_idx);
                                                    }
                                                });
                                            });
                                    }
                                });
                                ui.add_space(2.0);
                            }
                        });
                }
            });

        // Применение действий со стилями
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

        // Панель управления текстом
        ui.horizontal(|ui| {
            if ui.button("◀").on_hover_text("Предыдущая страница").clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos > 0 {
                        self.state.current_page = pages[pos - 1];
                    }
                }
            }

            ui.label(RichText::new(format!("Стр. {}", current_page)).strong());

            if ui.button("▶").on_hover_text("Следующая страница").clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos + 1 < pages.len() {
                        self.state.current_page = pages[pos + 1];
                    }
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let detach_label = if self.show_full_text_window { "Вкладка" } else { "Плав." };
                if ui.small_button(detach_label).on_hover_text("Открыть текст в отдельном плавающем окне").clicked() {
                    self.show_full_text_window = !self.show_full_text_window;
                }

                if ui.small_button("🎯").on_hover_text("Перейти к текущей странице/реплике").clicked() {
                    if let Some(cur) = self.state.lines.get(self.state.current_line_idx) {
                        self.state.current_page = cur.page;
                    }
                }

                ui.checkbox(&mut self.filter_current_page_only, "Стр.")
                    .on_hover_text("Показывать только реплики текущей страницы");
            });
        });

        // Поиск и статистика
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.full_text_filter).hint_text("🔍 Поиск...").desired_width(100.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{}/{} (ост. {})", applied_count, total_lines, pending_count))
                        .weak()
                        .small(),
                );
            });
        });

        ui.separator();

        if total_lines == 0 {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Сценарий не загружен").weak());
                if ui.button("📥 Загрузить .txt файл сценария").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Текстовые файлы (*.txt)", &["txt"])
                        .set_title("Выберите файл сценария")
                        .pick_file()
                    {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            self.script_input_buffer = content.clone();
                            self.state.load_script(&content);
                            self.auto_save();
                        }
                    }
                }
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
                let mut last_rendered_page = 0;

                for (idx, line) in self.state.lines.iter().enumerate() {
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
                            ui.label(RichText::new(format!("─── Страница {} ───", line.page)).color(t.text_dim).strong().small());
                        });
                        ui.add_space(2.0);
                    }

                    let is_active = idx == self.state.current_line_idx;
                    let (card_bg, card_border) = if is_active {
                        (t.row_selected, t.accent)
                    } else if line.is_applied {
                        (t.card, t.card_border)
                    } else {
                        (t.card, t.card_border)
                    };

                    Frame::NONE
                        .fill(card_bg)
                        .stroke(Stroke::new(if is_active { 1.5 } else { 1.0 }, card_border))
                        .corner_radius(CornerRadius::same(t.radius_sm as u8))
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let avail = ui.available_width();
                                let tag_w = 26.0;
                                let btn_w = if is_active { 26.0 } else { 52.0 };
                                let spacing = 4.0;
                                ui.spacing_mut().item_spacing.x = spacing;
                                let text_w = (avail - tag_w - btn_w - spacing * 3.0).max(40.0);

                                if is_active {
                                    ui.add_sized(vec2(tag_w, 18.0), egui::Label::new(RichText::new("▶").color(t.accent).strong().small()));
                                } else if line.is_applied {
                                    ui.add_sized(vec2(tag_w, 18.0), egui::Label::new(RichText::new("✓").color(t.text_faint).small()));
                                } else {
                                    ui.add_sized(vec2(tag_w, 18.0), egui::Label::new(RichText::new(format!("{:2}", idx + 1)).color(t.text_dim).small()));
                                }

                                let text_color = if is_active {
                                    t.text
                                } else if line.is_applied {
                                    t.text_faint
                                } else {
                                    t.text
                                };
                                let label = egui::Label::new(RichText::new(&line.text).color(text_color).strong()).truncate();
                                let text_resp = ui.add_sized(vec2(text_w, 18.0), label.sense(Sense::click()));
                                if text_resp.clicked() {
                                    line_to_set = Some(idx);
                                }

                                if !is_active && ui.small_button("◎").on_hover_text("Сделать эту реплику текущей").clicked() {
                                    line_to_set = Some(idx);
                                }

                                let btn_fill = if is_active { Color32::from_rgb(120, 180, 240) } else { t.accent };
                                let btn_text = if is_active { Color32::from_rgb(20, 25, 35) } else { t.accent_text };
                                let insert_btn = egui::Button::new(
                                    RichText::new("▶").color(btn_text).strong()
                                )
                                .fill(btn_fill)
                                .min_size(vec2(22.0, 16.0));

                                if ui.add(insert_btn).on_hover_text("Вставить эту реплику в текущее выделение").clicked() {
                                    line_to_insert = Some((idx, line.clone()));
                                }
                            });
                        });
                    ui.add_space(2.0);
                }

                if let Some(idx) = line_to_set {
                    self.state.set_current_line(idx);
                }

                if let Some((idx, line)) = line_to_insert {
                    if let Some(preset) = self.state.active_preset().cloned() {
                        *action = Some(TypesetterAction {
                            text: line.text.clone(),
                            preset,
                        });
                        self.state.set_current_line(idx + 1);
                        self.auto_save();
                    }
                }
            });
    }



    /// Модальное окно редактирования настроек конкретного шрифта и стиля.
    fn render_font_settings_modal(&mut self, ui: &mut egui::Ui, idx: usize, close_modal: &mut bool) {
        let mut to_delete = false;
        let mut to_duplicate = false;
        let mut changed = false;

        if let Some(preset) = self.state.presets.get_mut(idx) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Имя стиля:").strong());
                if ui.text_edit_singleline(&mut preset.name).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Папка:");
                let mut folder_str = preset.folder.clone().unwrap_or_default();
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

            ui.label(RichText::new("Гарнитура и начертание").strong());
            ui.horizontal(|ui| {
                ui.label("Шрифт:");
                if ui.text_edit_singleline(&mut preset.font_family).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("PostScript:");
                if ui.text_edit_singleline(&mut preset.font_postscript).changed() {
                    changed = true;
                }
                ui.label("Начертание:");
                if ui.text_edit_singleline(&mut preset.font_style).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                if ui.checkbox(&mut preset.bold, "Жирный (Bold)").changed() {
                    if preset.bold && !preset.font_style.to_lowercase().contains("bold") {
                        preset.font_style = if preset.italic { "Bold Italic".into() } else { "Bold".into() };
                    } else if !preset.bold && preset.font_style.to_lowercase().contains("bold") {
                        preset.font_style = if preset.italic { "Italic".into() } else { "Regular".into() };
                    }
                    changed = true;
                }
                if ui.checkbox(&mut preset.italic, "Курсив (Italic)").changed() {
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
            });

            ui.separator();

            ui.label(RichText::new("Размеры типографики (Line Spacing Sync)").strong());
            ui.horizontal(|ui| {
                ui.label("Кегль (pt):");
                let mut size = preset.font_size;
                if ui.add(egui::DragValue::new(&mut size).speed(0.5).range(4.0..=200.0)).changed() {
                    preset.sync_leading_with_size(size);
                    changed = true;
                }

                if ui.small_button("A-").clicked() {
                    preset.sync_leading_with_size((preset.font_size - 1.0).max(4.0));
                    changed = true;
                }
                if ui.small_button("A+").clicked() {
                    preset.sync_leading_with_size(preset.font_size + 1.0);
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Интерлиньяж (Leading):");
                if ui.add(egui::DragValue::new(&mut preset.leading).speed(0.5).range(4.0..=300.0)).changed() {
                    changed = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label("Трекинг:");
                if ui.add(egui::DragValue::new(&mut preset.tracking).speed(1.0).range(-200..=1000)).changed() {
                    changed = true;
                }
                ui.label("Гориз. шкала (%):");
                if ui.add(egui::DragValue::new(&mut preset.horizontal_scale).speed(1.0).range(20.0..=300.0)).changed() {
                    changed = true;
                }
            });

            ui.separator();

            ui.label(RichText::new("Обводка текста").strong());
            ui.horizontal(|ui| {
                ui.label("Толщина (px):");
                if ui.add(egui::DragValue::new(&mut preset.stroke_width).speed(0.2).range(0.0..=30.0)).changed() {
                    changed = true;
                }
                let desc = if preset.stroke_width > 0.0 { "Включена" } else { "Без обводки" };
                ui.label(RichText::new(desc).small().weak());
            });

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("📋 Дублировать").on_hover_text("Создать копию этого стиля").clicked() {
                    to_duplicate = true;
                }
                if ui.button("🗑 Удалить").on_hover_text("Удалить этот стиль").clicked() {
                    to_delete = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("✓ Готово").clicked() {
                        *close_modal = true;
                    }
                });
            });
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

        ui.label(RichText::new("ЗАКРЕПЛЕНИЕ ПАНЕЛИ").size(11.0).color(t.text_dim).strong());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Положение:").color(t.text));
            let dock_str = match self.dock_mode {
                TypesetterDockMode::Floating => "Плавающее окно",
                TypesetterDockMode::DockRight => "Закрепить справа",
                TypesetterDockMode::DockLeft => "Закрепить слева",
                TypesetterDockMode::DockBottom => "Закрепить снизу",
            };
            egui::ComboBox::from_id_salt("settings_dock_mode")
                .selected_text(dock_str)
                .width(160.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::Floating, "Плавающее окно").clicked() {
                        self.dock_mode = TypesetterDockMode::Floating;
                        self.auto_save();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockRight, "Закрепить справа").clicked() {
                        self.dock_mode = TypesetterDockMode::DockRight;
                        self.auto_save();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockLeft, "Закрепить слева").clicked() {
                        self.dock_mode = TypesetterDockMode::DockLeft;
                        self.auto_save();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockBottom, "Закрепить снизу").clicked() {
                        self.dock_mode = TypesetterDockMode::DockBottom;
                        self.auto_save();
                    }
                });
        });

        widgets::hairline(ui);
        ui.add_space(4.0);

        ui.label(RichText::new("ПОВЕДЕНИЕ ТАЙПСЕТТЕРА").size(11.0).color(t.text_dim).strong());

        if ui.checkbox(
            &mut self.state.options.auto_page_switch,
            "Автопереход на след. страницу",
        ).on_hover_text("Автоматический переход на следующую страницу скрипта").changed() {
            self.auto_save();
        }

        if ui.checkbox(
            &mut self.state.options.line_spacing_sync,
            "Синхронизация интерлиньяжа",
        ).on_hover_text("Синхронизация интерлиньяжа (Line Spacing) с кеглем шрифта").changed() {
            self.auto_save();
        }

        if ui.checkbox(
            &mut self.state.options.strip_character_names,
            "Очищать имена персонажей",
        ).on_hover_text("Очищать имена персонажей в начале строки ('ГЕРОЙ: Текст')").changed() {
            self.auto_save();
        }

        if ui.checkbox(
            &mut self.state.options.strip_dash_prefix,
            "Очищать тире в начале реплик",
        ).on_hover_text("Очищать тире в начале реплик ('- Текст', '— Текст')").changed() {
            self.auto_save();
        }

        widgets::hairline(ui);
        ui.add_space(4.0);

        ui.label(RichText::new("ГЕОМЕТРИЯ И ЦЕНТРИРОВАНИЕ").size(11.0).color(t.text_dim).strong());

        ui.horizontal(|ui| {
            ui.label(RichText::new("Сдвиг базовой линии:").color(t.text));
            if ui.add(
                egui::Slider::new(&mut self.state.options.optical_baseline_factor, 0.0..=0.6)
                    .show_value(true)
                    .suffix(" ×"),
            ).changed() {
                self.auto_save();
            }
        });
        ui.label(
            RichText::new("Сдвиг базовой линии вниз обеспечивает точное визуальное центрирование текста внутри баббла.")
                .color(t.text_faint)
                .small(),
        );

        widgets::hairline(ui);
        ui.add_space(4.0);

        ui.label(RichText::new("ХРАНИЛИЩЕ ПРЕСЕТОВ").size(11.0).color(t.text_dim).strong());
        if let Some(path) = typesetter_storage_path() {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Файл: {}", path.display()))
                        .color(t.text_faint)
                        .small(),
                ).truncate(),
            ).on_hover_text(path.display().to_string());
        }

        ui.horizontal(|ui| {
            if ui.button("Сохранить сейчас").clicked() {
                self.auto_save();
                self.status_message = Some("Пресеты сохранены".into());
                self.status_error = false;
            }
            if ui.button("Загрузить из файла").clicked() {
                self.auto_load();
                self.status_message = Some("Пресеты загружены из файла".into());
                self.status_error = false;
            }
        });
    }
}