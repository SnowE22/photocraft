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
    assert_eq!(a, ["-i", "C:/t/in.tif", "-o", "C:/t/out", "--cf", "-m", "hf", "--mv", "2", "--scale", "2", "--dn", "30", "-f", "tif", "--bd", "0", "-q"].map(String::from), "High Fidelity V2: a bare -m hf fails");
    let a = cli_args(&json!({"model": "recovery", "modelVersion": 2, "scale": 2, "detail": 30, "faceRecovery": 80, "faceRecoveryVersion": 2}), i, o).unwrap();
    assert_eq!(a[5..15], ["-m", "recovery", "--mv", "2", "--scale", "2", "--dt", "30", "--fr", "80"].map(String::from));
    assert_eq!(a[15..17], ["--frv", "2"].map(String::from));
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
        json!({"scale": 2, "faceRecoveryVersion": 1.5}),
        json!({"scale": 2, "detail": -1}),
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

/// A stand-in for Gigapixel's command-line tool in `dir`: it copies its input (`-i`) into its
/// output folder (`-o`) and appends its arguments to `dir/args.txt`.
fn fake_cli(dir: &Path) -> PathBuf {
    script(
        dir,
        "fake-gigapixel",
        "@echo off\r\nmkdir \"%~4\"\r\ncopy /y \"%~2\" \"%~4\\\" >nul\r\necho %* >> \"%~dp0args.txt\"\r\n",
        "#!/bin/sh\nmkdir -p \"$4\"\ncp \"$2\" \"$4/\"\necho \"$@\" >> \"$(dirname \"$0\")/args.txt\"\n",
    )
}

/// An executable script in `dir`: `windows` as a `.cmd` file there, `unix` elsewhere.
fn script(dir: &Path, name: &str, windows: &str, unix: &str) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.cmd"));
        std::fs::write(&p, windows).unwrap();
        p
    } else {
        let p = dir.join(name);
        std::fs::write(&p, unix).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }
}

/// The runs the stand-in saw, one line of arguments each.
fn runs(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("args.txt")).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

fn save_preset(s: &mut Session, name: &str, settings: Value) -> Result<Value> {
    s.execute(presets::SAVE_ID, json!({"name": name, "settings": settings}))
}

#[test]
fn presets_are_saved_checked_renamed_and_deleted() {
    let mut s = Session::new();
    save_preset(&mut s, "Faces 2x", json!({"model": "std", "scale": 2, "faceRecovery": 60, "denoise": null})).unwrap();
    save_preset(&mut s, "Art 4x", json!({"model": "art", "scale": 4})).unwrap();
    save_preset(&mut s, "Both", json!({"presets": ["Faces 2x", "art 4X"], "combine": "layers"})).unwrap();
    let list = s.execute(presets::LIST_ID, json!({})).unwrap();
    assert_eq!(list["presets"][0], json!({"name": "Faces 2x", "settings": {"model": "std", "scale": 2, "faceRecovery": 60}}), "nulls dropped");
    assert_eq!(list["presets"][2]["settings"], json!({"presets": ["Faces 2x", "art 4X"], "combine": "layers"}));
    // Overwriting by name keeps the place.
    save_preset(&mut s, "faces 2X", json!({"model": "std", "scale": 3})).unwrap();
    assert_eq!(s.presets.gigapixel_presets[0].settings["scale"], json!(3));
    assert_eq!(s.presets.gigapixel_presets.len(), 3);
    for (name, bad) in [
        ("x", json!({})),
        ("x", json!({"scale": 0})),
        ("x", json!({"scale": 2, "colour": 1})),
        ("x", json!({"presets": ["Faces 2x"]})),
        ("x", json!({"presets": ["Faces 2x", "Missing"]})),
        ("x", json!({"presets": ["Faces 2x", "Both"]})),
        ("x", json!({"presets": ["Faces 2x", "Art 4x"], "combine": "blend"})),
        ("x", json!({"presets": ["Faces 2x", "Art 4x"], "scale": 2})),
        ("x", json!({"presets": "Faces 2x"})),
        ("x", json!({"presets": [1, 2]})),
        ("Self", json!({"presets": ["Self", "Art 4x"]})),
        ("  ", json!({"scale": 2})),
        ("x", json!([1])),
    ] {
        assert!(save_preset(&mut s, name, bad.clone()).is_err(), "{name}: {bad}");
    }
    // A preset others combine stays one upscale.
    assert!(save_preset(&mut s, "Art 4x", json!({"presets": ["Faces 2x", "Faces 2x"]})).is_err());
    // Renaming follows into the presets combining it; renaming onto another name is refused.
    s.execute(presets::SAVE_ID, json!({"name": "Painting 4x", "settings": {"model": "art", "scale": 4}, "replace": "art 4x"})).unwrap();
    assert_eq!(s.presets.gigapixel_presets[1].name, "Painting 4x");
    assert_eq!(s.presets.gigapixel_presets[2].settings["presets"], json!(["Faces 2x", "Painting 4x"]));
    assert!(s.execute(presets::SAVE_ID, json!({"name": "Both", "settings": {"scale": 2}, "replace": "Painting 4x"})).is_err());
    assert!(s.execute(presets::SAVE_ID, json!({"name": "New", "settings": {"scale": 2}, "replace": "Nothing"})).is_err());
    // They persist with the other presets, and a damaged entry drops alone.
    let mut saved = s.presets.to_json(&s);
    saved["gigapixel_presets"].as_array_mut().unwrap().push(json!({"name": "Bad", "settings": {"scale": 99}}));
    let mut t = Session::new();
    t.load_presets_json(saved);
    assert_eq!(t.presets.gigapixel_presets, s.presets.gigapixel_presets);
    s.execute(presets::DELETE_ID, json!({"name": "both"})).unwrap();
    assert!(s.execute(presets::DELETE_ID, json!({"name": "both"})).is_err());
    assert!(s.execute(presets::DELETE_ID, json!({})).is_err());
    assert_eq!(s.presets.gigapixel_presets.len(), 2);
}

