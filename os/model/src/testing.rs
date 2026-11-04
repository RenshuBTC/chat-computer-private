//! Small models with random weights, made here, for tests that need a model without downloading one: the engine's
//! own (tests.rs), and Chat Computer's, which start the engine's program with one (examples/tiny_model.rs writes it).
//! One can have a small vision part too (`tiny_seeing`), for pictures.

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
    build(experts, false)
}

/// The same with a vision part (2 blocks, 64 wide, patches of 4 pixels, 2x2 of them a row: a picture 8 pixels a
/// side a row, 8 by 8 to 64 by 64 pixels) and M-RoPE's positions, and the pictures' special tokens.
pub fn tiny_seeing(experts: bool) -> Vec<u8> {
    build(experts, true)
}

/// The vision's special tokens, after the chat's (ids 265 to 267).
const VISION_SPECIALS: [&str; 3] = ["<|vision_start|>", "<|vision_end|>", "<|image_pad|>"];

fn build(experts: bool, seeing: bool) -> Vec<u8> {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15 ^ experts as u64);
    let (h, vocab) = (64usize, 300usize);
    let mut config = String::from(
        "arch=qwen3_5\nhidden=64\nlayers=4\nheads=4\nkv_heads=2\nhead_dim=32\nlinear_key_heads=2\nlinear_value_heads=4\n\
         linear_key_dim=16\nlinear_value_dim=16\nconv_kernel=4\nfull_attention_interval=4\nvocab=300\nrope_theta=10000000\n\
         rotary_dim=8\nrms_eps=0.000001\ncontext=262144\nchat.style=qwen3.5\n",
    );
    if experts {
        config.push_str("experts=4\nexperts_used=2\nexpert_intermediate=32\nshared_intermediate=64\n");
    } else {
        config.push_str("intermediate=96\n");
    }
    if seeing {
        config.push_str(
            "mrope_section=2,1,1\nmrope_interleaved=1\nvision.depth=2\nvision.hidden=64\nvision.heads=2\n\
             vision.intermediate=96\nvision.patch=4\nvision.temporal=2\nvision.merge=2\nvision.positions=16\nvision.out=64\n\
             vision.rope_theta=10000\nvision.image_token=267\nvision.start_token=265\nvision.end_token=266\n\
             vision.min_pixels=64\nvision.max_pixels=4096\nvision.mean=0.5,0.5,0.5\nvision.std=0.5,0.5,0.5\n",
        );
    }
    let mut w = Writer::default();
    w.add("config", Dtype::Blob, &[config.len()], config.as_bytes());
    // The tokenizer: bytes, no merges, the specials.
    let mut tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    tokens.extend(SPECIALS.iter().map(|s| s.as_bytes().to_vec()));
    if seeing {
        tokens.extend(VISION_SPECIALS.iter().map(|s| s.as_bytes().to_vec()));
    }
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
    let all: Vec<&str> = SPECIALS.iter().chain(if seeing { &VISION_SPECIALS[..] } else { &[] }).copied().collect();
    let specials: String = all.iter().enumerate().map(|(i, s)| format!("{} {s}\n", 256 + i)).collect();
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
    if seeing {
        // The vision part: (vh wide, 2 blocks, its patches 3 x 2 x 4 x 4 numbers, 4 x 4 learned positions).
        let vh = 64;
        let pairs = |w: &mut Writer, name: &str, r: &mut Rng| {
            vector(w, name, r.vec(vh, 0.1).iter().map(|v| 1.0 + v).collect());
            vector(w, &format!("{name}_bias"), r.vec(vh, 0.1));
        };
        matrix(&mut w, "vision.patch", vh, 96, &mut r);
        vector(&mut w, "vision.patch_bias", r.vec(vh, 0.1));
        matrix(&mut w, "vision.pos", 16, vh, &mut r);
        for b in 0..2 {
            let p = |n: &str| format!("vision.{b}.{n}");
            pairs(&mut w, &p("norm1"), &mut r);
            matrix(&mut w, &p("qkv"), 3 * vh, vh, &mut r);
            vector(&mut w, &p("qkv_bias"), r.vec(3 * vh, 0.1));
            matrix(&mut w, &p("proj"), vh, vh, &mut r);
            vector(&mut w, &p("proj_bias"), r.vec(vh, 0.1));
            pairs(&mut w, &p("norm2"), &mut r);
            matrix(&mut w, &p("fc1"), 96, vh, &mut r);
            vector(&mut w, &p("fc1_bias"), r.vec(96, 0.1));
            matrix(&mut w, &p("fc2"), vh, 96, &mut r);
            vector(&mut w, &p("fc2_bias"), r.vec(vh, 0.1));
        }
        pairs(&mut w, "vision.merger.norm", &mut r);
        matrix(&mut w, "vision.merger.fc1", 4 * vh, 4 * vh, &mut r);
        vector(&mut w, "vision.merger.fc1_bias", r.vec(4 * vh, 0.1));
        matrix(&mut w, "vision.merger.fc2", h, 4 * vh, &mut r);
        vector(&mut w, "vision.merger.fc2_bias", r.vec(h, 0.1));
    }
    let mut out = w.head();
    out.extend_from_slice(&w.data);
    out
}

