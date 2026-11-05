//! Kokoro-82M's published files (config.json, kokoro-v1_0.pth and voices/NAME.pt, PyTorch's checkpoints read by
//! pth.rs) made into the engine's own file, as model/src/voice.rs reads it:
//!
//! - the weight-normed weights folded (g · v / |v|, the norm over all but the first dimension);
//! - convolutions as matrices, a row for each output of its inputs × kernel (each row padded with zeros to a
//!   multiple of 32), and the two transposed convolutions that upsample as a matrix for each phase of their output;
//! - the LSTMs' two biases summed; the small weights (the styles' linear layers, the embeddings, norms, the pitch's
//!   own convolutions) in f32, the rest at `bits` (8, 16 or 32; at 8, only the decoder's, the rest at 16);
//! - the phonemes it reads ("kokoro.symbols": "id symbol" lines), and each voice's styles ("kokoro.voice.NAME").

use crate::pth::Checkpoint;
use model::json::Json;
use model::quant::Dtype;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Whether a folder has Kokoro's files.
pub fn is_kokoro(folder: &Path) -> bool {
    folder.join("kokoro-v1_0.pth").is_file()
}

pub fn run(folder: &Path, out: &Path, bits: u32) -> Result<(), String> {
    let matrix_type = match bits {
        8 => Dtype::Q8,
        16 => Dtype::F16,
        32 => Dtype::F32,
        _ => return Err(format!("a voice at {bits} bits: 8, 16 or 32")),
    };
    let text = std::fs::read_to_string(folder.join("config.json")).map_err(|e| format!("config.json: {e}"))?;
    let config = model::json::parse(&text).ok_or("config.json isn't JSON")?;
    let num = |j: Option<&Json>, what: &str| -> Result<usize, String> { j.and_then(Json::usize).ok_or(format!("config.json: {what}")) };
    let list = |j: Option<&Json>, what: &str| -> Result<String, String> {
        let v: Option<Vec<String>> = j.and_then(Json::array).map(|a| a.iter().map(|x| x.usize().map(|n| n.to_string())).collect::<Option<_>>()).flatten();
        Ok(v.ok_or(format!("config.json: {what}"))?.join(","))
    };
    let istft = config.get("istftnet").ok_or("config.json: no istftnet")?;
    let plbert = config.get("plbert").ok_or("config.json: no plbert")?;
    let dilations = istft.get("resblock_dilation_sizes").and_then(Json::array).ok_or("config.json: resblock_dilation_sizes")?;
    for d in dilations {
        if list(Some(d), "a dilation")? != "1,3,5" {
            return Err("its residual blocks' dilations aren't 1, 3 and 5, which the engine's voice has".into());
        }
    }

    // The weights.
    let bytes = std::fs::read(folder.join("kokoro-v1_0.pth")).map_err(|e| format!("kokoro-v1_0.pth: {e}"))?;
    let ckpt = Checkpoint::open(&bytes)?;
    let mut weights: BTreeMap<String, (Vec<usize>, Vec<f32>)> = BTreeMap::new();
    for (name, t) in ckpt.tensors() {
        // ("bert.module.embeddings...": DataParallel's "module." left out.)
        let name = name.replacen(".module.", ".", 1);
        if name.starts_with("bert.pooler.") {
            continue;        // (ALBERT's pooler: not used)
        }
        weights.insert(name, (t.size.clone(), ckpt.floats(&t)?));
    }
    drop(ckpt);
    drop(bytes);
    // Weight norm folded.
    let normed: Vec<String> = weights.keys().filter_map(|k| k.strip_suffix(".weight_v").map(str::to_string)).collect();
    for p in normed {
        let (dims, v) = weights.remove(&format!("{p}.weight_v")).unwrap();
        let (_, g) = weights.remove(&format!("{p}.weight_g")).ok_or(format!("{p}: weight_v without weight_g"))?;
        let rows = dims[0];
        let per = v.len() / rows;
        let mut w = v;
        for r in 0..rows {
            let row = &mut w[r * per..(r + 1) * per];
            let norm = row.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
            let k = if norm > 0.0 { g[r] as f64 / norm } else { 0.0 };
            for x in row.iter_mut() {
                *x = (*x as f64 * k) as f32;
            }
        }
        weights.insert(format!("{p}.weight"), (dims, w));
    }
    // The LSTMs' biases summed.
    let lstm_biases: Vec<String> = weights.keys().filter_map(|k| k.strip_suffix(".bias_ih_l0").map(str::to_string)).collect();
    for p in lstm_biases {
        for suffix in ["", "_reverse"] {
            let (dims, a) = weights.remove(&format!("{p}.bias_ih_l0{suffix}")).ok_or(format!("{p}: no bias_ih_l0{suffix}"))?;
            let (_, b) = weights.remove(&format!("{p}.bias_hh_l0{suffix}")).ok_or(format!("{p}: no bias_hh_l0{suffix}"))?;
            weights.insert(format!("{p}.bias_l0{suffix}"), (dims, a.iter().zip(&b).map(|(x, y)| x + y).collect()));
        }
    }

    let mut w = model::file::Writer::default();
    let mut matrices = 0usize;
    let upsample_rates: Vec<usize> = istft.get("upsample_rates").and_then(Json::array).ok_or("config.json: upsample_rates")?
        .iter().filter_map(Json::usize).collect();
    for (name, (dims, v)) in &weights {
        let small = name.ends_with(".fc.weight")
            || name.contains("_embeddings.")
            || name.starts_with("text_encoder.embedding.")
            || name.ends_with(".pool.weight")
            || name.contains("F0_conv")
            || name.contains("N_conv")
            || name.contains("F0_proj")
            || name.contains("N_proj")
            || name.contains("m_source")
            || name.contains(".weight_hh_l0");
        // (At 8 bits, only the decoder's: the rest, where each phoneme's duration is decided and a rounding error
        // would make a frame more or less, at 16; it's a sixth of the weights and little of the arithmetic.)
        let matrix_type = if matrix_type == Dtype::Q8 && !name.starts_with("decoder.") { Dtype::F16 } else { matrix_type };
        let ups = name.strip_prefix("decoder.generator.ups.").and_then(|r| r.strip_suffix(".weight")).and_then(|i| i.parse::<usize>().ok());
        if let Some(i) = ups {
            // A transposed convolution's weights (inputs × outputs × k): a matrix for each phase of its output, a
            // row for each output, of each input's taps φ + j·stride.
            let (cin, cout, k) = (dims[0], dims[1], dims[2]);
            let s = *upsample_rates.get(i).ok_or("more upsamplings than config.json says")?;
            let taps = k / s;
            let cols = model::voice::padded(cin * taps);
            let mut m = vec![0.0f32; s * cout * cols];
            for phase in 0..s {
                for o in 0..cout {
                    for c in 0..cin {
                        for j in 0..taps {
                            m[(phase * cout + o) * cols + c * taps + j] = v[(c * cout + o) * k + phase + j * s];
                        }
                    }
                }
            }
            w.add(name, matrix_type, &[s * cout, cols], &encode(&m, matrix_type)?);
            matrices += 1;
        } else if !small && (dims.len() == 2 || dims.len() == 3) && (name.ends_with(".weight") || name.contains(".weight_ih_l0")) {
            // A convolution's (outputs × inputs × k) or a linear layer's (outputs × inputs) as a matrix, each row
            // padded to a multiple of 32.
            let rows = dims[0];
            let real = v.len() / rows;
            let cols = model::voice::padded(real);
            let mut m = vec![0.0f32; rows * cols];
            for r in 0..rows {
                m[r * cols..r * cols + real].copy_from_slice(&v[r * real..(r + 1) * real]);
            }
            w.add(name, matrix_type, &[rows, cols], &encode(&m, matrix_type)?);
            matrices += 1;
        } else {
            w.add(name, Dtype::F32, &[v.len()], &encode(v, Dtype::F32)?);
        }
    }

    // The phonemes it reads.
    let mut symbols = String::new();
    let vocab = config.get("vocab").and_then(Json::object).ok_or("config.json: no vocab")?;
    for (sym, id) in vocab {
        let id = id.usize().ok_or("config.json: a phoneme's id")?;
        if sym.chars().count() != 1 || sym.contains('\n') {
            return Err(format!("config.json: a phoneme that isn't one character: {sym:?}"));
        }
        symbols.push_str(&format!("{id} {sym}\n"));
    }
    w.add("kokoro.symbols", Dtype::Blob, &[symbols.len()], symbols.as_bytes());

    // The voices (each [510, 1, 256]).
    let mut names = Vec::new();
    let voices_dir = if folder.join("voices").is_dir() { folder.join("voices") } else { folder.to_path_buf() };
    let mut files: Vec<_> = std::fs::read_dir(&voices_dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "pt"))
        .collect();
    files.sort();
    for f in files {
        let name = f.file_stem().and_then(|s| s.to_str()).ok_or("a voice's name")?.to_string();
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err(format!("a voice's name that isn't plain: {name}"));
        }
        let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let ckpt = Checkpoint::open(&bytes)?;
        let crate::pth::Value::Tensor(t) = &ckpt.root else { return Err(format!("{}: not a tensor", f.display())) };
        let v = ckpt.floats(t)?;
        if v.len() != model::voice::MOST_PHONEMES * 256 {
            return Err(format!("{}: {} numbers, not a voice's", f.display(), v.len()));
        }
        w.add(&format!("kokoro.voice.{name}"), Dtype::F32, &[model::voice::MOST_PHONEMES, 256], &encode(&v, Dtype::F32)?);
        names.push(name);
    }
    if names.is_empty() {
        return Err("no voices (voices/NAME.pt)".into());
    }

    let word = weights.get("bert.embeddings.word_embeddings.weight").ok_or("no bert.embeddings.word_embeddings")?;
    let settings = format!(
        "converter={}\narch=kokoro\nkokoro.vocab={}\nkokoro.hidden={}\nkokoro.style={}\nkokoro.layers={}\nkokoro.max_dur={}\n\
         kokoro.text_kernel={}\nkokoro.bert_hidden={}\nkokoro.bert_embed={}\nkokoro.bert_heads={}\nkokoro.bert_layers={}\n\
         kokoro.bert_ffn={}\nkokoro.bert_positions={}\nkokoro.upsample_rates={}\nkokoro.upsample_kernels={}\n\
         kokoro.upsample_initial={}\nkokoro.resblock_kernels={}\nkokoro.n_fft={}\nkokoro.hop={}\nkokoro.voices={}\n",
        model::convert::CONVERTER,
        num(config.get("n_token"), "n_token")?,
        num(config.get("hidden_dim"), "hidden_dim")?,
        num(config.get("style_dim"), "style_dim")?,
        num(config.get("n_layer"), "n_layer")?,
        num(config.get("max_dur"), "max_dur")?,
        num(config.get("text_encoder_kernel_size"), "text_encoder_kernel_size")?,
        num(plbert.get("hidden_size"), "plbert.hidden_size")?,
        word.0[1],
        num(plbert.get("num_attention_heads"), "plbert.num_attention_heads")?,
        num(plbert.get("num_hidden_layers"), "plbert.num_hidden_layers")?,
        num(plbert.get("intermediate_size"), "plbert.intermediate_size")?,
        num(plbert.get("max_position_embeddings"), "plbert.max_position_embeddings")?,
        list(istft.get("upsample_rates"), "upsample_rates")?,
        list(istft.get("upsample_kernel_sizes"), "upsample_kernel_sizes")?,
        num(istft.get("upsample_initial_channel"), "upsample_initial_channel")?,
        list(istft.get("resblock_kernel_sizes"), "resblock_kernel_sizes")?,
        num(istft.get("gen_istft_n_fft"), "gen_istft_n_fft")?,
        num(istft.get("gen_istft_hop_size"), "gen_istft_hop_size")?,
        names.join(","),
    );
    w.add("config", Dtype::Blob, &[settings.len()], settings.as_bytes());

    let mut f = std::io::BufWriter::new(std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?);
    f.write_all(&w.head()).map_err(|e| e.to_string())?;
    f.write_all(&w.data).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    eprintln!("wrote {} ({} weights, {} of them matrices at {bits} bits; voices {}; {:.2} GB)", out.display(), weights.len(),
              matrices, names.join(", "), (w.head().len() + w.data.len()) as f64 / 1e9);
    Ok(())
}

fn encode(v: &[f32], dtype: Dtype) -> Result<Vec<u8>, String> {
    let mut raw = Vec::with_capacity(v.len() * 4);
    for x in v {
        raw.extend_from_slice(&x.to_le_bytes());
    }
    model::convert::encode("F32", &raw, dtype)
}
