//! The Remove tool's Generative AI mode: LaMa inpainting from `photocraft-ml`, run on this computer
//! by ONNX Runtime (on Windows on the GPU through DirectML). The model and the runtime are installed
//! separately (`packaging/models/README.md`); without them the Remove tool uses its patch fill.
//!
//! These are the hooks the shell needs around `paint.remove`'s `generative` param: the model's state
//! for the options bar, loading it before the first stroke, and more folders to look for it in.

use std::path::PathBuf;

pub use photocraft_ml::Device;
pub use photocraft_ml::lama::State;

/// What the model is doing. Touches the disk while the model isn't loaded (it looks for the files),
/// so the shell polls it at most every second or so.
pub fn state() -> State {
    photocraft_ml::lama::state()
}

/// Load the model on a background thread ahead of the first removal (seconds, once per run). Does
/// nothing when it isn't installed, or is already loading or loaded.
pub fn preload() {
    photocraft_ml::lama::preload();
}

/// Also look for the model and ONNX Runtime in `dir` (the app adds its settings folder's `models`).
pub fn add_search_dir(dir: PathBuf) {
    photocraft_ml::add_search_dir(dir);
}

/// The folders searched for the model and the runtime, in order.
pub fn search_dirs() -> Vec<PathBuf> {
    photocraft_ml::search_dirs()
}
