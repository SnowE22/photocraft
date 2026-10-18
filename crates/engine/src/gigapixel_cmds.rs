//! File › Automate › Topaz Gigapixel AI…: upscale the image in Topaz Gigapixel AI and bring the
//! result back, as Photoshop's Automate plug-in does: a new top layer, "Topaz Gigapixel layer",
//! over the document resized to the result's size.
//!
//! `file.automate.topazGigapixel` has two forms:
//! - `{}` (the menu item): the flattened image goes to Gigapixel's app as a TIFF in a folder of
//!   its own, opened there in the app's external-editor mode (an image file on its command line).
//!   The person picks the model and exports. With Gigapixel's export settings at their defaults
//!   (the source's folder, the name plus a `-gigapixel…` suffix) the result lands next to the
//!   TIFF, where the shell watches for it ([`Pending::poll`]) and then applies it with
//!   `file.automate.topazGigapixel.apply {"result": path}`.
//! - `{"model"?, "scale" | "width" | "height", …}`: upscale with Gigapixel's command-line tool
//!   (`gigapixel.exe`, Gigapixel 7.3 and later, as licensed) and apply the result at once, for
//!   actions, batch and agents.
//!
//! Gigapixel is never bundled: the installed app is found at its default place (Windows
//! `%ProgramFiles%\Topaz Labs LLC\Topaz Gigapixel AI`, macOS `/Applications/Topaz Gigapixel
//! AI.app`) or at `PHOTOCRAFT_GIGAPIXEL` (the app's executable).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const ID: &str = "file.automate.topazGigapixel";
/// Applies an upscaled image (needs no Gigapixel: any upscaled copy of the image will do).
pub const APPLY_ID: &str = "file.automate.topazGigapixel.apply";
/// The layer the result arrives as (the name Photoshop's plug-in gives it).
pub const LAYER_NAME: &str = "Topaz Gigapixel layer";
/// History label of the step that adds the result.
const LABEL: &str = "Topaz Gigapixel AI";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn has_document(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Set by test harnesses that run every command (`panic_hunt`): Gigapixel is never started.
static LAUNCH_DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Never start Gigapixel (its app or command-line tool) in this process.
pub fn disable_launching() {
    LAUNCH_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn launching_allowed() -> Result<()> {
    if LAUNCH_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(EngineError::Other("starting Topaz Gigapixel AI is disabled in this process".into()));
    }
    Ok(())
}

/// The installed Gigapixel: its app and, when present, its command-line tool.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub app: PathBuf,
    pub cli: Option<PathBuf>,
}

