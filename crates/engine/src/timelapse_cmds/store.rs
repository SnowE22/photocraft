//! Recordings on disk, in the Photoshop Timelapse Recorder's layout so either app can continue or
//! export the other's recordings:
//!
//! ```text
//! <recordings folder>/
//!   Portrait (1a2b3c4d)/
//!     recording.json        frame list, work time, export history
//!     frames/f_000001.jpg …  the captured frames
//!     thumbs32.bin          flicker-check thumbnails (see `deflicker`)
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{EngineError, Result};

pub const META: &str = "recording.json";
pub const FRAMES: &str = "frames";
/// Largest `recording.json` read (a recording of 100 000 frames is about 12 MB).
const MAX_META_BYTES: u64 = 64 << 20;

/// One captured frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameMeta {
    /// Frame number (`frames/f_{n:06}.jpg`).
    pub n: u64,
    /// When it was captured (ms since the Unix epoch).
    pub t: u64,
    /// Work time at capture (ms).
    pub w: u64,
    /// Pixel size.
    pub x: u32,
    pub y: u32,
    /// File size in bytes.
    pub b: u64,
    /// Session key: the open-document session that captured it (history ids restart on reopen).
    pub s: String,
    /// The history step it shows (within session `s`).
    pub h: u64,
    /// 1 while that step is undone (left out of the video when "Remove frames with undo" is on).
    pub u: u8,
    /// Keys this app doesn't know, kept as they are.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One export.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportMeta {
    pub t: u64,
    pub path: String,
    pub mode: String,
    pub frames: usize,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `recording.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Meta {
    pub version: u32,
    pub id: String,
    pub name: String,
    pub doc_path: String,
    pub created: u64,
    pub updated: u64,
    /// When something was last recorded (the recordings list's order).
    pub recorded: u64,
    /// "recording" (resumes when the document is opened again) or "paused".
    pub state: String,
    pub work_ms: u64,
    pub bytes: u64,
    pub next_index: u64,
    pub frames: Vec<FrameMeta>,
    pub exports: Vec<ExportMeta>,
    /// Highest frame number whose flicker thumbnail is cached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbs_up_to: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            version: 1,
            id: String::new(),
            name: String::new(),
            doc_path: String::new(),
            created: 0,
            updated: 0,
            recorded: 0,
            state: "paused".into(),
            work_ms: 0,
            bytes: 0,
            next_index: 1,
            frames: Vec::new(),
            exports: Vec::new(),
            thumbs_up_to: None,
            extra: Map::new(),
        }
    }
}

impl Meta {
    /// Repair what an older or hand-edited file may lack (the plug-in's `normalizeMeta`).
    fn normalize(&mut self) {
        let max_n = self.frames.iter().map(|f| f.n).max().unwrap_or(0);
        self.next_index = self.next_index.max(max_n.saturating_add(1)).max(1);
        if self.bytes == 0 {
            self.bytes = self.frames.iter().map(|f| f.b).sum();
        }
        if self.state != "recording" {
            self.state = "paused".into();
        }
        if self.recorded == 0 {
            self.recorded = self.frames.last().map_or(self.created, |f| f.t);
        }
    }

    /// Frames that go into the video.
    pub fn usable(&self, hide_undone: bool) -> impl Iterator<Item = &FrameMeta> {
        self.frames.iter().filter(move |f| !(hide_undone && f.u != 0))
    }
}

/// A recording: its folder and metadata.
#[derive(Clone, Debug)]
pub struct Recording {
    pub dir: PathBuf,
    pub meta: Meta,
}

/// `f_000001.jpg`.
pub fn frame_name(n: u64) -> String {
    format!("f_{n:06}.jpg")
}

impl Recording {
    pub fn frames_dir(&self) -> PathBuf {
        self.dir.join(FRAMES)
    }
    pub fn frame_path(&self, n: u64) -> PathBuf {
        self.frames_dir().join(frame_name(n))
    }

    /// Write `recording.json` (through a temporary file, so a crash never leaves half a file).
    pub fn save(&mut self, now: u64) -> Result<()> {
        self.meta.updated = now;
        let text = serde_json::to_vec(&self.meta).map_err(|e| EngineError::Other(format!("timelapse: {e}")))?;
        let tmp = self.dir.join(format!("{META}.tmp"));
        std::fs::write(&tmp, text).map_err(|e| io_err(&tmp, e))?;
        std::fs::rename(&tmp, self.dir.join(META)).map_err(|e| io_err(&self.dir, e))
    }
}

pub fn io_err(path: &Path, e: std::io::Error) -> EngineError {
    EngineError::Other(format!("{}: {e}", path.display()))
}

/// A document name as a folder name: no extension, no characters Windows refuses, at most 60
/// characters.
pub fn safe_name(name: &str) -> String {
    let stem = match name.rsplit_once('.') {
        Some((a, ext)) if !a.is_empty() && !ext.contains(' ') && ext.len() <= 5 => a,
        _ => name,
    };
    let mut s = String::new();
    let mut last_bad = false;
    for c in stem.chars() {
        let bad = matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control();
        if bad {
            if !last_bad {
                s.push('_');
            }
        } else if c.is_whitespace() {
            if !s.ends_with(' ') {
                s.push(' ');
            }
        } else {
            s.push(c);
        }
        last_bad = bad;
    }
    let s: String = s.trim().chars().take(60).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

/// "Portrait (1a2b3c4d)": the document's name plus the start of the recording id.
pub fn folder_name(id: &str, nice: &str) -> String {
    let short: String = id.chars().take(8).collect();
    format!("{nice} ({short})")
}

/// Read one recording folder (`None` when it holds no readable `recording.json`).
pub fn load(dir: &Path) -> Option<Recording> {
    let path = dir.join(META);
    let len = std::fs::metadata(&path).ok()?.len();
    if len > MAX_META_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let mut meta: Meta = serde_json::from_slice(&bytes).ok()?;
    if meta.id.is_empty() {
        return None;
    }
    meta.normalize();
    Some(Recording { dir: dir.to_path_buf(), meta })
}

/// Recordings read by [`scan`], by `recording.json` path, with the size and time it had: the
/// Library rescans every few seconds, and a recording's file lists every frame.
type Scanned = std::collections::HashMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>, Recording)>;

/// Every recording in the recordings folder, most recently recorded first. A `recording.json`
/// whose size and modification time are unchanged since the last scan isn't read again.
pub fn scan(root: &Path) -> Vec<Recording> {
    static SCANNED: std::sync::Mutex<Option<Scanned>> = std::sync::Mutex::new(None);
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut cache = SCANNED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(Default::default);
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Recording> = Vec::new();
    for dir in entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
        let path = dir.join(META);
        let Ok(md) = std::fs::metadata(&path) else { continue };
        let stamp = (md.len(), md.modified().ok());
        // File times tick coarsely: a file written in the last two seconds is read again, so two
        // writes of the same size close together are both seen.
        let settled = stamp.1.and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() >= 2);
        let rec = match cache.get(&path) {
            Some((len, time, rec)) if settled && (*len, *time) == stamp => Some(rec.clone()),
            _ => load(&dir),
        };
        if let Some(rec) = rec {
            cache.insert(path.clone(), (stamp.0, stamp.1, rec.clone()));
            seen.insert(path);
            out.push(rec);
        }
    }
    // Forget folders that went away (or other roots), so the cache stays the size of the library.
    cache.retain(|p, _| seen.contains(p));
    out.sort_by_key(|r| std::cmp::Reverse(r.meta.recorded));
    out
}

/// Make a new recording folder.
pub fn create(root: &Path, id: &str, name: &str, doc_path: &str, now: u64) -> Result<Recording> {
    let nice = safe_name(name);
    let dir = root.join(folder_name(id, &nice));
    std::fs::create_dir_all(dir.join(FRAMES)).map_err(|e| io_err(&dir, e))?;
    let meta = Meta { id: id.into(), name: nice, doc_path: doc_path.into(), created: now, updated: now, recorded: now, ..Meta::default() };
    let mut rec = Recording { dir, meta };
    rec.save(now)?;
    Ok(rec)
}

/// Same file, ignoring case and slash direction (Windows paths).
pub fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    !a.is_empty() && norm(a) == norm(b)
}

/// The XMP namespace the Photoshop plug-in stores a document's recording id under.
pub const XMP_NS: &str = "http://ns.timelapse-recorder.local/1.0/";

/// Where the value of `<prefix>:RecordingID` is in `xmp` (as attribute or element), with the
/// namespace looked up by URI since another app may bind it to another prefix.
fn recording_id_span(xmp: &str) -> Option<std::ops::Range<usize>> {
    let at = xmp.find(XMP_NS)?;
    // `xmlns:PREFIX="URI"`: the prefix ends just before `="` in front of the URI.
    let head = xmp.get(..at)?;
    let decl = head.rfind("xmlns:")?;
    let prefix = head.get(decl + 6..)?.split(['=', ' ', '\t', '\n', '\r']).next()?.trim();
    if prefix.is_empty() {
        return None;
    }
    let key = format!("{prefix}:RecordingID");
    let mut from = 0;
    while let Some(rel) = xmp.get(from..)?.find(&key) {
        let pos = from + rel;
        let after = pos + key.len();
        from = after;
        if xmp.get(..pos)?.ends_with("</") {
            continue;
        }
        let rest = xmp.get(after..)?;
        let trimmed = rest.trim_start();
        let at = after + (rest.len() - trimmed.len());
        if let Some(r) = trimmed.strip_prefix('=') {
            let value = r.trim_start();
            let open = at + 1 + (r.len() - value.len());
            let q = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let start = open + 1;
            let end = start + xmp.get(start..)?.find(q)?;
            return Some(start..end);
        }
        if trimmed.starts_with('>') {
            let start = at + 1;
            let end = start + xmp.get(start..)?.find('<')?;
            return Some(start..end);
        }
    }
    None
}

