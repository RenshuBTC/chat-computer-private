//! Makes a model file (Chat Computer's own format) from a model's published files:
//!
//!     chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]
//!
//! MODEL_FOLDER has Qwen's config.json, tokenizer.json, chat template and its .safetensors weights (or a speech
//! recognition model's: OpenAI's Whisper, as transformers publishes it, with its generation_config.json). Each weight is
//! read, changed and written one at a time, so a model bigger than the computer's memory converts too.

use model::convert;
use model::quant::Dtype;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

struct Planned {
    ours: String,
    dtype: Dtype,
    dims: Vec<usize>,
    size: usize,
    /// Where it comes from: the file, its type there, and its byte range there.
    from: Option<(PathBuf, String, u64, usize)>,
    blob: Vec<u8>,
}

pub fn main_with(args: Vec<String>) -> i32 {
    let mut bits = 8;
    let mut paths = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--bits" {
            bits = args.get(i + 1).and_then(|b| b.parse().ok()).expect("--bits 8 or 4");
            i += 2;
        } else {
            paths.push(args[i].clone());
            i += 1;
        }
    }
    let [folder, out] = &paths[..] else {
        eprintln!("convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]");
        return 2;
    };
    match run(Path::new(folder), Path::new(out), bits) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("convert: {e}");
            1
        }
    }
}

pub fn run(folder: &Path, out: &Path, bits: u32) -> Result<(), String> {
    let read = |n: &str| fs::read_to_string(folder.join(n)).map_err(|e| format!("{n}: {e}"));
    let config = model::json::parse(&read("config.json")?).ok_or("config.json isn't JSON")?;
    let whisper = config.get("model_type").and_then(model::json::Json::str) == Some("whisper");
    let tj = model::json::parse(&read("tokenizer.json")?).ok_or("tokenizer.json isn't JSON")?;
    let settings = if whisper {
        // Speech recognition (Whisper): its settings, the special tokens and those never chosen.
        let generation = model::json::parse(&read("generation_config.json")?).ok_or("generation_config.json isn't JSON")?;
        convert::whisper_settings(&config, &generation, &tj)?
    } else {
        let mut settings = convert::settings(&config)?;
        // The vision part's settings, with how pictures are prepared (if the model's files say).
        let preprocessor = fs::read_to_string(folder.join("preprocessor_config.json")).ok().and_then(|t| model::json::parse(&t));
        settings.push_str(&convert::vision_settings(&config, preprocessor.as_ref()));
        // How the model's conversations are written: its chat template, said as one of the styles chat.rs knows.
        let template = match fs::read_to_string(folder.join("chat_template.jinja")) {
            Ok(t) => t,
            Err(_) => {
                let tc = model::json::parse(&read("tokenizer_config.json")?).ok_or("tokenizer_config.json isn't JSON")?;
                tc.get("chat_template").and_then(model::json::Json::str).ok_or("no chat template")?.to_string()
            }
        };
        let style = model::chat::Style::of_template(&template)
            .ok_or("its chat template isn't one the engine knows how to write (Qwen 3.5, 3.6 or 3.8's)")?;
        settings.push_str(&format!("chat.style={style}\n"));
        settings
    };
    let (tokens, merges, specials) = convert::tokenizer(&tj)?;
    drop(tj);
    let mut planned = vec![
        blob("config", settings.into_bytes()),
        blob("tokenizer.tokens", tokens),
        blob("tokenizer.merges", merges),
        blob("tokenizer.specials", specials.into_bytes()),
    ];
    let mut files: Vec<PathBuf> = fs::read_dir(folder).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    files.sort();
    let mut weights = Vec::new();
    for f in &files {
        let mut h = fs::File::open(f).map_err(|e| e.to_string())?;
        let mut len = [0u8; 8];
        h.read_exact(&mut len).map_err(|e| e.to_string())?;
        let len = u64::from_le_bytes(len) as usize;
        let mut header = vec![0u8; len];
        h.read_exact(&mut header).map_err(|e| e.to_string())?;
        let header = String::from_utf8(header).map_err(|_| "a header isn't UTF-8")?;
        for (name, dtype, dims, a, b) in convert::safetensors(&header)? {
            let plan = if whisper { convert::plan_whisper(&name, &dims, bits) } else { convert::plan(&name, &dims, bits) };
            let Some((ours, to, shape)) = plan else { continue };
            let elements: usize = shape.iter().product();
            let size = to.bytes_for(elements).ok_or_else(|| format!("{name}: not whole blocks"))?;
            weights.push(Planned { ours, dtype: to, dims: shape, size, from: Some((f.clone(), dtype, (8 + len + a) as u64, b - a)), blob: Vec::new() });
        }
    }
    weights.sort_by(|x, y| x.ours.cmp(&y.ours));
    planned.extend(weights);
    // The table: each entry's place, every one on a 64-byte boundary.
    let mut table = String::new();
    let mut at = 0usize;
    for p in &planned {
        at = at.div_ceil(64) * 64;
        let dims = if p.dims.is_empty() { "-".to_string() } else { p.dims.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",") };
        table.push_str(&format!("{} {} {dims} {at} {}\n", p.ours, p.dtype.name(), p.size));
        at += p.size;
    }
    let mut f = std::io::BufWriter::new(fs::File::create(out).map_err(|e| e.to_string())?);
    let mut head = Vec::new();
    head.extend_from_slice(model::file::MAGIC);
    head.extend_from_slice(&(table.len() as u32).to_le_bytes());
    head.extend_from_slice(table.as_bytes());
    while head.len() % 64 != 0 {
        head.push(0);
    }
    f.write_all(&head).map_err(|e| e.to_string())?;
    let mut written = 0usize;
    let started = std::time::Instant::now();
    for (n, p) in planned.iter().enumerate() {
        let pad = written.div_ceil(64) * 64 - written;
        f.write_all(&vec![0u8; pad]).map_err(|e| e.to_string())?;
        written += pad;
        let bytes = match &p.from {
            None => p.blob.clone(),
            Some((file, source, offset, len)) => {
                let mut h = fs::File::open(file).map_err(|e| e.to_string())?;
                h.seek(SeekFrom::Start(*offset)).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; *len];
                h.read_exact(&mut raw).map_err(|e| e.to_string())?;
                convert::encode(source, &raw, p.dtype)?
            }
        };
        if bytes.len() != p.size {
            return Err(format!("{}: {} bytes, not {}", p.ours, bytes.len(), p.size));
        }
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        written += bytes.len();
        if n % 50 == 0 {
            eprintln!("{n}/{} {} ({:.0} s)", planned.len(), p.ours, started.elapsed().as_secs_f64());
        }
    }
    f.flush().map_err(|e| e.to_string())?;
    eprintln!("wrote {} ({} entries, {:.2} GB)", out.display(), planned.len(), (head.len() + written) as f64 / 1e9);
    Ok(())
}

fn blob(name: &str, bytes: Vec<u8>) -> Planned {
    Planned { ours: name.to_string(), dtype: Dtype::Blob, dims: vec![bytes.len()], size: bytes.len(), from: None, blob: bytes }
}
