//! File › Timelapse: record a timelapse of your painting while you work and export it as an MP4,
//! ported from the Photoshop Timelapse Recorder plug-in (a UXP panel). Built in rather than a
//! WebAssembly plug-in: recording needs to follow every edit, undo and redo and capture in the
//! background, which the plug-in ABI (pixel filters only) can't do.
//!
//! - [`recorder`]: follows edits and captures frames on a worker thread
//!   ([`Session::timelapse_tick`], called by the shell every frame).
//! - [`store`]: recordings on disk, in the Photoshop plug-in's layout, so either app continues or
//!   exports the other's recordings (a PSD the plug-in recorded resumes its recording here).
//! - [`plan`], [`deflicker`], [`raster`], [`export`]: the export (a background job).
//!
//! Commands: `file.timelapse.record`, `.pause`, `.export`, `.referenceLayer` (menu) and
//! `.status`, `.settings`, `.capture`, `.list`, `.deleteFrames`, `.deleteRecording` (for the
//! panel's Library and agents). All of them live under
//! `file.` so untrusted automation, which may not reach the file system, can't use them.

pub mod deflicker;
pub mod export;
pub mod plan;
pub mod raster;
pub mod recorder;
pub mod store;

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
pub use recorder::{ExportSettings, Frame, Recorder, Settings};

pub const RECORD: &str = "file.timelapse.record";
pub const PAUSE: &str = "file.timelapse.pause";
pub const EXPORT: &str = "file.timelapse.export";
pub const REFERENCE: &str = "file.timelapse.referenceLayer";
pub const STATUS: &str = "file.timelapse.status";
pub const SETTINGS: &str = "file.timelapse.settings";
pub const CAPTURE: &str = "file.timelapse.capture";
pub const LIST: &str = "file.timelapse.list";
pub const DELETE_FRAMES: &str = "file.timelapse.deleteFrames";
pub const DELETE_RECORDING: &str = "file.timelapse.deleteRecording";

/// Reference tags offered in the settings (any text works).
pub const REFERENCE_TAGS: &[&str] = &["[ref]", "[reference]", "(ref)", "#ref", "_ref"];

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

impl Session {
    /// Once per UI frame: follow edits, capture frames, keep recordings saved. `input` is whether
    /// the user did anything this frame (keeps the work timer running). Returns how many
    /// milliseconds until it needs to run again (`None`: only after the next change).
    pub fn timelapse_tick(&mut self, input: bool) -> Option<f64> {
        let now = recorder::mono_ms();
        self.timelapse_tick_at(now, input).map(|at| (at - now).max(0.0))
    }

    /// [`Session::timelapse_tick`] at an explicit time (ms on [`recorder::mono_ms`]'s clock);
    /// returns the absolute time of the next needed tick.
    pub fn timelapse_tick_at(&mut self, now: f64, input: bool) -> Option<f64> {
        let jobs = if self.has_jobs() { self.jobs() } else { Vec::new() };
        let exporting = jobs.iter().any(|j| j.command == EXPORT);
        let locked: Vec<_> = jobs.iter().filter_map(|j| j.document).collect();
        let Session { docs, active, timelapse, prefs, .. } = self;
        timelapse.sync_docs(docs, &locked, exporting, now);
        timelapse.tick(docs, *active, prefs.get(), now, input)
    }

    /// Messages for the status bar (a recording resumed, a frame couldn't be saved…).
    pub fn timelapse_messages(&mut self) -> Vec<String> {
        self.timelapse.take_messages()
    }

    /// Wait for frames being captured and record them (tests, the capture command).
    pub fn timelapse_wait(&mut self) {
        self.timelapse.wait(recorder::mono_ms());
    }

    /// The settings (loaded from the preferences).
    pub fn timelapse_settings(&mut self) -> Settings {
        let Session { timelapse, prefs, .. } = self;
        timelapse.settings(prefs.get()).clone()
    }

    /// What `file.timelapse.status` answers, for the panel (no command dispatch per frame).
    pub fn timelapse_status(&mut self) -> Value {
        status(self)
    }

