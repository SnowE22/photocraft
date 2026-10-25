use std::path::{Path, PathBuf};

use serde_json::json;

use super::plan::{Part, PlanOptions, plan_export};
use super::*;

/// A fresh scratch directory per test (removed first, so a rerun starts clean).
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-timelapse-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A session with a 64×48 red document, recording into `dir`.
fn recording(dir: &Path) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute(SETTINGS, json!({"folder": dir.to_string_lossy()})).unwrap();
    s.execute(RECORD, json!({})).unwrap();
    s
}

/// Let the recorder notice the latest change and take its frame (past the settle time).
fn settle(s: &mut Session, t: &mut f64) {
    *t += 10.0;
    s.timelapse_tick_at(*t, false);
    *t += recorder::MAX_WAIT_MS + 10.0;
    s.timelapse_tick_at(*t, false);
    s.timelapse_wait();
}

fn track(s: &Session) -> &recorder::Track {
    s.timelapse.track(s.active().unwrap().doc.id).unwrap()
}

fn frames(s: &Session) -> Vec<(u64, u8)> {
    track(s).rec.meta.frames.iter().map(|f| (f.n, f.u)).collect()
}

// ---- plan --------------------------------------------------------------------------------------

#[test]
fn auto_length_shows_every_frame_once_within_limits() {
    let o = PlanOptions::default();
    assert_eq!(plan_export(450, &o).total, 450, "15 s at 30 fps");
    assert_eq!(plan_export(10, &o).total, 150, "at least 5 s");
    assert_eq!(plan_export(10_000, &o).total, 1800, "at most a minute");
    let p = plan_export(450, &o);
    assert_eq!(p.at(0).src, 0);
    assert_eq!(p.at(449).src, 449);
}

#[test]
fn intro_flash_and_hold_frame_the_main_part() {
    let o = PlanOptions { length_sec: Some(10.0), intro_sec: 1.0, flash: true, hold_sec: 2.0, ..Default::default() };
    let p = plan_export(100, &o);
    assert_eq!((p.intro, p.main.len(), p.hold), (30, 300, 60));
    assert_eq!(p.flash.len(), 3 + 15);
    assert_eq!(p.total, 30 + 300 + 18 + 60);
    assert_eq!(p.at(0), plan::At { src: 99, part: Part::Intro, white: 0.0 }, "the finished drawing first");
    assert_eq!(p.at(30).part, Part::Main);
    assert_eq!(p.at(30).src, 0);
    let peak = (0..p.total).map(|k| p.at(k).white).fold(0.0f32, f32::max);
    assert_eq!(peak, 1.0, "the flash reaches white");
    assert_eq!(p.at(p.total - 1), plan::At { src: 99, part: Part::Hold, white: 0.0 });
}

#[test]
fn slow_end_spends_longer_on_the_last_frames() {
    let fast = plan_export(100, &PlanOptions { length_sec: Some(10.0), ..Default::default() });
    let slow = plan_export(100, &PlanOptions { length_sec: Some(10.0), slow_end: true, ..Default::default() });
    let tail = |p: &plan::Plan| p.main.iter().filter(|&&s| s >= 90).count();
    assert!(tail(&slow) * 2 > tail(&fast) * 3, "{} vs {}", tail(&slow), tail(&fast));
    assert!(slow.main.windows(2).all(|w| w[0] <= w[1]), "never goes backwards");
    assert_eq!(*slow.main.last().unwrap(), 99);
}

#[test]
fn hostile_plan_options_stay_bounded() {
    let o = PlanOptions { fps: f64::NAN, length_sec: Some(f64::INFINITY), intro_sec: -3.0, hold_sec: 1e12, flash: false, slow_end: true };
    let p = plan_export(1, &o);
    assert!(p.total <= plan::MAX_OUTPUT_FRAMES * 2);
    assert_eq!(p.at(p.total - 1).src, 0);
}

// ---- deflicker ---------------------------------------------------------------------------------

fn flat(v: u8) -> Vec<u8> {
    vec![v; deflicker::THUMB]
}

