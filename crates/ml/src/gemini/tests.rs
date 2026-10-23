use super::*;
use base64::Engine as _;

/// The red marker and the whole page: what the tests below were written for.
const PAINT: Options = Options { mark: Mark::Paint, area: Area::Page };

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn png(rgb: &[f32], w: u32, h: u32) -> Vec<u8> {
    let img = photocraft_codecs::Image::from_normalized(w, h, photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8, rgb).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
}

fn answer_with(png: &[u8]) -> String {
    json!({
        "status": "completed",
        "steps": [
            {"type": "thought", "summary": [{"type": "text", "text": "thinking"}]},
            {"type": "model_output", "content": [{"type": "image", "mime_type": "image/png", "data": b64(png)}]},
        ],
    })
    .to_string()
}

#[test]
fn frames_take_the_nearest_ratio_and_hold_the_crop() {
    assert_eq!(frame(1000, 1000), ("1:1", 1000, 1000));
    assert_eq!(frame(1000, 760), ("4:3", 1014, 760));
    assert_eq!(frame(760, 1000), ("3:4", 760, 1014));
    let (r, w, h) = frame(3000, 300);
    assert_eq!((r, w, h), ("8:1", 3000, 375));
    for (cw, ch) in [(1, 1), (513, 512), (777, 1999), (4096, 17), (17, 4096)] {
        let (_, w, h) = frame(cw, ch);
        assert!(w >= cw && h >= ch, "{cw}x{ch} -> {w}x{h}");
    }
}

#[test]
fn the_model_sees_the_whole_page_or_a_wide_area_around_the_hole() {
    // A page that fits what is sent goes whole, as a large Photoshop selection gives it.
    for page in [Rect::new(0, 0, 729, 500), Rect::new(0, 0, 2048, 1400), Rect::new(0, 0, 300, 200)] {
        assert_eq!(context(Rect::new(100, 50, 150, 90), page, Area::Page), page);
    }
    let canvas = Rect::new(0, 0, 6000, 4000);
    // A small hole far from the edges: at least MIN_CONTEXT across, the hole in the middle.
    let c = context(Rect::new(3000, 2000, 3040, 2020), canvas, Area::Page);
    assert_eq!((c.width(), c.height()), (MIN_CONTEXT, MIN_CONTEXT));
    assert!(c.contains_rect(&Rect::new(3000, 2000, 3040, 2020)));
    // Near a corner: shifted inside, not cut.
    let c = context(Rect::new(5, 5, 45, 25), canvas, Area::Page);
    assert_eq!((c.x0, c.y0, c.width(), c.height()), (0, 0, MIN_CONTEXT, MIN_CONTEXT));
    // A large hole: its own size around it.
    let c = context(Rect::new(2000, 1500, 2800, 1900), canvas, Area::Page);
    assert_eq!(c, Rect::new(1200, 700, 3600, 2700));
}

#[test]
fn near_sends_about_twice_the_hole_for_a_sharper_fill() {
    let page = Rect::new(0, 0, 1200, 1800);
    // A small hole: NEAR_MIN across, around it (sent at 2K: drawn at about 2.7 times).
    let c = context(Rect::new(500, 900, 600, 980), page, Area::Near);
    assert_eq!((c.width(), c.height()), (NEAR_MIN, NEAR_MIN));
    assert!(c.contains_rect(&Rect::new(500, 900, 600, 980)));
    assert_eq!(image_size(c.width(), c.height()), "2K");
    // A 400 × 300 hole: half its longer side around it.
    assert_eq!(context(Rect::new(400, 700, 800, 1000), page, Area::Near), Rect::new(200, 466, 1000, 1234), "200 px around, grown to NEAR_MIN tall");
    // A wide title (1000 × 150): at most NEAR_MAX across (here the page's width), NEAR_MIN tall.
    assert_eq!(context(Rect::new(100, 1200, 1100, 1350), page, Area::Near), Rect::new(0, 891, 1200, 1659));
    // A hole larger than NEAR_MAX keeps NEAR_MARGIN around it.
    let c = context(Rect::new(1000, 1000, 2500, 1400), Rect::new(0, 0, 6000, 4000), Area::Near);
    assert_eq!(c, Rect::new(1000 - NEAR_MARGIN as i32, 816, 2500 + NEAR_MARGIN as i32, 1584));
    // The whole page, as before, when asked for.
    assert_eq!(context(Rect::new(500, 900, 600, 980), page, Area::Page), page);
}

