//! The engine's own tests, on small models made here with random weights (a dense one and one with experts), so
//! they need no download: reading many tokens at once gives exactly what reading them one at a time does,
//! snapshots go back exactly, a session's reuse changes no answer, and the arithmetic's kernels agree.
//! (Against the published models and the reference implementation: tools/refcheck.)

use crate::chat::{Message, Options, Style};
use crate::file::{File, Writer};
use crate::net::{Model, Serial};
use crate::quant::{self, Dtype, Prepared};
use crate::sample::Sampling;
use crate::session::{Request, Session};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 20001) as f32 / 10000.0 - 1.0
    }

    fn vec(&mut self, n: usize, scale: f32) -> Vec<f32> {
        (0..n).map(|_| self.next() * scale).collect()
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
/// each of the first 256 tokens, then the chat's special tokens.
fn tiny(experts: bool) -> Vec<u8> {
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
        let v = r.vec(rows * cols, 1.0 / (cols as f32).sqrt() * 1.7);
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
            vector(&mut w, &p("a_log"), (0..4).map(|i| (1.0 + i as f32 * 3.0).ln()).collect());
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

fn tokens(n: usize, seed: u64) -> Vec<u32> {
    let mut r = Rng(seed);
    (0..n).map(|_| ((r.next() + 1.0) * 127.9) as u32).collect()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

#[test]
fn many_tokens_at_once_are_read_as_one_at_a_time() {
    for experts in [false, true] {
        let bytes = tiny(experts);
        let file = File::open(&bytes).unwrap();
        let m = Model::load(&file).unwrap();
        // More than one batch's worth, and positions far enough for attention to matter.
        let toks = tokens(300, 7);
        let (mut one, mut many) = (m.state(), m.state());
        let (mut a, mut b) = (vec![0.0; 300], vec![0.0; 300]);
        for (i, &t) in toks.iter().enumerate() {
            m.step(&mut one, t, (i + 1 == toks.len()).then_some(&mut a[..]), &Serial);
        }
        m.forward(&mut many, &toks, Some(&mut b), &Serial);
        assert_eq!(max_diff(&a, &b), 0.0, "experts: {experts}");
        assert!(a.iter().all(|v| v.is_finite()) && max_diff(&a, &[0.0; 300]) > 0.01);
    }
}

#[test]
fn a_snapshot_goes_back_exactly() {
    let bytes = tiny(false);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let (first, second, other) = (tokens(50, 1), tokens(30, 2), tokens(30, 3));
    let mut st = m.state();
    let mut logits = vec![0.0; 300];
    m.forward(&mut st, &first, None, &Serial);
    let snap = st.snapshot();
    m.forward(&mut st, &second, None, &Serial);
    st.rewind(&snap);
    m.forward(&mut st, &other, Some(&mut logits), &Serial);
    let mut fresh = m.state();
    let mut want = vec![0.0; 300];
    let all: Vec<u32> = first.iter().chain(&other).copied().collect();
    m.forward(&mut fresh, &all, Some(&mut want), &Serial);
    assert_eq!(max_diff(&logits, &want), 0.0);
    assert_eq!(st.pos, 80);
}

#[test]
fn a_session_reads_only_what_is_new_and_answers_the_same() {
    let bytes = tiny(true);
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let ask = |s: &mut Session, msgs: &[Message]| {
        let r = Request { tools: &[], messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 12, seed: 1 };
        s.ask(&r, &Serial, &mut |_, _| true).unwrap()
    };
    let mut s = Session::new(&m, Style::QWEN35);
    let mut msgs = vec![Message::System("Be brief.".into()), Message::User("Hello there".into())];
    let (a1, st1) = ask(&mut s, &msgs);
    assert_eq!(st1.read, st1.prompt);
    msgs.push(Message::Assistant(a1.clone()));
    msgs.push(Message::User("And again".into()));
    let (a2, st2) = ask(&mut s, &msgs);
    // Only the new turn was read: the first prompt and the answer's own tokens were there already.
    assert!(st2.read < st2.prompt - st1.prompt, "{} of {}", st2.read, st2.prompt);
    let mut fresh = Session::new(&m, Style::QWEN35);
    ask(&mut fresh, &msgs[..2]);
    let (b2, _) = ask(&mut fresh, &msgs);
    assert_eq!(a2, b2);
    // The last message changed: from the first prompt's snapshot, and the same answer as a new session's.
    *msgs.last_mut().unwrap() = Message::User("Something else".into());
    let (a3, st3) = ask(&mut s, &msgs);
    assert!(st3.read < st3.prompt);
    let mut fresh = Session::new(&m, Style::QWEN35);
    ask(&mut fresh, &msgs[..2]);
    let (b3, _) = ask(&mut fresh, &msgs);
    assert_eq!(a3, b3);
    // Asked again: nothing to read.
    let (a4, st4) = ask(&mut s, &msgs);
    assert_eq!((a4, st4.read), (a3, 0));
}

#[test]
fn the_kernels_agree() {
    use crate::cpu::Level;
    let mut r = Rng(42);
    for (dtype, cols) in [(Dtype::Q8, 256usize), (Dtype::Q4, 256), (Dtype::Q8, 96), (Dtype::Q4, 32)] {
        let rows = 5;
        let w = r.vec(rows * cols, 1.0);
        let mut data = Vec::new();
        if dtype == Dtype::Q8 { quant::quantize_q8(&w, &mut data) } else { quant::quantize_q4(&w, &mut data) }
        let xs: Vec<Prepared> = (0..9).map(|_| Prepared::new(&r.vec(cols, 2.0))).collect();
        let refs: Vec<&Prepared> = xs.iter().collect();
        for row in 0..rows {
            let mut want = vec![0.0f32; xs.len()];
            quant::dot_row_many_at(Level::Plain, dtype, &data, cols, row, &refs, &mut want);
            // Exactly the same integer sums; the floats only in their order of adding up.
            for level in [Level::Avx2, Level::AvxVnni, Level::Avx512Vnni] {
                let mut got = vec![0.0f32; xs.len()];
                quant::dot_row_many_at(level, dtype, &data, cols, row, &refs, &mut got);
                for (g, e) in got.iter().zip(&want) {
                    assert!((g - e).abs() <= 1e-4 * e.abs().max(1.0), "{dtype:?} {level:?}: {g} vs {e}");
                }
            }
            // And against the numbers themselves, before quantizing.
            for (t, x) in xs.iter().enumerate() {
                let exact: f32 = w[row * cols..(row + 1) * cols].iter().zip(&x.f).map(|(a, b)| a * b).sum();
                let close = if dtype == Dtype::Q8 { 0.02 } else { 0.25 };
                assert!((want[t] - exact).abs() <= close * (cols as f32).sqrt(), "{dtype:?}: {} vs {exact}", want[t]);
            }
        }
    }
}

#[test]
fn a_damaged_file_is_refused() {
    let mut bytes = tiny(false);
    assert!(File::open(&bytes[..100]).is_err());
    let at = bytes.windows(5).position(|w| w == b"embed").unwrap();
    bytes[at..at + 5].copy_from_slice(b"embex");
    let file = File::open(&bytes).unwrap();
    assert!(Model::load(&file).is_err());
    assert_eq!(File::open(b"not a model").err(), Some(crate::Error::NotModel));
}