#[test]
fn a_frame_that_flashes_in_and_out_is_left_out() {
    let t = [flat(10), flat(10), flat(200), flat(12), flat(12)];
    let refs: Vec<Option<&[u8]>> = t.iter().map(|v| Some(v.as_slice())).collect();
    assert_eq!(deflicker::find_flicker(&refs, deflicker::SENSITIVITY), vec![2]);
    // Gradual change is not flicker.
    let g: Vec<Vec<u8>> = (0..6).map(|i| flat(i * 40)).collect();
    let refs: Vec<Option<&[u8]>> = g.iter().map(|v| Some(v.as_slice())).collect();
    assert!(deflicker::find_flicker(&refs, deflicker::SENSITIVITY).is_empty());
}

#[test]
fn bursts_of_two_and_three_are_found_but_never_the_ends() {
    // (At sensitivity 200 a short plateau between two jumps counts as a burst too, so the steady
    // stretches here are long.)
    let t = [flat(10), flat(200), flat(201), flat(10), flat(10), flat(10), flat(10), flat(90), flat(91), flat(92), flat(10), flat(10)];
    let refs: Vec<Option<&[u8]>> = t.iter().map(|v| Some(v.as_slice())).collect();
    assert_eq!(deflicker::find_flicker(&refs, deflicker::SENSITIVITY), vec![1, 2, 7, 8, 9]);
    // Jumps at the very start and end: the first and last frames are never judged.
    let ends = [flat(200), flat(10), flat(10), flat(10), flat(10), flat(200)];
    let refs: Vec<Option<&[u8]>> = ends.iter().map(|v| Some(v.as_slice())).collect();
    assert!(deflicker::find_flicker(&refs, deflicker::SENSITIVITY).is_empty());
    // Unknown thumbnails are never judged.
    let refs = vec![Some(t[0].as_slice()), None, Some(t[3].as_slice())];
    assert!(deflicker::find_flicker(&refs, deflicker::SENSITIVITY).is_empty());
}

#[test]
fn the_thumbnail_cache_round_trips_and_survives_a_torn_file() {
    let mut m = std::collections::HashMap::new();
    m.insert(7u32, flat(3));
    m.insert(2u32, flat(9));
    let mut bytes = deflicker::serialize_thumbs(&m);
    assert_eq!(bytes.len(), 2 * (4 + deflicker::THUMB));
    assert_eq!(&bytes[..4], &2u32.to_le_bytes(), "sorted by frame number");
    bytes.extend_from_slice(&[1, 2, 3]);
    assert_eq!(deflicker::parse_thumbs(&bytes), m);
    let tiny = deflicker::thumbnail(&[255, 0, 0, 0, 0, 255], 2, 1).unwrap();
    assert_eq!(tiny.len(), deflicker::THUMB);
    assert!(deflicker::thumbnail(&[1, 2], 4, 4).is_none(), "short buffer");
}

// ---- raster ------------------------------------------------------------------------------------

#[test]
fn resize_and_letterbox_keep_colour_and_add_bars() {
    let red: Vec<u8> = std::iter::repeat_n([200u8, 10, 10], 40 * 20).flatten().collect();
    let half = raster::resize(&red, 40, 20, 20, 10);
    assert_eq!(half.len(), 20 * 10 * 3);
    assert!(half.chunks(3).all(|p| p == [200, 10, 10]));
    // A 2:1 frame in a 1:1 box: bars above and below.
    let boxed = raster::letterbox(&red, 40, 20, 20, 20, [0, 0, 0]);
    assert_eq!(&boxed[..3], &[0, 0, 0], "bar");
    let mid = (10 * 20 + 10) * 3;
    assert_eq!(&boxed[mid..mid + 3], &[200, 10, 10]);
    // Wrong-sized input: black, not a panic.
    assert_eq!(raster::resize(&[1, 2, 3], 5, 5, 2, 2), vec![0; 12]);
    assert_eq!(raster::letterbox(&red, 0, 0, 2, 2, [9, 9, 9]), vec![9; 12]);
}