/// Unknown presets and bad combinations fail before anything is exported or started.
#[test]
fn bad_preset_runs_fail_first() {
    let mut s = Session::new();
    red_document(&mut s);
    save_preset(&mut s, "A", json!({"scale": 2})).unwrap();
    save_preset(&mut s, "B", json!({"scale": 2})).unwrap();
    save_preset(&mut s, "AB", json!({"presets": ["A", "B"]})).unwrap();
    for bad in [
        json!({"preset": "Nope"}),
        json!({"preset": 3}),
        json!({"presets": []}),
        json!({"presets": ["A", "Nope"]}),
        json!({"presets": ["A", "AB"]}),
        json!({"presets": ["A", "B", "A", "B", "A"]}),
        json!({"presets": ["A", 2]}),
        json!({"presets": "A"}),
        json!({"presets": ["A", "B"], "combine": "stack"}),
    ] {
        let e = run(&mut s, &bad).unwrap_err().to_string();
        assert!(!e.contains("disabled"), "{bad}: {e}");
    }
}

/// A preset runs Gigapixel's command-line tool with its settings; two presets either chain (the
/// second upscales the first's result: one layer) or each make a layer.
#[test]
fn presets_run_one_after_the_other_or_side_by_side() {
    let dir = temp_dir("fake-cli");
    let cli = fake_cli(&dir);
    FAKE_CLI.with(|f| *f.borrow_mut() = Some(cli));
    let mut s = Session::new();
    red_document(&mut s);
    save_preset(&mut s, "Faces", json!({"model": "std", "scale": 2, "faceRecovery": 60})).unwrap();
    save_preset(&mut s, "Art", json!({"model": "art", "width": 4000})).unwrap();
    save_preset(&mut s, "Faces then Art", json!({"presets": ["Faces", "Art"]})).unwrap();

    let r = s.execute(ID, json!({"preset": "faces"})).unwrap();
    assert_eq!(r["presets"], json!(["Faces"]));
    let calls = runs(&dir);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains("-m std --mv 2 --scale 2 --fr 60"), "{}", calls[0]);
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.last().unwrap().name, format!("{LAYER_NAME} (Faces)"));
    assert_eq!(st.history.undo_label(), Some(LABEL));

    let before = s.active().unwrap().doc.layers.len();
    s.execute(ID, json!({"preset": "Faces then Art"})).unwrap();
    let calls = runs(&dir);
    assert_eq!(calls.len(), 3);
    assert!(calls[1].contains("-m std") && calls[2].contains("-m art --width 4000"), "{calls:?}");
    assert!(calls[2].contains("step1"), "the second run reads the first one's result: {}", calls[2]);
    let st = s.active().unwrap();
    assert_eq!(st.doc.layers.len(), before + 1, "one result");
    assert_eq!(st.doc.layers.last().unwrap().name, format!("{LAYER_NAME} (Faces → Art)"));

    let before = st.doc.layers.len();
    let r = s.execute(ID, json!({"presets": ["Faces", "Art"], "combine": "layers"})).unwrap();
    assert_eq!(r["layers"].as_array().map(Vec::len), Some(2));
    let calls = runs(&dir);
    assert_eq!(calls.len(), 5);
    assert!(!calls[4].contains("step1"), "each reads the image: {}", calls[4]);
    let st = s.active().unwrap();
    let names: Vec<&str> = st.doc.layers.iter().skip(before).map(|l| l.name.as_str()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&format!("{LAYER_NAME} (Faces)").as_str()) && names.contains(&format!("{LAYER_NAME} (Art)").as_str()), "{names:?}");
    FAKE_CLI.with(|f| *f.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tool that makes no image is an error naming what it said; the document is unchanged and
/// the work folder is removed.
#[test]
fn a_run_that_makes_nothing_is_an_error() {
    let dir = temp_dir("fake-cli-fails");
    let cli = script(&dir, "fails", "@echo off\r\necho \"Could not load model\"\r\nexit /b 1\r\n", "#!/bin/sh\necho '\"Could not load model\"'\nexit 1\n");
    FAKE_CLI.with(|f| *f.borrow_mut() = Some(cli));
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "fails-test"})).unwrap();
    let layers = s.active().unwrap().doc.layers.len();
    let e = s.execute(ID, json!({"scale": 2})).unwrap_err().to_string();
    assert!(e.contains("made no image") && e.contains("Could not load model"), "{e}");
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    let base = std::env::temp_dir().join("PhotoCraft").join("Gigapixel");
    let left: Vec<String> = std::fs::read_dir(&base).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    assert!(!left.iter().any(|n| n.starts_with("fails-test")), "{left:?}");
    FAKE_CLI.with(|f| *f.borrow_mut() = None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// With the real Gigapixel installed: two presets chained, then side by side, on a small image.
/// `cargo test -p photocraft-engine --lib gigapixel_cmds::tests::real_gigapixel -- --ignored`
#[test]
#[ignore = "needs Topaz Gigapixel AI with its command-line tool"]
fn real_gigapixel_chains_two_presets() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#3080c0"})).unwrap();
    save_preset(&mut s, "Std 2x", json!({"model": "std", "scale": 2})).unwrap();
    save_preset(&mut s, "Art 2x", json!({"model": "art", "scale": 2})).unwrap();
    let r = s.execute(ID, json!({"presets": ["Std 2x", "Art 2x"], "combine": "chain"})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(256), Some(192)), "{r}");
    let r = s.execute(ID, json!({"presets": ["Std 2x", "Art 2x"], "combine": "layers"})).unwrap();
    assert_eq!(r["layers"].as_array().map(Vec::len), Some(2), "{r}");
}