/// Where Gigapixel installs itself, from the environment `env` (so tests can fake it).
pub fn installed_with(env: &impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<Installed> {
    let mut apps: Vec<PathBuf> = Vec::new();
    if let Some(p) = env("PHOTOCRAFT_GIGAPIXEL").filter(|p| !p.is_empty()) {
        apps.push(PathBuf::from(p));
    }
    if cfg!(target_os = "windows") {
        for var in ["ProgramFiles", "ProgramW6432"] {
            if let Some(pf) = env(var).filter(|p| !p.is_empty()) {
                apps.push(PathBuf::from(pf).join("Topaz Labs LLC").join("Topaz Gigapixel AI").join("Topaz Gigapixel AI.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        apps.push(PathBuf::from("/Applications/Topaz Gigapixel AI.app/Contents/MacOS/Topaz Gigapixel AI"));
    }
    let app = apps.into_iter().find(|p| p.is_file())?;
    let cli = cli_beside(&app).filter(|p| p.is_file());
    Some(Installed { app, cli })
}

/// The command-line tool next to the app: `gigapixel.exe` beside it on Windows, the bundle's
/// `Contents/Resources/bin/gigapixel` on macOS.
fn cli_beside(app: &Path) -> Option<PathBuf> {
    let dir = app.parent()?;
    if cfg!(target_os = "macos") {
        Some(dir.parent()?.join("Resources").join("bin").join("gigapixel"))
    } else {
        Some(dir.join(if cfg!(target_os = "windows") { "gigapixel.exe" } else { "gigapixel" }))
    }
}

/// [`installed_with`] for this process's environment.
#[cfg(not(target_arch = "wasm32"))]
pub fn installed() -> Option<Installed> {
    installed_with(&|k| std::env::var_os(k))
}

#[cfg(target_arch = "wasm32")]
pub fn installed() -> Option<Installed> {
    None
}

/// The menu item needs an open RGB or Grayscale 8/16-bit document and Gigapixel on this computer.
fn enabled(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    check_document(&st.doc).map_err(|e| e.to_string())?;
    if installed().is_none() {
        return Err("Topaz Gigapixel AI is not installed (or set PHOTOCRAFT_GIGAPIXEL to its executable)".into());
    }
    Ok(())
}

/// Gigapixel reads 8- and 16-bit RGB and Grayscale images.
fn check_document(doc: &photocraft_doc::Document) -> Result<()> {
    use photocraft_color::{ColorMode, SampleType};
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(bad("Topaz Gigapixel AI takes RGB or Grayscale images: convert the document first (Image › Mode)"));
    }
    if doc.depth == SampleType::F32 {
        return Err(bad("Topaz Gigapixel AI takes 8 or 16 bits per channel: convert the document first (Image › Mode › 16 Bits/Channel)"));
    }
    Ok(())
}

/// A file name made of `name`'s letters, digits, spaces, dashes and underscores (at most 64).
fn safe_stem(name: &str) -> String {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let clean: String = stem.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).take(64).collect();
    let clean = clean.trim().to_string();
    if clean.is_empty() { "image".to_string() } else { clean }
}

/// The flattened image, written as a TIFF into a new folder of its own under the temporary
/// directory: `(folder, file)`.
#[cfg(not(target_arch = "wasm32"))]
fn export(s: &mut Session) -> Result<(PathBuf, PathBuf)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    check_document(&st.doc)?;
    let stem = safe_stem(&st.doc.name);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let folder = std::env::temp_dir().join("PhotoCraft").join("Gigapixel").join(format!("{stem}-{stamp:x}"));
    std::fs::create_dir_all(&folder).map_err(|e| EngineError::Other(format!("{}: {e}", folder.display())))?;
    let input = folder.join(format!("{stem}.tif"));
    let save = crate::commands::find("file.saveACopy").ok_or_else(|| EngineError::Other("Save a Copy is missing".into()))?;
    (save.run)(s, &json!({"path": input.to_string_lossy(), "layers": false}))?;
    Ok((folder, input))
}

/// A file's size and modification time, to tell when it was rewritten.
#[cfg(not(target_arch = "wasm32"))]
fn stamp(p: &Path) -> Option<(u64, std::time::SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Image files Gigapixel exports.
const IMAGE_EXTENSIONS: &[&str] = &["tif", "tiff", "png", "jpg", "jpeg", "dng"];

/// A round trip waiting for Gigapixel's export (kept by the shell between frames).
#[derive(Clone, Debug)]
pub struct Pending {
    /// The document the image came from ([`photocraft_doc::DocId`]).
    pub document: u64,
    pub folder: PathBuf,
    pub input: PathBuf,
    /// The input's size and time when Gigapixel got it: a rewrite means it saved over it.
    input_stamp: Option<(u64, std::time::SystemTime)>,
    /// Candidate files and their size at the last look: a file is complete once its size holds.
    sizes: HashMap<PathBuf, u64>,
}

impl Pending {
    /// The waiting round trip a `{}` call returned (its `folder`, `input` and `document`).
    pub fn from_result(r: &Value) -> Option<Pending> {
        if r.get("waiting").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let folder = PathBuf::from(r.get("folder")?.as_str()?);
        let input = PathBuf::from(r.get("input")?.as_str()?);
        #[cfg(not(target_arch = "wasm32"))]
        let input_stamp = stamp(&input);
        #[cfg(target_arch = "wasm32")]
        let input_stamp = None;
        Some(Pending { document: r.get("document")?.as_u64()?, folder, input, input_stamp, sizes: HashMap::new() })
    }

    /// The upscaled file, once Gigapixel has finished writing it: a new image in the folder
    /// whose name starts with the input's (Gigapixel's suffix settings add to it), or the input
    /// itself rewritten. A file counts once its size is the same at two looks in a row.
    pub fn poll(&mut self) -> Option<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let stem = self.input.file_stem()?.to_string_lossy().to_lowercase();
            let mut found: Vec<PathBuf> = std::fs::read_dir(&self.folder)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| *p != self.input && p.is_file())
                .filter(|p| p.extension().is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())))
                .filter(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase().contains(&stem)))
                .collect();
            if stamp(&self.input) != self.input_stamp {
                found.push(self.input.clone());
            }
            found.sort();
            for p in found {
                let Some((len, _)) = stamp(&p) else { continue };
                if len > 0 && self.sizes.get(&p) == Some(&len) {
                    return Some(p);
                }
                self.sizes.insert(p, len);
            }
            None
        }
        #[cfg(target_arch = "wasm32")]
        None
    }

    /// Remove the folder and what Gigapixel wrote into it.
    pub fn clean_up(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Only ever a folder this module made under the temporary directory.
            if self.folder.starts_with(std::env::temp_dir().join("PhotoCraft").join("Gigapixel")) {
                let _ = std::fs::remove_dir_all(&self.folder);
            }
        }
    }
}

