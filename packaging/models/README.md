# Local AI models (not committed, not bundled)

PhotoCraft runs one local model so far: **LaMa**, the Remove tool's Generative AI mode (options bar
› Mode: Auto / Generative AI On / Generative AI Off). It runs on this computer through
**ONNX Runtime**, on Windows on the GPU through **DirectML** (any DirectX 12 GPU), otherwise on the
CPU. Neither the model nor the runtime is in this repository or in a release: PhotoCraft loads them
at run time when they are installed, and without them the Remove tool uses its patch fill (Auto)
or says what is missing (Generative AI On). The options bar shows "AI model not installed" with
the folders it looked in.

## Install (Windows)

Put these files in a `models` folder next to `photocraft.exe` (or next to the executable itself,
or in the settings folder's `models`, `%APPDATA%\Photocraft\models`):

| File | Size | Where to get it | Licence |
|---|---|---|---|
| `lama_fp32.onnx` | 208 MB | [Carve/LaMa-ONNX](https://huggingface.co/Carve/LaMa-ONNX) (`lama_fp32.onnx`), an export of big-lama from [advimman/lama](https://github.com/advimman/lama) | Apache-2.0 |
| `onnxruntime.dll`, `onnxruntime_providers_shared.dll` | 21 MB | the [`onnxruntime-directml`](https://pypi.org/project/onnxruntime-directml/) package (1.17 or later; checked with 1.24.4), folder `onnxruntime/capi/` of the wheel (a zip) | MIT |
| `DirectML.dll` | 18 MB | the same folder of the same package | [Microsoft DirectML licence](https://www.nuget.org/packages/Microsoft.AI.DirectML/) (redistributable) |

Keep the licence texts next to them (`LICENSE` and `ThirdPartyNotices.txt` from the wheel, the
model card's Apache-2.0 notice).

`PHOTOCRAFT_MODEL_DIR` names another folder to look in first; `ORT_DYLIB_PATH` names the runtime
library directly (ONNX Runtime's own convention).

## What happens at run time

- Picking the Remove tool starts loading the model in the background (about 12–15 s the first time
  on an RTX 5070: the graph is rewritten for DirectML in memory and compiled for the GPU), unless
  Mode is Generative AI Off. The options bar says "Loading the AI model…" meanwhile.
- A removal crops a square of at least 512 pixels around each part of the stroke, scales it to the
  model's 512 × 512 and back, and changes only the stroke's pixels: about 0.2 s per part on the GPU,
  several seconds on the CPU.
- Auto uses the model for layer pixels of RGB and Grayscale images with opaque surroundings and the
  patch fill otherwise (masks, channels, CMYK, Lab, transparent areas); the command's result says
  which (`"fill": "generative"` or `"patch"`).

Other platforms: the same files from the plain `onnxruntime` package for macOS
(`libonnxruntime.dylib`) or Linux (`libonnxruntime.so`); the model then runs on the CPU.
