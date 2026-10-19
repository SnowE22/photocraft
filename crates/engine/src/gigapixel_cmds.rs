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
//!   (`gigapixel.exe`, Gigapixel 7.3 and later, as licensed) and apply the result, for actions,
//!   batch and agents.
//! - `{"preset": name}` or `{"presets": [a, b], "combine": "chain" | "layers"}`: the same with
//!   saved settings ([`presets`], File › Automate › Topaz Gigapixel Presets). `chain` upscales the
//!   first preset's result again with the second; `layers` upscales the image with each and keeps
//!   every result as a layer.
//!
//! The command-line forms run as a background job (progress, Esc cancels and stops Gigapixel):
//! the image is exported on the calling thread, Gigapixel runs and its results are read on a
//! worker, and they are applied on the calling thread.
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

pub mod presets;
use presets::Combine;

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

/// One Gigapixel run: the preset it comes from (if any) and its command-line settings.
#[derive(Clone, Debug)]
struct Step {
    preset: Option<String>,
    params: Value,
}

/// The upscales `p` asks for and how to combine their results; params are checked here, before
/// anything is written or started.
fn plan(s: &Session, p: &Value) -> Result<(Vec<Step>, Combine)> {
    let named = |steps: Vec<(String, Value)>| steps.into_iter().map(|(n, params)| Step { preset: Some(n), params }).collect::<Vec<_>>();
    let (steps, combine) = if let Some(v) = p.get("preset").filter(|v| !v.is_null()) {
        let name = v.as_str().ok_or_else(|| bad("`preset` must be a preset name"))?;
        let preset = presets::find(s, name).ok_or_else(|| bad(format!("there is no Topaz Gigapixel preset {name:?}")))?;
        match preset.combined() {
            Some(names) => (named(presets::steps_of(s, &names, ID)?), Combine::parse(preset.settings.get("combine"), ID)?),
            None => (named(vec![(preset.name.clone(), preset.settings.clone())]), Combine::Chain),
        }
    } else if let Some(v) = p.get("presets").filter(|v| !v.is_null()) {
        let list = v.as_array().ok_or_else(|| bad("`presets` must be a list of preset names"))?;
        let names: Vec<&str> = list.iter().filter_map(Value::as_str).collect();
        if names.is_empty() || names.len() > presets::MAX_STEPS || names.len() != list.len() {
            return Err(bad(format!("`presets` must name 1 to {} presets", presets::MAX_STEPS)));
        }
        (named(presets::steps_of(s, &names, ID)?), Combine::parse(p.get("combine"), ID)?)
    } else {
        (vec![Step { preset: None, params: p.clone() }], Combine::Chain)
    };
    for step in &steps {
        cli_args(&step.params, Path::new("in.tif"), Path::new("out"))?;
    }
    Ok((steps, combine))
}

/// The folder a run works in, removed when the run's result is dropped (applied, failed or
/// cancelled).
#[cfg(not(target_arch = "wasm32"))]
struct Scratch(PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
thread_local! {
    /// A stand-in for Gigapixel's command-line tool (tests).
    static FAKE_CLI: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Gigapixel's command-line tool, when it may be started.
#[cfg(not(target_arch = "wasm32"))]
fn cli_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(p) = FAKE_CLI.with(|f| f.borrow().clone()) {
        return Ok(p);
    }
    launching_allowed()?;
    let gp = installed().ok_or_else(|| bad("Topaz Gigapixel AI is not installed (or set PHOTOCRAFT_GIGAPIXEL to its executable)"))?;
    gp.cli.ok_or_else(|| bad("this Gigapixel has no command-line tool (Gigapixel 7.3 or later has gigapixel.exe beside the app)"))
}

/// Run the command-line tool on `input`, writing into `out`; the image it made. Cancelling the
/// job stops Gigapixel.
#[cfg(not(target_arch = "wasm32"))]
fn run_cli(cli: &Path, params: &Value, input: &Path, out: &Path, ctx: &crate::jobs::JobCtx) -> Result<PathBuf> {
    let args = cli_args(params, input, out)?;
    let log_path = out.with_extension("log");
    let log = std::fs::File::create(&log_path).map_err(|e| EngineError::Other(format!("{}: {e}", log_path.display())))?;
    let log2 = log.try_clone().map_err(|e| EngineError::Other(format!("{}: {e}", log_path.display())))?;
    let mut cmd = std::process::Command::new(cli);
    cmd.args(&args).stdin(std::process::Stdio::null()).stdout(log).stderr(log2);
    #[cfg(target_os = "windows")]
    {
        // No console window over the app.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| EngineError::Other(format!("{}: {e}", cli.display())))?;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if ctx.cancelled() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::Cancelled);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => {
                let _ = child.kill();
                return Err(EngineError::Other(format!("{}: {e}", cli.display())));
            }
        }
    };
    let made = std::fs::read_dir(out)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file() && p.extension().is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())));
    made.ok_or_else(|| {
        let said = std::fs::read_to_string(&log_path).unwrap_or_default();
        let said = said.lines().rev().map(|l| l.trim().trim_matches('"')).find(|l| !l.is_empty()).unwrap_or("").to_string();
        EngineError::Other(format!("Topaz Gigapixel AI's command-line tool made no image (exit {status}): {said}"))
    })
}

