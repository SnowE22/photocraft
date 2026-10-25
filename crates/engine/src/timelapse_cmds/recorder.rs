//! The capture side: each document with a recording gets a [`Track`]; while it records, every
//! real edit (a new or changed history state) makes a frame shortly afterwards, rendered on a
//! worker thread from a copy-on-write snapshot of the document, so painting never waits for it.
//!
//! As in the Photoshop Timelapse Recorder:
//! - Non-edits are skipped: view changes (zoom, channel eyes), selecting layers, and showing or
//!   hiding layers (history steps in PhotoCraft, recognised by their names). After a show/hide
//!   nothing is captured until the next real edit, so comparing layers never makes the video
//!   flicker.
//! - Rapid edits are batched: a frame waits for a quiet moment ([`SETTLE_MS`]), but never longer
//!   than [`MAX_WAIT_MS`] after the first edit it covers.
//! - Frames where (almost) nothing visible changed are skipped.
//! - Undo flags the frames of the undone steps (left out of the video when "Remove frames with
//!   undo" is on); redo brings them back; a new edit after an undo drops them for good.
//! - Reference layers (name containing the reference tag, `[ref]` by default) are hidden in
//!   every frame but stay visible to the user.
//! - A work timer counts time between actions and stops after `idleSec` without any.
//!
//! PhotoCraft's history has no stable ids, so each step is identified by its document snapshot
//! (held weakly): undo and redo return to a snapshot already seen, a new edit makes a new one.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Weak, mpsc};

use photocraft_doc::{DocId, Document};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::store::{self, FrameMeta, Recording};
use crate::{DocState, EngineError, Result};

/// Quiet time before a frame is taken…
pub const SETTLE_MS: f64 = 350.0;
/// …but no longer than this after the first edit it covers.
pub const MAX_WAIT_MS: f64 = 2500.0;
/// A channel difference up to this counts as unchanged…
const SIMILAR_TOL: u8 = 4;
/// …and frames with this few changed pixels or fewer are skipped.
const SIMILAR_PX: usize = 4;
/// `recording.json` is written at most this often while frames come in.
const SAVE_EVERY_MS: f64 = 2000.0;
/// Steps of history mirrored per document (older ones only keep their frames live).
const MAX_CHAIN: usize = 1024;
/// History step names that only show or hide layers.
const VISIBILITY_STEPS: &[&str] = &["Layer Visibility", "Show Layer", "Hide Layer", "Show Layers", "Hide Layers", "Show Only This Layer"];
/// JPEG quality of the stored frames.
const FRAME_QUALITY: u8 = 90;
/// Where the settings are kept in the preferences (`Preferences::dialogs`).
pub const SETTINGS_KEY: &str = "file.timelapse";

/// Settings (persisted in the preferences under [`SETTINGS_KEY`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Recordings folder (empty until chosen).
    pub folder: String,
    /// Frame size: "low" (1280 px long side), "default" (1920) or "high" (2880); never larger than
    /// the canvas.
    pub quality: String,
    /// Undoing removes the undone frames from the timelapse.
    pub hide_undone: bool,
    /// Layers whose name contains this are hidden in frames.
    pub reference_tag: String,
    /// The work timer stops after this many seconds without activity.
    pub idle_sec: u32,
    /// Record whatever document is active (unless paused by hand).
    pub auto_record: bool,
    /// ffmpeg executable for MP4 export (empty: look next to the app, then on PATH).
    pub ffmpeg: String,
    /// The export box's last choices.
    pub export: ExportSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: String::new(),
            quality: "default".into(),
            hide_undone: true,
            reference_tag: "[ref]".into(),
            idle_sec: 30,
            auto_record: false,
            ffmpeg: String::new(),
            export: ExportSettings::default(),
        }
    }
}

impl Settings {
    /// Longest side of a stored frame.
    pub fn max_long(&self) -> u32 {
        match self.quality.as_str() {
            "low" => 1280,
            "high" => 2880,
            _ => 1920,
        }
    }
    fn idle_ms(&self) -> f64 {
        f64::from(self.idle_sec.clamp(5, 3600)) * 1000.0
    }
    fn tag(&self) -> String {
        self.reference_tag.trim().to_lowercase()
    }
}

/// Export options, as the export box last had them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportSettings {
    /// Main part length in seconds; `None` is Auto.
    pub length_sec: Option<f64>,
    pub intro_sec: f64,
    pub flash: bool,
    pub hold_sec: f64,
    pub slow_end: bool,
    pub deflicker: bool,
    pub timer: bool,
    /// "tl" | "tr" | "bl" | "br".
    pub corner: String,
    /// Bars colour when the canvas size changed: "black" | "white" | "gray".
    pub letterbox: String,
    /// "ffmpeg" (MP4) | "frames" (a JPEG sequence).
    pub encoder: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            length_sec: None,
            intro_sec: 0.0,
            flash: false,
            hold_sec: 0.0,
            slow_end: false,
            deflicker: false,
            timer: false,
            corner: "br".into(),
            letterbox: "black".into(),
            encoder: "ffmpeg".into(),
        }
    }
}

