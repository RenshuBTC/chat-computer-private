//! From the model's published files (Qwen's config.json, tokenizer.json and safetensors weights) to its file in
//! Chat Computer's own format: the settings it needs, the tokenizer's tables, and each weight renamed, reshaped
//! and stored as 8 or 4 bits a weight (the small ones, and what the model is most sensitive to, kept more exactly).
//! The vision part is kept too (in 8 bits, or 16), with how pictures are to be prepared (preprocessor_config.json).

use crate::json::Json;
use crate::quant::{self, Dtype};
use crate::token::byte_level_table;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Which converter made a file, as its settings say ("converter=2"): 2 since the vision part is kept. A file
/// without the setting was made before (by 1), without the vision part: the model can't see with it, so Chat
/// Computer's apps make it again (a model that sees, with none of that part, made by 1).
pub const CONVERTER: u32 = 2;

/// The settings, as "key=value" lines, from config.json (its text model's part), after which converter made them.
pub fn settings(config: &Json) -> Result<String, String> {
    let t = config.get("text_config").unwrap_or(config);
    let num = |k: &str| t.get(k).and_then(Json::usize).ok_or_else(|| format!("config.json has no {k}"));
    let arch = config.get("model_type").and_then(Json::str).unwrap_or("");
    if !arch.starts_with("qwen3_5") && !arch.starts_with("qwen3_next") {
        return Err(format!("a {arch} model, which the engine doesn't run (Qwen 3.5's family only)"));
    }
    let head_dim = num("head_dim")?;
    let rope = t.get("rope_parameters").or_else(|| t.get("rope_scaling"));
    let theta = rope.and_then(|r| r.get("rope_theta")).or_else(|| t.get("rope_theta")).and_then(Json::num).ok_or("no rope_theta")?;
    let factor = rope.and_then(|r| r.get("partial_rotary_factor")).or_else(|| t.get("partial_rotary_factor"))
        .and_then(Json::num).unwrap_or(1.0);
    let interval = num("full_attention_interval")?;
    // The layers' kinds must be the pattern the engine knows: three linear, then one full, over and over.
    if let Some(kinds) = t.get("layer_types").and_then(Json::array) {
        for (i, k) in kinds.iter().enumerate() {
            let full = k.str() == Some("full_attention");
            if full != (i + 1).is_multiple_of(interval) {
                return Err(format!("layer {i}'s kind isn't the usual pattern"));
            }
        }
    }
    let mut s = String::new();
    let mut put = |k: &str, v: String| {
        s.push_str(k);
        s.push('=');
        s.push_str(&v);
        s.push('\n');
    };
    put("converter", CONVERTER.to_string());
    put("arch", arch.to_string());
    put("hidden", num("hidden_size")?.to_string());
    put("layers", num("num_hidden_layers")?.to_string());
    put("heads", num("num_attention_heads")?.to_string());
    put("kv_heads", num("num_key_value_heads")?.to_string());
    put("head_dim", head_dim.to_string());
    if let Ok(e) = num("num_experts") {
        put("experts", e.to_string());
        put("experts_used", num("num_experts_per_tok")?.to_string());
        put("expert_intermediate", num("moe_intermediate_size")?.to_string());
        put("shared_intermediate", num("shared_expert_intermediate_size")?.to_string());
    } else {
        put("intermediate", num("intermediate_size")?.to_string());
    }
    put("linear_key_heads", num("linear_num_key_heads")?.to_string());
    put("linear_value_heads", num("linear_num_value_heads")?.to_string());
    put("linear_key_dim", num("linear_key_head_dim")?.to_string());
    put("linear_value_dim", num("linear_value_head_dim")?.to_string());
    put("conv_kernel", num("linear_conv_kernel_dim")?.to_string());
    put("full_attention_interval", interval.to_string());
    put("vocab", num("vocab_size")?.to_string());
    put("rope_theta", format!("{theta}"));
    put("rotary_dim", ((head_dim as f64 * factor) as usize).to_string());
    put("rms_eps", format!("{}", t.get("rms_norm_eps").and_then(Json::num).unwrap_or(1e-6)));
    put("context", num("max_position_embeddings").unwrap_or(32768).to_string());
    // Positions for pictures (M-RoPE): which of the rotary pairs turn by a picture's row, which by its column.
    if let Some(sections) = rope.and_then(|r| r.get("mrope_section")).and_then(Json::array) {
        let parts: Vec<String> = sections.iter().filter_map(Json::usize).map(|v| v.to_string()).collect();
        put("mrope_section", parts.join(","));
        let interleaved = rope.and_then(|r| r.get("mrope_interleaved")).is_some_and(|v| matches!(v, Json::Bool(true)));
        put("mrope_interleaved", (interleaved as u8).to_string());
    }
    Ok(s)
}

