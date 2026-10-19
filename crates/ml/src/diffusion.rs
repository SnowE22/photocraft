//! Stable Diffusion inpainting (Rombach et al., *High-Resolution Image Synthesis with Latent
//! Diffusion Models*, CVPR 2022; SDXL: Podell et al., *SDXL: Improving Latent Diffusion Models
//! for High-Resolution Image Synthesis*, 2023) on ONNX Runtime: the Remove tool's "Generative AI
//! (Diffusion)" fill and Edit › Generative Fill. Slower than LaMa (seconds per fill on a GPU), but
//! it draws what was most likely behind: a face half under a speech bubble gets its other half,
//! and it extends a picture past its edge (outpainting).
//!
//! Two kinds of model, both *inpainting* checkpoints (a 9-channel UNet: noisy latent, mask,
//! masked-image latent) in a folder with a `model.json`:
//!
//! - `"kind": "sdxl-inpaint"` (`packaging/models/export_sdxl_inpaint.py`): Stable Diffusion XL,
//!   1024-pixel crops, two text encoders in one `text_encoder.onnx` (hidden states and a pooled
//!   embedding), a UNet that also takes the pooled text and SDXL's six size numbers. The GPU takes
//!   `unet8.onnx` (int8 weights, about 5.2 GB of graphics memory at 1024) or `unet16.onnx`; the
//!   CPU `unet.onnx` when exported. The VAE (`vae_*.onnx`, fp32) runs on the CPU, so it fits an
//!   8 GB GPU. Looked for first, in [`XL_FOLDER`].
//! - `"kind": "sd-inpaint"` (`packaging/models/export_sd_inpaint.py`): Stable Diffusion 1.x, 512
//!   pixels, `text_encoder.onnx`, `unet.onnx` (fp32) and optionally `unet16.onnx`, the VAE. Anything
//!   4.0 inpainting ([`DEFAULT_FOLDER`]) is trained on anime and manga art.
//!
//! Both have `vae_encoder.onnx`, `vae_decoder.onnx`, and CLIP's `vocab.json` and `merges.txt`. A
//! run whose GPU fails while running the model (a driver reset) goes on on the CPU.
//!
//! One crop is filled in [`run`]: the prompt and the negative prompt encoded once (classifier-free
//! guidance), the masked crop encoded by the VAE, then DDIM steps (Song et al. 2021) from noise,
//! or with a `hint` (the crop's current fill, [`crate::inpaint`]'s full-resolution pass) from that
//! hint noised to `strength`, so the pass keeps the shapes and adds detail. Only the latent around
//! the hole is decoded ([`decode_window`]): the rest of the crop isn't used.

use std::path::PathBuf;

use crate::MlError;

/// The model's name in messages.
pub const NAME: &str = "Stable Diffusion inpainting";

/// The SDXL model's folder, looked for first.
pub const XL_FOLDER: &str = "sdxl-inpainting";

/// The SD 1.x model's folder, looked for next; then any folder whose `model.json` says
/// `sd-inpaint` or `sdxl-inpaint`.
pub const DEFAULT_FOLDER: &str = "anything-4.0-inpainting";

/// What the model is doing, for the UI (as [`crate::lama::State`]).
pub use crate::lama::State;

/// How a fill is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// What should be there; empty: [`Settings::prompts_for`]'s default for the crop.
    pub prompt: String,
    /// What shouldn't; empty: the default.
    pub negative: String,
    /// Denoising steps from noise (fewer with a hint, in proportion to `strength`).
    pub steps: u32,
    /// Classifier-free guidance scale.
    pub guidance: f32,
    pub seed: u64,
    /// How much a hinted pass may change its hint (0–1).
    pub strength: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { prompt: String::new(), negative: String::new(), steps: 25, guidance: 7.0, seed: 0x5d_1f11, strength: 0.45 }
    }
}

/// What a model was trained on, which picks the default prompts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Anime and manga art (Anything 4.0).
    Anime,
    /// Photographs and art of any kind (SDXL).
    General,
}

/// Prompts that work for filling in manga and anime art.
const PROMPT: &str = "masterpiece, best quality, highly detailed, clean";
const PROMPT_MONO: &str = "monochrome, greyscale, manga, screentone, lineart, ink";
const PROMPT_COLOUR: &str = "anime illustration, detailed background";
const NEGATIVE: &str = "text, speech bubble, letters, words, caption, watermark, signature, logo, frame, border, lowres, blurry, jpeg artifacts, bad anatomy, worst quality, low quality";
/// And for anything else.
const PROMPT_GENERAL: &str = "high quality, highly detailed, sharp focus, natural, seamless";
const PROMPT_GENERAL_MONO: &str = "black and white, monochrome";
const NEGATIVE_GENERAL: &str = "text, letters, words, caption, watermark, signature, logo, frame, border, lowres, blurry, jpeg artifacts, deformed, distorted, disfigured, worst quality, low quality";

impl Settings {
    /// The prompt and negative prompt for a crop: the user's, or the defaults for the model's
    /// style (with monochrome terms for a black-and-white crop).
    pub fn prompts_for(&self, monochrome: bool, style: Style) -> (String, String) {
        let (base, mono, colour, neg) = match style {
            Style::Anime => (PROMPT, PROMPT_MONO, Some(PROMPT_COLOUR), NEGATIVE),
            Style::General => (PROMPT_GENERAL, PROMPT_GENERAL_MONO, None, NEGATIVE_GENERAL),
        };
        let user = self.prompt.trim();
        let mut prompt = if user.is_empty() { base.to_string() } else { user.to_string() };
        if let Some(extra) = if monochrome { Some(mono) } else { colour } {
            prompt = format!("{prompt}, {extra}");
        }
        let negative = if self.negative.trim().is_empty() {
            if monochrome { format!("{neg}, color, colorful") } else { neg.to_string() }
        } else {
            self.negative.trim().to_string()
        };
        (prompt, negative)
    }
}

/// The model's settings from its `model.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    /// Stable Diffusion XL (`sdxl-inpaint`), else 1.x.
    pub xl: bool,
    /// The square crop the model works on, in pixels (512 for SD 1.x, 1024 for SDXL).
    pub side: usize,
    pub in_channels: usize,
    pub hidden: usize,
    /// The pooled text embedding's size (SDXL; 0 for SD 1.x).
    pub pooled: usize,
    pub style: Style,
    pub scaling: f32,
    pub beta_start: f64,
    pub beta_end: f64,
    pub train_steps: usize,
    pub steps_offset: usize,
}