#[test]
fn parts_whose_areas_overlap_are_one_request() {
    let canvas = Rect::new(0, 0, 3000, 2000);
    let (a, b, far) = (Rect::new(500, 500, 540, 540), Rect::new(900, 500, 940, 540), Rect::new(2500, 1500, 2540, 1540));
    assert_eq!(merged(vec![a, b, far], canvas, Area::Near), vec![a.union(&b), far]);
    assert_eq!(merged(vec![a, far], canvas, Area::Near), vec![a, far]);
    // The whole page: one request.
    let small = Rect::new(0, 0, 2000, 1500);
    assert_eq!(merged(vec![a, far.intersect(&small).union(&Rect::new(1900, 1400, 1940, 1440))], small, Area::Page).len(), 1);
}

#[test]
fn the_marker_is_red_unless_the_picture_is() {
    let grey = vec![0.5f32; 30];
    let hole = vec![false; 10];
    assert_eq!(pick_marker(&grey, &hole), 0);
    let reds: Vec<f32> = (0..10).flat_map(|_| [0.95, 0.05, 0.1]).collect();
    assert_eq!(pick_marker(&reds, &hole), 1, "a red picture gets green");
    // The hole's own pixels don't count.
    assert_eq!(pick_marker(&reds, &[true; 10]), 0);
    assert!(prompt("", MARKERS[1].1).contains("solid pure green"));
}

#[test]
fn large_frames_are_sent_smaller_and_ask_for_2k() {
    assert_eq!(send_size(1000, 750), (1000, 750));
    assert_eq!(send_size(4000, 3000), (2048, 1536));
    assert_eq!(send_size(300, 9000), (68, 2048));
    assert_eq!(image_size(640, 480), "1K");
    assert_eq!(image_size(1024, 768), "2K", "a crop past 640 px asks for 2K, drawn at about twice its size");
    assert_eq!(image_size(2048, 1536), "2K");
    assert_eq!(price(&[600, 1500]), PRICE_1K + PRICE_2K);
}

#[test]
fn resizing_keeps_a_flat_image_flat_and_averages_when_shrinking() {
    let flat = vec![0.25f32; 40 * 30 * 3];
    for (w, h) in [(40, 30), (13, 7), (100, 81)] {
        let r = resize(&flat, 40, 30, w, h);
        assert_eq!(r.len(), w * h * 3);
        assert!(r.iter().all(|v| (v - 0.25).abs() < 1e-5), "{w}x{h}");
    }
    // Alternating columns shrink to their mean.
    let stripes: Vec<f32> = (0..8 * 2).flat_map(|i| if i % 2 == 0 { [0.0; 3] } else { [1.0; 3] }).collect();
    let r = resize(&stripes, 8, 2, 4, 1);
    assert!(r.iter().all(|v| (v - 0.5).abs() < 0.2), "{r:?}");
    assert_eq!(resize(&[], 0, 0, 3, 2), vec![0.0; 18]);
}

#[test]
fn dilation_grows_by_the_radius() {
    let (w, h) = (9, 7);
    let mut m = vec![false; w * h];
    m[3 * w + 4] = true;
    let d = dilate(&m, w, h, 2);
    let set: Vec<(usize, usize)> = (0..w * h).filter(|k| d[*k]).map(|k| (k % w, k / w)).collect();
    assert_eq!(set.len(), 25);
    assert!(set.iter().all(|(x, y)| (2..=6).contains(x) && (1..=5).contains(y)));
}

