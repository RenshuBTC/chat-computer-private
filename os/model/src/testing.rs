//! Small models with random weights, made here, for tests that need a model without downloading one: the engine's
//! own (tests.rs), and Chat Computer's, which start the engine's program with one (examples/tiny_model.rs writes it).

use crate::file::Writer;
use crate::quant::{self, Dtype};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Numbers that look random, the same each time from the same start (xorshift).
pub struct Rng(pub u64);

impl Rng {
    pub fn float(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 20001) as f32 / 10000.0 - 1.0
    }

    pub fn vec(&mut self, n: usize, scale: f32) -> Vec<f32> {
        (0..n).map(|_| self.float() * scale).collect()
    }
}

const SPECIALS: [&str; 9] = [
    "<|endoftext|>",
    "<|im_start|>",
    "<|im_end|>",
    "<think>",
    "</think>",
    "<tool_call>",
    "</tool_call>",
    "<tool_response>",
    "</tool_response>",
];

/// A model file with random weights: 4 layers (three of linear attention, one of attention), 64 wide; a byte for
/// each of the first 256 tokens, then the chat's special tokens. The same bytes every time.
pub fn tiny(experts: bool) -> Vec<u8> {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15 ^ experts as u64);
    let (h, vocab) = (64usize, 300usize);
    let mut config = String::from(
        "arch=qwen3_5\nhidden=64\nlayers=4\nheads=4\nkv_heads=2\nhead_dim=32\nlinear_key_heads=2\nlinear_value_heads=4\n\
         linear_key_dim=16\nlinear_value_dim=16\nconv_kernel=4\nfull_attention_interval=4\nvocab=300\nrope_theta=10000000\n\
         rotary_dim=8\nrms_eps=0.000001\ncontext=4096\nchat.style=qwen3.5\n",
    );
    if experts {
        config.push_str("experts=4\nexperts_used=2\nexpert_intermediate=32\nshared_intermediate=64\n");
    } else {
        config.push_str("intermediate=96\n");
    }
    let mut w = Writer::default();
    w.add("config", Dtype::Blob, &[config.len()], config.as_bytes());
    // The tokenizer: bytes, no merges, the specials.
    let mut tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    tokens.extend(SPECIALS.iter().map(|s| s.as_bytes().to_vec()));
    let mut blob = Vec::new();
    blob.extend_from_slice(&(tokens.len() as u32).to_le_bytes());
    let mut at = 0u32;
    blob.extend_from_slice(&at.to_le_bytes());
    for t in &tokens {
        at += t.len() as u32;
        blob.extend_from_slice(&at.to_le_bytes());
    }
    for t in &tokens {
        blob.extend_from_slice(t);
    }
    w.add("tokenizer.tokens", Dtype::Blob, &[blob.len()], &blob);
    w.add("tokenizer.merges", Dtype::Blob, &[0], &[]);
    let specials: String = SPECIALS.iter().enumerate().map(|(i, s)| format!("{} {s}\n", 256 + i)).collect();
    w.add("tokenizer.specials", Dtype::Blob, &[specials.len()], specials.as_bytes());
    let matrix = |w: &mut Writer, name: &str, rows: usize, cols: usize, r: &mut Rng| {
        let v = r.vec(rows * cols, 1.0 / crate::math::sqrt(cols as f32) * 1.7);
        let mut q = Vec::new();
        quant::quantize_q8(&v, &mut q);
        w.add(name, Dtype::Q8, &[rows, cols], &q);
    };
    let f32s = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let vector = |w: &mut Writer, name: &str, v: Vec<f32>| w.add(name, Dtype::F32, &[v.len()], &f32s(&v));
    matrix(&mut w, "embed", vocab, h, &mut r);
    matrix(&mut w, "head", vocab, h, &mut r);
    vector(&mut w, "norm", r.vec(h, 0.1));
    for l in 0..4 {
        let p = |n: &str| format!("layers.{l}.{n}");
        vector(&mut w, &p("norm1"), r.vec(h, 0.1));
        vector(&mut w, &p("norm2"), r.vec(h, 0.1));
        if l == 3 {
            matrix(&mut w, &p("q"), 4 * 32 * 2, h, &mut r);
            matrix(&mut w, &p("k"), 2 * 32, h, &mut r);
            matrix(&mut w, &p("v"), 2 * 32, h, &mut r);
            matrix(&mut w, &p("o"), h, 4 * 32, &mut r);
            vector(&mut w, &p("q_norm"), r.vec(32, 0.1));
            vector(&mut w, &p("k_norm"), r.vec(32, 0.1));
        } else {
            matrix(&mut w, &p("qkv"), 2 * 32 + 64, h, &mut r);
            matrix(&mut w, &p("z"), 64, h, &mut r);
            matrix(&mut w, &p("b"), 4, h, &mut r);
            matrix(&mut w, &p("a"), 4, h, &mut r);
            let conv = r.vec(128 * 4, 0.3);
            w.add(&p("conv"), Dtype::F32, &[128, 4], &f32s(&conv));
            vector(&mut w, &p("a_log"), (0..4).map(|i| crate::math::ln(1.0 + i as f32 * 3.0)).collect());
            vector(&mut w, &p("dt_bias"), r.vec(4, 0.5));
            vector(&mut w, &p("linear_norm"), r.vec(16, 0.1).iter().map(|v| 1.0 + v).collect());
            matrix(&mut w, &p("out"), h, 64, &mut r);
        }
        if experts {
            let router = r.vec(4 * h, 0.4);
            w.add(&p("router"), Dtype::F32, &[4, h], &f32s(&router));
            matrix(&mut w, &p("experts_gate_up"), 4 * 2 * 32, h, &mut r);
            matrix(&mut w, &p("experts_down"), 4 * h, 32, &mut r);
            matrix(&mut w, &p("shared_gate"), 64, h, &mut r);
            matrix(&mut w, &p("shared_up"), 64, h, &mut r);
            matrix(&mut w, &p("shared_down"), h, 64, &mut r);
            let sw = r.vec(h, 0.3);
            w.add(&p("shared_weight"), Dtype::F32, &[1, h], &f32s(&sw));
        } else {
            matrix(&mut w, &p("gate"), 96, h, &mut r);
            matrix(&mut w, &p("up"), 96, h, &mut r);
            matrix(&mut w, &p("down"), h, 96, &mut r);
        }
    }
    let mut out = w.head();
    out.extend_from_slice(&w.data);
    out
}
