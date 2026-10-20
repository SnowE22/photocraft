//! Which recorded frame goes into each frame of the exported video: speeding up (or stretching)
//! to the target length, the "finished drawing first" intro, the white flash, the hold at the end
//! and the ending slow-down. Pure logic, the same plan as the Photoshop Timelapse Recorder's.

/// Auto length never makes a video shorter than this (seconds)…
pub const AUTO_MIN_SEC: f64 = 5.0;
/// …or longer than this.
pub const AUTO_MAX_SEC: f64 = 60.0;

/// Ending slow-down: the last quarter of the timelapse eases down to a fifth of full speed.
const SLOW_FROM: f64 = 0.75;
const SLOW_TO: f64 = 0.2;
/// Steps of the slow-down's time-warp table.
const WARP_STEPS: usize = 2000;

/// Export options that shape the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanOptions {
    pub fps: f64,
    /// Length of the main part in seconds; `None` is Auto (every frame once, 5 s – 1 min).
    pub length_sec: Option<f64>,
    /// "Show the finished drawing first" (0 = off).
    pub intro_sec: f64,
    /// White flash before the final image.
    pub flash: bool,
    /// Hold the final image (0 = off).
    pub hold_sec: f64,
    /// Slow down at the end.
    pub slow_end: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self { fps: 30.0, length_sec: None, intro_sec: 0.0, flash: false, hold_sec: 0.0, slow_end: false }
    }
}

/// Which part of the video an output frame belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Intro,
    Main,
    Flash,
    Hold,
}

/// One output frame: the recorded frame it shows (index into the usable frames), its part and how
/// far it is mixed toward white (the flash).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct At {
    pub src: usize,
    pub part: Part,
    pub white: f32,
}

/// The whole plan; [`Plan::at`] says what output frame `k` shows.
#[derive(Clone, Debug)]
pub struct Plan {
    pub fps: f64,
    pub intro: usize,
    pub main: Vec<usize>,
    pub flash: Vec<f32>,
    pub hold: usize,
    pub total: usize,
    last: usize,
}

impl Plan {
    pub fn seconds(&self) -> f64 {
        self.total as f64 / self.fps
    }

    pub fn at(&self, k: usize) -> At {
        let last = self.last;
        if k < self.intro {
            return At { src: last, part: Part::Intro, white: 0.0 };
        }
        let k = k - self.intro;
        if let Some(&src) = self.main.get(k) {
            return At { src, part: Part::Main, white: 0.0 };
        }
        let k = k - self.main.len();
        if let Some(&white) = self.flash.get(k) {
            return At { src: last, part: Part::Flash, white };
        }
        At { src: last, part: Part::Hold, white: 0.0 }
    }
}

/// Upper bound on output frames (10 minutes at 60 fps): a plan never allocates more.
pub const MAX_OUTPUT_FRAMES: usize = 36_000;

/// Auto length: every recorded frame shown once, between [`AUTO_MIN_SEC`] and [`AUTO_MAX_SEC`].
pub fn auto_seconds(frame_count: usize, fps: f64) -> f64 {
    (frame_count as f64 / fps).clamp(AUTO_MIN_SEC, AUTO_MAX_SEC)
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Maps playback time (0..1) to recording progress (0..1) for the ending slow-down.
fn warp_table() -> Vec<f64> {
    let speed = |u: f64| if u < SLOW_FROM { 1.0 } else { 1.0 - (1.0 - SLOW_TO) * smoothstep((u - SLOW_FROM) / (1.0 - SLOW_FROM)) };
    let n = WARP_STEPS as f64;
    let mut table = vec![0.0; WARP_STEPS + 1];
    let mut acc = 0.0;
    for i in 1..=WARP_STEPS {
        acc += (speed((i - 1) as f64 / n) + speed(i as f64 / n)) / (2.0 * n);
        if let Some(t) = table.get_mut(i) {
            *t = acc;
        }
    }
    if acc > 0.0 {
        for t in &mut table {
            *t /= acc;
        }
    }
    table
}

fn warp(table: &[f64], u: f64) -> f64 {
    let x = u.clamp(0.0, 1.0) * WARP_STEPS as f64;
    let i = (x.floor() as usize).min(WARP_STEPS - 1);
    let (a, b) = (table.get(i).copied().unwrap_or(0.0), table.get(i + 1).copied().unwrap_or(1.0));
    a + (b - a) * (x - i as f64)
}

/// White-flash strength per frame: a quick rise to white, then a fade back to the final image.
pub fn flash_curve(fps: f64) -> Vec<f32> {
    let up = ((fps * 0.1).round() as usize).max(2);
    let down = ((fps * 0.5).round() as usize).max(4);
    let mut a = Vec::with_capacity(up + down);
    a.extend((1..=up).map(|i| i as f32 / up as f32));
    a.extend((1..=down).map(|i| 1.0 - i as f32 / down as f32));
    a
}

/// Frames for `sec` seconds (at least one when `sec` is positive), capped.
fn frames_for(sec: f64, fps: f64) -> usize {
    if sec.is_finite() && sec > 0.0 { ((sec * fps).round() as usize).clamp(1, MAX_OUTPUT_FRAMES) } else { 0 }
}

/// Plan an export of `frame_count` recorded frames (at least one is assumed).
pub fn plan_export(frame_count: usize, o: &PlanOptions) -> Plan {
    let fps = if o.fps.is_finite() && o.fps >= 1.0 { o.fps.min(120.0) } else { 30.0 };
    let main_sec = o.length_sec.filter(|s| s.is_finite() && *s > 0.0).unwrap_or_else(|| auto_seconds(frame_count, fps));
    let main_count = frames_for(main_sec, fps).max(1);
    let last = frame_count.saturating_sub(1);
    let table = o.slow_end.then(warp_table);
    let main = (0..main_count)
        .map(|j| {
            let u = if main_count == 1 { 1.0 } else { j as f64 / (main_count - 1) as f64 };
            let w = table.as_deref().map_or(u, |t| warp(t, u));
            ((w * last as f64).round() as usize).min(last)
        })
        .collect::<Vec<_>>();
    let intro = frames_for(o.intro_sec, fps);
    let flash = if o.flash { flash_curve(fps) } else { Vec::new() };
    let hold = frames_for(o.hold_sec, fps);
    let total = intro + main.len() + flash.len() + hold;
    Plan { fps, intro, main, flash, hold, total, last }
}