    fn active_doc_id(&self) -> Result<photocraft_doc::DocId> {
        self.active().map(|d| d.doc.id).ok_or(EngineError::NoDocument)
    }
}

fn is_recording(s: &Session) -> bool {
    s.active().and_then(|d| s.timelapse.track(d.doc.id)).is_some_and(|t| t.recording)
}

fn can_record(s: &Session) -> std::result::Result<(), String> {
    if let Some(e) = recorder::unsupported() {
        return Err(e.to_string());
    }
    s.active().ok_or("no document open")?;
    if is_recording(s) { Err("already recording".into()) } else { Ok(()) }
}

fn can_pause(s: &Session) -> std::result::Result<(), String> {
    if is_recording(s) { Ok(()) } else { Err("this document is not being recorded".into()) }
}

fn can_export(_: &Session) -> std::result::Result<(), String> {
    if let Some(e) = recorder::unsupported() {
        return Err(e.to_string());
    }
    Ok(())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    crate::active_layer_of(s).map(|_| ())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn status(s: &mut Session) -> Value {
    s.timelapse_settings();
    s.timelapse.status(s.active(), recorder::mono_ms())
}

fn record(s: &mut Session, _p: &Value) -> Result<Value> {
    s.timelapse_settings();
    let now = recorder::mono_ms();
    let Session { docs, active, timelapse, .. } = &mut *s;
    let st = active.and_then(|i| docs.get(i)).ok_or(EngineError::NoDocument)?;
    timelapse.start(st, now)?;
    Ok(status(s))
}

fn pause(s: &mut Session, _p: &Value) -> Result<Value> {
    let id = s.active_doc_id()?;
    s.timelapse.pause(id, recorder::mono_ms(), true)?;
    Ok(status(s))
}

fn capture(s: &mut Session, _p: &Value) -> Result<Value> {
    let now = recorder::mono_ms();
    let Session { docs, active, timelapse, .. } = &mut *s;
    let st = active.and_then(|i| docs.get(i)).ok_or(EngineError::NoDocument)?;
    let captured = timelapse.capture_now(st, now)?;
    let mut v = status(s);
    if let Some(o) = v.as_object_mut() {
        o.insert("captured".into(), json!(captured));
    }
    Ok(v)
}

fn get_bool(p: &Value, key: &str, cmd: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(v) => Err(bad(cmd, format!("`{key}` must be true or false, got {v}"))),
    }
}

fn get_num(p: &Value, key: &str, cmd: &str, lo: f64, hi: f64) -> Result<Option<f64>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (lo..=hi).contains(&x) => Ok(Some(x)),
            _ => Err(bad(cmd, format!("`{key}` must be a number from {lo} to {hi}, got {v}"))),
        },
    }
}

fn get_str<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(v) => Err(bad(cmd, format!("`{key}` must be a string, got {v}"))),
    }
}

fn get_choice<'a>(p: &'a Value, key: &str, cmd: &str, allowed: &[&str]) -> Result<Option<&'a str>> {
    match get_str(p, key, cmd)? {
        Some(v) if !allowed.contains(&v) => Err(bad(cmd, format!("`{key}` must be one of {}, got \"{v}\"", allowed.join(", ")))),
        v => Ok(v),
    }
}

/// Apply export options in `p` over `o`.
fn merge_export(o: &mut ExportSettings, p: &Value, cmd: &str) -> Result<()> {
    match p.get("lengthSec") {
        None => {}
        Some(Value::String(a)) if a == "auto" => o.length_sec = None,
        Some(Value::Null) => o.length_sec = None,
        Some(_) => o.length_sec = get_num(p, "lengthSec", cmd, 1.0, 600.0)?,
    }
    if let Some(v) = get_num(p, "introSec", cmd, 0.0, 10.0)? {
        o.intro_sec = v;
    }
    if let Some(v) = get_num(p, "holdSec", cmd, 0.0, 30.0)? {
        o.hold_sec = v;
    }
    for (key, field) in [("flash", &mut o.flash), ("slowEnd", &mut o.slow_end), ("deflicker", &mut o.deflicker), ("timer", &mut o.timer)] {
        if let Some(v) = get_bool(p, key, cmd)? {
            *field = v;
        }
    }
    if let Some(v) = get_choice(p, "corner", cmd, &["tl", "tr", "bl", "br"])? {
        o.corner = v.into();
    }
    if let Some(v) = get_choice(p, "letterbox", cmd, &["black", "white", "gray"])? {
        o.letterbox = v.into();
    }
    if let Some(v) = get_choice(p, "encoder", cmd, &["ffmpeg", "frames"])? {
        o.encoder = v.into();
    }
    Ok(())
}