/// A stored frame's pixels (sRGB, tightly packed RGB8).
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// What a capture worker came back with.
enum Outcome {
    Captured { frame: Arc<Frame>, bytes: u64 },
    Unchanged,
    Failed(String),
}

struct Pending {
    rx: mpsc::Receiver<Outcome>,
    n: u64,
    step: u64,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// One document's recording.
pub struct Track {
    pub doc: DocId,
    pub rec: Recording,
    pub recording: bool,
    /// Paused by hand: auto record leaves it alone.
    pub user_paused: bool,
    /// Tags this open-document session's frames (`FrameMeta::s`).
    key: String,
    // The history mirror.
    seen_rev: u64,
    seen_view: u64,
    cur: Weak<Document>,
    top_past: Option<Weak<Document>>,
    past_len: usize,
    redo_len: usize,
    chain: Vec<(u64, Weak<Document>)>,
    /// Undone steps, the next redo first.
    redo_chain: Vec<(u64, Weak<Document>)>,
    next_step: u64,
    // Capture scheduling.
    dirty: bool,
    first_dirty: f64,
    last_activity: Option<f64>,
    last_frame: Option<Arc<Frame>>,
    pending: Option<Pending>,
    failures: u32,
    retry_at: f64,
    meta_dirty: bool,
    last_save: f64,
    /// Bumps whenever the newest live frame changes (the panel's preview).
    pub preview_rev: u64,
    /// When to try renaming the folder again after the file system refused.
    rename_at: f64,
}

/// The recorder: settings and every document's track.
#[derive(Default)]
pub struct Recorder {
    settings: Option<Settings>,
    pub(crate) tracks: Vec<Track>,
    /// Open documents already looked at for a linked recording.
    probed: HashSet<DocId>,
    /// Open documents whose recording was stopped for good: auto record leaves them alone.
    no_auto: HashSet<DocId>,
    messages: Vec<String>,
    counter: u64,
}

/// Monotonic milliseconds (0 on wasm, where the recorder is unavailable).
pub fn mono_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

/// Wall-clock milliseconds since the Unix epoch (0 on wasm).
pub fn unix_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// The recorder needs a file system and threads.
pub fn unsupported() -> Option<EngineError> {
    cfg!(target_arch = "wasm32").then(|| EngineError::Other("timelapse recording needs the desktop app".into()))
}

fn ptr_eq(w: &Weak<Document>, d: &Arc<Document>) -> bool {
    std::ptr::eq(w.as_ptr(), Arc::as_ptr(d))
}

/// What changed since a track last looked at its document.
#[derive(Debug, PartialEq)]
enum Change {
    None,
    /// View-only or selection-only: counts as activity.
    Activity,
    /// A real edit; `visibility` when it only showed or hid layers.
    Edit { dead: Vec<u64>, visibility: bool },
    Undo(Vec<u64>),
    Redo(Vec<u64>),
}

impl Track {
    fn new(doc: DocId, rec: Recording, key: String) -> Self {
        Self {
            doc,
            rec,
            recording: false,
            user_paused: false,
            key,
            seen_rev: 0,
            seen_view: 0,
            cur: Weak::new(),
            top_past: None,
            past_len: 0,
            redo_len: 0,
            chain: Vec::new(),
            redo_chain: Vec::new(),
            next_step: 1,
            dirty: false,
            first_dirty: 0.0,
            last_activity: None,
            last_frame: None,
            pending: None,
            failures: 0,
            retry_at: 0.0,
            meta_dirty: false,
            last_save: 0.0,
            preview_rev: 1,
            rename_at: 0.0,
        }
    }

    /// The document `old` points to moved to `new` (its XMP was written): follow it, so the
    /// history mirror doesn't take it for an edit.
    fn repoint(&mut self, old: *const Document, new: &Arc<Document>) {
        let fix = |w: &mut Weak<Document>| {
            if std::ptr::eq(w.as_ptr(), old) {
                *w = Arc::downgrade(new);
            }
        };
        fix(&mut self.cur);
        if let Some(w) = &mut self.top_past {
            fix(w);
        }
        for c in self.chain.iter_mut().chain(self.redo_chain.iter_mut()) {
            fix(&mut c.1);
        }
    }

    /// Its frames were deleted (`file.timelapse.deleteFrames`).
    pub(crate) fn frames_cleared(&mut self) {
        self.last_frame = None;
        self.preview_rev += 1;
        self.meta_dirty = false;
    }

    /// Start mirroring the document's history from its current state.
    fn seed(&mut self, st: &DocState) {
        self.seen_rev = st.revision;
        self.seen_view = st.view_revision;
        self.cur = Arc::downgrade(&st.doc);
        self.past_len = st.history.past_len();
        self.redo_len = st.history.redo_labels().count();
        self.top_past = self.past_len.checked_sub(1).and_then(|i| st.history.state(i)).map(|d| Arc::downgrade(&d));
        let id = self.take_step();
        self.chain = vec![(id, Arc::downgrade(&st.doc))];
        self.redo_chain.clear();
    }

