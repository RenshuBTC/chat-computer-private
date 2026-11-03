//! The model's file made in a web page from Qwen's published files as they download: the same file, byte for byte, as
//! the engine's own program makes (`chat-computer-model convert`, src/bin/chat-computer-model/convert.rs in os/model),
//! with the same library functions, but fed a piece at a time, in the order the bytes arrive, so a page never holds
//! the 1.7 GB download (only the model's file, which is what it runs).
//!
//! First the small files (config.json, the chat template, tokenizer.json) and each weights file's header; then the
//! plan: the file's table, every entry's place in it, the settings and the tokenizer written in, all as the program
//! does; then each weights file's bytes, in order, in pieces of any size: each weight is stored as 8 or 4 bits a
//! number (or as it is) as soon as enough of it has come (a block of 32 numbers for the quantized ones), into its
//! place in the file. A weight's numbers are the same in pieces as whole: each block is quantized on its own.

use model::convert;
use model::json::{self, Json};
use model::quant::{self, Dtype};

/// A weights file: its name (they're taken in order of name, as the program takes them from a folder), and its header
/// (the JSON naming its weights, and where each one's bytes are) with its length.
struct Weights {
    name: String,
    header_len: usize,
    header: String,
}

/// A weight's way from the published file to ours.
struct Weight {
    /// Our name, type and shape, and its place and size in our file.
    ours: String,
    to: Dtype,
    dims: Vec<usize>,
    at: usize,
    size: usize,
    /// Which weights file it's in (its number as added), its type there, and where its bytes are in that file.
    file: usize,
    source: String,
    start: u64,
    len: u64,
    /// How many of its bytes have come, and how many of ours are written.
    taken: u64,
    written: usize,
    /// The start of a block (or a number) that hasn't all come yet.
    carry: Vec<u8>,
}

impl Weight {
    /// The published bytes of one block (or one number): what's stored at once.
    fn unit(&self) -> Result<usize, String> {
        let bytes = match self.source.as_str() {
            "BF16" | "F16" => 2,
            "F32" => 4,
            other => return Err(format!("weights stored as {other}, which the converter doesn't read")),
        };
        Ok(if matches!(self.to, Dtype::Q8 | Dtype::Q4) { quant::BLOCK * bytes } else { bytes })
    }
}

#[derive(Default)]
pub struct Converter {
    bits: u32,
    config: Option<String>,
    template: Option<String>,
    template_config: Option<String>,
    tokenizer: Option<(Vec<u8>, Vec<u8>, String)>,
    files: Vec<Weights>,
    weights: Vec<Weight>,
    /// Per weights file: its weights in the order their bytes come.
    by_file: Vec<Vec<usize>>,
    /// Per weights file: how far its bytes have come.
    fed: Vec<u64>,
    /// The model's file, once planned.
    pub out: Vec<u8>,
}

/// At most this many published bytes are stored at once (a big weight is stored in parts of this, each a whole
/// number of blocks), so storing one needs little memory besides the file itself.
const PART: usize = 1 << 20;

impl Converter {
    pub fn new(bits: u32) -> Converter {
        Converter { bits, ..Converter::default() }
    }

