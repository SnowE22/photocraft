//! Local machine-learning models.
//!
//! One model so far: LaMa (Suvorov et al., *Resolution-robust Large Mask Inpainting with Fourier
//! Convolutions*, WACV 2022; Apache-2.0), the inpainting network behind the Remove tool's
//! Generative AI mode ([`lama`]). It runs on ONNX Runtime, which is loaded at run time from next
//! to the executable (on Windows `onnxruntime.dll` with `DirectML.dll`), so a build or an install
//! without it works and just reports the model as missing. Nothing links at build time and no
//! weights are committed: `packaging/models/README.md` says what goes where.
//!
//! On Windows the model runs on DirectML (any DirectX 12 GPU), elsewhere or when DirectML fails on
//! the CPU. LaMa's Fourier units multiply a 2-D matrix with 5-D tensors, which DirectML refuses;
//! [`onnx_edit`] rewrites those products in 4-D when the model loads, on the protobuf bytes.
//!
//! [`inpaint`] takes a document region and a hole to the model's fixed 512 × 512 input and back:
//! a crop around each part of the hole, scaled to the model's size, and the fill scaled back into
//! the hole only. It is pure and runs everywhere; the WebAssembly build has no runtime and
//! answers [`MlError::Unsupported`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod inpaint;
pub mod lama;
pub mod onnx_edit;
#[cfg(not(target_arch = "wasm32"))]
mod runtime;

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// Why a model couldn't run. Every failure is a value: the caller falls back or reports it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MlError {
    #[error("ONNX Runtime isn't installed (looked for {file} in {searched})")]
    RuntimeMissing { file: String, searched: String },
    #[error("the {model} model isn't installed (looked for {file} in {searched})")]
    ModelMissing { model: &'static str, file: &'static str, searched: String },
    #[error("ONNX Runtime didn't load: {0}")]
    Runtime(String),
    #[error("the model didn't load: {0}")]
    Load(String),
    #[error("the model failed: {0}")]
    Inference(String),
    /// The model can't do this one (transparent surroundings, a colour mode it has no channels for):
    /// the caller uses its other method.
    #[error("{0}")]
    Unsuitable(String),
    #[error("{0}")]
    BadInput(String),
    #[error("local AI models aren't available in this build")]
    Unsupported,
    #[error("cancelled")]
    Cancelled,
}

impl From<photocraft_raster::Cancelled> for MlError {
    fn from(_: photocraft_raster::Cancelled) -> Self {
        MlError::Cancelled
    }
}

/// Where a model ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// DirectML on a DirectX 12 GPU (Windows).
    DirectMl,
    Cpu,
}

impl Device {
    pub fn name(self) -> &'static str {
        match self {
            Device::DirectMl => "DirectML",
            Device::Cpu => "CPU",
        }
    }
}

static EXTRA_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Also look for models and the runtime in `dir` (the app adds its settings folder's `models`).
pub fn add_search_dir(dir: PathBuf) {
    let mut dirs = EXTRA_DIRS.lock().unwrap_or_else(PoisonError::into_inner);
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// Where models and the runtime are looked for, in order: `PHOTOCRAFT_MODEL_DIR`, the
/// executable's folder and its `models` folder, then the folders added with [`add_search_dir`].
pub fn search_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("PHOTOCRAFT_MODEL_DIR").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(d));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        out.push(exe_dir.join("models"));
        out.push(exe_dir);
    }
    for d in EXTRA_DIRS.lock().unwrap_or_else(PoisonError::into_inner).iter() {
        if !out.contains(d) {
            out.push(d.clone());
        }
    }
    out
}

/// The first of `names` found in `dirs`.
pub(crate) fn find_in(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// `dirs` for a message.
pub(crate) fn list(dirs: &[PathBuf]) -> String {
    if dirs.is_empty() {
        return "no folder".into();
    }
    dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_dirs_are_searched_once_and_in_order() {
        let a = std::env::temp_dir().join("pc-ml-a");
        add_search_dir(a.clone());
        add_search_dir(a.clone());
        let dirs = search_dirs();
        assert_eq!(dirs.iter().filter(|d| **d == a).count(), 1);
        // The executable's folder comes before the added ones.
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        if let Some(exe) = exe {
            let (e, x) = (dirs.iter().position(|d| *d == exe), dirs.iter().position(|d| *d == a));
            assert!(e < x, "{dirs:?}");
        }
    }

    #[test]
    fn a_file_is_found_in_the_first_folder_that_has_it() {
        let root = std::env::temp_dir().join(format!("pc-ml-find-{}", std::process::id()));
        let (one, two) = (root.join("one"), root.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(two.join("b.onnx"), b"x").unwrap();
        std::fs::write(one.join("c.onnx"), b"x").unwrap();
        let dirs = vec![one.clone(), two.clone()];
        assert_eq!(find_in(&dirs, &["a.onnx", "b.onnx"]), Some(two.join("b.onnx")));
        assert_eq!(find_in(&dirs, &["c.onnx", "b.onnx"]), Some(one.join("c.onnx")));
        assert_eq!(find_in(&dirs, &["d.onnx"]), None);
        assert!(list(&dirs).contains("two"));
        assert_eq!(list(&[]), "no folder");
        let _ = std::fs::remove_dir_all(root);
    }
}
