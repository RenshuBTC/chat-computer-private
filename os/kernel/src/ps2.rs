//! The PS/2 keyboard controller (the "8042"): how most laptops' built-in keyboards reach the processor,
//! and the round keyboard socket of older PCs. Keys arrive one byte at a time as scan codes (set 1,
//! which the controller translates the keyboard's own codes into), and are turned into the same key
//! numbers USB keyboards use, so `keyboard` treats both alike.
//!
//! On a PC whose firmware made a USB keyboard look like a PS/2 one, the controller here was the
//! firmware's imitation; it stopped when Chat Computer took the USB controllers over, so this runs after.

use crate::cpu::{inb, outb};
use crate::keyboard;
use crate::sync::SpinLock;
use crate::time;

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const COMMAND: u16 = 0x64;
const OUTPUT_FULL: u8 = 1 << 0;
const INPUT_FULL: u8 = 1 << 1;
const FROM_MOUSE: u8 = 1 << 5;

const ACK: u8 = 0xFA;

struct Ps2 {
    present: bool,
    /// The last byte was 0xE0: the next is one of the extra keys.
    extended: bool,
    /// The key report built from the bytes so far, in USB's boot format, and the previous one.
    now: [u8; 8],
    last: [u8; 8],
    /// The lights last set (0xFF: not yet).
    lights: u8,
    /// Bytes still to skip of the Pause key's sequence.
    skip: u8,
}

static PS2: SpinLock<Ps2> =
    SpinLock::new(Ps2 { present: false, extended: false, now: [0; 8], last: [0; 8], lights: 0xFF, skip: 0 });

fn wait(ms: u64, done: impl Fn(u8) -> bool) -> bool {
    let end = time::now_us() + ms * 1000;
    loop {
        if done(unsafe { inb(STATUS) }) {
            return true;
        }
        if time::now_us() > end {
            return false;
        }
        time::delay_us(20);
    }
}

fn send_command(c: u8) -> bool {
    wait(20, |s| s & INPUT_FULL == 0) && {
        unsafe { outb(COMMAND, c) };
        true
    }
}

fn send_data(b: u8) -> bool {
    wait(20, |s| s & INPUT_FULL == 0) && {
        unsafe { outb(DATA, b) };
        true
    }
}

fn read_data(ms: u64) -> Option<u8> {
    wait(ms, |s| s & OUTPUT_FULL != 0).then(|| unsafe { inb(DATA) })
}

/// Sends a byte to the keyboard itself and waits for its acknowledgement.
fn to_keyboard(b: u8) -> bool {
    send_data(b) && (0..4).any(|_| read_data(50) == Some(ACK))
}

/// Finds the controller and a keyboard on it. Returns whether there is one.
pub fn init() -> bool {
    if unsafe { inb(STATUS) } == 0xFF {
        return false; // nothing answers at the controller's ports
    }
    // Both sockets off while setting it up; whatever is waiting is thrown away.
    if !send_command(0xAD) || !send_command(0xA7) {
        return false;
    }
    for _ in 0..32 {
        if read_data(2).is_none() {
            break;
        }
    }
    if !send_command(0x20) {
        return false;
    }
    let Some(config) = read_data(20) else { return false };
    // Keyboard clock on, translation to set 1 on, no interrupts (Chat Computer looks for bytes instead),
    // the mouse socket left off.
    let config = (config | 1 << 6 | 1 << 5) & !(1 << 0 | 1 << 1 | 1 << 4);
    if !send_command(0x60) || !send_data(config) || !send_command(0xAE) {
        return false;
    }
    // A keyboard acknowledges "start sending keys"; with nothing plugged in, nothing does.
    let present = to_keyboard(0xF4);
    PS2.lock().present = present;
    present
}

/// Whether a PS/2 keyboard was found.
pub fn present() -> bool {
    PS2.lock().present
}

