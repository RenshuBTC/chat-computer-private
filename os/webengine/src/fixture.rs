//! A small model with random weights as Qwen publishes its models (config.json, tokenizer.json, the chat template,
//! the weights in two safetensors files, with a vision part and a further-ahead layer the engine leaves out), for
//! checking that a page makes the same model's file from them as the engine's own program does (convert.rs' tests,
//! and tests/test_web_engine.py, through examples/tiny_published.rs). Its shape is the engine's tiny test model's
//! (model::testing): 4 layers, 64 wide.

use model::json::{self, Json};
use model::token::byte_level_table;

/// Numbers that look random, the same each time (xorshift).
struct Rng(u64);

impl Rng {
    fn float(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 20001) as f32 / 10000.0 - 1.0
    }
}

/// Merges for the tokenizer: a few pairs of English, each a token of its own.
const MERGES: [(&str, &str); 6] = [("h", "e"), ("Ġ", "t"), ("Ġt", "he"), ("e", "r"), ("i", "n"), ("Ġ", "a")];
const SPECIALS: [&str; 9] = [
    "<|endoftext|>", "<|im_start|>", "<|im_end|>", "<think>", "</think>", "<tool_call>", "</tool_call>",
    "<tool_response>", "</tool_response>",
];
/// The model's vocabulary: a token for each byte, the merges', and the special tokens.
pub const VOCAB: usize = 256 + MERGES.len() + SPECIALS.len();

/// The lines of Qwen 3.5's small models' chat template that tell its style (chat.rs: Style::of_template), in a
/// template of its own (the engine writes conversations itself: it reads only which style a template is).
const TEMPLATE: &str = "{%- for message in messages %}{{- '<|im_start|>' + message.role + '\\n' }}{%- endfor %}\n\
    {{- '<tool_call>\\n<function=' }}{{- '\\n<tool_response>\\n' }}\n\
    {%- if enable_thinking is defined and enable_thinking is true %}{{- '<think>\\n' }}{%- endif %}\n";

