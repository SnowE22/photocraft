//! Алгоритм вёрстки строк Кнута—Пласса (Knuth–Plass line-breaking / Boxes & Glue)
//! и типографической подготовки текста (устранение «висячих» предлогов и союзов)
//! с оптимизацией под форму манга-баллона (овал / ромб).

/// Символ неразрывного пробела (Non-Breaking Space, NBSP)
pub const NBSP: char = '\u{00A0}';

/// Привязывает короткие предлоги, союзы и частицы неразрывным пробелом (NBSP),
/// устраняя «висячие» предлоги на концах строк и одинокие частицы в начале строк.
pub fn bind_orphan_prepositions(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // Предлоги и союзы, которые привязываются к СЛЕДУЮЩЕМУ слову
    const FORWARD_BINDERS: &[&str] = &[
        // Однобуквенные и двухбуквенные предлоги/союзы (RU)
        "а", "в", "и", "к", "о", "с", "у", "я",
        "не", "ни", "на", "но", "по", "за", "из", "от", "до", "во", "со", "ко", "об",
        "под", "над", "при", "про", "для", "без",
        // English common prepositions & articles
        "a", "an", "the", "in", "on", "at", "to", "by", "or", "as", "if", "is", "of", "it", "no", "so", "my", "he", "we"
    ];

    // Частицы, которые привязываются к ПРЕДЫДУЩЕМУ слову
    const BACKWARD_BINDERS: &[&str] = &[
        "же", "бы", "ли", "ж", "б", "ль", "то", "ка"
    ];

    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let ch = chars[i];

        // Проверяем пробельные разделители
        if ch == ' ' {
            // Заглядываем вперед: что за слово идет после пробела?
            let mut next_start = i + 1;
            while next_start < n && chars[next_start] == ' ' {
                next_start += 1;
            }

            if next_start < n {
                // Извлекаем следующее слово (буквенно-цифровую часть)
                let mut next_end = next_start;
                while next_end < n && (chars[next_end].is_alphanumeric() || chars[next_end] == '-') {
                    next_end += 1;
                }

                let next_word: String = chars[next_start..next_end].iter().collect();
                let next_lower = next_word.to_lowercase();

                // 1. Проверяем обратную привязку: если следующее слово - частица (же, бы, ли...)
                // то пробел перед ней заменяем на неразрывный пробел (NBSP)
                if BACKWARD_BINDERS.iter().any(|&b| b == next_lower) {
                    result.push(NBSP);
                    i += 1;
                    continue;
                }

                // 2. Проверяем прямую привязку: если следующее слово - предлог/союз,
                // то после него пробел будет заменен на NBSP.
            }

            result.push(ch);
            i += 1;
            continue;
        }

        // Если это начало слова: проверим, не является ли оно прямым предлогом/союзом
        if (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '«' || chars[i - 1] == '"' || chars[i - 1] == '(' || chars[i - 1] == '[')
            && (ch.is_alphanumeric() || ch == '-')
        {
            let mut word_end = i;
            while word_end < n && (chars[word_end].is_alphanumeric() || chars[word_end] == '-') {
                word_end += 1;
            }

            let word: String = chars[i..word_end].iter().collect();
            let word_lower = word.to_lowercase();

            // Проверяем, является ли слово предлогом/союзом
            let is_forward = FORWARD_BINDERS.iter().any(|&b| b == word_lower);

            for c in chars[i..word_end].iter() {
                result.push(*c);
            }
            i = word_end;

            // Если это предлог и дальше идет обычный пробел — заменяем его на неразрывный пробел!
            if is_forward && i < n && chars[i] == ' ' {
                result.push(NBSP);
                i += 1;
                // Пропускаем возможные лишние пробелы
                while i < n && chars[i] == ' ' {
                    i += 1;
                }
            }
            continue;
        }

        result.push(ch);
        i += 1;
    }

    result
}

/// Приблизительный коэффициент ширины символа относительно кегля шрифта (em)
pub fn char_width_factor(c: char) -> f32 {
    match c {
        ' ' | '\u{00A0}' => 0.28,
        '.' | ',' | ':' | ';' | '!' | '|' | '\'' | '`' | 'i' | 'l' | 'j' => 0.25,
        'f' | 'r' | 't' | '(' | ')' | '[' | ']' | '{' | '}' => 0.35,
        'm' | 'w' | 'M' | 'W' | '—' | '…' => 0.85,
        'Ж' | 'Ш' | 'Щ' | 'Ю' | 'Ф' => 0.90,
        'ж' | 'ш' | 'щ' | 'ю' | 'ф' => 0.75,
        c if c.is_ascii_uppercase() => 0.65,
        c if ('А'..='Я').contains(&c) || c == 'Ё' => 0.65,
        c if ('а'..='я').contains(&c) || c == 'ё' => 0.52,
        c if c.is_ascii_digit() => 0.55,
        c if ('\u{3000}'..='\u{9FFF}').contains(&c) => 1.0, // CJK
        _ => 0.50,
    }
}

