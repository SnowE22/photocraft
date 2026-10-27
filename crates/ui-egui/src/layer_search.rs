//! Global layer & text search across every open document tab (Edit › Find in Open Pages…).
//!
//! A floating palette (Spotlight / VS Code Quick Open style) that searches every layer name and
//! every text-layer content across all open documents, with prefix filters (`@` text only, `#`
//! groups only, `!` hidden only), fuzzy matching and highlighted hits. Wrong keyboard layouts are
//! handled too: the query is also tried re-mapped through the QWERTY ↔ ЙЦУКЕН layout (`ghbdtn`
//! finds «привет», `игииду` finds "Bubble"), so a typesetter who forgot to switch still finds what
//! they meant. Activating a result switches to its document, selects the layer, expands every
//! collapsed parent group and centres the canvas viewport on the layer.
//!
//! See [`show`] for the UI and [`activate`] for the jump-and-focus action.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, TextFormat, vec2};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, comps::position_bounds};
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons};

/// Width of the floating palette (px). Wide enough for "[01.png] [T] SFX Boom mic" rows, with the
/// matched snippet still readable to the right.
const WIDTH: f32 = 580.0;
/// How many results the list shows at most (matches the command palette's cap).
const CAP: usize = 12;
/// Vertical offset of the palette from the top of the screen (px), so it sits clear of any toolbar.
const TOP_OFFSET: f32 = 100.0;
/// Maximum chars on each side of a text hit's match point, for the snippet shown in the row.
const SNIPPET_RADIUS: usize = 40;

/// One search hit. `name_matches` / `text_matches` carry the byte offsets that the query matched, so
/// the UI can highlight them. The score picks the order: substring beats fuzzy, active-document
/// beats the rest, and shorter names break ties.
#[derive(Clone)]
struct Hit {
    /// 0-based index of the document in `app.session.documents()`.
    doc: usize,
    /// The tab title (the document's name, e.g. `01.png`).
    doc_name: String,
    /// The layer's id, looked up again by id when the hit is activated.
    layer: LayerId,
    /// The layer's name, as a single-line label.
    label: String,
    /// The text content of a text layer with line breaks flattened to spaces (for display), if any.
    text: Option<String>,
    /// Layer-type badge: `[T]` text, `[G]` group, `[Px]` raster, …
    badge: &'static str,
    /// Whether the layer is hidden (`!layer.visible`): shown dimmed.
    hidden: bool,
    /// Whether the layer belongs to the active document: those sort first.
    active_doc: bool,
    /// The byte offsets in `label` the query matched, for highlighting.
    name_matches: Vec<usize>,
    /// The byte offsets in `text` (the flattened text content) the query matched, for highlighting.
    text_matches: Vec<usize>,
    /// Score (higher is better); see [`score_hit`].
    score: i32,
}

/// The query, with its leading `@` / `#` / `!` filters split off and the body normalized.
struct Query {
    /// Show only text layers.
    text_only: bool,
    /// Show only groups.
    groups_only: bool,
    /// Show only hidden layers.
    hidden_only: bool,
    /// Lower-cased, whitespace-trimmed body. Empty means "list everything that passes the filters".
    body: String,
    /// The body and its wrong-layout re-mappings, as `(text, rank_penalty)` pairs: the body itself
    /// first (no penalty), then every [`layout_variants`] entry at [`TRANSLIT_PENALTY`]. Substring
    /// matching runs over all of them, so `ghbdtn` also finds «привет» and back.
    variants: Vec<(String, i32)>,
}

/// How far a translit (wrong-layout) match ranks below the same match for the raw query: a real
/// match always wins over a re-mapped one, but a translit substring still beats a raw fuzzy one.
const TRANSLIT_PENALTY: i32 = 25;