/// A speech recognition model with random weights (Whisper's design, 64 wide, 2 encoder and 2 decoder layers, 80
/// mels, its 1,500 positions and 64 for tokens): a byte for each of the first 256 tokens, then Whisper's special
/// tokens (an English-only model's) and four timestamps. The same bytes every time.
pub fn tiny_listening() -> Vec<u8> {
    let mut r = Rng(0x005E_ED0F_5EEC);
    let (w, ffn, mels, vocab) = (64usize, 96usize, 80usize, 268usize);
    let config = String::from(
        "converter=2\narch=whisper\nwhisper.mels=80\nwhisper.width=64\nwhisper.encoder_layers=2\nwhisper.decoder_layers=2\n\
         whisper.encoder_heads=2\nwhisper.decoder_heads=2\nwhisper.encoder_ffn=96\nwhisper.decoder_ffn=96\nwhisper.vocab=268\n\
         whisper.audio_ctx=1500\nwhisper.text_ctx=64\nwhisper.sot=257\nwhisper.eot=256\nwhisper.no_timestamps=263\n\
         whisper.no_speech=262\nwhisper.timestamp_begin=264\nwhisper.blank=32\nwhisper.suppress=257,258,259,260,261,262\n\
         whisper.begin_suppress=32,256\n",
    );
    let mut wr = Writer::default();
    wr.add("config", Dtype::Blob, &[config.len()], config.as_bytes());
    let specials = [
        "<|endoftext|>", "<|startoftranscript|>", "<|translate|>", "<|transcribe|>", "<|startoflm|>", "<|startofprev|>",
        "<|nocaptions|>", "<|notimestamps|>", "<|0.00|>", "<|0.02|>", "<|0.04|>", "<|0.06|>",
    ];
    let mut tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    tokens.extend(specials.iter().map(|s| s.as_bytes().to_vec()));
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
    wr.add("tokenizer.tokens", Dtype::Blob, &[blob.len()], &blob);
    wr.add("tokenizer.merges", Dtype::Blob, &[0], &[]);
    let lines: String = specials.iter().enumerate().map(|(i, s)| format!("{} {s}\n", 256 + i)).collect();
    wr.add("tokenizer.specials", Dtype::Blob, &[lines.len()], lines.as_bytes());
    let f32s = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let matrix = |wr: &mut Writer, name: &str, rows: usize, cols: usize, r: &mut Rng| {
        let v = r.vec(rows * cols, 1.0 / crate::math::sqrt(cols as f32) * 1.7);
        let mut q = Vec::new();
        quant::quantize_q8(&v, &mut q);
        wr.add(name, Dtype::Q8, &[rows, cols], &q);
    };
    let vector = |wr: &mut Writer, name: &str, v: Vec<f32>| wr.add(name, Dtype::F32, &[v.len()], &f32s(&v));
    let norm = |wr: &mut Writer, name: &str, r: &mut Rng| {
        vector(wr, name, r.vec(w, 0.1).iter().map(|v| 1.0 + v).collect());
        vector(wr, &format!("{name}_bias"), r.vec(w, 0.1));
    };
    let attention = |wr: &mut Writer, p: &str, r: &mut Rng| {
        for m in ["q", "k", "v", "o"] {
            matrix(wr, &format!("{p}.{m}"), w, w, r);
        }
        for b in ["q_bias", "v_bias", "o_bias"] {
            vector(wr, &format!("{p}.{b}"), r.vec(w, 0.1));
        }
    };
    let mlp = |wr: &mut Writer, p: &str, r: &mut Rng| {
        matrix(wr, &format!("{p}.fc1"), ffn, w, r);
        vector(wr, &format!("{p}.fc1_bias"), r.vec(ffn, 0.1));
        matrix(wr, &format!("{p}.fc2"), w, ffn, r);
        vector(wr, &format!("{p}.fc2_bias"), r.vec(w, 0.1));
    };
    let conv1 = r.vec(w * 3 * mels, 0.1);
    wr.add("whisper.conv1", Dtype::F32, &[w, 3 * mels], &f32s(&conv1));
    vector(&mut wr, "whisper.conv1_bias", r.vec(w, 0.1));
    matrix(&mut wr, "whisper.conv2", w, 3 * w, &mut r);
    vector(&mut wr, "whisper.conv2_bias", r.vec(w, 0.1));
    vector(&mut wr, "whisper.encoder_pos", r.vec(1500 * w, 0.5));
    for l in 0..2 {
        let p = format!("whisper.encoder.{l}");
        norm(&mut wr, &format!("{p}.norm1"), &mut r);
        attention(&mut wr, &format!("{p}.attention"), &mut r);
        norm(&mut wr, &format!("{p}.norm2"), &mut r);
        mlp(&mut wr, &p, &mut r);
    }
    norm(&mut wr, "whisper.encoder_norm", &mut r);
    matrix(&mut wr, "whisper.embed", vocab, w, &mut r);
    vector(&mut wr, "whisper.decoder_pos", r.vec(64 * w, 0.5));
    for l in 0..2 {
        let p = format!("whisper.decoder.{l}");
        norm(&mut wr, &format!("{p}.norm1"), &mut r);
        attention(&mut wr, &format!("{p}.attention"), &mut r);
        norm(&mut wr, &format!("{p}.norm2"), &mut r);
        attention(&mut wr, &format!("{p}.cross"), &mut r);
        norm(&mut wr, &format!("{p}.norm3"), &mut r);
        mlp(&mut wr, &p, &mut r);
    }
    norm(&mut wr, "whisper.decoder_norm", &mut r);
    let mut out = wr.head();
    out.extend_from_slice(&wr.data);
    out
}