/// The layer name for a result: the name Photoshop's plug-in gives it, plus the presets that
/// made it.
fn layer_name(presets: &[&str]) -> String {
    if presets.is_empty() { LAYER_NAME.to_string() } else { format!("{LAYER_NAME} ({})", presets.join(" → ")) }
}

/// Upscale with the command-line tool, in the background, then apply the results.
#[cfg(not(target_arch = "wasm32"))]
fn upscale(s: &mut Session, p: &Value) -> Result<Value> {
    let (steps, combine) = plan(s, p)?;
    let cli = cli_path()?;
    let document = s.active().ok_or(EngineError::NoDocument)?.doc.id.0;
    let (folder, input) = export(s)?;
    let scratch = Scratch(folder.clone());
    let names: Vec<String> = steps.iter().filter_map(|st| st.preset.clone()).collect();
    let label = if names.is_empty() { LABEL.to_string() } else { format!("{LABEL}: {}", names.join(if combine == Combine::Chain { " → " } else { " + " })) };
    let model = p.get("model").cloned().unwrap_or(Value::Null);
    crate::jobs::run(
        s,
        &label,
        true,
        move |ctx| {
            let n = steps.len();
            let mut source = input.clone();
            let mut made: Vec<(Vec<&str>, PathBuf)> = Vec::new();
            for (i, step) in steps.iter().enumerate() {
                ctx.check()?;
                let what = step.preset.as_deref().unwrap_or(LABEL);
                let message = if n > 1 { format!("Upscaling with {what} ({} of {n})", i + 1) } else { format!("Upscaling with {what}") };
                ctx.progress(i as f32 / n as f32 * 0.9, &message);
                let from = if combine == Combine::Chain { source.clone() } else { input.clone() };
                let result = run_cli(&cli, &step.params, &from, &folder.join(format!("step{}", i + 1)), ctx)?;
                let by: Vec<&str> = match combine {
                    Combine::Chain => steps.iter().take(i + 1).filter_map(|st| st.preset.as_deref()).collect(),
                    Combine::Layers => step.preset.as_deref().into_iter().collect(),
                };
                if combine == Combine::Chain {
                    made.clear();
                }
                made.push((by, result.clone()));
                source = result;
            }
            ctx.progress(0.9, "Reading the upscaled image");
            let mut images: Vec<(String, photocraft_doc::Document)> = Vec::new();
            for (by, path) in made {
                ctx.check()?;
                images.push((layer_name(&by), import_result(&path)?));
            }
            // Smallest first: the document ends at the largest size, and only smaller results
            // are resampled to it.
            images.sort_by_key(|(_, d)| u64::from(d.size.width) * u64::from(d.size.height));
            Ok((scratch, images))
        },
        move |s, (scratch, images)| {
            let mut layers = Vec::new();
            let mut last = Value::Null;
            for (name, image) in images {
                last = apply_image(s, Some(document), image, &name)?;
                layers.push(last["layer"].clone());
            }
            drop(scratch);
            let mut r = last;
            r["layers"] = json!(layers);
            r["model"] = model;
            if !names.is_empty() {
                r["presets"] = json!(names);
            }
            Ok(r)
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn upscale(s: &mut Session, p: &Value) -> Result<Value> {
    plan(s, p)?;
    Err(EngineError::Other("Topaz Gigapixel AI needs the desktop app".into()))
}

/// An upscaled image file, read.
fn import_result(path: &Path) -> Result<photocraft_doc::Document> {
    let name = path.to_string_lossy().into_owned();
    let bytes = crate::file_cmds::read_file(&name)?;
    let imported = photocraft_io::import(&name, &bytes).map_err(|e| EngineError::Other(format!("{name}: {e}")))?;
    let src = imported.document;
    if src.size.width == 0 || src.size.height == 0 {
        return Err(bad(format!("{name} is empty")));
    }
    Ok(src)
}

/// `{"result": path, "document": id?}`: the document (by id; default the active one) resized to
/// the upscaled image, and the image added on top as [`LAYER_NAME`].
fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let path = crate::file_cmds::str_param(p, "result", APPLY_ID)?.to_string();
    let document = match p.get("document").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_u64().ok_or_else(|| bad("`document` must be a document id"))?),
    };
    if let Some(id) = document
        && !s.documents().iter().any(|d| d.doc.id.0 == id)
    {
        return Err(bad("the document the image came from is closed"));
    }
    let image = import_result(Path::new(&path))?;
    apply_image(s, document, image, LAYER_NAME)
}

