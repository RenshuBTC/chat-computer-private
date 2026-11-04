//! Sound: the PC's sound controller (HD Audio: `hda.rs`), and what Chat Computer plays on it, the chimes of its timers
//! and reminders (made by the `chime` crate, the main Chat Computer's). Each chime is made the first time it rings and
//! kept for the next. While a USB sound card is plugged in (a headset, speakers: `usb/audio.rs`), the chimes play on
//! it instead, as a computer switches to one. And what it hears: its microphone (or line in), recorded as it comes, for Chat
//! Computer's own speech recognition (listening.rs).

pub mod hda;

use crate::pci;
use crate::sync::SpinLock;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use chime::Kind;
use hda::{Hda, Sound};

struct Speaker {
    hda: Hda,
    /// Each kind of chime, made at the volume, once it has rung.
    chimes: [Option<Sound>; 3],
}

/// How loud the chimes are, in percent (of their own loudness, which is the main Chat Computer's).
static VOLUME: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(100);

static SOUND: SpinLock<Option<Speaker>> = SpinLock::new(None);
/// What was found, for computer_status: the controller and codec, or why there's no sound.
static FOUND: SpinLock<Option<String>> = SpinLock::new(None);

/// How loud the chimes are made: the main Chat Computer's level.
const LOUD: f64 = 0.32;

fn index(kind: Kind) -> usize {
    match kind {
        Kind::Reminder => 0,
        Kind::Timer => 1,
        Kind::Alarm => 2,
    }
}

/// Finds the sound controllers (PCI class 4, subclass 3: HD Audio) and starts the first with outputs. Says what it
/// found, for the log.
pub fn init() -> String {
    let controllers: alloc::vec::Vec<pci::Function> = pci::of_class(0x04).into_iter().filter(|f| f.subclass == 0x03).collect();
    if controllers.is_empty() {
        *FOUND.lock() = Some(String::from("none: no HD Audio controller"));
        return String::from("sound: no HD Audio controller");
    }
    let mut why = String::new();
    for f in &controllers {
        match Hda::start(f) {
            Ok(hda) => {
                let said = format!("sound: {} at {} ({})", hda.describe(), f.at, hda.paths());
                *FOUND.lock() = Some(hda.describe());
                *SOUND.lock() = Some(Speaker { hda, chimes: [None, None, None] });
                return said;
            }
            Err(e) => {
                if !why.is_empty() {
                    why += "; ";
                }
                why += &format!("{} ({:04x}:{:04x}): {e}", f.at, f.vendor, f.device);
            }
        }
    }
    *FOUND.lock() = Some(format!("none that works with Chat Computer's own system yet ({why})"));
    format!("sound: none started: {why}")
}

/// Rings `kind`'s chime (if there's sound): from its start, whatever was playing stops. On a USB sound card while
/// one's plugged in, else on the PC's own sound.
pub fn ring(kind: Kind) -> bool {
    let volume = VOLUME.load(core::sync::atomic::Ordering::Relaxed);
    if let Some((hc, slot, _, rate)) = crate::usb::sound_card() {
        let mut samples: Vec<i16> =
            chime::chime(kind, rate, LOUD).iter().map(|&v| (v as i32 * volume as i32 / 100) as i16).collect();
        // (A quarter of a second's silence after it, as on the PC's own sound: its end isn't cut short.)
        samples.resize(samples.len() + rate as usize / 4, 0);
        if crate::usb::sound_card_play(hc, slot, &samples) {
            if option_env!("CHATCOMPUTER_TEST") == Some("sound") {
                crate::log!("Chat Computer kernel: test: chime at {} ms, on the USB sound card", crate::time::uptime_ms());
            }
            return true;
        }
    }
    let mut s = SOUND.lock();
    let Some(sp) = s.as_mut() else { return false };
    let i = index(kind);
    if sp.chimes[i].is_none() {
        let samples = chime::chime(kind, hda::RATE, LOUD);
        sp.chimes[i] = Sound::new(&samples, volume, !sp.hda.wide);
    }
    let Speaker { hda, chimes, .. } = sp;
    match &chimes[i] {
        Some(sound) => {
            hda.play(sound);
            // (A test build says when, for check.py --sound to check how often it rings.)
            if option_env!("CHATCOMPUTER_TEST") == Some("sound") {
                crate::log!("Chat Computer kernel: test: chime at {} ms", crate::time::uptime_ms());
            }
            true
        }
        None => false,
    }
}

