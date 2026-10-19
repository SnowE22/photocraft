//! LaMa inpainting on ONNX Runtime: finding the model, one session per process, and a run.
//!
//! The model file is the Carve export of big-lama (`lama_fp32.onnx`, Apache-2.0: declared 512 ×
//! 512, opset 17; inputs `image`, RGB 0–1, and `mask`, 1 = fill; output RGB 0–255). Its Fourier
//! units build their DFT matrices from the tensors' shapes, so once [`crate::onnx_edit::symbolic_size`]
//! frees the declared size it runs at any square side ([`SIDES`]): 1024 for the Quality mode
//! ([`crate::inpaint::LAMA_QUALITY`]). Loading takes
//! seconds (DirectML compiles the network for the GPU), so [`preload`] starts it in the background
//! when the Remove tool is picked, and the session then serves every removal of the run.

use std::path::PathBuf;

use crate::{Device, MlError};

/// The model's file.
pub const FILE: &str = "lama_fp32.onnx";

/// The file names looked for, in order. A copy already rewritten for DirectML (by
/// [`crate::onnx_edit`] or by hand) is used as it is.
pub const FILES: &[&str] = &[FILE, "lama_fp32_dml.onnx"];

/// The model's name in messages.
pub const NAME: &str = "LaMa";

/// The square sides the model is run at: the standard crop and the Quality mode's.
pub const SIDES: [usize; 2] = [crate::inpaint::SIDE, 1024];

/// The square side of planar RGB `rgb` and its `mask`, when it is one of [`SIDES`].
pub fn side_of(rgb: &[f32], mask: &[f32]) -> Option<usize> {
    SIDES.into_iter().find(|s| mask.len() == s * s && rgb.len() == 3 * s * s)
}


/// What the model is doing, for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The model or the runtime isn't installed (or this build has none).
    Missing,
    /// Installed, not loaded yet.
    Idle,
    Loading,
    Ready(Device),
    Failed(String),
}

