//! Language detection and hyphenation for the TypeR typesetter.
//!
//! A port of TypeR Studio's `hyphenation.js`: the text's own words and signature letters decide
//! the language; Cyrillic resolves to Russian or Ukrainian, scripts that are not hyphenated in
//! manga lettering (CJK, Hangul, Arabic, Thai) stay unbroken. Break points are character indices
//! into the word, so callers can map them to bytes without guessing UTF-8 boundaries.

/// Languages the detector and the hyphenator know, in TypeR's order.
pub const LANGUAGES: [&str; 9] = ["de", "en", "es", "fr", "it", "pt", "ru", "tr", "uk"];

const FR_WORDS: &str = "je tu il elle nous vous ils elles on moi toi lui le les des du une et est suis es sont ai as ont pas ne ce cette ces ça c'est j'ai qu'il qu'on qu'elle mais pour dans avec sur oui non qui bien très tout ici alors quoi comment pourquoi merci bonjour salut hein mon ton son ma ta sa mes tes ses a me te se au aux";
const EN_WORDS: &str = "i you he she we they it him her us them the an and is am are was were be been do does did don't i'm you're it's that's have has had will would can could should not yes what why how who where when this that these those my your his our their of to in for with at but so just";
const ES_WORDS: &str = "yo tú él ella nosotros ellos ellas el los las y soy eres estoy está están qué por para pero con sin sí muy más también aquí ahora porque cómo gracias hola mi tu su mis esto eso nada todo";
const PT_WORDS: &str = "eu você ele ela nós eles elas os um uma é sou são estou está não sim por para mas sem muito mais também aqui agora porque como obrigado olá meu minha seu sua isso nada tudo";
const DE_WORDS: &str = "ich du er sie wir ihr der die das den dem ein eine einen und ist bin bist sind war nicht ja nein was warum wie wer wo mit für auf aber auch noch schon hier jetzt danke mein dein sein";
const IT_WORDS: &str = "io lei noi voi loro lo gli è sono sei siamo ho hai sì che per ma con senza molto più anche qui ora perché come grazie ciao mio tuo suo questo quello niente tutto";
const TR_WORDS: &str = "ben sen biz siz onlar bir ve bu şu için ama ile çok daha evet hayır ne neden nasıl kim var yok değil benim senin teşekkürler merhaba";

/// The everyday-word list of `lang` (space separated), used to score a text's language.
fn words_of(lang: &str) -> Option<&'static str> {
    match lang {
        "fr" => Some(FR_WORDS),
        "en" => Some(EN_WORDS),
        "es" => Some(ES_WORDS),
        "pt" => Some(PT_WORDS),
        "de" => Some(DE_WORDS),
        "it" => Some(IT_WORDS),
        "tr" => Some(TR_WORDS),
        _ => None,
    }
}

/// Letters whose presence alone hints at a language (weighted 1.5 per occurrence).
fn signature_letters(lang: &str) -> &'static str {
    match lang {
        "fr" => "êàèùûîïœç",
        "es" => "ñ¿¡",
        "pt" => "ãõ",
        "de" => "äöüß",
        "it" => "ìò",
        "tr" => "ğış",
        _ => "",
    }
}

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

fn is_ukrainian_letter(c: char) -> bool {
    matches!(c, 'і' | 'ї' | 'є' | 'ґ' | 'І' | 'Ї' | 'Є' | 'Ґ')
}

fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{00FF}').contains(&c) || ('\u{0100}'..='\u{017F}').contains(&c)
}

fn is_unbroken_script(c: char) -> bool {
    ('\u{0590}'..='\u{08FF}').contains(&c) // Arabic and neighbours
        || ('\u{0E00}'..='\u{0E7F}').contains(&c) // Thai
        || ('\u{3040}'..='\u{30FF}').contains(&c) // Hiragana, Katakana
        || ('\u{3400}'..='\u{9FFF}').contains(&c) // CJK
        || ('\u{AC00}'..='\u{D7AF}').contains(&c) // Hangul
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}


