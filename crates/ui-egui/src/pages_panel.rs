//! Модуль `pages_panel.rs` — панель управления страницами главы манги/комикса для PhotoCraft.
//!
//! Реализует паттерн InDesign / Balloon Translator:
//! - Каждая страница главы является физически отдельным документом в `app.session`.
//! - Панель выступает визуальным переключателем страниц, менеджером порядка листов
//!   и индикатором прогресса верстки текста (синхронизация с модулем `TypeR`).
//! - Полное соответствие строгим требованиям безопасности: `#![deny(clippy::unwrap_used)]`.

#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use egui::{
    self, Align, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Id, Layout, Margin, Pos2,
    Rect, RichText, Sense, Stroke, StrokeKind, TextureId, Ui, Vec2,
};

// ============================================================================
// 1. МОДЕЛИ ДАННЫХ И СОСТОЯНИЯ
// ============================================================================

/// Режим отображения списка страниц в док-панели.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PagesViewMode {
    /// Подробный список в одну колонку (крупная миниатюра слева, метаданные и прогресс справа).
    #[default]
    SingleColumnList,
    /// Компактная двухколоночная сетка вертикальных карточек (как в InDesign Pages).
    TwoColumnGrid,
}

/// Информация об одном открытом документе (странице) для отрисовки в кадре.
#[derive(Debug, Clone)]
pub struct PageItemInfo {
    /// Индекс документа во внутреннем массиве `session.documents()`.
    pub doc_index: usize,
    /// Отображаемое имя файла или заголовок документа.
    pub title: String,
    /// Полный путь к файлу на диске (если документ сохранен или загружен с диска).
    pub path: Option<PathBuf>,
    /// Автоматически распознанный номер страницы из имени файла (1-based).
    pub detected_page: usize,
    /// Является ли данная страница текущим активным документом на холсте.
    pub is_active: bool,
    /// Флаг наличия несохраненных изменений в документе.
    pub is_dirty: bool,
    /// Общее количество текстовых реплик (баблов) в `TypeR` для этой страницы.
    pub total_lines: usize,
    /// Количество уже сверстанных (вставленных на холст) реплик.
    pub applied_lines: usize,
    /// Опциональный идентификатор текстуры миниатюры страницы в `egui`.
    pub thumbnail_texture: Option<TextureId>,
    /// Разрешение исходного холста страницы (ширина, высота) для подсказки.
    pub dimensions: Option<(u32, u32)>,
}

impl PageItemInfo {
    /// Конструктор элемента страницы с автоматическим определением номера из заголовка/пути.
    #[must_use]
    pub fn new(doc_index: usize, title: String, path: Option<PathBuf>, is_active: bool) -> Self {
        let detected_page = path
            .as_deref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .and_then(parse_page_number_from_name)
            .or_else(|| parse_page_number_from_name(&title))
            .unwrap_or(doc_index.saturating_add(1));

        Self {
            doc_index,
            title,
            path,
            detected_page,
            is_active,
            is_dirty: false,
            total_lines: 0,
            applied_lines: 0,
            thumbnail_texture: None,
            dimensions: None,
        }
    }

    /// Вычисляет процент готовности перевода/верстки страницы (от `0.0` до `1.0`).
    #[must_use]
    pub fn completion_ratio(&self) -> f32 {
        if self.total_lines == 0 {
            0.0
        } else {
            (self.applied_lines as f32 / self.total_lines as f32).clamp(0.0, 1.0)
        }
    }

    /// Проверяет, завершена ли верстка всех реплик на странице.
    #[must_use]
    pub fn is_translation_complete(&self) -> bool {
        self.total_lines > 0 && self.applied_lines >= self.total_lines
    }
}

/// Срез данных о документах сессии, передаваемый в `PagesPanelState::render_embedded` на каждом кадре.
#[derive(Debug, Clone, Default)]
pub struct PagesViewInfo {
    /// Список всех открытых страниц главы.
    pub items: Vec<PageItemInfo>,
    /// Индекс текущего активного документа в `session.documents()`.
    pub active_doc_index: Option<usize>,
    /// Название текущей открытой папки главы (если открыта папка).
    pub chapter_folder_name: Option<String>,
    /// Разделен ли левый док на 2 окна
    pub is_left_dock_split: bool,
}

/// Действия пользователя в панели страниц, возвращаемые в основное приложение (Action Pattern).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PagesPanelAction {
    /// Переключить активный документ на холсте (передается `doc_index`).
    SelectPage(usize),
    /// Закрыть указанный документ (передается `doc_index`).
    ClosePage(usize),
    /// Открыть системный диалог выбора папки с главой манги.
    OpenChapterFolder,
    /// Открыть системный диалог добавления отдельных файлов страниц.
    AddPages,
    /// Переместить страницу в списке документов (изменение порядка следования).
    ReorderPages {
        /// Исходный индекс документа.
        from_idx: usize,
        /// Целевой индекс документа.
        to_idx: usize,
    },
    /// Переключить флаг автоматической синхронизации активной страницы с `TypeR`.
    ToggleAutoSync(bool),
    /// Показать файл страницы в системном проводнике (Finder / Explorer).
    RevealInExplorer(PathBuf),
    /// Автоматически отсортировать открытые документы в натуральном порядке имен файлов.
    SortPagesNaturally,
    /// Переключить дробление левого контейнера на два дока (Страницы + TypeR)
    ToggleSplitDock,
}

/// Фильтр состояния готовности страниц для быстрого отбора неготовых листов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageStatusFilter {
    /// Показывать все страницы.
    #[default]
    All,
    /// Только страницы с незавершенными репликами (`applied_lines < total_lines`).
    IncompleteOnly,
    /// Только полностью готовые страницы (`applied_lines >= total_lines && total_lines > 0`).
    CompletedOnly,
    /// Только страницы с несохраненными изменениями (`is_dirty == true`).
    DirtyOnly,
}

/// Состояние панели страниц главы (`Pages Panel`).
#[derive(Debug, Clone)]
pub struct PagesPanelState {
    /// Текущий режим отображения (список или сетка 2 колонки).
    pub view_mode: PagesViewMode,
    /// Строка быстрого поиска/фильтрации по имени файла или номеру страницы.
    pub search_query: String,
    /// Дополнительный фильтр по статусу верстки/сохранения.
    pub status_filter: PageStatusFilter,
    /// Автоматически синхронизировать выбор страницы с инспектором реплик `TypeR`.
    pub auto_sync_typer: bool,
    /// Индекс документа, который в данный момент перетаскивается мышью (Drag-and-Drop).
    pub dragged_doc_index: Option<usize>,
    /// Индекс документа, над которым находится курсор при перетаскивании (Drop Target).
    pub drop_target_doc_index: Option<usize>,
    /// Масштаб высоты карточек в режиме сетки (от `0.75` до `1.5`).
    pub thumbnail_scale: f32,
}

