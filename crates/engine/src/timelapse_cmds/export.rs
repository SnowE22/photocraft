//! Turning a recording into a video (a background job):
//!
//! 1. optionally leave out flicker frames ([`super::deflicker`]),
//! 2. plan which recorded frame each video frame shows ([`super::plan`]),
//! 3. draw each video frame: fitted into the final canvas size (a crop or canvas resize during the
//!    recording), the work-time badge, the white flash,
//! 4. pipe them as raw RGB into ffmpeg (H.264 MP4), or write them as a JPEG sequence.
//!
//! Unlike the Photoshop plug-in, nothing is staged on disk for ffmpeg: frames go straight down
//! its standard input, so progress is exact and Cancel stops it at once.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::{Value, json};

use super::deflicker;
use super::plan::{self, Part, PlanOptions};
use super::raster::{self, Corner};
use super::recorder::ExportSettings;
use super::store::{FrameMeta, Recording};
use crate::jobs::JobCtx;
use crate::{EngineError, Result};

/// Longest output side (8K): larger recordings are refused rather than allocating without bound.
const MAX_SIDE: u32 = 8192;

/// Everything an export needs, gathered on the UI thread.
#[derive(Clone, Debug)]
pub struct Job {
    pub rec: Recording,
    pub hide_undone: bool,
    pub o: ExportSettings,
    pub fps: f64,
    pub out: PathBuf,
    pub ffmpeg: PathBuf,
}

/// What an export made.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub file: String,
    pub mode: String,
    pub video_frames: usize,
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
    pub used_frames: usize,
    pub left_out: usize,
    pub warning: Option<String>,
}

impl Exported {
    pub fn to_json(&self) -> Value {
        json!({
            "file": self.file,
            "mode": self.mode,
            "videoFrames": self.video_frames,
            "seconds": self.seconds,
            "width": self.width,
            "height": self.height,
            "frames": self.used_frames,
            "flickerLeftOut": self.left_out,
            "warning": self.warning,
        })
    }
}

/// The ffmpeg to run: the configured one, `PHOTOCRAFT_FFMPEG`, one next to the app (or in its
/// `ffmpeg` / `ffmpeg/bin` folder), else `ffmpeg` from PATH.
pub fn find_ffmpeg(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured.trim());
    }
    if let Ok(p) = std::env::var("PHOTOCRAFT_FFMPEG")
        && !p.trim().is_empty()
    {
        return PathBuf::from(p.trim());
    }
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(app) = std::env::current_exe()
        && let Some(dir) = app.parent()
    {
        for c in [dir.join(exe), dir.join("ffmpeg").join(exe), dir.join("ffmpeg").join("bin").join(exe)] {
            if c.is_file() {
                return c;
            }
        }
    }
    PathBuf::from("ffmpeg")
}

fn even(v: u32) -> u32 {
    (v / 2 * 2).max(2)
}

fn bg_of(name: &str) -> [u8; 3] {
    match name {
        "white" => [255, 255, 255],
        "gray" => [45, 45, 45],
        _ => [0, 0, 0],
    }
}

/// Decode one stored frame to RGB8.
pub fn decode_frame(path: &Path) -> Result<(usize, usize, Vec<u8>)> {
    use photocraft_codecs::{ChannelLayout, Format, SampleType};
    let bytes = std::fs::read(path).map_err(|e| super::store::io_err(path, e))?;
    let img = photocraft_codecs::decode_as(Format::Jpeg, &bytes).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 || img.width() > MAX_SIDE * 2 || img.height() > MAX_SIDE * 2 {
        return Err(EngineError::Other(format!("{}: unexpected frame size {w}×{h}", path.display())));
    }
    let rgb = img.converted(ChannelLayout::Rgb, SampleType::U8).data().to_vec();
    Ok((w, h, rgb))
}

/// Frames that go into the video and whose file is there.
pub fn usable_frames(rec: &Recording, hide_undone: bool) -> Vec<FrameMeta> {
    rec.meta.usable(hide_undone).filter(|f| rec.frame_path(f.n).is_file()).cloned().collect()
}