/// Оценивает визуальную ширину слова или токена в пикселях с учетом шрифта и масштаба
pub fn measure_token_width(token: &str, font_size_px: f32, h_scale: f32) -> f32 {
    let mut w = 0.0;
    for c in token.chars() {
        w += char_width_factor(c) * font_size_px * h_scale;
    }
    w
}


fn is_vowel(ch: char) -> bool {
    matches!(
        ch.to_ascii_lowercase(),
        'a' | 'e' | 'i' | 'o' | 'u' | 'y' | 'а' | 'е' | 'ё' | 'и' | 'о' | 'у' | 'ы' | 'э' | 'ю' | 'я'
    )
}

/// Returns a byte offset where a conservative language-agnostic hyphen may be inserted.
/// Long alphabetic words break after a vowel group near the middle; short words never do.
pub fn hyphenation_point(word: &str) -> Option<usize> {
    hyphenation_candidates(word).into_iter().next()
}

fn hyphenation_candidates(word: &str) -> Vec<usize> {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 7 || chars.len() > 160 || !chars.iter().all(|ch| ch.is_alphabetic() || *ch == '-') {
        return Vec::new();
    }
    let middle = chars.len() / 2;

    // 1. Existing hyphens in compound words (e.g. "какой-то") are priority break points
    let mut candidates: Vec<usize> = Vec::new();
    for (byte, ch) in word.char_indices() {
        if ch == '-' && byte + 1 < word.len() {
            candidates.push(byte + 1);
        }
    }

    // 2. Language-aware syllabification ported from TypeR Studio hyphenation rules; the old
    // vowel-group heuristic remains as a fallback for words without a legal break.
    if candidates.is_empty() {
        candidates = crate::snow_hyphenation::hyphenate(word, None)
            .into_iter()
            .filter_map(|index| word.char_indices().nth(index).map(|(byte, _)| byte))
            .collect();
    }

    // 3. Vowel-group fallback mapping char index to exact UTF-8 byte boundary
    if candidates.is_empty() {
        candidates = (3..chars.len().saturating_sub(2))
            .filter(|&index| is_vowel(chars[index - 1]) && !is_vowel(chars[index]))
            .filter_map(|index| word.char_indices().nth(index).map(|(byte, _)| byte))
            .collect();
    }
    candidates.sort_by_key(|byte| word.get(..*byte).map_or(0, |prefix| prefix.chars().count()).abs_diff(middle));
    candidates
}

/// Splits `token` into a hyphenated left part and the remaining right part when the left
/// part fits `max_width`. At most one hyphen is inserted per word, as TypeR does.
pub fn hyphenate_token(token: &str, max_width: f32, font_size_px: f32, h_scale: f32) -> Option<(String, String)> {
    for break_at in hyphenation_candidates(token) {
        if !token.is_char_boundary(break_at) {
            continue;
        }
        let (left, right) = token.split_at(break_at);
        if left.is_empty() || right.is_empty() {
            continue;
        }
        let left = if left.ends_with('-') {
            left.to_string()
        } else {
            format!("{left}-")
        };
        if measure_token_width(&left, font_size_px, h_scale) <= max_width {
            return Some((left, right.to_string()));
        }
    }
    None
}

/// Adds forced line breaks after hyphens in words wider than `max_width`. Used by the
/// contour fit, which lays out each resulting paragraph line separately.
pub fn hyphenate_long_words_with_breaks(text: &str, max_width: f32, font_size_px: f32, h_scale: f32) -> String {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let mut output = String::new();
        for (index, word) in line.split(' ').filter(|word| !word.is_empty()).enumerate() {
            if index > 0 {
                output.push(' ');
            }
            if measure_token_width(word, font_size_px, h_scale) > max_width
                && let Some((left, right)) = hyphenate_token(word, max_width, font_size_px, h_scale)
            {
                output.push_str(&left);
                output.push('\n');
                output.push_str(&right);
            } else {
                output.push_str(word);
            }
        }
        lines.push(output);
    }
    lines.join("\n")
}

fn join_tokens(tokens: &[String]) -> String {
    let mut output = String::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 && !tokens[index - 1].ends_with('-') {
            output.push(' ');
        }
        output.push_str(token);
    }
    output
}

/// Оценивает величину оптической компенсации (выноса за границу строки) для знаков препинания на концах слов
pub fn punctuation_compensation_px(token: &str, font_size_px: f32, h_scale: f32) -> f32 {
    let mut comp = 0.0;
    if token.is_empty() {
        return 0.0;
    }
    if token.ends_with("...") || token.ends_with("…") {
        comp += 0.50 * font_size_px * h_scale;
    } else if let Some(last) = token.chars().last() {
        match last {
            '.' | ',' | ':' | ';' | '!' | '?' => {
                comp += 0.25 * font_size_px * h_scale;
            }
            '—' | '–' | '-' => {
                comp += 0.35 * font_size_px * h_scale;
            }
            '»' | ')' | ']' | '}' | '"' | '\'' => {
                comp += 0.25 * font_size_px * h_scale;
            }
            _ => {}
        }
    }
    comp
}