impl Default for PagesPanelState {
    fn default() -> Self {
        Self {
            view_mode: PagesViewMode::SingleColumnList,
            search_query: String::new(),
            status_filter: PageStatusFilter::All,
            auto_sync_typer: true,
            dragged_doc_index: None,
            drop_target_doc_index: None,
            thumbnail_scale: 1.0,
        }
    }
}

// ============================================================================
// 2. ВСПОМОГАТЕЛЬНЫЕ АЛГОРИТМЫ (ПАРСИНГ НОМЕРОВ И НАТУРАЛЬНАЯ СОРТИРОВКА)
// ============================================================================

/// Умное извлечение номера страницы из имени файла манги/комикса.
///
/// Поддерживает распространенные конвенции именования сканов и верстки:
/// - Чистые номера: `"01.png"` -> `Some(1)`, `"005.psd"` -> `Some(5)`
/// - С префиксом страницы: `"ch02_p14.jpg"` -> `Some(14)`, `"page_005.psd"` -> `Some(5)`, `"p3.png"` -> `Some(3)`
/// - Сканы с разделителями: `"raw-12.webp"` -> `Some(12)`, `"manga_vol01_ch05_022.png"` -> `Some(22)`
///
/// Алгоритм игнорирует номера томов (`vol`, `v`) и глав (`ch`, `c`, `chapter`),
/// отдавая приоритет явному маркеру страницы (`p`, `pg`, `page`, `стр`) либо последнему числовому блоку.
#[must_use]
pub fn parse_page_number_from_name(filename: &str) -> Option<usize> {
    // 1. Отделяем имя файла от пути и расширения
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename)
        .trim();

    if stem.is_empty() {
        return None;
    }

    let lower = stem.to_lowercase();

    // 2. Разбиваем строку на чередующиеся буквенные и цифровые сегменты
    let mut tokens: Vec<(bool, &str)> = Vec::new();
    let mut start = 0;
    let mut prev_is_digit: Option<bool> = None;

    for (idx, ch) in lower.char_indices() {
        let is_digit = ch.is_ascii_digit();
        if let Some(prev) = prev_is_digit {
            if prev != is_digit {
                if let Some(slice) = lower.get(start..idx) {
                    tokens.push((prev, slice));
                }
                start = idx;
            }
        }
        prev_is_digit = Some(is_digit);
    }
    if let (Some(prev), Some(slice)) = (prev_is_digit, lower.get(start..)) {
        tokens.push((prev, slice));
    }

    // 3. Ищем явный префикс страницы перед числом: "page", "pg", "p", "стр"
    for i in 0..tokens.len() {
        let Some(&(is_digit, num_str)) = tokens.get(i) else {
            continue;
        };
        if !is_digit {
            continue;
        }

        if i > 0 {
            if let Some(&(_, prefix_raw)) = tokens.get(i.saturating_sub(1)) {
                let prefix = prefix_raw.trim_matches(|c: char| !c.is_alphabetic());
                let is_page_marker = matches!(prefix, "p" | "pg" | "page" | "pages" | "стр")
                    || prefix.ends_with("_p")
                    || prefix.ends_with("-p")
                    || prefix.ends_with("page");

                if is_page_marker {
                    if let Ok(page_num) = num_str.parse::<usize>() {
                        return Some(page_num);
                    }
                }
            }
        }
    }

    // 4. Если явного префикса "p/page" нет, собираем все числа, которые НЕ являются томом/главой
    let mut candidate_numbers: Vec<usize> = Vec::new();
    for i in 0..tokens.len() {
        let Some(&(is_digit, num_str)) = tokens.get(i) else {
            continue;
        };
        if !is_digit {
            continue;
        }

        let is_chapter_or_volume = if i > 0 {
            tokens
                .get(i.saturating_sub(1))
                .map(|&(_, pref)| {
                    let clean = pref.trim_matches(|c: char| !c.is_alphabetic());
                    matches!(
                        clean,
                        "ch" | "c" | "chap" | "chapter" | "vol" | "v" | "volume" | "том" | "гл"
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };

        if !is_chapter_or_volume {
            if let Ok(num) = num_str.parse::<usize>() {
                candidate_numbers.push(num);
            }
        }
    }

    // Берем последнее число (в именах вида "01_14.png" или "title_05.png" номер страницы всегда в конце)
    if let Some(&last_num) = candidate_numbers.last() {
        return Some(last_num);
    }

    // 5. Fallback: если в имени было только одно число с префиксом (например "ch01.png"), берем последнее любое число
    tokens
        .iter()
        .rev()
        .find(|&&(is_digit, _)| is_digit)
        .and_then(|&(_, num_str)| num_str.parse::<usize>().ok())
}

/// Токен для натуральной сортировки строк (текстовый фрагмент или числовое значение).
#[derive(Debug, Clone, PartialEq, Eq)]
enum NaturalToken {
    Text(String),
    Number(u64, usize), // (числовое значение, количество ведущих нулей для стабильности)
}

impl PartialOrd for NaturalToken {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NaturalToken {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(n1, z1), Self::Number(n2, z2)) => {
                n1.cmp(n2).then_with(|| z1.cmp(z2))
            }
            (Self::Text(t1), Self::Text(t2)) => t1.cmp(t2),
            // Числа идут перед буквами при прочих равных
            (Self::Number(..), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(..)) => Ordering::Greater,
        }
    }
}

/// Разбивает строку на последовательность `NaturalToken` для человекочитаемой сортировки.
#[must_use]
fn tokenize_natural(input: &str) -> Vec<NaturalToken> {
    let mut result = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch.is_ascii_digit() {
            let mut num_str = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    num_str.push(d);
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            let leading_zeros = num_str.chars().take_while(|&c| c == '0').count();
            let val = num_str.parse::<u64>().unwrap_or(0);
            result.push(NaturalToken::Number(val, leading_zeros));
        } else {
            let mut text_str = String::new();
            while let Some(&c) = chars.peek() {
                if !c.is_ascii_digit() {
                    for lower_c in c.to_lowercase() {
                        text_str.push(lower_c);
                    }
                    let _ = chars.next();
                } else {
                    break;
                }
            }
            result.push(NaturalToken::Text(text_str));
        }
    }

    result
}