fn store_settings(s: &mut Session, settings: Settings) {
    let v = serde_json::to_value(&settings).unwrap_or(Value::Null);
    s.timelapse.set_settings(settings);
    s.edit_prefs(|p| {
        p.dialogs.insert(recorder::SETTINGS_KEY.into(), v);
    });
}

fn settings_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = SETTINGS;
    if !p.is_null() && !p.is_object() {
        return Err(bad(cmd, "expected an object"));
    }
    let mut st = s.timelapse_settings();
    let old_folder = st.folder.clone();
    if let Some(v) = get_str(p, "folder", cmd)? {
        st.folder = v.trim().into();
    }
    if let Some(v) = get_choice(p, "quality", cmd, &["low", "default", "high"])? {
        st.quality = v.into();
    }
    if let Some(v) = get_bool(p, "hideUndone", cmd)? {
        st.hide_undone = v;
    }
    if let Some(v) = get_str(p, "referenceTag", cmd)? {
        let v = v.trim();
        if v.is_empty() || v.chars().count() > 32 {
            return Err(bad(cmd, "`referenceTag` must be 1 to 32 characters"));
        }
        st.reference_tag = v.into();
    }
    if let Some(v) = get_num(p, "idleSec", cmd, 5.0, 3600.0)? {
        st.idle_sec = v as u32;
    }
    if let Some(v) = get_bool(p, "autoRecord", cmd)? {
        st.auto_record = v;
    }
    if let Some(v) = get_str(p, "ffmpeg", cmd)? {
        st.ffmpeg = v.trim().into();
    }
    if let Some(e) = p.get("export") {
        if !e.is_object() {
            return Err(bad(cmd, "`export` must be an object"));
        }
        merge_export(&mut st.export, e, cmd)?;
    }
    if st.folder != old_folder {
        // Recordings live in the old folder: stop following them (a recording one resumes when
        // its document is next opened with that folder chosen again).
        let now = recorder::mono_ms();
        s.timelapse.wait(now);
        let ids: Vec<_> = s.timelapse.tracks.iter().map(|t| t.doc).collect();
        for id in ids {
            let _ = s.timelapse.pause(id, now, false);
        }
        s.timelapse.tracks.clear();
        s.timelapse.reset_probed();
    }
    store_settings(s, st);
    Ok(status(s))
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let st = s.timelapse_settings();
    if st.folder.is_empty() {
        return Ok(json!({"folder": "", "recordings": []}));
    }
    let open: Vec<(String, bool)> = s.timelapse.tracks.iter().map(|t| (t.rec.meta.id.clone(), t.recording)).collect();
    let recs: Vec<Value> = store::scan(std::path::Path::new(&st.folder))
        .into_iter()
        .map(|mut r| {
            // An open recording's newest state is in memory.
            if let Some(t) = s.timelapse.tracks.iter().find(|t| t.rec.meta.id == r.meta.id) {
                r = t.rec.clone();
            }
            let m = &r.meta;
            let live = m.usable(st.hide_undone).count();
            json!({
                "id": m.id,
                "name": m.name,
                "dir": r.dir.to_string_lossy(),
                "documentPath": m.doc_path,
                "frames": live,
                "allFrames": m.frames.len(),
                "bytes": m.bytes,
                "workMs": m.work_ms,
                "recorded": m.recorded,
                "exported": !m.exports.is_empty(),
                "lastExport": m.exports.last().map(|e| json!({"t": e.t, "path": e.path, "frames": e.frames})),
                "framesSinceExport": m.exports.last().map_or(m.frames.len(), |e| m.frames.len().saturating_sub(e.frames)),
                "open": open.iter().any(|(id, _)| *id == m.id),
                "recording": open.iter().any(|(id, rec)| *id == m.id && *rec),
            })
        })
        .collect();
    Ok(json!({"folder": st.folder, "recordings": recs}))
}

