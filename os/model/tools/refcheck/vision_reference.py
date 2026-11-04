"""The reference for the engine's vision part (vision.rs): Qwen 3.5's picture processor and vision model as
transformers runs them (float32, on the processor), for a picture drawn here. Writes, beside OUT_PREFIX:

  .rgb          the picture's pixels (RGB bytes, row after row), and .size its "width height"
  .resized.rgb  the processor's resized pixels (what torchvision's bicubic resampling made), .grid "t h w"
  .patches.f32  the processor's patches (float32, as the vision model reads them)
  .rows.f32     the vision model's rows for the language model (float32, one per 2x2 group of patches)

    python vision_reference.py MODEL_FOLDER OUT_PREFIX [--most-rows N] [--picture FILE]

--most-rows: at most this many rows (the processor's max_pixels set to N x 32 x 32), as the engine is asked.
--picture: a picture of one's own (else one is drawn)."""
import json
import struct
import sys

import numpy as np
import torch
from PIL import Image, ImageDraw
from safetensors import safe_open
from transformers import AutoImageProcessor
from transformers.models.qwen3_5 import modeling_qwen3_5 as M
from transformers.models.qwen3_5.configuration_qwen3_5 import Qwen3_5VisionConfig

folder, out = sys.argv[1], sys.argv[2]
args = sys.argv[3:]
most_rows = int(args[args.index("--most-rows") + 1]) if "--most-rows" in args else None
if "--picture" in args:
    picture = Image.open(args[args.index("--picture") + 1]).convert("RGB")
else:
    # A picture with what pictures have: gradients, edges, shapes, words.
    picture = Image.new("RGB", (500, 375))
    px = picture.load()
    for y in range(375):
        for x in range(500):
            px[x, y] = (x * 255 // 499, y * 255 // 374, (x + y) * 255 // 873)
    d = ImageDraw.Draw(picture)
    d.ellipse((40, 40, 200, 200), fill=(250, 220, 90))
    d.rectangle((260, 60, 460, 160), fill=(30, 60, 160), outline=(255, 255, 255), width=5)
    d.polygon([(300, 330), (380, 200), (460, 330)], fill=(200, 30, 40))
    d.text((60, 260), "Chat Computer 42", fill=(0, 0, 0))
    d.line((0, 374, 499, 0), fill=(255, 255, 255), width=3)

open(out + ".rgb", "wb").write(np.asarray(picture, dtype=np.uint8).tobytes())
open(out + ".size", "w").write(f"{picture.width} {picture.height}")

kwargs = {}
if most_rows:
    pre = json.load(open(folder + "/preprocessor_config.json"))
    kwargs["size"] = {"shortest_edge": min(pre["size"]["shortest_edge"], most_rows * 32 * 32), "longest_edge": most_rows * 32 * 32}
proc = AutoImageProcessor.from_pretrained(folder, **kwargs)
# The resized pixels themselves (the processor resizes, then scales and cuts into patches).
from transformers.models.qwen2_vl.image_processing_qwen2_vl import smart_resize
import torchvision.transforms.v2.functional as tvF
tensor = torch.from_numpy(np.asarray(picture, dtype=np.uint8).copy()).permute(2, 0, 1)
h, w = smart_resize(picture.height, picture.width, factor=32, min_pixels=proc.size["shortest_edge"], max_pixels=proc.size["longest_edge"])
resized = tvF.resize(tensor, [h, w], interpolation=tvF.InterpolationMode.BICUBIC, antialias=True)
open(out + ".resized.rgb", "wb").write(resized.permute(1, 2, 0).contiguous().numpy().tobytes())

batch = proc(images=[picture], return_tensors="pt")
pixel_values, grid = batch["pixel_values"].float(), batch["image_grid_thw"]
open(out + ".grid", "w").write(" ".join(str(int(v)) for v in grid[0]))
open(out + ".patches.f32", "wb").write(pixel_values.numpy().astype("<f4").tobytes())

cfg = json.load(open(folder + "/config.json"))
vcfg = Qwen3_5VisionConfig(**cfg["vision_config"])
vcfg._attn_implementation = "eager"
model = M.Qwen3_5VisionModel(vcfg).float().eval()
state = {}
with safe_open(folder + "/model.safetensors-00001-of-00001.safetensors", framework="pt") as f:
    for name in f.keys():
        if name.startswith("model.visual."):
            state[name[len("model.visual."):]] = f.get_tensor(name).float()
missing, unexpected = model.load_state_dict(state, strict=False)
assert not unexpected and all("rotary" in m for m in missing), (missing, unexpected)
with torch.no_grad():
    rows = model(pixel_values, grid_thw=grid).pooler_output
open(out + ".rows.f32", "wb").write(rows.numpy().astype("<f4").tobytes())
print(f"picture {picture.width}x{picture.height} read at {w}x{h}: grid {grid[0].tolist()}, {rows.shape[0]} rows of {rows.shape[1]}")
