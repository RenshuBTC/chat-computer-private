"""A tiny Qwen 3.5/3.6 mixture-of-experts model with random weights, saved as Qwen publishes theirs (config.json with
a text_config, bf16 safetensors named model.language_model.*, a tokenizer and 3.6's chat template), for the engine's
experts to be checked against transformers:

    python tiny_moe.py OUT_FOLDER TOKENIZER TEMPLATE [EXPERTS_USED [calm]]

TOKENIZER: a folder with a model's tokenizer.json and tokenizer_config.json; TEMPLATE: Qwen3.6-35B-A3B's
chat_template.jinja. EXPERTS_USED: 2 (of 8) unless given; with all 8, no token's choice of experts can flip on a
rounding. "calm": each layer adds less, so 8-bit rounding isn't blown up by the random weights (to check the 8-bit
arithmetic itself: reference.py --q8).
"""

import json
import math
import os
import shutil
import sys

import torch
from safetensors.torch import save_file
from transformers.models.qwen3_5_moe import modeling_qwen3_5_moe as M
from transformers.models.qwen3_5_moe.configuration_qwen3_5_moe import Qwen3_5MoeTextConfig

out, tokenizer_dir, template = sys.argv[1], sys.argv[2], sys.argv[3]
used = int(sys.argv[4]) if len(sys.argv) > 4 else 2
os.makedirs(out, exist_ok=True)
torch.manual_seed(0)

layers = 8
text = dict(
    hidden_size=64, num_hidden_layers=layers, num_attention_heads=4, num_key_value_heads=2, head_dim=32,
    linear_num_key_heads=2, linear_num_value_heads=4, linear_key_head_dim=16, linear_value_head_dim=16,
    linear_conv_kernel_dim=4, num_experts=8, num_experts_per_tok=used, moe_intermediate_size=32,
    shared_expert_intermediate_size=48, vocab_size=248320, full_attention_interval=4, rms_norm_eps=1e-6,
    max_position_embeddings=4096, hidden_act="silu", attn_output_gate=True, tie_word_embeddings=False,
    rope_parameters={"rope_type": "default", "rope_theta": 10000000, "partial_rotary_factor": 0.25,
                     "mrope_section": [2, 1, 1], "mrope_interleaved": True},
    layer_types=["full_attention" if (i + 1) % 4 == 0 else "linear_attention" for i in range(layers)],
)
cfg = Qwen3_5MoeTextConfig(**text)
model = M.Qwen3_5MoeForCausalLM(cfg)

with torch.no_grad():
    for name, p in model.named_parameters():
        if name.endswith(("input_layernorm.weight", "post_attention_layernorm.weight", "model.norm.weight",
                          "q_norm.weight", "k_norm.weight")):
            p.copy_(torch.randn_like(p) * 0.1)  # (1 + w)
        elif name.endswith("linear_attn.norm.weight"):
            p.copy_(1 + torch.randn_like(p) * 0.1)
        elif name.endswith("A_log"):
            p.copy_(torch.log(torch.empty_like(p).uniform_(1, 16)))
        elif name.endswith("dt_bias"):
            p.copy_(torch.randn_like(p) * 0.5)
        elif "conv1d" in name:
            p.copy_(torch.randn_like(p) * 0.3)
        elif name.endswith("embed_tokens.weight"):
            p.copy_(torch.randn_like(p) * 0.5)
        elif name.endswith("mlp.gate.weight"):
            p.copy_(torch.randn_like(p) * 3 / math.sqrt(p.shape[-1]))
        elif name.endswith("shared_expert_gate.weight"):
            p.copy_(torch.randn_like(p) * 0.3)
        else:
            p.copy_(torch.randn_like(p) / math.sqrt(p.shape[-1]))

# "calm": each layer adds less to the stream, so 8-bit rounding isn't blown up by the random weights (for checking
# the 8-bit arithmetic itself).
if len(sys.argv) > 5 and sys.argv[5] == "calm":
    with torch.no_grad():
        for name, p in model.named_parameters():
            if name.endswith(("out_proj.weight", "o_proj.weight", "experts.down_proj", "shared_expert.down_proj.weight")):
                p.mul_(0.2)
state = {}
for name, t in model.state_dict().items():
    hub = "lm_head.weight" if name == "lm_head.weight" else "model.language_model." + name[len("model."):]
    state[hub] = t.to(torch.bfloat16).contiguous()
save_file(state, os.path.join(out, "model.safetensors-00001-of-00001.safetensors"), metadata={"format": "pt"})
config = {
    "architectures": ["Qwen3_5MoeForConditionalGeneration"],
    "model_type": "qwen3_5_moe",
    "text_config": {**text, "model_type": "qwen3_5_moe_text", "dtype": "bfloat16"},
    "tie_word_embeddings": False,
}
json.dump(config, open(os.path.join(out, "config.json"), "w"), indent=2)
for f in ("tokenizer.json", "tokenizer_config.json"):
    shutil.copy(os.path.join(tokenizer_dir, f), out)
shutil.copy(template, os.path.join(out, "chat_template.jinja"))
print("saved", sum(t.numel() for t in state.values()), "weights")