/// Lowercases the text and splits it into Latin words (apostrophes stay inside a word).
fn latin_words(text: &str) -> Vec<String> {
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    let mut words = Vec::new();
    let mut current = String::new();
    for c in lower.chars() {
        if is_latin_letter(c) || is_apostrophe(c) {
            current.push(c);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The language the text is written in, from its everyday words and signature letters; `None`
/// when no language wins clearly (a short label, a name, a language this module does not know).
pub fn detect_language(text: &str) -> Option<&'static str> {
    if text.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let words = latin_words(text);
    let mut best: Option<&'static str> = None;
    let mut best_score = 0.0f64;
    let mut runner_up = 0.0f64;
    for lang in LANGUAGES {
        let mut score = 0.0f64;
        if let Some(list) = words_of(lang) {
            for word in &words {
                if list.split(' ').any(|known| known == word) {
                    score += 1.0;
                }
            }
        }
        let signature = signature_letters(lang);
        if !signature.is_empty() {
            score += 1.5 * lower.chars().filter(|c| signature.contains(*c)).count() as f64;
        }
        if score > best_score {
            runner_up = best_score;
            best_score = score;
            best = Some(lang);
        } else if score > runner_up {
            runner_up = score;
        }
    }
    if best_score > 0.0 && best_score > runner_up { best } else { None }
}

/// The language a text hyphenates in: `None` for scripts that do not hyphenate, `uk`/`ru` by the
/// alphabet, the detected Latin language otherwise (English when the words are indecisive).
pub fn resolve_language(text: &str) -> Option<&'static str> {
    if text.chars().any(is_unbroken_script) {
        return None;
    }
    if text.chars().any(is_cyrillic) {
        return Some(if text.chars().any(is_ukrainian_letter) { "uk" } else { "ru" });
    }
    if !text.chars().any(is_latin_letter) {
        return None;
    }
    detect_language(text).or(Some("en"))
}

const RU_VOWELS: &str = "аеёиоуыэюя";
const UK_VOWELS: &str = "аеєиіїоуюя";

fn lower_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_vowel(c: char) -> bool {
    RU_VOWELS.contains(lower_char(c))
}

fn is_vowel_lang(c: char, lang: &str) -> bool {
    let c = lower_char(c);
    if lang == "uk" { UK_VOWELS.contains(c) } else { is_vowel(c) }
}

/// Consonant clusters that may open a Russian/Ukrainian syllable as a whole.
const RU_ONSETS: [&str; 30] = [
    "бл", "бр", "вл", "вр", "гл", "гн", "гр", "др", "зл", "зн", "кл", "кн", "кр", "мн", "пл", "пр", "ск", "сл", "см", "сн", "сп", "ст",
    "св", "тв", "тр", "фл", "фр", "хл", "хр", "шк",
];

const RU_ONSETS_3: [&str; 6] = ["стр", "спр", "скр", "взр", "сбл", "здр"];

fn is_onset(cluster: &[char]) -> bool {
    if cluster.len() < 2 || cluster.len() > 3 {
        return false;
    }
    let text: String = cluster.iter().map(|c| lower_char(*c)).collect();
    if cluster.len() == 2 {
        RU_ONSETS.contains(&text.as_str())
    } else {
        RU_ONSETS_3.contains(&text.as_str())
    }
}

/// Russian/Ukrainian syllabification: a single consonant between vowels opens the next syllable
/// (cut before it), while a legal onset cluster travels together.
fn syllabic_ru(body: &[char], lang: &str) -> Vec<usize> {
    let mut vowels: Vec<usize> = Vec::new();
    for (index, c) in body.iter().enumerate() {
        if is_vowel_lang(*c, lang) {
            vowels.push(index);
        }
    }
    let mut points = Vec::new();
    for pair in vowels.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if right <= left + 1 {
            continue;
        }
        let cluster = &body[left + 1..right];
        let cut = if cluster.len() == 1 {
            right - 1
        } else if is_onset(cluster) {
            left + 1
        } else {
            left + 2
        };
        points.push(cut);
    }
    points
}

/// French consonant clusters that legally open a syllable.
const FRENCH_ONSETS: [&str; 26] = [
    "bl", "br", "ch", "cl", "cr", "dr", "fl", "fr", "gl", "gn", "gr", "kl", "kr", "ph", "pl", "pr", "rh", "th", "tr", "vr", "chl", "chr",
    "phl", "phr", "thr", "gn",
];

const FRENCH_VOWELS: &str = "aeiouyàâäéèêëîïôöùûüœ";

fn is_french_vowel(c: char) -> bool {
    FRENCH_VOWELS.contains(lower_char(c))
}