#[test]
fn the_body_asks_for_an_unstored_png_of_the_frame() {
    let b: Value = serde_json::from_str(&body(&["QUJD".to_string()], "remove", "4:3", (1200, 900))).unwrap();
    // No response mime type: Google's examples for the model ask for JPEG or nothing.
    assert_eq!(b["model"], MODEL);
    assert_eq!(b["store"], false);
    assert_eq!(b["input"][0], json!({"type": "text", "text": "remove"}));
    assert_eq!(b["input"][1], json!({"type": "image", "mime_type": "image/png", "data": "QUJD"}));
    assert_eq!(b["response_format"], json!({"type": "image", "aspect_ratio": "4:3", "image_size": "2K"}));
    assert!(prompt("", MARKERS[0].1).contains("solid pure red"));
    assert!(prompt("  a window  ", MARKERS[0].1).ends_with("What should be there instead: a window"));
}

#[test]
fn answers_give_their_last_output_image_or_say_why_not() {
    let bytes = png(&[0.5; 2 * 2 * 3], 2, 2);
    assert_eq!(answer_image(&answer_with(&bytes)).unwrap(), bytes);
    let words = json!({"status": "completed", "steps": [{"type": "model_output", "content": [{"type": "text", "text": "I can't edit this image."}]}]});
    let e = answer_image(&words.to_string()).unwrap_err().to_string();
    assert!(e.contains("no image") && e.contains("can't edit"), "{e}");
    assert!(answer_image("{}").unwrap_err().to_string().contains("no image"));
    assert!(answer_image("not json").is_err());
    let bad = json!({"steps": [{"type": "model_output", "content": [{"type": "image", "data": "%%%"}]}]});
    assert!(answer_image(&bad.to_string()).unwrap_err().to_string().contains("base64"));
}

#[test]
fn api_errors_say_what_to_do() {
    let body = json!({"error": {"code": 403, "message": "API key not valid.", "status": "PERMISSION_DENIED"}}).to_string();
    let e = api_error(403, &body).to_string();
    assert!(e.contains("refused the API key") && e.contains("API key not valid"), "{e}");
    assert!(api_error(429, "{}").to_string().contains("quota"));
    assert!(api_error(400, "plain text").to_string().contains("refused the request (400): plain text"));
    assert!(api_error(503, &"x".repeat(5000)).to_string().len() < 400, "long answers are cut");
}

#[test]
fn keys_are_checked_for_shape() {
    assert!(valid_key("AIzaSyA-example_Key123").is_ok());
    assert!(valid_key("").is_err());
    assert!(valid_key("has a space").is_err());
    assert!(valid_key(&"k".repeat(300)).is_err());
    let e = valid_key("secret value").unwrap_err();
    assert!(!e.contains("secret"), "an error never repeats the key: {e}");
}

/// A 200 × 120 RGB window: a horizontal ramp, a hole in the middle.
fn scene() -> (Rect, Vec<f32>, Vec<bool>) {
    let win = Rect::new(0, 0, 200, 120);
    let (w, h) = (200usize, 120usize);
    let px: Vec<f32> = (0..w * h).flat_map(|i| [0.2 + 0.4 * (i % w) as f32 / w as f32, 0.5, 0.6]).collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (80..120).contains(&(i % w)) && (40..80).contains(&(i / w))).collect();
    (win, px, hole)
}

fn request<'a>(win: Rect, px: &'a [f32], hole: &'a [bool], ch: usize, alpha: Option<usize>) -> Request<'a> {
    Request { canvas: win, window: win, hole, pixels: px, ch, colours: Colours::Rgb, alpha, linear: false, transparent_unknown: false }
}