fn obj(items: Vec<(&str, Json)>) -> Json {
    Json::Object(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn n(v: f64) -> Json {
    Json::Number(v)
}

pub fn config() -> String {
    let layer_types = (0..4).map(|i| Json::Str(if i == 3 { "full_attention" } else { "linear_attention" }.into())).collect();
    let text = obj(vec![
        ("model_type", Json::Str("qwen3_5_text".into())),
        ("hidden_size", n(64.0)),
        ("num_hidden_layers", n(4.0)),
        ("num_attention_heads", n(4.0)),
        ("num_key_value_heads", n(2.0)),
        ("head_dim", n(32.0)),
        ("intermediate_size", n(96.0)),
        ("linear_num_key_heads", n(2.0)),
        ("linear_num_value_heads", n(4.0)),
        ("linear_key_head_dim", n(16.0)),
        ("linear_value_head_dim", n(16.0)),
        ("linear_conv_kernel_dim", n(4.0)),
        ("full_attention_interval", n(4.0)),
        ("layer_types", Json::Array(layer_types)),
        ("vocab_size", n(VOCAB as f64)),
        ("rope_parameters", obj(vec![("rope_theta", n(10_000_000.0)), ("partial_rotary_factor", n(0.25))])),
        ("rms_norm_eps", n(1e-6)),
        ("max_position_embeddings", n(262_144.0)),
        ("tie_word_embeddings", Json::Bool(true)),
    ]);
    json::dumps(&obj(vec![("model_type", Json::Str("qwen3_5".into())), ("text_config", text), ("tie_word_embeddings", Json::Bool(true))]))
}

pub fn tokenizer() -> String {
    let table = byte_level_table();
    let mut vocab: Vec<(String, Json)> = (0..256).map(|b| (table[b].to_string(), n(b as f64))).collect();
    for (i, (l, r)) in MERGES.iter().enumerate() {
        vocab.push((format!("{l}{r}"), n((256 + i) as f64)));
    }
    let merges = MERGES.iter().map(|(l, r)| Json::Str(format!("{l} {r}"))).collect();
    let added = SPECIALS
        .iter()
        .enumerate()
        .map(|(i, s)| obj(vec![("id", n((256 + MERGES.len() + i) as f64)), ("content", Json::Str(s.to_string())), ("special", Json::Bool(true))]))
        .collect();
    json::dumps(&obj(vec![
        ("added_tokens", Json::Array(added)),
        ("model", obj(vec![("type", Json::Str("BPE".into())), ("vocab", Json::Object(vocab)), ("merges", Json::Array(merges))])),
    ]))
}

/// A weight: its published name, type ("BF16" or "F32"), shape and numbers.
struct Tensor {
    name: String,
    dtype: &'static str,
    shape: Vec<usize>,
    values: Vec<f32>,
}

fn tensors() -> Vec<Tensor> {
    let mut r = Rng(0x5EED_CAFE_F00D_0001);
    let mut t = |name: &str, dtype: &'static str, shape: &[usize], scale: f32, offset: f32| {
        let count: usize = shape.iter().product();
        let values = (0..count).map(|_| offset + r.float() * scale).collect();
        Tensor { name: name.to_string(), dtype, shape: shape.to_vec(), values }
    };
    let (h, p) = (64usize, "model.language_model.");
    let mut out = vec![
        t(&format!("{p}embed_tokens.weight"), "BF16", &[VOCAB, h], 0.2, 0.0),
        t(&format!("{p}norm.weight"), "BF16", &[h], 0.1, 0.0),
        // (The vision part and the layer for predicting further ahead: left out.)
        t("model.visual.blocks.0.attn.qkv.weight", "BF16", &[96, 32], 0.2, 0.0),
        t("mtp.fc.weight", "BF16", &[64, 128], 0.2, 0.0),
    ];
    for l in 0..4 {
        let q = |n: &str| format!("{p}layers.{l}.{n}");
        out.push(t(&q("input_layernorm.weight"), "BF16", &[h], 0.1, 0.0));
        out.push(t(&q("post_attention_layernorm.weight"), "BF16", &[h], 0.1, 0.0));
        if l == 3 {
            out.push(t(&q("self_attn.q_proj.weight"), "BF16", &[4 * 32 * 2, h], 0.2, 0.0));
            out.push(t(&q("self_attn.k_proj.weight"), "BF16", &[2 * 32, h], 0.2, 0.0));
            out.push(t(&q("self_attn.v_proj.weight"), "BF16", &[2 * 32, h], 0.2, 0.0));
            out.push(t(&q("self_attn.o_proj.weight"), "BF16", &[h, 4 * 32], 0.2, 0.0));
            out.push(t(&q("self_attn.q_norm.weight"), "BF16", &[32], 0.1, 0.0));
            out.push(t(&q("self_attn.k_norm.weight"), "BF16", &[32], 0.1, 0.0));
        } else {
            out.push(t(&q("linear_attn.in_proj_qkv.weight"), "BF16", &[2 * 32 + 64, h], 0.2, 0.0));
            out.push(t(&q("linear_attn.in_proj_z.weight"), "BF16", &[64, h], 0.2, 0.0));
            out.push(t(&q("linear_attn.in_proj_b.weight"), "BF16", &[4, h], 0.2, 0.0));
            out.push(t(&q("linear_attn.in_proj_a.weight"), "BF16", &[4, h], 0.2, 0.0));
            out.push(t(&q("linear_attn.conv1d.weight"), "BF16", &[128, 1, 4], 0.3, 0.0));
            out.push(t(&q("linear_attn.A_log"), "F32", &[4], 0.5, 1.0));
            out.push(t(&q("linear_attn.dt_bias"), "BF16", &[4], 0.5, 0.0));
            out.push(t(&q("linear_attn.norm.weight"), "F32", &[16], 0.1, 1.0));
            out.push(t(&q("linear_attn.out_proj.weight"), "BF16", &[h, 64], 0.2, 0.0));
        }
        out.push(t(&q("mlp.gate_proj.weight"), "BF16", &[96, h], 0.2, 0.0));
        out.push(t(&q("mlp.up_proj.weight"), "BF16", &[96, h], 0.2, 0.0));
        out.push(t(&q("mlp.down_proj.weight"), "BF16", &[h, 96], 0.2, 0.0));
    }
    out
}

/// A safetensors file: its header (in the tensors' order, its length padded to 8 bytes with spaces), then their
/// numbers in order of name, as Hugging Face's library writes them.
fn safetensors(mut ts: Vec<&Tensor>) -> Vec<u8> {
    let header_order: Vec<&Tensor> = ts.clone();
    ts.sort_by(|a, b| a.name.cmp(&b.name));
    let mut data = Vec::new();
    let mut places = Vec::new();
    for t in &ts {
        let start = data.len();
        for &v in &t.values {
            if t.dtype == "F32" {
                data.extend_from_slice(&v.to_le_bytes());
            } else {
                data.extend_from_slice(&((v.to_bits() >> 16) as u16).to_le_bytes());
            }
        }
        places.push((t.name.clone(), start, data.len()));
    }
    let mut items = vec![("__metadata__".to_string(), obj(vec![("format", Json::Str("pt".into()))]))];
    for t in header_order {
        let (_, a, b) = places.iter().find(|p| p.0 == t.name).unwrap();
        items.push((t.name.clone(), obj(vec![
            ("dtype", Json::Str(t.dtype.into())),
            ("shape", Json::Array(t.shape.iter().map(|&d| n(d as f64)).collect())),
            ("data_offsets", Json::Array(vec![n(*a as f64), n(*b as f64)])),
        ])));
    }
    let mut header = json::compact(&Json::Object(items)).into_bytes();
    while header.len() % 8 != 0 {
        header.push(b' ');
    }
    let mut out = (header.len() as u64).to_le_bytes().to_vec();
    out.extend_from_slice(&header);
    out.extend_from_slice(&data);
    out
}

/// The model's files, by name, as its repository has them.
pub fn files() -> Vec<(String, Vec<u8>)> {
    let ts = tensors();
    // The first file: the embedding and the first two layers; the second, the rest (each tensor in one or the other).
    let first = |t: &&Tensor| t.name.contains("embed_tokens") || t.name.contains("layers.0.") || t.name.contains("layers.1.");
    let one: Vec<&Tensor> = ts.iter().filter(first).collect();
    let two: Vec<&Tensor> = ts.iter().filter(|t| !first(t)).rev().collect();
    vec![
        ("chat_template.jinja".to_string(), TEMPLATE.as_bytes().to_vec()),
        ("config.json".to_string(), config().into_bytes()),
        ("model.safetensors-00001-of-00002.safetensors".to_string(), safetensors(one)),
        ("model.safetensors-00002-of-00002.safetensors".to_string(), safetensors(two)),
        ("tokenizer.json".to_string(), tokenizer().into_bytes()),
    ]
}