fn is_french_letter(c: char) -> bool {
    let lower = c.to_ascii_lowercase();
    lower.is_ascii_alphabetic()
        || FRENCH_VOWELS.contains(lower)
        || matches!(lower, 'ç' | 'ñ')
}

fn is_french_consonant(c: char) -> bool {
    is_french_letter(c) && !is_french_vowel(c)
}

fn has_french_vowel(chars: &[char]) -> bool {
    chars.iter().any(|c| is_french_vowel(*c))
}

/// TypeR's French syllable penalty (lower is a better break); `INFINITY` forbids the break.
fn french_penalty(body: &[char], index: usize) -> f64 {
    if index == 0 || index >= body.len() {
        return f64::INFINITY;
    }
    let lower: Vec<char> = body.iter().map(|c| c.to_ascii_lowercase()).collect();
    if !has_french_vowel(&lower[..index]) || !has_french_vowel(&lower[index..]) {
        return f64::INFINITY;
    }
    let prev = lower[index - 1];
    let current = lower[index];
    if is_french_vowel(prev) && is_french_vowel(current) {
        return 7.0;
    }
    if is_french_consonant(prev) && is_french_vowel(current) {
        return 16.0;
    }
    if current == 'x' && lower.get(index + 1).is_some_and(|c| is_french_vowel(*c)) {
        return 12.0;
    }
    if is_french_consonant(prev) && FRENCH_ONSETS.contains(&format!("{prev}{current}").as_str()) {
        return 7.0;
    }
    let mut start = index;
    while start > 0 && is_french_consonant(lower[start - 1]) {
        start -= 1;
    }
    let mut end = index;
    while end < lower.len() && is_french_consonant(lower[end]) {
        end += 1;
    }
    let onset: String = lower[index..end].iter().collect();
    let coda = index - start;
    let rest: String = lower[end..].iter().collect();
    if rest == "e" || rest == "es" {
        return 6.0;
    }
    if onset == "h" {
        return 6.0;
    }
    if coda >= 2 {
        return 10.0;
    }
    if onset.chars().count() == 1 {
        return if coda == 0 { 0.0 } else { 1.0 };
    }
    if FRENCH_ONSETS.contains(&onset.as_str()) {
        return if coda == 0 { 0.5 } else { 1.5 };
    }
    10.0
}

fn syllabic_fr(body: &[char]) -> Vec<usize> {
    let mut points = Vec::new();
    if body.len() < 7 {
        return points;
    }
    let lowered: Vec<char> = body.iter().map(|c| lower_char(*c)).collect();
    for index in 3..=body.len().saturating_sub(3) {
        if french_penalty(&lowered, index) < 3.0 {
            points.push(index);
        }
    }
    points
}

/// Shared Latin heuristic: after a vowel group, before the following consonant, with at least
/// two letters on both sides. German keeps its usual cluster openings together.
fn generic_latin(body: &[char], lang: &str) -> Vec<usize> {
    let mut points = Vec::new();
    if body.len() < 4 {
        return points;
    }
    for index in 2..body.len().saturating_sub(1) {
        let prev = body[index - 1].to_ascii_lowercase();
        let current = body[index].to_ascii_lowercase();
        if !is_french_vowel(prev) || is_french_vowel(current) {
            continue;
        }
        if !has_french_vowel(&body[index..]) {
            continue;
        }
        let rest: String = body[index..].iter().map(|c| c.to_ascii_lowercase()).collect();
        let blocked = match lang {
            "de" => ["st", "sp", "ck", "ss", "tz", "pf", "sch"].iter().any(|cluster| rest.starts_with(cluster)),
            "en" => ["ing", "ed", "es"].iter().any(|suffix| rest == *suffix),
            _ => false,
        };
        if blocked {
            continue;
        }
        points.push(index);
    }
    points
}