/// The recording id in a document's XMP (`<prefix>:RecordingID`).
pub fn xmp_recording_id(xmp: &str) -> Option<String> {
    let v = xmp.get(recording_id_span(xmp)?)?.trim();
    (!v.is_empty() && v.len() <= 64).then(|| v.to_string())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// `xmp` carrying recording id `id` (the plug-in's `injectId`): the value replaced where the
/// document already has one, else an `rdf:Description` added; a new packet when there is no XMP.
pub fn xmp_with_recording_id(xmp: Option<&str>, id: &str) -> String {
    let id = xml_escape(id);
    let desc = format!("<rdf:Description rdf:about=\"\" xmlns:tlr=\"{XMP_NS}\" tlr:RecordingID=\"{id}\"/>");
    let Some(xmp) = xmp.filter(|x| x.contains("</rdf:RDF>")) else {
        return format!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">{desc}</rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
        );
    };
    if let Some(span) = recording_id_span(xmp)
        && let (Some(a), Some(b)) = (xmp.get(..span.start), xmp.get(span.end..))
    {
        return format!("{a}{id}{b}");
    }
    xmp.replacen("</rdf:RDF>", &format!("{desc}</rdf:RDF>"), 1)
}

/// Whether the recording still carries another name than the document's (after Save As).
pub fn needs_rename(rec: &Recording, doc_name: &str) -> bool {
    let nice = safe_name(doc_name);
    rec.meta.name != nice || rec.dir.file_name().is_none_or(|n| n.to_string_lossy() != folder_name(&rec.meta.id, &nice))
}

