use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke,
    Vec2, Window,
};
use photocraft_engine::typesetter::{TextStylePreset, TypesetterState};
use serde::{Deserialize, Serialize};

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

        match self.dock_mode {
            TypesetterDockMode::Floating => {
                Window::new("TypeR Manga Typesetter")
                    .open(&mut open_flag)
                    .default_width(380.0)
                    .default_height(620.0)
                    .min_width(320.0)
                    .max_width(450.0)
                    .min_height(350.0)
                    .collapsible(false)
                    .resizable(true)
                    .show(&ctx, |ui| {
                        self.render_header_toolbar(ui);
                        self.render_body(ui, &mut action);
                    });
            }
            TypesetterDockMode::DockRight => {
                egui::Panel::right("typesetter_dock_right")
                    .resizable(true)
                    .default_size(340.0)
                    .size_range(280.0..=450.0)
                    .frame(egui::Frame::NONE.fill(Color32::from_rgb(26, 28, 34)).inner_margin(Margin::same(6)))
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
                    .size_range(280.0..=450.0)
                    .frame(egui::Frame::NONE.fill(Color32::from_rgb(26, 28, 34)).inner_margin(Margin::same(6)))
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
                    .frame(egui::Frame::NONE.fill(Color32::from_rgb(26, 28, 34)).inner_margin(Margin::same(6)))
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
        ui.horizontal(|ui| {
            ui.label(RichText::new("TypeR Тайпсеттер").strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("✕").on_hover_text("Закрыть панель").clicked() {
                    *open_flag = false;
                }
                if ui.small_button("🪟").on_hover_text("Открепить в плавающее окно").clicked() {
                    self.dock_mode = TypesetterDockMode::Floating;
                    self.auto_save();
                }
            });
        });
        ui.separator();
    }

    /// Верхняя панель вкладок и переключения режимов закрепления/импорта/экспорта.
    fn render_header_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.current_tab, TypesetterTab::Queue, "⚡ Очередь");
            ui.selectable_value(&mut self.current_tab, TypesetterTab::FullText, "📜 Текст");
            ui.selectable_value(&mut self.current_tab, TypesetterTab::Styles, "🎨 Стили");
            ui.selectable_value(&mut self.current_tab, TypesetterTab::Settings, "⚙ Опции");

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Переключатель закрепления в интерфейсе
                ui.menu_button("📌 Док", |ui| {
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::Floating, "🪟 Плавающее окно").clicked() {
                        self.dock_mode = TypesetterDockMode::Floating;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockRight, "📌 Закрепить справа").clicked() {
                        self.dock_mode = TypesetterDockMode::DockRight;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockLeft, "📌 Закрепить слева").clicked() {
                        self.dock_mode = TypesetterDockMode::DockLeft;
                        self.auto_save();
                        ui.close();
                    }
                    if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockBottom, "📌 Закрепить снизу").clicked() {
                        self.dock_mode = TypesetterDockMode::DockBottom;
                        self.auto_save();
                        ui.close();
                    }
                });

                // Экспорт стилей TypeR JSON
                if ui.button("💾 Экспорт").on_hover_text("Экспортировать стили в TypeR JSON").clicked() {
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

                // Импорт стилей TypeR JSON
                if ui.button("📁 Импорт").on_hover_text("Загрузить стили из TypeR JSON").clicked() {
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
            });
        });

        // Статусное сообщение
        if let Some(msg) = &self.status_message {
            let color = if self.status_error {
                Color32::from_rgb(255, 110, 110)
            } else {
                Color32::from_rgb(110, 210, 255)
            };
            ui.label(RichText::new(msg).color(color).small());
        }

        ui.separator();
    }

    /// Тело активной вкладки тайпсеттера.
    fn render_body(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        match self.current_tab {
            TypesetterTab::Queue => self.render_queue_tab(ui, action),
            TypesetterTab::FullText => self.render_full_text_content(ui, action),
            TypesetterTab::Styles => self.render_styles_tab(ui),
            TypesetterTab::Settings => self.render_settings_tab(ui),
        }
    }

    /// Вкладка "⚡ Очередь": текущая реплика, интерактивное превью, быстрая вставка.
    fn render_queue_tab(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        let pages = self.state.pages();
        let total_pages = pages.len();
        let current_page = self.state.current_page;

        // 1. Навигация по страницам
        ui.horizontal(|ui| {
            if ui.button("◀").on_hover_text("Предыдущая страница").clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos > 0 {
                        self.state.current_page = pages[pos - 1];
                    }
                }
            }

            ui.label(RichText::new(format!("📄 Стр. {}", current_page)).strong());

            egui::ComboBox::from_id_salt("page_select_combo")
                .selected_text(format!("Стр. {}/{}", current_page, pages.last().copied().unwrap_or(1)))
                .width(90.0)
                .show_ui(ui, |ui| {
                    for &p in &pages {
                        ui.selectable_value(&mut self.state.current_page, p, format!("Страница {}", p));
                    }
                });

            if ui.button("▶").on_hover_text("Следующая страница").clicked() {
                if let Some(pos) = pages.iter().position(|&p| p == current_page) {
                    if pos + 1 < total_pages {
                        self.state.current_page = pages[pos + 1];
                    }
                }
            }

            let lines_on_page = self.state.lines_for_page(current_page).len();
            ui.label(RichText::new(format!("({} репл.)", lines_on_page)).weak().small());

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("📜 Весь текст").on_hover_text("Перейти к полному тексту сценария").clicked() {
                    self.current_tab = TypesetterTab::FullText;
                }
            });
        });

        ui.separator();

        // 2. Выбор активного стиля
        ui.horizontal(|ui| {
            ui.label("Стиль:");
            egui::ComboBox::from_id_salt("script_preset_select")
                .selected_text(
                    self.state
                        .active_preset()
                        .map(|p| p.name.as_str())
                        .unwrap_or("Не выбран"),
                )
                .show_ui(ui, |ui| {
                    for (idx, preset) in self.state.presets.iter().enumerate() {
                        let label = if let Some(folder) = &preset.folder {
                            format!("[{}] {}", folder, preset.name)
                        } else {
                            preset.name.clone()
                        };
                        ui.selectable_value(&mut self.state.selected_preset_idx, idx, label);
                    }
                });
        });

        // 3. Live Preview карточка
        let preview_bg = Color32::from_rgb(26, 28, 34);
        let preview_border = Color32::from_rgb(52, 58, 70);

        Frame::NONE
            .fill(preview_bg)
            .stroke(Stroke::new(1.0, preview_border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let total_lines = self.state.lines.len();
                    let cur_idx = if total_lines == 0 { 0 } else { self.state.current_line_idx + 1 };
                    ui.label(
                        RichText::new(format!("ПРЕДПРОСМОТР #{} / {}", cur_idx, total_lines))
                            .color(Color32::from_rgb(160, 175, 200))
                            .small()
                            .strong(),
                    );

                    if let Some(preset) = self.state.active_preset() {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let stroke_txt = if preset.stroke_width > 0.0 {
                                format!("Обводка: {}px", preset.stroke_width)
                            } else {
                                "Без обводки".into()
                            };
                            ui.label(RichText::new(stroke_txt).color(Color32::from_rgb(140, 200, 160)).small());
                        });
                    }
                });

                ui.add_space(2.0);

                let line_text = self.state.active_line().map(|l| l.text.as_str()).unwrap_or("(нет активной реплики)");
                let display_text = if self.state.active_preset().map_or(false, |p| p.all_caps) {
                    line_text.to_uppercase()
                } else {
                    line_text.to_string()
                };

                let active_style_info = if let Some(preset) = self.state.active_preset() {
                    let variant = if !preset.font_style.is_empty() && preset.font_style != "Regular" {
                        preset.font_style.clone()
                    } else if preset.bold && preset.italic {
                        "Bold Italic".to_string()
                    } else if preset.bold {
                        "Bold".to_string()
                    } else if preset.italic {
                        "Italic".to_string()
                    } else {
                        "Regular".to_string()
                    };
                    format!("{} | {} | {}pt / L:{}pt | Trk:{}", preset.font_family, variant, preset.font_size, preset.leading, preset.tracking)
                } else {
                    "Стиль не выбран".into()
                };

                ui.label(
                    RichText::new(&display_text)
                        .size(16.0)
                        .strong()
                        .color(Color32::from_rgb(255, 235, 150)),
                );

                ui.label(RichText::new(active_style_info).weak().small());

                ui.add_space(3.0);

                // Быстрое масштабирование A- / A+
                ui.horizontal(|ui| {
                    if ui.button("A-").on_hover_text("Уменьшить кегль (-1pt) с синхронизацией интерлиньяжа").clicked() {
                        self.state.adjust_active_preset_size(-1.0);
                        self.auto_save();
                    }
                    if ui.button("A+").on_hover_text("Увеличить кегль (+1pt) с синхронизацией интерлиньяжа").clicked() {
                        self.state.adjust_active_preset_size(1.0);
                        self.auto_save();
                    }
                    if ui.button("A--").on_hover_text("-3pt").clicked() {
                        self.state.adjust_active_preset_size(-3.0);
                        self.auto_save();
                    }
                    if ui.button("A++").on_hover_text("+3pt").clicked() {
                        self.state.adjust_active_preset_size(3.0);
                        self.auto_save();
                    }

                    if let Some(preset) = self.state.active_preset_mut() {
                        if ui.checkbox(&mut preset.all_caps, "ALL CAPS").changed() {
                            self.auto_save();
                        }
                    }
                });
            });

        ui.add_space(4.0);

        // 4. Главная кнопка вставки реплики
        if let Some(line) = self.state.active_line() {
            let preview: String = line.text.chars().take(24).collect();
            let ellipsis = if line.text.chars().count() > 24 { "…" } else { "" };
            let btn_text = format!(
                "🎯 Вставить в бабл #{}: \"{}{}\"",
                self.state.current_line_idx + 1,
                preview,
                ellipsis
            );

            let insert_btn = egui::Button::new(
                RichText::new(btn_text)
                    .size(14.0)
                    .strong()
                    .color(Color32::from_rgb(20, 25, 30)),
            )
            .fill(Color32::from_rgb(255, 210, 60))
            .min_size(Vec2::new(ui.available_width(), 32.0));

            if ui.add(insert_btn).clicked() {
                if let Some(preset) = self.state.active_preset().cloned() {
                    *action = Some(TypesetterAction {
                        text: line.text.clone(),
                        preset,
                    });
                    self.state.advance_line();
                    self.auto_save();
                }
            }
        } else {
            ui.add_enabled(
                false,
                egui::Button::new("🎯 Сценарий пуст или завершен").min_size(Vec2::new(ui.available_width(), 30.0)),
            );
        }

        // Навигация
        ui.horizontal(|ui| {
            if ui.button("◀ Назад").clicked() {
                self.state.prev_line();
            }
            if ui.button("Вперёд ▶").clicked() {
                self.state.advance_line();
            }
            if ui.button("✓ Готово").on_hover_text("Отметить текущую строку как готовую и перейти дальше").clicked() {
                if let Some(line) = self.state.lines.get_mut(self.state.current_line_idx) {
                    line.is_applied = true;
                }
                self.state.advance_line();
                self.auto_save();
            }
            if ui.button("↺ Сброс").on_hover_text("Сбросить отметки готовности всех реплик").clicked() {
                for line in &mut self.state.lines {
                    line.is_applied = false;
                }
                self.state.current_line_idx = 0;
                self.auto_save();
            }
        });

        ui.separator();

        // 5. Очередь реплик
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.filter_current_page_only, "Только текущая страница");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.line_filter).hint_text("🔍 Поиск..."));
            });
        });

        ScrollArea::vertical()
            .max_height(180.0)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let filter_str = self.line_filter.to_lowercase();
                let mut line_to_set = None;

                for (idx, line) in self.state.lines.iter().enumerate() {
                    if self.filter_current_page_only && line.page != self.state.current_page {
                        continue;
                    }
                    if !filter_str.is_empty() && !line.text.to_lowercase().contains(&filter_str) {
                        continue;
                    }

                    let is_active = idx == self.state.current_line_idx;
                    let (status_icon, text_color) = if is_active {
                        ("▶", Color32::from_rgb(255, 215, 60))
                    } else if line.is_applied {
                        ("✓", Color32::from_rgb(130, 135, 145))
                    } else {
                        ("○", Color32::WHITE)
                    };

                    let line_label = RichText::new(format!(
                        "{} P{:02} #{:02}: {}",
                        status_icon,
                        line.page,
                        idx + 1,
                        line.text
                    ))
                    .color(text_color);

                    if ui.selectable_label(is_active, line_label).clicked() {
                        line_to_set = Some(idx);
                    }
                }

                if let Some(idx) = line_to_set {
                    self.state.set_current_line(idx);
                }
            });

        ui.separator();

        // 6. Загрузка сценария
        ui.collapsing("📥 Загрузка / Редактор сценария", |ui| {
            ui.horizontal(|ui| {
                if ui.button("📄 Загрузить .txt файл").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Текстовые файлы (*.txt)", &["txt"])
                        .set_title("Выберите файл сценария")
                        .pick_file()
                    {
                        match std::fs::read_to_string(&path) {
                            Ok(content) => {
                                self.script_input_buffer = content.clone();
                                self.state.load_script(&content);
                                self.status_message = Some(format!("Загружен сценарий: {} реплик", self.state.lines.len()));
                                self.status_error = false;
                                self.auto_save();
                            }
                            Err(err) => {
                                self.status_message = Some(format!("Ошибка чтения файла: {}", err));
                                self.status_error = true;
                            }
                        }
                    }
                }

                if ui.button("Вставить из поля").clicked() {
                    if !self.script_input_buffer.trim().is_empty() {
                        self.state.load_script(&self.script_input_buffer);
                        self.status_message = Some(format!("Сценарий обновлен: {} реплик", self.state.lines.len()));
                        self.status_error = false;
                        self.auto_save();
                    }
                }
            });

            ui.add(
                egui::TextEdit::multiline(&mut self.script_input_buffer)
                    .hint_text("Вставьте текст сценария сюда (с разделением по страницам: 'Page 1', 'Стр. 2' и т.д.)...")
                    .desired_rows(4)
                    .desired_width(ui.available_width()),
            );
        });
    }

    /// Вкладка "📜 Весь текст": окно всего текста с листанием наперёд и полным просмотром.
    fn render_full_text_content(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
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

            ui.checkbox(&mut self.filter_current_page_only, "Только текущая стр.");

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let detach_label = if self.show_full_text_window { "🗗 Вкладка" } else { "↗ Отдельное окно" };
                if ui.button(detach_label).on_hover_text("Открыть текст в отдельном плавающем окне").clicked() {
                    self.show_full_text_window = !self.show_full_text_window;
                }

                if ui.button("🎯 К текущей").on_hover_text("Перейти к текущей реплике очереди").clicked() {
                    if let Some(cur) = self.state.lines.get(self.state.current_line_idx) {
                        self.state.current_page = cur.page;
                    }
                }
            });
        });

        // Поиск и статистика
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.full_text_filter).hint_text("🔍 Поиск по всему тексту..."));
            ui.label(
                RichText::new(format!("Всего: {} | Готово: {} | В очереди: {}", total_lines, applied_count, pending_count))
                    .weak()
                    .small(),
            );
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
                            ui.label(RichText::new(format!("────── Страница {} ──────", line.page)).color(Color32::from_rgb(130, 170, 220)).strong().small());
                        });
                        ui.add_space(2.0);
                    }

                    let is_active = idx == self.state.current_line_idx;
                    let (card_bg, card_border) = if is_active {
                        (Color32::from_rgb(38, 44, 58), Color32::from_rgb(255, 215, 60))
                    } else if line.is_applied {
                        (Color32::from_rgb(22, 24, 28), Color32::from_rgb(40, 44, 52))
                    } else {
                        (Color32::from_rgb(28, 30, 36), Color32::from_rgb(48, 54, 66))
                    };

                    Frame::NONE
                        .fill(card_bg)
                        .stroke(Stroke::new(if is_active { 1.5 } else { 1.0 }, card_border))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if is_active {
                                    ui.label(RichText::new("▶ ТЕКУЩАЯ").color(Color32::from_rgb(255, 215, 60)).strong().small());
                                } else if line.is_applied {
                                    ui.label(RichText::new("✓ Готово").color(Color32::from_rgb(120, 200, 140)).small());
                                } else {
                                    ui.label(RichText::new(format!("#{}", idx + 1)).weak().small());
                                }

                                let text_color = if is_active {
                                    Color32::from_rgb(255, 240, 180)
                                } else if line.is_applied {
                                    Color32::from_rgb(130, 135, 145)
                                } else {
                                    Color32::WHITE
                                };

                                ui.label(RichText::new(&line.text).color(text_color).strong());

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("🎯 Вставить").on_hover_text("Вставить эту реплику в текущее выделение").clicked() {
                                        line_to_insert = Some((idx, line.clone()));
                                    }
                                    if !is_active && ui.small_button("В очередь").on_hover_text("Сделать эту реплику текущей").clicked() {
                                        line_to_set = Some(idx);
                                    }
                                });
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

    /// Вкладка "🎨 Стили": стили представлены отдельными кнопками + модальные настройки шрифта.
    fn render_styles_tab(&mut self, ui: &mut egui::Ui) {
        // --- 1. Папки как вкладки ---
        let folders = self.state.folders();

        ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(self.selected_folder.is_none(), "📁 Все").clicked() {
                    self.selected_folder = None;
                }

                for folder in &folders {
                    let is_sel = self.selected_folder.as_deref() == Some(folder.as_str());
                    if ui.selectable_label(is_sel, format!("📁 {}", folder)).clicked() {
                        self.selected_folder = Some(folder.clone());
                    }
                }

                if ui.small_button("➕ Папка").on_hover_text("Создать новую папку стилей").clicked() {
                    self.creating_new_folder = true;
                    self.new_folder_name_buffer = "Новая папка".into();
                }
                if let Some(f) = &self.selected_folder {
                    if ui.small_button("📋 Копия").on_hover_text("Дублировать текущую папку").clicked() {
                        self.duplicating_folder = Some(f.clone());
                        self.duplicate_folder_name_buffer = format!("{} (Копия)", f);
                    }
                }
            });
        });

        // Создание новой папки
        if self.creating_new_folder {
            Frame::NONE
                .fill(Color32::from_rgb(35, 38, 48))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Имя папки:");
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
                .fill(Color32::from_rgb(35, 38, 48))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::same(6))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("Дублировать папку '{}':", src_folder)).strong());
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

        ui.separator();

        // Поиск и кнопка создания стиля
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.style_filter).hint_text("🔍 Поиск стиля..."));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("➕ Новый стиль").clicked() {
                    let new_preset = TextStylePreset {
                        name: "Новый стиль".to_string(),
                        folder: self.selected_folder.clone(),
                        ..TextStylePreset::default()
                    };
                    self.state.presets.push(new_preset);
                    self.editing_preset_idx = Some(self.state.presets.len() - 1);
                    self.auto_save();
                }
            });
        });

        ui.add_space(4.0);

        // --- 2. СТИЛИ ШРИФТОВ В ВИДЕ ОТДЕЛЬНЫХ КНОПОК ---
        let max_content_w = ui.available_width().max(280.0);
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.set_max_width(max_content_w);
            let filter = self.style_filter.to_lowercase();
            let mut to_select = None;
            let mut to_edit = None;
            let active_preset_idx = self.state.selected_preset_idx;

            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(max_content_w);
                for (idx, preset) in self.state.presets.iter().enumerate() {
                    if let Some(selected_f) = &self.selected_folder {
                        if preset.folder.as_ref() != Some(selected_f) {
                            continue;
                        }
                    }
                    if !filter.is_empty()
                        && !preset.name.to_lowercase().contains(&filter)
                        && !preset.font_family.to_lowercase().contains(&filter)
                    {
                        continue;
                    }

                    let is_active = idx == active_preset_idx;
                    let pill_bg = if is_active {
                        Color32::from_rgb(45, 55, 75)
                    } else {
                        Color32::from_rgb(30, 33, 40)
                    };
                    let pill_border = if is_active {
                        Color32::from_rgb(110, 180, 255)
                    } else {
                        Color32::from_rgb(55, 60, 75)
                    };

                    Frame::NONE
                        .fill(pill_bg)
                        .stroke(Stroke::new(if is_active { 1.5 } else { 1.0 }, pill_border))
                        .corner_radius(CornerRadius::same(5))
                        .inner_margin(Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                // Кнопка выбора стиля
                                let title_color = if is_active {
                                    Color32::from_rgb(255, 220, 90)
                                } else {
                                    Color32::WHITE
                                };

                                let btn_label = RichText::new(&preset.name).color(title_color).strong();
                                if ui.selectable_label(is_active, btn_label).clicked() {
                                    to_select = Some(idx);
                                }

                                // Бейджик кегля
                                ui.label(RichText::new(format!("{}pt", preset.font_size.round())).color(Color32::from_rgb(160, 175, 200)).small());

                                // Кнопка захода в настройки шрифта
                                if ui.small_button("⚙").on_hover_text("Настройки шрифта и стиля").clicked() {
                                    to_edit = Some(idx);
                                }
                            });
                        });

                    ui.add_space(2.0);
                }
            });

            if let Some(sel_idx) = to_select {
                self.state.selected_preset_idx = sel_idx;
                self.auto_save();
            }

            if let Some(edit_idx) = to_edit {
                self.editing_preset_idx = Some(edit_idx);
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

    /// Вкладка "⚙ Опции": общие настройки тайпсеттера, привязки и сохранение.
    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Закрепление в интерфейсе (Photoshop Docking)").strong());
        ui.horizontal(|ui| {
            ui.label("Режим панели:");
            if ui.selectable_label(self.dock_mode == TypesetterDockMode::Floating, "🪟 Плавающее окно").clicked() {
                self.dock_mode = TypesetterDockMode::Floating;
                self.auto_save();
            }
            if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockRight, "📌 Справа").clicked() {
                self.dock_mode = TypesetterDockMode::DockRight;
                self.auto_save();
            }
            if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockLeft, "📌 Слева").clicked() {
                self.dock_mode = TypesetterDockMode::DockLeft;
                self.auto_save();
            }
            if ui.selectable_label(self.dock_mode == TypesetterDockMode::DockBottom, "📌 Снизу").clicked() {
                self.dock_mode = TypesetterDockMode::DockBottom;
                self.auto_save();
            }
        });

        ui.separator();

        ui.label(RichText::new("Поведение тайпсеттера (TypeR)").strong());

        if ui.checkbox(&mut self.state.options.auto_page_switch, "Автоматический переход на следующую страницу скрипта").changed() {
            self.auto_save();
        }

        if ui.checkbox(
            &mut self.state.options.line_spacing_sync,
            "Line Spacing Sync (синхронизация интерлиньяжа с размером шрифта)",
        ).changed() {
            self.auto_save();
        }

        if ui.checkbox(&mut self.state.options.strip_character_names, "Очищать имена персонажей в начале строки ('ГЕРОЙ: Текст')").changed() {
            self.auto_save();
        }

        if ui.checkbox(&mut self.state.options.strip_dash_prefix, "Очищать тире в начале реплик ('- Текст', '— Текст')").changed() {
            self.auto_save();
        }

        ui.separator();

        ui.label(RichText::new("Геометрия и центрирование текста").strong());

        ui.horizontal(|ui| {
            ui.label("Оптический сдвиг базовой линии:");
            if ui.add(
                egui::Slider::new(&mut self.state.options.optical_baseline_factor, 0.0..=0.6)
                    .text("× font_size"),
            ).changed() {
                self.auto_save();
            }
        });
        ui.label(
            RichText::new("Сдвиг базовой линии вниз (по умолчанию 0.35) обеспечивает точное визуальное центрирование текста по высоте внутри баббла.")
                .weak()
                .small(),
        );

        ui.separator();

        ui.label(RichText::new("Хранилище пресетов и автосохранение").strong());
        if let Some(path) = typesetter_storage_path() {
            ui.label(RichText::new(format!("Файл сохранения: {}", path.display())).weak().small());
        }

        ui.horizontal(|ui| {
            if ui.button("💾 Сохранить пресеты сейчас").clicked() {
                self.auto_save();
                self.status_message = Some("Пресеты сохранены".into());
            }
            if ui.button("↺ Перезагрузить из файла").clicked() {
                self.auto_load();
                self.status_message = Some("Пресеты загружены из файла".into());
            }
        });
    }
}