/// QWERTY ↔ ЙЦУКЕН wrong-layout pairs as `(latin, cyrillic)`. A typesetter who forgot to switch
/// layouts types `ghbdtn` when they mean «привет» (and `игииду` when they mean "Bubble"), so the
/// query is also tried re-mapped through this table. The punctuation keys that carry Cyrillic
/// letters on ЙЦУКЕН (`х`, `ъ`, `ж`, `э`, `б`, `ю`, `ё`) are included.
const LAYOUT_PAIRS: &[(char, char)] = &[
    ('q', 'й'), ('w', 'ц'), ('e', 'у'), ('r', 'к'), ('t', 'е'), ('y', 'н'),
    ('u', 'г'), ('i', 'ш'), ('o', 'щ'), ('p', 'з'), ('[', 'х'), (']', 'ъ'),
    ('a', 'ф'), ('s', 'ы'), ('d', 'в'), ('f', 'а'), ('g', 'п'), ('h', 'р'),
    ('j', 'о'), ('k', 'л'), ('l', 'д'), (';', 'ж'), ('\'', 'э'),
    ('z', 'я'), ('x', 'ч'), ('c', 'с'), ('v', 'м'), ('b', 'и'), ('n', 'т'),
    ('m', 'ь'), (',', 'б'), ('.', 'ю'), ('`', 'ё'),
];

/// Re-map each char of `text` through the keyboard layout: QWERTY → ЙЦУКЕН when `to_cyr`, the
/// reverse otherwise. Chars with no mapping (digits, spaces, other scripts) pass through.
fn remap_layout(text: &str, to_cyr: bool) -> String {
    text.chars()
        .map(|c| {
            for &(latin, cyr) in LAYOUT_PAIRS {
                if to_cyr && c == latin {
                    return cyr;
                }
                if !to_cyr && c == cyr {
                    return latin;
                }
            }
            c
        })
        .collect()
}

/// The query re-mapped through the keyboard layout(s): a latin body also tries cyrillic, a
/// cyrillic body also tries latin (see [`LAYOUT_PAIRS`]); a mixed body tries both directions. A
/// body with no mappable letters (digits, punctuation, other scripts) has no variants.
fn layout_variants(body: &str) -> Vec<String> {
    let has_latin = body.chars().any(|c| c.is_ascii_alphabetic());
    let has_cyr = body
        .chars()
        .any(|c| matches!(c, '\u{0410}'..='\u{044F}' | '\u{0401}' | '\u{0451}'));
    let mut out = Vec::with_capacity(2);
    if has_latin {
        out.push(remap_layout(body, true));
    }
    if has_cyr {
        out.push(remap_layout(body, false));
    }
    out
}

impl Query {
    /// Parse the user input: peel leading `@`/`#`/`!` (in any order, one of each at most), then trim
    /// and lower-case the rest.
    fn parse(raw: &str) -> Self {
        let mut text_only = false;
        let mut groups_only = false;
        let mut hidden_only = false;
        let mut s = raw.trim_start();
        // Consume prefix flags one at a time, so `@#` and `#@` parse the same way.
        loop {
            match s.chars().next() {
                Some('@') => {
                    text_only = true;
                    s = &s[1..];
                }
                Some('#') => {
                    groups_only = true;
                    s = &s[1..];
                }
                Some('!') => {
                    hidden_only = true;
                    s = &s[1..];
                }
                _ => break,
            }
            s = s.trim_start();
        }
        let body = s.to_lowercase();
        let mut variants: Vec<(String, i32)> = Vec::with_capacity(3);
        variants.push((body.clone(), 0));
        variants.extend(layout_variants(&body).into_iter().map(|v| (v, TRANSLIT_PENALTY)));
        Self {
            text_only,
            groups_only,
            hidden_only,
            body,
            variants,
        }
    }
}

/// Flatten line breaks (and other whitespace runs) into single spaces so a multi-line manga
/// dialogue bubble matches a single-line query. The original text is kept for display separately.
fn flatten_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    for c in s.chars() {
        // `\r` and `\n` are whitespace too, so a single `is_whitespace` check covers them.
        if c.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(c);
            last_was_space = false;
        }
    }
    out.trim().to_string()
}

