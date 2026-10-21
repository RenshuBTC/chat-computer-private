//! Sound: the PC's sound controller (HD Audio: `hda.rs`), and what Chat Computer plays on it, the chimes of its timers
//! and reminders (made by the `chime` crate, the main Chat Computer's). Each chime is made the first time it rings and
//! kept for the next.

pub mod hda;

use crate::pci;
use crate::sync::SpinLock;
use alloc::format;
use alloc::string::String;
use chime::Kind;
use hda::{Hda, Sound};

struct Speaker {
    hda: Hda,
    /// How loud, in percent (of the chimes' own loudness, which is the main Chat Computer's).
    volume: u32,
    /// Each kind of chime, made at this volume, once it has rung.
    chimes: [Option<Sound>; 3],
}

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
                *SOUND.lock() = Some(Speaker { hda, volume: 100, chimes: [None, None, None] });
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

/// Rings `kind`'s chime (if there's sound): from its start, whatever was playing stops.
pub fn ring(kind: Kind) -> bool {
    let mut s = SOUND.lock();
    let Some(sp) = s.as_mut() else { return false };
    let i = index(kind);
    if sp.chimes[i].is_none() {
        let samples = chime::chime(kind, hda::RATE, LOUD);
        sp.chimes[i] = Sound::new(&samples, sp.volume, !sp.hda.wide);
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

/// Stops what's playing, at once.
pub fn stop() {
    if let Some(s) = SOUND.lock().as_mut() {
        s.hda.stop();
    }
}

/// Sets how loud the chimes are (0 to 100 percent); none if there's no sound.
pub fn set_volume(percent: u32) -> Option<u32> {
    let mut s = SOUND.lock();
    let sp = s.as_mut()?;
    sp.volume = percent.min(100);
    sp.chimes = [None, None, None];
    Some(sp.volume)
}

/// The volume, if there's sound.
pub fn volume() -> Option<u32> {
    SOUND.lock().as_ref().map(|s| s.volume)
}

/// Stops a chime once it has played (after every wake).
pub fn work() {
    if let Some(s) = SOUND.lock().as_mut() {
        s.hda.work();
    }
}

/// What the computer plays sound on, for people (computer_status).
pub fn status() -> String {
    let found = FOUND.lock().clone().unwrap_or_else(|| String::from("not looked for"));
    match volume() {
        Some(v) => format!("{found}; chimes at {v}% volume"),
        None => found,
    }
}
