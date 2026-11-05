//! How fast a model of Qwen3.5 4B's shape reads a prompt and writes, per layer: four of its layers (three of linear
//! attention, one of attention), its width, random Q4 weights. Reading is timed 64 tokens at once (one step of the
//! kernel's), and a token at a time; the matrices' share is worked out from their sizes.
//!
//!     cargo run --release -p model --features std --example bench_forward [THREADS]
use model::file::{File, Writer};
use model::net::{Model, Pool, Serial};
use model::quant::{self, Dtype};
use model::testing::Rng;
use model::threads::Threads;
use std::time::Instant;

fn main() {
    let threads: Option<usize> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let (h, inter, heads, kv, hd, lk, lv, kd, vd) = (2560usize, 9216usize, 16usize, 4usize, 256usize, 16usize, 32usize, 128usize, 128usize);
    let layers = 4;
    let vocab = 300;
    let config = format!(
        "arch=qwen3_5\nhidden={h}\nlayers={layers}\nheads={heads}\nkv_heads={kv}\nhead_dim={hd}\nlinear_key_heads={lk}\n\
         linear_value_heads={lv}\nlinear_key_dim={kd}\nlinear_value_dim={vd}\nconv_kernel=4\nfull_attention_interval=4\n\
         vocab={vocab}\nrope_theta=10000000\nrotary_dim=64\nrms_eps=0.000001\ncontext=262144\nchat.style=qwen3.5\n\
         intermediate={inter}\n"
    );
    let mut r = Rng(7);
    let mut w = Writer::default();
    w.add("config", Dtype::Blob, &[config.len()], config.as_bytes());
    let tokens: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).chain(model::testing::SPECIALS.iter().map(|s| s.as_bytes().to_vec())).collect();
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
    let specials: String = model::testing::SPECIALS.iter().enumerate().map(|(i, s)| format!("{} {s}\n", 256 + i)).collect();
    w.add("tokenizer.specials", Dtype::Blob, &[specials.len()], specials.as_bytes());
    let mut macs = 0usize;
    let mut matrix = |w: &mut Writer, name: &str, rows: usize, cols: usize, r: &mut Rng, count: bool| {
        let v = r.vec(rows * cols, 1.0 / (cols as f32).sqrt() * 1.7);
        let mut q = Vec::new();
        quant::quantize_q4(&v, &mut q);
        w.add(name, Dtype::Q4, &[rows, cols], &q);
        if count {
            macs += rows * cols;
        }
    };
    let f32s = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let vector = |w: &mut Writer, name: &str, v: Vec<f32>| w.add(name, Dtype::F32, &[v.len()], &f32s(&v));
    matrix(&mut w, "embed", vocab, h, &mut r, false);
    matrix(&mut w, "head", vocab, h, &mut r, false);
    vector(&mut w, "norm", r.vec(h, 0.1));
    let conv = 2 * lk * kd + lv * vd;
    for l in 0..layers {
        let p = |n: &str| format!("layers.{l}.{n}");
        vector(&mut w, &p("norm1"), r.vec(h, 0.1));
        vector(&mut w, &p("norm2"), r.vec(h, 0.1));
        if l == layers - 1 {
            matrix(&mut w, &p("q"), heads * hd * 2, h, &mut r, true);
            matrix(&mut w, &p("k"), kv * hd, h, &mut r, true);
            matrix(&mut w, &p("v"), kv * hd, h, &mut r, true);
            matrix(&mut w, &p("o"), h, heads * hd, &mut r, true);
            vector(&mut w, &p("q_norm"), r.vec(hd, 0.1));
            vector(&mut w, &p("k_norm"), r.vec(hd, 0.1));
        } else {
            matrix(&mut w, &p("qkv"), conv, h, &mut r, true);
            matrix(&mut w, &p("z"), lv * vd, h, &mut r, true);
            matrix(&mut w, &p("b"), lv, h, &mut r, true);
            matrix(&mut w, &p("a"), lv, h, &mut r, true);
            w.add(&p("conv"), Dtype::F32, &[conv, 4], &f32s(&r.vec(conv * 4, 0.3)));
            vector(&mut w, &p("a_log"), (0..lv).map(|i| (1.0 + i as f32 * 0.1f32).ln()).collect());
            vector(&mut w, &p("dt_bias"), r.vec(lv, 0.5));
            vector(&mut w, &p("linear_norm"), r.vec(vd, 0.1).iter().map(|v| 1.0 + v).collect());
            matrix(&mut w, &p("out"), h, lv * vd, &mut r, true);
        }
        matrix(&mut w, &p("gate"), inter, h, &mut r, true);
        matrix(&mut w, &p("up"), inter, h, &mut r, true);
        matrix(&mut w, &p("down"), h, inter, &mut r, true);
    }
    let mut bytes = w.head();
    bytes.extend_from_slice(&w.data);
    let file = File::open(&bytes).expect("the file");
    let m = Model::load(&file).expect("the model");
    let pool: Box<dyn Pool> = match threads {
        Some(1) => Box::new(Serial),
        Some(n) => Box::new(Threads::new(n)),
        None => Box::new(Threads::for_this_computer()),
    };
    println!("{} layers of Qwen3.5 4B's shape, {:.0} MB of Q4 weights, {:.1} M multiply-adds a token, arithmetic {:?}",
             layers, bytes.len() as f64 / 1e6, macs as f64 / 1e6, model::cpu::level());
    let prompt: Vec<u32> = (0..256u32).map(|i| (i * 7 + 3) % 256).collect();
    for &n in &[64usize, 16, 4, 1] {
        let mut st = m.state();
        // (A first step, so what's in the caches is as it would be.)
        m.forward(&mut st, &prompt[..n], None, &*pool);
        let reps = (256 / n).max(2);
        let t = Instant::now();
        for k in 0..reps {
            let from = (k * n) % 192;
            m.forward(&mut st, &prompt[from..from + n], None, &*pool);
        }
        let s = t.elapsed().as_secs_f64() / reps as f64;
        let per_token = s / n as f64;
        println!(
            "{n:>3} tokens a step: {:.1} ms a step, {:.2} ms a token, {:.0} tokens a second for these {layers} layers \
             ({:.1} for 32), {:.1} GMAC/s",
            s * 1e3,
            per_token * 1e3,
            1.0 / per_token,
            1.0 / per_token * layers as f64 / 32.0,
            macs as f64 / per_token / 1e9
        );
    }
}