/// Lower-cased form of a layer's name (the comparison key; the display form keeps its case).
fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Substring search: returns the byte offsets of every char of `query` in `text` (both lower-cased),
/// or `None` when `query` is not a contiguous substring. An empty query matches at offset 0.
fn substring_matches(query: &str, text: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return Some(Vec::new());
    }
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = text.chars().collect();
    if q.len() > t.len() {
        return None;
    }
    for window in 0..=t.len().saturating_sub(q.len()) {
        if t[window..window + q.len()] == q[..] {
            // Map back to byte offsets in the lower-cased string.
            let mut byte_offs = Vec::with_capacity(q.len());
            let mut at = text
                .char_indices()
                .nth(window)
                .map(|(b, _)| b)
                .unwrap_or(0);
            for c in &q {
                byte_offs.push(at);
                at += c.len_utf8();
            }
            return Some(byte_offs);
        }
    }
    None
}

/// Fuzzy subsequence match: returns the byte offsets of the matched chars in `text`, and a score,
/// or `None` if the query is not a subsequence. Mirrors [`crate::palette::fuzzy_score`] but also
/// reports offsets so the UI can highlight the matched chars.
fn fuzzy_matches(query: &str, text: &str) -> Option<(Vec<usize>, i32)> {
    if query.is_empty() {
        return Some((Vec::new(), 0));
    }
    let t: Vec<char> = text.chars().collect();
    // Pre-compute byte offset of each char index in `text`.
    let byte_offs: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last: Option<usize> = None;
    let mut matched: Vec<usize> = Vec::with_capacity(query.chars().count());
    for qc in query.chars() {
        let mut found = false;
        while ti < t.len() {
            if t[ti] == qc {
                score += if last == Some(ti.wrapping_sub(1)) { 8 } else { 1 };
                if ti == 0 || t[ti - 1] == ' ' {
                    score += 5;
                }
                matched.push(byte_offs[ti]);
                last = Some(ti);
                ti += 1;
                found = true;
                break;
            }
            ti += 1;
        }
        if !found {
            return None;
        }
    }
    Some((matched, score - (t.len() as i32 / 8)))
}

/// How much a shorter layer name is worth within one rank tier: a substring of "Lamp" is a more
/// specific answer to "la" than a substring of "Ladder" (like VS Code's quick open). Clamped to
/// stay below the gap between tiers, so it can never promote a hit across tiers.
fn specificity(label_lower: &str) -> i32 {
    99 - (label_lower.len() as i32).min(99)
}

/// The first variant that is a contiguous substring of `text`, preferring the raw query over its
/// translit variants (ties are impossible: only the re-mapped variants carry a penalty). Returns
/// the byte offsets of the matched chars and the variant's rank penalty.
fn best_substring(variants: &[(String, i32)], text: &str) -> Option<(Vec<usize>, i32)> {
    variants
        .iter()
        .filter_map(|(v, penalty)| substring_matches(v, text).map(|offs| (offs, *penalty)))
        .min_by_key(|&(_, penalty)| penalty)
}

/// Score and pick the best match for one layer. Tries a substring match against the name and (for
/// text layers) the flattened text content — for the query and its wrong-layout variants — then
/// falls back to fuzzy, against the query itself only. An empty body returns a baseline hit (no
/// highlighting) so the palette can list every layer. Rank tiers, best first: name substring,
/// text substring, name fuzzy, text fuzzy; a translit match ranks [`TRANSLIT_PENALTY`] below the
/// same match for the raw query.
fn score_hit(query: &Query, label_lower: &str, text_lower: Option<&str>) -> Option<(Vec<usize>, Vec<usize>, i32)> {
    if query.body.is_empty() {
        return Some((Vec::new(), Vec::new(), 0));
    }
    // Substring in the name beats everything else.
    if let Some((offs, penalty)) = best_substring(&query.variants, label_lower) {
        return Some((offs, Vec::new(), 1000 + specificity(label_lower) - penalty));
    }
    // Then a substring in the text content.
    if let Some(text_lower) = text_lower
        && let Some((offs, penalty)) = best_substring(&query.variants, text_lower)
    {
        return Some((Vec::new(), offs, 900 + specificity(label_lower) - penalty));
    }
    // Then a fuzzy match in the name.
    if let Some((offs, s)) = fuzzy_matches(&query.body, label_lower) {
        return Some((offs, Vec::new(), 100 + s));
    }
    // Then a fuzzy match in the text content.
    if let Some(text_lower) = text_lower
        && let Some((offs, s)) = fuzzy_matches(&query.body, text_lower)
    {
        return Some((Vec::new(), offs, s));
    }
    None
}