impl Meta {
    pub fn parse(json: &str) -> Result<Self, MlError> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| MlError::Load(format!("model.json: {e}")))?;
        let xl = match v.get("kind").and_then(|k| k.as_str()) {
            Some("sd-inpaint") => false,
            Some("sdxl-inpaint") => true,
            _ => return Err(MlError::Load("model.json isn't an sd-inpaint or sdxl-inpaint model".into())),
        };
        if v.get("prediction_type").and_then(|k| k.as_str()).is_some_and(|p| p != "epsilon") {
            return Err(MlError::Load("only epsilon-prediction models are supported".into()));
        }
        if v.get("beta_schedule").and_then(|k| k.as_str()).is_some_and(|p| p != "scaled_linear") {
            return Err(MlError::Load("only the scaled_linear noise schedule is supported".into()));
        }
        let num = |k: &str, d: f64| v.get(k).and_then(serde_json::Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).unwrap_or(d);
        let style = match v.get("style").and_then(|k| k.as_str()) {
            Some("anime") => Style::Anime,
            Some("general") => Style::General,
            _ if xl => Style::General,
            _ => Style::Anime,
        };
        let meta = Self {
            xl,
            side: num("side", if xl { 1024.0 } else { 512.0 }) as usize,
            in_channels: num("in_channels", 9.0) as usize,
            hidden: num("hidden_size", if xl { 2048.0 } else { 768.0 }) as usize,
            pooled: if xl { num("pooled_size", 1280.0) as usize } else { 0 },
            style,
            scaling: num("vae_scaling_factor", if xl { 0.13025 } else { 0.18215 }) as f32,
            beta_start: num("beta_start", 0.00085),
            beta_end: num("beta_end", 0.012),
            train_steps: (num("num_train_timesteps", 1000.0) as usize).clamp(10, 10_000),
            steps_offset: num("steps_offset", 1.0) as usize,
        };
        if meta.in_channels != 9 {
            return Err(MlError::Load(format!("not an inpainting UNet ({} input channels, not 9)", meta.in_channels)));
        }
        if !(1..=4096).contains(&meta.hidden) || meta.scaling <= 0.0 || (xl && !(1..=4096).contains(&meta.pooled)) {
            return Err(MlError::Load("model.json has impossible sizes".into()));
        }
        if !(256..=2048).contains(&meta.side) || !meta.side.is_multiple_of(64) {
            return Err(MlError::Load(format!("model.json's side ({}) isn't a multiple of 64 from 256 to 2048", meta.side)));
        }
        Ok(meta)
    }
}

/// The model folder, or why there is none: `PHOTOCRAFT_DIFFUSION_MODEL` (a folder, or a folder's
/// name in a search folder) when set, else [`XL_FOLDER`], then [`DEFAULT_FOLDER`] in a search
/// folder, else the first subfolder of one with an `sd-inpaint` or `sdxl-inpaint` `model.json`.
pub fn model_dir() -> Result<PathBuf, MlError> {
    let dirs = crate::search_dirs();
    let ok = |d: &PathBuf| d.join("model.json").is_file() && ["unet.onnx", "unet16.onnx", "unet8.onnx"].iter().any(|u| d.join(u).is_file());
    if let Some(chosen) = std::env::var_os("PHOTOCRAFT_DIFFUSION_MODEL").filter(|v| !v.is_empty()) {
        let chosen = PathBuf::from(chosen);
        return std::iter::once(chosen.clone()).chain(dirs.iter().map(|d| d.join(&chosen))).find(|p| ok(p)).ok_or_else(|| MlError::Load(format!("PHOTOCRAFT_DIFFUSION_MODEL: no model in {} (looked in {})", chosen.display(), crate::list(&dirs))));
    }
    for name in [XL_FOLDER, DEFAULT_FOLDER] {
        for d in &dirs {
            let p = d.join(name);
            if ok(&p) {
                return Ok(p);
            }
        }
    }
    for d in &dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if ok(&p) && std::fs::read_to_string(p.join("model.json")).is_ok_and(|j| Meta::parse(&j).is_ok()) {
                return Ok(p);
            }
        }
    }
    Err(MlError::ModelMissing { model: NAME, file: "sdxl-inpainting/model.json", searched: crate::list(&dirs) })
}

/// The installed model's settings, without loading it.
pub fn installed_meta() -> Result<Meta, MlError> {
    let dir = model_dir()?;
    let json = std::fs::read_to_string(dir.join("model.json")).map_err(|e| MlError::Load(format!("{}: {e}", dir.join("model.json").display())))?;
    Meta::parse(&json)
}

/// DDIM's cumulative alphas (`scaled_linear` betas) and the timesteps of an `n`-step run
/// ("leading" spacing with the model's offset), highest first.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn schedule(meta: &Meta, n: usize) -> (Vec<f64>, Vec<usize>) {
    let t = meta.train_steps;
    let (a, b) = (meta.beta_start.sqrt(), meta.beta_end.sqrt());
    let mut cum = Vec::with_capacity(t);
    let mut prod = 1.0f64;
    for i in 0..t {
        let s = a + (b - a) * i as f64 / (t - 1).max(1) as f64;
        prod *= 1.0 - s * s;
        cum.push(prod);
    }
    let n = n.clamp(1, t);
    let ratio = t / n;
    let mut steps: Vec<usize> = (0..n).map(|i| (i * ratio + meta.steps_offset).min(t - 1)).collect();
    steps.reverse();
    (cum, steps)
}

