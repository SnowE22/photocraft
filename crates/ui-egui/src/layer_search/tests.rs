//! Tests for the layer & text search palette: query parsing, wrong-layout (translit) matching,
//! hit collection and scoring, the jump-and-focus action, and the floating panel's keyboard
//! navigation (type → arrows → Enter / Shift+Enter / Esc).

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId, comps::position_bounds};
use serde_json::json;

use super::{
    CAP, TRANSLIT_PENALTY, Query, LAYOUT_PAIRS, activate, collect_hits, flatten_text, layout_variants,
    remap_layout, score_hit, substring_matches,
};
use crate::{PhotocraftApp, theme::ThemeKind};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// The ids the assertions need, captured while the fixture is built.
#[derive(Clone, Copy)]
struct Ids {
    reply: LayerId,
    group: LayerId,
    draft: LayerId,
    bubble: LayerId,
    lamp: LayerId,
    ladder: LayerId,
}

/// Page 1: a collapsed group «Диалоги» holding the text layer "Reply 1" with the two-line content
/// "Привет, мир!\r\nКак дела?", the rasters "Фон" and the hidden "Draft" above the group.
/// Page 2: the rasters "Bubble", "Lamp" and "Ladder". Ends with page 2 active, like a real session
/// would after opening a second file.
fn fixture() -> (PhotocraftApp, Ids) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let reply = LayerId(
        app.run("type.create", json!({"text": "Привет, мир!\r\nКак дела?", "name": "Reply 1"}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap(),
    );
    app.run("layer.groupLayers", json!({})).unwrap();
    let group = app
        .session
        .active()
        .unwrap()
        .doc
        .walk()
        .into_iter()
        .find(|(_, _, l)| l.is_group())
        .map(|(_, _, l)| l.id)
        .unwrap();
    app.run("layer.renameLayer", json!({"layer": group.0, "name": "Диалоги"})).unwrap();
    app.run("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    app.run("layer.new.layer", json!({"name": "Фон"})).unwrap();
    let draft = LayerId(app.run("layer.new.layer", json!({"name": "Draft"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.hideLayers", json!({"layer": draft.0})).unwrap();
    // Page 2.
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let bubble = LayerId(app.run("layer.new.layer", json!({"name": "Bubble"})).unwrap()["layer"].as_u64().unwrap());
    let lamp = LayerId(app.run("layer.new.layer", json!({"name": "Lamp"})).unwrap()["layer"].as_u64().unwrap());
    let ladder = LayerId(app.run("layer.new.layer", json!({"name": "Ladder"})).unwrap()["layer"].as_u64().unwrap());
    app.sync_views();
    (app, Ids { reply, group, draft, bubble, lamp, ladder })
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            super::show(app, ui.ctx());
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::ProMedium);
    h.state_mut().ui.theme = ThemeKind::ProMedium;
    h.run_steps(4);
    h
}

fn open(h: &mut Harness<'static, PhotocraftApp>) {
    h.state_mut().ui.layer_search_open = true;
    h.run_steps(2);
}

fn key(h: &mut Harness<'static, PhotocraftApp>, k: Key, modifiers: Modifiers) {
    // A real platform delivers modifier changes around the key events — `InputState::modifiers`
    // only updates from `ModifiersChanged` — so send them like `shortcut_dispatch/tests` does.
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers });
    h.event(Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers });
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn type_text(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

/// The centre the palette should have scrolled the viewport of `doc` to for `layer`.
fn expected_center(app: &PhotocraftApp, doc: usize, layer: LayerId) -> [f32; 2] {
    let st = app.session.documents()[doc].clone();
    let bounds = position_bounds(st.doc.layer(layer).unwrap());
    if bounds.is_empty() {
        [st.doc.size.width as f32 / 2.0, st.doc.size.height as f32 / 2.0]
    } else {
        [((bounds.x0 + bounds.x1) as f32) / 2.0, ((bounds.y0 + bounds.y1) as f32) / 2.0]
    }
}

// ---------------------------------------------------------------------------------------------
// Query parsing and the wrong-layout (translit) table
// ---------------------------------------------------------------------------------------------

#[test]
fn layout_table_roundtrips_every_pair() {
    for &(latin, cyr) in LAYOUT_PAIRS {
        assert_eq!(remap_layout(&latin.to_string(), true), cyr.to_string());
        assert_eq!(remap_layout(&cyr.to_string(), false), latin.to_string());
    }
    // Chars without a mapping (digits, spaces) pass through.
    assert_eq!(remap_layout("a1 b", true), "ф1 и");
}

#[test]
fn wrong_layout_queries_get_the_translit_variant() {
    let q = Query::parse("GHBDTN");
    assert_eq!(q.body, "ghbdtn");
    assert!(q.variants.iter().any(|(v, p)| v == "ghbdtn" && *p == 0), "the raw query first");
    assert!(q.variants.iter().any(|(v, _)| v == "привет"), "…then the re-mapped one");

    // And the other way round: a cyrillic body also tries latin.
    let q = Query::parse("Привет");
    assert!(q.variants.iter().any(|(v, _)| v == "ghbdtn"));

    assert_eq!(layout_variants("мама"), vec!["vfvf".to_string()]);
    assert_eq!(layout_variants("игииду"), vec!["bubble".to_string()]);
    // Digits and punctuation carry no layout, so no variants.
    assert!(layout_variants("123 ?!").is_empty());
    assert!(layout_variants("").is_empty());
    // A mixed body tries both directions.
    assert_eq!(layout_variants("qй").len(), 2);
}

#[test]
fn prefixes_parse_in_any_order_and_body_is_lower_cased() {
    let q = Query::parse("  #! ПРИВет");
    assert!(q.groups_only && q.hidden_only && !q.text_only);
    assert_eq!(q.body, "привет");
    let q = Query::parse("@");
    assert!(q.text_only);
    assert_eq!(q.body, "", "a bare prefix means 'list everything that passes'");
}

#[test]
fn substring_matches_reports_byte_offsets() {
    // Cyrillic chars are two bytes wide: offsets are byte offsets.
    assert_eq!(substring_matches("при", "привет"), Some(vec![0, 2, 4]));
    assert_eq!(substring_matches("", "abc"), Some(vec![]));
    assert_eq!(substring_matches("b", "abc"), Some(vec![1]));
    assert_eq!(substring_matches("xyz", "abc"), None);
    assert_eq!(substring_matches("too long", "short"), None);
}

#[test]
fn flatten_text_joins_line_breaks() {
    assert_eq!(flatten_text("Привет, мир!\r\nКак\r дела?\n"), "Привет, мир! Как дела?");
    assert_eq!(flatten_text("a  \t b"), "a b");
    assert_eq!(flatten_text(""), "");
}

// ---------------------------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------------------------

#[test]
fn raw_matches_rank_above_translit_ones() {
    // The same name substring hit: the raw query outranks its translit variant by the penalty.
    let raw = score_hit(&Query::parse("привет"), "привет", None).unwrap().2;
    let translit = score_hit(&Query::parse("ghbdtn"), "привет", None).unwrap().2;
    assert_eq!(translit, raw - TRANSLIT_PENALTY);

    // A translit hit in the name still beats a raw hit in the text content (same-tier steps stay
    // bigger than the penalty).
    let name_translit = score_hit(&Query::parse("ghbdtn"), "привет", Some("какие-то слова")).unwrap().2;
    let text_raw = score_hit(&Query::parse("слова"), "zzz", Some("тут слова")).unwrap().2;
    assert!(name_translit > text_raw, "{name_translit} vs {text_raw}");

    // Within a tier the shorter (more specific) name wins.
    let lamp = score_hit(&Query::parse("la"), "lamp", None).unwrap().2;
    let ladder = score_hit(&Query::parse("la"), "ladder", None).unwrap().2;
    assert!(lamp > ladder, "{lamp} vs {ladder}");

    // An empty body lists everything at the baseline score.
    assert_eq!(score_hit(&Query::parse(""), "что угодно", None), Some((vec![], vec![], 0)));
}

// ---------------------------------------------------------------------------------------------
// Hit collection over a real session
// ---------------------------------------------------------------------------------------------

#[test]
fn wrong_layout_queries_find_both_directions() {
    let (app, _) = fixture();
    // "ghbdtn" is «привет» typed with the layout still on QWERTY: it matches the text content of
    // Reply 1 across the \r\n line break, and nothing else.
    let hits = collect_hits(&app, &Query::parse("ghbdtn"));
    assert_eq!(hits.len(), 1, "{:#?}", hits.iter().map(|h| &h.label).collect::<Vec<_>>());
    assert_eq!(hits[0].label, "Reply 1");
    assert_eq!(hits[0].badge, "[T]");
    assert_eq!(hits[0].text.as_deref(), Some("Привет, мир! Как дела?"));
    assert_eq!(hits[0].doc, 0);

    // «игииду» is "Bubble" typed with ЙЦУКЕН left on.
    let hits = collect_hits(&app, &Query::parse("игииду"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].label, "Bubble");
    assert_eq!(hits[0].doc, 1);
}

#[test]
fn prefix_filters_select_text_groups_and_hidden_layers() {
    let (app, ids) = fixture();
    let hits = collect_hits(&app, &Query::parse("@ghbdtn"));
    assert_eq!(hits.len(), 1, "@: text layers only");
    assert_eq!(hits[0].layer, ids.reply);

    let hits = collect_hits(&app, &Query::parse("#диалог"));
    assert_eq!(hits.len(), 1, "#: groups only");
    assert_eq!(hits[0].layer, ids.group);
    assert_eq!(hits[0].badge, "[G]");

    let hits = collect_hits(&app, &Query::parse("!draft"));
    assert_eq!(hits.len(), 1, "!: hidden layers only");
    assert_eq!(hits[0].layer, ids.draft);
    assert!(hits[0].hidden);
}

#[test]
fn empty_query_lists_the_active_document_first_and_is_capped() {
    let (app, _) = fixture();
    assert_eq!(app.session.active_index(), Some(1), "the fixture ends on page 2");
    let hits = collect_hits(&app, &Query::parse(""));
    assert!(!hits.is_empty());
    assert!(hits[0].active_doc && hits[0].doc == 1, "page 2 is active, so it sorts first");

    let mut wide = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    wide.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    for i in 0..15 {
        wide.run("layer.new.layer", json!({"name": format!("Filler {i:02}")})).unwrap();
    }
    let hits = collect_hits(&wide, &Query::parse("filler"));
    assert_eq!(hits.len(), CAP, "the list shows at most CAP hits");
}

// ---------------------------------------------------------------------------------------------
// The jump-and-focus action
// ---------------------------------------------------------------------------------------------

#[test]
fn activate_switches_tabs_selects_and_expands_groups() {
    let (mut app, ids) = fixture();
    assert_eq!(app.session.active_index(), Some(1), "the fixture ends on page 2");
    // The hit lives in a collapsed group on page 1.
    let hit = collect_hits(&app, &Query::parse("ghbdtn")).remove(0);
    activate(&mut app, &hit).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(app.session.active_index(), Some(0), "switched to the page with the hit");
    assert_eq!(st.active_layer, Some(ids.reply), "the hit's layer is selected");
    assert!(
        matches!(&st.doc.layer(ids.group).unwrap().content, LayerContent::Group(g) if g.expanded),
        "the collapsed parent group opened"
    );
    assert_eq!(app.ui.views[0].center, expected_center(&app, 0, ids.reply), "the viewport centres on the layer");
    assert!(!app.ui.views[0].fit_pending);
}

#[test]
fn activate_fails_gracefully_for_a_closed_document() {
    let (mut app, _) = fixture();
    let mut gone = collect_hits(&app, &Query::parse("ghbdtn")).remove(0);
    gone.doc = 999;
    assert!(activate(&mut app, &gone).is_err(), "the document is no longer open");
    // And for a deleted layer.
    let gone = collect_hits(&app, &Query::parse("bubble")).remove(0);
    app.session.set_active(1);
    app.session.execute("layer.delete", json!({"layer": gone.layer.0})).unwrap();
    assert!(activate(&mut app, &gone).is_err(), "the layer is no longer in the document");
}

// ---------------------------------------------------------------------------------------------
// The floating panel: search and keyboard navigation (egui_kittest)
// ---------------------------------------------------------------------------------------------

#[test]
fn typing_a_wrong_layout_query_finds_and_jumps_to_the_text() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    type_text(&mut h, "ghbdtn");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(0), "jumped to the page with the hit");
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.reply), "the hit's layer is selected");
    assert!(!app.ui.layer_search_open, "Enter closes the palette");
    assert_eq!(app.ui.views[0].center, expected_center(app, 0, ids.reply), "the viewport centres on the layer");
}

