//! What this computer is, as the kernel found it at start-up, and how it's doing now: for the model's
//! `computer_status` tool (assistant.rs).

use crate::sync::SpinLock;
use crate::{clock, memory, network, time, usb};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub struct Facts {
    pub cpu: String,
    pub screen: (usize, usize),
    pub diagonal_in: Option<u32>,
    pub secure_boot: Option<bool>,
    pub firmware: String,
    pub ps2_keyboard: Option<&'static str>,
}

static FACTS: SpinLock<Option<Facts>> = SpinLock::new(None);

pub fn set(f: Facts) {
    *FACTS.lock() = Some(f);
}

pub fn set_ps2(name: Option<&'static str>) {
    if let Some(f) = FACTS.lock().as_mut() {
        f.ps2_keyboard = name;
    }
}

fn size(bytes: u64) -> String {
    let mib = bytes >> 20;
    if mib < 2048 { format!("{mib} MB") } else { format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64) }
}

/// The computer's status, in a few lines.
pub fn status() -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(f) = FACTS.lock().as_ref() {
        lines.push(format!("Processor: {}", f.cpu));
        lines.push(crate::cores::status());
        let (w, h) = f.screen;
        lines.push(match f.diagonal_in {
            Some(d) => format!("Screen: {w} by {h} pixels, {d} inches"),
            None => format!("Screen: {w} by {h} pixels"),
        });
        if !f.firmware.is_empty() {
            lines.push(format!("Firmware: {}", f.firmware));
        }
        lines.push(String::from(match f.secure_boot {
            Some(true) => "Secure Boot: on",
            Some(false) => "Secure Boot: off",
            None => "Secure Boot: unknown",
        }));
    }
    let m = memory::stats();
    lines.push(format!("Memory: {} in all, {} free", size(m.total_ram), size(m.free)));
    // The battery, power, lid and temperature, as the firmware's programs say.
    lines.extend(crate::firmware::status());
    let up = time::uptime_ms() / 1000;
    lines.push(format!(
        "Chat Computer's own system has been running for {} h {} min {} s (since this PC started it)",
        up / 3600,
        up / 60 % 60,
        up % 60
    ));
    lines.push(match clock::now_text() {
        Some(t) => format!("Date and time, by this PC's clock: {t}"),
        None => String::from("Date and time: this PC's clock couldn't be read"),
    });
    let n = network::status();
    lines.push(match (&n.describe, n.address) {
        (None, _) => String::from("Network: no card Chat Computer has a driver for"),
        (Some(d), Some(a)) => format!("Network: {d}; address {a}"),
        (Some(d), None) if !n.link => format!("Network: {d}; no cable connected"),
        (Some(d), None) => format!("Network: {d}; no address yet"),
    });
    let u = usb::status();
    let mut keyboards = u.keyboards;
    if let Some(ps2) = FACTS.lock().as_ref().and_then(|f| f.ps2_keyboard) {
        keyboards.push(String::from(ps2));
    }
    lines.push(if keyboards.is_empty() { String::from("Keyboards: none found") } else { format!("Keyboards: {}", keyboards.join(", ")) });
    lines.push(if u.mice.is_empty() {
        String::from("Mice: none found (a USB mouse's wheel would scroll the conversation)")
    } else {
        format!("Mice: {} (the wheel scrolls the conversation; there is no pointer on the screen to move or click yet)", u.mice.join(", "))
    });
    lines.push(format!("Sound: {}", crate::sound::status()));
    lines.push(crate::schedule::status());
    lines.push(crate::files::status());
    lines.push(crate::keep::status());
    lines.push(crate::assistant::status());
    lines.push(crate::listening::status());
    lines.push(String::from(
        "Not yet in Chat Computer's own system: showing websites (it searches the web and reads pages' words), \
         changing files (it reads Windows' drives and USB drives, and writes only its own memory and log files), seeing \
         pictures (so a scanned page's words), Office's files from before 2007, sound other than its chimes (no speech, music or microphones), Wi-Fi, Bluetooth, sleep (it can turn \
         off), and the screen's brightness",
    ));
    lines.join("\n")
}