/// Indexes into `frames` of the flicker frames, with the thumbnail cache brought up to date.
fn find_flicker(job: &Job, frames: &[FrameMeta], ctx: &JobCtx, lo: f32, hi: f32) -> Result<Vec<usize>> {
    let cache_path = job.rec.dir.join(deflicker::THUMBS_FILE);
    let mut cache = std::fs::read(&cache_path).map(|b| deflicker::parse_thumbs(&b)).unwrap_or_default();
    let missing: Vec<&FrameMeta> = frames.iter().filter(|f| u32::try_from(f.n).is_ok_and(|n| !cache.contains_key(&n))).collect();
    if !missing.is_empty() {
        let done = std::sync::atomic::AtomicUsize::new(0);
        let total = missing.len();
        let made: Vec<(u32, Vec<u8>)> = missing
            .par_iter()
            .map(|f| {
                ctx.check()?;
                let (w, h, rgb) = decode_frame(&job.rec.frame_path(f.n))?;
                let t = deflicker::thumbnail(&rgb, w, h).ok_or_else(|| EngineError::Other("flicker check: empty frame".into()))?;
                let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                ctx.progress(lo + (hi - lo) * k as f32 / total as f32, "Checking for flicker");
                Ok((u32::try_from(f.n).unwrap_or(u32::MAX), t))
            })
            .collect::<Result<_>>()?;
        cache.extend(made);
        // Frames deleted since: their thumbnails go too.
        let keep: std::collections::HashSet<u32> = job.rec.meta.frames.iter().filter_map(|f| u32::try_from(f.n).ok()).collect();
        cache.retain(|n, _| keep.contains(n));
        // The cache only saves time: a folder that can't be written is no reason to fail.
        let _ = std::fs::write(&cache_path, deflicker::serialize_thumbs(&cache));
    }
    let thumbs: Vec<Option<&[u8]>> = frames.iter().map(|f| u32::try_from(f.n).ok().and_then(|n| cache.get(&n)).map(Vec::as_slice)).collect();
    Ok(deflicker::find_flicker(&thumbs, deflicker::SENSITIVITY))
}

/// Where output frames go.
trait Sink {
    fn frame(&mut self, k: usize, rgb: &[u8], repeat: bool) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<()>;
    fn abort(self: Box<Self>);
}

struct Ffmpeg {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    errors: Option<std::thread::JoinHandle<String>>,
    out: PathBuf,
    exe: PathBuf,
}

impl Ffmpeg {
    fn start(exe: &Path, out: &Path, w: u32, h: u32, fps: f64) -> Result<Self> {
        use std::process::{Command, Stdio};
        let mut cmd = Command::new(exe);
        cmd.args(["-hide_banner", "-nostats", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s"])
            .arg(format!("{w}x{h}"))
            .arg("-framerate")
            .arg(format!("{fps}"))
            .args(["-i", "-", "-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p", "-c:v", "libx264", "-preset", "medium", "-crf", "18"])
            .args(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv", "-movflags", "+faststart"])
            .arg(out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashes up.
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                EngineError::Other(format!(
                    "ffmpeg wasn't found ({}). Install ffmpeg, set its path in the timelapse settings, or export a JPEG sequence instead",
                    exe.display()
                ))
            } else {
                EngineError::Other(format!("couldn't start ffmpeg ({}): {e}", exe.display()))
            }
        })?;
        let stdin = child.stdin.take();
        let errors = child.stderr.take().map(|mut err| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = std::io::Read::read_to_string(&mut err, &mut s);
                s
            })
        });
        Ok(Self { child, stdin, errors, out: out.to_path_buf(), exe: exe.to_path_buf() })
    }

    fn error_text(&mut self) -> String {
        let text = self.errors.take().and_then(|h| h.join().ok()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        lines.iter().rev().take(3).rev().copied().collect::<Vec<_>>().join(" ")
    }
}

impl Sink for Ffmpeg {
    fn frame(&mut self, _k: usize, rgb: &[u8], _repeat: bool) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else { return Err(EngineError::Other("ffmpeg's input is closed".into())) };
        if stdin.write_all(rgb).is_err() {
            // ffmpeg quit: say why.
            self.stdin = None;
            let _ = self.child.wait();
            let why = self.error_text();
            return Err(EngineError::Other(format!("ffmpeg stopped: {}", if why.is_empty() { "no message" } else { &why })));
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| EngineError::Other(format!("ffmpeg: {e}")))?;
        if status.success() {
            return Ok(());
        }
        let why = self.error_text();
        let _ = std::fs::remove_file(&self.out);
        Err(EngineError::Other(format!("ffmpeg ({}) stopped with an error: {}", self.exe.display(), if why.is_empty() { status.to_string() } else { why })))
    }

    fn abort(mut self: Box<Self>) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = self.errors.take().map(std::thread::JoinHandle::join);
        let _ = std::fs::remove_file(&self.out);
    }
}

/// A JPEG sequence (`frame_000001.jpg` …) in a folder.
struct Sequence {
    dir: PathBuf,
    w: u32,
    h: u32,
    last: Vec<u8>,
    written: Vec<PathBuf>,
}