#[test]
fn a_cyrillic_query_finds_the_latin_layer_name() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    type_text(&mut h, "игииду");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(1), "switched tabs");
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.bubble));
    assert!(!app.ui.layer_search_open, "Enter closes the palette");
}

#[test]
fn enter_jumps_to_the_best_hit() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    // "la" matches "Lamp" (shorter, so it sorts first) and "Ladder", both on page 2.
    type_text(&mut h, "la");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(1), "switched tabs");
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.lamp), "the best hit is selected");
    assert!(!app.ui.layer_search_open, "Enter closes the palette");
}

#[test]
fn arrows_move_the_selection_and_enter_jumps() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    type_text(&mut h, "la");
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    key(&mut h, Key::Enter, Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.ladder), "ArrowDown moved past Lamp");
    assert!(!app.ui.layer_search_open);
}

#[test]
fn shift_enter_peeks_and_escape_closes() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    type_text(&mut h, "la");
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    key(&mut h, Key::Enter, Modifiers::SHIFT);
    {
        let app = h.state();
        assert_eq!(app.session.active().unwrap().active_layer, Some(ids.ladder), "peeked at the hit");
        assert!(app.ui.layer_search_open, "a peek keeps the palette open");
    }
    key(&mut h, Key::Escape, Modifiers::NONE);
    let app = h.state();
    assert!(!app.ui.layer_search_open, "Esc closes the palette");
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.ladder), "Esc does not undo the jump");
}

#[test]
fn the_hidden_filter_jumps_to_a_hidden_layer() {
    let (app, ids) = fixture();
    let mut h = harness(app);
    open(&mut h);
    type_text(&mut h, "!draft");
    key(&mut h, Key::Enter, Modifiers::NONE);
    let app = h.state();
    assert_eq!(app.session.active_index(), Some(0));
    assert_eq!(app.session.active().unwrap().active_layer, Some(ids.draft));
}

#[test]
fn escape_closes_the_untouched_palette() {
    let (app, _) = fixture();
    let mut h = harness(app);
    open(&mut h);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!h.state().ui.layer_search_open);
}