/// The vision part's settings, as more "key=value" lines (none if the model has no vision part, or one the engine
/// doesn't run): from config.json's vision_config, and how pictures are prepared (preprocessor_config.json, if
/// there is one; else Qwen 3.5's).
pub fn vision_settings(config: &Json, preprocessor: Option<&Json>) -> String {
    let Some(v) = config.get("vision_config") else { return String::new() };
    // (Features taken from the middle of the vision part into the language model's first layers: not run here.)
    if v.get("deepstack_visual_indexes").and_then(Json::array).is_some_and(|a| !a.is_empty()) {
        return String::new();
    }
    let num = |k: &str| v.get(k).and_then(Json::usize);
    let (Some(depth), Some(hidden), Some(heads), Some(inter), Some(patch), Some(temporal), Some(merge), Some(positions), Some(out)) = (
        num("depth"),
        num("hidden_size"),
        num("num_heads"),
        num("intermediate_size"),
        num("patch_size"),
        num("temporal_patch_size"),
        num("spatial_merge_size"),
        num("num_position_embeddings"),
        num("out_hidden_size"),
    ) else {
        return String::new();
    };
    let ids = |k: &str| config.get(k).and_then(Json::usize);
    let (Some(image), Some(start), Some(end)) = (ids("image_token_id"), ids("vision_start_token_id"), ids("vision_end_token_id")) else {
        return String::new();
    };
    let mut s = String::new();
    let mut put = |k: &str, v: String| {
        s.push_str("vision.");
        s.push_str(k);
        s.push('=');
        s.push_str(&v);
        s.push('\n');
    };
    put("depth", depth.to_string());
    put("hidden", hidden.to_string());
    put("heads", heads.to_string());
    put("intermediate", inter.to_string());
    put("patch", patch.to_string());
    put("temporal", temporal.to_string());
    put("merge", merge.to_string());
    put("positions", positions.to_string());
    put("out", out.to_string());
    let theta = v.get("rope_parameters").and_then(|r| r.get("rope_theta")).and_then(Json::num).unwrap_or(10000.0);
    put("rope_theta", format!("{theta}"));
    put("image_token", image.to_string());
    put("start_token", start.to_string());
    put("end_token", end.to_string());
    let p = preprocessor;
    let size = p.and_then(|p| p.get("size"));
    let min = size.and_then(|s| s.get("shortest_edge")).or_else(|| p.and_then(|p| p.get("min_pixels"))).and_then(Json::usize);
    let max = size.and_then(|s| s.get("longest_edge")).or_else(|| p.and_then(|p| p.get("max_pixels"))).and_then(Json::usize);
    put("min_pixels", min.unwrap_or(65536).to_string());
    put("max_pixels", max.unwrap_or(16_777_216).to_string());
    let three = |k: &str| -> String {
        let vals: Vec<String> = p.and_then(|p| p.get(k)).and_then(Json::array).map(|a| a.iter().filter_map(Json::num).map(|x| format!("{x}")).collect()).unwrap_or_default();
        if vals.len() == 3 { vals.join(",") } else { String::from("0.5,0.5,0.5") }
    };
    put("mean", three("image_mean"));
    put("std", three("image_std"));
    s
}

