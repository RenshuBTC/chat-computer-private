"""The reference: Qwen 3.5's (and 3.6's, with experts) text model as transformers runs it (float32, on the processor),
for the engine's scores to be checked against. Writes the prompt's token ids (as transformers' tokenizer makes them)
and every position's scores.

    python reference.py MODEL_FOLDER OUT_PREFIX "a prompt" [--q8]

--q8: with the engine's 8-bit arithmetic copied (weights and the vectors they multiply rounded as quant.rs rounds
them), to check the engine's 8-bit path itself."""
import json
import sys

import numpy as np
import torch
from transformers import AutoTokenizer
from safetensors import safe_open

folder, out = sys.argv[1], sys.argv[2]
text = sys.argv[3]
eight_bits = "--q8" in sys.argv[4:]
tok = AutoTokenizer.from_pretrained(folder)
ids = tok(text, add_special_tokens=False)["input_ids"]
cfg = json.load(open(folder + "/config.json"))
if cfg["model_type"] == "qwen3_5_moe":
    from transformers.models.qwen3_5_moe import modeling_qwen3_5_moe as M
    from transformers.models.qwen3_5_moe.configuration_qwen3_5_moe import Qwen3_5MoeTextConfig as TextConfig
    Model, Rotary = M.Qwen3_5MoeForCausalLM, M.Qwen3_5MoeTextRotaryEmbedding
else:
    from transformers.models.qwen3_5 import modeling_qwen3_5 as M
    from transformers.models.qwen3_5.configuration_qwen3_5 import Qwen3_5TextConfig as TextConfig
    Model, Rotary = M.Qwen3_5ForCausalLM, M.Qwen3_5TextRotaryEmbedding
tcfg = TextConfig(**cfg["text_config"])
tcfg._attn_implementation = "eager"
# (Made without memory, then given the weights as float32 one by one: the machine has little memory.)
with torch.device("meta"):
    model = Model(tcfg)
state = {}
with safe_open(folder + "/model.safetensors-00001-of-00001.safetensors", framework="pt") as f:
    for name in f.keys():
        if name.startswith("model.language_model."):
            state["model." + name[len("model.language_model."):]] = f.get_tensor(name).float()
        elif name == "lm_head.weight":
            state["lm_head.weight"] = f.get_tensor(name).float()
if "lm_head.weight" not in state:
    state["lm_head.weight"] = state["model.embed_tokens.weight"]
QUANTIZED = ("in_proj_qkv.weight", "in_proj_z.weight", "in_proj_b.weight", "in_proj_a.weight", "out_proj.weight",
             "q_proj.weight", "k_proj.weight", "v_proj.weight", "o_proj.weight", "gate_proj.weight", "up_proj.weight",
             "down_proj.weight", "experts.gate_up_proj", "experts.down_proj", "embed_tokens.weight", "lm_head.weight")

def q8(x):
    """quant.rs's quantize_q8 and back: blocks of 32 along the last axis, scale max/127 as f16, rounded half away."""
    shape = x.shape
    b = x.reshape(-1, 32)
    d = (b.abs().amax(-1, keepdim=True) / 127.0).to(torch.float16).to(torch.float32)
    inv = torch.where(d > 0, 1.0 / d, torch.zeros_like(d))
    q = (torch.sign(b * inv) * torch.floor((b * inv).abs() + 0.5)).clamp(-127, 127)
    return (q * d).reshape(shape)

quantized_storage = set()
for name in list(state) if eight_bits else []:
    if name.endswith(QUANTIZED) and state[name].shape[-1] % 32 == 0:
        state[name] = q8(state[name])
        if not name.endswith("embed_tokens.weight"):
            quantized_storage.add(name)
missing, unexpected = model.load_state_dict(state, strict=False, assign=True)
ptrs = set()
for n, p in model.named_parameters():
    if n in quantized_storage:
        ptrs.add(p.untyped_storage().data_ptr())
F = torch.nn.functional
plain_linear = F.linear
def linear(x, w, b=None):
    if w.untyped_storage().data_ptr() in ptrs:
        x = q8(x)
    return plain_linear(x, w, b)
F.linear = linear
if eight_bits:
    print(len(ptrs), "matrices with 8-bit arithmetic")
assert not missing and not unexpected, (missing[:5], unexpected[:5])
model.model.rotary_emb = Rotary(config=tcfg)   # (its frequencies aren't weights)
model = model.eval()
with torch.no_grad():
    logits = model(torch.tensor([ids])).logits[0].float().numpy()
json.dump(ids, open(out + ".ids.json", "w"))
logits.astype(np.float32).tofile(out + ".logits.f32")
print(len(ids), "tokens; top token at the end:", int(logits[-1].argmax()), repr(tok.decode([int(logits[-1].argmax())])))
