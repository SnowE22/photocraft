use super::*;

/// A fresh scratch directory per test (removed first, so a rerun starts clean).
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("photocraft-gigapixel-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A `w` × `h` PNG filled with `color`, written to `path`.
fn png(path: &Path, w: u32, h: u32, color: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
    s.execute("file.saveACopy", json!({"path": path.to_string_lossy(), "layers": false})).unwrap();
}

fn red_document(s: &mut Session) {
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
}

/// The result arrives as Photoshop's plug-in leaves it: the document at the upscaled size and the
/// image on top as "Topaz Gigapixel layer".
#[test]
fn a_result_resizes_the_document_and_lands_on_top() {
    let dir = temp_dir("apply");
    let result = dir.join("photo-gigapixel-std-2x.png");
    png(&result, 64, 48, "#0000ff");
    let mut s = Session::new();
    red_document(&mut s);
    s.execute("layer.new.layer", json!({"name": "Under"})).unwrap();
    s.execute("layer.select", json!({"layer": s.active().unwrap().doc.layers[0].id.0})).unwrap();
    let r = apply(&mut s, &json!({"result": result.to_string_lossy()})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64(), r["scale"].as_f64()), (Some(64), Some(48), Some(2.0)));
    let st = s.active().unwrap();
    assert_eq!((st.doc.size.width, st.doc.size.height), (64, 48));
    let top = st.doc.layers.last().unwrap();
    assert_eq!(top.name, LAYER_NAME, "on top even though the bottom layer was active");
    assert_eq!(st.active_layer, Some(top.id));
    let LayerContent::Raster(px) = &top.content else { panic!("a pixel layer") };
    assert_eq!(px.pixel(10, 10), vec![0.0, 0.0, 1.0, 1.0], "blue");
    assert_eq!(st.history.undo_label(), Some(LABEL));
    // The same size: no Image Size step, just the layer.
    let before = s.active().unwrap().history.past_len();
    apply(&mut s, &json!({"result": result.to_string_lossy()})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The document the image came from gets it, even after another one became active.
#[test]
fn a_result_goes_to_its_own_document() {
    let dir = temp_dir("document");
    let result = dir.join("a-gigapixel.png");
    png(&result, 64, 48, "#00ff00");
    let mut s = Session::new();
    red_document(&mut s);
    let first = s.active().unwrap().doc.id.0;
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    apply(&mut s, &json!({"result": result.to_string_lossy(), "document": first})).unwrap();
    let st = s.active().unwrap();
    assert_eq!(st.doc.id.0, first);
    assert_eq!((st.doc.size.width, st.doc.size.height), (64, 48));
    assert!(apply(&mut s, &json!({"result": result.to_string_lossy(), "document": 999_999_999})).is_err(), "closed");
    assert!(apply(&mut s, &json!({"result": result.to_string_lossy(), "document": "one"})).is_err());
    assert!(apply(&mut s, &json!({"result": dir.join("missing.png").to_string_lossy()})).is_err());
    assert!(apply(&mut s, &json!({"result": 5})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_line_arguments() {
    let (i, o) = (Path::new("C:/t/in.tif"), Path::new("C:/t/out"));
    let a = cli_args(&json!({"model": "hf", "scale": 2, "denoise": 30}), i, o).unwrap();
    assert_eq!(a, ["-i", "C:/t/in.tif", "-o", "C:/t/out", "--cf", "-m", "hf", "--scale", "2", "--dn", "30", "-f", "tif", "--bd", "0", "-q"].map(String::from));
    assert_eq!(cli_args(&json!({"width": 4000}), i, o).unwrap()[5..7], ["--width", "4000"].map(String::from), "Gigapixel's own model by default");
    assert_eq!(cli_args(&json!({"model": "std", "scale": 4}), i, o).unwrap()[5..9], ["-m", "std", "--mv", "2"].map(String::from), "Standard V2");
    assert_eq!(cli_args(&json!({"model": "art", "modelVersion": 3, "scale": 4}), i, o).unwrap()[5..9], ["-m", "art", "--mv", "3"].map(String::from));
    for bad in [
        json!({}),
        json!({"scale": 2, "width": 10}),
        json!({"scale": 0}),
        json!({"scale": 99}),
        json!({"scale": "2"}),
        json!({"scale": 2, "model": "nope"}),
        json!({"scale": 2, "model": 3}),
        json!({"scale": 2, "denoise": 500}),
        json!({"height": f64::NAN}),
        json!({"scale": 2, "modelVersion": 2}),
        json!({"scale": 2, "model": "std", "modelVersion": 0}),
    ] {
        assert!(cli_args(&bad, i, o).is_err(), "{bad}");
    }
}

/// The shell's watcher: a file is the result once Gigapixel finished writing it (its size held
/// between two looks), named after the input; other files are not; a rewritten input is.
#[test]
fn the_watcher_waits_for_a_finished_export() {
    let dir = temp_dir("watch");
    let input = dir.join("photo.tif");
    std::fs::write(&input, b"tiff").unwrap();
    let mut p = Pending::from_result(&json!({"waiting": true, "document": 7, "folder": dir.to_string_lossy(), "input": input.to_string_lossy()})).unwrap();
    assert_eq!(p.document, 7);
    assert_eq!(p.poll(), None, "nothing yet");
    std::fs::write(dir.join("notes.txt"), b"x").unwrap();
    std::fs::write(dir.join("other.png"), b"x").unwrap();
    assert_eq!(p.poll(), None, "not images of the input");
    let out = dir.join("photo-gigapixel-standard v2-2x.tif");
    std::fs::write(&out, b"part").unwrap();
    assert_eq!(p.poll(), None, "seen once: maybe still being written");
    std::fs::write(&out, b"partial").unwrap();
    assert_eq!(p.poll(), None, "grew: still being written");
    assert_eq!(p.poll(), Some(out.clone()), "held its size");
    // Saved over the input instead.
    let dir2 = temp_dir("watch-overwrite");
    let input2 = dir2.join("photo.tif");
    std::fs::write(&input2, b"tiff").unwrap();
    let mut q = Pending::from_result(&json!({"waiting": true, "document": 1, "folder": dir2.to_string_lossy(), "input": input2.to_string_lossy()})).unwrap();
    assert_eq!(q.poll(), None);
    std::fs::write(&input2, b"a much bigger upscaled tiff").unwrap();
    assert_eq!(q.poll(), None);
    assert_eq!(q.poll(), Some(input2.clone()));
    assert!(Pending::from_result(&json!({"waiting": false})).is_none());
    assert!(Pending::from_result(&json!({"waiting": true})).is_none(), "no folder");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn the_installed_app_is_found_where_it_installs_or_where_the_environment_says() {
    let dir = temp_dir("installed");
    // Not "Gigapixel.exe": on a case-insensitive disk that is the command-line tool's name too.
    let app = dir.join("Topaz app.exe");
    std::fs::write(&app, b"").unwrap();
    let env = |k: &str| (k == "PHOTOCRAFT_GIGAPIXEL").then(|| app.clone().into_os_string());
    let found = installed_with(&env).unwrap();
    assert_eq!(found.app, app);
    assert_eq!(found.cli, None, "no command-line tool beside it");
    if cfg!(target_os = "windows") {
        std::fs::write(dir.join("gigapixel.exe"), b"").unwrap();
        assert_eq!(installed_with(&env).unwrap().cli, Some(dir.join("gigapixel.exe")));
        let pf = dir.join("pf");
        let real = pf.join("Topaz Labs LLC").join("Topaz Gigapixel AI");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("Topaz Gigapixel AI.exe"), b"").unwrap();
        let env2 = |k: &str| (k == "ProgramFiles").then(|| pf.clone().into_os_string());
        assert_eq!(installed_with(&env2).unwrap().app, real.join("Topaz Gigapixel AI.exe"));
    }
    assert!(installed_with(&|_| None).is_none_or(|i| !i.app.starts_with(&dir)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Harnesses that run every command never start Gigapixel; bad CLI params fail first anyway.
#[test]
fn launching_can_be_disabled() {
    disable_launching();
    let mut s = Session::new();
    red_document(&mut s);
    assert!(run(&mut s, &json!({})).unwrap_err().to_string().contains("disabled"));
    assert!(run(&mut s, &json!({"scale": 2})).unwrap_err().to_string().contains("disabled"));
    assert!(run(&mut s, &json!({"scale": 0})).unwrap_err().to_string().contains("scale"));
}

#[test]
fn only_rgb_and_gray_8_and_16_bit_documents_go() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 32})).unwrap();
    assert!(check_document(&s.active().unwrap().doc).is_err());
    s.execute("file.new", json!({"width": 8, "height": 8, "mode": "cmyk"})).unwrap();
    assert!(check_document(&s.active().unwrap().doc).is_err());
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 16})).unwrap();
    assert!(check_document(&s.active().unwrap().doc).is_ok());
    assert_eq!(safe_stem("My photo: v2?.psd"), "My photo v2");
    assert_eq!(safe_stem("***"), "image");
}