/// Солвер переносов строк Кнута—Пласса (Knuth–Plass / Boxes & Glue)
/// с оптимизацией распределения слов под форму овального или ромбовидного баббла манги.
pub fn knuth_plass_bubble_wrap(
    text: &str,
    bubble_width: f32,
    bubble_height: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
) -> String {
    knuth_plass_bubble_wrap_ext(text, bubble_width, bubble_height, font_size_pt, dpi, horizontal_scale_pct, true)
}

/// Enum для определения формы манга-баббла
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleShape {
    VerticalOval,
    HorizontalOval,
    Rectangle,
    Diamond,
    RoundOval,
}

impl BubbleShape {
    /// Определяет форму на основе соотношения сторон баббла
    pub fn from_aspect_ratio(width: f32, height: f32) -> Self {
        if width <= 10.0 || height <= 10.0 {
            return BubbleShape::Rectangle;
        }
        let ratio = width / height;
        if ratio > 1.35 {
            BubbleShape::HorizontalOval
        } else if ratio < 0.85 {
            BubbleShape::VerticalOval
        } else {
            BubbleShape::RoundOval
        }
    }
}

/// Расширенная версия солвера Кнута—Пласса с поддержкой оптической компенсации знаков в строках (висячая пунктуация)
pub fn knuth_plass_bubble_wrap_ext(
    text: &str,
    bubble_width: f32,
    bubble_height: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
    hanging_punctuation: bool,
) -> String {
    let shape = BubbleShape::from_aspect_ratio(bubble_width, bubble_height);
    knuth_plass_bubble_wrap_with_shape(
        text,
        bubble_width,
        bubble_height,
        font_size_pt,
        dpi,
        horizontal_scale_pct,
        hanging_punctuation,
        shape,
    )
}