/// Collect every layer across every open document that passes the query's filters and matches its
/// body (substring or fuzzy). The active document's hits are flagged so they sort first.
fn collect_hits(app: &PhotocraftApp, query: &Query) -> Vec<Hit> {
    let active_idx = app.session.active_index();
    let docs = app.session.documents();
    let mut out: Vec<Hit> = Vec::new();
    for (di, st) in docs.iter().enumerate() {
        let doc: &Arc<Document> = &st.doc;
        for (_, _, layer) in doc.walk() {
            if let Some(hit) = build_hit(layer, di, &st.doc.name, active_idx == Some(di), query) {
                out.push(hit);
            }
        }
    }
    // Sort: active doc first, then by descending score, then by shorter label (more specific).
    out.sort_by(|a, b| {
        b.active_doc
            .cmp(&a.active_doc)
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.label.cmp(&b.label))
    });
    out.truncate(CAP);
    out
}

/// Build a hit for one layer if it passes the filters and matches the query.
fn build_hit(layer: &Layer, doc: usize, doc_name: &str, active_doc: bool, query: &Query) -> Option<Hit> {
    let (badge, text_content): (&'static str, Option<&str>) = match &layer.content {
        LayerContent::Text(t) => ("[T]", Some(t.text.as_str())),
        LayerContent::Group(_) => ("[G]", None),
        LayerContent::Raster(_) => ("[Px]", None),
        LayerContent::Shape(_) => ("[S]", None),
        LayerContent::Smart(_) => ("[Sm]", None),
        LayerContent::Adjustment(_) => ("[Adj]", None),
        LayerContent::Fill(_) => ("[Fill]", None),
        LayerContent::Screentone(_) => ("[ST]", None),
    };
    // Filters.
    if query.text_only && !matches!(layer.content, LayerContent::Text(_)) {
        return None;
    }
    if query.groups_only && !matches!(layer.content, LayerContent::Group(_)) {
        return None;
    }
    if query.hidden_only && layer.visible {
        return None;
    }
    let label = layer.name.clone();
    let label_lower = lower(&label);
    let flat_text = text_content.map(flatten_text);
    let text_lower = flat_text.as_ref().map(|s| lower(s));
    let (name_matches, text_matches, score) = score_hit(query, &label_lower, text_lower.as_deref())?;
    Some(Hit {
        doc,
        doc_name: doc_name.to_string(),
        layer: layer.id,
        label,
        text: flat_text.filter(|s| !s.is_empty()),
        badge,
        hidden: !layer.visible,
        active_doc,
        name_matches,
        text_matches,
        score,
    })
}

