# Local AI models (not committed, not bundled)

PhotoCraft runs two local models: **LaMa**, the Remove tool's fast Generative AI fill (options bar ›
Mode: Auto / Generative AI On / Generative AI Off, Model: LaMa), and a **Stable Diffusion
inpainting** model, slower but able to draw what was behind (options bar › Model: Diffusion, and
Edit › Generative Fill…, which also extends a picture past its edge). It runs on this computer through
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
- Options bar › Quality: High runs LaMa at 1024 × 1024 instead (the model is made size-free when
  it loads), so a hole up to 896 pixels across is filled 1:1: large removals come out sharper
  (on an RTX 5070 Laptop a 700 × 420 hole kept 61 % of its surroundings' sharpness instead of 43 %,
  1.3 s), at about four times the work per run.
- Auto uses the model for layer pixels of RGB and Grayscale images with opaque surroundings and the
  patch fill otherwise (masks, channels, CMYK, Lab, transparent areas); the command's result says
  which (`"fill": "generative"` or `"patch"`).

## The diffusion model (Remove tool › Model: Diffusion, Edit › Generative Fill)

An **inpainting** checkpoint exported to ONNX, in its own folder inside `models`: Stable
Diffusion XL (recommended, any kind of picture) or Stable Diffusion 1.x (smaller, for anime and
manga). PhotoCraft takes `sdxl-inpainting` first, then `anything-4.0-inpainting`, else the first
folder whose `model.json` says `sdxl-inpaint` or `sd-inpaint`. `PHOTOCRAFT_DIFFUSION_MODEL` picks
one (a folder, or a folder's name in a models folder), for example `anything-4.0-inpainting` to
keep the anime model for manga while both are installed.

### Stable Diffusion XL inpainting (recommended)

[SDXL 1.0 inpainting 0.1](https://huggingface.co/diffusers/stable-diffusion-xl-1.0-inpainting-0.1)
(CreativeML OpenRAIL++-M, whose use restrictions apply to what you make with it) with
[SDXL-VAE-FP16-Fix](https://huggingface.co/madebyollin/sdxl-vae-fp16-fix) (MIT, SDXL's VAE made
safe in half precision). It works on 1024 × 1024 crops instead of 512 and is trained on
photographs and art of every kind, where Anything 4.0 turns a photo's hole into anime art. It
needs a GPU with **8 GB** of graphics memory (with `--int8`, about 5.2 GB while filling) and
about 4 GB of RAM for its text encoders and VAE, which run on the CPU. Export it once (CPU only,
about 15 minutes, 7 GB download, about 20 GB of RAM):

```sh
.venv/Scripts/python packaging/models/export_sdxl_inpaint.py diffusers/stable-diffusion-xl-1.0-inpainting-0.1 <models>/sdxl-inpainting --int8
```

The folder then holds `model.json`, `text_encoder.onnx` (both CLIP encoders in one graph, fp32,
3.2 GB), `unet16.onnx` (fp16, 5.1 GB), with `--int8` `unet8.onnx` (the same UNet with its weights
stored in int8 and expanded one layer at a time, 2.6 GB), `vae_encoder.onnx`/`vae_decoder.onnx`
(fp32) and `vae16_encoder.onnx`/`vae16_decoder.onnx` (fp16), each with its `.data` weights, plus
`vocab.json` and `merges.txt`. The GPU takes `unet8.onnx` when it is there, else `unet16.onnx`
(fine on a 12 GB GPU; on 8 GB it doesn't fit next to the desktop and Windows pages graphics
memory, which makes a fill take over ten minutes). Without `--cpu` there is no fp32 UNet and the
model runs on the GPU only; `--cpu` adds `unet.onnx` (10 GB). `--style anime` gives an anime
SDXL checkpoint the anime default prompts. Any SDXL inpainting checkpoint in diffusers format
exports the same way.

On an RTX 5070 Laptop GPU (8 GB, DirectML, on mains power) a 470 × 290 hole took about 75 s with
the model's first load included and a 400 × 290 one about 46 s after it (Anything 4.0: 10 s each),
peaking at 6.0 GB of graphics memory with the desktop's 0.8 GB: 25 steps of the int8 UNet at
0.5 s per guidance branch, about 13–16 s to encode the crop on the CPU, then 5–12 s to decode
it. Only the hole's part of the latent is decoded (its bounding box plus 128 pixels; a whole
1024 crop takes 28–34 s), so a smaller hole decodes faster. The VAE runs on the CPU because at
1024 it holds 3.8 GB of graphics memory on DirectML next to the UNet's 5.2 GB (and even 256-pixel
tiles would add 0.2–0.45 GB, as the UNet keeps its memory between fills);
`PHOTOCRAFT_SDXL_VAE=gpu` keeps it on a GPU with room for it. On battery a fill takes several
times longer (the CPU slows down). A big
hole with an empty prompt tends to get an object drawn into it (as in Photoshop's Generative
Fill); to just remove something, use the Remove tool's LaMa model.

### Stable Diffusion 1.x inpainting (Anything 4.0)

[Anything 4.0 inpainting](https://huggingface.co/Sanster/anything-4.0-inpainting)
(trained on anime and manga art; CreativeML OpenRAIL-M, whose use restrictions apply to what you
make with it). Export it once with Python 3.10–3.12 (CPU only, about 15 minutes, 2 GB download):

```sh
python -m venv .venv && .venv/Scripts/python -m pip install --index-url https://download.pytorch.org/whl/cpu torch
.venv/Scripts/python -m pip install diffusers transformers accelerate safetensors onnx huggingface_hub
.venv/Scripts/python packaging/models/export_sd_inpaint.py Sanster/anything-4.0-inpainting <models>/anything-4.0-inpainting
```

The folder then holds `model.json`, `text_encoder.onnx`, `unet.onnx` + `unet.onnx.data` (fp32,
3.4 GB: the CPU, whose provider has no fp16 kernels for it, and the GPU when there is no fp16 one),
`unet16.onnx` + `unet16.onnx.data` (traced in fp16, 1.7 GB: the GPU), `vae_encoder.onnx`,
`vae_decoder.onnx`, `vocab.json` and `merges.txt` (5.6 GB in all). Any
other SD 1.x inpainting checkpoint in diffusers format exports the same way.

Loading takes tens of seconds the first time in a run (the options bar shows it). A fill is a
512 × 512 crop around each part of the hole, 25 denoising steps (DDIM, guidance 7), then, where
the crop was shrunk, a pass over 512 tiles at full resolution that keeps the shapes and adds detail.
On an RTX 5070 Laptop GPU (DirectML, fp16) a 360 × 440 hole took 16 s with the model's first
load included and a 536 × 536 one 12.5 s after it (the full-resolution pass included); on the CPU
2–5 minutes. Each guided step runs as two short GPU calls: Windows resets
a GPU that stays busy for about 2 s (the fp32 UNet in one batch of two did that here), and if the
GPU fails anyway the run goes on on the CPU. An empty prompt uses one that suits the crop
(monochrome manga terms for a black-and-white page); the negative prompt keeps text and speech
bubbles out. `PHOTOCRAFT_ML_DEVICE=cpu` keeps both models off the GPU.

Other platforms: the same files from the plain `onnxruntime` package for macOS
(`libonnxruntime.dylib`) or Linux (`libonnxruntime.so`); the model then runs on the CPU.
