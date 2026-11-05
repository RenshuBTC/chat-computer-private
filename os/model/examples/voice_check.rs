//! The engine's voice on some phonemes, for checking against the reference (tools/refcheck/kokoro_reference.py):
//!
//!     cargo run --release -p model --features std --example voice_check -- VOICE.ccm PHONEMES OUT_PREFIX
//!         [--voice NAME] [--speed S] [--seed N] [--level plain|avx2|avx-vnni|avx-512-vnni]
//!
//! PHONEMES: the phonemes themselves, or @FILE. Writes beside OUT_PREFIX, as the reference does: .json (the ids,
//! each one's duration in frames, the frames, how long it took), .f0.f32 and .n.f32 (the pitch and energy curves),
//! .audio.f32 and .wav (24,000 samples a second).

use model::voice::{Voice, RATE};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (Some(path), Some(phonemes), Some(out)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("voice_check VOICE.ccm PHONEMES OUT_PREFIX [--voice NAME] [--speed S] [--seed N]");
        std::process::exit(2);
    };
    if let Some(level) = option("--level") {
        model::cpu::at_most(match level.as_str() {
            "plain" => model::cpu::Level::Plain,
            "avx2" => model::cpu::Level::Avx2,
            "avx-vnni" => model::cpu::Level::AvxVnni,
            _ => model::cpu::Level::Avx512Vnni,
        });
    }
    let phonemes = match phonemes.strip_prefix('@') {
        Some(f) => std::fs::read_to_string(f).expect("the phonemes' file").trim().to_string(),
        None => phonemes.clone(),
    };
    let bytes = std::fs::read(path).expect("the voice's file");
    let file = model::File::open(&bytes).expect("a model file");
    let v = Voice::load(&file).expect("a voice");
    let voice = option("--voice").map(|n| v.voice(&n).expect("no such voice")).unwrap_or(0);
    let speed: f32 = option("--speed").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let seed: u64 = option("--seed").and_then(|s| s.parse().ok()).unwrap_or(0);
    let pool = model::threads::Threads::for_this_computer();
    let started = Instant::now();
    let mut s = v.begin(&phonemes, voice, speed, seed);
    let mut steps = 0;
    let mut times: Vec<(f64, String)> = Vec::new();
    loop {
        let (at, what) = (Instant::now(), s.stage());
        let done = v.advance(&mut s, &pool);
        times.push((at.elapsed().as_secs_f64(), what));
        if done {
            break;
        }
        steps += 1;
    }
    if args.iter().any(|a| a == "--times") {
        // Each kind of step's time, the longest first.
        let mut by: Vec<(String, f64, usize)> = Vec::new();
        for (t, what) in &times {
            let kind: String = what.split(['(', ' ']).next().unwrap_or("").to_string()
                + &what.split_once("Part::").map(|(_, p)| p.to_string()).unwrap_or_default();
            let kind = if let Some(rest) = what.strip_prefix("Generator(") { format!("Generator {}", rest.split(',').skip(1).collect::<String>().split('(').next().unwrap_or("").trim()) } else { kind };
            match by.iter_mut().find(|(k, _, _)| *k == kind) {
                Some(e) => {
                    e.1 += t;
                    e.2 += 1;
                }
                None => by.push((kind, *t, 1)),
            }
        }
        by.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (k, t, n) in by {
            eprintln!("{t:8.3} s  {n:3} x {k}");
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    let audio = std::mem::take(&mut s.audio);
    let floats = |x: &[f32]| x.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    std::fs::write(format!("{out}.audio.f32"), floats(&audio)).unwrap();
    std::fs::write(format!("{out}.f0.f32"), floats(&s.f0)).unwrap();
    std::fs::write(format!("{out}.n.f32"), floats(&s.energy)).unwrap();
    let frames: u32 = s.durations.iter().sum();
    std::fs::write(
        format!("{out}.json"),
        format!(
            "{{\"ids\": {:?}, \"durations\": {:?}, \"frames\": {frames}, \"samples\": {}, \"steps\": {}, \"seconds\": {seconds:.3}}}\n",
            s.ids, s.durations, audio.len(), steps + 1
        ),
    )
    .unwrap();
    // 16-bit WAV.
    let mut wav = Vec::new();
    let data_len = (audio.len() * 2) as u32;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&(RATE as u32).to_le_bytes());
    wav.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for a in &audio {
        wav.extend_from_slice(&((a.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(format!("{out}.wav"), wav).unwrap();
    eprintln!(
        "{} ids, {frames} frames, {} samples ({:.2} s of sound) in {seconds:.2} s ({:.2}x as fast as it plays), {} steps",
        s.ids.len(),
        audio.len(),
        audio.len() as f64 / RATE as f64,
        audio.len() as f64 / RATE as f64 / seconds,
        steps + 1
    );
}