/// The recording `p.recording` names: the open one (its newest state is in memory), else the one
/// in the recordings folder. Refused while it records, or while a timelapse is being exported
/// (the export reads the folder).
fn library_target(s: &mut Session, p: &Value, cmd: &str) -> Result<(Option<photocraft_doc::DocId>, store::Recording)> {
    if let Some(e) = recorder::unsupported() {
        return Err(e);
    }
    if !p.is_object() {
        return Err(bad(cmd, "expected {\"recording\": id}"));
    }
    let id = get_str(p, "recording", cmd)?.filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(cmd, "need `recording` (an id from file.timelapse.list)"))?;
    if s.jobs().iter().any(|j| j.command == EXPORT) {
        return Err(EngineError::Other("a timelapse is being exported: wait for it to finish".into()));
    }
    s.timelapse.wait(recorder::mono_ms());
    if let Some(t) = s.timelapse.tracks.iter().find(|t| t.rec.meta.id == id) {
        if t.recording {
            return Err(EngineError::Other(format!("“{}” is recording: pause it first", t.rec.meta.name)));
        }
        return Ok((Some(t.doc), t.rec.clone()));
    }
    let folder = s.timelapse_settings().folder;
    if folder.is_empty() {
        return Err(EngineError::Other("no recordings folder chosen yet".into()));
    }
    let rec = store::scan(std::path::Path::new(&folder)).into_iter().find(|r| r.meta.id == id).ok_or_else(|| bad(cmd, format!("no recording with id `{id}`")))?;
    Ok((None, rec))
}

/// The Library's Delete frames: the recording's frames go; its work time, export history and
/// exported videos stay.
fn delete_frames(s: &mut Session, p: &Value) -> Result<Value> {
    let (doc, mut rec) = library_target(s, p, DELETE_FRAMES)?;
    let frames = rec.meta.frames.len();
    let bytes = rec.meta.bytes;
    let r = store::clear_frames(&mut rec, recorder::unix_ms());
    if let Some(t) = doc.and_then(|d| s.timelapse.track_mut(d)) {
        t.rec = rec.clone();
        t.frames_cleared();
    }
    let deleted = r?;
    Ok(json!({"id": rec.meta.id, "name": rec.meta.name, "frames": frames, "deletedFiles": deleted, "bytes": bytes}))
}