/// The tokenizer's tables, from tokenizer.json: every token's bytes (count, offsets, bytes), the merges (left,
/// right, merged ids), and the special tokens ("id text" lines).
pub fn tokenizer(tj: &Json) -> Result<(Vec<u8>, Vec<u8>, String), String> {
    let model = tj.get("model").ok_or("tokenizer.json has no model")?;
    if model.get("type").and_then(Json::str) != Some("BPE") {
        return Err("not a BPE tokenizer".into());
    }
    let vocab = model.get("vocab").and_then(Json::object).ok_or("no vocab")?;
    // The byte-level alphabet back to bytes.
    let table = byte_level_table();
    let mut back: BTreeMap<char, u8> = BTreeMap::new();
    for (b, &c) in table.iter().enumerate() {
        back.insert(c, b as u8);
    }
    let mut ids: BTreeMap<&str, u32> = BTreeMap::new();
    let mut tokens: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for (text, id) in vocab {
        let id = id.usize().ok_or("a vocab id isn't a number")? as u32;
        let bytes: Option<Vec<u8>> = text.chars().map(|c| back.get(&c).copied()).collect();
        tokens.insert(id, bytes.ok_or_else(|| format!("token {text:?} isn't in the byte alphabet"))?);
        ids.insert(text.as_str(), id);
    }
    let mut specials = String::new();
    for a in tj.get("added_tokens").and_then(Json::array).unwrap_or(&[]) {
        let id = a.get("id").and_then(Json::usize).ok_or("an added token without an id")? as u32;
        let text = a.get("content").and_then(Json::str).ok_or("an added token without text")?;
        tokens.insert(id, text.as_bytes().to_vec());
        specials.push_str(&format!("{id} {text}\n"));
    }
    let count = tokens.keys().next_back().map_or(0, |&m| m as usize + 1);
    let mut blob = Vec::new();
    blob.extend_from_slice(&(count as u32).to_le_bytes());
    let mut offsets = Vec::with_capacity(count + 1);
    let mut bytes = Vec::new();
    offsets.push(0u32);
    for id in 0..count as u32 {
        if let Some(t) = tokens.get(&id) {
            bytes.extend_from_slice(t);
        }
        offsets.push(bytes.len() as u32);
    }
    for o in &offsets {
        blob.extend_from_slice(&o.to_le_bytes());
    }
    blob.extend_from_slice(&bytes);
    let mut merges = Vec::new();
    for m in model.get("merges").and_then(Json::array).ok_or("no merges")? {
        // "a b", or ["a", "b"].
        let (l, r) = match m {
            Json::Str(s) => s.split_once(' ').ok_or("a merge without a space")?,
            Json::Array(p) if p.len() == 2 => (p[0].str().ok_or("a merge's part")?, p[1].str().ok_or("a merge's part")?),
            _ => return Err("a merge isn't a pair".into()),
        };
        let joined = format!("{l}{r}");
        let (Some(&a), Some(&b), Some(&c)) = (ids.get(l), ids.get(r), ids.get(joined.as_str())) else {
            return Err(format!("merge {l:?} {r:?} names a token that isn't there"));
        };
        merges.extend_from_slice(&a.to_le_bytes());
        merges.extend_from_slice(&b.to_le_bytes());
        merges.extend_from_slice(&c.to_le_bytes());
    }
    Ok((blob, merges, specials))
}

