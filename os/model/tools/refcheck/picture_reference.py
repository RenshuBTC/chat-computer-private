"""The reference for a conversation with a picture: Qwen 3.5 as transformers runs it (float32, on the processor),
vision part and language model together, for the engine's (examples/picture_check.rs) to be checked against.

    python picture_reference.py MODEL_FOLDER PICTURE.rgb WIDTH HEIGHT OUT_PREFIX "a question" [--last K] [--new N]

The person's message is the picture, then the question; the model answers without thinking first. Writes, beside
OUT_PREFIX: .ids (the prompt's token ids, as the processor makes them, a line), .last.f32 (the scores at the
prompt's last K positions, float32, K by the vocabulary), and .answer (the N tokens it writes, always the likeliest,
a line)."""
import json
import sys

import numpy as np
import torch
from PIL import Image
from safetensors import safe_open
from transformers import AutoProcessor
from transformers.models.qwen3_5 import modeling_qwen3_5 as M
from transformers.models.qwen3_5.configuration_qwen3_5 import Qwen3_5Config

folder, rgb_path, width, height, out, question = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5], sys.argv[6]
args = sys.argv[7:]
last = int(args[args.index("--last") + 1]) if "--last" in args else 16
new = int(args[args.index("--new") + 1]) if "--new" in args else 24
picture = Image.frombytes("RGB", (width, height), open(rgb_path, "rb").read())

proc = AutoProcessor.from_pretrained(folder)
messages = [{"role": "user", "content": [{"type": "image"}, {"type": "text", "text": question}]}]
text = proc.apply_chat_template(messages, add_generation_prompt=True, enable_thinking=False, tokenize=False)
batch = proc(text=[text], images=[picture], return_tensors="pt")
ids = batch["input_ids"]
open(out + ".ids", "w").write(" ".join(str(int(i)) for i in ids[0]))

cfg = Qwen3_5Config(**json.load(open(folder + "/config.json")))
cfg._attn_implementation = "eager"
cfg.text_config._attn_implementation = "eager"
cfg.vision_config._attn_implementation = "eager"
# (Made without memory, then given the weights as float32 one by one: the machine has little memory.)
with torch.device("meta"):
    model = M.Qwen3_5ForConditionalGeneration(cfg)
state = {}
with safe_open(folder + "/model.safetensors-00001-of-00001.safetensors", framework="pt") as f:
    for name in f.keys():
        if name.startswith("mtp."):
            continue
        state[name] = f.get_tensor(name).float()
if "lm_head.weight" not in state:
    state["lm_head.weight"] = state["model.language_model.embed_tokens.weight"]
model.load_state_dict(state, strict=False, assign=True)
# (The buffers made on "meta" made again for real.)
model.model.visual.rotary_pos_emb = M.Qwen3_5VisionRotaryEmbedding(cfg.vision_config)
model.model.language_model.rotary_emb = M.Qwen3_5TextRotaryEmbedding(cfg.text_config)
model.eval()
with torch.no_grad():
    logits = model(**batch).logits[0]
    open(out + ".last.f32", "wb").write(logits[-last:].float().numpy().astype("<f4").tobytes())
    made = model.generate(**batch, max_new_tokens=new, do_sample=False)
answer = made[0, ids.shape[1]:].tolist()
open(out + ".answer", "w").write(" ".join(str(int(i)) for i in answer))
print(f"{ids.shape[1]} tokens in the prompt ({int((ids == cfg.image_token_id).sum())} of the picture's);",
      "it answers:", repr(proc.tokenizer.decode(answer)))
