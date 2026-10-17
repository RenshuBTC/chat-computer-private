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

struct State {
    queue: VecDeque<Press>,
    /// The modifier keys held down (the report's first byte).
    modifiers: u8,
    /// The key that repeats while held, and when it repeats next.
    repeat: Option<(u8, u64)>,
}

static STATE: SpinLock<State> = SpinLock::new(State { queue: VecDeque::new(), modifiers: 0, repeat: None });
static CAPS: AtomicBool = AtomicBool::new(false);

/// A keyboard's report in the boot format: modifier keys, a reserved byte, then up to six keys held
/// down. `last` is that keyboard's previous report.
pub fn report(last: &mut [u8; 8], now: &[u8; 8]) {
    if now[2..].contains(&1) {
        return; // too many keys at once for the keyboard to tell which
    }
    let mut s = STATE.lock();
    s.modifiers = now[0];
    for &k in &now[2..] {
        if k >= 4 && !last[2..].contains(&k) {
            s.down(k);
        }
    }
    for &k in &last[2..] {
        if k >= 4 && !now[2..].contains(&k) && s.repeat.map(|r| r.0) == Some(k) {
            s.repeat = None;
        }
    }
    *last = *now;
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
