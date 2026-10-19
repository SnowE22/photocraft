# photocraft-ml

Local machine-learning models for PhotoCraft (layer L4): **LaMa** inpainting (the Remove tool's
fast Generative AI fill) and **Stable Diffusion inpainting** (the Remove tool's Diffusion model
and Edit › Generative Fill: slower, draws what was behind, extends a picture past its edge).

## Public API

| Item | What it does |
|---|---|
| `lama::state()` | `Missing` / `Idle` / `Loading` / `Ready(Device)` / `Failed(msg)`, cheap, for the UI |
| `lama::preload()` | Loads the model on a background thread (once; DirectML takes ~12 s) |
| `lama::inpaint(rgb, mask)` | One 512 × 512 pass: planar RGB 0–1 and a mask (1 = fill) in, planar RGB 0–1 out |
| `lama::model_file()` | The model file, or `MlError::ModelMissing` with the folders searched |
| `inpaint::fill(req, read, model, ctl)` / `fill_with(…, &Options)` | A document region with a hole to the model and back: a crop per group of hole pixels (1:1 up to 384 px across, else shrunk to the model and scaled back bicubically), then the shrunk part redone at full resolution in model-sized tiles (LaMa: ring by ring from the edge; diffusion: one pass hinted with the shrunk fill), each fill corrected for drift in tone and kept neutral on black-and-white crops. Only the hole changes. Pure, runs everywhere |
| `diffusion::{state, preload, availability, inpaint, fill}` | Stable Diffusion 1.x inpainting (9-channel UNet) from an exported folder (`model.json`, text encoder, UNet, VAE encoder and decoder, CLIP vocab and merges): CLIP tokenizer, classifier-free guidance, DDIM. `fill` = `inpaint::fill_with` with `inpaint::DIFFUSION` |
| `clip_tokenizer::Tokenizer` | CLIP's byte-level BPE (77 tokens) |
| `cpu_only()` | `PHOTOCRAFT_ML_DEVICE=cpu`: never the GPU |
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
| `anything-4.0-inpainting/` (folder) | [Sanster/anything-4.0-inpainting](https://huggingface.co/Sanster/anything-4.0-inpainting), exported by `packaging/models/export_sd_inpaint.py` | CreativeML OpenRAIL-M |
| `lama_fp32.onnx` (208 MB) | [Carve/LaMa-ONNX](https://huggingface.co/Carve/LaMa-ONNX), an export of [advimman/lama](https://github.com/advimman/lama) big-lama | Apache-2.0 |
| `onnxruntime.dll`, `onnxruntime_providers_shared.dll` | the `onnxruntime-directml` package (1.17 or later) | MIT |
| `DirectML.dll` | the same package | Microsoft DirectML redistributable licence |

`packaging/models/README.md` has the install steps. On Windows the session runs on DirectML on the
high-performance GPU and falls back to the CPU provider; elsewhere it runs on the CPU.

## Tests

`cargo test -p photocraft-ml` runs everything that needs no model. With the model and runtime in
`PHOTOCRAFT_MODEL_DIR`, `cargo test -p photocraft-ml -- --ignored` also runs the real model.