/// The settings Gigapixel 8.4.4 wrote into real exports become settings the tool takes.
#[test]
fn settings_are_read_from_gigapixels_description() {
    let cases = [
        (
            "Upscaled with Gigapixel v8.4.4. 3072x4096 => 6144x8192 (2x) Model: Low Resolution V2, denoise: 0.07587824016809464, sharpen: 0.6943150758743286, decompression: 0.8566693067550659",
            json!({"model": "lowres", "modelVersion": 2, "scale": 2.0, "denoise": 8.0, "sharpen": 69.0, "compression": 86.0}),
        ),
        (
            "Upscaled with Gigapixel v8.4.4. 828x1104 => 3312x4416 (4x) Model: RecoveryV2, detail: 0.5 Face recovery version: 2, strength: 0.8, creativity: 0 (Realistic), selectedFaces: 1",
            json!({"model": "recovery", "modelVersion": 2, "scale": 4.0, "detail": 50.0, "faceRecovery": 80.0, "faceRecoveryVersion": 2}),
        ),
        (
            "Upscaled with Gigapixel v8.4.4. 768x1376 => 3072x5504 (4x) Model: Text Refine, denoise: 0.46242988109588623, sharpen: 0.6463850140571594, decompression: 0.20109833776950836",
            json!({"model": "text", "scale": 4.0, "denoise": 46.0, "sharpen": 65.0, "compression": 20.0}),
        ),
        (
            "Upscaled with Gigapixel v8.4.4. 3200x333 => 6400x666 (2x) @ -1 ppcm Model: Standard V2, denoise: 0.013314668089151382, sharpen: 0.3213237226009369, decompression: 0.7473556399345398",
            json!({"model": "std", "modelVersion": 2, "scale": 2.0, "denoise": 1.0, "sharpen": 32.0, "compression": 75.0}),
        ),
        // Redefine's numbers aren't the tool's strengths.
        (
            "Upscaled with Gigapixel v8.4.4. 828x1104 => 2484x3312 (3x) Model: RecoveryXL, denoise: 1, decompression: 0, sharpen: , sharpen: 0, sharpen: 3",
            json!({"model": "redefine", "scale": 3.0}),
        ),
        ("Upscaled with Gigapixel v8.4.4. 96x64 => 240x160 (2.5x) @ -1 ppcm Model: CGI, denoise: 0.5, sharpen: 0.07", json!({"model": "art", "scale": 2.5, "denoise": 50.0, "sharpen": 7.0})),
        // Past the tool's 16×: the width.
        ("Upscaled with Gigapixel v8.4.4. 100x50 => 2000x1000 (20x) Model: High Compression", json!({"model": "vc", "width": 2000.0})),
        ("Upscaled with Gigapixel v8.4.4. 1075x532 => 12124x6000 (11.2782x) Model: Rescue, detail: 0.82", json!({"model": "recovery", "modelVersion": 1, "scale": 11.28, "detail": 82.0})),
    ];
    for (text, want) in cases {
        assert_eq!(settings_from_description(text), Some(want), "{text}");
    }
    for bad in ["", "Upscaled with Gigapixel", "Upscaled with Gigapixel v8. Model: Something New", "Made with another app. Model: Standard V2", "Upscaled with Gigapixel v8.4.4. 0x0 => 5x5 Model: CGI"] {
        let r = settings_from_description(bad);
        assert!(r.is_none() || bad.contains("0x0"), "{bad}: {r:?}");
    }
    assert_eq!(describe_settings(&json!({"model": "lowres", "modelVersion": 2, "scale": 2.0})), "Low Resolution V2 2×");
    assert_eq!(describe_settings(&json!({"model": "art", "scale": 2.5})), "Art & CG 2.5×");
    assert_eq!(describe_settings(&json!({"width": 4000.0})), "Gigapixel 4000 px wide");
}