/// Stops what's playing, at once (on a USB sound card, once what it was sent has played: a few tenths of a second).
pub fn stop() {
    if let Some(s) = SOUND.lock().as_mut() {
        s.hda.stop();
    }
    if let Some((hc, slot, _, _)) = crate::usb::sound_card() {
        crate::usb::sound_card_stop(hc, slot);
    }
}

/// Sets how loud the chimes are (0 to 100 percent); none if there's no sound.
pub fn set_volume(percent: u32) -> Option<u32> {
    let mut s = SOUND.lock();
    if s.is_none() && crate::usb::sound_card().is_none() {
        return None;
    }
    VOLUME.store(percent.min(100), core::sync::atomic::Ordering::Relaxed);
    if let Some(sp) = s.as_mut() {
        sp.chimes = [None, None, None];
    }
    volume()
}

/// The volume, if there's sound.
pub fn volume() -> Option<u32> {
    let there = SOUND.lock().is_some() || crate::usb::sound_card().is_some();
    there.then(|| VOLUME.load(core::sync::atomic::Ordering::Relaxed))
}

/// Stops a chime once it has played (after every wake); a test build's listening, once it has heard enough.
pub fn work() {
    if let Some(s) = SOUND.lock().as_mut() {
        s.hda.work();
    }
    if option_env!("CHATCOMPUTER_TEST") == Some("microphone") {
        test_heard();
    }
}

/// Starts listening to the microphone (a plugged-in one, else the built-in one, else the line in): what to, or why
/// it can't.
pub fn listen() -> Result<String, String> {
    let mut s = SOUND.lock();
    match s.as_mut() {
        Some(sp) => sp.hda.listen(),
        None => Err(String::from("there's no sound")),
    }
}

/// Whether there's a microphone (or line in) to listen to.
pub fn has_input() -> bool {
    SOUND.lock().as_ref().is_some_and(|sp| sp.hda.has_input())
}

/// What's been heard since last asked (48 kHz, 16 bits, one channel), appended to `out`.
pub fn heard(out: &mut Vec<i16>) {
    if let Some(sp) = SOUND.lock().as_mut() {
        sp.hda.heard(out);
    }
}

/// Stops listening.
pub fn stop_listening() {
    if let Some(sp) = SOUND.lock().as_mut() {
        sp.hda.stop_listening();
    }
}

/// A test build's listening (check.py --microphone): when it started, and what it has heard.
static TEST: SpinLock<Option<(u64, String, Vec<i16>)>> = SpinLock::new(None);
/// How long it listens.
const TEST_MS: u64 = 2500;

/// Starts a test build's listening, for check.py --microphone: what to.
pub fn test_listen() -> String {
    match listen() {
        Ok(what) => {
            *TEST.lock() = Some((crate::time::uptime_ms(), what.clone(), Vec::new()));
            format!("listening to {what}")
        }
        Err(e) => format!("can't listen: {e}"),
    }
}

/// A test build's listening, once it has heard enough: what it heard (after its first quarter second, the stream
/// starting), for check.py to check: how many samples, their root mean square and peak, and how often they cross
/// zero (a tone's frequency, twice over).
fn test_heard() {
    let mut t = TEST.lock();
    let Some((start, what, samples)) = t.as_mut() else { return };
    heard(samples);
    if crate::time::uptime_ms() < *start + TEST_MS {
        return;
    }
    stop_listening();
    let s = &samples[(hda::RATE as usize / 4).min(samples.len())..];
    let n = s.len().max(1) as u64;
    let squares: u64 = s.iter().map(|&v| (v as i64 * v as i64) as u64).sum();
    let peak = s.iter().map(|&v| (v as i32).unsigned_abs()).max().unwrap_or(0);
    let mut crossings = 0;
    let mut sign = 0i32;
    for &v in s {
        let now = (v as i32).signum();
        if now != 0 {
            if sign != 0 && now != sign {
                crossings += 1;
            }
            sign = now;
        }
    }
    crate::log!(
        "Chat Computer kernel: test: heard {} samples from {what}: root mean square {}, peak {peak}, {crossings} zero crossings",
        s.len(),
        (squares / n).isqrt()
    );
    *t = None;
}

/// What the computer plays sound on, for people (computer_status).
pub fn status() -> String {
    let mut found = FOUND.lock().clone().unwrap_or_else(|| String::from("not looked for"));
    if let Some((_, _, name, rate)) = crate::usb::sound_card() {
        found += &format!("; the chimes play on the USB sound card \"{name}\" ({} kHz) while it's plugged in", rate as f64 / 1000.0);
    }
    match volume() {
        Some(v) => format!("{found}; chimes at {v}% volume"),
        None => found,
    }
}
