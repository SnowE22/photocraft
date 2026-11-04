//! Phase 3: paste-on-click with auto-clean — the balloon is filled on a `TypeR Clean` layer with
//! white or black chosen from the artwork, the translation is pasted and the queue advances.

use serde_json::json;

use crate::PhotocraftApp;
use photocraft_doc::LayerContent;
use photocraft_raster::Surface;

fn rect_selection(x0: i32, y0: i32, width: i32, height: i32) -> Surface {
    let bounds = photocraft_geom::Rect::new(x0, y0, x0 + width, y0 + height);
    let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
    selection.write_region(bounds, &vec![1.0; width as usize * height as usize]);
    selection
}

fn app_with(dark: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
    if dark {
        app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    }
    app
}

fn clean_pixel(app: &PhotocraftApp, x: i32, y: i32) -> Option<[u8; 4]> {
    let st = app.session.active()?;
    let layer = st.doc.layers.iter().find(|layer| layer.name == crate::snow_clean::CLEAN_LAYER)?;
    let LayerContent::Raster(surface) = &layer.content else { return None };
    let mut value = [[0u8; 4]; 1];
    surface.read_rgba8_into(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &mut value);
    Some(value[0])
}

#[test]
fn auto_clean_fills_the_balloon_and_advances_the_queue() {
    let mut app = app_with(false);
    app.typesetter.state.options.paste_on_click = true;
    app.typesetter.state.options.auto_clean = true;
    app.typesetter.state.load_script("Clean line\nNext line");
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(10, 10, 80, 80));
    app.maybe_typesetter_paste_on_click();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(20, 20, 90, 90));
    app.maybe_typesetter_paste_on_click();

    let pixel = clean_pixel(&app, 40, 40).expect("the TypeR Clean layer has pixels");
    assert!(pixel[3] > 200, "the balloon is filled: {pixel:?}");
    assert_eq!(&pixel[..3], &[255, 255, 255], "bright art is cleaned with white: {pixel:?}");
    assert_eq!(app.typesetter.state.current_line_idx, 1, "the queue advances to the next line");
}

#[test]
fn dark_art_is_cleaned_with_black() {
    let mut app = app_with(true);
    app.typesetter.state.options.paste_on_click = true;
    app.typesetter.state.options.auto_clean = true;
    app.typesetter.state.load_script("Clean line");
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(10, 10, 80, 80));
    app.maybe_typesetter_paste_on_click();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(20, 20, 90, 90));
    app.maybe_typesetter_paste_on_click();

    let pixel = clean_pixel(&app, 40, 40).expect("the TypeR Clean layer has pixels");
    assert_eq!(&pixel[..3], &[0, 0, 0], "dark art is cleaned with black: {pixel:?}");
}

#[test]
fn auto_clean_is_off_by_default_and_leaves_no_layer() {
    let mut app = app_with(false);
    assert!(!app.typesetter.state.options.auto_clean);
    app.typesetter.state.options.paste_on_click = true;
    app.typesetter.state.load_script("Line");
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(10, 10, 80, 80));
    app.maybe_typesetter_paste_on_click();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(rect_selection(20, 20, 90, 90));
    app.maybe_typesetter_paste_on_click();
    assert!(!app.session.active().unwrap().doc.layers.iter().any(|layer| layer.name == crate::snow_clean::CLEAN_LAYER));
}

#[test]
fn text_shape_previews_follow_the_selected_page_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app
        .run("type.create", json!({"x": 20, "y": 60, "text": "A selected text layer on the page with words", "size": 24}))
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    app.session.active_mut().unwrap().active_layer = Some(photocraft_doc::LayerId(id));
    app.refresh_text_shape_previews_from_page();
    assert!(app.typesetter.has_text_shape_candidates(), "the page layer fills the preview cards");
    assert!(app.typesetter.first_text_shape_candidate_text().is_some());
    app.session.active_mut().unwrap().active_layer = None;
    app.refresh_text_shape_previews_from_page();
    assert!(!app.typesetter.has_text_shape_candidates(), "no selected layer clears the previews");
}
#[test]
fn style_preset_applies_to_the_selected_text_layer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("type.create", json!({"x": 20, "y": 60, "text": "Style me please", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    app.session.active_mut().unwrap().active_layer = Some(photocraft_doc::LayerId(id));
    let preset = photocraft_engine::typesetter::TextStylePreset {
        name: "Applied".into(),
        font_size: 33.0,
        alignment: "right".into(),
        first_line_indent: 5.0,
        hyphenate: true,
        ..photocraft_engine::typesetter::TextStylePreset::default()
    };
    app.apply_typesetter_style_to_active_layer(&preset);
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
    let LayerContent::Text(text) = &layer.content else { panic!("text layer") };
    assert_eq!(text.runs[0].style.size_pt, 33.0);
    assert_eq!(text.paragraphs[0].style.align, photocraft_doc::text::TextAlign::Right);
    assert_eq!(text.paragraphs[0].style.first_line_indent_pt, 5.0);
}