/// Resize document `document` (default the active one) to `src` and add `src` on top as `name`.
fn apply_image(s: &mut Session, document: Option<u64>, src: photocraft_doc::Document, name: &str) -> Result<Value> {
    if let Some(id) = document {
        let i = s.documents().iter().position(|d| d.doc.id.0 == id).ok_or_else(|| bad("the document the image came from is closed"))?;
        s.set_active(i);
    }
    let (w, h) = (src.size.width, src.size.height);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (cw, ch) = (st.doc.size.width, st.doc.size.height);
    if (w, h) != (cw, ch) {
        let size = crate::commands::find("image.imageSize").ok_or_else(|| EngineError::Other("Image Size is missing".into()))?;
        (size.run)(s, &json!({"width": w, "height": h, "resample": "bicubic"}))?;
    }
    let fmt = s.active().ok_or(EngineError::NoDocument)?.doc.pixel_format();
    let px = crate::file_cmds::flattened(&src, fmt);
    let name = name.to_string();
    let layer = s.edit(LABEL, |doc, active| {
        let top = doc.layers.last().map(|l| l.id);
        let id = doc.insert_above(top, Layer::new(name, LayerContent::Raster(px)));
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": layer.0, "width": w, "height": h, "scale": f64::from(w) / f64::from(cw.max(1))}))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    if p.get("result").is_some() {
        return Err(bad(format!("an upscaled image is applied with `{APPLY_ID}`")));
    }
    if ["preset", "presets", "model", "modelVersion", "scale", "width", "height"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
        return upscale(s, p);
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
            params: r##"{} (open the flattened image in Topaz Gigapixel AI; returns {"waiting":true,"document","folder","input"} and the app brings the export back when it appears, with file.automate.topazGigapixel.apply) | {"preset":name} | {"presets":[name,…] (1..4),"combine":"chain|layers"?} (saved settings: file.automate.topazGigapixel.presets.*; chain upscales each result again, layers keeps one layer per preset) | {"model":"std|hf|lowres|art|lines|text|vc|redefine|recovery"? (default: Gigapixel's choice),"modelVersion":n?,"scale":1..16|"width":px|"height":px,"denoise":0..100?,"sharpen":0..100?,"compression":0..100?,"faceRecovery":0..100?} (upscale with Gigapixel's command-line tool and apply, as a background job) → {layer,layers,width,height,scale,presets?}"##,
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
