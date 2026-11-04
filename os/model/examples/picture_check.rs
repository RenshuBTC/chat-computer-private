//! A conversation with a picture, the engine against the reference (tools/refcheck/picture_reference.py): the same
//! picture and question, the prompt's tokens as the processor made them (the picture's rows in the place of its pad
//! tokens), the scores at the prompt's last positions, and the answer written with always the likeliest token.
//!
//!     cargo run --release --features std --example picture_check -- MODEL.ccm PICTURE.rgb WIDTH HEIGHT REF_PREFIX "the question"
//!
//! (Without the reference's files, it gives its answer at length, and how long it took.)
use model::chat::{Content, Message, Options, Picture, Style};
use model::file::Writer;
use model::net;
use model::quant::Dtype;
use model::sample::Sampling;
use model::session::{Request, Session, Step};
use model::{File, Model};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

/// A model file of only the vision part of the published files in `folder`, converted as the convert command does at
/// `bits` a weight.
fn vision_file(folder: &str, bits: u32) -> Vec<u8> {
    let config = model::json::parse(&std::fs::read_to_string(format!("{folder}/config.json")).unwrap()).unwrap();
    let pre = std::fs::read_to_string(format!("{folder}/preprocessor_config.json")).ok().and_then(|t| model::json::parse(&t));
    let mut w = Writer::default();
    w.add("config", Dtype::Blob, &[], model::convert::vision_settings(&config, pre.as_ref()).as_bytes());
    let mut f = std::fs::File::open(format!("{folder}/model.safetensors-00001-of-00001.safetensors")).unwrap();
    let mut len = [0u8; 8];
    f.read_exact(&mut len).unwrap();
    let len = u64::from_le_bytes(len) as usize;
    let mut header = vec![0u8; len];
    f.read_exact(&mut header).unwrap();
    for (name, dtype, dims, a, b) in model::convert::safetensors(&String::from_utf8(header).unwrap()).unwrap() {
        let Some((ours, mut to, shape)) = model::convert::plan(&name, &dims, bits).filter(|p| p.0.starts_with("vision")) else { continue };
        // (CC_VISION_EXACT: which of the weights to keep in 16 bits whatever the bits, as vision_check has it.)
        if let Ok(exact) = std::env::var("CC_VISION_EXACT")
            && to == Dtype::Q8
            && exact.split(',').any(|e| !e.is_empty() && ours.ends_with(e))
        {
            to = Dtype::BF16;
        }
        f.seek(SeekFrom::Start((8 + len + a) as u64)).unwrap();
        let mut raw = vec![0u8; b - a];
        f.read_exact(&mut raw).unwrap();
        w.add(&ours, to, &shape, &model::convert::encode(&dtype, &raw, to).unwrap());
    }
    let mut bytes = w.head();
    bytes.extend_from_slice(&w.data);
    bytes
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (path, rgb, width, height, prefix, question) = (&a[0], &a[1], a[2].parse().unwrap(), a[3].parse().unwrap(), &a[4], &a[5]);
    let bytes: &'static [u8] = Box::leak(std::fs::read(path).unwrap().into_boxed_slice());
    let file: &'static File = Box::leak(Box::new(File::open(bytes).unwrap()));
    let mut model = Model::load(file).unwrap();
    // (CC_VISION_FROM=FOLDER, CC_VISION_BITS=16|8: the vision part converted from the published files instead, at
    // those bits, the language model still the file's: to tell the vision part's rounding from the language model's.)
    if let Ok(folder) = std::env::var("CC_VISION_FROM") {
        let bits = std::env::var("CC_VISION_BITS").ok().and_then(|b| b.parse().ok()).unwrap_or(16);
        let vision_bytes: &'static [u8] = Box::leak(vision_file(&folder, bits).into_boxed_slice());
        let vision_file: &'static File = Box::leak(Box::new(File::open(vision_bytes).unwrap()));
        model.vision = model::vision::Vision::load(vision_file).unwrap();
        println!("the vision part from {folder}, at {bits} bits");
    }
    assert!(model.vision.is_some(), "the model's file has no vision part");
    let style = Style::named(file.config().unwrap().str("chat.style").unwrap_or("qwen3.5")).unwrap();
    let pool = model::threads::Threads::for_this_computer();
    let picture = Arc::new(Picture::new(width, height, std::fs::read(rgb).unwrap()));
    let messages = vec![Message::UserParts(vec![Content::Picture(picture), Content::Text(question.clone())])];
    let options = Options { think: Some(false), ..Options::default() };
    let request = Request { tools: &[], messages: &messages, options, sampling: Sampling::GREEDY, max_tokens: 64, seed: 1 };
    let mut session = Session::new(&model, style);
    // (No reference: only its answer, at length, and how long it took.)
    if !std::path::Path::new(&format!("{prefix}.ids")).exists() {
        let started = std::time::Instant::now();
        let request = Request { max_tokens: 200, ..request };
        let (answer, stats) = session.ask(&request, &pool, &mut |_, _| true).unwrap();
        println!("{} tokens read, {} written, in {:.1} s: {:?}", stats.prompt, stats.generated, started.elapsed().as_secs_f64(), answer.text);
        return;
    }
    let tokens = session.prompt(&request).unwrap();
    // The tokens, the picture's rows as the pad token they stand for.
    let pad = model.special("<|image_pad|>").unwrap();
    let ours: Vec<u32> = tokens.iter().map(|&t| if net::picture_of(t).is_some() { pad } else { t }).collect();
    let theirs: Vec<u32> = std::fs::read_to_string(format!("{prefix}.ids")).unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
    println!("prompt: {} tokens ({} of the picture's); the same as the reference's: {}", ours.len(), ours.iter().filter(|&&t| t == pad).count(), ours == theirs);
    assert_eq!(ours, theirs, "the prompt's tokens");
    // The scores at the last positions: read all but those, then each in turn.
    let reference: Vec<f32> = std::fs::read(format!("{prefix}.last.f32")).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    let vocab = model.settings.vocab;
    let last = reference.len() / vocab;
    let started = std::time::Instant::now();
    session.read(&tokens[..tokens.len() - last], &pool);
    let mut worst: f32 = 0.0;
    let (mut kl_sum, mut same_top) = (0.0f64, 0);
    for k in 0..last {
        let upto = tokens.len() - last + k + 1;
        session.read(&tokens[..upto], &pool);
        let ours = session.scores();
        let theirs = &reference[k * vocab..(k + 1) * vocab];
        worst = worst.max(ours.iter().zip(theirs).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max));
        let soft = |l: &[f32]| -> Vec<f64> {
            let m = l.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)) as f64;
            let e: Vec<f64> = l.iter().map(|&v| (v as f64 - m).exp()).collect();
            let s: f64 = e.iter().sum();
            e.into_iter().map(|v| v / s).collect()
        };
        let (p, q) = (soft(theirs), soft(ours));
        kl_sum += p.iter().zip(&q).filter(|(a, _)| **a > 0.0).map(|(a, b)| a * (a / b.max(1e-30)).ln()).sum::<f64>();
        let top = |l: &[f32]| l.iter().enumerate().fold((0, f32::NEG_INFINITY), |m, (i, &v)| if v > m.1 { (i, v) } else { m }).0;
        same_top += (top(ours) == top(theirs)) as usize;
    }
    println!(
        "the last {last} positions' scores (read in {:.1} s): within {worst}, mean KL {:.5}, the same likeliest token at {same_top} of {last}",
        started.elapsed().as_secs_f64(),
        kl_sum / last as f64
    );
    // The answer.
    let mut t = session.begin(&request).unwrap();
    let mut answer = Vec::new();
    while answer.len() < 24 {
        match session.step(&mut t, usize::MAX, &pool) {
            Step::Over => break,
            Step::Read { .. } | Step::Seeing { .. } => {}
            Step::Said(_) => answer = t.generated().to_vec(),
        }
        if t.finished() {
            break;
        }
    }
    let theirs: Vec<u32> = std::fs::read_to_string(format!("{prefix}.answer")).unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
    let n = answer.len().min(theirs.len());
    let agree = answer.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
    let words: Vec<u8> = answer.iter().flat_map(|&t| model.tokenizer.bytes(t).to_vec()).collect();
    println!("its answer: {:?}", String::from_utf8_lossy(&words));
    println!("the same as the reference's for {agree} of its first {n} tokens");
}