/// Google stand-in: checks the request, answers with the image sent at 1.5× with the red made
/// green and everything 0.1 brighter.
fn fake(sent: &mut Vec<Value>) -> impl FnMut(&str, &Interrupt) -> Result<String, MlError> + '_ {
    move |body, _ctl| {
        let v: Value = serde_json::from_str(body).unwrap();
        let data = base64::engine::general_purpose::STANDARD.decode(v["input"][1]["data"].as_str().unwrap()).unwrap();
        let img = photocraft_codecs::decode(&data).unwrap();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let rgb: Vec<f32> = img.to_rgba_f32().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        assert!(rgb.as_chunks::<3>().0.contains(&[1.0, 0.0, 0.0]), "the hole is sent red");
        let edited: Vec<f32> =
            rgb.as_chunks::<3>().0.iter().flat_map(|p| if *p == [1.0, 0.0, 0.0] { [0.1, 1.0, 0.1] } else { [p[0] + 0.1, p[1] + 0.1, p[2] + 0.1] }).collect();
        let (bw, bh) = (w * 3 / 2, h * 3 / 2);
        let big = resize(&edited, w, h, bw, bh);
        sent.push(v);
        Ok(answer_with(&png(&big, bw as u32, bh as u32)))
    }
}

#[test]
fn a_fill_changes_only_the_hole_and_matches_the_surrounding_colours() {
    let (win, px, hole) = scene();
    let req = request(win, &px, &hole, 3, None);
    let mut sent = Vec::new();
    let mut read = |r: Rect| {
        let mut out = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = (y as usize * 200 + x as usize) * 3;
                out.extend_from_slice(&px[k..k + 3]);
            }
        }
        out
    };
    let out = fill_via(&req, &mut read, "", PAINT, &Interrupt::NONE, &mut fake(&mut sent)).unwrap();
    assert_eq!(sent.len(), 1, "one part, one request");
    assert_eq!(sent[0]["store"], false);
    // The canvas (200 × 120) is smaller than the context: all of it, in the nearest ratio to
    // 200:120 (1.67), 16:9 (1.78) rather than 3:2 (1.5).
    assert_eq!(sent[0]["response_format"]["aspect_ratio"], "16:9");
    for (k, h) in hole.iter().enumerate() {
        let (a, b) = (&px[k * 3..k * 3 + 3], &out[k * 3..k * 3 + 3]);
        let (x, y) = (k % 200, k / 200);
        if *h && (83..117).contains(&x) && (43..77).contains(&y) {
            // Green, shifted back by the 0.1 the answer brightened its surroundings (the hole's
            // edge mixes with its neighbours through the two resamplings).
            assert!(b[1] > 0.8 && b[0] < 0.1 && b[2] < 0.1, "hole pixel {k}: {b:?}");
        } else if !*h {
            assert_eq!(a, b, "pixel {k} outside the hole is untouched");
        }
    }
}

#[test]
fn transparent_surroundings_are_refused_unless_unknown() {
    let (win, rgb, hole) = scene();
    let px: Vec<f32> = rgb.as_chunks::<3>().0.iter().enumerate().flat_map(|(i, p)| [p[0], p[1], p[2], if i % 200 < 40 { 0.0 } else { 1.0 }]).collect();
    let mut read = |r: Rect| {
        let mut out = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = (y as usize * 200 + x as usize) * 4;
                out.extend_from_slice(&px[k..k + 4]);
            }
        }
        out
    };
    let req = request(win, &px, &hole, 4, Some(3));
    let mut calls = 0;
    let e = fill_via(&req, &mut read, "", PAINT, &Interrupt::NONE, &mut |_, _| {
        calls += 1;
        Err(MlError::Inference("not reached".into()))
    })
    .unwrap_err();
    assert!(matches!(e, MlError::Unsuitable(_)), "{e}");
    assert_eq!(calls, 0, "nothing is sent");
    let req = Request { transparent_unknown: true, ..req };
    let mut sent = Vec::new();
    let out = fill_via(&req, &mut read, "", PAINT, &Interrupt::NONE, &mut fake(&mut sent)).unwrap();
    let k = 60 * 200 + 100;
    assert_eq!(out[k * 4 + 3], 1.0, "the hole comes back opaque");
}