/// A recording that sounds like something (a voice-like hum rising and falling, its harmonics, a little noise),
/// `seconds` long at 16 kHz, the same each time from the same `seed`.
pub fn recording(seconds: f32, seed: u64) -> Vec<f32> {
    let mut r = Rng((seed << 1) | 1);
    let n = (seconds * 16000.0) as usize;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / 16000.0;
            let pitch = 140.0 + 40.0 * crate::math::sin_cos(2.0 * core::f64::consts::PI * 0.7 * t).0;
            phase += 2.0 * core::f64::consts::PI * pitch / 16000.0;
            let mut v = 0.0;
            for k in 1..6 {
                v += crate::math::sin_cos(phase * k as f64).0 / k as f64;
            }
            let envelope = 0.5 + 0.5 * crate::math::sin_cos(2.0 * core::f64::consts::PI * 2.0 * t).0;
            (0.2 * v * envelope) as f32 + 0.01 * r.float()
        })
        .collect()
}

/// A picture of `width` by `height` pixels that looks like something (gradients, a square, a stripe), the same each
/// time from the same `seed`.
pub fn picture(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng((seed << 1) | 1);
    let mut rgb = Vec::with_capacity(width * height * 3);
    let (sx, sy) = ((r.float().abs() * width as f32) as usize, (r.float().abs() * height as f32) as usize);
    for y in 0..height {
        for x in 0..width {
            let square = x >= sx && x < sx + width / 3 && y >= sy && y < sy + height / 3;
            let stripe = (x + y) % 7 == 0;
            let base = [(x * 255 / width.max(1)) as u8, (y * 255 / height.max(1)) as u8, ((x ^ y) & 0xFF) as u8];
            let px = if square { [250, 220, 90] } else if stripe { [20, 20, 20] } else { base };
            rgb.extend_from_slice(&px);
        }
    }
    rgb
}