/// `{}`: export and open the image in Gigapixel's app.
#[cfg(not(target_arch = "wasm32"))]
fn open_in_app(s: &mut Session) -> Result<Value> {
    launching_allowed()?;
    let gp = installed().ok_or_else(|| bad("Topaz Gigapixel AI is not installed (or set PHOTOCRAFT_GIGAPIXEL to its executable)"))?;
    let document = s.active().ok_or(EngineError::NoDocument)?.doc.id.0;
    let (folder, input) = export(s)?;
    // External-editor mode: the image path on the command line.
    std::process::Command::new(&gp.app).arg(&input).spawn().map_err(|e| EngineError::Other(format!("{}: {e}", gp.app.display())))?;
    Ok(json!({"waiting": true, "document": document, "folder": folder.to_string_lossy(), "input": input.to_string_lossy(), "app": gp.app.to_string_lossy()}))
}

#[cfg(target_arch = "wasm32")]
fn open_in_app(_: &mut Session) -> Result<Value> {
    Err(EngineError::Other("Topaz Gigapixel AI needs the desktop app".into()))
}

/// The CLI's model names (`-m`); Gigapixel takes more aliases, these are the documented ones.
pub const MODELS: &[&str] = &["std", "hf", "lowres", "art", "lines", "text", "vc", "redefine", "recovery"];

/// `gigapixel` arguments for `p` (`model`, `modelVersion`, `scale` | `width` | `height`,
/// `denoise`, `sharpen`, `compression`, `faceRecovery`), reading `input` and writing into `out`.
/// Without a model Gigapixel picks its own (Standard V2 in Gigapixel 8.4); `std` alone means
/// version 2, since Gigapixel 8.4 reads a bare `-m std` as a version 0 it may not have installed
/// ("Could not load model").
pub fn cli_args(p: &Value, input: &Path, out: &Path) -> Result<Vec<String>> {
    let mut a: Vec<String> = vec!["-i".into(), input.to_string_lossy().into_owned(), "-o".into(), out.to_string_lossy().into_owned(), "--cf".into()];
    let model = p.get("model").filter(|m| !m.is_null()).map(|m| m.as_str().ok_or_else(|| bad("`model` must be a name"))).transpose()?;
    let version = match p.get("modelVersion") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|v| (1..=20).contains(v)).ok_or_else(|| bad("`modelVersion` must be a whole number from 1 to 20"))?),
    };
    if let Some(m) = model {
        if !MODELS.contains(&m) {
            return Err(bad(format!("unknown `model` {m:?} (one of {})", MODELS.join(", "))));
        }
        a.extend(["-m".into(), m.into()]);
        match (version, m) {
            (Some(v), _) => a.extend(["--mv".into(), v.to_string()]),
            (None, "std") => a.extend(["--mv".into(), "2".into()]),
            (None, _) => {}
        }
    } else if version.is_some() {
        return Err(bad("`modelVersion` needs a `model`"));
    }
    let size = ["scale", "width", "height"].iter().filter(|k| p.get(**k).is_some_and(|v| !v.is_null())).count();
    if size != 1 {
        return Err(bad("give one of `scale`, `width` or `height`"));
    }
    let num = |k: &str, lo: f64, hi: f64| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)).map(Some).ok_or_else(|| bad(format!("`{k}` must be a number from {lo} to {hi}")))
            }
        }
    };
    if let Some(x) = num("scale", 1.0, 16.0)? {
        a.extend(["--scale".into(), format!("{x}")]);
    }
    if let Some(x) = num("width", 1.0, 65_000.0)? {
        a.extend(["--width".into(), format!("{}", x.round())]);
    }
    if let Some(x) = num("height", 1.0, 65_000.0)? {
        a.extend(["--height".into(), format!("{}", x.round())]);
    }
    for (key, flag) in [("denoise", "--dn"), ("sharpen", "--sh"), ("compression", "--cm"), ("faceRecovery", "--fr")] {
        if let Some(x) = num(key, 0.0, 100.0)? {
            a.extend([flag.into(), format!("{x}")]);
        }
    }
    // A TIFF back, at the input's bit depth.
    a.extend(["-f".into(), "tif".into(), "--bd".into(), "0".into(), "-q".into()]);
    Ok(a)
}