#[test]
fn whiten_count_and_label() {
    let mut px = vec![0u8, 100, 255];
    raster::whiten(&mut px, 0.5);
    assert_eq!(px, vec![128, 178, 255]);
    raster::whiten(&mut px, f32::NAN);
    assert_eq!(px, vec![128, 178, 255]);
    assert_eq!(raster::count_changed(&[0, 0, 0, 10, 10, 10], &[3, 0, 0, 30, 10, 10], 4, 10), 1);
    assert_eq!(raster::format_work_label(45 * 60_000), "45m");
    assert_eq!(raster::format_work_label(125 * 60_000 + 59_000), "2h 05m");
}

#[test]
fn the_work_time_badge_lands_in_its_corner() {
    let (w, h) = (320usize, 240usize);
    let badge = raster::make_badge("1h 23m", h);
    assert!(badge.w > badge.h && badge.h > 0);
    let mut frame = vec![0u8; w * h * 3];
    raster::draw_badge(&mut frame, w, h, &badge, raster::Corner::BottomRight);
    let lit = |x0: usize, x1: usize, y0: usize, y1: usize| (y0..y1).any(|y| (x0..x1).any(|x| frame[(y * w + x) * 3] > 100));
    assert!(lit(w / 2, w, h / 2, h), "white text bottom right");
    assert!(!lit(0, w / 2, 0, h / 2), "nothing top left");
}

// ---- store -------------------------------------------------------------------------------------

#[test]
fn folder_names_are_safe() {
    assert_eq!(store::safe_name("Portrait.psd"), "Portrait");
    assert_eq!(store::safe_name("a:b*c?.png"), "a_b_c_");
    assert_eq!(store::safe_name("   "), "Untitled");
    assert_eq!(store::safe_name("v1.2 final"), "v1.2 final");
    assert_eq!(store::folder_name("1a2b3c4d-ffff", "Tree"), "Tree (1a2b3c4d)");
}