/// Основная реализация Knuth-Plass с учетом формы баббла
#[allow(clippy::too_many_arguments)]
pub fn knuth_plass_bubble_wrap_lines(
    text: &str,
    bubble_width: f32,
    bubble_height: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
    hanging_punctuation: bool,
    shape: BubbleShape,
    target_lines: Option<usize>,
) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() || bubble_width <= 10.0 {
        return text.to_string();
    }

    // Если текст уже содержит явные переносы строк, применяем висячку к каждой строке отдельно
    if trimmed.contains('\n') {
        let lines: Vec<String> = trimmed.split('\n').map(bind_orphan_prepositions).collect();
        return lines.join("\n");
    }

    // 1. Применяем «висячку» (NBSP для предлогов и союзов)
    let bound_text = bind_orphan_prepositions(trimmed);

    let dpi_scale = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let font_size_px = font_size_pt * dpi_scale;
    let h_scale = (horizontal_scale_pct / 100.0).clamp(0.1, 5.0);

    // Высота строки
    let line_height_px = font_size_px * 1.25;

    // Максимальное количество строк, которое может вместить баббло по высоте
    let _max_height_lines = if bubble_height > 10.0 {
        ((bubble_height * 0.88) / line_height_px.max(1.0)).floor() as usize
    } else {
        usize::MAX
    };

    // Максимальная безопасная ширина строки внутри баббла
    let max_safe_width = match shape {
        BubbleShape::VerticalOval => (bubble_width * 0.78).max(font_size_px * 1.8),
        BubbleShape::HorizontalOval => (bubble_width * 0.90).max(font_size_px * 2.0),
        BubbleShape::Rectangle => (bubble_width * 0.92).max(font_size_px * 2.0),
        _ => (bubble_width * 0.85).max(font_size_px * 2.0),
    };

    // Токенизация: разделяем по обычным пробелам (токены с NBSP внутри остаются едиными блоками!)
    let mut tokens: Vec<String> = Vec::new();
    for token in bound_text.split(' ').filter(|s| !s.is_empty()) {
        let mut cur = token.to_string();
        while measure_token_width(&cur, font_size_px, h_scale) > max_safe_width {
            if let Some((left, right)) = hyphenate_token(&cur, max_safe_width, font_size_px, h_scale) {
                tokens.push(left);
                cur = right;
            } else {
                break;
            }
        }
        tokens.push(cur);
    }
    if tokens.len() <= 1 {
        return bound_text;
    }

    let space_width = char_width_factor(' ') * font_size_px * h_scale;
    let token_widths: Vec<f32> = tokens.iter().map(|token| measure_token_width(token, font_size_px, h_scale)).collect();
    let total_text_width: f32 = token_widths.iter().sum::<f32>() + (tokens.len() - 1) as f32 * space_width;

    // TypeR Studio: короткий текст из 1-2 слов можно оставить в одну строку,
    // но текст из 3+ слов форматируется в сбалансированные строки
    if tokens.len() <= 2 && target_lines.is_none() && total_text_width <= max_safe_width * 0.95 {
        return bound_text;
    }

    // Оцениваем количество строк: диапазон k строго ограничен числом токенов n
    let n = tokens.len();
    let est_lines = (total_text_width / (max_safe_width * 0.75)).ceil() as usize;
    let (min_lines, max_lines) = if let Some(k) = target_lines {
        let k_clamped = k.clamp(1, n);
        (k_clamped, k_clamped)
    } else {
        let min_l = if n >= 3 { 2.min(n) } else { 1 };
        let max_l = (est_lines + 2).clamp(min_l, n);
        (min_l, max_l)
    };

    let mut best_demerits = f64::MAX;
    let mut best_break_indices: Vec<usize> = Vec::new();

    // Профиль целевой ширины для эллипса/ромба манга-баллона:
    let target_width_for_line = |line_idx: usize, total_k: usize| -> f32 {
        if total_k <= 1 {
            return max_safe_width;
        }
        let t = (line_idx as f32 + 0.5) / total_k as f32; // 0.0 .. 1.0
        let y = (t * 2.0 - 1.0).clamp(-1.0, 1.0); // -1.0 .. +1.0
        
        match shape {
            BubbleShape::Rectangle => max_safe_width * 0.95,
            BubbleShape::VerticalOval => {
                let ellipse_factor = (1.0 - y * y).max(0.0).sqrt();
                max_safe_width * (0.65 + 0.32 * ellipse_factor)
            }
            BubbleShape::HorizontalOval => {
                let ellipse_factor = (1.0 - y * y).max(0.0).sqrt();
                max_safe_width * (0.75 + 0.22 * ellipse_factor)
            }
            BubbleShape::Diamond => {
                let diamond_factor = (1.0 - y.abs()).max(0.0);
                max_safe_width * (0.50 + 0.48 * diamond_factor)
            }
            BubbleShape::RoundOval => {
                let ellipse_factor = (1.0 - y * y).max(0.0).sqrt();
                max_safe_width * (0.60 + 0.35 * ellipse_factor)
            }
        }
    };

    // Перебираем количество строк k от min_lines до max_lines
    for k in min_lines..=max_lines {
        // Динамическое программирование: dp[i][line_num] = (min_demerits, prev_word_idx)
        let n = tokens.len();
        // dp[w][l] = (demerits, prev_w)
        let mut dp = vec![vec![(f64::MAX, 0usize); k + 1]; n + 1];
        dp[0][0] = (0.0, 0);

        for line_num in 1..=k {
            let target_w = target_width_for_line(line_num - 1, k);

            for j in 1..=n {
                // Ищем лучший предыдущий разрыв i
                let mut line_w = 0.0;
                for i in (0..j).rev() {
                    let w_tok = token_widths[i];
                    line_w = if i == j - 1 { w_tok } else { line_w + space_width + w_tok };

                    if dp[i][line_num - 1].0 == f64::MAX {
                        continue;
                    }

                    // Оптическая компенсация знаков в строках (вынос концевой пунктуации за границу строки)
                    let comp = if hanging_punctuation {
                        punctuation_compensation_px(&tokens[j - 1], font_size_px, h_scale)
                    } else {
                        0.0
                    };
                    let effective_w = (line_w - comp).max(0.0);

                    // Штрафы Кнута—Пласса (Demerits):
                    // 1. Разница с целевой шириной строки
                    let diff = (effective_w - target_w).abs();
                    let relative_diff = diff / target_w;
                    let mut demerits = (relative_diff * 100.0) as f64 * (relative_diff * 100.0) as f64;

                    // 2. Жесткий штраф за превышение максимальной безопасной ширины баббла
                    if effective_w > max_safe_width {
                        let overflow = effective_w - max_safe_width;
                        demerits += (overflow * 500.0) as f64;
                    }

                    // 3. Штраф за слишком короткую строку из одного короткого слова (одиночка/висячка)
                    if j - i == 1 && tokens[i].chars().count() <= 2 {
                        demerits += 25000.0;
                    }

                    // 4. Штраф за перенос дефисного слова
                    if tokens[i].ends_with('-') || tokens[i].ends_with('—') {
                        demerits += 3000.0;
                    }

                    let total_demerits = dp[i][line_num - 1].0 + demerits;
                    if total_demerits < dp[j][line_num].0 {
                        dp[j][line_num] = (total_demerits, i);
                    }
                }
            }
        }

        let mut cand_demerits = dp[n][k].0;
        // Штраф за превышение высоты баббла
        if bubble_height > 10.0 {
            let block_h = k as f32 * line_height_px;
            if block_h > bubble_height * 0.90 {
                let overflow_h = block_h - bubble_height * 0.90;
                cand_demerits += (overflow_h * 400.0) as f64;
            }
        }

        if cand_demerits < best_demerits {
            best_demerits = cand_demerits;
            // Восстанавливаем разрывы строк
            let mut breaks = Vec::new();
            let mut curr_w = n;
            let mut curr_l = k;
            while curr_l > 0 && curr_w > 0 {
                let prev_w = dp[curr_w][curr_l].1;
                breaks.push(prev_w);
                curr_w = prev_w;
                curr_l -= 1;
            }
            breaks.reverse();
            best_break_indices = breaks;
        }
    }

    if best_break_indices.is_empty() {
        return bound_text;
    }

    // Собираем итоговые строки
    let mut lines = Vec::new();
    for window in best_break_indices.windows(2) {
        let start = window[0];
        let end = window[1];
        lines.push(join_tokens(&tokens[start..end]));
    }
    if let Some(&last_start) = best_break_indices.last()
        && last_start < tokens.len()
    {
        lines.push(join_tokens(&tokens[last_start..]));
    }

    lines.join("\n")
}