/// The Library's Delete folder, for a recording whose frames were deleted: it leaves the library.
/// Files PhotoCraft didn't write (a video saved there) stay, and so does the folder then.
fn delete_recording(s: &mut Session, p: &Value) -> Result<Value> {
    let (doc, rec) = library_target(s, p, DELETE_RECORDING)?;
    if !rec.meta.frames.is_empty() {
        return Err(EngineError::Other(format!("“{}” still has {} frames: delete them first (file.timelapse.deleteFrames)", rec.meta.name, rec.meta.frames.len())));
    }
    if let Some(d) = doc {
        s.timelapse.forget(d);
    }
    let kept = store::delete_recording(&rec)?;
    Ok(json!({"id": rec.meta.id, "name": rec.meta.name, "folderRemoved": kept.is_empty(), "kept": kept}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = EXPORT;
    if let Some(e) = recorder::unsupported() {
        return Err(e);
    }
    if !p.is_object() {
        return Err(bad(cmd, "expected {\"path\": …}"));
    }
    let out = get_str(p, "path", cmd)?.filter(|v| !v.trim().is_empty()).ok_or_else(|| bad(cmd, "need `path` (the .mp4, or a folder for a JPEG sequence)"))?;
    let mut settings = s.timelapse_settings();
    let mut o = settings.export.clone();
    merge_export(&mut o, p, cmd)?;
    let fps = get_num(p, "fps", cmd, 1.0, 120.0)?.unwrap_or(30.0);
    let ffmpeg = get_str(p, "ffmpeg", cmd)?.map(str::to_string).unwrap_or_else(|| settings.ffmpeg.clone());
    let now = recorder::mono_ms();
    // The recording: the one named, else the active document's.
    let rec = match get_str(p, "recording", cmd)? {
        Some(id) => match s.timelapse.tracks.iter().find(|t| t.rec.meta.id == id).map(|t| t.doc) {
            Some(doc) => {
                s.timelapse.flush(doc, now)?;
                s.timelapse.track(doc).map(|t| t.rec.clone())
            }
            None if !settings.folder.is_empty() => store::scan(std::path::Path::new(&settings.folder)).into_iter().find(|r| r.meta.id == id),
            None => None,
        }
        .ok_or_else(|| bad(cmd, format!("no recording with id `{id}`")))?,
        None => {
            let doc = s.active_doc_id()?;
            s.timelapse.flush(doc, now)?;
            s.timelapse.track(doc).map(|t| t.rec.clone()).ok_or_else(|| EngineError::Other("this document has no timelapse recording".into()))?
        }
    };
    if export::usable_frames(&rec, settings.hide_undone).is_empty() {
        return Err(EngineError::Other("this recording doesn't have any frames yet".into()));
    }
    if get_bool(p, "remember", cmd)?.unwrap_or(true) && o != settings.export {
        settings.export = o.clone();
        store_settings(s, settings.clone());
    }
    let id = rec.meta.id.clone();
    let frames_now = rec.meta.frames.len();
    let job = export::Job { rec, hide_undone: settings.hide_undone, o, fps, out: std::path::PathBuf::from(out), ffmpeg: export::find_ffmpeg(&ffmpeg) };
    crate::jobs::run(
        s,
        "Export Timelapse",
        false,
        move |ctx| export::run(&job, ctx),
        move |s, r: export::Exported| {
            let entry = store::ExportMeta { t: recorder::unix_ms(), path: r.file.clone(), mode: r.mode.clone(), frames: frames_now, ..Default::default() };
            if let Some(t) = s.timelapse.tracks.iter_mut().find(|t| t.rec.meta.id == id) {
                t.rec.meta.exports.push(entry);
                let _ = t.rec.save(recorder::unix_ms());
            } else if let Some(mut r) = store::scan(std::path::Path::new(&s.timelapse_settings().folder)).into_iter().find(|r| r.meta.id == id) {
                r.meta.exports.push(entry);
                let _ = r.save(recorder::unix_ms());
            }
            Ok(r.to_json())
        },
    )
}

/// Layers named with the reference tag stay visible to you but are hidden in every frame. Adds
/// the tag to the targeted layers, or removes it when they all have it (`reference` forces one).
fn reference_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = REFERENCE;
    let want = get_bool(p, "reference", cmd)?;
    let tag = s.timelapse_settings().reference_tag;
    let tag = if tag.trim().is_empty() { "[ref]".to_string() } else { tag.trim().to_string() };
    let tag_lc = tag.to_lowercase();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<_> = st.selected_layers().into_iter().filter(|id| !st.doc.is_background(*id)).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("select a layer first (the Background can't be a reference layer: duplicate it first)".into()));
    }
    let has = |id| st.doc.layer(id).is_some_and(|l| l.name.to_lowercase().contains(&tag_lc));
    let remove = want.map_or_else(|| ids.iter().all(|id| has(*id)), |w| !w);
    let todo: Vec<_> = ids.iter().copied().filter(|id| has(*id) == remove).collect();
    if !todo.is_empty() {
        s.edit(if remove { "Remove Reference Tag" } else { "Make Reference Layer" }, |doc, _| {
            for id in &todo {
                let l = doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
                l.name = if remove { without_tag(&l.name, &tag) } else { format!("{} {tag}", l.name.trim_end()) };
            }
            Ok(())
        })?;
    }
    Ok(json!({"reference": !remove, "changed": todo.len(), "tag": tag}))
}