/// A seeded source of standard normal samples (SplitMix64 and Box–Muller): the same seed gives
/// the same fill.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) struct Noise(u64);

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl Noise {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f64 {
        ((self.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    pub(crate) fn gaussian(&mut self, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n + 1);
        while out.len() < n {
            let (u, v) = (self.uniform(), self.uniform());
            let r = (-2.0 * u.ln()).sqrt();
            let a = std::f64::consts::TAU * v;
            out.push((r * a.cos()) as f32);
            out.push((r * a.sin()) as f32);
        }
        out.truncate(n);
        out
    }
}

/// The networks one fill needs (what [`run`] calls), so the sampling can be tested without them.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) trait Nets {
    /// Token ids (`count × 77`) to hidden states (`count × 77 × hidden`) and, for SDXL, pooled
    /// embeddings (`count × pooled`; empty for SD 1.x).
    fn text(&mut self, ids: &[i64], count: usize) -> Result<(Vec<f32>, Vec<f32>), MlError>;
    /// An image (`3 × side × side`, −1 to 1) to its latent mean (`4 × side/8 × side/8`), unscaled.
    fn encode(&mut self, image: &[f32], side: usize) -> Result<Vec<f32>, MlError>;
    /// Batch 2 (unconditional, conditional) of 9-channel latents at timestep `t`: the predicted
    /// noise. `pooled` is [`Nets::text`]'s (SDXL), which also gets the crop's size as its size
    /// conditioning.
    fn unet(&mut self, sample: &[f32], t: f32, hidden: &[f32], pooled: &[f32], latent_side: usize) -> Result<Vec<f32>, MlError>;
    /// A latent (`4 × height/8 × width/8`), unscaled, to an image (`3 × height × width`, −1 to 1).
    fn decode(&mut self, latent: &[f32], width: usize, height: usize) -> Result<Vec<f32>, MlError>;
    /// Token ids of a prompt.
    fn tokens(&self, text: &str) -> Vec<i64>;
}

/// One crop: planar RGB (0–1, `side × side`, `side` a multiple of 64), the mask (1 = fill), and
/// optionally a hint for the masked pixels (planar RGB); the fill as planar RGB (0–1).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    nets: &mut dyn Nets,
    meta: &Meta,
    s: &Settings,
    rgb: &[f32],
    mask: &[f32],
    hint: Option<&[f32]>,
    side: usize,
    ctl: &dyn Fn(f32) -> bool,
) -> Result<Vec<f32>, MlError> {
    let n = side * side;
    if side == 0 || !side.is_multiple_of(64) || rgb.len() != 3 * n || mask.len() != n || hint.is_some_and(|h| h.len() != 3 * n) {
        return Err(MlError::BadInput(format!("{NAME} takes a square crop a multiple of 64 pixels wide")));
    }
    let ls = side / 8;
    let ln = ls * ls;
    // Prompts: is the crop's known part black and white?
    let known: Vec<usize> = (0..n).filter(|p| mask.get(*p).is_some_and(|m| *m <= 0.0)).collect();
    let grey = known
        .iter()
        .filter(|p| {
            let (r, g, b) = (rgb[**p], rgb[n + **p], rgb[2 * n + **p]);
            (r - g).abs() < 0.02 && (g - b).abs() < 0.02 && (r - b).abs() < 0.02
        })
        .count();
    let mono = !known.is_empty() && grey as f64 >= known.len() as f64 * 0.98;
    let (prompt, negative) = s.prompts_for(mono, meta.style);
    let mut ids = nets.tokens(&negative);
    ids.extend(nets.tokens(&prompt));
    let (hidden, pooled) = nets.text(&ids, 2)?;
    if hidden.len() != 2 * crate::clip_tokenizer::LENGTH * meta.hidden || pooled.len() != 2 * meta.pooled {
        return Err(MlError::Inference("the text encoder gave the wrong number of values (does model.json match the model?)".into()));
    }
    // The masked crop's latent, and the mask at latent size (any masked pixel masks its cell).
    let to_signed = |v: f32| v.clamp(0.0, 1.0) * 2.0 - 1.0;
    let masked_image: Vec<f32> = (0..3 * n).map(|i| if mask.get(i % n).is_some_and(|m| *m > 0.0) { 0.0 } else { to_signed(rgb[i]) }).collect();
    let masked_latent: Vec<f32> = nets.encode(&masked_image, side)?.into_iter().map(|v| v * meta.scaling).collect();
    if masked_latent.len() != 4 * ln {
        return Err(MlError::Inference("the VAE gave a latent of the wrong size".into()));
    }
    let mut small = vec![0f32; ln];
    for y in 0..side {
        for x in 0..side {
            if mask[y * side + x] > 0.0 {
                small[(y / 8) * ls + x / 8] = 1.0;
            }
        }
    }
    let (cum, mut steps) = schedule(meta, s.steps.clamp(1, 150) as usize);
    let mut noise = Noise::new(s.seed);
    let eps0 = noise.gaussian(4 * ln);
    // Start: pure noise, or the hint's latent noised to the first step kept.
    let mut lat = match hint {
        None => eps0,
        Some(h) => {
            let strength = s.strength.clamp(0.05, 1.0);
            let keep = ((steps.len() as f32 * strength).round() as usize).clamp(1, steps.len());
            steps = steps.split_off(steps.len() - keep);
            let image: Vec<f32> = (0..3 * n).map(|i| if mask.get(i % n).is_some_and(|m| *m > 0.0) { to_signed(h[i]) } else { to_signed(rgb[i]) }).collect();
            let x0: Vec<f32> = nets.encode(&image, side)?.into_iter().map(|v| v * meta.scaling).collect();
            let a = cum.get(steps.first().copied().unwrap_or(0)).copied().unwrap_or(1.0);
            x0.iter().zip(&eps0).map(|(x, e)| (a.sqrt() as f32) * x + ((1.0 - a).sqrt() as f32) * e).collect()
        }
    };
    if lat.len() != 4 * ln {
        return Err(MlError::Inference("the VAE gave a latent of the wrong size".into()));
    }
    let ratio = meta.train_steps / s.steps.clamp(1, 150) as usize;
    let final_alpha = cum.first().copied().unwrap_or(1.0);
    for (i, &t) in steps.iter().enumerate() {
        if ctl(i as f32 / steps.len() as f32) {
            return Err(MlError::Cancelled);
        }
        // Batch 2: [latent, mask, masked latent] twice.
        let mut sample = Vec::with_capacity(2 * 9 * ln);
        for _ in 0..2 {
            sample.extend_from_slice(&lat);
            sample.extend_from_slice(&small);
            sample.extend_from_slice(&masked_latent);
        }
        let pred = nets.unet(&sample, t as f32, &hidden, &pooled, ls)?;
        if pred.len() != 2 * 4 * ln {
            return Err(MlError::Inference("the UNet gave the wrong number of values".into()));
        }
        let (uncond, cond) = pred.split_at(4 * ln);
        let a_t = cum.get(t).copied().unwrap_or(1.0);
        let a_prev = t.checked_sub(ratio).and_then(|p| cum.get(p).copied()).unwrap_or(final_alpha);
        for (j, l) in lat.iter_mut().enumerate() {
            let e = f64::from(uncond[j] + s.guidance * (cond[j] - uncond[j]));
            let x = f64::from(*l);
            let x0 = (x - (1.0 - a_t).sqrt() * e) / a_t.sqrt();
            *l = (a_prev.sqrt() * x0 + (1.0 - a_prev).sqrt() * e) as f32;
        }
        if lat.iter().any(|v| !v.is_finite()) {
            return Err(MlError::Inference("the sampling diverged (not a number)".into()));
        }
    }
    let unscaled: Vec<f32> = lat.iter().map(|v| v / meta.scaling).collect();
    let wrong = || MlError::Inference("the VAE decoder gave the wrong number of values".into());
    // Only the hole and the pixels just around it are used: decode the latent around the hole and
    // keep the crop elsewhere (SDXL's CPU decoder costs about 27 s for a whole 1024 crop).
    let image = match decode_window(&small, ls, DECODE_MARGIN) {
        Some(w) => {
            let part = nets.decode(&crop_latent(&unscaled, ls, w).ok_or_else(wrong)?, w.w * 8, w.h * 8)?;
            let mut image: Vec<f32> = rgb.iter().map(|v| to_signed(*v)).collect();
            paste_decoded(&mut image, side, &part, w).ok_or_else(wrong)?;
            image
        }
        None => nets.decode(&unscaled, side, side)?,
    };
    if image.len() != 3 * n {
        return Err(wrong());
    }
    Ok(image.into_iter().map(|v| if v.is_finite() { ((v + 1.0) / 2.0).clamp(0.0, 1.0) } else { 0.0 }).collect())
}

/// Latent cells decoded around the hole's when only part of the latent is decoded: the decoder
/// is off for about 4 cells in from a window's edge, and a wider window keeps its normalisation
/// (statistics over the whole window) close to a whole decode's.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const DECODE_MARGIN: usize = 16;

/// A window covering at least this share of the latent is decoded whole.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const DECODE_WHOLE_SHARE: f64 = 0.85;

/// A rectangle of latent cells.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// The latent cells to decode for a latent mask (`ls × ls`, > 0 = hole): the hole's bounding box
/// grown by `margin` cells and cut to the latent. `None` (decode it all) when nothing is masked,
/// the mask is the wrong size, or the window is most of the latent anyway.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn decode_window(small: &[f32], ls: usize, margin: usize) -> Option<Window> {
    if ls == 0 || ls.checked_mul(ls) != Some(small.len()) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, _) in small.iter().enumerate().filter(|(_, m)| **m > 0.0) {
        let (x, y) = (i % ls, i / ls);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
    }
    if x0 == usize::MAX {
        return None;
    }
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = (x1.saturating_add(margin).min(ls), y1.saturating_add(margin).min(ls));
    let w = Window { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
    (((w.w * w.h) as f64) < DECODE_WHOLE_SHARE * (ls * ls) as f64).then_some(w)
}

/// `w`'s cells of a 4-channel latent `ls` square; `None` when `w` doesn't fit.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn crop_latent(latent: &[f32], ls: usize, w: Window) -> Option<Vec<f32>> {
    let ln = ls.checked_mul(ls)?;
    if latent.len() != 4 * ln || w.x.checked_add(w.w)? > ls || w.y.checked_add(w.h)? > ls {
        return None;
    }
    let mut out = Vec::with_capacity(4 * w.w * w.h);
    for c in 0..4 {
        for y in w.y..w.y + w.h {
            let o = c * ln + y * ls + w.x;
            out.extend_from_slice(latent.get(o..o + w.w)?);
        }
    }
    Some(out)
}

/// Put a decoded window (`3 × 8·h × 8·w`) into a decoded crop (`3 × side × side`); `None` when
/// the sizes don't match.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn paste_decoded(image: &mut [f32], side: usize, part: &[f32], w: Window) -> Option<()> {
    let n = side.checked_mul(side)?;
    let (px, py, pw, ph) = (w.x.checked_mul(8)?, w.y.checked_mul(8)?, w.w.checked_mul(8)?, w.h.checked_mul(8)?);
    if image.len() != 3 * n || part.len() != 3 * pw * ph || px + pw > side || py + ph > side {
        return None;
    }
    for c in 0..3 {
        for y in 0..ph {
            let src = part.get((c * ph + y) * pw..(c * ph + y + 1) * pw)?;
            let o = c * n + (py + y) * side + px;
            image.get_mut(o..o + pw)?.copy_from_slice(src);
        }
    }
    Some(())
}