#[test]
fn the_photoshop_plug_ins_recording_id_is_read_from_xmp() {
    let attr = r#"<rdf:Description rdf:about="" xmlns:tlr="http://ns.timelapse-recorder.local/1.0/" tlr:RecordingID="b26b3312-aaaa"/>"#;
    assert_eq!(store::xmp_recording_id(attr).as_deref(), Some("b26b3312-aaaa"));
    let elem = r#"<rdf:Description xmlns:ns1="http://ns.timelapse-recorder.local/1.0/"><ns1:RecordingID>abc</ns1:RecordingID></rdf:Description>"#;
    assert_eq!(store::xmp_recording_id(elem).as_deref(), Some("abc"));
    assert_eq!(store::xmp_recording_id("<x:xmpmeta/>"), None);
    assert_eq!(store::xmp_recording_id(r#"xmlns:t="http://ns.timelapse-recorder.local/1.0/" t:RecordingID="#), None, "truncated");
}

/// A recording the Photoshop plug-in wrote: PhotoCraft reads it, keeps what it doesn't know and
/// exports it.
#[test]
fn a_plug_in_recording_loads_and_exports() {
    let dir = temp_dir("plugin");
    let rec_dir = dir.join("Tree (b26b3312)");
    std::fs::create_dir_all(rec_dir.join("frames")).unwrap();
    let frame = Frame { width: 40, height: 30, rgb: vec![128; 40 * 30 * 3] };
    std::fs::write(rec_dir.join("frames/f_000001.jpg"), recorder::encode_jpeg(&frame).unwrap()).unwrap();
    std::fs::write(rec_dir.join("frames/f_000002.jpg"), recorder::encode_jpeg(&Frame { rgb: vec![30; 40 * 30 * 3], ..frame }).unwrap()).unwrap();
    let meta = json!({
        "version": 1, "id": "b26b3312-1111-4222-a333-444455556666", "name": "Tree", "docPath": "C:\\art\\Tree.psd",
        "created": 1, "updated": 2, "state": "recording", "workMs": 600000, "nextIndex": 3,
        "frames": [
            {"n": 1, "t": 10, "w": 1000, "x": 40, "y": 30, "b": 100, "s": "abcd1234", "h": 55, "u": 0},
            {"n": 2, "t": 20, "w": 2000, "x": 40, "y": 30, "b": 100, "s": "abcd1234", "h": 56, "u": 0, "future": true}
        ],
        "exports": [], "pluginOnly": {"keep": 1}
    });
    std::fs::write(rec_dir.join("recording.json"), meta.to_string()).unwrap();
    let r = store::load(&rec_dir).unwrap();
    assert_eq!((r.meta.frames.len(), r.meta.bytes, r.meta.recorded), (2, 200, 20));
    let mut s = Session::new();
    s.execute(SETTINGS, json!({"folder": dir.to_string_lossy()})).unwrap();
    let listed = s.execute(LIST, json!({})).unwrap();
    assert_eq!(listed["recordings"][0]["frames"], 2);
    let out = dir.join("seq");
    let done = s
        .execute(EXPORT, json!({"recording": r.meta.id, "path": out.to_string_lossy(), "encoder": "frames", "lengthSec": 1, "remember": false}))
        .unwrap();
    assert_eq!(done["videoFrames"], 30);
    assert!(out.join("frame_000030.jpg").is_file());
    // The export is written back, with the plug-in's own keys intact.
    let back: serde_json::Value = serde_json::from_slice(&std::fs::read(rec_dir.join("recording.json")).unwrap()).unwrap();
    assert_eq!(back["exports"][0]["mode"], "frames");
    assert_eq!(back["pluginOnly"]["keep"], 1);
    assert_eq!(back["frames"][1]["future"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- recording ---------------------------------------------------------------------------------

#[test]
fn recording_captures_edits_but_not_visibility_or_unchanged_states() {
    let dir = temp_dir("record");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    assert_eq!(frames(&s).len(), 1, "the document as it was when recording started");
    let rec_dir = track(&s).rec.dir.clone();
    assert!(rec_dir.join("frames/f_000001.jpg").is_file());

    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    assert_eq!(frames(&s).len(), 2);

    // Hiding a layer is a history step here, but never a frame.
    s.execute("layer.new.layer", json!({})).unwrap();
    settle(&mut s, &mut t);
    let n = frames(&s).len();
    assert_eq!(n, 2, "an empty new layer changes nothing visible");
    s.execute("layer.hideLayers", json!({})).unwrap();
    settle(&mut s, &mut t);
    assert_eq!(frames(&s).len(), n, "show/hide is not recorded");

    // Edits wait for a quiet moment.
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    t += 10.0;
    s.timelapse_tick_at(t, false);
    let due = s.timelapse_tick_at(t + 1.0, true).unwrap();
    assert!(due > t, "not yet");
    assert_eq!(frames(&s).len(), n);

    // The recording is saved, and pausing stops it.
    s.execute(PAUSE, json!({})).unwrap();
    let saved = store::load(&rec_dir).unwrap();
    assert_eq!(saved.meta.state, "paused");
    assert_eq!(saved.meta.frames.len(), frames(&s).len());
    assert!(s.execute(PAUSE, json!({})).is_err(), "already paused");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_hides_frames_redo_restores_them_and_a_new_edit_drops_them() {
    let dir = temp_dir("undo");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    s.execute("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 0), (3, 0)]);

    assert!(s.undo());
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 0), (3, 1)], "the green frame is undone");
    assert_eq!(s.execute(STATUS, json!({})).unwrap()["recording"]["frames"], 2);
    assert!(s.redo());
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 0), (3, 0)], "redo brings it back");

    assert!(s.undo());
    assert!(s.undo());
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 1), (3, 1)], "two steps back");
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    settle(&mut s, &mut t);
    let f = frames(&s);
    assert_eq!(f.len(), 4);
    assert_eq!(&f[1..3], &[(2, 1), (3, 1)]);
    // Those steps are gone: no undo or redo brings their frames back.
    assert!(s.undo());
    assert!(s.redo());
    settle(&mut s, &mut t);
    assert_eq!(&frames(&s)[1..3], &[(2, 1), (3, 1)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reference_layers_are_hidden_in_frames_only() {
    let dir = temp_dir("reference");
    let mut s = recording(&dir);
    s.execute("layer.new.layer", json!({"name": "Sketch"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    let r = s.execute(REFERENCE, json!({})).unwrap();
    assert_eq!((r["reference"].as_bool(), r["changed"].as_u64()), (Some(true), Some(1)));
    let st = s.active().unwrap();
    let top = st.doc.layers.last().unwrap();
    assert_eq!(top.name, "Sketch [ref]");
    assert!(top.visible, "still visible to you");
    let frame = recorder::render_frame(&st.doc, 64, "[ref]").unwrap();
    assert_eq!(&frame.rgb[..3], &[255, 0, 0], "the frame shows what's under it");
    // Toggling again removes the tag.
    s.execute(REFERENCE, json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.last().unwrap().name, "Sketch");
    assert_eq!(without_tag("[REF] Sketch  one", "[ref]"), "Sketch one");
    assert_eq!(without_tag("[ref]", "[ref]"), "Layer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn work_time_counts_activity_and_stops_when_idle() {
    let dir = temp_dir("work");
    let mut s = recording(&dir);
    s.execute(SETTINGS, json!({"idleSec": 5})).unwrap();
    // The folder didn't change, so the recording carries on.
    assert!(s.execute(STATUS, json!({})).unwrap()["recording"]["recording"].as_bool().unwrap());
    let base = recorder::mono_ms();
    s.timelapse_tick_at(base, true);
    s.timelapse_tick_at(base + 2000.0, true);
    let w0 = track(&s).work_now(base + 2000.0, 5000.0);
    // A minute of nothing counts as 5 s at most.
    let w1 = track(&s).work_now(base + 62_000.0, 5000.0);
    assert!(w1 - w0 <= 5000, "{w0} → {w1}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_writes_a_frame_sequence_with_the_plan() {
    let dir = temp_dir("export");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    for c in ["#0000ff", "#00ff00", "#ffff00"] {
        s.execute("edit.fill", json!({"contents": "color", "color": c})).unwrap();
        settle(&mut s, &mut t);
    }
    let out = dir.join("out");
    let r = s
        .execute(EXPORT, json!({"path": out.to_string_lossy(), "encoder": "frames", "lengthSec": 2, "holdSec": 1, "flash": true, "timer": true, "deflicker": true}))
        .unwrap();
    assert_eq!(r["frames"], 4);
    assert_eq!(r["videoFrames"], 60 + 30 + 18);
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)));
    let n = std::fs::read_dir(&out).unwrap().count();
    assert_eq!(n, 108);
    let (w, h, first) = export::decode_frame(&out.join("frame_000001.jpg")).unwrap();
    assert_eq!((w, h), (64, 48));
    assert!(first[0] > 200 && first[2] < 60, "starts red");
    assert!(track(&s).rec.dir.join(deflicker::THUMBS_FILE).is_file(), "flicker thumbnails cached");
    assert_eq!(track(&s).rec.meta.exports.len(), 1);
    // The choices are remembered for next time.
    assert_eq!(s.timelapse_settings().export.hold_sec, 1.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// MP4 through a real ffmpeg, when one is available (`PHOTOCRAFT_FFMPEG`).
#[test]
fn export_makes_an_mp4_with_ffmpeg() {
    let Ok(ff) = std::env::var("PHOTOCRAFT_FFMPEG") else { return };
    let dir = temp_dir("mp4");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    let out = dir.join("video.mp4");
    let r = s.execute(EXPORT, json!({"path": out.to_string_lossy(), "ffmpeg": ff, "encoder": "ffmpeg"})).unwrap();
    assert_eq!(r["mode"], "ffmpeg");
    assert_eq!(r["videoFrames"], 150);
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.len() > 1000 && &bytes[4..8] == b"ftyp", "an MP4");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_ffmpeg_is_an_error_that_says_so() {
    let dir = temp_dir("noffmpeg");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    let out = dir.join("v.mp4");
    let e = s.execute(EXPORT, json!({"path": out.to_string_lossy(), "ffmpeg": dir.join("nope.exe").to_string_lossy(), "remember": false})).unwrap_err();
    assert!(e.to_string().contains("ffmpeg wasn't found"), "{e}");
    assert!(!out.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reopening_a_recorded_file_resumes_its_recording() {
    let dir = temp_dir("resume");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    // As the app's Save As does (the engine has no file.saveAs): write, then take the path.
    let file = dir.join("art.psd");
    let path = file.to_string_lossy().into_owned();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    s.active_mut().unwrap().saved_to(path.clone());
    settle(&mut s, &mut t);
    let id = track(&s).rec.meta.id.clone();
    s.execute("file.close", json!({"save": false})).unwrap();
    s.timelapse_tick_at(t + 10.0, false);
    assert!(s.timelapse.tracks.is_empty());
    crate::file_cmds::open_bytes_as(&mut s, "art.psd", &std::fs::read(&file).unwrap(), None, Some(path)).unwrap();
    s.timelapse_tick_at(t + 20.0, false);
    let tr = track(&s);
    assert_eq!(tr.rec.meta.id, id);
    assert!(tr.recording, "it was recording when closed");
    assert!(s.timelapse_messages().iter().any(|m| m.contains("Resumed")));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- linking: XMP id, Save As ------------------------------------------------------------------

#[test]
fn the_recording_id_is_written_into_xmp_like_the_plug_in_does() {
    let ns = store::XMP_NS;
    // No XMP: a new packet.
    let fresh = store::xmp_with_recording_id(None, "abc-1");
    assert!(fresh.starts_with("<?xpacket") && fresh.contains("</rdf:RDF>"));
    assert_eq!(store::xmp_recording_id(&fresh).as_deref(), Some("abc-1"));
    // XMP without the namespace: a description is added, the rest kept.
    let other = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r"><rdf:Description rdf:about="" xmlns:dc="d"><dc:title>T</dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let added = store::xmp_with_recording_id(Some(other), "abc-2");
    assert!(added.contains("<dc:title>T</dc:title>"));
    assert_eq!(store::xmp_recording_id(&added).as_deref(), Some("abc-2"));
    // An id already there under another prefix, as attribute or element: replaced in place.
    let attr = format!(r#"<rdf:RDF><rdf:Description xmlns:ns9="{ns}" ns9:RecordingID = 'old'/></rdf:RDF>"#);
    let a = store::xmp_with_recording_id(Some(&attr), "new");
    assert_eq!(a, format!(r#"<rdf:RDF><rdf:Description xmlns:ns9="{ns}" ns9:RecordingID = 'new'/></rdf:RDF>"#));
    let elem = format!(r#"<rdf:RDF><rdf:Description xmlns:t="{ns}"><t:RecordingID>old</t:RecordingID></rdf:Description></rdf:RDF>"#);
    let e = store::xmp_with_recording_id(Some(&elem), "new");
    assert_eq!(store::xmp_recording_id(&e).as_deref(), Some("new"));
    assert_eq!(e.matches("RecordingID").count(), 2, "one element, not a second id");
    // A hostile id can't break out of the attribute.
    let x = store::xmp_with_recording_id(None, r#"a"/><evil x=""#);
    assert!(!x.contains("<evil"));
}

#[test]
fn recording_links_the_document_without_making_it_dirty() {
    let dir = temp_dir("xmp");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute(SETTINGS, json!({"folder": dir.to_string_lossy()})).unwrap();
    {
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
    }
    let steps = s.active().unwrap().history.past_len();
    s.execute(RECORD, json!({})).unwrap();
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    let id = track(&s).rec.meta.id.clone();
    let st = s.active().unwrap();
    assert_eq!(st.doc.metadata.xmp.as_deref().and_then(store::xmp_recording_id), Some(id.clone()));
    assert_eq!(st.saved_revision, st.revision, "still clean");
    assert_eq!(st.history.past_len(), steps, "no history step");
    assert_eq!(frames(&s).len(), 1, "linking is not an edit");

    // Edits keep it; undoing to a state from before it was written writes it again, and the undo
    // is still seen as one.
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 0)]);
    assert!(s.undo());
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 1)], "the undo flagged its frame");
    assert_eq!(s.active().unwrap().doc.metadata.xmp.as_deref().and_then(store::xmp_recording_id), Some(id.clone()));
    assert!(s.redo());
    settle(&mut s, &mut t);
    assert_eq!(frames(&s), vec![(1, 0), (2, 0)], "redo after the relink is still a redo");

    // A saved file carries the id: renamed or moved, it finds its recording.
    let file = dir.join("moved.psd").to_string_lossy().into_owned();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, &file, None).unwrap();
    s.active_mut().unwrap().saved_to(file.clone());
    t += 10.0;
    s.timelapse_tick_at(t, false);
    s.execute("file.close", json!({"save": false})).unwrap();
    s.timelapse_tick_at(t + 10.0, false);
    assert!(s.timelapse.tracks.is_empty());
    let elsewhere = dir.join("elsewhere.psd");
    std::fs::rename(&file, &elsewhere).unwrap();
    let path = elsewhere.to_string_lossy().into_owned();
    crate::file_cmds::open_bytes_as(&mut s, "elsewhere.psd", &std::fs::read(&elsewhere).unwrap(), None, Some(path)).unwrap();
    s.timelapse_tick_at(t + 20.0, false);
    assert_eq!(track(&s).rec.meta.id, id, "found by its XMP id, not its path");
    assert!(track(&s).recording, "and it resumes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_as_renames_the_recording_and_its_folder() {
    let dir = temp_dir("rename");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    let id = track(&s).rec.meta.id.clone();
    let short: String = id.chars().take(8).collect();
    let old_dir = track(&s).rec.dir.clone();
    assert!(old_dir.file_name().unwrap().to_string_lossy().starts_with("Untitled"));

    // While a timelapse is exported nothing is renamed (the export reads the folder).
    let path = dir.join("Tree.psd").to_string_lossy().into_owned();
    s.active_mut().unwrap().saved_to(path.clone());
    let Session { docs, timelapse, .. } = &mut s;
    timelapse.sync_docs(docs, &[], true, t);
    assert_eq!(track(&s).rec.dir, old_dir);

    t += 10.0;
    s.timelapse_tick_at(t, false);
    let r = &track(&s).rec;
    assert_eq!(r.meta.name, "Tree");
    assert_eq!(r.dir, dir.join(format!("Tree ({short})")));
    assert_eq!(r.meta.doc_path, path);
    assert!(!old_dir.exists());
    assert!(r.frame_path(1).is_file(), "the frames moved with it");
    assert_eq!(store::load(&r.dir).unwrap().meta.name, "Tree");

    // Recording carries on in the new folder.
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    assert!(track(&s).rec.frame_path(2).is_file());

    // A folder of that name already there: the name changes, the folder stays.
    std::fs::create_dir_all(dir.join(format!("Bush ({short})"))).unwrap();
    s.active_mut().unwrap().saved_to(dir.join("Bush.psd").to_string_lossy().into_owned());
    t += 10.0;
    s.timelapse_tick_at(t, false);
    assert_eq!(track(&s).rec.meta.name, "Bush");
    assert_eq!(track(&s).rec.dir, dir.join(format!("Tree ({short})")));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- library -----------------------------------------------------------------------------------

#[test]
fn the_library_deletes_frames_then_the_empty_recording() {
    let dir = temp_dir("library");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    s.execute("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    settle(&mut s, &mut t);
    let id = track(&s).rec.meta.id.clone();
    let rec_dir = track(&s).rec.dir.clone();
    let listed = s.execute(LIST, json!({})).unwrap();
    let entry = &listed["recordings"][0];
    assert_eq!((entry["frames"].as_u64(), entry["recording"].as_bool(), entry["exported"].as_bool()), (Some(2), Some(true), Some(false)));
    assert_eq!(entry["lastExport"], json!(null));

    // Refused while it records.
    let e = s.execute(DELETE_FRAMES, json!({"recording": id})).unwrap_err();
    assert!(e.to_string().contains("pause"), "{e}");
    s.execute(PAUSE, json!({})).unwrap();
    let work = track(&s).rec.meta.work_ms;

    // Its frames still there: the folder can't go yet.
    let e = s.execute(DELETE_RECORDING, json!({"recording": id})).unwrap_err();
    assert!(e.to_string().contains("delete them first"), "{e}");

    let r = s.execute(DELETE_FRAMES, json!({"recording": id})).unwrap();
    assert_eq!((r["frames"].as_u64(), r["deletedFiles"].as_u64()), (Some(2), Some(2)));
    assert_eq!(std::fs::read_dir(rec_dir.join("frames")).unwrap().count(), 0);
    let tr = track(&s);
    assert!(tr.rec.meta.frames.is_empty());
    assert_eq!((tr.rec.meta.bytes, tr.rec.meta.work_ms), (0, work), "the work time stays");
    let on_disk = store::load(&rec_dir).unwrap();
    assert!(on_disk.meta.frames.is_empty());
    assert_eq!(s.execute(LIST, json!({})).unwrap()["recordings"][0]["frames"], 0);

    // A video saved in the folder survives the recording's deletion, and so does the folder.
    std::fs::write(rec_dir.join("video.mp4"), b"mp4").unwrap();
    let r = s.execute(DELETE_RECORDING, json!({"recording": id})).unwrap();
    assert_eq!(r["folderRemoved"], false);
    assert_eq!(r["kept"], json!(["video.mp4"]));
    assert!(rec_dir.join("video.mp4").is_file());
    assert!(!rec_dir.join("recording.json").exists());
    assert_eq!(s.execute(LIST, json!({})).unwrap()["recordings"], json!([]));
    assert!(s.timelapse.tracks.is_empty(), "the open document lets go of it");
    // …and auto record doesn't start a new one straight away.
    s.execute(SETTINGS, json!({"autoRecord": true})).unwrap();
    s.timelapse_tick_at(t + 10.0, false);
    assert!(s.timelapse.tracks.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_library_deletes_a_closed_recording_and_its_folder() {
    let dir = temp_dir("library-closed");
    let mut s = recording(&dir);
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    let id = track(&s).rec.meta.id.clone();
    let rec_dir = track(&s).rec.dir.clone();
    s.execute("file.close", json!({"save": false})).unwrap();
    s.timelapse_tick_at(t + 10.0, false);
    s.execute(DELETE_FRAMES, json!({"recording": id})).unwrap();
    let r = s.execute(DELETE_RECORDING, json!({"recording": id})).unwrap();
    assert_eq!(r["folderRemoved"], true);
    assert!(!rec_dir.exists());
    assert!(s.execute(DELETE_RECORDING, json!({"recording": id})).is_err(), "gone");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- never panic -------------------------------------------------------------------------------

#[test]
fn bad_params_are_errors() {
    let mut s = Session::new();
    assert!(s.execute(RECORD, json!({})).is_err(), "no document");
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert!(s.execute(RECORD, json!({})).unwrap_err().to_string().contains("folder"), "no folder yet");
    for p in [
        json!({"quality": "ultra"}),
        json!({"idleSec": -1}),
        json!({"idleSec": "x"}),
        json!({"referenceTag": ""}),
        json!({"autoRecord": 3}),
        json!({"export": 5}),
        json!({"export": {"lengthSec": 0}}),
        json!({"export": {"corner": "middle"}}),
        json!([1, 2]),
    ] {
        assert!(s.execute(SETTINGS, p.clone()).is_err(), "{p}");
    }
    for p in [json!({}), json!({"path": ""}), json!({"path": 5}), json!({"path": "x.mp4", "fps": 0}), json!({"path": "x.mp4", "introSec": f64::MAX}), json!(null)] {
        assert!(s.execute(EXPORT, p.clone()).is_err(), "{p}");
    }
    assert!(s.execute(EXPORT, json!({"path": "x.mp4", "recording": "missing"})).is_err());
    assert!(s.execute(CAPTURE, json!({})).is_err(), "not recording");
    assert!(s.execute(REFERENCE, json!({"reference": "yes"})).is_err());
    // Queries always answer.
    assert_eq!(s.execute(STATUS, json!({"anything": 1})).unwrap()["recording"], json!(null));
    assert_eq!(s.execute(LIST, json!({})).unwrap()["recordings"], json!([]));
    for id in [DELETE_FRAMES, DELETE_RECORDING] {
        for p in [json!(null), json!([]), json!({}), json!({"recording": ""}), json!({"recording": 7}), json!({"recording": "missing"})] {
            assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
        }
    }
}

#[test]
fn a_folder_that_cannot_be_written_never_panics() {
    let dir = temp_dir("blocked");
    let blocker = dir.join("file");
    std::fs::write(&blocker, b"not a folder").unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute(SETTINGS, json!({"folder": blocker.join("inside").to_string_lossy()})).unwrap();
    assert!(s.execute(RECORD, json!({})).is_err());
    let mut t = recorder::mono_ms();
    settle(&mut s, &mut t);
    let _ = std::fs::remove_dir_all(&dir);
}
