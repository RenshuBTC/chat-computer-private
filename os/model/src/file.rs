//! The model's file: Chat Computer's own format, made from the model's published weights by its own converter
//! (the program's convert command), and read in place (the weights aren't copied: the file's bytes are the model).
//!
//!   "CCMODEL1", then the table's length (u32, little-endian), then the table: one line per entry,
//!   "name dtype dim,dim,... offset size" (offsets from the data's start, which is the next multiple of 64 after
//!   the table), and the data. Its settings are an entry too ("config": key=value lines), as is the tokenizer's.
//!
//! Every name, number, offset and size is checked against the file before it's used.

use crate::quant::Dtype;
use alloc::string::String;
use alloc::vec::Vec;

pub const MAGIC: &[u8; 8] = b"CCMODEL1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a model file.
    NotModel,
    /// Something in it doesn't add up.
    Damaged,
    /// An entry the model needs isn't there (its name).
    Missing,
    /// A setting the model needs isn't there or isn't understood.
    Config,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub dtype: Dtype,
    pub dims: Vec<usize>,
    /// Where its bytes are in the file.
    pub at: usize,
    pub size: usize,
}

impl Entry {
    pub fn elements(&self) -> usize {
        self.dims.iter().product()
    }
}

/// A model file's table, over its bytes.
pub struct File<'a> {
    pub bytes: &'a [u8],
    pub entries: Vec<Entry>,
}

impl<'a> File<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<File<'a>, Error> {
        if bytes.len() < 12 || &bytes[..8] != MAGIC {
            return Err(Error::NotModel);
        }
        let len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let table = bytes.get(12..12 + len).ok_or(Error::Damaged)?;
        let table = core::str::from_utf8(table).map_err(|_| Error::Damaged)?;
        let data = (12 + len).div_ceil(64) * 64;
        let mut entries = Vec::new();
        for line in table.lines().filter(|l| !l.is_empty()) {
            let mut parts = line.split(' ');
            let (Some(name), Some(dtype), Some(dims), Some(offset), Some(size), None) =
                (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(Error::Damaged);
            };
            let dtype = Dtype::from_name(dtype).ok_or(Error::Damaged)?;
            let dims: Vec<usize> = if dims.is_empty() || dims == "-" {
                Vec::new()
            } else {
                dims.split(',').map(|d| d.parse().map_err(|_| Error::Damaged)).collect::<Result<_, _>>()?
            };
            let offset: usize = offset.parse().map_err(|_| Error::Damaged)?;
            let size: usize = size.parse().map_err(|_| Error::Damaged)?;
            let at = data.checked_add(offset).ok_or(Error::Damaged)?;
            if at.checked_add(size).is_none_or(|end| end > bytes.len()) {
                return Err(Error::Damaged);
            }
            // The size must be what the type and shape say (blobs: any).
            let elements = dims.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).ok_or(Error::Damaged)?;
            if dtype != Dtype::Blob && dtype.bytes_for(elements) != Some(size) {
                return Err(Error::Damaged);
            }
            entries.push(Entry { name: String::from(name), dtype, dims, at, size });
        }
        Ok(File { bytes, entries })
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// The file's own mark: FNV-1a over its table (every entry's name, type, shape and place) and its length.
    /// What's kept for a model file beside it (a state read, a refusal direction) is used only with the same file.
    pub fn fingerprint(&self) -> u64 {
        let table = u32::from_le_bytes(self.bytes[8..12].try_into().unwrap()) as usize;
        fnv(0xcbf2_9ce4_8422_2325, self.bytes[..12 + table].iter().chain(&(self.bytes.len() as u64).to_le_bytes()))
    }

    pub fn data(&self, e: &Entry) -> &'a [u8] {
        &self.bytes[e.at..e.at + e.size]
    }

    /// The settings: the "config" entry's key=value lines.
    pub fn config(&self) -> Result<Config<'a>, Error> {
        let e = self.get("config").ok_or(Error::Missing)?;
        let text = core::str::from_utf8(self.data(e)).map_err(|_| Error::Damaged)?;
        Ok(Config { text })
    }
}

/// FNV-1a, 64 bits, from `h` on over the bytes.
pub fn fnv<'b>(mut h: u64, bytes: impl IntoIterator<Item = &'b u8>) -> u64 {
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The model's settings, read by name.
pub struct Config<'a> {
    pub text: &'a str,
}

impl Config<'_> {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.text.lines().find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim()))
    }

    pub fn usize(&self, key: &str) -> Result<usize, Error> {
        self.str(key).and_then(|v| v.parse().ok()).ok_or(Error::Config)
    }

    pub fn f64(&self, key: &str) -> Result<f64, Error> {
        self.str(key).and_then(parse_f64).ok_or(Error::Config)
    }
}

/// A decimal number ("1e-06", "10000000", "0.25"), without the standard library's parser (no_std has it, but this
/// keeps what's accepted plain).
pub fn parse_f64(s: &str) -> Option<f64> {
    s.parse::<f64>().ok()
}

/// Writing a model file (the converter's side): entries are added with their bytes, then it's all written out.
#[derive(Default)]
pub struct Writer {
    table: String,
    pub data: Vec<u8>,
}

impl Writer {
    pub fn add(&mut self, name: &str, dtype: Dtype, dims: &[usize], bytes: &[u8]) {
        use core::fmt::Write;
        // Each entry's data starts on a 64-byte boundary.
        while !self.data.len().is_multiple_of(64) {
            self.data.push(0);
        }
        let dims: Vec<String> = dims.iter().map(|d| alloc::format!("{d}")).collect();
        let dims = if dims.is_empty() { String::from("-") } else { dims.join(",") };
        let _ = writeln!(self.table, "{name} {} {dims} {} {}", dtype.name(), self.data.len(), bytes.len());
        self.data.extend_from_slice(bytes);
    }

    /// The file's bytes up to its data (the data follows).
    pub fn head(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.table.len() as u32).to_le_bytes());
        out.extend_from_slice(self.table.as_bytes());
        while !out.len().is_multiple_of(64) {
            out.push(0);
        }
        out
    }
}
