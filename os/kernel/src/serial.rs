//! The kernel's log, written to the first serial port (COM1, a 16550-compatible UART), and kept in memory as it's
//! written, for its file on the start-up partition (journal.rs), which Windows' setup reads back: most PCs have no
//! serial port, where writes to it go nowhere. In a virtual PC, the serial port is how tests see what happened.

use crate::cpu::{inb, outb};
use crate::sync::SpinLock;
use alloc::format;
use alloc::vec::Vec;
use core::fmt::{self, Write};

const COM1: u16 = 0x3F8;

/// What's kept in memory: the log's first part as it was written (its start, which says the most about a PC), and
/// its latest part, in a ring.
const FIRST: usize = 24 * 1024;
const LATEST: usize = 40 * 1024;

struct Port {
    present: bool,
    first: [u8; FIRST],
    first_len: usize,
    latest: [u8; LATEST],
    /// Where the next byte goes in `latest`, and how much of it is filled.
    latest_at: usize,
    latest_len: usize,
    /// Bytes written since the start, and how many of them are kept in neither part.
    written: u64,
    left_out: u64,
}

static PORT: SpinLock<Port> = SpinLock::new(Port {
    present: false,
    first: [0; FIRST],
    first_len: 0,
    latest: [0; LATEST],
    latest_at: 0,
    latest_len: 0,
    written: 0,
    left_out: 0,
});

pub fn init() {
    let mut port = PORT.lock();
    unsafe {
        outb(COM1 + 1, 0x00); // no interrupts
        outb(COM1 + 3, 0x80); // set the speed...
        outb(COM1, 0x01); // ...115200 bits per second
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8 data bits, no parity, 1 stop bit
        outb(COM1 + 2, 0xC7); // buffers on and cleared
        // Is a UART there? Send one byte to itself.
        outb(COM1 + 4, 0x1E); // loopback
        outb(COM1, 0xAE);
        port.present = inb(COM1) == 0xAE;
        outb(COM1 + 4, 0x0F); // normal operation
    }
}

impl Port {
    /// A byte kept in memory.
    fn keep(&mut self, byte: u8) {
        self.written += 1;
        if self.first_len < FIRST {
            self.first[self.first_len] = byte;
            self.first_len += 1;
            return;
        }
        if self.latest_len == LATEST {
            self.left_out += 1;
        } else {
            self.latest_len += 1;
        }
        self.latest[self.latest_at] = byte;
        self.latest_at = (self.latest_at + 1) % LATEST;
    }

    /// A byte to the serial port, if there's one.
    fn send(&mut self, byte: u8) {
        if !self.present {
            return;
        }
        let mut spins = 0;
        while unsafe { inb(COM1 + 5) } & 0x20 == 0 && spins < 100_000 {
            spins += 1;
        }
        unsafe { outb(COM1, byte) };
    }
}

impl Write for Port {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            // (The kept log has plain line ends; the port, a terminal's.)
            self.keep(b);
            if b == b'\n' {
                self.send(b'\r');
            }
            self.send(b);
        }
        Ok(())
    }
}

impl Port {
    /// The log as kept: its first part, a line saying how much was left out (if anything was), its latest part.
    fn kept(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.first_len + self.latest_len + 80);
        out.extend_from_slice(&self.first[..self.first_len]);
        if self.left_out > 0 {
            out.extend_from_slice(format!("\n[... {} bytes of the log left out ...]\n", self.left_out).as_bytes());
        }
        let start = (self.latest_at + LATEST - self.latest_len) % LATEST;
        for i in 0..self.latest_len {
            out.push(self.latest[(start + i) % LATEST]);
        }
        out
    }
}

/// The log as kept in memory, and how many bytes had been written when it was taken (to see whether there's more
/// since). `None` if it's being written just now (for a crash's last words: never waits).
pub fn kept(wait: bool) -> Option<(Vec<u8>, u64)> {
    let port = if wait { Some(PORT.lock()) } else { PORT.try_lock() }?;
    Some((port.kept(), port.written))
}

/// How many bytes the log has had written to it.
pub fn written() -> u64 {
    PORT.lock().written
}

pub fn write(args: fmt::Arguments) {
    let _ = PORT.lock().write_fmt(args);
}

/// For the last words of a crash: doesn't wait for the lock (the crash may have happened while it was
/// held).
pub fn write_anyway(args: fmt::Arguments) {
    match PORT.try_lock() {
        Some(mut port) => {
            let _ = port.write_fmt(args);
        }
        None => {
            // (Straight to the port: not kept, the kept log being in the middle of a change.)
            struct Raw;
            impl Write for Raw {
                fn write_str(&mut self, s: &str) -> fmt::Result {
                    for b in s.bytes() {
                        let mut spins = 0;
                        while unsafe { inb(COM1 + 5) } & 0x20 == 0 && spins < 100_000 {
                            spins += 1;
                        }
                        if b == b'\n' {
                            unsafe { outb(COM1, b'\r') };
                        }
                        unsafe { outb(COM1, b) };
                    }
                    Ok(())
                }
            }
            let _ = Raw.write_fmt(args);
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::serial::write(format_args!("{}\n", format_args!($($arg)*))) };
}
