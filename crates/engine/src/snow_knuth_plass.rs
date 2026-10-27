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
        c if c >= '\u{3000}' && c <= '\u{9FFF}' => 1.0, // CJK
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

/// Расширенная версия солвера Кнута—Пласса с поддержкой оптической компенсации знаков в строках (висячая пунктуация)
pub fn knuth_plass_bubble_wrap_ext(
    text: &str,
    bubble_width: f32,
    _bubble_height: f32,
    font_size_pt: f32,
    dpi: f32,
    horizontal_scale_pct: f32,
    hanging_punctuation: bool,
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

    // Максимальная безопасная ширина строки внутри баббла (с учетом внутренних отступов)
    let max_safe_width = (bubble_width * 0.85).max(font_size_px * 2.0);

    // Токенизация: разделяем по обычным пробелам (токены с NBSP внутри остаются едиными блоками!)
    let tokens: Vec<&str> = bound_text.split(' ').filter(|s| !s.is_empty()).collect();
    if tokens.len() <= 1 {
        return bound_text;
    }

    let space_width = char_width_factor(' ') * font_size_px * h_scale;
    let token_widths: Vec<f32> = tokens.iter().map(|&t| measure_token_width(t, font_size_px, h_scale)).collect();
    let total_text_width: f32 = token_widths.iter().sum::<f32>() + (tokens.len() - 1) as f32 * space_width;

    // Если весь текст комфортно помещается в одну строку
    if total_text_width <= max_safe_width * 0.80 {
        return bound_text;
    }

    // Оцениваем диапазон количества строк для перебора
    let est_lines = (total_text_width / (max_safe_width * 0.72)).round() as usize;
    let min_lines = est_lines.max(2).saturating_sub(1).max(2);
    let max_lines = (est_lines + 2).min(tokens.len()).max(min_lines);

    let mut best_demerits = f64::MAX;
    let mut best_break_indices: Vec<usize> = Vec::new();

    // Профиль целевой ширины для эллипса/ромба манга-баллона:
    // Центральные строки имеют максимальную ширину, верхние и нижние — более компактные.
    let target_width_for_line = |line_idx: usize, total_k: usize| -> f32 {
        if total_k <= 1 {
            return max_safe_width;
        }
        let t = (line_idx as f32 + 0.5) / total_k as f32; // 0.0 .. 1.0
        let y = (t * 2.0 - 1.0).clamp(-1.0, 1.0); // -1.0 .. +1.0
        // Профиль эллипса: W(y) = W_max * sqrt(1 - y^2)
        let ellipse_factor = (1.0 - y * y).max(0.0).sqrt();
        // Сглаженный профиль (минимум 60% ширины по краям, 95% в центре)
        max_safe_width * (0.60 + 0.35 * ellipse_factor)
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
                        punctuation_compensation_px(tokens[j - 1], font_size_px, h_scale)
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

        let cand_demerits = dp[n][k].0;
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
        lines.push(tokens[start..end].join(" "));
    }
    if let Some(&last_start) = best_break_indices.last() {
        if last_start < tokens.len() {
            lines.push(tokens[last_start..].join(" "));
        }
    }

    lines.join("\n")
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
}