#[test]
fn a_cancelled_fill_sends_nothing() {
    let (win, px, hole) = scene();
    let req = request(win, &px, &hole, 3, None);
    let cancel = || true;
    let ctl = Interrupt::cancel_only(&cancel);
    let mut calls = 0;
    let e = fill_via(&req, &mut |_| Vec::new(), "", PAINT, &ctl, &mut |_, _| {
        calls += 1;
        Ok(String::new())
    })
    .unwrap_err();
    assert_eq!((e, calls), (MlError::Cancelled, 0));
}

#[test]
fn far_apart_parts_of_a_large_page_are_one_request_each_and_a_small_page_one() {
    for (w, xs, requests) in [(6000usize, [100, 2900, 5700], 3), (2000, [100, 900, 1700], 1)] {
        let win = Rect::new(0, 0, w as i32, 400);
        let h = 400usize;
        let px = vec![0.5f32; w * h * 3];
        let hole: Vec<bool> = (0..w * h).map(|i| (100..130).contains(&(i / w)) && xs.iter().any(|x0| (*x0..x0 + 30).contains(&(i % w)))).collect();
        let req = request(win, &px, &hole, 3, None);
        let mut sent = Vec::new();
        let mut read = |r: Rect| vec![0.5f32; r.width() as usize * r.height() as usize * 3];
        fill_via(&req, &mut read, "", PAINT, &Interrupt::NONE, &mut fake(&mut sent)).unwrap();
        assert_eq!(sent.len(), requests, "{w} px wide");
    }
}

#[test]
fn a_mismatched_request_is_an_error_not_a_panic() {
    let win = Rect::new(0, 0, 10, 10);
    let px = vec![0.0f32; 10];
    let hole = vec![true; 100];
    let req = request(win, &px, &hole, 3, None);
    assert!(matches!(fill_via(&req, &mut |_| Vec::new(), "", PAINT, &Interrupt::NONE, &mut |_, _| Ok(String::new())), Err(MlError::BadInput(_))));
}

#[test]
fn an_answer_that_covers_its_frame_is_put_back_where_it_was() {
    let (fw, fh) = (150usize, 100usize);
    let rgb: Vec<f32> = (0..fw * fh)
        .flat_map(|i| {
            let (x, y) = ((i % fw) as f32, (i / fw) as f32);
            let v = 0.5 + 0.45 * (x * 0.6 + y * 0.11).sin();
            [v, v, v]
        })
        .collect();
    let hole: Vec<bool> = (0..fw * fh).map(|i| (60..90).contains(&(i % fw)) && (40..60).contains(&(i / fw))).collect();
    let c = Crop {
        rect: Rect::new(0, 0, 150, 100),
        frame: Rect::new(0, 0, 150, 100),
        ratio: "3:2",
        send: (150, 100),
        rgb: rgb.clone(),
        hole: hole.clone(),
        marker: 0,
    };
    // As the model does it: the frame scaled evenly by 1.7 to cover a 252 × 170 answer (255 wide),
    // the overhang cut (1 px at the left, 2 at the right).
    let big = resize(&rgb, fw, fh, 255, 170);
    let answer: Vec<f32> = (0..170).flat_map(|y| big[(y * 255 + 1) * 3..(y * 255 + 253) * 3].to_vec()).collect();
    let err = |out: &[f32]| -> f32 {
        let (sum, n) = out.chunks(3).zip(rgb.chunks(3)).zip(&hole).filter(|(_, h)| !**h).fold((0.0, 0), |(s, n), ((a, b), _)| (s + (a[0] - b[0]).abs(), n + 1));
        sum / n as f32
    };
    let (fitted, placed, matched) = c.fitted_placed(&answer, 252, 170);
    assert!(matched > 0.9, "it matches the page once placed: {matched}");
    let stretched = resize(&answer, 252, 170, fw, fh);
    assert!(err(&fitted) < 0.03 && err(&fitted) < err(&stretched) / 3.0, "placed {placed:?}: {} vs stretched {}", err(&fitted), err(&stretched));
}