/// The round trip's result carries the settings Gigapixel embedded, wherever the file keeps them;
/// a file without them carries none.
#[test]
fn an_applied_export_reports_its_settings() {
    let dir = temp_dir("embedded");
    let plain = dir.join("photo-gigapixel.png");
    png(&plain, 64, 48, "#0000ff");
    let mut s = Session::new();
    red_document(&mut s);
    let r = apply(&mut s, &json!({"result": plain.to_string_lossy()})).unwrap();
    assert!(r.get("settings").is_none(), "{r}");
    assert_eq!(s.presets.gigapixel_last, None, "no settings came back");
    // A PNG with Gigapixel's text chunk appended before IEND, as Gigapixel writes it.
    let mut bytes = std::fs::read(&plain).unwrap();
    let text = b"Upscaled with Gigapixel v8.4.4. 32x24 => 64x48 (2x) Model: Standard V2, denoise: 0.2, sharpen: 0.1";
    let iend = bytes.len() - 12;
    let mut body = b"tEXtDescription\0".to_vec();
    body.extend_from_slice(text);
    let crc = body.iter().fold(!0u32, |c, &b| (0..8).fold(c ^ u32::from(b), |c, _| if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 }));
    let mut chunk = ((body.len() - 4) as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&(!crc).to_be_bytes());
    bytes.splice(iend..iend, chunk);
    let tagged = dir.join("tagged.png");
    std::fs::write(&tagged, &bytes).unwrap();
    assert_eq!(embedded_description(&tagged).as_deref(), std::str::from_utf8(text).ok());
    let r = apply(&mut s, &json!({"result": tagged.to_string_lossy()})).unwrap();
    let want = json!({"model": "std", "modelVersion": 2, "scale": 2.0, "denoise": 20.0, "sharpen": 10.0});
    assert_eq!(r["settings"], want);
    assert_eq!(s.presets.gigapixel_last, Some(want.clone()), "kept for Save Preset from Last Upscale");
    assert_eq!(s.execute(presets::LIST_ID, json!({})).unwrap()["last"], want);
    // Far from the start too (a TIFF directory written after the pixels).
    let mut big = vec![0u8; 6 << 20];
    big.extend_from_slice(text);
    big.push(0);
    let far = dir.join("far.bin");
    std::fs::write(&far, &big).unwrap();
    assert_eq!(embedded_description(&far).as_deref(), std::str::from_utf8(text).ok());
    assert_eq!(embedded_description(&dir.join("missing.tif")), None);
    let _ = std::fs::remove_dir_all(&dir);
}