/// Сравнивает две строки по правилам натуральной сортировки (`"2.png" < "10.png"`).
#[must_use]
pub fn natural_cmp_str(a: &str, b: &str) -> Ordering {
    let tokens_a = tokenize_natural(a);
    let tokens_b = tokenize_natural(b);
    tokens_a.cmp(&tokens_b).then_with(|| a.cmp(b))
}

/// Натуральная сортировка среза путей `PathBuf` на месте (`in-place`).
///
/// Гарантирует правильный порядок страниц манги:
/// `"1.png"`, `"2.png"`, `"10.png"`, `"ch1_p2.png"`, `"ch1_p10.png"`.
pub fn natural_sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| {
        let name_a = a
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let name_b = b
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();

        // Сначала сравниваем имена файлов натурально, а при совпадении — полные пути
        let cmp = natural_cmp_str(name_a, name_b);
        if cmp == Ordering::Equal {
            a.cmp(b)
        } else {
            cmp
        }
    });
}

// ============================================================================
// 3. ОСНОВНАЯ РЕАЛИЗАЦИЯ ОТРИСОВКИ ПАНЕЛИ (EGUI 0.36)
// ============================================================================

impl PagesPanelState {
    /// Создает новый экземпляр состояния панели страниц с настройками по умолчанию.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Главный метод встраиваемой отрисовки панели страниц в боковом доке (Left/Right Dock).
    ///
    /// Возвращает `Some(PagesPanelAction)`, если пользователь совершил действие
    /// (выбор страницы, закрытие, перестановка, открытие главы и т.д.).
    #[must_use]
    pub fn render_embedded(
        &mut self,
        ui: &mut Ui,
        view_info: &PagesViewInfo,
    ) -> Option<PagesPanelAction> {
        let mut triggered_action: Option<PagesPanelAction> = None;

        // Внешний отступ и вертикальная компоновка всей панели
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);

        // 1. ВЕРХНИЙ ТУЛБАР (Открытие главы, добавление страниц, режимы вида, поиск)
        if let Some(action) = self.render_top_toolbar(ui, view_info) {
            triggered_action = Some(action);
        }

        ui.separator();

        // 2. СВОДНАЯ ПОЛОСА ПРОГРЕССА ГЛАВЫ (если есть открытые страницы)
        if !view_info.items.is_empty() {
            self.render_chapter_summary_bar(ui, view_info);
            ui.add_space(2.0);
        }

        // 3. ФИЛЬТРАЦИЯ СПИСКА СТРАНИЦ
        let filtered_items: Vec<&PageItemInfo> = view_info
            .items
            .iter()
            .filter(|item| self.matches_filter(item))
            .collect();