/// A weight in the published files: where it goes in ours (its name, type and shape), or None to leave it out
/// (the extra layer for predicting further ahead).
pub fn plan(name: &str, dims: &[usize], bits: u32) -> Option<(String, Dtype, Vec<usize>)> {
    let n = name.strip_prefix("model.language_model.").or_else(|| name.strip_prefix("model."))
        .unwrap_or(name);
    if let Some(v) = name.strip_prefix("model.visual.").or_else(|| name.strip_prefix("visual.")) {
        return plan_vision(v, dims, bits);
    }
    if name.starts_with("mtp.") {
        return None;
    }
    // (16: every weight as published, BF16, for checking against the reference; 8 and 4: quantized.)
    let exact = bits == 16;
    let big = match bits { 4 => Dtype::Q4, 16 => Dtype::BF16, _ => Dtype::Q8 };
    let q8 = if exact { Dtype::BF16 } else { Dtype::Q8 };
    let flat: Vec<usize> = dims.to_vec();
    let rows_of = |d: &[usize]| -> Vec<usize> {
        // (A stack of matrices: one tall matrix.)
        if d.len() == 3 { alloc::vec![d[0] * d[1], d[2]] } else { d.to_vec() }
    };
    let simple = match n {
        "embed_tokens.weight" => Some(("embed".to_string(), q8, flat.clone())),
        "norm.weight" => Some(("norm".to_string(), Dtype::F32, flat.clone())),
        _ => None,
    };
    if simple.is_some() {
        return simple;
    }
    if name == "lm_head.weight" {
        return Some(("head".to_string(), q8, flat));
    }
    let rest = n.strip_prefix("layers.")?;
    let (layer, part) = rest.split_once('.')?;
    let layer: usize = layer.parse().ok()?;
    let (ours, dtype, shape) = match part {
        "input_layernorm.weight" => ("norm1", Dtype::F32, flat),
        "post_attention_layernorm.weight" => ("norm2", Dtype::F32, flat),
        "linear_attn.in_proj_qkv.weight" => ("qkv", big, flat),
        "linear_attn.in_proj_z.weight" => ("z", big, flat),
        "linear_attn.in_proj_b.weight" => ("b", q8, flat),
        "linear_attn.in_proj_a.weight" => ("a", q8, flat),
        "linear_attn.conv1d.weight" => ("conv", Dtype::F32, alloc::vec![dims[0], *dims.last()?]),
        "linear_attn.A_log" => ("a_log", Dtype::F32, flat),
        "linear_attn.dt_bias" => ("dt_bias", Dtype::F32, flat),
        "linear_attn.norm.weight" => ("linear_norm", Dtype::F32, flat),
        "linear_attn.out_proj.weight" => ("out", big, flat),
        "self_attn.q_proj.weight" => ("q", big, flat),
        "self_attn.k_proj.weight" => ("k", big, flat),
        "self_attn.v_proj.weight" => ("v", big, flat),
        "self_attn.o_proj.weight" => ("o", big, flat),
        "self_attn.q_norm.weight" => ("q_norm", Dtype::F32, flat),
        "self_attn.k_norm.weight" => ("k_norm", Dtype::F32, flat),
        "mlp.gate_proj.weight" => ("gate", big, flat),
        "mlp.up_proj.weight" => ("up", big, flat),
        "mlp.down_proj.weight" => ("down", big, flat),
        "mlp.gate.weight" => ("router", Dtype::F32, flat),
        "mlp.experts.gate_up_proj" => ("experts_gate_up", big, rows_of(dims)),
        "mlp.experts.down_proj" => ("experts_down", big, rows_of(dims)),
        "mlp.shared_expert.gate_proj.weight" => ("shared_gate", big, flat),
        "mlp.shared_expert.up_proj.weight" => ("shared_up", big, flat),
        "mlp.shared_expert.down_proj.weight" => ("shared_down", big, flat),
        "mlp.shared_expert_gate.weight" => ("shared_weight", Dtype::F32, flat),
        _ => return None,
    };
    // Quantized only where each row is whole blocks.
    let dtype = if matches!(dtype, Dtype::Q8 | Dtype::Q4) && !shape.last().is_some_and(|c| c.is_multiple_of(quant::BLOCK)) {
        Dtype::F32
    } else {
        dtype
    };
    Some((format!("layers.{layer}.{ours}"), dtype, shape))
}

/// A vision weight (its name after "model.visual."): where it goes in ours. Its matrices in 8 bits even when the
/// language model's are in 4 (seeing suffers more from rounding, and the vision part is small), or 16 as published.
fn plan_vision(v: &str, dims: &[usize], bits: u32) -> Option<(String, Dtype, Vec<usize>)> {
    let m = if bits == 16 { Dtype::BF16 } else { Dtype::Q8 };
    let flat = dims.to_vec();
    // (The patch embedding's convolution, as a matrix: each output's weights over a patch's numbers. Always in 16
    // bits: a picture's pixels and these weights each rounded to 8 bits took the rows twice as far from the
    // reference's, for a hundredth of the vision part's arithmetic.)
    let as_matrix = |d: &[usize]| -> Vec<usize> { alloc::vec![d[0], d[1..].iter().product()] };
    let (ours, dtype, shape) = match v {
        "patch_embed.proj.weight" if dims.len() >= 2 => ("vision.patch".to_string(), Dtype::BF16, as_matrix(dims)),
        "patch_embed.proj.bias" => ("vision.patch_bias".to_string(), Dtype::F32, flat),
        "pos_embed.weight" => ("vision.pos".to_string(), m, flat),
        "merger.norm.weight" => ("vision.merger.norm".to_string(), Dtype::F32, flat),
        "merger.norm.bias" => ("vision.merger.norm_bias".to_string(), Dtype::F32, flat),
        "merger.linear_fc1.weight" => ("vision.merger.fc1".to_string(), m, flat),
        "merger.linear_fc1.bias" => ("vision.merger.fc1_bias".to_string(), Dtype::F32, flat),
        "merger.linear_fc2.weight" => ("vision.merger.fc2".to_string(), m, flat),
        "merger.linear_fc2.bias" => ("vision.merger.fc2_bias".to_string(), Dtype::F32, flat),
        _ => {
            let rest = v.strip_prefix("blocks.")?;
            let (block, part) = rest.split_once('.')?;
            let block: usize = block.parse().ok()?;
            let (ours, dtype) = match part {
                "norm1.weight" => ("norm1", Dtype::F32),
                "norm1.bias" => ("norm1_bias", Dtype::F32),
                "norm2.weight" => ("norm2", Dtype::F32),
                "norm2.bias" => ("norm2_bias", Dtype::F32),
                "attn.qkv.weight" => ("qkv", m),
                "attn.qkv.bias" => ("qkv_bias", Dtype::F32),
                "attn.proj.weight" => ("proj", m),
                "attn.proj.bias" => ("proj_bias", Dtype::F32),
                "mlp.linear_fc1.weight" => ("fc1", m),
                "mlp.linear_fc1.bias" => ("fc1_bias", Dtype::F32),
                "mlp.linear_fc2.weight" => ("fc2", m),
                "mlp.linear_fc2.bias" => ("fc2_bias", Dtype::F32),
                _ => return None,
            };
            (format!("vision.{block}.{ours}"), dtype, flat)
        }
    };
    let dtype = if matches!(dtype, Dtype::Q8 | Dtype::Q4) && !shape.last().is_some_and(|c| c.is_multiple_of(quant::BLOCK)) {
        Dtype::F32
    } else {
        dtype
    };
    Some((ours, dtype, shape))
}

