//! From the model's published files (Qwen's config.json, tokenizer.json and safetensors weights) to its file in
//! Chat Computer's own format: the settings it needs, the tokenizer's tables, and each weight renamed, reshaped
//! and stored as 8 or 4 bits a weight (the small ones, and what the model is most sensitive to, kept more exactly).

use crate::json::Json;
use crate::quant::{self, Dtype};
use crate::token::byte_level_table;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The settings, as "key=value" lines, from config.json (its text model's part).
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
    Ok(s)
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
/// (the vision part, and the extra layer for predicting further ahead).
pub fn plan(name: &str, dims: &[usize], bits: u32) -> Option<(String, Dtype, Vec<usize>)> {
    let n = name.strip_prefix("model.language_model.").or_else(|| name.strip_prefix("model."))
        .unwrap_or(name);
    if name.contains("visual") || name.starts_with("mtp.") {
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
