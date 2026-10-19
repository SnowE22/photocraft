"""Export a Stable Diffusion 1.x inpainting checkpoint (diffusers layout) to the ONNX files
PhotoCraft's diffusion fill loads (photocraft-ml `diffusion`).

    python export_sd_inpaint.py <hf-repo-or-folder> <out-dir>

Writes text_encoder.onnx, unet.onnx (+ unet.onnx.data), vae_encoder.onnx and vae_decoder.onnx
(fp32), unet16.onnx (+ .data: the UNet traced in fp16, fp32 inputs and outputs, for the GPU), the
CLIP tokenizer's vocab.json and merges.txt, and model.json. Runs on the CPU (PyTorch's CPU fp16
kernels trace the half-precision UNet); nothing here needs a GPU.
"""

import json
import os
import shutil
import sys

import torch

from huggingface_hub import snapshot_download

torch.set_grad_enabled(False)
OPSET = 17


def main():
    src, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    if not os.path.isdir(src):
        src = snapshot_download(
            src,
            allow_patterns=["model_index.json", "scheduler/*", "text_encoder/*", "tokenizer/*", "unet/*", "vae/*"],
            ignore_patterns=["*.bin", "*.ckpt", "*.msgpack", "*.h5", "*non_ema*"],
        )
    from diffusers import AutoencoderKL, UNet2DConditionModel
    from transformers import CLIPTextModel

    def load_dtype(cls, sub, dtype):
        variant = "fp16" if any(f.endswith(".fp16.safetensors") for f in os.listdir(os.path.join(src, sub))) else None
        return cls.from_pretrained(src, subfolder=sub, variant=variant, torch_dtype=dtype).eval()

    def load(cls, sub):
        return load_dtype(cls, sub, torch.float32)

    # Text encoder: token ids -> hidden states.
    te = load(CLIPTextModel, "text_encoder")

    class TE(torch.nn.Module):
        def __init__(self, m):
            super().__init__()
            self.m = m

        def forward(self, input_ids):
            return self.m(input_ids=input_ids, return_dict=False)[0]

    ids = torch.zeros((1, 77), dtype=torch.int64)
    torch.onnx.export(TE(te), (ids,), f"{out}/text_encoder.onnx", input_names=["input_ids"], output_names=["last_hidden_state"],
                      dynamic_axes={"input_ids": {0: "batch"}, "last_hidden_state": {0: "batch"}}, opset_version=OPSET, dynamo=False)
    print("text encoder done", flush=True)
    hidden = te.config.hidden_size
    del te

    # VAE encoder (the latent distribution's mean) and decoder.
    vae = load(AutoencoderKL, "vae")

    class Enc(torch.nn.Module):
        def __init__(self, v):
            super().__init__()
            self.v = v

        def forward(self, image):
            return self.v.encode(image).latent_dist.mean

    class Dec(torch.nn.Module):
        def __init__(self, v):
            super().__init__()
            self.v = v

        def forward(self, latent):
            return self.v.decode(latent, return_dict=False)[0]

    img = torch.zeros((1, 3, 512, 512))
    torch.onnx.export(Enc(vae), (img,), f"{out}/vae_encoder.onnx", input_names=["image"], output_names=["latent"],
                      dynamic_axes={"image": {0: "b", 2: "h", 3: "w"}, "latent": {0: "b", 2: "h8", 3: "w8"}}, opset_version=OPSET, dynamo=False)
    lat = torch.zeros((1, 4, 64, 64))
    torch.onnx.export(Dec(vae), (lat,), f"{out}/vae_decoder.onnx", input_names=["latent"], output_names=["image"],
                      dynamic_axes={"latent": {0: "b", 2: "h8", 3: "w8"}, "image": {0: "b", 2: "h", 3: "w"}}, opset_version=OPSET, dynamo=False)
    scaling = float(getattr(vae.config, "scaling_factor", 0.18215))
    print("vae done", flush=True)
    del vae

    # UNet: fp32 (`unet.onnx`, for the CPU and as a fallback) and fp16 traced in half precision
    # (`unet16.onnx`, for the GPU: about twice as fast, half the memory). Converting the fp32
    # graph to fp16 afterwards (onnxconverter-common) left nodes with mixed fp16/fp32 inputs.
    import onnx

    def export_unet(dtype, name):
        unet = load_dtype(UNet2DConditionModel, "unet", dtype)

        class U(torch.nn.Module):
            def __init__(self, u):
                super().__init__()
                self.u = u

            def forward(self, sample, timestep, encoder_hidden_states):
                y = self.u(sample.to(dtype), timestep, encoder_hidden_states.to(dtype), return_dict=False)[0]
                return y.float()

        raw = os.path.join(out, "_raw_" + name)
        os.makedirs(raw, exist_ok=True)
        sample = torch.zeros((2, unet.config.in_channels, 64, 64))
        t = torch.tensor([999.0, 999.0])
        ehs = torch.zeros((2, 77, unet.config.cross_attention_dim))
        torch.onnx.export(U(unet), (sample, t, ehs), os.path.join(raw, name + ".onnx"), external_data=True,
                          input_names=["sample", "timestep", "encoder_hidden_states"], output_names=["noise_pred"],
                          dynamic_axes={"sample": {0: "b", 2: "h", 3: "w"}, "timestep": {0: "b"}, "encoder_hidden_states": {0: "b"}, "noise_pred": {0: "b", 2: "h", 3: "w"}},
                          opset_version=OPSET, dynamo=False)
        # One weights file next to the graph instead of one per tensor.
        m = onnx.load(os.path.join(raw, name + ".onnx"), load_external_data=True)
        onnx.save_model(m, os.path.join(out, name + ".onnx"), save_as_external_data=True, all_tensors_to_one_file=True, location=name + ".onnx.data", size_threshold=1024)
        shutil.rmtree(raw, ignore_errors=True)
        in_ch = unet.config.in_channels
        del unet, m
        print(name, "done", flush=True)
        return in_ch

    in_ch = export_unet(torch.float32, "unet")
    export_unet(torch.float16, "unet16")

    for f in ("vocab.json", "merges.txt"):
        shutil.copy(os.path.join(src, "tokenizer", f), os.path.join(out, f))
    with open(os.path.join(src, "scheduler", "scheduler_config.json")) as f:
        sched = json.load(f)
    meta = {
        "kind": "sd-inpaint",
        "source": sys.argv[1],
        "in_channels": in_ch,
        "hidden_size": hidden,
        "vae_scaling_factor": scaling,
        "beta_start": sched.get("beta_start", 0.00085),
        "beta_end": sched.get("beta_end", 0.012),
        "beta_schedule": sched.get("beta_schedule", "scaled_linear"),
        "num_train_timesteps": sched.get("num_train_timesteps", 1000),
        "prediction_type": sched.get("prediction_type", "epsilon"),
        "steps_offset": sched.get("steps_offset", 1),
    }
    with open(os.path.join(out, "model.json"), "w") as f:
        json.dump(meta, f, indent=2)
    print("EXPORT_DONE", json.dumps(meta), flush=True)


if __name__ == "__main__":
    main()
