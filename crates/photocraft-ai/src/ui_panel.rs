use crate::GeminiClient;
use std::sync::{Arc, Mutex};

pub struct AiCopilotPanel {
    pub prompt: String,
    pub history: Vec<String>,
    pub client: Option<Arc<GeminiClient>>,
    pub is_loading: bool,
}

impl Default for AiCopilotPanel {
    fn default() -> Self {
        // The key comes from the environment only; without it the client is left unusable and
        // the panel reports the request error, so no credential is baked into the binary.
        let api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
        
        Self {
            prompt: String::new(),
            history: vec!["AI Copilot готов к работе.".to_string()],
            client: Some(Arc::new(GeminiClient::new(api_key))),
            is_loading: false,
        }
    }
}

impl AiCopilotPanel {
    pub fn render_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Gemini Copilot");
        ui.separator();

        // История диалога
        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
            for line in &self.history {
                ui.label(line);
            }
        });

        ui.separator();

        // Поле ввода и кнопка
        ui.horizontal(|ui| {
            let response = ui.text_edit_singleline(&mut self.prompt);
            let send_clicked = ui.button("Отправить").clicked();

            if (send_clicked || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))))
                && !self.prompt.trim().is_empty()
                && !self.is_loading
            {
                let text = self.prompt.clone();
                self.prompt.clear();
                self.history.push(format!("Вы: {}", text));
                self.is_loading = true;

                if let Some(client) = self.client.clone() {
                    let ctx = ui.ctx().clone();
                    // В реальном цикле запускается фоновый task через tokio/thread
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .unwrap();
                        
                        let reply = rt.block_on(async {
                            client.ask(&text).await.unwrap_or_else(|e| format!("Ошибка: {e}"))
                        });
                        
                        // Запрос перерисовки
                        ctx.request_repaint();
                    });
                }
            }
        });

        if self.is_loading {
            ui.spinner();
        }
    }
}