/// Show the floating palette if it is open. Closes itself on `Escape` and after a plain `Enter`
/// (jump); a `Shift+Enter` (peek) keeps the palette open.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.layer_search_open {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Scrim: full-screen, dimmed, click to dismiss (mirrors `palette::show`).
    egui::Area::new(egui::Id::new("layer-search-scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let r = ui.allocate_exact_size(screen.size(), Sense::click()).1;
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
            if r.clicked() {
                app.ui.layer_search_open = false;
            }
        });
    // The window itself, on top of the scrim.
    let mut run: Option<(Hit, bool)> = None;
    egui::Area::new(egui::Id::new("layer-search"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, screen.top() + TOP_OFFSET))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    let qid = egui::Id::new("layer-search-query");
                    let mut q: String = ui.data_mut(|d| d.get_temp(qid).unwrap_or_default());
                    // Query input row, with the search icon and a hint that lists the prefixes.
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        icons::paint(ui, r, "search", 16.0, t.text_dim);
                        let te = egui::TextEdit::singleline(&mut q)
                            .hint_text(tl!("Find layer or text across open pages (@text, #group)..."))
                            .frame(egui::Frame::NONE)
                            .font(egui::FontId::proportional(15.0))
                            .desired_width(WIDTH - 40.0);
                        let resp = ui.add(te);
                        // Only request focus when missing: every request_focus interrupts the IME
                        // (#585). Mirrors `palette::show`.
                        if !resp.has_focus() {
                            resp.request_focus();
                        }
                    });
                    ui.add_space(6.0);
                    crate::widgets::hairline(ui);
                    ui.add_space(6.0);
                    // Parse, collect, sort hits.
                    let query = Query::parse(&q);
                    let hits = collect_hits(app, &query);
                    // Keyboard navigation.
                    let sel_id = egui::Id::new("layer-search-sel");
                    let mut sel: usize = ui.data_mut(|d| d.get_temp(sel_id).unwrap_or(0));
                    let (down, up, enter, shift_enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::Enter) && !i.modifiers.shift,
                            i.key_pressed(egui::Key::Enter) && i.modifiers.shift,
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if down {
                        sel = (sel + 1).min(hits.len().saturating_sub(1));
                    }
                    if up {
                        sel = sel.saturating_sub(1);
                    }
                    sel = sel.min(hits.len().saturating_sub(1));
                    if esc {
                        app.ui.layer_search_open = false;
                    }
                    if hits.is_empty() {
                        ui.label(RichText::new(tl!("No matching layers or text in open pages")).color(t.text_faint));
                    }
                    let plain = TextFormat { font_id: theme::medium(13.0), color: t.text, ..Default::default() };
                    let high = TextFormat { font_id: theme::medium(13.0), color: t.accent, background: t.accent_soft, ..Default::default() };
                    let tplain = TextFormat { font_id: egui::FontId::proportional(11.5), color: t.text_dim, ..Default::default() };
                    let thigh = TextFormat { font_id: egui::FontId::proportional(11.5), color: t.accent, background: t.accent_soft, ..Default::default() };
                    for (i, hit) in hits.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(WIDTH, 30.0), Sense::click());
                        if i == sel || resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, t.radius_sm, if i == sel { t.accent_soft } else { t.hover });
                        }
                        let badge_color = if hit.hidden { t.text_faint } else { t.text_dim };
                        // Document tab badge (e.g. `[01.png]`), at the left.
                        let max_doc_chars = 13;
                        let doc_display = if hit.doc_name.chars().count() > max_doc_chars {
                            let prefix: String = hit.doc_name.chars().take(max_doc_chars - 1).collect();
                            format!("{prefix}…")
                        } else {
                            hit.doc_name.clone()
                        };
                        let doc_label = format!("[{doc_display}]");
                        let doc_w = 90.0;
                        ui.painter().text(
                            rect.left_center() + vec2(10.0, 0.0),
                            Align2::LEFT_CENTER,
                            doc_label,
                            egui::FontId::proportional(11.0),
                            badge_color,
                        );
                        // Layer-type badge (`[T]`, `[G]`, …) after the document badge.
                        ui.painter().text(
                            rect.left_center() + vec2(10.0 + doc_w, 0.0),
                            Align2::LEFT_CENTER,
                            hit.badge,
                            egui::FontId::proportional(11.0),
                            badge_color,
                        );
                        let badge_w = 34.0;
                        // Layer name (highlighted).
                        let name_left = 10.0 + doc_w + badge_w;
                        let mut job = egui::text::LayoutJob::default();
                        let max_name_w = if hit.text.is_some() {
                            190.0
                        } else {
                            (rect.right() - 12.0 - (rect.left() + name_left)).max(60.0)
                        };
                        job.wrap.max_width = max_name_w;
                        job.wrap.max_rows = 1;
                        let name_plain = TextFormat { color: if hit.hidden { t.text_faint } else { plain.color }, ..plain.clone() };
                        apply_matches(&mut job, &hit.label, &hit.name_matches, &name_plain, &high);
                        let gal = ui.painter().layout_job(job);
                        let pos = rect.left_center() + vec2(name_left, -gal.size().y / 2.0);
                        ui.painter().galley(pos, gal, name_plain.color);
                        // Text snippet (highlighted), shown only for text layers with a hit in the text.
                        if let Some(text) = hit.text.as_ref() {
                            let text_left = name_left + 200.0;
                            if text_left < rect.right() - 12.0 {
                                let max_w = (rect.right() - 12.0 - text_left).max(60.0);
                                let (snippet, start) = snippet_around(text, &hit.text_matches, SNIPPET_RADIUS);
                                let rel = relative_matches(&snippet, &hit.text_matches, start);
                                let mut tjob = egui::text::LayoutJob::default();
                                tjob.wrap.max_width = max_w;
                                apply_matches(&mut tjob, &snippet, &rel, &tplain, &thigh);
                                let gal = ui.painter().layout_job(tjob);
                                let pos = rect.left_center() + vec2(text_left, -gal.size().y / 2.0);
                                ui.painter().galley(pos, gal, tplain.color);
                            }
                        }
                        if resp.clicked() {
                            run = Some((hit.clone(), false));
                        }
                        if i == sel && (enter || shift_enter) {
                            run = Some((hit.clone(), shift_enter));
                        }
                    }
                    if enter && let Some(h) = hits.get(sel) {
                        // Enter (jump) closes the palette after the run; Shift+Enter (peek) keeps it open.
                        // Use `is_none()` rather than `Option::or` so the closure captures `run` by
                        // mutable reference rather than moving it (it's read again after the closure).
                        if run.is_none() {
                            run = Some((h.clone(), false));
                        }
                        // A dialog opened below would read this Enter as OK: consume it (#536).
                        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(tl!("Enter: Jump • Shift+Enter: Peek • Esc: Close"))
                            .small()
                            .color(t.text_faint),
                    );
                    ui.data_mut(|d| {
                        d.insert_temp(qid, q);
                        d.insert_temp(sel_id, sel);
                    });
                });
        });
    // Apply a chosen result outside the closure so we can borrow `app` mutably without aliasing.
    if let Some((hit, peek)) = run {
        if !peek {
            app.ui.layer_search_open = false;
        }
        // A failure here (no document, no layer) just leaves a status message; the palette is closed.
        if let Err(e) = activate(app, &hit) {
            app.ui.status = e;
        }
    }
}

