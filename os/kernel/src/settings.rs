//! Chat Computer's settings: the file the Windows setup (own-os.ps1) writes on the start-up partition,
//! \EFI\chatcomputer-own\settings.txt, which the start-up stage passes on. One setting a line, "name = value";
//! lines starting with # are notes.
//!
//!     claude-api-key = sk-ant-...       the key Chat Computer talks to Claude with (from console.anthropic.com)
//!     clock-offset-minutes = 480        how far the PC's clock is ahead of UTC (Windows keeps it in local time)
//!     memory-id = 1a2b3c4d              the memory file's id (its tags carry it: remember crate), in hex
//!
//! The key is never written to the log or shown on the screen.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Default)]
pub struct Settings {
    pub claude_api_key: Option<String>,
    /// Why the key that's there can't be used, if it can't.
    pub key_problem: Option<&'static str>,
    pub clock_offset_minutes: Option<i32>,
    pub memory_id: Option<u32>,
    /// How many lines weren't understood (their words aren't logged: one might be a key).
    pub not_understood: usize,
}

/// The file's text: UTF-8 (with or without its mark), or UTF-16 as Windows PowerShell 5 writes by default.
fn text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect();
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn parse(bytes: &[u8]) -> Settings {
    let mut s = Settings::default();
    for line in text(bytes).lines() {
        let line = line.trim().trim_start_matches('\u{FEFF}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            s.not_understood += 1;
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().trim_matches('"').trim();
        match name.as_str() {
            "claude-api-key" => {
                s.claude_api_key = None;
                s.key_problem = None;
                if value.is_empty() {
                    s.key_problem = Some("the Claude API key in the settings is empty");
                } else if value.len() > 512 || !value.bytes().all(|b| b.is_ascii_graphic()) {
                    // (A key is letters, digits, - and _: anything else, a space or a line break above all, would
                    // break the request it goes in.)
                    s.key_problem = Some("the Claude API key in the settings has characters a key doesn't have");
                } else {
                    s.claude_api_key = Some(String::from(value));
                }
            }
            "clock-offset-minutes" => {
                s.clock_offset_minutes = value.parse::<i32>().ok().filter(|m| (-1440..=1440).contains(m));
                if s.clock_offset_minutes.is_none() {
                    s.not_understood += 1;
                }
            }
            "memory-id" => {
                s.memory_id = u32::from_str_radix(value, 16).ok().filter(|&id| id != 0);
                if s.memory_id.is_none() {
                    s.not_understood += 1;
                }
            }
            _ => s.not_understood += 1,
        }
    }
    s
}
