use egui::{Color32, Context, RichText, ScrollArea, Window};
use photocraft_engine::typesetter::{TextStylePreset, TypesetterState};

#[derive(PartialEq)]
pub enum TypesetterTab {
    Script,
    Styles,
}

pub struct TypesetterUi {
    pub is_open: bool,
    pub current_tab: TypesetterTab,
    pub state: TypesetterState,
    pub script_input_buffer: String,
    pub style_filter: String,
    pub selected_folder: Option<String>,
    pub status_message: Option<String>,
}

impl Default for TypesetterUi {
    fn default() -> Self {
        Self {
            is_open: true,
            current_tab: TypesetterTab::Script,
            state: TypesetterState::default(),
            script_input_buffer: String::new(),
            style_filter: String::new(),
            selected_folder: None,
            status_message: None,
        }
    }
}

pub struct TypesetterAction {
    pub text: String,
    pub preset: TextStylePreset,
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

        Window::new("TypeR Tool")
            .open(&mut open_flag)
            .default_width(380.0)
            .default_height(580.0)
            .resizable(true)
            .show(ctx, |ui| {
                // Вкладки как в оригинальном TypeR: Сценарий | Каталог стилей
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.current_tab, TypesetterTab::Script, "📝 Сценарий");
                    ui.selectable_value(&mut self.current_tab, TypesetterTab::Styles, "🎨 Каталог стилей");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("📁 Загрузить .json").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("TypeR JSON (*.json)", &["json"])
                                .set_title("Выберите файл стилей TypeR")
                                .pick_file()
                            {
                                match std::fs::read_to_string(&path) {
                                    Ok(content) => match self.state.import_styles_json(&content) {
                                        Ok(count) => {
                                            self.status_message = Some(format!("Загружено стилей: {}", count));
                                        }
                                        Err(err) => {
                                            self.status_message = Some(format!("Ошибка: {}", err));
                                        }
                                    },
                                    Err(err) => {
                                        self.status_message = Some(format!("Ошибка чтения: {}", err));
                                    }
                                }
                            }
                        }
                    });
                });

                if let Some(msg) = &self.status_message {
                    ui.label(RichText::new(msg).color(Color32::from_rgb(100, 200, 255)).small());
                }

                ui.separator();

                match self.current_tab {
                    TypesetterTab::Script => {
                        self.render_script_tab(ui, &mut action);
                    }
                    TypesetterTab::Styles => {
                        self.render_styles_tab(ui);
                    }
                }
            });

        self.is_open = open_flag;
        action
    }

    fn render_script_tab(&mut self, ui: &mut egui::Ui, action: &mut Option<TypesetterAction>) {
        // Быстрый выбор текущего стиля
        ui.horizontal(|ui| {
            ui.label("Стиль:");
            egui::ComboBox::from_id_salt("script_active_preset")
                .selected_text(
                    self.state
                        .active_preset()
                        .map(|p| p.name.as_str())
                        .unwrap_or("Не выбран"),
                )
                .show_ui(ui, |ui| {
                    for (idx, preset) in self.state.presets.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.state.selected_preset_idx,
                            idx,
                            &preset.name,
                        );
                    }
                });
        });

        if let Some(preset) = self.state.active_preset() {
            let variant = format!(
                "{}{}{}",
                preset.font_style,
                if preset.bold { " Bold" } else { "" },
                if preset.italic { " Italic" } else { "" }
            );
            ui.label(
                RichText::new(format!(
                    "{}: {} | {}pt | Обводка: {}px",
                    preset.font_family, variant.trim(), preset.font_size, preset.stroke_width
                ))
                .weak()
                .small(),
            );
        }

        ui.separator();

        // Управление очередью реплик
        ui.horizontal(|ui| {
            if ui.button("◀ Назад").clicked() && self.state.current_line_idx > 0 {
                self.state.current_line_idx -= 1;
            }
            if ui.button("Вперёд ▶").clicked() {
                self.state.advance_line();
            }
        });

        // Кнопка вставки в баббл / выделение
        if let Some(line) = self.state.active_line() {
            let preview: String = line.text.chars().take(24).collect();
            let ellipsis = if line.text.chars().count() > 24 { "…" } else { "" };
            let btn_text = format!(
                "🎯 Вставить в баббл #{}: \"{}{}\"",
                self.state.current_line_idx + 1,
                preview,
                ellipsis
            );

            if ui.button(RichText::new(btn_text).strong().color(Color32::from_rgb(255, 220, 100))).clicked() {
                if let Some(preset) = self.state.active_preset().cloned() {
                    *action = Some(TypesetterAction {
                        text: line.text.clone(),
                        preset,
                    });
                    self.state.advance_line();
                }
            }
        }

        ui.separator();

        // Большой редактор сценария и список очередей
        ui.collapsing("📥 Редактор текста / Импорт", |ui| {
            ui.label("Вставьте сценарий (по реплике на строку):");
            ui.text_edit_multiline(&mut self.script_input_buffer);
            if ui.button("Обновить очередь реплик").clicked() {
                self.state.load_script(&self.script_input_buffer);
            }
        });

        let total = self.state.lines.len();
        let current = if total == 0 { 0 } else { self.state.current_line_idx + 1 };
        ui.label(format!("Очередь реплик ({}/{}):", current, total));

        ScrollArea::vertical()
            .max_height(260.0)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (idx, line) in self.state.lines.iter().enumerate() {
                    let is_active = idx == self.state.current_line_idx;
                    let text_color = if is_active {
                        Color32::from_rgb(255, 215, 0)
                    } else if line.is_applied {
                        Color32::from_rgb(120, 120, 120)
                    } else {
                        Color32::WHITE
                    };

                    let label = RichText::new(format!("{}. {}", idx + 1, line.text)).color(text_color);
                    if ui.selectable_label(is_active, label).clicked() {
                        self.state.current_line_idx = idx;
                    }
                }
            });
    }

    fn render_styles_tab(&mut self, ui: &mut egui::Ui) {
        // Поиск стилей
        ui.horizontal(|ui| {
            ui.label("🔍 Поиск:");
            ui.text_edit_singleline(&mut self.style_filter);
        });

        // Сбор списка уникальных папок
        let mut folders: Vec<String> = self
            .state
            .presets
            .iter()
            .filter_map(|p| p.folder.clone())
            .collect();
        folders.sort();
        folders.dedup();

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
            });
        }

        ui.separator();

        // Карточки всех стилей
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let filter = self.style_filter.to_lowercase();
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

                let is_active = idx == self.state.selected_preset_idx;
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(is_active, RichText::new(&preset.name).strong()).clicked() {
                            self.state.selected_preset_idx = idx;
                        }
                        if let Some(folder) = &preset.folder {
                            ui.label(RichText::new(format!("[{}]", folder)).weak().small());
                        }
                    });

                    ui.label(
                        RichText::new(format!(
                            "Шрифт: {} ({}) | Кегль: {}pt | Трекинг: {} | Обводка: {}px",
                            preset.font_family,
                            if preset.italic { "Курсив" } else { &preset.font_style },
                            preset.font_size,
                            preset.tracking,
                            preset.stroke_width
                        ))
                        .small(),
                    );
                });
            }
        });
    }
}