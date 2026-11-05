//! What is typed: key presses from any keyboard, turned into characters and editing keys, with Shift,
//! Caps Lock and key repeat. Keys are read by their place on the keyboard (the USB "usage" numbers);
//! the characters are the US layout's, which is also what keyboards sold in Singapore print.

use crate::sync::SpinLock;
use crate::time;
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Function(u8),
}

#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    /// The Windows key, or Command on a Mac keyboard.
    pub logo: bool,
}

/// How long a key is held before it repeats, and how often it repeats then.
const REPEAT_DELAY_MS: u64 = 500;
const REPEAT_EVERY_MS: u64 = 33;
const QUEUE_MAX: usize = 256;
const CAPS_LOCK: u8 = 0x39;

/// A key pressed this soon after the same key through another of a keyboard's interfaces is the same press: a gaming
/// keyboard may tell a key through its boot interface and its every-key-a-bit one alike.
const SAME_PRESS_MS: u64 = 40;

struct State {
    queue: VecDeque<Press>,
    /// The modifier keys held down (the report's first byte).
    modifiers: u8,
    /// The key that repeats while held, and when it repeats next.
    repeat: Option<(u8, u64)>,
    /// The last key pressed: its usage, when, and through what (a keyboard's interface's number, from `source`).
    last_press: (u8, u64, u32),
}

static STATE: SpinLock<State> =
    SpinLock::new(State { queue: VecDeque::new(), modifiers: 0, repeat: None, last_press: (0, 0, 0) });
static CAPS: AtomicBool = AtomicBool::new(false);

/// A keyboard's report in the boot format: modifier keys, a reserved byte, then up to six keys held
/// down. `last` is that keyboard's previous report.
pub fn report(last: &mut [u8; 8], now: &[u8; 8]) {
    if now[2..].contains(&1) {
        return; // too many keys at once for the keyboard to tell which
    }
    let keys = |r: &[u8; 8]| -> alloc::vec::Vec<u16> { r[2..].iter().filter(|&&k| k >= 4).map(|&k| k as u16).collect() };
    let mut held = keys(last);
    held_now(&mut held, now[0], &keys(now), 0);
    *last = *now;
}

/// What a keyboard's report says is held (as the HID descriptor reads it: in any format): the modifiers (in the boot
/// format's bits) and the other keys. `held` is what its last report held, and becomes this one's; `source` tells
/// keyboards' interfaces apart (0 for one that's alone), so that one press told by two of them counts once.
pub fn held_now(held: &mut alloc::vec::Vec<u16>, modifiers: u8, now: &[u16], source: u32) {
    let mut s = STATE.lock();
    s.modifiers = modifiers;
    let at = time::uptime_ms();
    for &k in now {
        if k >= 4 && k < 0x100 && !held.contains(&k) {
            let (usage, when, from) = s.last_press;
            if usage == k as u8 && from != source && at < when + SAME_PRESS_MS {
                continue;
            }
            s.last_press = (k as u8, at, source);
            s.down(k as u8);
        }
    }
    for &k in held.iter() {
        if k < 0x100 && !now.contains(&k) && s.repeat.map(|r| r.0) == Some(k as u8) {
            s.repeat = None;
        }
    }
    held.clear();
    held.extend_from_slice(now);
}

impl State {
    fn down(&mut self, usage: u8) {
        if usage == CAPS_LOCK {
            CAPS.fetch_xor(true, Ordering::Relaxed);
            self.repeat = None;
            return;
        }
        if let Some(p) = press(usage, self.modifiers) {
            self.push(p);
            self.repeat = Some((usage, time::uptime_ms() + REPEAT_DELAY_MS));
        }
    }

    fn push(&mut self, p: Press) {
        if self.queue.len() < QUEUE_MAX {
            self.queue.push_back(p);
        }
    }
}

/// From the main loop: a key held down repeats.
pub fn repeat() {
    let now = time::uptime_ms();
    let mut s = STATE.lock();
    if let Some((usage, next)) = s.repeat
        && now >= next
    {
        if let Some(p) = press(usage, s.modifiers) {
            s.push(p);
        }
        s.repeat = Some((usage, now + REPEAT_EVERY_MS));
    }
}

/// A keyboard went away: whatever it held down stops repeating.
pub fn stop_repeat() {
    STATE.lock().repeat = None;
}

/// The next key typed, if any.
pub fn take() -> Option<Press> {
    STATE.lock().queue.pop_front()
}

/// The keyboards' lights in the boot format: Num Lock always on (the number pad types numbers), Caps
/// Lock as it is.
pub fn lights() -> u8 {
    1 | if CAPS.load(Ordering::Relaxed) { 2 } else { 0 }
}

fn press(usage: u8, modifiers: u8) -> Option<Press> {
    let shift = modifiers & 0x22 != 0;
    let caps = CAPS.load(Ordering::Relaxed);
    let pick = |plain: &[u8], shifted: &[u8], i: usize| Key::Char(if shift { shifted[i] } else { plain[i] } as char);
    let key = match usage {
        0x04..=0x1D => {
            let c = (b'a' + usage - 0x04) as char;
            Key::Char(if shift != caps { c.to_ascii_uppercase() } else { c })
        }
        0x1E..=0x27 => pick(b"1234567890", b"!@#$%^&*()", (usage - 0x1E) as usize),
        0x28 | 0x58 => Key::Enter,
        0x29 => Key::Escape,
        0x2A => Key::Backspace,
        0x2B => Key::Tab,
        0x2C => Key::Char(' '),
        0x2D..=0x38 => pick(b"-=[]\\#;'`,./", b"_+{}|~:\"~<>?", (usage - 0x2D) as usize),
        0x3A..=0x45 => Key::Function(usage - 0x39),
        0x4A => Key::Home,
        0x4B => Key::PageUp,
        0x4C => Key::Delete,
        0x4D => Key::End,
        0x4E => Key::PageDown,
        0x4F => Key::Right,
        0x50 => Key::Left,
        0x51 => Key::Down,
        0x52 => Key::Up,
        0x54..=0x57 => Key::Char(b"/*-+"[(usage - 0x54) as usize] as char),
        0x59..=0x62 => Key::Char(b"1234567890"[(usage - 0x59) as usize] as char),
        0x63 => Key::Char('.'),
        0x64 => pick(b"\\", b"|", 0), // the extra key next to left Shift on European keyboards
        _ => return None,
    };
    Some(Press { key, ctrl: modifiers & 0x11 != 0, alt: modifiers & 0x44 != 0, logo: modifiers & 0x88 != 0 })
}
