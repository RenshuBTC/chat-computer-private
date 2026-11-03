//! Runs a model file: a question in, the answer out (as it comes), and how fast.
//!
//!     cargo run --release --features std --example run -- MODEL.ccm "What is the capital of France?" [--tokens 200]
//!         [--think] [--serial] [--ids 1,2,3 --logits OUT.f32]
//!
//! --ids feeds those tokens and writes the scores after each (f32s) to OUT, for checking against a reference.

use model::net::{Pool, Serial};
use model::sample::{Random, Said, Sampling, choose};
use model::threads::Threads;
use model::{File, Model};
use std::io::Write;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut tokens = 200usize;
    let (mut think, mut serial, mut check_batch) = (false, false, false);
    let (mut ids, mut logits_out): (Option<Vec<u32>>, Option<String>) = (None, None);
    let mut tokenize: Option<String> = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tokens" => { tokens = args[i + 1].parse().unwrap(); i += 1; }
            "--think" => think = true,
            "--serial" => serial = true,
            "--check-batch" => check_batch = true,
            "--level" => {
                model::cpu::at_most(match args[i + 1].as_str() {
                    "plain" => model::cpu::Level::Plain,
                    "avx2" => model::cpu::Level::Avx2,
                    _ => model::cpu::Level::Avx512Vnni,
                });
                i += 1;
            }
            "--ids" => { ids = Some(args[i + 1].split(',').map(|t| t.parse().unwrap()).collect()); i += 1; }
            "--logits" => { logits_out = Some(args[i + 1].clone()); i += 1; }
            "--tokenize" => { tokenize = Some(args[i + 1].clone()); i += 1; }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    let started = Instant::now();
    let bytes = std::fs::read(&rest[0]).expect("the model file");
    let file = File::open(&bytes).expect("a model file");
    let m = Model::load(&file).expect("the model");
    eprintln!("loaded in {:.1} s: {} layers, {} hidden, vocab {}", started.elapsed().as_secs_f64(), m.settings.layers, m.settings.hidden, m.settings.vocab);
    if let Some(text) = tokenize {
        let mut toks = Vec::new();
        m.tokenizer.encode_marked(&text, &mut toks);
        println!("{}", toks.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(","));
        return;
    }
    let threads;
    let pool: &dyn Pool = if serial { &Serial } else { threads = Threads::for_this_computer(); &threads };
    let mut st = m.state();
    let mut logits = vec![0.0f32; m.settings.vocab];
    if let Some(ids) = ids {
        let mut out = Vec::new();
        for &t in &ids {
            m.step(&mut st, t, Some(&mut logits), pool);
            for v in &logits {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        std::fs::write(logits_out.unwrap(), out).unwrap();
        return;
    }
    let question = &rest[1];
    let prompt = format!("<|im_start|>user\n{question}<|im_end|>\n<|im_start|>assistant\n{}", if think { "<think>\n" } else { "<think>\n\n</think>\n\n" });
    let mut toks = Vec::new();
    m.tokenizer.encode_marked(&prompt, &mut toks);
    eprintln!("{} prompt tokens", toks.len());
    if check_batch {
        // The same prompt a token at a time: its last scores must be the batch's.
        let mut one = m.state();
        let mut want = vec![0.0f32; m.settings.vocab];
        let t = Instant::now();
        for (k, &tok) in toks.iter().enumerate() {
            m.step(&mut one, tok, if k + 1 == toks.len() { Some(&mut want) } else { None }, pool);
        }
        let single = t.elapsed().as_secs_f64();
        let t = Instant::now();
        m.forward(&mut st, &toks, Some(&mut logits), pool);
        let batched = t.elapsed().as_secs_f64();
        let diff = want.iter().zip(&logits).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        println!("{} tokens: one at a time {:.1} tok/s, together {:.1} tok/s; largest difference {diff}",
            toks.len(), toks.len() as f64 / single, toks.len() as f64 / batched);
        return;
    }
    let t0 = Instant::now();
    m.forward(&mut st, &toks, Some(&mut logits), pool);
    let prefill = t0.elapsed().as_secs_f64();
    let end = m.tokenizer.special("<|im_end|>").unwrap();
    let mut rng = Random(1234567);
    let sampling = Sampling::GREEDY;
    let mut said = Said::new(m.settings.vocab);
    let mut out_bytes: Vec<u8> = Vec::new();
    let t1 = Instant::now();
    let mut n = 0;
    for _ in 0..tokens {
        let next = choose(&logits, &said, &sampling, &mut rng);
        if next == end {
            break;
        }
        said.add(next);
        out_bytes.extend_from_slice(m.tokenizer.bytes(next));
        // Print what's whole UTF-8 so far.
        let valid = match std::str::from_utf8(&out_bytes) { Ok(s) => s.len(), Err(e) => e.valid_up_to() };
        print!("{}", std::str::from_utf8(&out_bytes[..valid]).unwrap());
        out_bytes.drain(..valid);
        std::io::stdout().flush().unwrap();
        m.step(&mut st, next, Some(&mut logits), pool);
        n += 1;
    }
    println!();
    eprintln!("prompt {:.1} tok/s, answer {} tokens at {:.1} tok/s", toks.len() as f64 / prefill, n, n as f64 / t1.elapsed().as_secs_f64());
}
