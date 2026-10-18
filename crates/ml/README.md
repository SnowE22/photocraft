# photocraft-ml

Local machine-learning models for PhotoCraft (layer L4). One model so far: **LaMa** inpainting,
the Remove tool's Generative AI mode.

## Public API

| Item | What it does |
|---|---|
| `lama::state()` | `Missing` / `Idle` / `Loading` / `Ready(Device)` / `Failed(msg)`, cheap, for the UI |
| `lama::preload()` | Loads the model on a background thread (once; DirectML takes ~12 s) |
| `lama::inpaint(rgb, mask)` | One 512 × 512 pass: planar RGB 0–1 and a mask (1 = fill) in, planar RGB 0–1 out |
| `lama::model_file()` | The model file, or `MlError::ModelMissing` with the folders searched |
| `inpaint::fill(req, read, model, ctl)` | A document region with a hole to the model and back: a crop per group of hole pixels, area-averaged to 512, the fill scaled back into the hole only. Pure, runs everywhere |
| `onnx_edit::for_directml(bytes)` | Rewrites LaMa's 2-D × 5-D Fourier MatMuls as 4-D ones so DirectML takes the model (on the protobuf bytes) |
| `add_search_dir(dir)`, `search_dirs()` | Where models and the runtime are looked for |

Every failure is an `MlError` value; nothing panics. WebAssembly builds have no runtime:
`state()` is `Missing` and `inpaint` returns `MlError::Unsupported`.

## Runtime and model (not committed)

ONNX Runtime is loaded at run time (`ort`'s `load-dynamic`): nothing links at build time. It and
the model are looked for in `PHOTOCRAFT_MODEL_DIR`, the executable's `models` folder, the
executable's folder, then the folders the app adds (its settings folder's `models`).
`ORT_DYLIB_PATH` names the runtime library directly.

| File | From | Licence |
|---|---|---|
| `lama_fp32.onnx` (208 MB) | [Carve/LaMa-ONNX](https://huggingface.co/Carve/LaMa-ONNX), an export of [advimman/lama](https://github.com/advimman/lama) big-lama | Apache-2.0 |
| `onnxruntime.dll`, `onnxruntime_providers_shared.dll` | the `onnxruntime-directml` package (1.17 or later) | MIT |
| `DirectML.dll` | the same package | Microsoft DirectML redistributable licence |

`packaging/models/README.md` has the install steps. On Windows the session runs on DirectML on the
high-performance GPU and falls back to the CPU provider; elsewhere it runs on the CPU.

## Tests

`cargo test -p photocraft-ml` runs everything that needs no model. With the model and runtime in
`PHOTOCRAFT_MODEL_DIR`, `cargo test -p photocraft-ml -- --ignored` also runs the real model.