/// How [`crate::inpaint`] crops for the installed model: [`crate::inpaint::DIFFUSION`] at the
/// model's side (512 for SD 1.x, 1024 for SDXL).
pub fn options() -> crate::inpaint::Options {
    let side = installed_meta().map_or(crate::inpaint::DIFFUSION.side, |m| m.side);
    crate::inpaint::Options { side, ..crate::inpaint::DIFFUSION }
}

/// Fill the request's hole with the diffusion model ([`options`]: a crop at the model's side
/// around each part of the hole, then, where that crop was shrunk, a hinted pass at full
/// resolution). Loads the model first if it isn't loaded (tens of seconds, once). Each model run
/// gets its own seed from `settings.seed`.
pub fn fill(
    req: &crate::inpaint::Request,
    read: &mut dyn FnMut(photocraft_geom::Rect) -> Vec<f32>,
    settings: &Settings,
    ctl: &photocraft_raster::Interrupt,
) -> Result<(Vec<f32>, crate::Device), MlError> {
    let mut device = crate::Device::Cpu;
    let mut run = 0u64;
    let out = crate::inpaint::fill_with(
        req,
        read,
        &mut |rgb, mask, hint| {
            let side = (mask.len() as f64).sqrt().round() as usize;
            let s = Settings { seed: settings.seed.wrapping_add(run.wrapping_mul(0x9e37_79b9)), ..settings.clone() };
            run += 1;
            let (o, d) = inpaint(rgb, mask, hint, side, &s, &|_| ctl.check().is_err())?;
            device = d;
            Ok(o)
        },
        ctl,
        &options(),
    )?;
    Ok((out, device))
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{availability, gpu_fallback, inpaint, preload, state};

#[cfg(target_arch = "wasm32")]
pub use web::{availability, gpu_fallback, inpaint, preload, state};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Mutex, PoisonError};

    use ort::session::Session;
    use ort::value::Tensor;

    use super::{Meta, NAME, Nets, Settings, State};
    use crate::clip_tokenizer::{LENGTH, Tokenizer};
    use crate::{Device, MlError, runtime};

    /// A session and the names of its outputs (the first is the one [`run_net`] reads).
    struct Net {
        session: Session,
        output: String,
        outputs: Vec<String>,
    }

    struct Loaded {
        text: Net,
        unet: Net,
        enc: Net,
        dec: Net,
        tokenizer: Tokenizer,
        meta: Meta,
        device: Device,
    }

    static SLOT: Mutex<Option<Result<Loaded, MlError>>> = Mutex::new(None);
    static PHASE: AtomicU8 = AtomicU8::new(IDLE);
    static FAILURE: Mutex<Option<MlError>> = Mutex::new(None);
    /// Why the GPU wasn't used, when the model fell back to the CPU.
    static GPU_FAILURE: Mutex<Option<String>> = Mutex::new(None);
    /// The GPU failed while running the model (a driver reset): the CPU from then on, this run.
    static GPU_BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    /// Its last use; unloaded after [`IDLE_AFTER`] without one (`crate::idle`).
    static KEEP: crate::idle::Keeper = crate::idle::Keeper::new();
    /// About 4 GB (SD 1.x) or 6 GB (SDXL) of an 8 GB GPU while loaded: dropped a minute after the
    /// last fill.
    const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);
    const IDLE: u8 = 0;
    const LOADING: u8 = 1;
    const READY_GPU: u8 = 2;
    const READY_CPU: u8 = 3;
    const FAILED: u8 = 4;

    pub fn state() -> State {
        match PHASE.load(Ordering::Acquire) {
            LOADING => State::Loading,
            READY_GPU => State::Ready(Device::DirectMl),
            READY_CPU => State::Ready(Device::Cpu),
            FAILED => State::Failed(FAILURE.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(ToString::to_string).unwrap_or_default()),
            _ if availability().is_ok() => State::Idle,
            _ => State::Missing,
        }
    }

    /// Why the model runs on the CPU although DirectML was tried (`None`: it wasn't, or it worked).
    pub fn gpu_fallback() -> Option<String> {
        GPU_FAILURE.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// `Ok` when the model is loaded, loading or installed, else why not.
    pub fn availability() -> Result<(), MlError> {
        match PHASE.load(Ordering::Acquire) {
            LOADING | READY_GPU | READY_CPU => Ok(()),
            FAILED => Err(FAILURE.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| MlError::Load("unknown error".into()))),
            _ => {
                super::model_dir()?;
                runtime::library(&crate::search_dirs())?;
                Ok(())
            }
        }
    }

    /// Load in the background (tens of seconds the first time), when installed and not started.
    pub fn preload() {
        // Unloaded for being idle: not again until it is used.
        if PHASE.load(Ordering::Acquire) != IDLE || KEEP.released() || availability().is_err() {
            return;
        }
        let started = std::thread::Builder::new().name("diffusion-load".into()).spawn(|| {
            let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
            if let Err(e) = ensure(&mut slot) {
                log::info!("[ml] {NAME} didn't load: {e}");
            }
        });
        if let Err(e) = started {
            log::warn!("[ml] couldn't start loading {NAME}: {e}");
        }
    }

    /// Fill one crop (see [`super::run`]); `side × side`, a multiple of 64. Loads the model first
    /// if needed. `cancelled(progress)` is asked before each step.
    pub fn inpaint(
        rgb: &[f32],
        mask: &[f32],
        hint: Option<&[f32]>,
        side: usize,
        settings: &Settings,
        cancelled: &dyn Fn(f32) -> bool,
    ) -> Result<(Vec<f32>, Device), MlError> {
        let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
        KEEP.touch();
        ensure(&mut slot)?;
        let mut out = run_loaded(&mut slot, rgb, mask, hint, side, settings, cancelled);
        if let Err(MlError::Inference(e)) = &out {
            let on_gpu = matches!(slot.as_ref(), Some(Ok(l)) if l.device == Device::DirectMl);
            // A lost device or a broken session: load again (on the GPU's failure, on the CPU).
            *slot = None;
            PHASE.store(IDLE, Ordering::Release);
            // A GPU-only model (no fp32 UNet) has nowhere to go: the sessions are dropped (their
            // memory freed) and the next fill tries the GPU again.
            let cpu_possible = super::model_dir().is_ok_and(|d| d.join("unet.onnx").is_file());
            if on_gpu && !cpu_possible {
                log::warn!("[ml] {NAME} failed on DirectML ({e}); it has no CPU version");
                return Err(MlError::Inference(format!(
                    "{e} (the model needs more free graphics memory: close other programs that use the GPU, or export it with --int8)"
                )));
            }
            if on_gpu {
                log::warn!("[ml] {NAME} failed on DirectML ({e}); using the CPU from now on");
                *GPU_FAILURE.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
                GPU_BROKEN.store(true, Ordering::Release);
                ensure(&mut slot)?;
                out = run_loaded(&mut slot, rgb, mask, hint, side, settings, cancelled);
            }
        }
        KEEP.touch();
        out
    }

    /// Drop the sessions when idle for [`IDLE_AFTER`] (see `crate::idle`).
    fn release_idle() -> crate::idle::Check {
        let Ok(mut slot) = SLOT.try_lock() else { return crate::idle::Check::Kept };
        if !matches!(*slot, Some(Ok(_))) {
            return crate::idle::Check::Gone;
        }
        if KEEP.idle_for() < IDLE_AFTER {
            return crate::idle::Check::Kept;
        }
        *slot = None;
        PHASE.store(IDLE, Ordering::Release);
        crate::idle::Check::Released
    }

    fn run_loaded(
        slot: &mut Option<Result<Loaded, MlError>>,
        rgb: &[f32],
        mask: &[f32],
        hint: Option<&[f32]>,
        side: usize,
        settings: &Settings,
        cancelled: &dyn Fn(f32) -> bool,
    ) -> Result<(Vec<f32>, Device), MlError> {
        match slot.as_mut() {
            Some(Ok(l)) => {
                let meta = l.meta.clone();
                let device = l.device;
                super::run(l, &meta, settings, rgb, mask, hint, side, cancelled).map(|v| (v, device))
            }
            Some(Err(e)) => Err(e.clone()),
            None => Err(MlError::Load("no session".into())),
        }
    }

    fn ensure(slot: &mut Option<Result<Loaded, MlError>>) -> Result<(), MlError> {
        if matches!(slot, Some(Err(MlError::ModelMissing { .. } | MlError::RuntimeMissing { .. }))) {
            *slot = None;
        }
        if slot.is_none() {
            PHASE.store(LOADING, Ordering::Release);
            let loaded = load();
            match &loaded {
                Ok(l) => PHASE.store(if l.device == Device::DirectMl { READY_GPU } else { READY_CPU }, Ordering::Release),
                Err(e @ (MlError::ModelMissing { .. } | MlError::RuntimeMissing { .. })) => {
                    log::info!("[ml] {e}");
                    PHASE.store(IDLE, Ordering::Release);
                }
                Err(e) => {
                    *FAILURE.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
                    PHASE.store(FAILED, Ordering::Release);
                }
            }
            let ok = loaded.is_ok();
            *slot = Some(loaded);
            if ok {
                KEEP.touch();
                KEEP.watch("diffusion", release_idle);
            }
        }
        match slot {
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(e.clone()),
            None => Err(MlError::Load("no session".into())),
        }
    }

    fn load() -> Result<Loaded, MlError> {
        let dirs = crate::search_dirs();
        let library = runtime::library(&dirs)?;
        let dir = super::model_dir()?;
        let env = runtime::environment(&library)?;
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map_err(|e| MlError::Load(format!("{}: {e}", dir.join(f).display())));
        let meta = Meta::parse(&read("model.json")?)?;
        let tokenizer = Tokenizer::new(&read("vocab.json")?, &read("merges.txt")?)?;
        let started = std::time::Instant::now();
        #[cfg(windows)]
        if !crate::cpu_only() && !GPU_BROKEN.load(Ordering::Acquire) {
            match sessions(&env, &dir, Device::DirectMl, meta.xl) {
                Ok((text, unet, enc, dec)) => {
                    log::info!("[ml] {NAME} on DirectML in {:.1} s ({})", started.elapsed().as_secs_f32(), dir.display());
                    return Ok(Loaded { text, unet, enc, dec, tokenizer, meta, device: Device::DirectMl });
                }
                Err(e) => {
                    log::warn!("[ml] {NAME} on DirectML failed ({e}); using the CPU");
                    *GPU_FAILURE.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.to_string());
                }
            }
        }
        let (text, unet, enc, dec) = sessions(&env, &dir, Device::Cpu, meta.xl)?;
        log::info!("[ml] {NAME} on the CPU in {:.1} s ({})", started.elapsed().as_secs_f32(), dir.display());
        Ok(Loaded { text, unet, enc, dec, tokenizer, meta, device: Device::Cpu })
    }

    fn sessions(env: &ort::environment::Environment, dir: &std::path::Path, device: Device, xl: bool) -> Result<(Net, Net, Net, Net), MlError> {
        let open_on = |file: &std::path::Path, device: Device| -> Result<Net, MlError> {
            let fail = |e: &dyn std::fmt::Display| MlError::Load(format!("{} on {}: {e}", file.display(), device.name()));
            if !file.is_file() {
                return Err(MlError::Load(format!("{} is missing", file.display())));
            }
            let builder = Session::builder(env).map_err(|e| fail(&e))?;
            let mut builder = match device {
                Device::DirectMl => directml(builder).map_err(|e| fail(&e))?,
                Device::Cpu => builder.with_execution_providers([ort::ep::CPU::default().build().error_on_failure()]).map_err(|e| fail(&e))?,
            };
            let session = builder.commit_from_file(file).map_err(|e| fail(&e))?;
            let outputs: Vec<String> = session.outputs().iter().map(|o| o.name().to_string()).collect();
            let output = outputs.first().cloned().ok_or_else(|| MlError::Load(format!("{} has no output", file.display())))?;
            Ok(Net { session, output, outputs })
        };
        // The GPU takes the fp16 networks when there are (exported in half precision, several
        // times faster and half the memory); the CPU, whose provider has no fp16 kernels for them,
        // the fp32 ones.
        let pick = |half: &str, full: &str| {
            let h = dir.join(half);
            if device == Device::DirectMl && h.is_file() { h } else { dir.join(full) }
        };
        // int8 weights first (SDXL on an 8 GB GPU: 2.6 GB instead of 5.1), then fp16.
        let unet = if device == Device::DirectMl && dir.join("unet8.onnx").is_file() { dir.join("unet8.onnx") } else { pick("unet16.onnx", "unet.onnx") };
        if !unet.is_file() && (dir.join("unet16.onnx").is_file() || dir.join("unet8.onnx").is_file()) {
            return Err(MlError::Load(format!("{} runs on the GPU only (export it with --cpu for unet.onnx)", dir.display())));
        }
        let open = |file: &std::path::Path| open_on(file, device);
        // The text encoders run twice a fill: on the CPU, their 0.5 GB (SDXL: 3 GB) stay off the GPU.
        let text = open_on(&dir.join("text_encoder.onnx"), Device::Cpu)?;
        let unet = open(&unet)?;
        // SDXL's VAE at 1024 holds about 3.8 GB of graphics memory on DirectML, next to the UNet's
        // 5 GB: more than an 8 GB GPU has, and the paging that follows makes a fill take many
        // minutes. On the CPU (fp32) it costs about 40 s a fill and no graphics memory.
        // `PHOTOCRAFT_SDXL_VAE=gpu` keeps it on a GPU with room for it.
        let vae_gpu = !xl || std::env::var("PHOTOCRAFT_SDXL_VAE").is_ok_and(|v| v.eq_ignore_ascii_case("gpu"));
        let (enc, dec) = if vae_gpu {
            (open(&pick("vae16_encoder.onnx", "vae_encoder.onnx"))?, open(&pick("vae16_decoder.onnx", "vae_decoder.onnx"))?)
        } else {
            (open_on(&dir.join("vae_encoder.onnx"), Device::Cpu)?, open_on(&dir.join("vae_decoder.onnx"), Device::Cpu)?)
        };
        Ok((text, unet, enc, dec))
    }

    #[cfg(windows)]
    fn directml(builder: ort::session::builder::SessionBuilder) -> Result<ort::session::builder::SessionBuilder, String> {
        use ort::ep::directml::PerformancePreference;
        let ep = ort::ep::DirectML::default().with_performance_preference(PerformancePreference::HighPerformance);
        let builder = builder.with_memory_pattern(false).map_err(|e| e.to_string())?;
        let builder = builder.with_parallel_execution(false).map_err(|e| e.to_string())?;
        builder.with_execution_providers([ep.build().error_on_failure()]).map_err(|e| e.to_string())
    }

    #[cfg(not(windows))]
    fn directml(_builder: ort::session::builder::SessionBuilder) -> Result<ort::session::builder::SessionBuilder, String> {
        Err("DirectML is Windows only".into())
    }

    fn fail(e: ort::Error) -> MlError {
        MlError::Inference(e.to_string())
    }

    /// Run `net` on `inputs`; its output as f32 values.
    fn run_net(net: &mut Net, inputs: Vec<(std::borrow::Cow<'_, str>, ort::session::SessionInputValue<'_>)>) -> Result<Vec<f32>, MlError> {
        let outputs = net.session.run(inputs).map_err(fail)?;
        let value = outputs.get(&net.output).ok_or_else(|| MlError::Inference("the model gave no output".into()))?;
        let (_, data) = value.try_extract_tensor::<f32>().map_err(fail)?;
        Ok(data.to_vec())
    }

    impl Nets for Loaded {
        fn text(&mut self, ids: &[i64], count: usize) -> Result<(Vec<f32>, Vec<f32>), MlError> {
            let t = Tensor::from_array(([count, LENGTH], ids.to_vec().into_boxed_slice())).map_err(fail)?;
            if !self.meta.xl {
                return Ok((run_net(&mut self.text, ort::inputs!["input_ids" => t])?, Vec::new()));
            }
            // SDXL: the hidden states and the pooled embedding.
            let pooled_name = self.text.outputs.get(1).cloned().ok_or_else(|| MlError::Inference("the text encoder has no pooled output (is it an SDXL export?)".into()))?;
            let outputs = self.text.session.run(ort::inputs!["input_ids" => t]).map_err(fail)?;
            let read = |name: &str| -> Result<Vec<f32>, MlError> {
                let value = outputs.get(name).ok_or_else(|| MlError::Inference("the text encoder gave no output".into()))?;
                Ok(value.try_extract_tensor::<f32>().map_err(fail)?.1.to_vec())
            };
            Ok((read(&self.text.output)?, read(&pooled_name)?))
        }

        fn encode(&mut self, image: &[f32], side: usize) -> Result<Vec<f32>, MlError> {
            let started = std::time::Instant::now();
            let t = Tensor::from_array(([1usize, 3, side, side], image.to_vec().into_boxed_slice())).map_err(fail)?;
            let out = run_net(&mut self.enc, ort::inputs!["image" => t]);
            log::debug!("[ml] {NAME}: encoded {side} × {side} in {:.1} s", started.elapsed().as_secs_f32());
            out
        }

        fn unet(&mut self, sample: &[f32], t: f32, hidden: &[f32], pooled: &[f32], ls: usize) -> Result<Vec<f32>, MlError> {
            // One call per guidance branch, not a batch of two: each GPU submission stays short
            // (Windows resets a GPU that is busy for more than about two seconds).
            let (s_len, h_len, p_len) = (9 * ls * ls, LENGTH * self.meta.hidden, self.meta.pooled);
            let mut out = Vec::with_capacity(2 * 4 * ls * ls);
            for i in 0..2 {
                let part = sample.get(i * s_len..(i + 1) * s_len).ok_or_else(|| MlError::Inference("bad UNet input".into()))?;
                let hpart = hidden.get(i * h_len..(i + 1) * h_len).ok_or_else(|| MlError::Inference("bad text input".into()))?;
                let s = Tensor::from_array(([1usize, 9, ls, ls], part.to_vec().into_boxed_slice())).map_err(fail)?;
                let ts = Tensor::from_array(([1usize], vec![t].into_boxed_slice())).map_err(fail)?;
                let h = Tensor::from_array(([1usize, LENGTH, self.meta.hidden], hpart.to_vec().into_boxed_slice())).map_err(fail)?;
                let pred = if self.meta.xl {
                    // SDXL's size conditioning: the crop is the whole picture, uncropped.
                    let px = (ls * 8) as f32;
                    let ppart = pooled.get(i * p_len..(i + 1) * p_len).ok_or_else(|| MlError::Inference("bad pooled text input".into()))?;
                    let pe = Tensor::from_array(([1usize, p_len], ppart.to_vec().into_boxed_slice())).map_err(fail)?;
                    let ids = Tensor::from_array(([1usize, 6], vec![px, px, 0.0, 0.0, px, px].into_boxed_slice())).map_err(fail)?;
                    run_net(&mut self.unet, ort::inputs!["sample" => s, "timestep" => ts, "encoder_hidden_states" => h, "text_embeds" => pe, "time_ids" => ids])?
                } else {
                    run_net(&mut self.unet, ort::inputs!["sample" => s, "timestep" => ts, "encoder_hidden_states" => h])?
                };
                out.extend(pred);
            }
            Ok(out)
        }

        fn decode(&mut self, latent: &[f32], width: usize, height: usize) -> Result<Vec<f32>, MlError> {
            let started = std::time::Instant::now();
            let t = Tensor::from_array(([1usize, 4, height / 8, width / 8], latent.to_vec().into_boxed_slice())).map_err(fail)?;
            let out = run_net(&mut self.dec, ort::inputs!["latent" => t]);
            log::debug!("[ml] {NAME}: decoded {width} × {height} in {:.1} s", started.elapsed().as_secs_f32());
            out
        }

        fn tokens(&self, text: &str) -> Vec<i64> {
            self.tokenizer.encode(text)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The real model, when `PHOTOCRAFT_MODEL_DIR` holds it and ONNX Runtime (set
        /// `PHOTOCRAFT_ML_DEVICE=cpu` to keep it off the GPU): a grey crop with a hole comes back
        /// grey-ish. `cargo test -p photocraft-ml --release -- --ignored diffusion`.
        #[test]
        #[ignore = "needs the diffusion model and ONNX Runtime"]
        fn the_installed_diffusion_model_fills_a_flat_grey() {
            let side = 512;
            let n = side * side;
            let rgb = vec![0.5f32; 3 * n];
            let mut mask = vec![0.0f32; n];
            for y in 200..300 {
                for x in 200..300 {
                    mask[y * side + x] = 1.0;
                }
            }
            let s = Settings { steps: 8, ..Settings::default() };
            let t = std::time::Instant::now();
            let (out, device) = inpaint(&rgb, &mask, None, side, &s, &|_| false).unwrap();
            eprintln!("diffusion: 8 steps on {device:?} in {:.1} s (load included); GPU: {:?}", t.elapsed().as_secs_f32(), gpu_fallback());
            let mean: f32 = (200..300).map(|y| out[y * side + 250]).sum::<f32>() / 100.0;
            assert!((mean - 0.5).abs() < 0.2, "{mean} on {device:?}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Settings, State};
    use crate::{Device, MlError};

    pub fn state() -> State {
        State::Missing
    }

    pub fn preload() {}

    pub fn gpu_fallback() -> Option<String> {
        None
    }

    pub fn availability() -> Result<(), MlError> {
        Err(MlError::Unsupported)
    }

    pub fn inpaint(
        _rgb: &[f32],
        _mask: &[f32],
        _hint: Option<&[f32]>,
        _side: usize,
        _settings: &Settings,
        _cancelled: &dyn Fn(f32) -> bool,
    ) -> Result<(Vec<f32>, Device), MlError> {
        Err(MlError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta::parse(r#"{"kind": "sd-inpaint", "in_channels": 9, "hidden_size": 8, "vae_scaling_factor": 0.18215, "beta_start": 0.00085, "beta_end": 0.012, "num_train_timesteps": 1000, "steps_offset": 1, "prediction_type": "epsilon", "beta_schedule": "scaled_linear"}"#).unwrap()
    }

    #[test]
    fn model_json_is_checked() {
        assert_eq!(meta().in_channels, 9);
        assert!(Meta::parse(r#"{"kind": "lama"}"#).is_err());
        assert!(Meta::parse(r#"{"kind": "sd-inpaint", "in_channels": 4}"#).is_err());
        assert!(Meta::parse(r#"{"kind": "sd-inpaint", "prediction_type": "v_prediction"}"#).is_err());
        assert!(Meta::parse("nope").is_err());
    }

    #[test]
    fn the_schedule_matches_stable_diffusion() {
        let (cum, steps) = schedule(&meta(), 25);
        // diffusers' DDIM with steps_offset 1 and "leading" spacing: 961, 921, … 1.
        assert_eq!(steps.len(), 25);
        assert_eq!(steps[0], 961);
        assert_eq!(steps[24], 1);
        // alphas_cumprod of SD 1.x: 0.99915 at t = 0, ~0.0047 at t = 999.
        assert!((cum[0] - 0.99915).abs() < 1e-5, "{}", cum[0]);
        assert!((cum[999] - 0.0047).abs() < 3e-4, "{}", cum[999]);
    }

    #[test]
    fn noise_is_standard_normal_and_seeded() {
        let a = Noise::new(7).gaussian(100_001);
        assert_eq!(a.len(), 100_001);
        let mean = a.iter().map(|v| f64::from(*v)).sum::<f64>() / a.len() as f64;
        let var = a.iter().map(|v| (f64::from(*v) - mean).powi(2)).sum::<f64>() / a.len() as f64;
        assert!(mean.abs() < 0.02 && (var - 1.0).abs() < 0.03, "{mean} {var}");
        assert_eq!(Noise::new(7).gaussian(10), a[..10]);
        assert_ne!(Noise::new(8).gaussian(10), a[..10]);
    }

    /// Stand-in networks: the "noise" the UNet predicts is the one that leads to a latent of all
    /// `target`, and the VAE is the identity on 8 × 8 means (latent = mean of each cell).
    struct Fake {
        target: f32,
        unet_calls: usize,
        timesteps: Vec<f32>,
        cum: Vec<f64>,
        pooled: usize,
        /// The sizes decoded.
        decoded: Vec<(usize, usize)>,
    }

    impl Nets for Fake {
        fn text(&mut self, ids: &[i64], count: usize) -> Result<(Vec<f32>, Vec<f32>), MlError> {
            assert_eq!(ids.len(), count * 77);
            Ok((vec![0.0; count * 77 * 8], vec![0.0; count * self.pooled]))
        }
        fn encode(&mut self, image: &[f32], side: usize) -> Result<Vec<f32>, MlError> {
            let ls = side / 8;
            let mut out = vec![0f32; 4 * ls * ls];
            for c in 0..3 {
                for y in 0..side {
                    for x in 0..side {
                        out[c * ls * ls + (y / 8) * ls + x / 8] += image[c * side * side + y * side + x] / 64.0;
                    }
                }
            }
            Ok(out)
        }
        fn unet(&mut self, sample: &[f32], t: f32, hidden: &[f32], pooled: &[f32], ls: usize) -> Result<Vec<f32>, MlError> {
            self.unet_calls += 1;
            self.timesteps.push(t);
            assert_eq!(sample.len(), 2 * 9 * ls * ls);
            assert_eq!(hidden.len(), 2 * 77 * 8);
            assert_eq!(pooled.len(), 2 * self.pooled);
            // eps such that x0 = target (in scaled latent units).
            let a = self.cum[t as usize];
            let ln = 4 * ls * ls;
            let x = &sample[..ln];
            let eps: Vec<f32> = x.iter().map(|x| ((f64::from(*x) - a.sqrt() * f64::from(self.target)) / (1.0 - a).sqrt()) as f32).collect();
            Ok([eps.clone(), eps].concat())
        }
        fn decode(&mut self, latent: &[f32], width: usize, height: usize) -> Result<Vec<f32>, MlError> {
            let (lw, lh) = (width / 8, height / 8);
            assert_eq!(latent.len(), 4 * lw * lh);
            self.decoded.push((width, height));
            let mut out = vec![0f32; 3 * width * height];
            for c in 0..3 {
                for y in 0..height {
                    for x in 0..width {
                        out[c * width * height + y * width + x] = latent[c * lw * lh + (y / 8) * lw + x / 8];
                    }
                }
            }
            Ok(out)
        }
        fn tokens(&self, _text: &str) -> Vec<i64> {
            vec![0; 77]
        }
    }

    #[test]
    fn sampling_reaches_what_the_unet_predicts_and_a_hint_shortens_it() {
        let m = meta();
        let (cum, _) = schedule(&m, 10);
        let side = 64;
        let n = side * side;
        let rgb = vec![0.5f32; 3 * n];
        let mut mask = vec![0f32; n];
        mask[..n / 2].fill(1.0);
        // The decoded value is target / scaling in −1..1: pick target so the image is 0.75.
        let target = 0.5 * m.scaling;
        let mut f = Fake { target, unet_calls: 0, timesteps: Vec::new(), cum: cum.clone(), pooled: 0, decoded: Vec::new() };
        let s = Settings { steps: 10, ..Settings::default() };
        let out = run(&mut f, &m, &s, &rgb, &mask, None, side, &|_| false).unwrap();
        assert_eq!(f.unet_calls, 10);
        // DDIM's last step keeps a trace of the noise (alpha never reaches 1): on average 0.75.
        let mean = out.iter().sum::<f32>() / out.len() as f32;
        assert!((mean - 0.75).abs() < 0.01 && out.iter().all(|v| (v - 0.75).abs() < 0.5), "{mean}");
        // A hint at strength 0.4 runs the last 4 of the 10 steps.
        let mut f = Fake { target, unet_calls: 0, timesteps: Vec::new(), cum: cum.clone(), pooled: 0, decoded: Vec::new() };
        let hint = vec![0.25f32; 3 * n];
        let out = run(&mut f, &m, &Settings { strength: 0.4, ..s.clone() }, &rgb, &mask, Some(&hint), side, &|_| false).unwrap();
        assert_eq!(f.unet_calls, 4);
        assert!(f.timesteps.windows(2).all(|w| w[0] > w[1]));
        assert!(out.iter().all(|v| v.is_finite()));
        // Cancelled before a step; bad sizes.
        assert_eq!(run(&mut f, &m, &s, &rgb, &mask, None, side, &|_| true), Err(MlError::Cancelled));
        assert!(matches!(run(&mut f, &m, &s, &rgb, &mask, None, 60, &|_| false), Err(MlError::BadInput(_))));
        assert!(matches!(run(&mut f, &m, &s, &rgb[..5], &mask, None, side, &|_| false), Err(MlError::BadInput(_))));
    }

    #[test]
    fn sdxl_passes_the_pooled_text_and_a_mismatched_model_json_is_an_error() {
        let m = Meta { hidden: 8, pooled: 6, ..Meta::parse(r#"{"kind": "sdxl-inpaint"}"#).unwrap() };
        let (cum, _) = schedule(&m, 5);
        let side = 64;
        let n = side * side;
        let (rgb, mut mask) = (vec![0.5f32; 3 * n], vec![0f32; n]);
        mask[..n / 2].fill(1.0);
        let s = Settings { steps: 5, ..Settings::default() };
        let mut f = Fake { target: 0.5 * m.scaling, unet_calls: 0, timesteps: Vec::new(), cum: cum.clone(), pooled: 6, decoded: Vec::new() };
        let out = run(&mut f, &m, &s, &rgb, &mask, None, side, &|_| false).unwrap();
        assert_eq!(f.unet_calls, 5);
        assert!(out.iter().all(|v| v.is_finite()));
        // The text encoder gives no pooled embedding (an SD 1.x export called sdxl): an error, not a panic.
        let mut f = Fake { target: 0.5, unet_calls: 0, timesteps: Vec::new(), cum, pooled: 0, decoded: Vec::new() };
        assert!(matches!(run(&mut f, &m, &s, &rgb, &mask, None, side, &|_| false), Err(MlError::Inference(_))));
        assert_eq!(f.unet_calls, 0);
    }

    #[test]
    fn the_decode_window_is_the_holes_box_plus_a_margin_inside_the_latent() {
        let ls = 32;
        let mut small = vec![0f32; ls * ls];
        // Nothing masked, or the wrong size: decode it all.
        assert_eq!(decode_window(&small, ls, 4), None);
        assert_eq!(decode_window(&small[..10], ls, 4), None);
        assert_eq!(decode_window(&[], 0, 4), None);
        // Cells (10..13, 20..22) masked: grown by 4.
        for y in 20..22 {
            for x in 10..13 {
                small[y * ls + x] = 1.0;
            }
        }
        assert_eq!(decode_window(&small, ls, 4), Some(Window { x: 6, y: 16, w: 11, h: 10 }));
        // Cut to the latent at its edges.
        small[0] = 1.0;
        small[ls * ls - 1] = 1.0;
        assert_eq!(decode_window(&small, ls, 0), None, "the whole latent");
        small[ls * ls - 1] = 0.0;
        assert_eq!(decode_window(&small, ls, 2), Some(Window { x: 0, y: 0, w: 15, h: 24 }));
        // A window over most of the latent: decoded whole.
        assert_eq!(decode_window(&small, ls, 16), None);
        // A margin past the latent saturates instead of overflowing.
        assert_eq!(decode_window(&small, ls, usize::MAX), None);
    }

    #[test]
    fn a_window_is_cut_from_the_latent_and_pasted_into_the_image() {
        let ls = 4;
        let latent: Vec<f32> = (0..4 * ls * ls).map(|i| i as f32).collect();
        let w = Window { x: 1, y: 2, w: 2, h: 1 };
        // Channel c, row 2, columns 1 and 2.
        assert_eq!(crop_latent(&latent, ls, w), Some(vec![9.0, 10.0, 25.0, 26.0, 41.0, 42.0, 57.0, 58.0]));
        assert_eq!(crop_latent(&latent, ls, Window { x: 3, y: 0, w: 2, h: 1 }), None);
        assert_eq!(crop_latent(&latent, ls, Window { x: usize::MAX, y: 0, w: 2, h: 1 }), None);
        assert_eq!(crop_latent(&latent[1..], ls, w), None);
        let side = ls * 8;
        let mut image = vec![-1f32; 3 * side * side];
        let part: Vec<f32> = (0..3 * 16 * 8).map(|i| (i / (16 * 8)) as f32).collect();
        assert_eq!(paste_decoded(&mut image, side, &part, w), Some(()));
        let at = |c: usize, x: usize, y: usize| image[c * side * side + y * side + x];
        assert_eq!((at(0, 8, 16), at(1, 23, 23), at(2, 15, 20)), (0.0, 1.0, 2.0));
        assert_eq!((at(0, 7, 16), at(1, 24, 16), at(2, 8, 15), at(2, 8, 24)), (-1.0, -1.0, -1.0, -1.0));
        assert_eq!(image.iter().filter(|v| **v >= 0.0).count(), part.len());
        // Wrong sizes: an error, not a panic.
        assert_eq!(paste_decoded(&mut image, side, &part[1..], w), None);
        assert_eq!(paste_decoded(&mut image, side, &part, Window { x: 3, ..w }), None);
        assert_eq!(paste_decoded(&mut image[1..], side, &part, w), None);
    }

    #[test]
    fn a_small_hole_decodes_only_its_window_and_keeps_the_crop_elsewhere() {
        let m = meta();
        let (cum, _) = schedule(&m, 4);
        let side = 512;
        let n = side * side;
        let rgb: Vec<f32> = (0..3 * n).map(|i| (i % 7) as f32 / 10.0).collect();
        let mut mask = vec![0f32; n];
        // 40 × 24 pixels at (200, 296): latent cells 25..30 × 37..40, plus the margin.
        for y in 296..320 {
            for x in 200..240 {
                mask[y * side + x] = 1.0;
            }
        }
        let mut f = Fake { target: 0.5 * m.scaling, unet_calls: 0, timesteps: Vec::new(), cum, pooled: 0, decoded: Vec::new() };
        let out = run(&mut f, &m, &Settings { steps: 4, ..Settings::default() }, &rgb, &mask, None, side, &|_| false).unwrap();
        let (cx, cy) = (25 - DECODE_MARGIN, 37 - DECODE_MARGIN);
        assert_eq!(f.decoded, vec![((5 + 2 * DECODE_MARGIN) * 8, (3 + 2 * DECODE_MARGIN) * 8)]);
        // The hole has the fill (on average 0.75); outside the window, the crop as it was.
        let hole: Vec<f32> = (0..n).filter(|p| mask[*p] > 0.0).map(|p| out[p]).collect();
        let mean = hole.iter().sum::<f32>() / hole.len() as f32;
        assert!((mean - 0.75).abs() < 0.05, "{mean}");
        for (x, y) in [(0, 0), (cx * 8 - 1, 310), (511, 511), (220, cy * 8 - 1)] {
            for c in 0..3 {
                let p = c * n + y * side + x;
                assert!((out[p] - rgb[p]).abs() < 1e-6, "{x},{y}");
            }
        }
    }

    #[test]
    fn sdxl_model_json_defaults_and_limits() {
        let m = Meta::parse(r#"{"kind": "sdxl-inpaint"}"#).unwrap();
        assert!(m.xl);
        assert_eq!((m.side, m.hidden, m.pooled, m.style), (1024, 2048, 1280, Style::General));
        assert!((m.scaling - 0.13025).abs() < 1e-6);
        assert_eq!(Meta::parse(r#"{"kind": "sdxl-inpaint", "style": "anime"}"#).unwrap().style, Style::Anime);
        // SD 1.x keeps its 512 crops, no pooled text, and the anime prompts.
        assert_eq!((meta().side, meta().pooled, meta().style, meta().xl), (512, 0, Style::Anime, false));
        assert!(Meta::parse(r#"{"kind": "sdxl-inpaint", "side": 1000}"#).is_err());
        assert!(Meta::parse(r#"{"kind": "sdxl-inpaint", "side": 1e30}"#).is_err());
        assert!(Meta::parse(r#"{"kind": "sdxl-inpaint", "pooled_size": 0}"#).is_err());
        assert!(Meta::parse(r#"{"kind": "sdxl-inpaint", "vae_scaling_factor": -1}"#).is_ok_and(|m| m.scaling > 0.0));
        assert!(Meta::parse(r#"{"kind": "sdxl-inpaint", "hidden_size": 1e12}"#).is_err());
    }

    #[test]
    fn prompts_default_by_colour_and_keep_the_users() {
        let s = Settings::default();
        let (p, n) = s.prompts_for(true, Style::Anime);
        assert!(p.contains("monochrome") && n.contains("speech bubble") && n.contains("color"));
        let (p, n) = s.prompts_for(false, Style::Anime);
        assert!(!p.contains("monochrome") && !n.contains("colorful") && p.contains("anime"));
        // The general style has no anime terms.
        let (p, n) = s.prompts_for(false, Style::General);
        assert!(!p.contains("anime") && !p.contains("masterpiece") && n.contains("watermark") && !n.contains("speech bubble"));
        let (p, _) = s.prompts_for(true, Style::General);
        assert!(p.contains("black and white") && !p.contains("manga"));
        let mine = Settings { prompt: "a girl's face".into(), negative: "hat".into(), ..s };
        let (p, n) = mine.prompts_for(false, Style::Anime);
        assert!(p.starts_with("a girl's face") && n == "hat");
        let (p, n) = mine.prompts_for(false, Style::General);
        assert!(p == "a girl's face" && n == "hat");
    }
}