    fn take_step(&mut self) -> u64 {
        let id = self.next_step;
        self.next_step += 1;
        id
    }

    /// The step the document shows now.
    fn step(&self) -> u64 {
        self.chain.last().map_or(0, |c| c.0)
    }

    fn observe(&mut self, st: &DocState) -> Change {
        if st.revision == self.seen_rev {
            return Change::None;
        }
        let moved = st.revision.saturating_sub(self.seen_rev);
        let view = st.view_revision.saturating_sub(self.seen_view);
        self.seen_rev = st.revision;
        self.seen_view = st.view_revision;
        let doc = &st.doc;
        let (past, redo) = (st.history.past_len(), st.history.redo_labels().count());
        let (prev_past, prev_redo) = (self.past_len, self.redo_len);
        self.past_len = past;
        self.redo_len = redo;
        let top = past.checked_sub(1).and_then(|i| st.history.state(i));
        let top_changed = match (&self.top_past, &top) {
            (Some(w), Some(t)) => !ptr_eq(w, t),
            (None, Some(_)) => true,
            (_, None) => false,
        };
        self.top_past = top.as_ref().map(Arc::downgrade);
        if ptr_eq(&self.cur, doc) || (view >= moved && !top_changed && past == prev_past && redo == prev_redo) {
            // The same snapshot (a selection or view change), or only view revisions moved.
            return Change::Activity;
        }
        self.cur = Arc::downgrade(doc);
        // Back to an earlier snapshot: undo (or a click higher up in the History panel).
        if let Some(i) = self.chain.iter().position(|(_, w)| ptr_eq(w, doc)) {
            let undone = self.chain.split_off(i + 1);
            let ids = undone.iter().map(|c| c.0).collect();
            self.redo_chain.splice(0..0, undone);
            return Change::Undo(ids);
        }
        // Forward to an undone snapshot: redo.
        if let Some(i) = self.redo_chain.iter().position(|(_, w)| ptr_eq(w, doc)) {
            let redone: Vec<_> = self.redo_chain.drain(..=i).collect();
            let ids = redone.iter().map(|c| c.0).collect();
            self.chain.extend(redone);
            return Change::Redo(ids);
        }
        // A snapshot not seen yet whose counts say undo or redo (the snapshot was copied, as an
        // undo across a Save As does): go by the counts.
        if redo > prev_redo && past < prev_past {
            let k = (redo - prev_redo).min(self.chain.len().saturating_sub(1));
            let undone = self.chain.split_off(self.chain.len() - k);
            let ids = undone.iter().map(|c| c.0).collect();
            self.redo_chain.splice(0..0, undone);
            if let Some(last) = self.chain.last_mut() {
                last.1 = Arc::downgrade(doc);
            }
            return Change::Undo(ids);
        }
        if redo > 0 && redo < prev_redo && past > prev_past {
            let k = (prev_redo - redo).min(self.redo_chain.len());
            let redone: Vec<_> = self.redo_chain.drain(..k).collect();
            let ids = redone.iter().map(|c| c.0).collect();
            self.chain.extend(redone);
            if let Some(last) = self.chain.last_mut() {
                last.1 = Arc::downgrade(doc);
            }
            return Change::Redo(ids);
        }
        // A new snapshot: an edit. With nothing left to redo, undone steps are gone for good.
        let dead = if redo == 0 { self.redo_chain.drain(..).map(|c| c.0).collect() } else { Vec::new() };
        if top_changed || self.chain.is_empty() {
            let id = self.take_step();
            self.chain.push((id, Arc::downgrade(doc)));
            if self.chain.len() > MAX_CHAIN {
                self.chain.remove(0);
            }
        } else if let Some(last) = self.chain.last_mut() {
            // The same step changed again (a coalesced edit, or a change outside the history).
            last.1 = Arc::downgrade(doc);
        }
        let visibility = st.history.undo_label().is_some_and(|l| VISIBILITY_STEPS.contains(&l));
        Change::Edit { dead, visibility }
    }

    fn active_gap(&self, now: f64, idle: f64) -> f64 {
        self.last_activity.map_or(0.0, |t| (now - t).clamp(0.0, idle))
    }

    /// Work time so far (ms).
    pub fn work_now(&self, now: f64, idle: f64) -> u64 {
        let live = if self.recording { self.active_gap(now, idle) } else { 0.0 };
        self.rec.meta.work_ms.saturating_add(live as u64)
    }

    fn touch(&mut self, now: f64, idle: f64) {
        if !self.recording {
            return;
        }
        if self.last_activity.is_some() {
            self.rec.meta.work_ms = self.rec.meta.work_ms.saturating_add(self.active_gap(now, idle) as u64);
        }
        self.last_activity = Some(self.last_activity.map_or(now, |t| t.max(now)));
    }

    fn commit_work(&mut self, now: f64, idle: f64) {
        if self.last_activity.is_some() {
            self.rec.meta.work_ms = self.rec.meta.work_ms.saturating_add(self.active_gap(now, idle) as u64);
            self.last_activity = None;
        }
    }