#[allow(clippy::too_many_arguments)]
pub fn knuth_plass_bubble_wrap_with_shape(
    text: &str,
    bubble_width: f32,
    bubble_height: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
    hanging_punctuation: bool,
    shape: BubbleShape,
) -> String {
    knuth_plass_bubble_wrap_lines(
        text,
        bubble_width,
        bubble_height,
        font_size_pt,
        dpi,
        horizontal_scale_pct,
        hanging_punctuation,
        shape,
        None,
    )
}

// ============================================================================
// TypeR Studio TextShapeR Core Engine (Ported directly from TypeR Studio textShapeR.js)
// ============================================================================

/// Единичная ширина символа по метрикам TypeR Studio (базовый символ = 1.0)
pub fn visible_char_width(c: char) -> f32 {
    if c.is_whitespace() || matches!(c, 'i' | 'l' | 'I' | '.' | ',' | ';' | ':' | '!' | '|' | '\'' | '’' | '«' | '»' | '‹' | '›' | '(' | ')' | '[' | ']' | '{' | '}' | '…') {
        0.45
    } else if matches!(c, 'm' | 'w' | 'M' | 'W' | '@' | '#' | '%' | '&' | 'ж' | 'ш' | 'щ' | 'ю' | 'Ж' | 'Ш' | 'Щ' | 'Ю') {
        1.32
    } else if c.is_ascii_uppercase() || ('А'..='Я').contains(&c) || matches!(c, 'Ё' | 'Ї' | 'І' | 'Є' | 'Ґ') {
        1.12
    } else {
        1.0
    }
}

/// Измеряет визуальную длину строки в единицах TypeR Studio
pub fn measure_visible_width(text: &str) -> f32 {
    let mut w = 0.0;
    for c in text.chars() {
        w += visible_char_width(c);
    }
    w
}

/// Измеряет визуальную длину строки с учетом межсловных пробелов (0.45 em)
pub fn line_visible_width(line: &str) -> f32 {
    let words: Vec<&str> = line.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return 0.0;
    }
    let words_w: f32 = words.iter().map(|w| measure_visible_width(w)).sum();
    words_w + (words.len() - 1) as f32 * 0.45
}

/// Расчет весов силуэта баббла (TypeR Studio buildWeights)
pub fn build_textshaper_weights(line_count: usize, curve: f32, shift: f32, min_weight: f32) -> Vec<f32> {
    if line_count <= 1 {
        return vec![1.0];
    }
    let center = (line_count as f32 - 1.0) / 2.0 + shift;
    let max_dist = center.max(line_count as f32 - 1.0 - center).max(1.0);
    (0..line_count)
        .map(|idx| {
            let dist = (idx as f32 - center).abs() / max_dist;
            (1.0 - dist * curve).max(min_weight)
        })
        .collect()
}

/// Построение целевой ширины строк (TypeR Studio buildTargets)
pub fn build_textshaper_targets(
    tokens: &[String],
    line_count: usize,
    curve: f32,
    shift: f32,
    bias: f32,
    min_weight: f32,
) -> Vec<f32> {
    let total: f32 = tokens.iter().map(|t| measure_visible_width(t)).sum::<f32>()
        + tokens.len().saturating_sub(line_count) as f32 * 0.45;
    let weights = build_textshaper_weights(line_count, curve, shift, min_weight);
    let w_sum: f32 = weights.iter().sum::<f32>().max(0.001);
    weights
        .into_iter()
        .map(|w| ((total * w) / w_sum + bias).max(1.0))
        .collect()
}