/// `name` without the first `tag` (any case), spaces tidied; never empty.
pub fn without_tag(name: &str, tag: &str) -> String {
    let lower = name.to_lowercase();
    let Some(i) = lower.find(&tag.to_lowercase()) else { return name.to_string() };
    // Lowercasing can change byte lengths; only cut where both strings line up.
    let (Some(a), Some(b)) = (name.get(..i), name.get(i + tag.len()..)) else { return name.to_string() };
    let joined = format!("{a}{b}");
    let tidy = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if tidy.is_empty() { "Layer".into() } else { tidy }
}

macro_rules! spec {
    ($id:expr, $label:expr, $menu:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: $menu, shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    let m = &["File", "Timelapse"] as &[&str];
    vec![
        spec!(RECORD, "Start Recording", m, "{} → status (records the active document: a frame shortly after each edit; needs a recordings folder, see file.timelapse.settings)", can_record, true, record),
        spec!(PAUSE, "Pause Recording", m, "{} → status", can_pause, true, pause),
        spec!(
            EXPORT,
            "Export Timelapse…",
            m,
            r##"{"path":".mp4 (or a folder with "encoder":"frames"),"recording":id? (default the active document's),"encoder":"ffmpeg|frames"?,"lengthSec":1..600|"auto"?,"introSec":0..10?,"flash":bool?,"holdSec":0..30?,"slowEnd":bool?,"deflicker":bool?,"timer":bool?,"corner":"tl|tr|bl|br"?,"letterbox":"black|white|gray"?,"fps":1..120?,"ffmpeg":path?,"remember":bool=true} (unset options: the last export's) → {file,seconds,videoFrames,width,height,frames,flickerLeftOut,warning} (a background job)"##,
            can_export,
            true,
            export
        ),
        spec!(REFERENCE, "Reference Layer", m, r##"{"reference":bool? (default: toggle)} → {reference,changed,tag} (tags the targeted layers' names: visible to you, hidden in the timelapse)"##, has_layer, true, reference_layer),
        spec!(STATUS, "Timelapse Status", &[], "{} → {available,document,recording:{id,name,folder,recording,idle,frames,allFrames,workMs,bytes,exports,lastExport,framesSinceExport,previewRev}|null,settings}", always, false, |s, _| Ok(status(s))),
        spec!(
            SETTINGS,
            "Timelapse Settings",
            &[],
            r##"{"folder":path?,"quality":"low|default|high"?,"hideUndone":bool?,"referenceTag":str?,"idleSec":5..3600?,"autoRecord":bool?,"ffmpeg":path?,"export":{…export options}?} → status (persisted in the preferences)"##,
            always,
            true,
            settings_cmd
        ),
        spec!(CAPTURE, "Capture Timelapse Frame", &[], "{} → status + {captured: bool} (a frame now, if anything visible changed)", can_pause, true, capture),
        spec!(
            LIST,
            "Timelapse Recordings",
            &[],
            "{} → {folder, recordings:[{id,name,dir,documentPath,frames,allFrames,bytes,workMs,recorded,exported,lastExport:{t,path,frames}|null,framesSinceExport,open,recording}]} (most recently recorded first)",
            always,
            false,
            list
        ),
        spec!(
            DELETE_FRAMES,
            "Delete Timelapse Frames",
            &[],
            r##"{"recording":id} → {id,name,frames,deletedFiles,bytes} (deletes the recording's frames; work time, export history and videos stay; refused while it records or a timelapse exports)"##,
            can_export,
            true,
            delete_frames
        ),
        spec!(
            DELETE_RECORDING,
            "Delete Timelapse Recording",
            &[],
            r##"{"recording":id} → {id,name,folderRemoved,kept} (a recording whose frames were deleted leaves the library with its folder; files PhotoCraft didn't write stay; refused while it records or a timelapse exports)"##,
            can_export,
            true,
            delete_recording
        ),
    ]
}

#[cfg(test)]
mod tests;