/// A speech recognition model's settings (OpenAI's Whisper, as transformers publishes it), as "key=value" lines:
/// from config.json, generation_config.json (the tokens never chosen, and never first) and tokenizer.json (the
/// special tokens, by name).
pub fn whisper_settings(config: &Json, generation: &Json, tj: &Json) -> Result<String, String> {
    let num = |k: &str| config.get(k).and_then(Json::usize).ok_or_else(|| format!("config.json has no {k}"));
    let mut specials: BTreeMap<String, usize> = BTreeMap::new();
    for a in tj.get("added_tokens").and_then(Json::array).unwrap_or(&[]) {
        if let (Some(id), Some(text)) = (a.get("id").and_then(Json::usize), a.get("content").and_then(Json::str)) {
            specials.insert(text.to_string(), id);
        }
    }
    let special = |names: &[&str]| -> Result<usize, String> {
        names.iter().find_map(|n| specials.get(*n).copied()).ok_or_else(|| format!("the tokenizer has no {}", names[0]))
    };
    let blank = tj.get("model").and_then(|m| m.get("vocab")).and_then(|v| v.get("\u{120}")).and_then(Json::usize)
        .ok_or("the tokenizer has no blank (a space alone)")?;
    let ids = |k: &str| -> Vec<String> {
        generation.get(k).and_then(Json::array).map(|a| a.iter().filter_map(Json::usize).map(|v| v.to_string()).collect()).unwrap_or_default()
    };
    let mut s = String::new();
    let mut put = |k: &str, v: String| {
        s.push_str(k);
        s.push('=');
        s.push_str(&v);
        s.push('\n');
    };
    put("converter", CONVERTER.to_string());
    put("arch", "whisper".to_string());
    put("whisper.mels", num("num_mel_bins")?.to_string());
    put("whisper.width", num("d_model")?.to_string());
    put("whisper.encoder_layers", num("encoder_layers")?.to_string());
    put("whisper.decoder_layers", num("decoder_layers")?.to_string());
    put("whisper.encoder_heads", num("encoder_attention_heads")?.to_string());
    put("whisper.decoder_heads", num("decoder_attention_heads")?.to_string());
    put("whisper.encoder_ffn", num("encoder_ffn_dim")?.to_string());
    put("whisper.decoder_ffn", num("decoder_ffn_dim")?.to_string());
    put("whisper.vocab", num("vocab_size")?.to_string());
    put("whisper.audio_ctx", num("max_source_positions")?.to_string());
    put("whisper.text_ctx", num("max_target_positions")?.to_string());
    put("whisper.sot", special(&["<|startoftranscript|>"])?.to_string());
    put("whisper.eot", special(&["<|endoftext|>"])?.to_string());
    let no_timestamps = special(&["<|notimestamps|>"])?;
    put("whisper.no_timestamps", no_timestamps.to_string());
    put("whisper.no_speech", special(&["<|nospeech|>", "<|nocaptions|>"])?.to_string());
    put("whisper.timestamp_begin", special(&["<|0.00|>"]).unwrap_or(no_timestamps + 1).to_string());
    put("whisper.blank", blank.to_string());
    // A model of many languages: its language tokens (between the start of a transcript and the task's).
    let multilingual = generation.get("is_multilingual").is_some_and(|v| matches!(v, Json::Bool(true)));
    if multilingual {
        let codes = generation.get("lang_to_id").and_then(Json::object).ok_or("generation_config.json has no lang_to_id")?;
        let langs: Vec<usize> = codes.iter().filter_map(|(_, v)| v.usize()).collect();
        let (first, last) = (langs.iter().min().ok_or("no languages")?, langs.iter().max().ok_or("no languages")?);
        if last - first + 1 != langs.len() {
            return Err("the language tokens aren't one run".into());
        }
        put("whisper.language_first", first.to_string());
        put("whisper.language_last", last.to_string());
        put("whisper.transcribe", special(&["<|transcribe|>"])?.to_string());
    }
    // The tokens never chosen, as faster-whisper's own list (the same, for these models): speaker tags, music and
    // the like, and the special tokens.
    let mut suppress = ids("suppress_tokens");
    for n in ["<|transcribe|>", "<|translate|>", "<|startoftranscript|>", "<|startofprev|>", "<|startoflm|>", "<|nospeech|>", "<|nocaptions|>"] {
        if let Some(id) = specials.get(n) {
            suppress.push(id.to_string());
        }
    }
    let mut suppress: Vec<usize> = suppress.iter().filter_map(|v| v.parse().ok()).collect();
    suppress.sort_unstable();
    suppress.dedup();
    put("whisper.suppress", suppress.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","));
    put("whisper.begin_suppress", ids("begin_suppress_tokens").join(","));
    Ok(s)
}