    /// One of the small files, by its name in Qwen's repository: config.json, chat_template.jinja,
    /// tokenizer_config.json (where the chat template is, when there's no chat_template.jinja) or tokenizer.json. Any
    /// other (generation_config.json, say) isn't the converter's, as the program reads only these from its folder.
    pub fn file(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if !["config.json", "chat_template.jinja", "tokenizer_config.json", "tokenizer.json"].contains(&name) {
            return Ok(());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{name} isn't UTF-8"))?;
        match name {
            "config.json" => {
                let config = json::parse(text).ok_or("config.json isn't JSON")?;
                self.config = Some(convert::settings(&config)?);
            }
            "chat_template.jinja" => self.template = Some(text.to_string()),
            "tokenizer_config.json" => self.template_config = Some(text.to_string()),
            _ => {
                let tj = json::parse(text).ok_or("tokenizer.json isn't JSON")?;
                self.tokenizer = Some(convert::tokenizer(&tj)?);
            }
        }
        Ok(())
    }

    /// A weights file's header (the JSON after its first 8 bytes); its number, which its bytes are fed with.
    pub fn weights(&mut self, name: &str, header: &[u8]) -> Result<usize, String> {
        let header = std::str::from_utf8(header).map_err(|_| "a header isn't UTF-8")?.to_string();
        self.files.push(Weights { name: name.to_string(), header_len: header.len(), header });
        Ok(self.files.len() - 1)
    }

    /// Plans the file as the program does: the settings (with the chat's style) and the tokenizer first, then every
    /// weight in order of its name, each on a 64-byte boundary. The file is made, the settings and tokenizer in it,
    /// and its size given; the weights follow as their bytes come.
    pub fn plan(&mut self) -> Result<usize, String> {
        let mut settings = self.config.take().ok_or("config.json hasn't come")?;
        // How the model's conversations are written: its chat template, said as one of the styles chat.rs knows.
        let template = match self.template.take() {
            Some(t) => t,
            None => {
                let tc = self.template_config.take().ok_or("no chat template")?;
                let tc = json::parse(&tc).ok_or("tokenizer_config.json isn't JSON")?;
                tc.get("chat_template").and_then(Json::str).ok_or("no chat template")?.to_string()
            }
        };
        let style = model::chat::Style::of_template(&template)
            .ok_or("its chat template isn't one the engine knows how to write (Qwen 3.5, 3.6 or 3.8's)")?;
        settings.push_str(&format!("chat.style={style}\n"));
        let (tokens, merges, specials) = self.tokenizer.take().ok_or("tokenizer.json hasn't come")?;
        let blobs = [
            ("config", settings.into_bytes()),
            ("tokenizer.tokens", tokens),
            ("tokenizer.merges", merges),
            ("tokenizer.specials", specials.into_bytes()),
        ];
        let mut order: Vec<usize> = (0..self.files.len()).collect();
        order.sort_by(|&a, &b| self.files[a].name.cmp(&self.files[b].name));
        let mut weights = Vec::new();
        for f in order {
            let w = &self.files[f];
            for (name, dtype, dims, a, b) in convert::safetensors(&w.header)? {
                let Some((ours, to, shape)) = convert::plan(&name, &dims, self.bits) else { continue };
                let elements: usize = shape.iter().product();
                let size = to.bytes_for(elements).ok_or_else(|| format!("{name}: not whole blocks"))?;
                if b < a {
                    return Err(format!("{name}: its bytes end before they start"));
                }
                weights.push(Weight {
                    ours, to, dims: shape, at: 0, size, file: f, source: dtype, start: (8 + w.header_len + a) as u64,
                    len: (b - a) as u64, taken: 0, written: 0, carry: Vec::new(),
                });
            }
        }
        weights.sort_by(|x, y| x.ours.cmp(&y.ours));
        // The table: each entry's place, every one on a 64-byte boundary.
        let mut table = String::new();
        let mut at = 0usize;
        let mut place = |name: &str, dtype: Dtype, dims: &[usize], size: usize| {
            at = at.div_ceil(64) * 64;
            let dims = if dims.is_empty() { "-".to_string() } else { dims.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",") };
            table.push_str(&format!("{name} {} {dims} {at} {size}\n", dtype.name()));
            let here = at;
            at += size;
            here
        };
        let blob_at: Vec<usize> = blobs.iter().map(|(n, b)| place(n, Dtype::Blob, &[b.len()], b.len())).collect();
        for w in weights.iter_mut() {
            w.at = place(&w.ours, w.to, &w.dims, w.size);
        }
        let mut head = Vec::new();
        head.extend_from_slice(model::file::MAGIC);
        head.extend_from_slice(&(table.len() as u32).to_le_bytes());
        head.extend_from_slice(table.as_bytes());
        while head.len() % 64 != 0 {
            head.push(0);
        }
        let total = head.len().checked_add(at).ok_or("the model is too big")?;
        let mut out = Vec::new();
        out.try_reserve_exact(total).map_err(|_| format!("no room for the model's {total} bytes"))?;
        out.resize(total, 0);
        out[..head.len()].copy_from_slice(&head);
        for (w, (_, b)) in blob_at.iter().zip(&blobs) {
            out[head.len() + w..head.len() + w + b.len()].copy_from_slice(b);
        }
        for w in weights.iter_mut() {
            w.at += head.len();
        }
        self.by_file = vec![Vec::new(); self.files.len()];
        for (i, w) in weights.iter().enumerate() {
            self.by_file[w.file].push(i);
        }
        for list in self.by_file.iter_mut() {
            list.sort_by_key(|&i| weights[i].start);
        }
        self.fed = vec![0; self.files.len()];
        self.weights = weights;
        self.out = out;
        Ok(total)
    }

    /// The next of a weights file's bytes, from `offset` in it (where the last piece ended: a piece given again, or
    /// part of one, is taken once). Any weights in them are stored as they come.
    pub fn feed(&mut self, file: usize, offset: u64, bytes: &[u8]) -> Result<(), String> {
        let fed = *self.fed.get(file).ok_or("no such weights file")?;
        if offset > fed {
            return Err(format!("{}: a part before byte {offset} hasn't come", self.files[file].name));
        }
        let end = offset + bytes.len() as u64;
        if end <= fed {
            return Ok(());
        }
        let (from, bytes) = (fed, &bytes[(fed - offset) as usize..]);
        for k in 0..self.by_file[file].len() {
            let i = self.by_file[file][k];
            let w = &self.weights[i];
            let (a, b) = (w.start.max(from), (w.start + w.len).min(end));
            if a >= b {
                if w.start >= end {
                    break;
                }
                continue;
            }
            let piece = &bytes[(a - from) as usize..(b - from) as usize];
            self.take(i, piece)?;
        }
        self.fed[file] = end;
        Ok(())
    }

