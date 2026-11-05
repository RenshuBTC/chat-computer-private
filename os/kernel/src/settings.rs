//! Chat Computer's settings: the file the Windows setup (own-os.ps1) writes on the start-up partition,
//! \EFI\chatcomputer-own\settings.txt, which the start-up stage passes on. One setting a line, "name = value";
//! lines starting with # are notes.
//!
//!     model = C:\Users\Sam\AppData\Local\ai.chatcomputer.desktop\models\qwen3.5-4b.ccm
//!                                       the model's file (without it, the one Chat Computer's app got ready is used)
//!     clock-offset-minutes = 480        how far the PC's clock is ahead of UTC (Windows keeps it in local time)
//!     memory-id = 1a2b3c4d              the memory file's id (its tags carry it: remember crate), in hex
//!     log-id = 5e6f7a8b                 the log file's (journal.rs), the same way
//!     firmware-programs = off           don't run the firmware's own programs (ACPI's AML: firmware.rs), for a PC
//!                                       whose firmware does something odd with them
//!     cores = 1                         use at most this many of the processor's cores (cores.rs; all, without it),
//!                                       for a PC whose other cores don't start well
//!
//!     claude-api-key = sk-ant-...       your own key for Claude (Anthropic's API), which the setup brings from
//!                                       Windows when it's pasted there: the spark beside + can then choose Claude
//!                                       (never logged, nor shown)
//!     ai = claude                       who answers at start: Claude (with the key), rather than Chat Computer's
//!                                       own model (the default); "claude opus-5-5 max" names its model and effort

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Default)]
pub struct Settings {
    /// The model's file, as Windows names it.
    pub model: Option<String>,
    /// Your own key for Claude's API (never logged).
    pub claude_key: Option<String>,
    /// Who answers at start, if not Chat Computer's own model: "claude", and maybe its model and effort.
    pub ai: Option<String>,
    pub clock_offset_minutes: Option<i32>,
    pub memory_id: Option<u32>,
    pub log_id: Option<u32>,
    /// Run the firmware's own programs (on unless the settings say off).
    pub firmware_programs_off: bool,
    /// The most processor cores to use (all, if not said).
    pub cores: Option<usize>,
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
            "model" => {
                s.model = (!value.is_empty() && value.len() <= 1024).then(|| String::from(value));
                if s.model.is_none() {
                    s.not_understood += 1;
                }
            }
            "claude-api-key" | "anthropic-api-key" | "anthropic-key" => {
                // (A key is printable ASCII, without spaces; anything else isn't one.)
                let ok = !value.is_empty() && value.len() <= 512 && value.bytes().all(|b| b.is_ascii_graphic());
                s.claude_key = ok.then(|| String::from(value));
                if !ok {
                    s.not_understood += 1;
                }
            }
            "ai" => {
                let v = value.to_ascii_lowercase();
                s.ai = (v.starts_with("claude") && v.len() <= 100).then_some(v);
                if s.ai.is_none() && !matches!(value.to_ascii_lowercase().as_str(), "own" | "local" | "qwen" | "") {
                    s.not_understood += 1;
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
            "log-id" => {
                s.log_id = u32::from_str_radix(value, 16).ok().filter(|&id| id != 0);
                if s.log_id.is_none() {
                    s.not_understood += 1;
                }
            }
            "cores" => {
                s.cores = value.parse::<usize>().ok().filter(|&n| n >= 1);
                if s.cores.is_none() {
                    s.not_understood += 1;
                }
            }
            "firmware-programs" => match value.to_ascii_lowercase().as_str() {
                "off" | "no" | "false" => s.firmware_programs_off = true,
                "on" | "yes" | "true" => s.firmware_programs_off = false,
                _ => s.not_understood += 1,
            },
            _ => s.not_understood += 1,
        }
    }
    s
}
