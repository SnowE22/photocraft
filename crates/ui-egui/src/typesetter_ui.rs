use egui::{
    Align, Color32, Context, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke,
    Vec2, Window,
};
use photocraft_engine::typesetter::{TextStylePreset, TypesetterState};

#[derive(PartialEq, Clone, Copy)]
pub enum TypesetterTab {
    Script,
    Styles,
    Settings,
}

pub struct TypesetterAction {
    pub text: String,
    pub preset: TextStylePreset,
}

pub struct TypesetterUi {
    pub is_open: bool,
    pub current_tab: TypesetterTab,
    pub state: TypesetterState,
    pub script_input_buffer: String,
    pub style_filter: String,
    pub line_filter: String,
    pub selected_folder: Option<String>,
    pub status_message: Option<String>,
    pub status_error: bool,
    pub filter_current_page_only: bool,
    pub editing_preset_idx: Option<usize>,
    pub duplicating_folder: Option<String>,
    pub duplicate_folder_name_buffer: String,
    pub new_preset_name_buffer: String,
}

impl Default for TypesetterUi {
    fn default() -> Self {
        Self {
            is_open: true,
            current_tab: TypesetterTab::Script,
            state: TypesetterState::default(),
            script_input_buffer: String::new(),
            style_filter: String::new(),
            line_filter: String::new(),
            selected_folder: None,
            status_message: None,
            status_error: false,
            filter_current_page_only: true,
            editing_preset_idx: None,
            duplicating_folder: None,
            duplicate_folder_name_buffer: String::new(),
            new_preset_name_buffer: String::new(),
        }
    }
}

impl TypesetterUi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn show(&mut self, ctx: &Context) -> Option<TypesetterAction> {
        if !self.is_open {
            return None;
        }

        let mut action = None;
        let mut open_flag = self.is_open;