/// A speech recognition model's weight (transformers' names): where it goes in ours. Its matrices in 8 bits (or 16
/// as published, for checking), the first convolution (which reads the spectrogram itself) and the learned
/// positions in full.
pub fn plan_whisper(name: &str, dims: &[usize], bits: u32) -> Option<(String, Dtype, Vec<usize>)> {
    let m = if bits == 16 { Dtype::F16 } else { Dtype::Q8 };
    let flat = dims.to_vec();
    let n = name.strip_prefix("model.")?;
    let as_matrix = |d: &[usize]| -> Vec<usize> { alloc::vec![d[0], d[1..].iter().product()] };
    let (ours, dtype, shape) = match n {
        "encoder.conv1.weight" => ("whisper.conv1".to_string(), Dtype::F32, as_matrix(dims)),
        "encoder.conv1.bias" => ("whisper.conv1_bias".to_string(), Dtype::F32, flat),
        "encoder.conv2.weight" => ("whisper.conv2".to_string(), m, as_matrix(dims)),
        "encoder.conv2.bias" => ("whisper.conv2_bias".to_string(), Dtype::F32, flat),
        "encoder.embed_positions.weight" => ("whisper.encoder_pos".to_string(), Dtype::F32, flat),
        "encoder.layer_norm.weight" => ("whisper.encoder_norm".to_string(), Dtype::F32, flat),
        "encoder.layer_norm.bias" => ("whisper.encoder_norm_bias".to_string(), Dtype::F32, flat),
        "decoder.embed_tokens.weight" => ("whisper.embed".to_string(), m, flat),
        "decoder.embed_positions.weight" => ("whisper.decoder_pos".to_string(), Dtype::F32, flat),
        "decoder.layer_norm.weight" => ("whisper.decoder_norm".to_string(), Dtype::F32, flat),
        "decoder.layer_norm.bias" => ("whisper.decoder_norm_bias".to_string(), Dtype::F32, flat),
        _ => {
            let (side, rest) = n.split_once(".layers.")?;
            let (layer, part) = rest.split_once('.')?;
            let layer: usize = layer.parse().ok()?;
            let (ours, dtype) = match part {
                "self_attn_layer_norm.weight" => ("norm1", Dtype::F32),
                "self_attn_layer_norm.bias" => ("norm1_bias", Dtype::F32),
                "self_attn.q_proj.weight" => ("attention.q", m),
                "self_attn.q_proj.bias" => ("attention.q_bias", Dtype::F32),
                "self_attn.k_proj.weight" => ("attention.k", m),
                "self_attn.v_proj.weight" => ("attention.v", m),
                "self_attn.v_proj.bias" => ("attention.v_bias", Dtype::F32),
                "self_attn.out_proj.weight" => ("attention.o", m),
                "self_attn.out_proj.bias" => ("attention.o_bias", Dtype::F32),
                "encoder_attn_layer_norm.weight" => ("norm2", Dtype::F32),
                "encoder_attn_layer_norm.bias" => ("norm2_bias", Dtype::F32),
                "encoder_attn.q_proj.weight" => ("cross.q", m),
                "encoder_attn.q_proj.bias" => ("cross.q_bias", Dtype::F32),
                "encoder_attn.k_proj.weight" => ("cross.k", m),
                "encoder_attn.v_proj.weight" => ("cross.v", m),
                "encoder_attn.v_proj.bias" => ("cross.v_bias", Dtype::F32),
                "encoder_attn.out_proj.weight" => ("cross.o", m),
                "encoder_attn.out_proj.bias" => ("cross.o_bias", Dtype::F32),
                // (The encoder's second norm is before its feed-forward part; the decoder's third.)
                "final_layer_norm.weight" => (if side == "encoder" { "norm2" } else { "norm3" }, Dtype::F32),
                "final_layer_norm.bias" => (if side == "encoder" { "norm2_bias" } else { "norm3_bias" }, Dtype::F32),
                "fc1.weight" => ("fc1", m),
                "fc1.bias" => ("fc1_bias", Dtype::F32),
                "fc2.weight" => ("fc2", m),
                "fc2.bias" => ("fc2_bias", Dtype::F32),
                _ => return None,
            };
            if side != "encoder" && side != "decoder" {
                return None;
            }
            (format!("whisper.{side}.{layer}.{ours}"), dtype, flat)
        }
    };
    let dtype = if matches!(dtype, Dtype::Q8 | Dtype::Q4) && !shape.last().is_some_and(|c| c.is_multiple_of(quant::BLOCK)) {
        Dtype::F32
    } else {
        dtype
    };
    Some((ours, dtype, shape))
}

