"""Export a Stable Diffusion XL inpainting checkpoint (diffusers layout) to the ONNX files
PhotoCraft's diffusion fill loads (photocraft-ml `diffusion`, `"kind": "sdxl-inpaint"`).

    python export_sdxl_inpaint.py <hf-repo-or-folder> <out-dir> [--int8] [--vae <repo>] [--style general|anime] [--cpu]

Writes:
- text_encoder.onnx (+ .data, fp32, run on the CPU): both of SDXL's text encoders in one graph.
  It takes CLIP token ids padded with the end token (`input_ids`, batch × 77) and gives the two
  encoders' penultimate hidden states side by side (`prompt_embeds`, batch × 77 × 2048) and the
  second encoder's projected pooled output (`text_embeds`, batch × 1280). The second encoder's
  own padding token (after the first end token) is put in inside the graph.
- unet16.onnx (+ .data): the 9-channel inpainting UNet traced in fp16 (fp32 inputs and outputs),
  for the GPU, with SDXL's extra inputs `text_embeds` and `time_ids` (batch × 6).
- unet8.onnx (+ .data, 2.6 GB): only with `--int8`; unet16 with its large weights in int8, one
  scale per output channel, expanded on the GPU one layer at a time. What an 8 GB GPU needs.
- unet.onnx (+ .data, fp32, 10 GB): only with `--cpu`; without it the model is GPU only.
- vae_encoder.onnx / vae_decoder.onnx (fp32, the CPU) and vae16_encoder.onnx /
  vae16_decoder.onnx (fp16, the GPU), from `--vae` (default madebyollin/sdxl-vae-fp16-fix, MIT:
  SDXL's VAE fine-tuned so it doesn't overflow in half precision; the same latent space).
- vocab.json, merges.txt and model.json.

Runs on the CPU (PyTorch's CPU fp16 kernels trace the half-precision networks); nothing here
needs a GPU. Downloads about 7 GB (the fp16 weights) and needs about 20 GB of RAM.
"""

import argparse
import json
import os
import shutil

import torch

from huggingface_hub import HfApi, snapshot_download

torch.set_grad_enabled(False)
OPSET = 17


def fetch(repo, subfolders):
    """A local folder for `repo`: the fp16 weights where the repo has them, else the full ones."""
    if os.path.isdir(repo):
        return repo
    files = HfApi().list_repo_files(repo)
    allow = ["model_index.json", "*.json", "*.txt"]
    for sub in subfolders:
        # "" is the repo's top level (a VAE-only repo).
        prefix = sub + "/" if sub else ""
        inside = [f for f in files if f.startswith(prefix) and "/" not in f[len(prefix):]]
        fp16 = [f for f in inside if f.endswith(".fp16.safetensors")]
        full = [f for f in inside if f.endswith(".safetensors") and ".fp16." not in f]
        allow += fp16 if fp16 else full
    return snapshot_download(repo, allow_patterns=allow)


def load(cls, src, sub, dtype):
    folder = os.path.join(src, sub) if sub else src
    variant = "fp16" if any(f.endswith(".fp16.safetensors") for f in os.listdir(folder)) else None
    kwargs = {"subfolder": sub} if sub else {}
    return cls.from_pretrained(src, variant=variant, torch_dtype=dtype, **kwargs).eval()


def export(module, args, path, input_names, output_names, dynamic_axes):
    """Export with all weights in one `<name>.data` file next to the graph; a graph already
    exported (an earlier, interrupted run) is kept."""
    import onnx

    out_dir, name = os.path.split(path)
    if os.path.isfile(path) and os.path.isfile(path + ".data"):
        print(name, "already exported", flush=True)
        return
    raw = os.path.join(out_dir, "_raw_" + name)
    os.makedirs(raw, exist_ok=True)
    torch.onnx.export(module, args, os.path.join(raw, name), external_data=True, input_names=input_names,
                      output_names=output_names, dynamic_axes=dynamic_axes, opset_version=OPSET, dynamo=False)
    m = onnx.load(os.path.join(raw, name), load_external_data=True)
    onnx.save_model(m, path, save_as_external_data=True, all_tensors_to_one_file=True, location=name + ".data", size_threshold=1024)
    del m
    shutil.rmtree(raw, ignore_errors=True)
    print(name, "done", flush=True)