/// From the main loop: reads the bytes that arrived, and sets the lights when Caps Lock changed.
pub fn poll() {
    let mut p = PS2.lock();
    if !p.present {
        return;
    }
    for _ in 0..64 {
        let status = unsafe { inb(STATUS) };
        if status & OUTPUT_FULL == 0 {
            break;
        }
        let byte = unsafe { inb(DATA) };
        if status & FROM_MOUSE == 0 {
            p.byte(byte);
        }
    }
    let lights = keyboard::lights();
    if lights != p.lights {
        p.lights = lights;
        // PS/2 orders the lights Scroll, Num, Caps; USB's boot format Num, Caps, Scroll.
        let ps2 = (lights & 1) << 1 | (lights & 2) << 1;
        let _ = to_keyboard(0xED) && to_keyboard(ps2);
    }
}

impl Ps2 {
    fn byte(&mut self, b: u8) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        match b {
            0xE0 => {
                self.extended = true;
                return;
            }
            0xE1 => {
                self.skip = 5; // Pause: E1 1D 45 E1 9D C5, no release
                return;
            }
            // Answers and errors, not keys. (0xAA, the keyboard's "self-test passed", is also left
            // Shift's release, so it goes through: a harmless release if it was the former.)
            ACK | 0xFE | 0x00 | 0xFF | 0xEE => return,
            _ => {}
        }
        let extended = core::mem::take(&mut self.extended);
        let released = b & 0x80 != 0;
        let code = b & 0x7F;
        if extended && code == 0x2A || extended && code == 0x36 {
            return; // the "fake Shift" some keys send around themselves
        }
        let Some(usage) = usage(code, extended) else { return };
        if let Some(bit) = modifier(usage) {
            if released {
                self.now[0] &= !bit;
            } else {
                self.now[0] |= bit;
            }
        } else if released {
            for k in self.now[2..].iter_mut() {
                if *k == usage {
                    *k = 0;
                }
            }
        } else if !self.now[2..].contains(&usage)
            && let Some(free) = self.now[2..].iter_mut().find(|k| **k == 0)
        {
            *free = usage;
        }
        let now = self.now;
        keyboard::report(&mut self.last, &now);
    }
}

/// Modifier keys, which this module keeps as USB numbers 0xE0-0xE7 until it builds a report: their bit
/// in the report's first byte.
fn modifier(usage: u8) -> Option<u8> {
    (0xE0..=0xE7).contains(&usage).then(|| 1 << (usage - 0xE0))
}

/// A set 1 scan code's USB key number.
fn usage(code: u8, extended: bool) -> Option<u8> {
    let u = if extended {
        match code {
            0x1C => 0x58, // keypad Enter
            0x1D => 0xE4, // right Ctrl
            0x35 => 0x54, // keypad /
            0x38 => 0xE6, // right Alt
            0x47 => 0x4A, // Home
            0x48 => 0x52, // Up
            0x49 => 0x4B, // Page Up
            0x4B => 0x50, // Left
            0x4D => 0x4F, // Right
            0x4F => 0x4D, // End
            0x50 => 0x51, // Down
            0x51 => 0x4E, // Page Down
            0x52 => 0x49, // Insert
            0x53 => 0x4C, // Delete
            0x5B => 0xE3, // left Windows key
            0x5C => 0xE7, // right Windows key
            0x5D => 0x65, // menu key
            _ => return None,
        }
    } else {
        const PLAIN: [u8; 0x59] = [
            0, 0x29, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2D, 0x2E, 0x2A, 0x2B, // 00-0F
            0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13, 0x2F, 0x30, 0x28, 0xE0, 0x04, 0x16, // 10-1F
            0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F, 0x33, 0x34, 0x35, 0xE1, 0x31, 0x1D, 0x1B, 0x06, 0x19, // 20-2F
            0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xE5, 0x55, 0xE2, 0x2C, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, // 30-3F
            0x3F, 0x40, 0x41, 0x42, 0x43, 0x53, 0x47, 0x5F, 0x60, 0x61, 0x56, 0x5C, 0x5D, 0x5E, 0x57, 0x59, // 40-4F
            0x5A, 0x5B, 0x62, 0x63, 0, 0, 0x64, 0x44, 0x45, // 50-58
        ];
        *PLAIN.get(code as usize)?
    };
    (u != 0).then_some(u)
}