        // 4. ОСНОВНАЯ ОБЛАСТЬ ПРОКРУТКИ КАРТОЧЕК СТРАНИЦ
        if view_info.items.is_empty() {
            if let Some(action) = self.render_empty_state(ui) {
                triggered_action = Some(action);
            }
        } else if filtered_items.is_empty() {
            self.render_no_filter_matches(ui);
        } else {
            egui::ScrollArea::vertical()
                .id_salt("photocraft_pages_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |scroll_ui| {
                    scroll_ui.add_space(2.0);
                    let list_action = match self.view_mode {
                        PagesViewMode::SingleColumnList => {
                            self.render_single_column_list(scroll_ui, &filtered_items, view_info.items.len())
                        }
                        PagesViewMode::TwoColumnGrid => {
                            self.render_two_column_grid(scroll_ui, &filtered_items, view_info.items.len())
                        }
                    };

                    if list_action.is_some() {
                        triggered_action = list_action;
                    }
                    scroll_ui.add_space(12.0);
                });
        }

        // Сброс состояния Drag-and-Drop при отпускании кнопки мыши вне карточек
        if ui.input(|i| i.pointer.any_released()) {
            self.dragged_doc_index = None;
            self.drop_target_doc_index = None;
        }

        triggered_action
    }

    // ------------------------------------------------------------------------
    // ВЕРХНИЙ ТУЛБАР И ФИЛЬТРЫ
    // ------------------------------------------------------------------------

    fn render_top_toolbar(
        &mut self,
        ui: &mut Ui,
        view_info: &PagesViewInfo,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        // Строка 1: Главные кнопки управления главой
        ui.horizontal(|ui| {
            let avail_w = ui.available_width();
            let btn_width = ((avail_w - 6.0) * 0.5).max(80.0);

            let open_chapter_btn = egui::Button::new(
                RichText::new("📂 Открыть главу…").size(12.5).strong(),
            )
            .min_size(Vec2::new(btn_width, 26.0));

            if ui
                .add(open_chapter_btn)
                .on_hover_text("Выбрать папку с изображениями главы (автоматическая натуральная сортировка)")
                .clicked()
            {
                action = Some(PagesPanelAction::OpenChapterFolder);
            }

            let add_pages_btn = egui::Button::new(
                RichText::new("➕ Добавить листы").size(12.5),
            )
            .min_size(Vec2::new(btn_width, 26.0));

            if ui
                .add(add_pages_btn)
                .on_hover_text("Добавить отдельные файлы страниц в текущую сессию")
                .clicked()
            {
                action = Some(PagesPanelAction::AddPages);
            }
        });

        // Строка 2: Переключатель вида (Список / Сетка), Сортировка и тумблер Автосинхронизации TypeR
        ui.horizontal(|ui| {
            // Кнопка режима 1 колонки
            let is_list = self.view_mode == PagesViewMode::SingleColumnList;
            if ui
                .selectable_label(is_list, "☰ Список")
                .on_hover_text("Подробный вид в одну колонку")
                .clicked()
            {
                self.view_mode = PagesViewMode::SingleColumnList;
            }

            // Кнопка режима 2-колоночной сетки
            let is_grid = self.view_mode == PagesViewMode::TwoColumnGrid;
            if ui
                .selectable_label(is_grid, "⊞ Сетка")
                .on_hover_text("Компактная двухколоночная сетка миниатюр")
                .clicked()
            {
                self.view_mode = PagesViewMode::TwoColumnGrid;
            }

            // Кнопка натуральной сортировки открытых листов
            if view_info.items.len() > 1
                && ui
                    .button("🔃 А-Я")
                    .on_hover_text("Отсортировать открытые страницы по номерам (1, 2, 10...)")
                    .clicked()
            {
                action = Some(PagesPanelAction::SortPagesNaturally);
            }

            // Кнопка быстрого дробления левого контейнера на 2 дока (Страницы + TypeR)
            let split_label = if view_info.is_left_dock_split { "⬍ Слить" } else { "⬍ 2 дока" };
            let split_tip = if view_info.is_left_dock_split {
                "Объединить панели в один док с вкладками"
            } else {
                "Разделить левый контейнер на два дока: Страницы сверху + TypeR снизу"
            };
            if ui.button(split_label).on_hover_text(split_tip).clicked() {
                action = Some(PagesPanelAction::ToggleSplitDock);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut sync_flag = self.auto_sync_typer;
                if ui
                    .checkbox(&mut sync_flag, RichText::new("🔗 TypeR").size(12.0))
                    .on_hover_text(
                        "Автоматически переключать таблицу реплик TypeR при выборе страницы",
                    )
                    .changed()
                {
                    self.auto_sync_typer = sync_flag;
                    action = Some(PagesPanelAction::ToggleAutoSync(sync_flag));
                }
            });
        });

        // Строка 3: Быстрый поиск и фильтр по статусу
        ui.horizontal(|ui| {
            let search_edit = egui::TextEdit::singleline(&mut self.search_query)
                .hint_text("🔍 Номер или имя файла…")
                .desired_width(ui.available_width() - 92.0);

            ui.add(search_edit);

            if !self.search_query.is_empty()
                && ui
                    .small_button("✖")
                    .on_hover_text("Очистить поиск")
                    .clicked()
            {
                self.search_query.clear();
            }

            // Компактный фильтр по статусу готовности
            egui::ComboBox::from_id_salt("pages_status_filter_combo")
                .width(82.0)
                .selected_text(match self.status_filter {
                    PageStatusFilter::All => "Все",
                    PageStatusFilter::IncompleteOnly => "В работе",
                    PageStatusFilter::CompletedOnly => "Готовые",
                    PageStatusFilter::DirtyOnly => "Измен.",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.status_filter, PageStatusFilter::All, "Все листы");
                    ui.selectable_value(
                        &mut self.status_filter,
                        PageStatusFilter::IncompleteOnly,
                        "⏳ В работе",
                    );
                    ui.selectable_value(
                        &mut self.status_filter,
                        PageStatusFilter::CompletedOnly,
                        "✅ Готовые",
                    );
                    ui.selectable_value(
                        &mut self.status_filter,
                        PageStatusFilter::DirtyOnly,
                        "● Несохраненные",
                    );
                });
        });

        action
    }

    /// Отрисовывает компактную сводку по главе (общее число страниц и прогресс реплик TypeR).
    fn render_chapter_summary_bar(&self, ui: &mut Ui, view_info: &PagesViewInfo) {
        let total_pages = view_info.items.len();
        let total_lines: usize = view_info.items.iter().map(|i| i.total_lines).sum();
        let applied_lines: usize = view_info.items.iter().map(|i| i.applied_lines).sum();
        let dirty_count = view_info.items.iter().filter(|i| i.is_dirty).count();

        egui::Frame::NONE
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let folder_title = view_info
                        .chapter_folder_name
                        .as_deref()
                        .unwrap_or("Сессия главы");

                    ui.label(
                        RichText::new(format!("📑 {folder_title}: {total_pages} стр."))
                            .size(11.5)
                            .strong(),
                    );

                    if dirty_count > 0 {
                        ui.label(
                            RichText::new(format!("• {dirty_count} не сохр."))
                                .size(11.0)
                                .color(Color32::from_rgb(240, 160, 50)),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if total_lines > 0 {
                            let all_done = applied_lines >= total_lines;
                            let badge_color = if all_done {
                                Color32::from_rgb(46, 160, 67)
                            } else {
                                Color32::from_rgb(88, 166, 255)
                            };
                            ui.label(
                                RichText::new(format!("📝 {applied_lines}/{total_lines}"))
                                    .size(11.5)
                                    .strong()
                                    .color(badge_color),
                            )
                            .on_hover_text("Общий прогресс вставки реплик TypeR по всей главе");
                        } else {
                            ui.label(
                                RichText::new("📝 0/0")
                                    .size(11.0)
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }
                    });
                });
            });
    }

    // ------------------------------------------------------------------------
    // ПРОВЕРКА ФИЛЬТРАЦИИ
    // ------------------------------------------------------------------------

    #[must_use]
    fn matches_filter(&self, item: &PageItemInfo) -> bool {
        // 1. Проверка фильтра по статусу
        let status_ok = match self.status_filter {
            PageStatusFilter::All => true,
            PageStatusFilter::IncompleteOnly => {
                item.total_lines == 0 || item.applied_lines < item.total_lines
            }
            PageStatusFilter::CompletedOnly => item.is_translation_complete(),
            PageStatusFilter::DirtyOnly => item.is_dirty,
        };

        if !status_ok {
            return false;
        }

        // 2. Проверка поискового запроса
        let query = self.search_query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }

        // Поиск по номеру страницы ("14", "стр 14", "p14")
        let page_str = item.detected_page.to_string();
        if page_str.contains(&query) || format!("p{page_str}").contains(&query) {
            return true;
        }

        // Поиск по заголовку или имени файла
        if item.title.to_lowercase().contains(&query) {
            return true;
        }

        if let Some(path) = &item.path {
            if let Some(fname) = path.file_name().and_then(|n| n.to_str()) {
                if fname.to_lowercase().contains(&query) {
                    return true;
                }
            }
        }

        false
    }

    // ------------------------------------------------------------------------
    // РЕЖИМ 1: СПИСОК В ОДНУ КОЛОНКУ (SINGLE COLUMN LIST)
    // ------------------------------------------------------------------------

    fn render_single_column_list(
        &mut self,
        ui: &mut Ui,
        items: &[&PageItemInfo],
        total_session_docs: usize,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        for &item in items {
            if let Some(card_action) =
                self.render_page_card_list_mode(ui, item, total_session_docs)
            {
                action = Some(card_action);
            }
        }

        action
    }

    /// Отрисовывает горизонтальную карточку страницы для режима одной колонки.
    fn render_page_card_list_mode(
        &mut self,
        ui: &mut Ui,
        item: &PageItemInfo,
        total_session_docs: usize,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        let card_height = 76.0;
        let avail_width = ui.available_width();
        let card_id = Id::new(("photocraft_page_card_list", item.doc_index));

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(avail_width, card_height), Sense::click_and_drag());

        // Обработка Drag-and-Drop
        if let Some(dnd_action) = self.handle_card_drag_and_drop(ui, &response, rect, item.doc_index)
        {
            action = Some(dnd_action);
        }

        // Цвета фона и рамки карточки
        let is_hovered = response.hovered();
        let (bg_color, border_stroke) = self.compute_card_style(ui, item.is_active, is_hovered, item.doc_index);

        // Отрисовка подложки карточки
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            bg_color,
            border_stroke,
            StrokeKind::Inside,
        );

        // Левая полоска-акцент для активной страницы
        if item.is_active {
            let accent_rect = Rect::from_min_max(
                rect.min,
                Pos2::new(rect.min.x + 4.0, rect.max.y),
            );
            ui.painter().rect_filled(
                accent_rect,
                CornerRadius {
                    nw: 6,
                    sw: 6,
                    ne: 0,
                    se: 0,
                },
                Color32::from_rgb(56, 139, 253),
            );
        }

        // 1. Миниатюра пропорций 3:4 слева
        let thumb_h = card_height - 12.0;
        let thumb_w = (thumb_h * 0.75).round();
        let thumb_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 10.0, rect.min.y + 6.0),
            Vec2::new(thumb_w, thumb_h),
        );
        self.paint_thumbnail_preview(ui, thumb_rect, item);

        // 2. Правая информационная часть карточки
        let content_min_x = thumb_rect.max.x + 10.0;
        let content_max_x = rect.max.x - 8.0;
        let content_rect = Rect::from_min_max(
            Pos2::new(content_min_x, rect.min.y + 6.0),
            Pos2::new(content_max_x.max(content_min_x + 10.0), rect.max.y - 6.0),
        );

        let mut child_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(Layout::top_down(Align::Min))
                .id_salt(card_id),
        );

        // Верхняя строка: "Стр. X", индикатор несохраненных изменений и кнопки быстрого перемещения/закрытия
        child_ui.horizontal(|ui| {
            let page_label_color = if item.is_active {
                Color32::from_rgb(100, 180, 255)
            } else {
                ui.visuals().strong_text_color()
            };

            ui.label(
                RichText::new(format!("Стр. {}", item.detected_page))
                    .size(13.5)
                    .strong()
                    .color(page_label_color),
            );

            if item.is_dirty {
                ui.label(
                    RichText::new("●")
                        .size(11.0)
                        .color(Color32::from_rgb(245, 166, 35)),
                )
                .on_hover_text("Есть несохраненные изменения (is_dirty)");
            }

            // Кнопки управления справа (вверх, вниз, закрыть)
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                if ui
                    .small_button("✖")
                    .on_hover_text("Закрыть страницу")
                    .clicked()
                {
                    action = Some(PagesPanelAction::ClosePage(item.doc_index));
                }

                if item.doc_index + 1 < total_session_docs
                    && ui
                        .small_button("↓")
                        .on_hover_text("Переместить страницу ниже")
                        .clicked()
                {
                    action = Some(PagesPanelAction::ReorderPages {
                        from_idx: item.doc_index,
                        to_idx: item.doc_index + 1,
                    });
                }

                if item.doc_index > 0
                    && ui
                        .small_button("↑")
                        .on_hover_text("Переместить страницу выше")
                        .clicked()
                {
                    action = Some(PagesPanelAction::ReorderPages {
                        from_idx: item.doc_index,
                        to_idx: item.doc_index.saturating_sub(1),
                    });
                }
            });
        });

        // Средняя строка: обрезанное имя файла
        let truncated_title = truncate_middle(&item.title, 24);
        child_ui.label(
            RichText::new(truncated_title)
                .size(11.5)
                .color(child_ui.visuals().text_color()),
        );

        child_ui.add_space(2.0);

        // Нижняя строка: Бейдж готовности реплик TypeR и полоска прогресса
        child_ui.horizontal(|ui| {
            self.render_typer_progress_badge(ui, item);
        });

        // Клик по самой карточке активирует страницу (если не была нажата вложенная кнопка)
        if response.clicked() && action.is_none() {
            action = Some(PagesPanelAction::SelectPage(item.doc_index));
        }

        if is_hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        // Всплывающая подсказка с полным путем и деталями
        let response = response.on_hover_ui(|hover_ui| {
            self.render_card_tooltip(hover_ui, item);
        });

        // Контекстное меню (ПКМ по карточке)
        response.context_menu(|menu_ui| {
            if let Some(menu_action) =
                self.render_card_context_menu(menu_ui, item, total_session_docs)
            {
                action = Some(menu_action);
            }
        });

        action
    }

    // ------------------------------------------------------------------------
    // РЕЖИМ 2: ДВУХКОЛОНОЧНАЯ СЕТКА МИНИАТЮР (TWO COLUMN GRID)
    // ------------------------------------------------------------------------

    fn render_two_column_grid(
        &mut self,
        ui: &mut Ui,
        items: &[&PageItemInfo],
        total_session_docs: usize,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        let spacing = 8.0;
        let avail_width = ui.available_width();
        let col_width = ((avail_width - spacing) * 0.5).max(88.0);

        // Разбиваем отфильтрованные элементы по 2 в ряд
        for row_pair in items.chunks(2) {
            ui.horizontal(|row_ui| {
                row_ui.spacing_mut().item_spacing.x = spacing;

                for &item in row_pair {
                    if let Some(card_action) =
                        self.render_page_card_grid_mode(row_ui, item, col_width, total_session_docs)
                    {
                        action = Some(card_action);
                    }
                }
            });
            ui.add_space(2.0);
        }

        action
    }

    /// Отрисовывает компактную вертикальную карточку страницы для 2-колоночной сетки.
    fn render_page_card_grid_mode(
        &mut self,
        ui: &mut Ui,
        item: &PageItemInfo,
        card_width: f32,
        total_session_docs: usize,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        // Пропорции миниатюры 3:4 + место под заголовок и бейдж
        let thumb_w = (card_width - 12.0).max(60.0);
        let thumb_h = (thumb_w * (4.0 / 3.0) * self.thumbnail_scale).round();
        let card_height = thumb_h + 54.0;

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(card_width, card_height), Sense::click_and_drag());

        // Обработка Drag-and-Drop
        if let Some(dnd_action) = self.handle_card_drag_and_drop(ui, &response, rect, item.doc_index)
        {
            action = Some(dnd_action);
        }

        let is_hovered = response.hovered();
        let (bg_color, border_stroke) =
            self.compute_card_style(ui, item.is_active, is_hovered, item.doc_index);

        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            bg_color,
            border_stroke,
            StrokeKind::Inside,
        );

        // 1. Область миниатюры 3:4 сверху
        let thumb_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + 6.0, rect.min.y + 6.0),
            Vec2::new(thumb_w, thumb_h),
        );
        self.paint_thumbnail_preview(ui, thumb_rect, item);

        // Оверлей-бейдж готовности реплик прямо поверх нижнего правого угла миниатюры
        self.paint_grid_badge_overlay(ui, thumb_rect, item);

        // Индикатор несохраненных изменений в левом верхнем углу миниатюры
        if item.is_dirty {
            let dirty_center = Pos2::new(thumb_rect.min.x + 10.0, thumb_rect.min.y + 10.0);
            ui.painter()
                .circle_filled(dirty_center, 5.5, Color32::from_rgba_unmultiplied(20, 20, 25, 200));
            ui.painter()
                .circle_filled(dirty_center, 4.0, Color32::from_rgb(245, 166, 35));
        }

        // 2. Подпись под миниатюрой: "Стр. X" и сокращенное имя файла
        let footer_rect = Rect::from_min_max(
            Pos2::new(rect.min.x + 6.0, thumb_rect.max.y + 4.0),
            Pos2::new(rect.max.x - 6.0, rect.max.y - 4.0),
        );

        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(footer_rect)
                .layout(Layout::top_down(Align::Min))
                .id_salt(("photocraft_page_card_grid_footer", item.doc_index)),
        );

        footer_ui.horizontal(|ui| {
            let title_color = if item.is_active {
                Color32::from_rgb(100, 180, 255)
            } else {
                ui.visuals().strong_text_color()
            };

            let dirty_suffix = if item.is_dirty { " *" } else { "" };
            ui.label(
                RichText::new(format!("Стр. {}{}", item.detected_page, dirty_suffix))
                    .size(12.5)
                    .strong()
                    .color(title_color),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button("✖")
                    .on_hover_text("Закрыть страницу")
                    .clicked()
                {
                    action = Some(PagesPanelAction::ClosePage(item.doc_index));
                }
            });
        });

        let short_name = truncate_middle(&item.title, 15);
        footer_ui.label(
            RichText::new(short_name)
                .size(11.0)
                .color(footer_ui.visuals().weak_text_color()),
        );

        if response.clicked() && action.is_none() {
            action = Some(PagesPanelAction::SelectPage(item.doc_index));
        }

        if is_hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        let response = response.on_hover_ui(|hover_ui| {
            self.render_card_tooltip(hover_ui, item);
        });

        response.context_menu(|menu_ui| {
            if let Some(menu_action) =
                self.render_card_context_menu(menu_ui, item, total_session_docs)
            {
                action = Some(menu_action);
            }
        });

        action
    }

    // ------------------------------------------------------------------------
    // ОТРИСОВКА МИНИАТЮРЫ, БЕЙДЖЕЙ И КОНТЕКСТНОГО МЕНЮ
    // ------------------------------------------------------------------------

    /// Отрисовывает прямоугольник миниатюры пропорций 3:4 (текстуру либо стилизованный лист манги).
    fn paint_thumbnail_preview(&self, ui: &mut Ui, rect: Rect, item: &PageItemInfo) {
        let painter = ui.painter();

        // Темная подложка под лист
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(28, 30, 36));

        if let Some(tex_id) = item.thumbnail_texture {
            // Отрисовка реальной текстуры страницы с сохранением пропорций и рамки
            let inner = rect.shrink(1.0);
            let img_rect = if let Some((w, h)) = item.dimensions {
                if w > 0 && h > 0 {
                    let page_aspect = w as f32 / h as f32;
                    let box_aspect = inner.width() / inner.height();
                    if page_aspect > box_aspect {
                        let fitted_h = (inner.width() / page_aspect).min(inner.height());
                        Rect::from_center_size(inner.center(), Vec2::new(inner.width(), fitted_h))
                    } else {
                        let fitted_w = (inner.height() * page_aspect).min(inner.width());
                        Rect::from_center_size(inner.center(), Vec2::new(fitted_w, inner.height()))
                    }
                } else {
                    inner
                }
            } else {
                inner
            };
            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
            painter.image(tex_id, img_rect, uv, Color32::WHITE);
        } else {
            // Заглушка в виде классического листа манги с рамками кадров и номером
            let paper_rect = rect.shrink(3.0);
            let paper_color = if item.is_active {
                Color32::from_rgb(242, 245, 250)
            } else {
                Color32::from_rgb(215, 220, 228)
            };
            painter.rect_filled(paper_rect, CornerRadius::same(2), paper_color);

            // Имитация фреймов раскадровки манги на листе-заглушке
            let inner = paper_rect.shrink(4.0);
            if inner.width() > 16.0 && inner.height() > 24.0 {
                let frame_stroke = Stroke::new(1.0, Color32::from_rgb(160, 168, 180));
                let split_y = inner.min.y + inner.height() * 0.42;

                let top_panel = Rect::from_min_max(inner.min, Pos2::new(inner.max.x, split_y - 1.5));
                let bottom_panel =
                    Rect::from_min_max(Pos2::new(inner.min.x, split_y + 1.5), inner.max);

                painter.rect_stroke(
                    top_panel,
                    CornerRadius::ZERO,
                    frame_stroke,
                    StrokeKind::Inside,
                );
                painter.rect_stroke(
                    bottom_panel,
                    CornerRadius::ZERO,
                    frame_stroke,
                    StrokeKind::Inside,
                );
            }

            // Крупный номер страницы по центру заглушки
            painter.text(
                paper_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}", item.detected_page),
                FontId::new(14.0, FontFamily::Proportional),
                Color32::from_rgb(55, 65, 85),
            );
        }

        // Тонкая обводка вокруг самого превью
        let thumb_border = if item.is_active {
            Stroke::new(1.0, Color32::from_rgb(88, 166, 255))
        } else {
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 30))
        };
        painter.rect_stroke(rect, CornerRadius::same(4), thumb_border, StrokeKind::Inside);
    }

    /// Отрисовывает бейдж прогресса реплик `📝 4/8` для списочного режима.
    fn render_typer_progress_badge(&self, ui: &mut Ui, item: &PageItemInfo) {
        let (badge_bg, badge_fg, badge_text) = Self::compute_badge_visuals(item);

        egui::Frame::NONE
            .fill(badge_bg)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(badge_text)
                        .size(11.0)
                        .strong()
                        .color(badge_fg),
                );
            });

        // Если есть реплики, рисуем компактный прогресс-бар рядом с бейджем
        if item.total_lines > 0 {
            let ratio = item.completion_ratio();
            let bar_width = ui.available_width().clamp(24.0, 64.0);
            let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(bar_width, 5.0), Sense::hover());

            ui.painter().rect_filled(
                bar_rect,
                CornerRadius::same(2),
                Color32::from_rgba_unmultiplied(120, 130, 145, 50),
            );

            if ratio > 0.0 {
                let fill_w = (bar_rect.width() * ratio).max(2.0);
                let fill_rect =
                    Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
                let fill_color = if item.is_translation_complete() {
                    Color32::from_rgb(46, 160, 67)
                } else {
                    Color32::from_rgb(56, 139, 253)
                };
                ui.painter()
                    .rect_filled(fill_rect, CornerRadius::same(2), fill_color);
            }
        }
    }

    /// Рисует компактный бейдж `📝 X/Y` в правом нижнем углу миниатюры для сеточного режима.
    fn paint_grid_badge_overlay(&self, ui: &mut Ui, thumb_rect: Rect, item: &PageItemInfo) {
        let (badge_bg, badge_fg, badge_text) = Self::compute_badge_visuals(item);

        let galley = ui.painter().layout_no_wrap(
            badge_text,
            FontId::new(10.5, FontFamily::Proportional),
            badge_fg,
        );

        let pad = Vec2::new(5.0, 2.5);
        let badge_size = galley.size() + pad * 2.0;
        let badge_rect = Rect::from_min_size(
            Pos2::new(
                thumb_rect.max.x - badge_size.x - 4.0,
                thumb_rect.max.y - badge_size.y - 4.0,
            ),
            badge_size,
        );

        ui.painter()
            .rect_filled(badge_rect, CornerRadius::same(4), badge_bg);
        ui.painter()
            .galley(badge_rect.min + pad, galley, badge_fg);
    }

    /// Вычисляет цвета и текст бейджа готовности реплик:
    /// - Зеленый, если все реплики вставлены (`applied == total > 0`)
    /// - Синий/янтарный, если в процессе (`0 < applied < total` или `applied == 0 && total > 0`)
    /// - Серый, если реплик на странице нет (`total == 0`)
    #[must_use]
    fn compute_badge_visuals(item: &PageItemInfo) -> (Color32, Color32, String) {
        if item.total_lines == 0 {
            (
                Color32::from_rgba_unmultiplied(60, 65, 75, 210),
                Color32::from_rgb(165, 175, 190),
                "📝 0/0".to_string(),
            )
        } else if item.applied_lines >= item.total_lines {
            (
                Color32::from_rgba_unmultiplied(28, 115, 52, 230),
                Color32::from_rgb(220, 255, 230),
                format!("📝 {}/{} ✔", item.applied_lines, item.total_lines),
            )
        } else if item.applied_lines > 0 {
            (
                Color32::from_rgba_unmultiplied(30, 85, 165, 230),
                Color32::from_rgb(220, 240, 255),
                format!("📝 {}/{}", item.applied_lines, item.total_lines),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(95, 70, 25, 225),
                Color32::from_rgb(255, 225, 165),
                format!("📝 0/{}", item.total_lines),
            )
        }
    }

    /// Вычисляет цвет фона и обводки карточки страницы с учетом активности, наведения и Drag-and-Drop.
    #[must_use]
    fn compute_card_style(
        &self,
        ui: &Ui,
        is_active: bool,
        is_hovered: bool,
        doc_index: usize,
    ) -> (Color32, Stroke) {
        let is_dragged = self.dragged_doc_index == Some(doc_index);
        let is_drop_target = self.drop_target_doc_index == Some(doc_index);

        let bg_color = if is_dragged {
            Color32::from_rgba_unmultiplied(56, 139, 253, 45)
        } else if is_active {
            Color32::from_rgba_unmultiplied(45, 95, 175, 55)
        } else if is_hovered {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().faint_bg_color
        };

        let stroke = if is_drop_target {
            // Яркая бирюзовая/зеленая рамка для цели сброса при перетаскивании
            Stroke::new(2.0, Color32::from_rgb(63, 185, 80))
        } else if is_active {
            // Синяя акцентная рамка для активной страницы
            Stroke::new(1.8, Color32::from_rgb(56, 139, 253))
        } else if is_hovered {
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(140, 160, 190, 140))
        } else {
            // Полупрозрачная рамка для неактивных карточек
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 130, 145, 60))
        };

        (bg_color, stroke)
    }

    /// Обрабатывает перетаскивание карточки мышью (Drag-and-Drop) для изменения порядка страниц.
    fn handle_card_drag_and_drop(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        rect: Rect,
        doc_index: usize,
    ) -> Option<PagesPanelAction> {
        if response.drag_started() {
            let pointer_pos = ui.input(|i| i.pointer.press_origin());
            let is_in_buttons = pointer_pos.is_some_and(|pos| {
                pos.x > rect.max.x - 70.0 && pos.y < rect.min.y + 28.0
            });
            if !is_in_buttons {
                self.dragged_doc_index = Some(doc_index);
            }
        }

        if let Some(from_idx) = self.dragged_doc_index {
            if from_idx != doc_index {
                let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                if let Some(pos) = pointer_pos {
                    if rect.contains(pos) {
                        self.drop_target_doc_index = Some(doc_index);

                        // Рисуем линию индикации места вставки
                        let line_y = if from_idx < doc_index {
                            rect.max.y
                        } else {
                            rect.min.y
                        };
                        ui.painter().line_segment(
                            [Pos2::new(rect.min.x, line_y), Pos2::new(rect.max.x, line_y)],
                            Stroke::new(2.5, Color32::from_rgb(63, 185, 80)),
                        );

                        if ui.input(|i| i.pointer.any_released()) {
                            self.dragged_doc_index = None;
                            self.drop_target_doc_index = None;
                            return Some(PagesPanelAction::ReorderPages {
                                from_idx,
                                to_idx: doc_index,
                            });
                        }
                    }
                }
            }
        }

        None
    }

    /// Контекстное меню карточки страницы (ПКМ).
    fn render_card_context_menu(
        &self,
        ui: &mut Ui,
        item: &PageItemInfo,
        total_session_docs: usize,
    ) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        ui.label(
            RichText::new(format!("📄 Стр. {} — {}", item.detected_page, item.title))
                .size(11.5)
                .strong(),
        );
        ui.separator();

        if !item.is_active
            && ui
                .button("🎯 Сделать активной")
                .clicked()
        {
            action = Some(PagesPanelAction::SelectPage(item.doc_index));
            ui.close();
        }

        // Перемещение вверх / вниз
        let can_move_up = item.doc_index > 0;
        if ui
            .add_enabled(can_move_up, egui::Button::new("⬆ Переместить вверх"))
            .clicked()
        {
            action = Some(PagesPanelAction::ReorderPages {
                from_idx: item.doc_index,
                to_idx: item.doc_index.saturating_sub(1),
            });
            ui.close();
        }

        let can_move_down = item.doc_index + 1 < total_session_docs;
        if ui
            .add_enabled(can_move_down, egui::Button::new("⬇ Переместить вниз"))
            .clicked()
        {
            action = Some(PagesPanelAction::ReorderPages {
                from_idx: item.doc_index,
                to_idx: item.doc_index + 1,
            });
            ui.close();
        }

        ui.separator();

        // Показать в проводнике (активно, если у документа есть путь на диске)
        if let Some(path) = &item.path {
            if ui.button("📂 Показать в проводнике").clicked() {
                action = Some(PagesPanelAction::RevealInExplorer(path.clone()));
                ui.close();
            }
        } else {
            ui.add_enabled(
                false,
                egui::Button::new("📂 Показать в проводнике (не сохранен)"),
            );
        }

        ui.separator();

        if ui
            .button(
                RichText::new("✖ Закрыть страницу").color(Color32::from_rgb(240, 90, 90)),
            )
            .clicked()
        {
            action = Some(PagesPanelAction::ClosePage(item.doc_index));
            ui.close();
        }

        action
    }

    /// Подробная всплывающая подсказка при наведении на карточку страницы.
    fn render_card_tooltip(&self, ui: &mut Ui, item: &PageItemInfo) {
        ui.label(
            RichText::new(format!("Страница #{}", item.detected_page))
                .strong()
                .size(13.0),
        );
        ui.label(format!("Файл: {}", item.title));

        if let Some(path) = &item.path {
            ui.label(
                RichText::new(path.display().to_string())
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        }

        if let Some((w, h)) = item.dimensions {
            ui.label(format!("Размер холста: {w} × {h} px"));
        }

        ui.separator();
        if item.total_lines > 0 {
            let pct = (item.completion_ratio() * 100.0).round() as usize;
            ui.label(format!(
                "Реплики TypeR: {} из {} вставлено ({pct}%)",
                item.applied_lines, item.total_lines
            ));
        } else {
            ui.label("Реплики TypeR: нет привязанных строк для этой страницы");
        }

        if item.is_dirty {
            ui.label(
                RichText::new("● Имеются несохраненные правки")
                    .color(Color32::from_rgb(245, 166, 35)),
            );
        }
    }

    // ------------------------------------------------------------------------
    // ПУСТЫЕ СОСТОЯНИЯ (EMPTY STATES)
    // ------------------------------------------------------------------------

    fn render_empty_state(&self, ui: &mut Ui) -> Option<PagesPanelAction> {
        let mut action: Option<PagesPanelAction> = None;

        ui.vertical_centered(|ui| {
            ui.add_space(36.0);
            ui.label(RichText::new("📚").size(32.0));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Нет открытых страниц главы")
                    .size(13.5)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Откройте папку с главой манги или добавьте\nотдельные листы для синхронной верстки с TypeR.",
                )
                .size(11.5)
                .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(12.0);

            if ui
                .button(RichText::new("📂 Открыть папку главы…").strong())
                .clicked()
            {
                action = Some(PagesPanelAction::OpenChapterFolder);
            }
        });

        action
    }

    fn render_no_filter_matches(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(28.0);
            ui.label(RichText::new("🔍 Страницы не найдены").strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("Ни одна страница не соответствует условиям фильтра.")
                    .size(11.5)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(8.0);
            if ui.button("Сбросить фильтр").clicked() {
                self.search_query.clear();
                self.status_filter = PageStatusFilter::All;
            }
        });
    }
}