#[test]
fn the_tone_match_maps_black_and_white_points_even_where_the_model_redrew() {
    // Paper and ink: the original's white 1.0 and black 0.0; the answer's paper 0.96 and ink 0.1,
    // with three times as much ink (it drew a marsh around the hole).
    let o: Vec<f64> = (0..1000).map(|i| if i % 10 == 0 { 0.0 } else { 1.0 }).collect();
    let a: Vec<f64> = (0..1000).map(|i| if i % 3 == 0 { 0.1 } else { 0.96 }).collect();
    let (from, gain, to) = levels(o, a);
    let map = |v: f32| to + (v - from) * gain;
    assert!((map(0.96) - 1.0).abs() < 0.01 && map(0.1).abs() < 0.01, "paper white, ink black: {} {}", map(0.96), map(0.1));
    // A flat area: the medians.
    let (from, gain, to) = levels(vec![0.5; 100], vec![0.45; 100]);
    assert!(gain == 1.0 && (to + (0.45 - from) * gain - 0.5).abs() < 1e-6);
    assert_eq!(levels(Vec::new(), Vec::new()), (0.0, 1.0, 0.0));
}

#[test]
fn an_answer_that_changed_the_framing_is_refused() {
    let (w, h) = (300usize, 200usize);
    let win = Rect::new(0, 0, 300, 200);
    let px: Vec<f32> = (0..w * h)
        .flat_map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let v = 0.5 + 0.45 * (x * 0.37 + y * 0.13).sin() * (y * 0.21).cos();
            [v, v, v]
        })
        .collect();
    let hole: Vec<bool> = (0..w * h).map(|i| (120..180).contains(&(i % w)) && (80..120).contains(&(i / w))).collect();
    let req = request(win, &px, &hole, 3, None);
    let mut read = |r: Rect| {
        let mut out = Vec::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = (y as usize * w + x as usize) * 3;
                out.extend_from_slice(&px[k..k + 3]);
            }
        }
        out
    };
    // Google zoomed in: the middle two thirds of what was sent, at 1.5 times its size.
    let mut zoomed = |body: &str, _: &Interrupt| -> Result<String, MlError> {
        let v: Value = serde_json::from_str(body).unwrap();
        let img = photocraft_codecs::decode(&base64::engine::general_purpose::STANDARD.decode(v["input"][1]["data"].as_str().unwrap()).unwrap()).unwrap();
        let (iw, ih) = (img.width() as usize, img.height() as usize);
        let rgb: Vec<f32> = img.to_rgba_f32().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let (cx, cy, cw, ch) = (iw / 6, ih / 6, iw * 2 / 3, ih * 2 / 3);
        let middle: Vec<f32> = (cy..cy + ch).flat_map(|y| rgb[(y * iw + cx) * 3..(y * iw + cx + cw) * 3].to_vec()).collect();
        let (bw, bh) = (iw * 3 / 2, ih * 3 / 2);
        Ok(answer_with(&png(&resize(&middle, cw, ch, bw, bh), bw as u32, bh as u32)))
    };
    let e = fill_via(&req, &mut read, "", Options::default(), &Interrupt::NONE, &mut zoomed).unwrap_err();
    assert!(e.to_string().contains("doesn't line up"), "{e}");
    // The same picture answered in its framing is taken.
    let mut sent = Vec::new();
    assert!(fill_via(&req, &mut read, "", Options::default(), &Interrupt::NONE, &mut fake(&mut sent)).is_ok());
}