def quantize_int8(src_path, dst_path, min_elements=65536):
    """`unet16.onnx` with its large MatMul, Gemm and Conv weights stored as int8 (one scale per
    output channel): each weight becomes DequantizeLinear -> Cast(fp16) in the graph, so the GPU
    holds 1 byte per weight (2.6 GB instead of 5.1) and expands one layer at a time. Activations
    stay fp16."""
    import numpy as np
    import onnx
    from onnx import TensorProto, helper, numpy_helper

    out_dir, name = os.path.split(dst_path)
    if os.path.isfile(dst_path) and os.path.isfile(dst_path + ".data"):
        print(name, "already exported", flush=True)
        return
    m = onnx.load(src_path, load_external_data=True)
    g = m.graph
    inits = {i.name: i for i in g.initializer}
    # Which input is a weight, and its output-channel axis: MatMul B is K x N (axis 1), Gemm B
    # is N x K with transB (axis 0) or K x N (axis 1), Conv W is O x I x kh x kw (axis 0).
    axis_of = {}
    for node in g.node:
        if node.op_type in ("MatMul", "Conv", "Gemm") and len(node.input) > 1 and node.input[1] in inits:
            w = inits[node.input[1]]
            if w.data_type != TensorProto.FLOAT16 or len(w.dims) < 2:
                continue
            if node.op_type == "MatMul":
                axis = len(w.dims) - 1
            elif node.op_type == "Gemm":
                trans_b = next((a.i for a in node.attribute if a.name == "transB"), 0)
                axis = 0 if trans_b else 1
            else:
                axis = 0
            if axis_of.setdefault(w.name, axis) != axis:
                axis_of[w.name] = None  # used two ways: leave it alone
    new_inits, new_nodes, done = [], [], 0
    for wname, axis in axis_of.items():
        if axis is None:
            continue
        w = numpy_helper.to_array(inits[wname]).astype(np.float32)
        if w.size < min_elements:
            continue
        reduce = tuple(i for i in range(w.ndim) if i != axis)
        scale = np.abs(w).max(axis=reduce) / 127.0
        scale = np.where(scale > 0, scale, 1.0).astype(np.float32)
        shape = [1] * w.ndim
        shape[axis] = -1
        q = np.clip(np.rint(w / scale.reshape(shape)), -127, 127).astype(np.int8)
        new_inits += [numpy_helper.from_array(q, wname + "_q8"), numpy_helper.from_array(scale, wname + "_s8"),
                      numpy_helper.from_array(np.zeros_like(scale, dtype=np.int8), wname + "_z8")]
        new_nodes += [helper.make_node("DequantizeLinear", [wname + "_q8", wname + "_s8", wname + "_z8"], [wname + "_f32"], axis=axis, name=wname + "_dq8"),
                      helper.make_node("Cast", [wname + "_f32"], [wname], to=TensorProto.FLOAT16, name=wname + "_cast16")]
        g.initializer.remove(inits[wname])
        done += 1
    g.initializer.extend(new_inits)
    # The dequantizing nodes first, so the graph stays topologically sorted.
    nodes = new_nodes + list(g.node)
    del g.node[:]
    g.node.extend(nodes)
    onnx.save_model(m, dst_path, save_as_external_data=True, all_tensors_to_one_file=True, location=name + ".data", size_threshold=1024)
    print(name, "done:", done, "weights in int8", flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("out")
    ap.add_argument("--vae", default="madebyollin/sdxl-vae-fp16-fix")
    ap.add_argument("--style", default="general", choices=["general", "anime"])
    ap.add_argument("--cpu", action="store_true", help="also export the fp32 UNet (10 GB) so the model can run on the CPU")
    ap.add_argument("--int8", action="store_true", help="also write unet8.onnx (int8 weights, 2.6 GB of graphics memory instead of 5.1), for 8 GB GPUs")
    a = ap.parse_args()
    out = a.out
    os.makedirs(out, exist_ok=True)
    src = fetch(a.src, ["text_encoder", "text_encoder_2", "tokenizer", "tokenizer_2", "unet", "scheduler"])
    from diffusers import AutoencoderKL, UNet2DConditionModel
    from transformers import CLIPTextModel, CLIPTextModelWithProjection

    # The second tokenizer pads with its own token ("!" in SDXL) instead of the end token.
    with open(os.path.join(src, "tokenizer", "vocab.json"), encoding="utf-8") as f:
        vocab = json.load(f)
    with open(os.path.join(src, "tokenizer_2", "special_tokens_map.json"), encoding="utf-8") as f:
        pad = json.load(f).get("pad_token", "<|endoftext|>")
    pad = pad.get("content") if isinstance(pad, dict) else pad
    eos_id, pad_id = vocab["<|endoftext|>"], vocab[pad]

    # Text encoders: token ids -> (penultimate hidden states of both, pooled projection of the 2nd).
    te1 = load(CLIPTextModel, src, "text_encoder", torch.float32)
    te2 = load(CLIPTextModelWithProjection, src, "text_encoder_2", torch.float32)

    class TE(torch.nn.Module):
        def __init__(self, t1, t2):
            super().__init__()
            self.t1, self.t2 = t1, t2

        def forward(self, input_ids):
            h1 = self.t1(input_ids=input_ids, output_hidden_states=True, return_dict=True).hidden_states[-2]
            # Positions after the first end token get the second tokenizer's padding token.
            pos = torch.arange(input_ids.shape[1], device=input_ids.device).unsqueeze(0)
            first_end = (input_ids == eos_id).int().argmax(dim=1, keepdim=True)
            ids2 = torch.where(pos > first_end, torch.full_like(input_ids, pad_id), input_ids)
            o2 = self.t2(input_ids=ids2, output_hidden_states=True, return_dict=True)
            return torch.cat([h1, o2.hidden_states[-2]], dim=-1), o2.text_embeds

    ids = torch.full((1, 77), eos_id, dtype=torch.int64)
    export(TE(te1, te2), (ids,), f"{out}/text_encoder.onnx", ["input_ids"], ["prompt_embeds", "text_embeds"],
           {"input_ids": {0: "b"}, "prompt_embeds": {0: "b"}, "text_embeds": {0: "b"}})
    hidden = te1.config.hidden_size + te2.config.hidden_size
    pooled = te2.config.projection_dim
    del te1, te2

    # VAE encoder (the latent distribution's mean) and decoder, fp32 and fp16.
    vae_src = fetch(a.vae, [""])

    class Enc(torch.nn.Module):
        def __init__(self, v, dtype):
            super().__init__()
            self.v, self.dtype = v, dtype

        def forward(self, image):
            return self.v.encode(image.to(self.dtype)).latent_dist.mean.float()

    class Dec(torch.nn.Module):
        def __init__(self, v, dtype):
            super().__init__()
            self.v, self.dtype = v, dtype

        def forward(self, latent):
            return self.v.decode(latent.to(self.dtype), return_dict=False)[0].float()

    # Traced at a small size (the sizes stay dynamic): PyTorch's CPU fp16 convolutions are very
    # slow, and a 256-pixel trace took over 20 minutes.
    scaling = 0.13025
    for dtype, prefix in ((torch.float32, "vae"), (torch.float16, "vae16")):
        vae = load(AutoencoderKL, vae_src, "", dtype)
        scaling = float(getattr(vae.config, "scaling_factor", scaling))
        export(Enc(vae, dtype), (torch.zeros((1, 3, 64, 64)),), f"{out}/{prefix}_encoder.onnx", ["image"], ["latent"],
               {"image": {0: "b", 2: "h", 3: "w"}, "latent": {0: "b", 2: "h8", 3: "w8"}})
        export(Dec(vae, dtype), (torch.zeros((1, 4, 8, 8)),), f"{out}/{prefix}_decoder.onnx", ["latent"], ["image"],
               {"latent": {0: "b", 2: "h8", 3: "w8"}, "image": {0: "b", 2: "h", 3: "w"}})
        del vae

    # UNet with SDXL's added conditioning (pooled text and the six size/crop numbers).
    def export_unet(dtype, name):
        unet = load(UNet2DConditionModel, src, "unet", dtype)

        class U(torch.nn.Module):
            def __init__(self, u):
                super().__init__()
                self.u = u

            def forward(self, sample, timestep, encoder_hidden_states, text_embeds, time_ids):
                cond = {"text_embeds": text_embeds.to(dtype), "time_ids": time_ids.to(dtype)}
                y = self.u(sample.to(dtype), timestep, encoder_hidden_states.to(dtype), added_cond_kwargs=cond, return_dict=False)[0]
                return y.float()

        b = 1
        args = (torch.zeros((b, unet.config.in_channels, 32, 32)), torch.tensor([999.0] * b), torch.zeros((b, 77, hidden)),
                torch.zeros((b, pooled)), torch.tensor([[256.0, 256.0, 0.0, 0.0, 256.0, 256.0]] * b))
        export(U(unet), args, f"{out}/{name}.onnx", ["sample", "timestep", "encoder_hidden_states", "text_embeds", "time_ids"], ["noise_pred"],
               {"sample": {0: "b", 2: "h", 3: "w"}, "timestep": {0: "b"}, "encoder_hidden_states": {0: "b"}, "text_embeds": {0: "b"},
                "time_ids": {0: "b"}, "noise_pred": {0: "b", 2: "h", 3: "w"}})
        in_ch = unet.config.in_channels
        del unet
        return in_ch

    in_ch = export_unet(torch.float16, "unet16")
    if a.int8:
        quantize_int8(f"{out}/unet16.onnx", f"{out}/unet8.onnx")
    if a.cpu:
        export_unet(torch.float32, "unet")

    for f in ("vocab.json", "merges.txt"):
        shutil.copy(os.path.join(src, "tokenizer", f), os.path.join(out, f))
    with open(os.path.join(src, "scheduler", "scheduler_config.json"), encoding="utf-8") as f:
        sched = json.load(f)
    meta = {
        "kind": "sdxl-inpaint",
        "source": a.src,
        "vae_source": a.vae,
        "style": a.style,
        "side": 1024,
        "in_channels": in_ch,
        "hidden_size": hidden,
        "pooled_size": pooled,
        "vae_scaling_factor": scaling,
        "beta_start": sched.get("beta_start", 0.00085),
        "beta_end": sched.get("beta_end", 0.012),
        "beta_schedule": sched.get("beta_schedule", "scaled_linear"),
        "num_train_timesteps": sched.get("num_train_timesteps", 1000),
        "prediction_type": sched.get("prediction_type", "epsilon"),
        "steps_offset": sched.get("steps_offset", 1),
    }
    with open(os.path.join(out, "model.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
    print("EXPORT_DONE", json.dumps(meta), flush=True)


if __name__ == "__main__":
    main()