    /// The next bytes of weight i: stored a whole number of blocks at a time, the rest kept for the next piece.
    fn take(&mut self, i: usize, mut piece: &[u8]) -> Result<(), String> {
        let unit = self.weights[i].unit()?;
        self.weights[i].taken += piece.len() as u64;
        if !self.weights[i].carry.is_empty() {
            let w = &mut self.weights[i];
            let n = (unit - w.carry.len()).min(piece.len());
            w.carry.extend_from_slice(&piece[..n]);
            piece = &piece[n..];
            if w.carry.len() < unit {
                return Ok(());
            }
            let carry = core::mem::take(&mut w.carry);
            self.store(i, &carry)?;
        }
        let whole = piece.len() / unit * unit;
        for part in piece[..whole].chunks(PART / unit * unit) {
            self.store(i, part)?;
        }
        self.weights[i].carry.extend_from_slice(&piece[whole..]);
        Ok(())
    }

    /// Whole blocks (or numbers) of weight i, stored in our type, after what's written of it.
    fn store(&mut self, i: usize, raw: &[u8]) -> Result<(), String> {
        let w = &mut self.weights[i];
        let bytes = convert::encode(&w.source, raw, w.to)?;
        if w.written + bytes.len() > w.size {
            return Err(format!("{}: more bytes than its shape has room for", w.ours));
        }
        self.out[w.at + w.written..w.at + w.written + bytes.len()].copy_from_slice(&bytes);
        w.written += bytes.len();
        Ok(())
    }

    /// Every weight came whole: the file is done.
    pub fn finish(&self) -> Result<(), String> {
        for w in &self.weights {
            if w.taken != w.len || !w.carry.is_empty() || w.written != w.size {
                return Err(format!("{}: {} bytes, not {}", w.ours, w.written, w.size));
            }
        }
        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    /// Numbers that look random, the same each time (xorshift).
    fn next(r: &mut u64) -> u64 {
        *r ^= *r << 13;
        *r ^= *r >> 7;
        *r ^= *r << 17;
        *r
    }

    #[test]
    fn makes_the_same_file_as_the_engine_s_own_program_in_pieces_of_any_size() {
        let dir = std::env::temp_dir().join(format!("webengine-convert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = fixture::files();
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        let mut r = 0x1234_5678_9ABC_DEF1u64;
        for bits in [8, 4, 16] {
            let out = dir.join(format!("program-{bits}.ccm"));
            crate::program_convert::run(&dir, &out, bits).unwrap();
            let want = std::fs::read(&out).unwrap();
            let mut c = Converter::new(bits);
            // (The small files in any order; the weights files' headers too.)
            for (name, bytes) in files.iter().rev() {
                if !name.ends_with(".safetensors") {
                    c.file(name, bytes).unwrap();
                }
            }
            let mut ids = Vec::new();
            for (name, bytes) in files.iter().rev().filter(|(n, _)| n.ends_with(".safetensors")) {
                let len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
                ids.push((c.weights(name, &bytes[8..8 + len]).unwrap(), bytes));
            }
            assert_eq!(c.plan().unwrap(), want.len());
            // Each file's bytes in pieces of 1 to 300 bytes, some of them given again (a piece asked for again after a
            // break), the files' pieces interleaved.
            let mut at = vec![0usize; ids.len()];
            while at.iter().zip(&ids).any(|(a, (_, b))| *a < b.len()) {
                let f = next(&mut r) as usize % ids.len();
                let (id, bytes) = ids[f];
                if at[f] >= bytes.len() {
                    continue;
                }
                let back = if next(&mut r) % 5 == 0 { (next(&mut r) as usize % 40).min(at[f]) } else { 0 };
                let from = at[f] - back;
                let to = (from + 1 + next(&mut r) as usize % 300).min(bytes.len());
                c.feed(id, from as u64, &bytes[from..to]).unwrap();
                at[f] = at[f].max(to);
            }
            c.finish().unwrap();
            assert!(c.out == want, "{bits} bits: the page's file isn't the program's");
            // And it's a model the engine runs.
            let file = model::File::open(&c.out).unwrap();
            let m = model::Model::load(&file).unwrap();
            assert_eq!((m.settings.vocab, m.settings.layers), (fixture::VOCAB, 4));
        }
        // A part missing, or a weights file cut short, is said.
        let mut c = Converter::new(8);
        for (name, bytes) in &files {
            if name.ends_with(".safetensors") {
                let len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
                c.weights(name, &bytes[8..8 + len]).unwrap();
            } else {
                c.file(name, bytes).unwrap();
            }
        }
        c.plan().unwrap();
        let one = &files.iter().find(|(n, _)| n.ends_with("00001-of-00002.safetensors")).unwrap().1;
        assert!(c.feed(0, 100, &one[100..200]).is_err());
        c.feed(0, 0, &one[..one.len() - 64]).unwrap();
        assert!(c.finish().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