        Window::new("TypeR Manga Typesetter")
            .open(&mut open_flag)
            .default_width(400.0)
            .default_height(640.0)
            .min_width(320.0)
            .min_height(400.0)
            .resizable(true)
            .show(ctx, |ui| {
                // Верхняя панель: Вкладки + Кнопки импорта/экспорта
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.current_tab, TypesetterTab::Script, "📝 Сценарий");
                    ui.selectable_value(&mut self.current_tab, TypesetterTab::Styles, "🎨 Стили");
                    ui.selectable_value(&mut self.current_tab, TypesetterTab::Settings, "⚙ Опции");

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
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

                // Статусная строка уведомлений
                if let Some(msg) = &self.status_message {
                    let color = if self.status_error {
                        Color32::from_rgb(255, 110, 110)
                    } else {
                        Color32::from_rgb(110, 210, 255)
                    };
                    ui.label(RichText::new(msg).color(color).small());
                }

                ui.separator();

                match self.current_tab {
                    TypesetterTab::Script => self.render_script_tab(ui, &mut action),
                    TypesetterTab::Styles => self.render_styles_tab(ui),
                    TypesetterTab::Settings => self.render_settings_tab(ui),
                }
            });

        self.is_open = open_flag;
        action
    }

    fn render_script_tab(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        // --- 1. Навигация по страницам (TypeR Automatic Page Detection) ---
        let pages = self.state.pages();
        let total_pages = pages.len();
        let current_page = self.state.current_page;

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
        });

        ui.separator();

        // --- 2. Выбор активного стиля ---
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

        // --- 3. TypeR Live Preview Block (Интерактивный блок предпросмотра) ---
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

                // Отрендеренный текст реплики
                let line_text = self.state.active_line().map(|l| l.text.as_str()).unwrap_or("(нет активной реплики)");
                let display_text = if self.state.active_preset().map_or(false, |p| p.all_caps) {
                    line_text.to_uppercase()
                } else {
                    line_text.to_string()
                };

                let active_style_info = if let Some(preset) = self.state.active_preset() {
                    let variant = format!(
                        "{}{}{}",
                        preset.font_style,
                        if preset.bold { " Bold" } else { "" },
                        if preset.italic { " Italic" } else { "" }
                    );
                    format!("{} | {} | {}pt / L:{}pt | Trk:{}", preset.font_family, variant.trim(), preset.font_size, preset.leading, preset.tracking)
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

                // Панель быстрого масштабирования (TypeR Line Spacing Sync)
                ui.horizontal(|ui| {
                    if ui.button("A-").on_hover_text("Уменьшить кегль (-1pt) с синхронизацией интерлиньяжа").clicked() {
                        self.state.adjust_active_preset_size(-1.0);
                    }
                    if ui.button("A+").on_hover_text("Увеличить кегль (+1pt) с синхронизацией интерлиньяжа").clicked() {
                        self.state.adjust_active_preset_size(1.0);
                    }
                    if ui.button("A--").on_hover_text("-3pt").clicked() {
                        self.state.adjust_active_preset_size(-3.0);
                    }
                    if ui.button("A++").on_hover_text("+3pt").clicked() {
                        self.state.adjust_active_preset_size(3.0);
                    }

                    if let Some(preset) = self.state.active_preset_mut() {
                        ui.checkbox(&mut preset.all_caps, "ALL CAPS");
                    }
                });
            });

        ui.add_space(4.0);

        // --- 4. Основная кнопка вставки реплики в выделение / баббл ---
        if let Some(line) = self.state.active_line() {
            let preview: String = line.text.chars().take(26).collect();
            let ellipsis = if line.text.chars().count() > 26 { "…" } else { "" };
            let btn_text = format!(
                "🎯 Вставить в баббл #{}: \"{}{}\"",
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
                }
            }
        } else {
            ui.add_enabled(
                false,
                egui::Button::new("🎯 Сценарий пуст или завершен").min_size(Vec2::new(ui.available_width(), 30.0)),
            );
        }

        // Кнопки навигации по репликам
        ui.horizontal(|ui| {
            if ui.button("◀ Назад").on_hover_text("Предыдущая реплика").clicked() {
                self.state.prev_line();
            }
            if ui.button("Вперёд ▶").on_hover_text("Следующая реплика").clicked() {
                self.state.advance_line();
            }
            if ui.button("✓ Готово").on_hover_text("Отметить текущую реплику как готовую").clicked() {
                if let Some(line) = self.state.lines.get_mut(self.state.current_line_idx) {
                    line.is_applied = true;
                }
                self.state.advance_line();
            }
            if ui.button("↺ Сброс").on_hover_text("Сбросить отметки готовности всех реплик").clicked() {
                for line in &mut self.state.lines {
                    line.is_applied = false;
                }
                self.state.current_line_idx = 0;
            }
        });

        ui.separator();

        // --- 5. Очередь реплик (с фильтрацией по странице) ---
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.filter_current_page_only, "Только текущая страница");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.line_filter).hint_text("🔍 Поиск..."));
            });
        });

        ScrollArea::vertical()
            .max_height(200.0)
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

        // --- 6. Редактор / Импорт сценария (TypeR Script Box) ---
        ui.collapsing("📥 Редактор текста / Загрузка скрипта", |ui| {
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
                            }
                            Err(err) => {
                                self.status_message = Some(format!("Ошибка чтения файла: {}", err));
                                self.status_error = true;
                            }
                        }
                    }
                }

                if ui.button("Обновить очередь реплик").clicked() {
                    self.state.load_script(&self.script_input_buffer);
                    self.status_message = Some(format!("Обновлено реплик: {}", self.state.lines.len()));
                    self.status_error = false;
                }
            });

            ui.add(
                egui::TextEdit::multiline(&mut self.script_input_buffer)
                    .desired_rows(6)
                    .desired_width(ui.available_width())
                    .hint_text("Вставьте сценарий (поддерживает разделение по 'Page 1', 'P02', 'Стр. 3', 'NAME: Text', '- Text')..."),
            );
        });
    }

    fn render_styles_tab(&mut self, ui: &mut egui::Ui) {
        // --- 1. Поиск стилей и фильтрация по папкам ---
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(egui::TextEdit::singleline(&mut self.style_filter).hint_text("Поиск стиля или шрифта..."));

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("➕ Новый").on_hover_text("Создать новый стиль на базе текущего").clicked() {
                    let base = self.state.active_preset().cloned().unwrap_or_default();
                    self.state.presets.push(TextStylePreset {
                        name: "Новый стиль".into(),
                        ..base
                    });
                    self.state.selected_preset_idx = self.state.presets.len() - 1;
                    self.editing_preset_idx = Some(self.state.selected_preset_idx);
                }
            });
        });

        // Список папок (Folder Tree / Pills)
        let folders = self.state.folders();
        if !folders.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(self.selected_folder.is_none(), "Все").clicked() {
                    self.selected_folder = None;
                }
                for f in &folders {
                    let is_sel = self.selected_folder.as_ref() == Some(f);
                    if ui.selectable_label(is_sel, f).clicked() {
                        self.selected_folder = Some(f.clone());
                    }
                }

                // Кнопка дублирования папки (TypeR Duplicate Style Folders)
                if let Some(active_f) = &self.selected_folder {
                    if ui.button("📋 Дублировать папку").clicked() {
                        self.duplicating_folder = Some(active_f.clone());
                        self.duplicate_folder_name_buffer = format!("{} (Копия)", active_f);
                    }
                }
            });
        }

        // Диалог ввода имени дублируемой папки
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
                        }
                        if ui.button("Отмена").clicked() {
                            self.duplicating_folder = None;
                        }
                    });
                });
        }

        ui.separator();

        // --- 2. Карточки стилей (Style Cards) ---
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let filter = self.style_filter.to_lowercase();
            let mut to_delete = None;
            let mut to_duplicate = None;
            let mut to_select = None;
            let active_preset_idx = self.state.selected_preset_idx;

            for (idx, preset) in self.state.presets.iter_mut().enumerate() {
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
                let card_bg = if is_active {
                    Color32::from_rgb(34, 40, 52)
                } else {
                    Color32::from_rgb(26, 28, 34)
                };
                let card_border = if is_active {
                    Color32::from_rgb(90, 150, 240)
                } else {
                    Color32::from_rgb(45, 50, 62)
                };

                Frame::NONE
                    .fill(card_bg)
                    .stroke(Stroke::new(1.0, card_border))
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let title = if is_active {
                                RichText::new(format!("⭐ {}", preset.name)).strong().color(Color32::from_rgb(255, 220, 90))
                            } else {
                                RichText::new(&preset.name).strong()
                            };

                            if ui.selectable_label(is_active, title).clicked() {
                                to_select = Some(idx);
                            }

                            if let Some(folder) = &preset.folder {
                                ui.label(RichText::new(format!("[{}]", folder)).color(Color32::from_rgb(140, 180, 220)).small());
                            }

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("🗑").on_hover_text("Удалить стиль").clicked() {
                                    to_delete = Some(idx);
                                }
                                if ui.small_button("📋").on_hover_text("Дублировать стиль").clicked() {
                                    to_duplicate = Some(idx);
                                }
                                let is_editing = self.editing_preset_idx == Some(idx);
                                if ui.small_button(if is_editing { "✓ Готово" } else { "✏ Изм." }).clicked() {
                                    self.editing_preset_idx = if is_editing { None } else { Some(idx) };
                                }
                            });
                        });

                        // Параметры типографики
                        let variant = format!(
                            "{}{}{}",
                            preset.font_style,
                            if preset.bold { " Bold" } else { "" },
                            if preset.italic { " Italic" } else { "" }
                        );
                        let stroke_desc = if preset.stroke_width > 0.0 {
                            format!("Обводка: {}px", preset.stroke_width)
                        } else {
                            "Без обводки".into()
                        };

                        ui.label(
                            RichText::new(format!(
                                "Шрифт: {} ({}) | {}pt / L:{}pt | Trk:{} | {}",
                                preset.font_family,
                                variant.trim(),
                                preset.font_size,
                                preset.leading,
                                preset.tracking,
                                stroke_desc
                            ))
                            .small()
                            .weak(),
                        );

                        // Редактор параметров стиля (если развернут)
                        if self.editing_preset_idx == Some(idx) {
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.label("Имя:");
                                ui.text_edit_singleline(&mut preset.name);
                                ui.label("Папка:");
                                let mut folder_str = preset.folder.clone().unwrap_or_default();
                                if ui.text_edit_singleline(&mut folder_str).changed() {
                                    preset.folder = if folder_str.trim().is_empty() {
                                        None
                                    } else {
                                        Some(folder_str.trim().to_string())
                                    };
                                }
                            });

                            ui.horizontal(|ui| {
                                ui.label("Гарнитура:");
                                ui.text_edit_singleline(&mut preset.font_family);
                                ui.label("PostScript:");
                                ui.text_edit_singleline(&mut preset.font_postscript);
                            });

                            ui.horizontal(|ui| {
                                ui.label("Кегль:");
                                ui.add(egui::DragValue::new(&mut preset.font_size).speed(0.5).range(4.0..=200.0));
                                ui.label("Интерлиньяж:");
                                ui.add(egui::DragValue::new(&mut preset.leading).speed(0.5).range(4.0..=300.0));
                                ui.label("Трекинг:");
                                ui.add(egui::DragValue::new(&mut preset.tracking).speed(1.0).range(-200..=1000));
                            });

                            ui.horizontal(|ui| {
                                ui.label("Обводка (px):");
                                ui.add(egui::DragValue::new(&mut preset.stroke_width).speed(0.2).range(0.0..=30.0));
                                ui.checkbox(&mut preset.bold, "Жирный");
                                ui.checkbox(&mut preset.italic, "Курсив");
                                ui.checkbox(&mut preset.all_caps, "ALL CAPS");
                            });
                        }
                    });
                ui.add_space(2.0);
            }

            if let Some(sel_idx) = to_select {
                self.state.selected_preset_idx = sel_idx;
            }
            if let Some(del_idx) = to_delete {
                self.state.delete_preset(del_idx);
            }
            if let Some(dup_idx) = to_duplicate {
                self.state.duplicate_preset(dup_idx);
            }
        });
    }

    fn render_settings_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Поведение тайпсеттера (TypeR)").strong());

        ui.checkbox(
            &mut self.state.options.auto_page_switch,
            "Автоматический переход на следующую страницу скрипта",
        );

        ui.checkbox(
            &mut self.state.options.line_spacing_sync,
            "Line Spacing Sync (синхронизация интерлиньяжа с размером шрифта)",
        )
        .on_hover_text("При увеличении или уменьшении кегля интерлиньяж (leading) масштабируется автоматически и пропорционально");

        ui.checkbox(
            &mut self.state.options.strip_character_names,
            "Очищать имена персонажей в начале строки ('ГЕРОЙ: Текст')",
        );

        ui.checkbox(
            &mut self.state.options.strip_dash_prefix,
            "Очищать тире в начале реплик ('- Текст', '— Текст')",
        );

        ui.separator();

        ui.label(RichText::new("Геометрия и центрирование текста").strong());

        ui.horizontal(|ui| {
            ui.label("Оптический сдвиг базовой линии:");
            ui.add(
                egui::Slider::new(&mut self.state.options.optical_baseline_factor, 0.0..=0.6)
                    .text("× font_size"),
            );
        });
        ui.label(
            RichText::new("Сдвиг базовой линии вниз (по умолчанию 0.35) обеспечивает точное визуальное центрирование текста по высоте внутри баббла.")
                .weak()
                .small(),
        );

        ui.separator();

        ui.label(RichText::new("Справка по горячим клавишам").strong());
        Frame::NONE
            .fill(Color32::from_rgb(26, 28, 34))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.label("🎯 Клик по кнопке 'Вставить в баббл' — Создать текстовый слой с обводкой");
                ui.label("A+ / A- — Быстрое изменение размера шрифта и интерлиньяжа");
                ui.label("◀ / ▶ — Навигация по очереди реплик сценария");
                ui.label("💾 Экспорт — Сохранение пресетов в формат TypeR JSON");
                ui.label("📁 Импорт — Загрузка существующих библиотек стилей TypeR");
            });
    }
}