    fn mark_dirty(&mut self, now: f64) {
        if !self.dirty {
            self.first_dirty = now;
        }
        self.dirty = true;
    }

    /// When the next frame is due (`None`: nothing to capture).
    pub fn due(&self) -> Option<f64> {
        if !self.recording || !self.dirty || self.pending.is_some() {
            return None;
        }
        let quiet = self.last_activity.unwrap_or(self.first_dirty) + SETTLE_MS;
        Some(quiet.min(self.first_dirty + MAX_WAIT_MS).max(self.retry_at))
    }

    /// Flag frames of these steps (this session's) undone or live again.
    fn mark(&mut self, steps: &[u64], undone: bool) -> bool {
        let mut changed = false;
        for f in self.rec.meta.frames.iter_mut().rev() {
            if f.s != self.key {
                break;
            }
            if steps.contains(&f.h) && (f.u != 0) != undone {
                f.u = u8::from(undone);
                changed = true;
            }
        }
        changed
    }

    /// Steps that are gone for good: their frames stay undone and are never matched again.
    fn bury(&mut self, steps: &[u64]) {
        for f in self.rec.meta.frames.iter_mut().rev() {
            if f.s != self.key {
                break;
            }
            if steps.contains(&f.h) {
                f.u = 1;
                f.h = 0;
            }
        }
    }

    /// Frames that go into the video.
    pub fn live_frames(&self, hide_undone: bool) -> usize {
        self.rec.meta.usable(hide_undone).count()
    }

    fn save(&mut self, now: f64) -> Result<()> {
        self.meta_dirty = false;
        self.last_save = now;
        self.rec.save(unix_ms())
    }
}

/// Hide every visible layer whose name contains `tag` (lowercase); a tagged group hides all it
/// holds. Returns whether anything was hidden.
pub fn hide_reference_layers(doc: &mut Document, tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    let mut any = false;
    let mut stack: Vec<&mut photocraft_doc::Layer> = doc.layers.iter_mut().collect();
    while let Some(l) = stack.pop() {
        if !l.visible {
            continue;
        }
        if l.name.to_lowercase().contains(tag) {
            l.visible = false;
            any = true;
            continue;
        }
        if let Some(ch) = l.children_mut() {
            stack.extend(ch.iter_mut());
        }
    }
    any
}

/// Render one frame: the document with reference layers hidden, reduced and in sRGB.
pub fn render_frame(doc: &Document, max_long: u32, tag: &str) -> Result<Frame> {
    let mut copy;
    let doc = if tag.is_empty() {
        doc
    } else {
        copy = doc.clone();
        if hide_reference_layers(&mut copy, tag) { &copy } else { doc }
    };
    let (width, height, rgb) = photocraft_io::psd_thumbnail::srgb_preview(doc, max_long).map_err(|e| EngineError::Other(format!("timelapse frame: {e}")))?;
    Ok(Frame { width, height, rgb })
}

/// Encode a frame as JPEG.
pub fn encode_jpeg(f: &Frame) -> Result<Vec<u8>> {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let img = Image::from_u8(f.width, f.height, ChannelLayout::Rgb, f.rgb.clone()).map_err(|e| EngineError::Other(e.to_string()))?;
    let opts = EncodeOptions { jpeg_quality: FRAME_QUALITY, embed_icc: false, embed_metadata: false, ..EncodeOptions::default() };
    photocraft_codecs::encode(&img, Format::Jpeg, &opts).map_err(|e| EngineError::Other(e.to_string()))
}

/// Runs `f` (a capture) on a quarter of the cores. A frame is never urgent, and rendering one
/// on every core every few seconds stuttered the brush painting beside it.
fn in_capture_pool<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
        let pool = POOL.get_or_init(|| {
            let n = std::thread::available_parallelism().map_or(4, |n| n.get());
            rayon::ThreadPoolBuilder::new().num_threads((n / 4).max(1)).thread_name(|i| format!("photocraft-timelapse-{i}")).build().ok()
        });
        if let Some(p) = pool {
            return p.install(f);
        }
    }
    f()
}