/// Rename the recording and its folder after the document ("Untitled-2 (b26b3312)" → "Tree
/// (b26b3312)"). The name always changes; the folder only when nothing else has that name and the
/// file system lets it (a file in it open elsewhere): `Ok(false)` then, and a later call tries
/// again. The caller makes sure no capture or export uses the folder meanwhile.
pub fn rename(rec: &mut Recording, doc_name: &str, now: u64) -> Result<bool> {
    let nice = safe_name(doc_name);
    let want_name = folder_name(&rec.meta.id, &nice);
    rec.meta.name = nice;
    let mut done = true;
    if rec.dir.file_name().is_none_or(|n| n.to_string_lossy() != want_name) {
        done = false;
        if let Some(root) = rec.dir.parent() {
            let want = root.join(&want_name);
            // A case-only rename is the same folder on Windows and macOS.
            let taken = want.exists() && !same_path(&want.to_string_lossy(), &rec.dir.to_string_lossy());
            if !taken && std::fs::rename(&rec.dir, &want).is_ok() {
                rec.dir = want;
                done = true;
            }
        }
    }
    rec.save(now)?;
    Ok(done)
}

/// Whether `name` is a frame file (`f_000001.jpg`).
fn is_frame_file(name: &str) -> bool {
    name.strip_prefix("f_").and_then(|r| r.strip_suffix(".jpg")).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Delete a recording's frames (and its flicker thumbnails); its work time, export history and
/// frame numbering stay. Returns how many frame files were deleted; files that couldn't be (open
/// elsewhere) are left and reported.
pub fn clear_frames(rec: &mut Recording, now: u64) -> Result<usize> {
    let mut deleted = 0;
    let mut failed = 0;
    if let Ok(entries) = std::fs::read_dir(rec.frames_dir()) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_file()) && is_frame_file(&e.file_name().to_string_lossy()) {
                if std::fs::remove_file(e.path()).is_ok() { deleted += 1 } else { failed += 1 }
            }
        }
    }
    let _ = std::fs::remove_file(rec.dir.join(super::deflicker::THUMBS_FILE));
    rec.meta.frames.clear();
    rec.meta.bytes = 0;
    rec.meta.thumbs_up_to = None;
    rec.save(now)?;
    if failed > 0 {
        return Err(EngineError::Other(format!("{failed} frame files in {} couldn't be deleted (open in another program?)", rec.frames_dir().display())));
    }
    Ok(deleted)
}

/// Delete a recording whose frames are gone: `recording.json`, the flicker thumbnails and the
/// empty `frames` folder, then the folder itself when nothing else is in it (a video someone
/// saved there stays, with the folder). Returns the names of what was kept.
pub fn delete_recording(rec: &Recording) -> Result<Vec<String>> {
    let tmp = format!("{META}.tmp");
    for name in [META, tmp.as_str(), super::deflicker::THUMBS_FILE] {
        let p = rec.dir.join(name);
        if p.is_file() {
            std::fs::remove_file(&p).map_err(|e| io_err(&p, e))?;
        }
    }
    let frames = rec.frames_dir();
    if frames.is_dir() {
        let _ = std::fs::remove_dir(&frames);
    }
    let kept: Vec<String> = std::fs::read_dir(&rec.dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    if kept.is_empty() {
        std::fs::remove_dir(&rec.dir).map_err(|e| io_err(&rec.dir, e))?;
    }
    Ok(kept)
}
