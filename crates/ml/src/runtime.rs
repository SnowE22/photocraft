//! Finding and starting ONNX Runtime (native builds).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::MlError;

/// The runtime library's file name.
#[cfg(windows)]
pub const LIBRARY: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "libonnxruntime.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
pub const LIBRARY: &str = "libonnxruntime.so";
#[cfg(not(any(windows, unix)))]
pub const LIBRARY: &str = "onnxruntime";

/// The runtime library: `ORT_DYLIB_PATH` (ONNX Runtime's own convention) when it names a file,
/// else the first search folder that has one.
pub fn library(dirs: &[PathBuf]) -> Result<PathBuf, MlError> {
    if let Some(p) = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    crate::find_in(dirs, &[LIBRARY]).ok_or_else(|| MlError::RuntimeMissing { file: LIBRARY.into(), searched: crate::list(dirs) })
}

/// The process's ONNX Runtime environment, started from `library` the first time. The outcome is
/// kept either way: a process loads ONNX Runtime once.
pub fn environment(library: &Path) -> Result<ort::environment::Environment, MlError> {
    static ENV: OnceLock<Result<ort::environment::Environment, MlError>> = OnceLock::new();
    ENV.get_or_init(|| {
        // ONNX Runtime's DirectML provider loads `DirectML.dll` by name, which finds Windows' own
        // older copy in System32 unless the one shipped beside the runtime is loaded first.
        #[cfg(windows)]
        if let Some(dml) = library.parent().map(|d| d.join("DirectML.dll")).filter(|p| p.is_file())
            && let Err(e) = ort::util::preload_dylib(dml.as_os_str())
        {
            log::warn!("[ml] {} didn't load: {e}", dml.display());
        }
        let builder = ort::init_from(library).map_err(|e| MlError::Runtime(format!("{}: {e}", library.display())))?;
        builder.with_name("PhotoCraft").build().map_err(|e| MlError::Runtime(e.to_string()))
    })
    .clone()
}