/// Character indices where `word` may be split, for `lang`.
///
/// Words shorter than four letters, non-letter words, words in unbroken scripts and words with an
/// inner apostrophe outside French/Italian are never split.
pub fn hyphen_break_points(word: &str, lang: &'static str) -> Vec<usize> {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 4 || !chars.iter().all(|c| c.is_alphabetic() || is_apostrophe(*c)) {
        return Vec::new();
    }
    let mut lead = 0;
    while lead < chars.len() && is_apostrophe(chars[lead]) {
        lead += 1;
    }
    let mut end = chars.len();
    while end > lead && is_apostrophe(chars[end - 1]) {
        end -= 1;
    }
    let Some(body) = chars.get(lead..end) else { return Vec::new() };
    if body.len() < 4 {
        return Vec::new();
    }
    let elided = body.iter().any(|c| is_apostrophe(*c));
    if elided && !matches!(lang, "fr" | "it") {
        return Vec::new();
    }
    let body = body.to_vec();
    let points = match lang {
        "ru" | "uk" => syllabic_ru(&body, lang),
        "fr" => syllabic_fr(&body),
        _ => generic_latin(&body, lang),
    };
    let mut out: Vec<usize> = points
        .into_iter()
        .map(|index| index + lead)
        .filter(|index| *index >= 2 && chars.len().saturating_sub(*index) >= 2 && *index < chars.len())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// [`hyphen_break_points`] as byte offsets into `word`.
pub fn byte_break_points(word: &str, lang: &'static str) -> Vec<usize> {
    hyphen_break_points(word, lang).into_iter().filter_map(|index| word.char_indices().nth(index).map(|(byte, _)| byte)).collect()
}

/// Character indices where `word` may split; `lang` or the language the word itself suggests.
pub fn hyphenate(word: &str, lang: Option<&'static str>) -> Vec<usize> {
    match lang.or_else(|| resolve_language(word)) {
        Some(language) => hyphen_break_points(word, language),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_latin_languages_by_their_everyday_words() {
        assert_eq!(detect_language("il était une fois"), Some("fr"));
        assert_eq!(detect_language("the quick brown fox"), Some("en"));
        assert_eq!(detect_language("ich habe das"), Some("de"));
        assert_eq!(detect_language("yo estoy muy aquí"), Some("es"));
        assert_eq!(detect_language("ben bir şu için"), Some("tr"));
        assert_eq!(detect_language("Привет как дела"), None, "Cyrillic is resolved by its alphabet");
        assert_eq!(detect_language(""), None);
    }

    #[test]
    fn resolves_scripts_and_cyrillic_alphabets() {
        assert_eq!(resolve_language("Привет"), Some("ru"));
        assert_eq!(resolve_language("Слава Україні"), Some("uk"));
        assert_eq!(resolve_language("ひらがなカタカナ"), None);
        assert_eq!(resolve_language("猫"), None);
        assert_eq!(resolve_language("1234"), None);
        assert_eq!(resolve_language("hello world"), Some("en"));
    }

    #[test]
    fn russian_words_split_at_syllable_boundaries() {
        let points = hyphen_break_points("НЕОБХОДИМОСТЬ", "ru");
        assert!(points.len() >= 3, "{points:?}");
        let len = "НЕОБХОДИМОСТЬ".chars().count();
        assert!(points.iter().all(|p| (2..=len - 2).contains(p)), "{points:?}");
        assert!(points.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn english_long_words_offer_breaks() {
        let points = hyphen_break_points("extraordinary", "en");
        assert!(!points.is_empty(), "no points");
        assert!(points.iter().all(|p| (2..=11).contains(p)), "{points:?}");
    }

    #[test]
    fn short_words_and_unbroken_scripts_never_split() {
        assert!(hyphen_break_points("кот", "ru").is_empty());
        assert!(hyphenate("猫猫", None).is_empty());
        assert!(hyphenate("a1b2", None).is_empty());
    }

    #[test]
    fn inner_apostrophes_split_only_in_french_or_italian() {
        assert!(hyphen_break_points("aujourd'hui", "en").is_empty(), "English keeps it whole");
        let italian = hyphen_break_points("dell'arte", "it");
        assert!(italian.iter().all(|p| *p >= 2 && *p <= 8), "{italian:?}");
    }

    #[test]
    fn byte_points_land_on_character_boundaries() {
        let word = "НЕОБХОДИМОСТЬ";
        for point in hyphen_break_points(word, "ru") {
            let byte = byte_break_points(word, "ru");
            assert!(byte.iter().any(|b| word.is_char_boundary(*b)), "{point}");
        }
        for byte in byte_break_points(word, "ru") {
            assert!(word.is_char_boundary(byte));
        }
    }

    #[test]
    fn hyphenate_uses_the_resolved_language() {
        assert!(!hyphenate("НЕОБХОДИМОСТЬ", None).is_empty());
        assert!(hyphenate("abcdef", None).len() <= 3);
        assert!(hyphenate("word", Some("ru")).is_empty(), "no vowel pairs in \"word\"");
    }
}
