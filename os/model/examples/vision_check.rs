//! The engine's vision part (vision.rs) against the reference's (tools/refcheck/vision_reference.py): from the
//! published files' vision weights (converted as the convert command converts them, here in memory, at 16 or 8 bits
//! a weight), the size a picture is read at, its resized pixels, its patches, and the rows the language model reads,
//! compared with what transformers made.
//!
//!     cargo run --release --features std --example vision_check -- MODEL_FOLDER REF_PREFIX [--bits 16|8] [--most-rows N]
use model::convert;
use model::file::{File, Writer};
use model::json::{self, Json};
use model::quant::Dtype;
use model::vision::Vision;
use std::io::{Read, Seek, SeekFrom};

fn floats(path: &str) -> Vec<f32> {
    std::fs::read(path).unwrap().chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn numbers(path: &str) -> Vec<usize> {
    std::fs::read_to_string(path).unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect()
}

/// How far `ours` is from `reference`: the largest difference, the mean, and the lowest cosine of a row's.
fn compare(ours: &[f32], reference: &[f32], width: usize) -> (f32, f32, f32) {
    assert_eq!(ours.len(), reference.len());
    let max = ours.iter().zip(reference).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    let mean = ours.iter().zip(reference).map(|(a, b)| (a - b).abs()).sum::<f32>() / ours.len() as f32;
    let mut lowest = 1.0f32;
    for (a, b) in ours.chunks_exact(width).zip(reference.chunks_exact(width)) {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
        let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
        lowest = lowest.min((dot / (na * nb)) as f32);
    }
    (max, mean, lowest)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = args.next().expect("the model's folder");
    let prefix = args.next().expect("the reference's files' prefix");
    let (mut bits, mut most_rows) = (16u32, usize::MAX);
    while let Some(a) = args.next() {
        let v = args.next().expect("a value");
        match a.as_str() {
            "--bits" => bits = v.parse().unwrap(),
            "--most-rows" => most_rows = v.parse().unwrap(),
            other => panic!("{other}?"),
        }
    }
    // The vision part, converted as the convert command does it.
    let config = json::parse(&std::fs::read_to_string(format!("{folder}/config.json")).unwrap()).unwrap();
    let pre = std::fs::read_to_string(format!("{folder}/preprocessor_config.json")).ok().and_then(|t| json::parse(&t));
    let settings = convert::vision_settings(&config, pre.as_ref());
    assert!(!settings.is_empty(), "the model has no vision part the engine runs");
    let mut w = Writer::default();
    w.add("config", Dtype::Blob, &[], settings.as_bytes());
    let path = format!("{folder}/model.safetensors-00001-of-00001.safetensors");
    let mut f = std::fs::File::open(&path).unwrap();
    let mut len = [0u8; 8];
    f.read_exact(&mut len).unwrap();
    let len = u64::from_le_bytes(len) as usize;
    let mut header = vec![0u8; len];
    f.read_exact(&mut header).unwrap();
    for (name, dtype, dims, a, b) in convert::safetensors(&String::from_utf8(header).unwrap()).unwrap() {
        if !name.contains("visual") {
            continue;
        }
        let Some((ours, mut to, shape)) = convert::plan(&name, &dims, bits) else { continue };
        // (CC_VISION_EXACT: which of the weights to keep in 16 bits whatever --bits says, to see which matter.)
        if let Ok(exact) = std::env::var("CC_VISION_EXACT")
            && to == Dtype::Q8
            && exact.split(',').any(|e| !e.is_empty() && ours.ends_with(e))
        {
            to = Dtype::BF16;
        }
        f.seek(SeekFrom::Start((8 + len + a) as u64)).unwrap();
        let mut raw = vec![0u8; b - a];
        f.read_exact(&mut raw).unwrap();
        // (CC_VISION_WEIGHTS_ONLY: the weights rounded to 8 bits, but kept as f32, so what's multiplied by them
        // isn't rounded: to tell the weights' rounding from the activations'.)
        if std::env::var("CC_VISION_WEIGHTS_ONLY").is_ok() && to == Dtype::Q8 {
            let q = convert::encode(&dtype, &raw, Dtype::Q8).unwrap();
            let cols = *shape.last().unwrap();
            let rows = q.len() / Dtype::Q8.bytes_for(cols).unwrap();
            let mut f32s = Vec::with_capacity(rows * cols * 4);
            let mut row = vec![0.0f32; cols];
            for r in 0..rows {
                model::quant::row_f32(Dtype::Q8, &q, cols, r, &mut row);
                for v in &row {
                    f32s.extend_from_slice(&v.to_le_bytes());
                }
            }
            w.add(&ours, Dtype::F32, &shape, &f32s);
            continue;
        }
        w.add(&ours, to, &shape, &convert::encode(&dtype, &raw, to).unwrap());
    }
    let mut bytes = w.head();
    bytes.extend_from_slice(&w.data);
    let file = File::open(&bytes).unwrap();
    let vision = Vision::load(&file).unwrap().expect("a vision part");
    let pool = model::threads::Threads::for_this_computer();

    let size = numbers(&format!("{prefix}.size"));
    let (width, height) = (size[0], size[1]);
    let rgb = std::fs::read(format!("{prefix}.rgb")).unwrap();
    let grid = numbers(&format!("{prefix}.grid"));
    let (h, w) = vision.size_for(height, width, most_rows);
    let p = vision.settings.patch;
    println!("read at {w}x{h} (the reference: {}x{})", grid[2] * p, grid[1] * p);
    assert_eq!((h / p, w / p), (grid[1], grid[2]), "the size it's read at");

    // The resized pixels.
    let reference = std::fs::read(format!("{prefix}.resized.rgb")).unwrap();
    let ours = vision.prepare(&rgb, width, height, most_rows);
    let off = ours.rgb.iter().zip(&reference).filter(|(a, b)| a != b).count();
    let worst = ours.rgb.iter().zip(&reference).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap_or(0);
    println!("resized: {off} of {} bytes differ (by at most {worst})", reference.len());

    // The patches, from the reference's own resized pixels (the same input), then the rows.
    let theirs = model::vision::Resized { width: w, height: h, rgb: reference };
    let patches = vision.patches(&theirs);
    let reference_patches = floats(&format!("{prefix}.patches.f32"));
    let (pmax, _, _) = compare(&patches, &reference_patches, 3 * 2 * p * p);
    println!("patches: within {pmax}");
    let started = std::time::Instant::now();
    let rows = vision.encode(&reference_patches, grid[1], grid[2], &pool);
    let took = started.elapsed();
    let reference_rows = floats(&format!("{prefix}.rows.f32"));
    let (max, mean, cosine) = compare(&rows, &reference_rows, vision.settings.out);
    if let Ok(dump) = std::env::var("CC_VISION_DUMP") {
        let bytes: Vec<u8> = rows.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(dump, bytes).unwrap();
    }
    let scale = reference_rows.iter().map(|v| v.abs()).fold(0.0f32, f32::max);
    println!(
        "rows ({} of {}, in {:.2} s): within {max} (the reference's largest is {scale}), {mean} on average; every row's cosine at least {cosine}",
        rows.len() / vision.settings.out,
        vision.settings.out,
        took.as_secs_f64()
    );
    // And from its own resizing.
    let own = vision.encode(&vision.patches(&ours), grid[1], grid[2], &pool);
    let (max2, mean2, cosine2) = compare(&own, &reference_rows, vision.settings.out);
    println!("rows from its own resizing: within {max2}, {mean2} on average; cosine at least {cosine2}");
    let _ = Json::Null;
}
