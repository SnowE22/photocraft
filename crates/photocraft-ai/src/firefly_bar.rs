use std::sync::{Arc, Mutex};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use serde::Deserialize;

const CONFIG_PATH: &str = r"D:\Program\ArtCraft\PhotoCraft\ai_modules\config.json";
const WORKER_PATH: &str = r"D:\Program\ArtCraft\PhotoCraft\ai_modules\worker.py";

#[derive(Deserialize, Clone)]
pub struct ModeItem {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Clone)]
pub struct ModuleConfig {
    pub title: String,
    pub hint: String,
    pub modes: Vec<ModeItem>,
}

impl Default for ModuleConfig {
    fn default() -> Self {
        Self {
            title: "✨ Светлячок".to_string(),
            hint: "Опишите, что сгенерировать или изменить...".to_string(),
            modes: vec![
                ModeItem { id: "gemini_edit".into(), name: "Gemini: Команды".into() },
            ],
        }
    }
}

pub struct SharedProgress {
    pub is_running: bool,
    pub progress: f32,
    pub status: String,
}

pub struct FireflyContextBar {
    pub prompt: String,
    pub selected_mode: usize,
    pub config: ModuleConfig,
    pub state: Arc<Mutex<SharedProgress>>,
}

impl Default for FireflyContextBar {
    fn default() -> Self {
        let mut bar = Self {
            prompt: String::new(),
            selected_mode: 0,
            config: ModuleConfig::default(),
            state: Arc::new(Mutex::new(SharedProgress {
                is_running: false,
                progress: 0.0,
                status: "Готов к работе".to_string(),
            })),
        };
        bar.reload_config();
        bar
    }
}

impl FireflyContextBar {
    pub fn reload_config(&mut self) {
        if let Ok(data) = std::fs::read_to_string(CONFIG_PATH) {
            if let Ok(cfg) = serde_json::from_str::<ModuleConfig>(&data) {
                self.config = cfg;
                if self.selected_mode >= self.config.modes.len() {
                    self.selected_mode = 0;
                }
            }
        }
    }

    pub fn render_context_bar(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.input(|i| {
            i.viewport()
                .inner_rect
                .unwrap_or(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0)))
        });

        let bar_pos = egui::pos2(screen_rect.center().x - 280.0, screen_rect.max.y - 115.0);

        let (is_running, progress, status) = {
            let st = self.state.lock().unwrap();
            (st.is_running, st.progress, st.status.clone())
        };

        egui::Window::new("##svetlyachok_bar")
            .fixed_pos(bar_pos)
            .fixed_size(egui::vec2(560.0, 76.0))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(egui::Frame {
                inner_margin: egui::Margin::symmetric(14, 10),
                corner_radius: egui::CornerRadius::same(18),
                fill: egui::Color32::from_rgba_premultiplied(24, 24, 28, 245),
                stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(75, 75, 85)),
                ..Default::default()
            })
            .show(ctx, |ui| {
                // Верхний ряд: Название "Светлячок", выбор модуля, поле ввода и кнопки
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&self.config.title)
                            .strong()
                            .color(egui::Color32::from_rgb(255, 215, 100)),
                    );

                    // Выпадающий список режимов из внешнего JSON
                    let current_mode_name = self.config.modes
                        .get(self.selected_mode)
                        .map(|m| m.name.as_str())
                        .unwrap_or("Режим");

                    egui::ComboBox::from_id_salt("svetlyachok_mode")
                        .selected_text(current_mode_name)
                        .width(135.0)
                        .show_ui(ui, |ui| {
                            for (idx, mode) in self.config.modes.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_mode, idx, &mode.name);
                            }
                        });

                    let edit = ui.add_sized(
                        [210.0, 24.0],
                        egui::TextEdit::singleline(&mut self.prompt)
                            .hint_text(&self.config.hint),
                    );

                    let can_run = !self.prompt.trim().is_empty() && !is_running;
                    if ui.add_enabled(can_run, egui::Button::new("Запуск")).clicked()
                        || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_run)
                    {
                        self.start_worker(ctx);
                    }

                    // Кнопка горячей перезагрузки конфига без перекомпиляции
                    if ui.button("🔄").on_hover_text("Перезагрузить модули из JSON").clicked() {
                        self.reload_config();
                    }
                });

                ui.add_space(4.0);

                // Нижний ряд: Прогресс-бар и статус выполнения
                if is_running || progress > 0.0 {
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .text(status)
                            .animate(is_running),
                    );
                } else {
                    ui.label(egui::RichText::new(&status).small().color(egui::Color32::GRAY));
                }
            });
    }

    fn start_worker(&mut self, ctx: &egui::Context) {
        let prompt_text = self.prompt.clone();
        let mode_id = self.config.modes
            .get(self.selected_mode)
            .map(|m| m.id.clone())
            .unwrap_or_else(|| "gemini_edit".to_string());

        let state_clone = self.state.clone();
        let ctx_clone = ctx.clone();

        {
            let mut st = state_clone.lock().unwrap();
            st.is_running = true;
            st.progress = 0.05;
            st.status = "Запуск модуля...".to_string();
        }

        std::thread::spawn(move || {
            let child = Command::new("python")
                .arg(WORKER_PATH)
                .arg(&mode_id)
                .arg(&prompt_text)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            match child {
                Ok(mut proc) => {
                    if let Some(stdout) = proc.stdout.take() {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines().map_while(Result::ok) {
                            if let Some(rest) = line.strip_prefix("PROGRESS:") {
                                let parts: Vec<&str> = rest.splitn(2, ':').collect();
                                if parts.len() == 2 {
                                    let pct = parts[0].parse::<f32>().unwrap_or(50.0) / 100.0;
                                    let msg = parts[1].to_string();
                                    if let Ok(mut st) = state_clone.lock() {
                                        st.progress = pct.clamp(0.0, 1.0);
                                        st.status = msg;
                                    }
                                    ctx_clone.request_repaint();
                                }
                            }
                        }
                    }
                    let _ = proc.wait();
                }
                Err(e) => {
                    if let Ok(mut st) = state_clone.lock() {
                        st.status = format!("Ошибка запуска worker.py: {e}");
                    }
                }
            }

            if let Ok(mut st) = state_clone.lock() {
                st.is_running = false;
            }
            ctx_clone.request_repaint();
        });
    }
}
