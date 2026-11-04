//! A model of Qwen3.5 0.8B's shape (its settings, every weight's type and shape as the converter makes them, 8 bits a
//! weight) with random weights, made in memory: the engine's speed in a browser, timed without downloading the model
//! (scripts/build_web_engine.py --bench builds a module with it; tests/web_engine_check.js --bench times it). The
//! arithmetic takes as long whatever the weights are.

use model::quant::{self, Dtype};

/// Qwen3.5 0.8B's settings, as the converter writes them from its config.json.
const SETTINGS: &str = "arch=qwen3_5\nhidden=1024\nlayers=24\nheads=8\nkv_heads=2\nhead_dim=256\nintermediate=3584\n\
    linear_key_heads=16\nlinear_value_heads=16\nlinear_key_dim=128\nlinear_value_dim=128\nconv_kernel=4\n\
    full_attention_interval=4\nvocab=248320\nrope_theta=10000000\nrotary_dim=64\nrms_eps=0.000001\ncontext=262144\n\
    chat.style=qwen3.5-small\n";

pub fn qwen35_08b() -> Vec<u8> {
    let (h, vocab, inter) = (1024usize, 248_320usize, 3584usize);
    let (conv_dim, vd, lv) = (2 * 16 * 128 + 16 * 128, 16 * 128, 16usize);
    // The tokenizer: a token for each byte, then two bytes each; the chat's special tokens.
    let mut tokens = (vocab as u32).to_le_bytes().to_vec();
    let mut at = 0u32;
    tokens.extend_from_slice(&at.to_le_bytes());
    let mut blob = Vec::new();
    for i in 0..vocab {
        if i < 256 {
            blob.push(i as u8);
        } else {
            blob.extend_from_slice(&[(i % 251) as u8 + 1, (i / 251 % 251) as u8 + 1]);
        }
        at = blob.len() as u32;
        tokens.extend_from_slice(&at.to_le_bytes());
    }
    tokens.extend_from_slice(&blob);
    let specials: String = ["<|endoftext|>", "<|im_start|>", "<|im_end|>", "<think>", "</think>", "<tool_call>", "</tool_call>"]
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{} {s}\n", 248_044 + i))
        .collect();
    let mut entries: Vec<(String, Dtype, Vec<usize>, Option<Vec<u8>>)> = vec![
        ("config".into(), Dtype::Blob, vec![SETTINGS.len()], Some(SETTINGS.as_bytes().to_vec())),
        ("tokenizer.tokens".into(), Dtype::Blob, vec![tokens.len()], Some(tokens)),
        ("tokenizer.merges".into(), Dtype::Blob, vec![0], Some(Vec::new())),
        ("tokenizer.specials".into(), Dtype::Blob, vec![specials.len()], Some(specials.into_bytes())),
    ];
    let mut weights = vec![("embed".to_string(), Dtype::Q8, vec![vocab, h]), ("norm".to_string(), Dtype::F32, vec![h])];
    for l in 0..24 {
        let p = |n: &str| format!("layers.{l}.{n}");
        weights.push((p("norm1"), Dtype::F32, vec![h]));
        weights.push((p("norm2"), Dtype::F32, vec![h]));
        if (l + 1) % 4 == 0 {
            weights.push((p("q"), Dtype::Q8, vec![8 * 256 * 2, h]));
            weights.push((p("k"), Dtype::Q8, vec![2 * 256, h]));
            weights.push((p("v"), Dtype::Q8, vec![2 * 256, h]));
            weights.push((p("o"), Dtype::Q8, vec![h, 8 * 256]));
            weights.push((p("q_norm"), Dtype::F32, vec![256]));
            weights.push((p("k_norm"), Dtype::F32, vec![256]));
        } else {
            weights.push((p("qkv"), Dtype::Q8, vec![conv_dim, h]));
            weights.push((p("z"), Dtype::Q8, vec![vd, h]));
            weights.push((p("b"), Dtype::Q8, vec![lv, h]));
            weights.push((p("a"), Dtype::Q8, vec![lv, h]));
            weights.push((p("conv"), Dtype::F32, vec![conv_dim, 4]));
            weights.push((p("a_log"), Dtype::F32, vec![lv]));
            weights.push((p("dt_bias"), Dtype::F32, vec![lv]));
            weights.push((p("linear_norm"), Dtype::F32, vec![128]));
            weights.push((p("out"), Dtype::Q8, vec![h, vd]));
        }
        weights.push((p("gate"), Dtype::Q8, vec![inter, h]));
        weights.push((p("up"), Dtype::Q8, vec![inter, h]));
        weights.push((p("down"), Dtype::Q8, vec![h, inter]));
    }
    weights.sort_by(|a, b| a.0.cmp(&b.0));
    entries.extend(weights.into_iter().map(|(n, d, s)| (n, d, s, None)));
    // The table, then the file at once (its weights random: each block's scale for numbers of about 1/sqrt(width)).
    let mut table = String::new();
    let mut place = 0usize;
    let mut places = Vec::new();
    for (name, dtype, dims, blob) in &entries {
        place = place.div_ceil(64) * 64;
        let size = blob.as_ref().map_or_else(|| dtype.bytes_for(dims.iter().product()).unwrap(), |b| b.len());
        let shape = dims.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",");
        table.push_str(&format!("{name} {} {shape} {place} {size}\n", dtype.name()));
        places.push((place, size));
        place += size;
    }
    let mut out = Vec::new();
    out.extend_from_slice(model::file::MAGIC);
    out.extend_from_slice(&(table.len() as u32).to_le_bytes());
    out.extend_from_slice(table.as_bytes());
    out.resize(out.len().div_ceil(64) * 64, 0);
    let data = out.len();
    out.resize(data + place, 0);
    let mut r = 0x9E37_79B9_7F4A_7C15u64;
    for ((name, dtype, dims, blob), &(at, size)) in entries.iter().zip(&places) {
        let bytes = &mut out[data + at..data + at + size];
        match (blob, dtype) {
            (Some(b), _) => bytes.copy_from_slice(b),
            (None, Dtype::Q8) => {
                let scale = quant::f32_to_f16(1.0 / (127.0 * (dims[1] as f32).sqrt())).to_le_bytes();
                for block in bytes.chunks_exact_mut(quant::Q8_BYTES) {
                    block[..2].copy_from_slice(&scale);
                    for chunk in block[2..].chunks_mut(8) {
                        r ^= r << 13;
                        r ^= r >> 7;
                        r ^= r << 17;
                        chunk.copy_from_slice(&r.to_le_bytes()[..chunk.len()]);
                    }
                }
            }
            (None, _) => {
                // Norms of 1 (Qwen's are zero-centred: 0 is 1, but for the linear attention's own), the decays'
                // logarithms 0, small convolution weights.
                let v: f32 = if name.ends_with("linear_norm") { 1.0 } else if name.ends_with("conv") { 0.1 } else { 0.0 };
                for c in bytes.chunks_exact_mut(4) {
                    c.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    out
}