/// The capture worker's job: render, skip if unchanged, write.
fn capture_work(doc: Arc<Document>, max_long: u32, tag: String, last: Option<Arc<Frame>>, path: PathBuf) -> Outcome {
    let frame = match render_frame(&doc, max_long, &tag) {
        Ok(f) => f,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    drop(doc);
    if let Some(last) = last
        && last.width == frame.width
        && last.height == frame.height
        && super::raster::count_changed(&frame.rgb, &last.rgb, SIMILAR_TOL, SIMILAR_PX) <= SIMILAR_PX
    {
        return Outcome::Unchanged;
    }
    let bytes = match encode_jpeg(&frame) {
        Ok(b) => b,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Outcome::Failed(format!("{}: {e}", dir.display()));
    }
    match std::fs::write(&path, &bytes) {
        Ok(()) => Outcome::Captured { frame: Arc::new(frame), bytes: bytes.len() as u64 },
        Err(e) => Outcome::Failed(format!("{}: {e}", path.display())),
    }
}

impl Drop for Recorder {
    /// The app is closing (or a session ends): finish frames being captured and save every
    /// recording with its work time. A recording that was recording stays so, and resumes when its
    /// document is opened again.
    fn drop(&mut self) {
        let now = mono_ms();
        self.wait(now);
        let idle = self.settings.as_ref().map_or(30_000.0, Settings::idle_ms);
        for t in &mut self.tracks {
            t.commit_work(now, idle);
            let _ = t.save(now);
        }
    }
}

impl Recorder {
    /// The settings, loaded from the preferences on first use.
    pub fn settings(&mut self, prefs: &crate::prefs::Preferences) -> &Settings {
        self.settings.get_or_insert_with(|| prefs.dialogs.get(SETTINGS_KEY).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default())
    }

    pub(crate) fn set_settings(&mut self, s: Settings) {
        self.settings = Some(s);
    }

    /// Look at every open document again for a linked recording (the folder changed).
    pub(crate) fn reset_probed(&mut self) {
        self.probed.clear();
    }

    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    pub fn track(&self, doc: DocId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.doc == doc)
    }

    pub(crate) fn track_mut(&mut self, doc: DocId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.doc == doc)
    }

    /// A fresh recording id (UUID-shaped).
    fn new_id(&mut self, doc: DocId) -> String {
        self.counter += 1;
        let mut h = blake3::Hasher::new();
        h.update(&unix_ms().to_le_bytes());
        h.update(&mono_ms().to_bits().to_le_bytes());
        h.update(&doc.0.to_le_bytes());
        h.update(&self.counter.to_le_bytes());
        h.update(&std::process::id().to_le_bytes());
        let hex = h.finalize().to_hex();
        let s = hex.as_str();
        let part = |a: usize, b: usize| s.get(a..b).unwrap_or("0");
        format!("{}-{}-4{}-a{}-{}", part(0, 8), part(8, 12), part(13, 16), part(17, 20), part(20, 32))
    }

    fn key(&mut self) -> String {
        self.counter += 1;
        let h = blake3::hash(&[unix_ms().to_le_bytes(), self.counter.to_le_bytes()].concat());
        h.to_hex().as_str().get(..8).unwrap_or("00000000").to_string()
    }

    /// The recording a document is linked to: by the id in its XMP (which the Photoshop plug-in
    /// writes), else by its file path. Recordings already open elsewhere don't count.
    fn find_linked(&self, root: &std::path::Path, st: &DocState) -> (Option<Recording>, Option<String>) {
        let xmp_id = st.doc.metadata.xmp.as_deref().and_then(store::xmp_recording_id);
        let open: HashSet<&str> = self.tracks.iter().map(|t| t.rec.meta.id.as_str()).collect();
        let all = store::scan(root);
        let by_id = xmp_id.as_deref().and_then(|id| all.iter().find(|r| r.meta.id == id));
        let path = st.path.as_deref().unwrap_or("");
        let by_path = || all.iter().find(|r| store::same_path(&r.meta.doc_path, path));
        let found = by_id.or_else(by_path).filter(|r| !open.contains(r.meta.id.as_str())).cloned();
        (found, xmp_id)
    }

    /// Start (or resume) recording `st`. `fresh` captures the current state right away.
    pub(crate) fn start(&mut self, st: &DocState, now: f64) -> Result<()> {
        if let Some(e) = unsupported() {
            return Err(e);
        }
        let folder = self.settings.as_ref().map(|s| s.folder.clone()).unwrap_or_default();
        if folder.trim().is_empty() {
            return Err(EngineError::Other("choose a folder for the recordings first (file.timelapse.settings {\"folder\": …})".into()));
        }
        let id = st.doc.id;
        if self.track(id).is_none() {
            let root = PathBuf::from(&folder);
            std::fs::create_dir_all(&root).map_err(|e| store::io_err(&root, e))?;
            let (found, xmp_id) = self.find_linked(&root, st);
            let rec = match found {
                Some(r) => r,
                None => {
                    let open: HashSet<String> = self.tracks.iter().map(|t| t.rec.meta.id.clone()).collect();
                    let rid = xmp_id.filter(|x| !open.contains(x)).unwrap_or_else(|| self.new_id(id));
                    store::create(&root, &rid, &st.doc.name, st.path.as_deref().unwrap_or(""), unix_ms())?
                }
            };
            let key = self.key();
            self.tracks.push(Track::new(id, rec, key));
            self.probed.insert(id);
        }
        self.no_auto.remove(&id);
        let t = self.track_mut(id).ok_or(EngineError::NoDocument)?;
        // Resuming keeps the history mirror: what happened while paused is caught up on the next
        // look (an undo still flags the frames it undid).
        if t.chain.is_empty() {
            t.seed(st);
        }
        if !t.recording {
            t.recording = true;
            t.user_paused = false;
            t.last_activity = Some(now);
            t.rec.meta.state = "recording".into();
            t.mark_dirty(now);
            // The first frame shows the document as it is now.
            t.first_dirty = now - MAX_WAIT_MS;
            t.save(now)?;
        }
        Ok(())
    }

    /// Pause recording `doc`.
    pub(crate) fn pause(&mut self, doc: DocId, now: f64, by_hand: bool) -> Result<()> {
        let idle = self.settings.as_ref().map_or(30_000.0, Settings::idle_ms);
        let t = self.track_mut(doc).ok_or_else(|| EngineError::Other("this document is not being recorded".into()))?;
        t.commit_work(now, idle);
        t.recording = false;
        t.dirty = false;
        t.user_paused |= by_hand;
        t.rec.meta.state = "paused".into();
        t.save(now)
    }

    /// Wait for in-flight captures to finish and apply them.
    pub(crate) fn wait(&mut self, now: f64) {
        for t in &mut self.tracks {
            if let Some(p) = &mut t.pending
                && let Some(h) = p.handle.take()
            {
                let _ = h.join();
            }
        }
        self.collect(now);
    }

    /// Apply finished captures.
    fn collect(&mut self, now: f64) {
        let idle = self.settings.as_ref().map_or(30_000.0, Settings::idle_ms);
        let mut messages = Vec::new();
        for t in &mut self.tracks {
            let Some(p) = &t.pending else { continue };
            let outcome = match p.rx.try_recv() {
                Ok(o) => o,
                Err(mpsc::TryRecvError::Empty) => continue,
                Err(mpsc::TryRecvError::Disconnected) => Outcome::Failed("the capture stopped unexpectedly".into()),
            };
            let (n, step) = (p.n, p.step);
            t.pending = None;
            match outcome {
                Outcome::Captured { frame, bytes } => {
                    t.failures = 0;
                    let rec = &mut t.rec.meta;
                    rec.next_index = rec.next_index.max(n + 1);
                    rec.recorded = unix_ms();
                    // A step undone while its frame was being made is flagged right away.
                    let undone = t.redo_chain.iter().any(|c| c.0 == step);
                    let fm = FrameMeta { n, t: rec.recorded, w: 0, x: frame.width, y: frame.height, b: bytes, s: t.key.clone(), h: step, u: u8::from(undone), ..Default::default() };
                    rec.bytes = rec.bytes.saturating_add(bytes);
                    rec.frames.push(fm);
                    let work = t.work_now(now, idle);
                    if let Some(f) = t.rec.meta.frames.last_mut() {
                        f.w = work;
                    }
                    t.last_frame = Some(frame);
                    t.preview_rev += 1;
                    t.meta_dirty = true;
                }
                Outcome::Unchanged => {}
                Outcome::Failed(e) => {
                    // Try again soon (a full disk, a folder that went away…).
                    t.dirty = true;
                    t.failures += 1;
                    t.retry_at = now + (400.0 * 2f64.powi(t.failures.min(5) as i32)).min(8000.0);
                    if t.failures == 3 {
                        messages.push(format!("Timelapse: couldn't save a frame of “{}” ({e}); trying again.", t.rec.meta.name));
                    }
                }
            }
        }
        self.messages.extend(messages);
    }

    /// Start a capture of `st` on a worker thread.
    fn spawn_capture(&mut self, st: &DocState) {
        let (max_long, tag) = self.settings.as_ref().map_or((1920, "[ref]".to_string()), |s| (s.max_long(), s.tag()));
        let Some(t) = self.track_mut(st.doc.id) else { return };
        t.dirty = false;
        let n = t.rec.meta.next_index;
        t.rec.meta.next_index += 1;
        let step = t.step();
        let path = t.rec.frame_path(n);
        let (doc, last) = (st.doc.clone(), t.last_frame.clone());
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("photocraft-timelapse".into()).spawn({
            let tx = tx.clone();
            let tag = tag.clone();
            let path = path.clone();
            let doc = doc.clone();
            let last = last.clone();
            move || {
                let work = || capture_work(doc, max_long, tag, last, path);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| in_capture_pool(work)))
                    .unwrap_or_else(|_| Outcome::Failed("the capture failed with an internal error".into()));
                let _ = tx.send(r);
            }
        });
        let handle = match spawned {
            Ok(h) => Some(h),
            Err(_) => {
                // No threads: capture inline.
                let _ = tx.send(capture_work(doc, max_long, tag, last, path));
                None
            }
        };
        t.pending = Some(Pending { rx, n, step, handle });
    }

    /// Look at every open document once: follow edits, schedule and collect captures, keep the
    /// work timer and `recording.json` up to date, start auto recording. `input` says the user did
    /// something this frame (pointer or keys), which keeps the work timer running. Returns when
    /// it next needs to run (`None`: only on the next change).
    pub(crate) fn tick(&mut self, docs: &[DocState], active: Option<usize>, prefs: &crate::prefs::Preferences, now: f64, input: bool) -> Option<f64> {
        if unsupported().is_some() {
            return None;
        }
        let settings = self.settings(prefs).clone();
        let idle = settings.idle_ms();
        self.collect(now);

        // Documents that closed: keep their recording's state (a recording one resumes when the
        // file is opened again).
        let open: HashSet<DocId> = docs.iter().map(|d| d.doc.id).collect();
        let mut i = 0;
        while i < self.tracks.len() {
            if self.tracks.get(i).is_some_and(|t| !open.contains(&t.doc)) {
                let mut t = self.tracks.remove(i);
                if let Some(p) = &mut t.pending
                    && let Some(h) = p.handle.take()
                {
                    let _ = h.join();
                }
                t.commit_work(now, idle);
                let _ = t.save(now);
            } else {
                i += 1;
            }
        }
        self.probed.retain(|d| open.contains(d));
        self.no_auto.retain(|d| open.contains(d));

        // Newly opened documents linked to a recording that was recording: resume it.
        if !settings.folder.trim().is_empty() {
            for st in docs {
                if self.probed.insert(st.doc.id) {
                    let root = PathBuf::from(&settings.folder);
                    if let (Some(rec), _) = self.find_linked(&root, st)
                        && rec.meta.state == "recording"
                    {
                        let name = rec.meta.name.clone();
                        let key = self.key();
                        self.tracks.push(Track::new(st.doc.id, rec, key));
                        if self.start(st, now).is_ok() {
                            self.messages.push(format!("Resumed the timelapse recording of “{name}”."));
                        }
                    }
                }
            }
            // Auto record: whatever document is active.
            if settings.auto_record
                && let Some(st) = active.and_then(|i| docs.get(i))
                && !self.no_auto.contains(&st.doc.id)
                && self.track(st.doc.id).is_none_or(|t| !t.recording && !t.user_paused)
                && self.start(st, now).is_ok()
            {
                self.messages.push(format!("Recording a timelapse of “{}”.", st.doc.name));
            }
        }

        let active_id = active.and_then(|i| docs.get(i)).map(|d| d.doc.id);
        let mut capture = Vec::new();
        let mut next: Option<f64> = None;
        for t in &mut self.tracks {
            let Some(st) = docs.iter().find(|d| d.doc.id == t.doc) else { continue };
            if !t.recording {
                continue;
            }
            if input && active_id == Some(t.doc) {
                t.touch(now, idle);
            }
            match t.observe(st) {
                Change::None => {}
                Change::Activity => t.touch(now, idle),
                Change::Edit { dead, visibility } => {
                    t.touch(now, idle);
                    if !dead.is_empty() {
                        t.bury(&dead);
                        t.meta_dirty = true;
                    }
                    if visibility {
                        // No frame until the next real edit.
                        t.dirty = false;
                    } else {
                        t.mark_dirty(now);
                    }
                    if t.rec.meta.doc_path.is_empty()
                        && let Some(p) = &st.path
                    {
                        t.rec.meta.doc_path.clone_from(p);
                        t.meta_dirty = true;
                    }
                }
                Change::Undo(steps) => {
                    t.touch(now, idle);
                    if settings.hide_undone {
                        if t.mark(&steps, true) {
                            t.meta_dirty = true;
                            t.preview_rev += 1;
                            t.last_frame = None;
                        }
                    } else {
                        t.mark_dirty(now);
                    }
                }
                Change::Redo(steps) => {
                    t.touch(now, idle);
                    if settings.hide_undone {
                        if t.mark(&steps, false) {
                            t.meta_dirty = true;
                            t.preview_rev += 1;
                            t.last_frame = None;
                        }
                    } else {
                        t.mark_dirty(now);
                    }
                }
            }
            if let Some(due) = t.due() {
                if now >= due {
                    capture.push(t.doc);
                } else {
                    next = Some(next.map_or(due, |n: f64| n.min(due)));
                }
            }
            if t.pending.is_some() {
                next = Some(next.map_or(now + 50.0, |n: f64| n.min(now + 50.0)));
            }
            if t.meta_dirty && now - t.last_save >= SAVE_EVERY_MS && t.pending.is_none() {
                let _ = t.save(now);
            }
            if t.meta_dirty {
                let at = t.last_save + SAVE_EVERY_MS;
                next = Some(next.map_or(at, |n: f64| n.min(at)));
            }
            // While recording, the work timer moves: the panel shows it once a second.
            next = Some(next.map_or(now + 1000.0, |n: f64| n.min(now + 1000.0)));
        }
        for id in capture {
            if let Some(st) = docs.iter().find(|d| d.doc.id == id) {
                self.spawn_capture(st);
            }
        }
        next
    }

    /// Keep each tracked document and its recording linked, as the plug-in's `ensureLinked` does:
    /// - the recording id goes into the document's XMP, so the saved file finds its recording
    ///   when renamed or moved. Written in place without a history step or a revision, so a
    ///   clean document stays clean; again after an undo to a state from before (documents
    ///   `locked` by a background job wait for it to end);
    /// - a new name (Save As) renames the recording and its folder, between captures and never
    ///   while `exporting` (the export reads the folder); a new path is remembered.
    pub(crate) fn sync_docs(&mut self, docs: &mut [DocState], locked: &[DocId], exporting: bool, now: f64) {
        if unsupported().is_some() || self.tracks.is_empty() {
            return;
        }
        self.collect(now);
        let mut messages = Vec::new();
        for t in &mut self.tracks {
            let Some(st) = docs.iter_mut().find(|d| d.doc.id == t.doc) else { continue };
            if !locked.contains(&t.doc) && st.doc.metadata.xmp.as_deref().and_then(store::xmp_recording_id).as_deref() != Some(t.rec.meta.id.as_str()) {
                let old = Arc::as_ptr(&st.doc);
                let xmp = store::xmp_with_recording_id(st.doc.metadata.xmp.as_deref(), &t.rec.meta.id);
                Arc::make_mut(&mut st.doc).metadata.xmp = Some(xmp);
                t.repoint(old, &st.doc);
            }
            let mut save = false;
            if let Some(p) = st.path.as_deref().filter(|p| !p.is_empty() && *p != t.rec.meta.doc_path) {
                t.rec.meta.doc_path = p.to_string();
                save = true;
            }
            if !exporting && t.pending.is_none() && now >= t.rename_at && store::needs_rename(&t.rec, &st.doc.name) {
                let old = t.rec.meta.name.clone();
                match store::rename(&mut t.rec, &st.doc.name, unix_ms()) {
                    Ok(true) => {
                        save = false;
                        t.meta_dirty = false;
                    }
                    Ok(false) => t.rename_at = now + 5000.0,
                    Err(e) => {
                        t.rename_at = now + 5000.0;
                        messages.push(format!("Timelapse: couldn't rename the recording “{old}” ({e})."));
                    }
                }
            }
            if save {
                let _ = t.save(now);
            }
        }
        self.messages.extend(messages);
    }

    /// Capture `st` now (waiting for the frame), whatever the schedule says. `false` when nothing
    /// visible changed since the last frame.
    pub(crate) fn capture_now(&mut self, st: &DocState, now: f64) -> Result<bool> {
        let t = self.track(st.doc.id).ok_or_else(|| EngineError::Other("this document is not being recorded".into()))?;
        if !t.recording {
            return Err(EngineError::Other("the recording is paused".into()));
        }
        self.wait(now);
        let before = self.track(st.doc.id).map_or(0, |t| t.rec.meta.frames.len());
        self.spawn_capture(st);
        self.wait(now);
        let t = self.track_mut(st.doc.id).ok_or(EngineError::NoDocument)?;
        let _ = t.save(now);
        Ok(t.rec.meta.frames.len() > before)
    }

    /// Stop tracking `doc` without saving its recording again (it was deleted).
    pub(crate) fn forget(&mut self, doc: DocId) {
        self.tracks.retain(|t| t.doc != doc);
        self.no_auto.insert(doc);
    }

    /// Flush `recording.json` of `doc` (before an export reads it).
    pub(crate) fn flush(&mut self, doc: DocId, now: f64) -> Result<()> {
        self.wait(now);
        match self.track_mut(doc) {
            Some(t) => t.save(now),
            None => Ok(()),
        }
    }

    /// Status of `doc`'s recording for the panel and agents.
    pub fn status(&self, doc: Option<&DocState>, now: f64) -> Value {
        let s = self.settings.clone().unwrap_or_default();
        let idle = s.idle_ms();
        let track = doc.and_then(|d| self.track(d.doc.id));
        let rec = track.map(|t| {
            let m = &t.rec.meta;
            json!({
                "id": m.id,
                "name": m.name,
                "folder": t.rec.dir.to_string_lossy(),
                "recording": t.recording,
                "idle": t.recording && t.last_activity.is_none_or(|a| now - a >= idle),
                "frames": t.live_frames(s.hide_undone),
                "allFrames": m.frames.len(),
                "workMs": t.work_now(now, idle),
                "bytes": m.bytes,
                "exports": m.exports.len(),
                "lastExport": m.exports.last().map(|e| e.path.clone()),
                "framesSinceExport": m.exports.last().map_or(m.frames.len(), |e| m.frames.len().saturating_sub(e.frames)),
                "previewRev": t.preview_rev,
            })
        });
        json!({
            "available": unsupported().is_none(),
            "document": doc.map(|d| d.doc.name.clone()),
            "recording": rec,
            "settings": s,
        })
    }

    /// The newest frame of `doc` that goes into the video, for the preview.
    pub fn preview(&self, doc: DocId) -> Option<(u64, PathBuf, Option<Arc<Frame>>)> {
        let t = self.track(doc)?;
        let hide = self.settings.as_ref().is_none_or(|s| s.hide_undone);
        let f = t.rec.meta.usable(hide).last()?;
        let live = t.last_frame.clone().filter(|_| t.rec.meta.frames.last().is_some_and(|l| l.n == f.n));
        Some((t.preview_rev, t.rec.frame_path(f.n), live))
    }
}