/// Append `text` to `job`, splitting it into sections so the byte offsets in `matches` use `high`
/// and the rest use `plain`. Offsets that don't land on a char boundary are skipped.
fn apply_matches(job: &mut egui::text::LayoutJob, text: &str, matches: &[usize], plain: &TextFormat, high: &TextFormat) {
    if matches.is_empty() {
        if !text.is_empty() {
            job.append(text, 0.0, plain.clone());
        }
        return;
    }
    let mut cursor = 0usize;
    for &m in matches {
        if m < cursor || m > text.len() || !text.is_char_boundary(m) {
            continue;
        }
        if m > cursor {
            job.append(&text[cursor..m], 0.0, plain.clone());
        }
        // The matched char itself: from `m` to the next char boundary.
        let next = text[m..].char_indices().nth(1).map(|(b, _)| m + b).unwrap_or(text.len());
        job.append(&text[m..next], 0.0, high.clone());
        cursor = next;
    }
    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, plain.clone());
    }
}

/// A snippet of `text` around the first matched char, plus the byte offset of that snippet in
/// `text`. `radius` is the maximum number of chars on each side of the match.
fn snippet_around(text: &str, matches: &[usize], radius: usize) -> (String, usize) {
    let first = match matches.first() {
        Some(&m) => m,
        None => return (text.to_string(), 0),
    };
    // Walk char indices to find a window of `2 * radius + 1` chars around `first`.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mid = chars.iter().position(|(b, _)| *b == first).unwrap_or(0);
    let start = mid.saturating_sub(radius);
    let end = (mid + radius + 1).min(chars.len());
    let start_byte = chars.get(start).map(|(b, _)| *b).unwrap_or(0);
    let snippet: String = chars[start..end].iter().map(|(_, c)| *c).collect();
    (snippet, start_byte)
}

