//! Chat Computer's own model engine: runs an open model on the PC's own processor, with nothing else, so Chat
//! Computer works for free, for good, and nothing it says goes anywhere. The models are Qwen's (Alibaba's,
//! published under the Apache 2.0 licence): 3.5, 3.6 and 3.8, all one design, from 0.8 to 35 billion weights.
//!
//! - file: the model's file (Chat Computer's own format, made by the convert command from the published weights);
//! - quant and cpu: how weights are stored (8 and 4 bits a weight) and multiplied, with the processor's fastest way;
//! - net: the network itself, a token or many at a time;
//! - token, unicode and nfc: words into tokens and back;
//! - chat: a conversation as the model reads it, and what it says back (words, thinking, tool calls);
//! - sample: the next token; session: a conversation kept going, reading only what's new;
//! - refusal: the direction the model refuses along, found, measured and taken out as it runs, so it answers what
//!   it's asked;
//! - whisper: speech recognition (OpenAI's Whisper), the words in a recording.
//!
//! It needs no standard library (Chat Computer's own kernel runs it), only memory; where there is one, a
//! processor pool runs its arithmetic on every core (threads), and the program in src/bin serves it. See README.md.

#![no_std]

extern crate alloc;

pub mod chat;
pub mod convert;
pub mod cpu;
pub mod file;
pub mod json;
pub mod math;
pub mod net;
pub mod nfc;
mod nfc_table;
pub mod quant;
pub mod refusal;
pub mod sample;
pub mod session;
#[doc(hidden)]
pub mod testing;
#[cfg(feature = "std")]
pub mod threads;
pub mod token;
mod unicode;
pub mod vector;
pub mod vision;
pub mod whisper;

#[cfg(test)]
mod tests;

pub use file::{Error, File};
pub use net::{Model, Pool, Serial, State};

use alloc::vec::Vec;

/// The tokenizer, from its entries in the file: every token's bytes ("tokenizer.tokens": count, offsets, bytes),
/// the merges in order ("tokenizer.merges": left, right and merged ids), and the special tokens
/// ("tokenizer.specials": "id text" lines).
pub fn load_tokenizer(file: &File) -> Result<token::Tokenizer, Error> {
    let get = |n: &str| file.get(n).map(|e| file.data(e)).ok_or(Error::Missing);
    let t = get("tokenizer.tokens")?;
    let u32_at = |b: &[u8], i: usize| -> Result<u32, Error> {
        Ok(u32::from_le_bytes(b.get(i..i + 4).ok_or(Error::Damaged)?.try_into().unwrap()))
    };
    let count = u32_at(t, 0)? as usize;
    let blob = 4 + 4 * (count + 1);
    let mut tokens = Vec::with_capacity(count);
    for i in 0..count {
        let (a, b) = (u32_at(t, 4 + 4 * i)? as usize, u32_at(t, 8 + 4 * i)? as usize);
        tokens.push(t.get(blob + a..blob + b).ok_or(Error::Damaged)?.to_vec());
    }
    let m = get("tokenizer.merges")?;
    let mut merges = Vec::with_capacity(m.len() / 12);
    for c in m.chunks_exact(12) {
        let (l, r, x) = (u32_at(c, 0)?, u32_at(c, 4)?, u32_at(c, 8)?);
        if l as usize >= count || r as usize >= count || x as usize >= count {
            return Err(Error::Damaged);
        }
        merges.push((l, r, x));
    }
    let s = core::str::from_utf8(get("tokenizer.specials")?).map_err(|_| Error::Damaged)?;
    let mut specials = Vec::new();
    for line in s.lines().filter(|l| !l.is_empty()) {
        let (id, text) = line.split_once(' ').ok_or(Error::Damaged)?;
        let id: u32 = id.parse().map_err(|_| Error::Damaged)?;
        if id as usize >= count {
            return Err(Error::Damaged);
        }
        specials.push((text.as_bytes().to_vec(), id));
    }
    Ok(token::Tokenizer::new(tokens, &merges, specials))
}