/// `{"model", "scale" | …}`: upscale with the command-line tool, then apply.
#[cfg(not(target_arch = "wasm32"))]
fn upscale_with_cli(s: &mut Session, p: &Value) -> Result<Value> {
    // Bad params fail before anything is written or started.
    cli_args(p, Path::new("in.tif"), Path::new("out"))?;
    launching_allowed()?;
    let gp = installed().ok_or_else(|| bad("Topaz Gigapixel AI is not installed (or set PHOTOCRAFT_GIGAPIXEL to its executable)"))?;
    let cli = gp.cli.ok_or_else(|| bad("this Gigapixel has no command-line tool (Gigapixel 7.3 or later has gigapixel.exe beside the app)"))?;
    let (folder, input) = export(s)?;
    let out = folder.join("out");
    let args = cli_args(p, &input, &out)?;
    let run = std::process::Command::new(&cli).args(&args).output().map_err(|e| EngineError::Other(format!("{}: {e}", cli.display())));
    let result = run.and_then(|o| {
        let made = out.join(input.file_name().unwrap_or_default());
        if made.is_file() {
            Ok(made)
        } else {
            let said = String::from_utf8_lossy(if o.stderr.is_empty() { &o.stdout } else { &o.stderr })
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .to_string();
            Err(EngineError::Other(format!("Topaz Gigapixel AI's command-line tool made no image (exit {}): {said}", o.status)))
        }
    });
    let applied = result.and_then(|made| apply(s, &json!({"result": made.to_string_lossy()})));
    let applied = applied.map(|mut v| {
        v["model"] = p.get("model").cloned().unwrap_or(Value::Null);
        v
    });
    let _ = std::fs::remove_dir_all(&folder);
    applied
}

#[cfg(target_arch = "wasm32")]
fn upscale_with_cli(_: &mut Session, _: &Value) -> Result<Value> {
    Err(EngineError::Other("Topaz Gigapixel AI needs the desktop app".into()))
}

/// `{"result": path, "document": id?}`: the document (by id; default the active one) resized to
/// the upscaled image, and the image added on top as [`LAYER_NAME`].
fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let path = crate::file_cmds::str_param(p, "result", APPLY_ID)?.to_string();
    if let Some(v) = p.get("document").filter(|v| !v.is_null()) {
        let id = v.as_u64().ok_or_else(|| bad("`document` must be a document id"))?;
        let i = s.documents().iter().position(|d| d.doc.id.0 == id).ok_or_else(|| bad("the document the image came from is closed"))?;
        s.set_active(i);
    }
    let bytes = crate::file_cmds::read_file(&path)?;
    let imported = photocraft_io::import(&path, &bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let src = imported.document;
    let (w, h) = (src.size.width, src.size.height);
    if w == 0 || h == 0 {
        return Err(bad(format!("{path} is empty")));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (cw, ch) = (st.doc.size.width, st.doc.size.height);
    if (w, h) != (cw, ch) {
        let size = crate::commands::find("image.imageSize").ok_or_else(|| EngineError::Other("Image Size is missing".into()))?;
        (size.run)(s, &json!({"width": w, "height": h, "resample": "bicubic"}))?;
    }
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let px = crate::file_cmds::flattened(&src, fmt);
    let layer = s.edit(LABEL, |doc, active| {
        let top = doc.layers.last().map(|l| l.id);
        let id = doc.insert_above(top, Layer::new(LAYER_NAME, LayerContent::Raster(px)));
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": layer.0, "width": w, "height": h, "scale": f64::from(w) / f64::from(cw.max(1))}))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("result").is_some() {
        return Err(bad(format!("an upscaled image is applied with `{APPLY_ID}`")));
    }
    if ["model", "modelVersion", "scale", "width", "height"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        return upscale_with_cli(s, p);
    }
    open_in_app(s)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: ID,
            label: "Topaz Gigapixel AI…",
            menu: &["File", "Automate"],
            shortcut: None,
            params: r##"{} (open the flattened image in Topaz Gigapixel AI; returns {"waiting":true,"document","folder","input"} and the app brings the export back when it appears, with file.automate.topazGigapixel.apply) | {"model":"std|hf|lowres|art|lines|text|vc|redefine|recovery"? (default: Gigapixel's choice),"modelVersion":n?,"scale":1..16|"width":px|"height":px,"denoise":0..100?,"sharpen":0..100?,"compression":0..100?,"faceRecovery":0..100?} (upscale with Gigapixel's command-line tool and apply)"##,
            enabled,
            run,
            journal: true,
        },
        CommandSpec {
            id: APPLY_ID,
            label: "Apply Topaz Gigapixel AI Result",
            menu: &[],
            shortcut: None,
            params: r##"{"result":path,"document":id? (default the active one)} → {layer,width,height,scale} (the document resized to the upscaled image, which is added on top as "Topaz Gigapixel layer", as Photoshop's plug-in leaves it)"##,
            enabled: has_document,
            run: apply,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