// ============================================================================
// 4. ВСПОМОГАТЕЛЬНЫЕ ФУНКЦИИ ФОРМАТИРОВАНИЯ
// ============================================================================

/// Аккуратно обрезает длинное имя файла посередине (`"ch01_page_..._final.png"`),
/// сохраняя начало и расширение файла, без использования паникующих срезов по байтам.
#[must_use]
fn truncate_middle(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars || max_chars < 6 {
        return text.to_string();
    }

    let head_len = (max_chars.saturating_sub(1)) / 2;
    let tail_len = max_chars.saturating_sub(1).saturating_sub(head_len);

    let head: String = text.chars().take(head_len).collect();
    let tail: String = text
        .chars()
        .skip(char_count.saturating_sub(tail_len))
        .collect();

    format!("{head}…{tail}")
}

// ============================================================================
// 5. МОДУЛЬНЫЕ ТЕСТЫ (БЕЗ UNWRAP / EXPECT)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_page_number_from_name() {
        assert_eq!(parse_page_number_from_name("01.png"), Some(1));
        assert_eq!(parse_page_number_from_name("ch02_p14.jpg"), Some(14));
        assert_eq!(parse_page_number_from_name("page_005.psd"), Some(5));
        assert_eq!(parse_page_number_from_name("raw-12.webp"), Some(12));
        assert_eq!(parse_page_number_from_name("p3.png"), Some(3));
        assert_eq!(parse_page_number_from_name("vol03_ch105_024.tif"), Some(24));
        assert_eq!(parse_page_number_from_name("cover_credits.png"), None);
    }

    #[test]
    fn test_natural_sort_paths() {
        let mut paths = vec![
            PathBuf::from("10.png"),
            PathBuf::from("2.png"),
            PathBuf::from("1.png"),
            PathBuf::from("ch1_p10.png"),
            PathBuf::from("ch1_p2.png"),
        ];

        natural_sort_paths(&mut paths);

        let expected = vec![
            PathBuf::from("1.png"),
            PathBuf::from("2.png"),
            PathBuf::from("10.png"),
            PathBuf::from("ch1_p2.png"),
            PathBuf::from("ch1_p10.png"),
        ];

        assert_eq!(paths, expected);
    }
}