/// 2D Динамическое программирование (TypeR Studio buildCandidate)
pub fn build_textshaper_candidate_dp(
    tokens: &[String],
    line_count: usize,
    targets: &[f32],
) -> Option<Vec<String>> {
    let n = tokens.len();
    if n < line_count || line_count < 1 {
        return None;
    }
    let token_widths: Vec<f32> = tokens.iter().map(|t| measure_visible_width(t)).collect();
    let mut prefix = vec![0.0; n + 1];
    let mut ends_break = vec![false; n];
    for (i, t) in tokens.iter().enumerate() {
        prefix[i + 1] = prefix[i] + token_widths[i];
        ends_break[i] = t.ends_with(['.', ',', ';', ':', '!', '?', '…', '»', ')', ']', '}']);
    }

    let stride = n + 1;
    let mut dp = vec![f64::INFINITY; (line_count + 1) * stride];
    let mut prev = vec![-1i32; (line_count + 1) * stride];
    dp[0] = 0.0;

    for line_idx in 0..line_count {
        let target = targets.get(line_idx).copied().unwrap_or(1.0) as f64;
        let prune_width = target * 3.5;
        let is_last = line_idx == line_count - 1;
        let row_offset = line_idx * stride;
        let next_row_offset = (line_idx + 1) * stride;
        let max_to = n - (line_count - line_idx - 1);

        for from_idx in line_idx..=n {
            let base_cost = dp[row_offset + from_idx];
            if base_cost.is_infinite() {
                continue;
            }
            let from_p = prefix[from_idx] as f64;
            for to_idx in (from_idx + 1)..=max_to {
                let w = (prefix[to_idx] as f64 - from_p) + 0.45 * (to_idx - from_idx - 1) as f64;
                if w > prune_width && to_idx > from_idx + 1 {
                    break;
                }
                let ratio = (w - target) / target;
                let mut cost = ratio * ratio;
                if !is_last && ends_break[to_idx - 1] {
                    cost = (cost - 0.09).max(0.0);
                }
                let next_cost = base_cost + cost;
                if next_cost < dp[next_row_offset + to_idx] {
                    dp[next_row_offset + to_idx] = next_cost;
                    prev[next_row_offset + to_idx] = from_idx as i32;
                }
            }
        }
    }

    if dp[line_count * stride + n].is_infinite() {
        return None;
    }

    let mut lines = Vec::with_capacity(line_count);
    let mut cursor = n;
    for line_idx in (1..=line_count).rev() {
        let from_idx = prev[line_idx * stride + cursor];
        if from_idx < 0 {
            return None;
        }
        lines.push(tokens[from_idx as usize..cursor].join(" "));
        cursor = from_idx as usize;
    }
    lines.reverse();
    Some(lines)
}

/// Эстетическая оценка силуэта баббла (TypeR Studio scoreSilhouetteAesthetics)
pub fn score_textshaper_silhouette(lengths: &[f32], smoothness_weight: f32, adjacent_slack: f32) -> f32 {
    let line_count = lengths.len();
    let max_len = lengths.iter().copied().fold(0.0f32, f32::max);
    if max_len <= 0.0 || line_count < 2 {
        return 0.0;
    }

    if line_count == 2 {
        let diff = (lengths[0] - lengths[1]).abs() / max_len;
        let excess = (diff - 0.24).max(0.0);
        return excess * excess * smoothness_weight;
    }

    let interior = &lengths[1..line_count - 1];
    let interior_max = interior.iter().copied().fold(0.0f32, f32::max);
    let mut score = 0.0;

    let edge_max = lengths[0].max(lengths[line_count - 1]);
    if edge_max > interior_max {
        let d = (edge_max - interior_max) / max_len;
        score += d * d * 420.0 + 20.0;
    }

    if interior_max > 0.0 {
        for &edge in &[lengths[0], lengths[line_count - 1]] {
            let edge_excess = (edge / interior_max - 0.85).max(0.0);
            score += edge_excess * edge_excess * 900.0;
        }
    }

    let peak_idx = lengths.iter().position(|&l| (l - max_len).abs() < 1e-4).unwrap_or(0);
    for i in 1..=peak_idx {
        let drop = lengths[i - 1] - lengths[i];
        if drop > 0.0 {
            let r = drop / max_len;
            score += r * r * 300.0;
        }
    }
    for i in (peak_idx + 1)..line_count {
        let rise = lengths[i] - lengths[i - 1];
        if rise > 0.0 {
            let r = rise / max_len;
            score += r * r * 300.0;
        }
    }

    let center_idx = (line_count as f32 - 1.0) / 2.0;
    if center_idx > 0.0 {
        let offset = (peak_idx as f32 - center_idx).abs() / center_idx;
        score += offset * offset * 60.0;
    }

    for i in 1..line_count {
        let diff = (lengths[i] - lengths[i - 1]).abs() / max_len;
        let excess = (diff - adjacent_slack).max(0.0);
        if excess > 0.0 {
            score += excess * excess * smoothness_weight;
        }
    }

    score
}