/// The model file, or why there is none.
pub fn model_file() -> Result<PathBuf, MlError> {
    let dirs = crate::search_dirs();
    crate::find_in(&dirs, FILES).ok_or_else(|| MlError::ModelMissing { model: NAME, file: FILE, searched: crate::list(&dirs) })
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{availability, inpaint, preload, state};

#[cfg(target_arch = "wasm32")]
pub use web::{availability, inpaint, preload, state};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Mutex, PoisonError};

    use ort::session::Session;
    use ort::value::Tensor;

    use super::{NAME, SIDES, State, side_of};
    use crate::inpaint::SIDE;
    use crate::{Device, MlError, runtime};

    struct Loaded {
        session: Session,
        device: Device,
        output: String,
    }

    /// The session; `None` until the first load, and again after a failed run so the next one
    /// starts afresh.
    static SLOT: Mutex<Option<Result<Loaded, MlError>>> = Mutex::new(None);
    /// The load's progress, readable while the slot is locked for a load.
    static PHASE: AtomicU8 = AtomicU8::new(IDLE);
    static FAILURE: Mutex<Option<MlError>> = Mutex::new(None);
    /// Its last use; unloaded after [`IDLE_AFTER`] without one (`crate::idle`).
    static KEEP: crate::idle::Keeper = crate::idle::Keeper::new();
    /// LaMa holds about 1.2 GB of GPU memory while loaded and loads again in seconds.
    const IDLE_AFTER: std::time::Duration = std::time::Duration::from_secs(300);
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

    /// Whether the model can run: `Ok` when it is loaded, loading or installed, else why not (the
    /// missing file and the folders searched, or the load's failure).
    pub fn availability() -> Result<(), MlError> {
        match PHASE.load(Ordering::Acquire) {
            LOADING | READY_GPU | READY_CPU => Ok(()),
            FAILED => Err(FAILURE.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| MlError::Load("unknown error".into()))),
            _ => {
                super::model_file()?;
                runtime::library(&crate::search_dirs())?;
                Ok(())
            }
        }
    }

    /// Start loading in the background, when the model and the runtime are installed and nothing
    /// has started yet.
    pub fn preload() {
        // Unloaded for being idle: not again until it is used.
        if PHASE.load(Ordering::Acquire) != IDLE || KEEP.released() || availability().is_err() {
            return;
        }
        let started = std::thread::Builder::new().name("lama-load".into()).spawn(|| {
            if let Err(e) = with_session(|_| Ok(())) {
                log::info!("[ml] {NAME} didn't load: {e}");
            }
        });
        if let Err(e) = started {
            log::warn!("[ml] couldn't start loading {NAME}: {e}");
        }
    }

    /// Run the model on planar RGB (a square of one of [`SIDES`], 0–1) and its mask (1 = fill);
    /// the fill comes back as planar RGB 0–1. Loads the model first if it isn't loaded (seconds,
    /// once).
    pub fn inpaint(rgb: &[f32], mask: &[f32]) -> Result<(Vec<f32>, Device), MlError> {
        let Some(side) = side_of(rgb, mask) else {
            return Err(MlError::BadInput(format!("{NAME} takes a square of {SIDES:?} pixels")));
        };
        let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
        KEEP.touch();
        let out = run_in(&mut slot, |l| run(l, rgb, mask, side).map(|v| (v, l.device)));
        KEEP.touch();
        if matches!(out, Err(MlError::Inference(_))) {
            // A lost device (driver reset) or a broken session: load again next time.
            *slot = None;
            PHASE.store(IDLE, Ordering::Release);
        }
        out
    }

    /// Drop the session when it has been idle for [`IDLE_AFTER`] (see `crate::idle`).
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

    fn with_session<R>(f: impl FnOnce(&mut Loaded) -> Result<R, MlError>) -> Result<R, MlError> {
        let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
        run_in(&mut slot, f)
    }

    fn run_in<R>(slot: &mut Option<Result<Loaded, MlError>>, f: impl FnOnce(&mut Loaded) -> Result<R, MlError>) -> Result<R, MlError> {
        // Missing files may have been installed since: look again.
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
                KEEP.watch("lama", release_idle);
            }
        }
        match slot.as_mut() {
            Some(Ok(l)) => f(l),
            Some(Err(e)) => Err(e.clone()),
            None => Err(MlError::Load("no session".into())),
        }
    }

    fn load() -> Result<Loaded, MlError> {
        let dirs = crate::search_dirs();
        let library = runtime::library(&dirs)?;
        let model = super::model_file()?;
        let env = runtime::environment(&library)?;
        let started = std::time::Instant::now();
        let mut bytes = std::fs::read(&model).map_err(|e| MlError::Load(format!("{}: {e}", model.display())))?;
        // Any square side, not just the declared 512 (the Quality mode runs it at 1024).
        match crate::onnx_edit::symbolic_size(&bytes) {
            Ok(Some(b)) => bytes = b,
            Ok(None) => {}
            Err(e) => log::warn!("[ml] {NAME}'s input size wasn't freed (only 512 will run): {e}"),
        }
        #[cfg(windows)]
        if !crate::cpu_only() {
            let rewritten = match crate::onnx_edit::for_directml(&bytes) {
                Ok(Some((b, n))) => {
                    log::info!("[ml] {NAME}: {n} Fourier MatMuls made 4-D for DirectML");
                    Some(b)
                }
                Ok(None) => None,
                Err(e) => {
                    log::warn!("[ml] {NAME} wasn't rewritten for DirectML: {e}");
                    None
                }
            };
            match session(&env, rewritten.as_deref().unwrap_or(&bytes), Device::DirectMl) {
                Ok(l) => {
                    log::info!("[ml] {NAME} on DirectML in {:.1} s", started.elapsed().as_secs_f32());
                    return Ok(l);
                }
                Err(e) => log::warn!("[ml] {NAME} on DirectML failed ({e}); using the CPU"),
            }
        }
        let l = session(&env, &bytes, Device::Cpu)?;
        log::info!("[ml] {NAME} on the CPU in {:.1} s", started.elapsed().as_secs_f32());
        Ok(l)
    }

    fn session(env: &ort::environment::Environment, bytes: &[u8], device: Device) -> Result<Loaded, MlError> {
        let fail = |e: &dyn std::fmt::Display| MlError::Load(format!("{}: {e}", device.name()));
        let builder = Session::builder(env).map_err(|e| fail(&e))?;
        let mut builder = match device {
            Device::DirectMl => directml(builder).map_err(|e| fail(&e))?,
            Device::Cpu => builder.with_execution_providers([ort::ep::CPU::default().build().error_on_failure()]).map_err(|e| fail(&e))?,
        };
        let session = builder.commit_from_memory(bytes).map_err(|e| fail(&e))?;
        let inputs: Vec<&str> = session.inputs().iter().map(|i| i.name()).collect();
        if !inputs.contains(&"image") || !inputs.contains(&"mask") {
            return Err(MlError::Load(format!("not the {NAME} model (its inputs are {inputs:?}, not image and mask)")));
        }
        let output = session.outputs().first().map(|o| o.name().to_string()).ok_or_else(|| MlError::Load("the model has no output".into()))?;
        let mut l = Loaded { session, device, output };
        // The first run sets the GPU up (a second or two): do it now, not on the first stroke.
        run(&mut l, &vec![0.0; 3 * SIDE * SIDE], &vec![0.0; SIDE * SIDE], SIDE).map_err(|e| MlError::Load(e.to_string()))?;
        Ok(l)
    }

    /// DirectML on the high-performance GPU; it needs sequential execution without memory patterns.
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

    fn run(l: &mut Loaded, rgb: &[f32], mask: &[f32], side: usize) -> Result<Vec<f32>, MlError> {
        let fail = |e: ort::Error| MlError::Inference(e.to_string());
        if !SIDES.contains(&side) {
            return Err(MlError::BadInput(format!("{NAME} takes a square of {SIDES:?} pixels")));
        }
        let image = Tensor::from_array(([1usize, 3, side, side], rgb.to_vec().into_boxed_slice())).map_err(fail)?;
        let mask = Tensor::from_array(([1usize, 1, side, side], mask.to_vec().into_boxed_slice())).map_err(fail)?;
        let outputs = l.session.run(ort::inputs!["image" => image, "mask" => mask]).map_err(fail)?;
        let value = outputs.get(&l.output).ok_or_else(|| MlError::Inference("the model gave no output".into()))?;
        let (_, data) = value.try_extract_tensor::<f32>().map_err(fail)?;
        if data.len() != 3 * side * side {
            return Err(MlError::Inference(format!("the model gave {} values, not {}", data.len(), 3 * side * side)));
        }
        // LaMa's output is 0–255.
        Ok(data.iter().map(|v| (v / 255.0).clamp(0.0, 1.0)).collect())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn wrong_sizes_are_refused_before_anything_loads() {
            assert!(matches!(inpaint(&[0.0; 3], &[0.0; 1]), Err(MlError::BadInput(_))));
            assert_eq!(PHASE.load(Ordering::Acquire), IDLE);
        }

        #[test]
        fn without_its_files_the_model_is_missing_and_says_where_it_looked() {
            if std::env::var_os("PHOTOCRAFT_MODEL_DIR").is_some() {
                return;
            }
            let e = availability().unwrap_err();
            assert!(matches!(e, MlError::ModelMissing { .. }), "{e}");
            assert!(e.to_string().contains("looked for lama_fp32.onnx in"), "{e}");
            assert_eq!(state(), State::Missing);
            // Preloading without a model starts nothing.
            preload();
            assert_eq!(PHASE.load(Ordering::Acquire), IDLE);
        }

        /// The real model, when `PHOTOCRAFT_MODEL_DIR` holds it and ONNX Runtime: a grey square
        /// with a hole comes back grey. `cargo test -p photocraft-ml -- --ignored`.
        #[test]
        #[ignore = "needs the LaMa model and ONNX Runtime"]
        fn the_installed_model_fills_a_flat_grey() {
            let n = SIDE * SIDE;
            let rgb = vec![0.5f32; 3 * n];
            let mut mask = vec![0.0f32; n];
            for y in 200..300 {
                for x in 200..300 {
                    mask[y * SIDE + x] = 1.0;
                }
            }
            let (out, device) = inpaint(&rgb, &mask).unwrap();
            let hole = out[250 * SIDE + 250];
            assert!((hole - 0.5).abs() < 0.05, "{hole} on {device:?}");
            assert!(matches!(state(), State::Ready(_)));
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::State;
    use crate::{Device, MlError};

    pub fn state() -> State {
        State::Missing
    }

    pub fn preload() {}

    pub fn availability() -> Result<(), MlError> {
        Err(MlError::Unsupported)
    }

    pub fn inpaint(_rgb: &[f32], _mask: &[f32]) -> Result<(Vec<f32>, Device), MlError> {
        Err(MlError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_are_recognised_and_others_refused() {
        let n = 512 * 512;
        assert_eq!(side_of(&vec![0.0; 3 * n], &vec![0.0; n]), Some(512));
        assert_eq!(side_of(&vec![0.0; 3 * 1024 * 1024], &vec![0.0; 1024 * 1024]), Some(1024));
        assert_eq!(side_of(&[0.0; 3], &[0.0; 1]), None);
        assert_eq!(side_of(&vec![0.0; 3 * n], &[0.0; 4]), None, "mask and image disagree");
        assert!(matches!(inpaint(&[0.0; 3], &[0.0; 1]), Err(MlError::BadInput(_))));
    }
}