impl Sink for Sequence {
    fn frame(&mut self, k: usize, rgb: &[u8], repeat: bool) -> Result<()> {
        if !repeat || self.last.is_empty() {
            let f = super::recorder::Frame { width: self.w, height: self.h, rgb: rgb.to_vec() };
            self.last = super::recorder::encode_jpeg(&f)?;
        }
        let path = self.dir.join(format!("frame_{:06}.jpg", k + 1));
        std::fs::write(&path, &self.last).map_err(|e| super::store::io_err(&path, e))?;
        self.written.push(path);
        Ok(())
    }
    fn finish(self: Box<Self>) -> Result<()> {
        Ok(())
    }
    fn abort(self: Box<Self>) {
        for p in &self.written {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Run the export (on a worker).
pub fn run(job: &Job, ctx: &JobCtx) -> Result<Exported> {
    ctx.progress(0.0, "Reading frames");
    let mut frames = usable_frames(&job.rec, job.hide_undone);
    if frames.is_empty() {
        return Err(EngineError::Other("this recording doesn't have any frames yet".into()));
    }
    let mut left_out = 0;
    let mut warning = None;
    let flicker_share = if job.o.deflicker { 0.25 } else { 0.0 };
    if job.o.deflicker {
        match find_flicker(job, &frames, ctx, 0.0, flicker_share) {
            Ok(drop) if !drop.is_empty() => {
                left_out = drop.len();
                let drop: std::collections::HashSet<usize> = drop.into_iter().collect();
                frames = frames.into_iter().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, f)| f).collect();
            }
            Ok(_) => {}
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(e) => warning = Some(format!("the flicker check didn't work ({e}), so every frame is in the video")),
        }
    }
    ctx.check()?;
    let last = frames.last().ok_or_else(|| EngineError::Other("no frames".into()))?.clone();
    let (w, h) = (even(last.x), even(last.y));
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(EngineError::Other(format!("the frames are {w}×{h}, larger than a video can be here ({MAX_SIDE} px)")));
    }
    let p = plan::plan_export(
        frames.len(),
        &PlanOptions { fps: job.fps, length_sec: job.o.length_sec, intro_sec: job.o.intro_sec, flash: job.o.flash, hold_sec: job.o.hold_sec, slow_end: job.o.slow_end },
    );
    let bg = bg_of(&job.o.letterbox);
    let corner = Corner::parse(&job.o.corner).unwrap_or(Corner::BottomRight);
    let final_label = raster::format_work_label(last.w);

    let ffmpeg = job.o.encoder != "frames";
    let mut sink: Box<dyn Sink> = if ffmpeg {
        if let Some(dir) = job.out.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| super::store::io_err(dir, e))?;
        }
        Box::new(Ffmpeg::start(&job.ffmpeg, &job.out, w, h, p.fps)?)
    } else {
        std::fs::create_dir_all(&job.out).map_err(|e| super::store::io_err(&job.out, e))?;
        Box::new(Sequence { dir: job.out.clone(), w, h, last: Vec::new(), written: Vec::new() })
    };

    let mut decoded: HashMap<u64, (usize, usize, Vec<u8>)> = HashMap::new();
    let mut badge: Option<(String, raster::Badge)> = None;
    let mut prev_key: Option<(u64, String, u32)> = None;
    let mut buf: Vec<u8> = Vec::new();
    let result = (|| -> Result<()> {
        for k in 0..p.total {
            ctx.check()?;
            let at = p.at(k);
            let f = frames.get(at.src).ok_or_else(|| EngineError::Other("frame plan out of range".into()))?;
            let label = match at.part {
                _ if !job.o.timer => String::new(),
                Part::Intro => String::new(),
                Part::Main => raster::format_work_label(f.w),
                Part::Flash | Part::Hold => final_label.clone(),
            };
            let key = (f.n, label.clone(), at.white.to_bits());
            let repeat = prev_key.as_ref() == Some(&key);
            if !repeat {
                if !decoded.contains_key(&f.n) {
                    decoded.clear();
                    decoded.insert(f.n, decode_frame(&job.rec.frame_path(f.n))?);
                }
                let (sw, sh, src) = decoded.get(&f.n).ok_or_else(|| EngineError::Other("frame cache".into()))?;
                buf = raster::letterbox(src, *sw, *sh, w as usize, h as usize, bg);
                if !label.is_empty() {
                    if badge.as_ref().is_none_or(|(l, _)| *l != label) {
                        badge = Some((label.clone(), raster::make_badge(&label, h as usize)));
                    }
                    if let Some((_, b)) = &badge {
                        raster::draw_badge(&mut buf, w as usize, h as usize, b, corner);
                    }
                }
                raster::whiten(&mut buf, at.white);
                prev_key = Some(key);
            }
            sink.frame(k, &buf, repeat)?;
            if k % 4 == 0 || k + 1 == p.total {
                let msg = if ffmpeg { "Making the video" } else { "Writing frames" };
                ctx.progress(flicker_share + (1.0 - flicker_share) * 0.98 * (k + 1) as f32 / p.total as f32, msg);
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => sink.finish()?,
        Err(e) => {
            sink.abort();
            return Err(e);
        }
    }
    ctx.progress(1.0, "Done");
    Ok(Exported {
        file: job.out.to_string_lossy().into_owned(),
        mode: if ffmpeg { "ffmpeg".into() } else { "frames".into() },
        video_frames: p.total,
        seconds: p.seconds(),
        width: w,
        height: h,
        used_frames: frames.len(),
        left_out,
        warning,
    })
}