/// Полный расчет штрафов кандидата (TypeR Studio scoreCandidate)
pub fn score_textshaper_candidate(
    lines: &[String],
    hyphen_count: usize,
    target_lines: usize,
    bubble_w: Option<f32>,
    bubble_h: Option<f32>,
    unit_px: f32,
    line_px: f32,
) -> f32 {
    let lengths: Vec<f32> = lines.iter().map(|l| line_visible_width(l)).collect();
    let line_count = lines.len();
    let total_len: f32 = lengths.iter().sum();
    let weights = build_textshaper_weights(line_count, 0.65, 0.0, 0.35);
    let w_sum = weights.iter().sum::<f32>().max(0.001);
    let targets: Vec<f32> = weights.into_iter().map(|w| (total_len * w) / w_sum).collect();
    let max_line_width = 28.0;

    let line_delta = line_count as f32 - target_lines as f32;
    let delta_factor = if line_delta < 0.0 { 3.0 } else { 1.0 };
    let mut score = hyphen_count as f32 * 130.0 + line_delta.abs().powf(1.5) * 16.0 * delta_factor;

    if let (Some(bw), Some(bh)) = (bubble_w, bubble_h) {
        let h_excess = ((line_count as f32 * line_px) / (bh * 0.90) - 1.0).max(0.0);
        if h_excess > 0.0 {
            score += 80.0 + h_excess * h_excess * 900.0;
        }
        let avail_units = (bw * 0.90) / unit_px.max(1.0);
        for &l in &lengths {
            let w_excess = (l / avail_units - 1.0).max(0.0);
            if w_excess > 0.0 {
                score += 60.0 + w_excess * w_excess * 1600.0;
            }
        }
    }

    for (idx, &l) in lengths.iter().enumerate() {
        let tgt = targets.get(idx).copied().unwrap_or(1.0).max(1.0);
        let rel = (l - tgt) / tgt;
        score += rel * rel * 120.0;
        let w_excess = ((l - max_line_width) / max_line_width).max(0.0);
        score += w_excess * w_excess * 320.0;
        if idx < line_count - 1 && lines[idx].ends_with(['.', ',', ';', ':', '!', '?', '…', '»']) {
            score -= 14.0;
        }
    }

    score += score_textshaper_silhouette(&lengths, 160.0, 0.3);
    score
}