/// A weight's numbers (as published: BF16, F16 or F32, little-endian) in our type.
pub fn encode(source: &str, raw: &[u8], dtype: Dtype) -> Result<Vec<u8>, String> {
    let values: Vec<f32> = match source {
        "BF16" => raw.chunks_exact(2).map(|c| quant::bf16_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect(),
        "F16" => raw.chunks_exact(2).map(|c| quant::f16_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect(),
        "F32" => raw.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        other => return Err(format!("weights stored as {other}, which the converter doesn't read")),
    };
    let mut out = Vec::new();
    match dtype {
        Dtype::F32 => {
            for v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Dtype::Q8 => quant::quantize_q8(&values, &mut out),
        Dtype::Q4 => quant::quantize_q4(&values, &mut out),
        Dtype::BF16 => {
            for v in values {
                out.extend_from_slice(&((v.to_bits() >> 16) as u16).to_le_bytes());
            }
        }
        Dtype::F16 => {
            for v in values {
                out.extend_from_slice(&quant::f32_to_f16(v).to_le_bytes());
            }
        }
        Dtype::Blob => return Err("not a weight".into()),
    }
    Ok(out)
}

/// A tensor in a safetensors file: its name, type, shape and byte range (from the data's start, which is 8 + the
/// header's length into the file).
pub type Tensor = (String, String, Vec<usize>, usize, usize);

/// A safetensors file's header: its tensors.
pub fn safetensors(header: &str) -> Result<Vec<Tensor>, String> {
    let j = crate::json::parse(header).ok_or("the weights file's header isn't JSON")?;
    let mut out = Vec::new();
    for (name, v) in j.object().ok_or("the header isn't an object")? {
        if name == "__metadata__" {
            continue;
        }
        let dtype = v.get("dtype").and_then(Json::str).ok_or("a tensor without a dtype")?.to_string();
        let shape: Option<Vec<usize>> = v.get("shape").and_then(Json::array).ok_or("no shape")?.iter().map(Json::usize).collect();
        let offs = v.get("data_offsets").and_then(Json::array).ok_or("no offsets")?;
        let (a, b) = (offs.first().and_then(Json::usize).ok_or("offset")?, offs.get(1).and_then(Json::usize).ok_or("offset")?);
        out.push((name.clone(), dtype, shape.ok_or("a shape isn't numbers")?, a, b));
    }
    Ok(out)
}
