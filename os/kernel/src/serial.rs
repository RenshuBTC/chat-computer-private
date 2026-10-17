//! The kernel's log, written to the first serial port (COM1, a 16550-compatible UART).
//! On a PC without one, writes go nowhere. In a virtual PC, the log is how tests see what happened.

use crate::cpu::{inb, outb};
use crate::sync::SpinLock;
use core::fmt::{self, Write};

const COM1: u16 = 0x3F8;

struct Port {
    present: bool,
}

static PORT: SpinLock<Port> = SpinLock::new(Port { present: false });

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
    fn put(&mut self, byte: u8) {
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
            if b == b'\n' {
                self.put(b'\r');
            }
            self.put(b);
        }
        Ok(())
    }
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
            let mut port = Port { present: true };
            let _ = port.write_fmt(args);
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::serial::write(format_args!("{}\n", format_args!($($arg)*))) };
}
