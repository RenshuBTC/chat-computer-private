//! The engine's speech recognition on a recording, for checking against the reference
//! (tools/refcheck/whisper_reference.py):
//!
//!     cargo run --release -p model --features std --example listen_check -- MODEL.ccm SOUND.wav [--dump PREFIX]
//!         [--language CODE] [--step N] [--level plain|avx2|avx-vnni|avx-512-vnni]
//!
//! Says (JSON) the tokens, the words, the chance it heard no speech and the mean log-likelihood. --dump: also
//! writes the features (PREFIX.mel.f32) and the encoder's output (PREFIX.enc.f32), as the reference does. --step:
//! a step at a time, N positions a step (the transcript must be the same).

use model::whisper::Whisper;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (Some(path), Some(sound)) = (args.first(), args.get(1)) else {
        eprintln!("listen_check MODEL.ccm SOUND.wav [--dump PREFIX] [--language CODE] [--step N]");
        std::process::exit(2);
    };
    // (The arithmetic the own system's kernel used, to compare with it.)
    if let Some(level) = option("--level") {
        model::cpu::at_most(match level.as_str() {
            "plain" => model::cpu::Level::Plain,
            "avx2" => model::cpu::Level::Avx2,
            "avx-vnni" => model::cpu::Level::AvxVnni,
            _ => model::cpu::Level::Avx512Vnni,
        });
    }
    let bytes = std::fs::read(path).expect("the model's file");
    let file = model::File::open(&bytes).expect("a model file");
    let w = Whisper::load(&file).expect("a speech recognition model");
    let samples = read_wav(&std::fs::read(sound).expect("the recording"));
    let pool = model::threads::Threads::for_this_computer();
    if let Some(prefix) = option("--dump") {
        let features = w.features(&samples);
        std::fs::write(format!("{prefix}.mel.f32"), floats(&features)).unwrap();
        let encoded = w.encoded(&samples, &pool);
        std::fs::write(format!("{prefix}.enc.f32"), floats(&encoded)).unwrap();
    }
    let language = option("--language");
    let started = Instant::now();
    let t = match option("--step").and_then(|n| n.parse::<usize>().ok()) {
        Some(n) => {
            // (--profile: how long each step took, a line each.)
            let profile = args.iter().any(|a| a == "--profile");
            let mut h = w.begin(&samples, language.as_deref());
            eprintln!("features: {:.3} s", started.elapsed().as_secs_f64());
            loop {
                let step = Instant::now();
                let done = w.advance(&mut h, n, &pool);
                if profile {
                    eprintln!("step: {:.3} s ({:?})", step.elapsed().as_secs_f64(), w.progress(&h));
                }
                if let Some(t) = done {
                    break t;
                }
            }
        }
        None => w.transcribe(&samples, language.as_deref(), &pool),
    };
    let seconds = started.elapsed().as_secs_f64();
    let tokens: Vec<String> = t.tokens.iter().map(|v| v.to_string()).collect();
    println!(
        "{{\"tokens\": [{}], \"text\": {:?}, \"no_speech\": {}, \"avg_logprob\": {}, \"language\": {:?}, \"seconds\": {seconds:.2}}}",
        tokens.join(", "),
        t.text,
        t.no_speech,
        t.avg_logprob,
        t.language
    );
}

fn floats(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// A WAV file's samples (16-bit PCM, mono, 16 kHz), from -1 to 1.
fn read_wav(b: &[u8]) -> Vec<f32> {
    assert!(b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "not a WAV file");
    let mut at = 12;
    let mut format = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &b[at + 8..(at + 8 + len).min(b.len())];
        if id == b"fmt " {
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            let bits = u16::from_le_bytes([body[14], body[15]]);
            format = Some((channels, rate, bits));
        } else if id == b"data" {
            assert_eq!(format, Some((1, 16000, 16)), "16 kHz mono 16-bit");
            return body.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        }
        at += 8 + len + (len & 1);
    }
    panic!("no data in the WAV file");
}