/// Translate absolute byte offsets in `text` to byte offsets in `snippet`, given the snippet's
/// start offset `start`. Offsets that fall outside the snippet are dropped.
fn relative_matches(snippet: &str, matches: &[usize], start: usize) -> Vec<usize> {
    let end = start + snippet.len();
    matches
        .iter()
        .copied()
        .filter(|&m| m >= start && m <= end)
        .map(|m| m - start)
        .collect()
}

/// Jump to a search result: switch tabs, select the layer, expand its collapsed parent groups, and
/// centre the canvas viewport on the layer's content bounds.
fn activate(app: &mut PhotocraftApp, hit: &Hit) -> Result<(), String> {
    // Switch to the result's document if it isn't the active one.
    let total = app.session.documents().len();
    if hit.doc >= total {
        return Err("the document this result was found in is no longer open".into());
    }
    if app.session.active_index() != Some(hit.doc) {
        if !app.session.set_active(hit.doc) {
            return Err("could not switch to that document".into());
        }
        // Keep `app.ui.views` aligned with the session's documents (a document may have been
        // added/removed since the last frame).
        app.sync_views();
    }
    // Select the target layer (no history step).
    if app.session.active().is_none() {
        return Err("no document open".into());
    }
    if app.session.active().and_then(|d| d.doc.layer(hit.layer)).is_none() {
        return Err("that layer is no longer in the document".into());
    }
    app.session
        .select_layer(hit.layer)
        .map_err(|e| e.to_string())?;
    // Expand every collapsed parent group, outermost first (a view change; not a history step).
    // We collect the parent group ids first (under the immutable borrow of `app.session`) and then
    // drop that borrow before calling `app.session.execute` (which takes `&mut`).
    let groups_to_expand: Vec<LayerId> = app
        .session
        .active()
        .and_then(|s| {
            let doc = &s.doc;
            let path = doc.path_of(hit.layer)?;
            Some(
                (1..path.len())
                    .filter_map(|n| {
                        let parent_path = path.get(..n)?;
                        let parent = doc.layer_at(parent_path)?;
                        matches!(&parent.content, LayerContent::Group(g) if !g.expanded).then_some(parent.id)
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    for id in groups_to_expand {
        // `layer.setExpanded` is a view change (no history step), so its failure just leaves the
        // group collapsed: we don't propagate the error.
        let _ = app.session.execute("layer.setExpanded", json!({"layer": id.0, "expanded": true}));
    }
    // Centre the canvas viewport on the layer's content bounds.
    let (bounds, doc_size) = app
        .session
        .active()
        .and_then(|s| s.doc.layer(hit.layer).map(|l| (position_bounds(l), s.doc.size)))
        .ok_or_else(|| "the layer has no bounds to centre on".to_string())?;
    // Find the view for the active document. `ui.views` is index-aligned with the session's
    // documents, kept in sync by `sync_views` after a tab switch above.
    if let Some(i) = app.session.active_index().filter(|&i| i < app.ui.views.len()) {
        let center = if bounds.is_empty() {
            // Adjustment/fill layers and empty groups have no content bounds: centre on the
            // document's own centre so the viewport lands somewhere sensible.
            [doc_size.width as f32 / 2.0, doc_size.height as f32 / 2.0]
        } else {
            [
                ((bounds.x0 as f32) + (bounds.x1 as f32)) / 2.0,
                ((bounds.y0 as f32) + (bounds.y1 as f32)) / 2.0,
            ]
        };
        app.ui.views[i].center = center;
        app.ui.views[i].fit_pending = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