/// Генерация разнообразных вариантов TextShapeR по полному алгоритму TypeR Studio
pub fn generate_textshaper_variants(
    text: &str,
    bubble_w: Option<f32>,
    bubble_h: Option<f32>,
    font_size_pt: f32,
    dpi: f32,
    limit: usize,
) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let bound_text = bind_orphan_prepositions(trimmed);
    let tokens: Vec<String> = bound_text.split(' ').filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
    let n = tokens.len();
    if n <= 1 {
        return vec![bound_text];
    }

    let dpi_scale = if dpi.is_finite() && dpi > 0.0 { dpi / 72.0 } else { 1.0 };
    let font_size_px = font_size_pt * dpi_scale;
    let unit_px = (font_size_px * 0.52).max(1.0);
    let line_px = (font_size_px * 1.25).max(1.0);

    let total_units: f32 = tokens.iter().map(|t| measure_visible_width(t)).sum::<f32>() + tokens.len().saturating_sub(1) as f32 * 0.45;

    let (min_lines, max_lines, target_lines) = if let (Some(bw), Some(bh)) = (bubble_w, bubble_h) {
        let avail_units = ((bw * 0.90) / unit_px).max(6.0);
        let est = (total_units / (avail_units * 0.75)).ceil() as usize;
        let aspect = (bh / bw.max(1.0)).clamp(0.25, 3.0);
        let tgt = if aspect >= 1.15 { est.max(((bh / line_px) * 0.7).round() as usize).max(1) } else { est.max(1) };
        let min_l = tgt.saturating_sub(1).max(1).min(n);
        let max_l = (tgt + 2).max(min_l).min(n);
        (min_l, max_l, tgt)
    } else {
        let vol = (total_units / 2.6).sqrt().round() as usize;
        let tgt = vol.clamp(if n >= 3 { 2 } else { 1 }, 6);
        let min_l = if n <= 2 { 1 } else { 2 };
        let max_l = (tgt + 2).clamp(min_l, n.min(8));
        (min_l, max_l, tgt)
    };

    let curves = [0.35f32, 0.45, 0.6, 0.75];
    let shifts = [0.0f32, -0.12, 0.12, -0.2, 0.2];
    let biases = [-1.0f32, 0.0, 1.0];

    let mut map: std::collections::HashMap<String, (f32, usize)> = std::collections::HashMap::new();

    for lc in min_lines..=max_lines {
        for &c in &curves {
            for &s in &shifts {
                for &b in &biases {
                    let targets = build_textshaper_targets(&tokens, lc, c, s, b, 0.35);
                    if let Some(lines) = build_textshaper_candidate_dp(&tokens, lc, &targets) {
                        let candidate_text = lines.join("\n");
                        map.entry(candidate_text).or_insert_with(|| {
                            let score = score_textshaper_candidate(&lines, 0, target_lines, bubble_w, bubble_h, unit_px, line_px);
                            (score, lc)
                        });
                    }
                }
            }
        }
    }

    if map.is_empty() {
        return vec![bound_text];
    }

    let mut sorted: Vec<(String, f32, usize)> = map.into_iter().map(|(t, (sc, lc))| (t, sc, lc)).collect();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    // TypeR Studio Diversity Guarantee: 1 лучший кандидат для каждой высоты строк, затем заполнение оставшихся слотов
    let mut picked = Vec::new();
    let mut seen_texts = std::collections::HashSet::new();
    let mut seen_lcs = std::collections::HashSet::new();

    for (t, _, lc) in &sorted {
        if !seen_lcs.contains(lc) && picked.len() < limit {
            picked.push(t.clone());
            seen_texts.insert(t.clone());
            seen_lcs.insert(*lc);
        }
    }

    for (t, _, _) in &sorted {
        if !seen_texts.contains(t) && picked.len() < limit {
            picked.push(t.clone());
            seen_texts.insert(t.clone());
        }
    }

    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_orphan_prepositions() {
        let input = "СВОБОДОЙ, Я В ИТОГЕ";
        let res = bind_orphan_prepositions(input);
        assert!(res.contains(&format!("Я{}", NBSP)));
        assert!(res.contains(&format!("В{}", NBSP)));

        let particles = "что же это было бы или нет ли";
        let p_res = bind_orphan_prepositions(particles);
        assert!(p_res.contains(&format!("{}же", NBSP)));
        assert!(p_res.contains(&format!("{}бы", NBSP)));
        assert!(p_res.contains(&format!("{}ли", NBSP)));
    }

    #[test]
    fn conservative_hyphenation_only_breaks_long_words() {
        assert!(hyphenation_point("wonderful").is_some());
        assert!(hyphenation_point("extraordinary").is_some());
        assert!(hyphenation_point("cat").is_none());
        assert!(hyphenation_point("?!").is_none());

        let (left, right) = hyphenate_token("wonderful", 1000.0, 20.0, 1.0).unwrap();
        assert!(left.ends_with('-'));
        assert!(!right.is_empty());
        assert_eq!(format!("{}{right}", left.trim_end_matches('-')), "wonderful");

        let wrapped = hyphenate_long_words_with_breaks("a wonderful word", 70.0, 20.0, 1.0);
        assert!(wrapped.contains("-\n"), "a long word must break after its hyphen: {wrapped}");
    }

    #[test]
    fn test_knuth_plass_bubble_wrap_shape() {
        let text = "Это длинный тестовый текст для проверки солвера Кнута-Пласса в баллонах манги";
        let wrapped = knuth_plass_bubble_wrap(text, 180.0, 200.0, 14.0, 72.0, 100.0);
        let lines: Vec<&str> = wrapped.split('\n').collect();
        assert!(lines.len() >= 2);
        // Ни одна строка не должна быть пустой
        for line in &lines {
            assert!(!line.trim().is_empty());
        }

    }

    /// Phase 1: a long Russian or English word in a narrow balloon breaks by syllables with a
    /// visible hyphen instead of running past the edge.
    #[test]
    fn language_hyphenation_breaks_words_with_a_hyphen() {
        let russian = hyphenate_long_words_with_breaks("НЕОБХОДИМОСТЬ", 150.0, 26.0, 1.0);
        assert!(russian.contains('-'), "{russian}");
        assert!(russian.contains(char::from(10)), "the word is split into lines: {russian}");
        let english = hyphenate_long_words_with_breaks("EXTRAORDINARY", 150.0, 26.0, 1.0);
        assert!(english.contains(char::from(10)), "{english}");
    }

    #[test]
    fn compound_words_with_hyphen_do_not_produce_double_hyphens() {
        let compound = "какой-то";
        let candidates = hyphenation_candidates(compound);
        assert!(!candidates.is_empty());
        let (left, right) = hyphenate_token(compound, 1000.0, 20.0, 1.0).unwrap();
        assert_eq!(left, "какой-");
        assert_eq!(right, "то");
        assert!(!left.ends_with("--"));
    }

    #[test]
    fn multi_hyphenation_breaks_very_long_words_iteratively() {
        let very_long = "Rindfleischetikettierungsüberwachungsaufgabenübertragungsgesetz";
        let wrapped = knuth_plass_bubble_wrap(very_long, 120.0, 120.0, 14.0, 72.0, 100.0);
        let lines: Vec<&str> = wrapped.split('\n').collect();
        assert!(lines.len() >= 2, "must break into multiple lines: {wrapped}");
        for line in &lines {
            assert!(!line.contains("--"));
        }
    }

    #[test]
    fn test_bubble_shape_wrapping() {
        
        let text = "Это длинный текст для проверки вертикального овала, который должен быть узким и высоким.";
        let result = knuth_plass_bubble_wrap_ext(text, 100.0, 300.0, 12.0, 72.0, 100.0, true);
        assert!(result.contains('\n'));
        
        let text2 = "Горизонтальный овал шире, чем он высок, поэтому текст должен распределяться по ширине.";
        let result2 = knuth_plass_bubble_wrap_ext(text2, 300.0, 100.0, 12.0, 72.0, 100.0, true);
        assert!(result2.contains('\n'));
        
        let text3 = "Прямоугольник имеет четкие границы, текст должен заполнять его равномерно.";
        let result3 = knuth_plass_bubble_wrap_ext(text3, 200.0, 150.0, 12.0, 72.0, 100.0, true);
        assert!(result3.contains('\n'));
        
        let text4 = "Ромбическая форма требует особого внимания к центральной части строки.";
        let result4 = knuth_plass_bubble_wrap_ext(text4, 150.0, 150.0, 12.0, 72.0, 100.0, true);
        assert!(result4.contains('\n'));
        
        let text5 = "Круглый овал симметричен, текст должен быть сбалансирован.";
        let result5 = knuth_plass_bubble_wrap_ext(text5, 120.0, 120.0, 12.0, 72.0, 100.0, true);
        assert!(result5.contains('\n'));
    }
}
