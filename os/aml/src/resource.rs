//! Resource templates: the buffers a device's _CRS (and _PRS) gives, saying which I/O ports, memory, interrupts,
//! GPIO pins and serial-bus connections it uses (the ACPI specification, 6.5, section 6.4). The embedded
//! controller's ports come from here; so will an I2C touchpad's controller, address and interrupt.

use alloc::string::String;
use alloc::vec::Vec;

/// One resource a device uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    /// I/O ports: the lowest and highest the device may be put at, and how many (IO, FixedIO: the same).
    Io { min: u16, max: u16, length: u16 },
    /// Interrupts: the old PC's (IRQ, IRQNoFlags), or extended ones (Interrupt), by number.
    Interrupt { numbers: Vec<u32>, edge: bool, active_low: bool, shared: bool, wake: bool },
    /// Memory: the lowest and highest base, and how much (Memory24, Memory32, Memory32Fixed).
    Memory { min: u64, max: u64, length: u64, writable: bool },
    /// A range of addresses a bridge passes on (WordIO, DWordMemory, QWordMemory, WordBusNumber...).
    Range { kind: RangeKind, min: u64, max: u64, translation: u64, length: u64 },
    /// GPIO pins on a controller (its path): ones that interrupt (GpioInt), or ones to read and set (GpioIo).
    Gpio { controller: String, pins: Vec<u16>, interrupt: bool, edge: bool, active_low: bool, wake: bool },
    /// A device on an I2C bus: the controller's path, its address, the speed (Hz).
    I2c { controller: String, address: u16, ten_bit: bool, speed: u32 },
    /// Another serial bus (SPI, UART, CSI-2): its type, and the controller's path.
    SerialBus { kind: u8, controller: String },
    /// Something else: its descriptor's tag.
    Other(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeKind {
    Memory,
    Io,
    BusNumbers,
    Other(u8),
}

fn le(b: &[u8], at: usize, n: usize) -> Option<u64> {
    let s = b.get(at..at + n)?;
    Some(s.iter().rev().fold(0u64, |v, &x| v << 8 | x as u64))
}

/// A resource source's name (a controller's path), NUL-ended, from `at` up to `end`.
fn source(b: &[u8], at: usize, end: usize) -> String {
    let s = b.get(at..end.min(b.len())).unwrap_or(&[]);
    let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
    s.iter().map(|&c| c as char).collect()
}

/// A resource template's resources, in order, up to its end tag (or as far as it makes sense: a damaged descriptor
/// ends the list).
pub fn resources(b: &[u8]) -> Vec<Resource> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let tag = b[at];
        if tag & 0x80 == 0 {
            // A small descriptor: its kind in bits 6-3, its length in bits 2-0.
            let len = (tag & 7) as usize;
            let d = match b.get(at + 1..at + 1 + len) {
                Some(d) => d,
                None => break,
            };
            match tag >> 3 & 0x0F {
                0x04 if len >= 2 => {
                    let mask = le(d, 0, 2).unwrap_or(0) as u16;
                    let info = if len >= 3 { d[2] } else { 1 };
                    out.push(Resource::Interrupt {
                        numbers: (0..16).filter(|i| mask & 1 << i != 0).collect(),
                        edge: info & 1 != 0,
                        active_low: info & 8 != 0,
                        shared: info & 0x10 != 0,
                        wake: info & 0x20 != 0,
                    });
                }
                0x08 if len >= 7 => {
                    out.push(Resource::Io { min: le(d, 1, 2).unwrap_or(0) as u16, max: le(d, 3, 2).unwrap_or(0) as u16, length: d[6] as u16 })
                }
                0x09 if len >= 3 => {
                    let base = le(d, 0, 2).unwrap_or(0) as u16 & 0x3FF;
                    out.push(Resource::Io { min: base, max: base, length: d[2] as u16 })
                }
                0x0F => break,
                kind => out.push(Resource::Other(kind)),
            }
            at += 1 + len;
        } else {
            let Some(len) = le(b, at + 1, 2) else { break };
            let len = len as usize;
            let Some(d) = b.get(at + 3..at + 3 + len) else { break };
            let whole = &b[at..at + 3 + len];
            match tag {
                0x81 if len >= 9 => out.push(Resource::Memory {
                    min: le(d, 1, 2).unwrap_or(0) << 8,
                    max: le(d, 3, 2).unwrap_or(0) << 8,
                    length: le(d, 7, 2).unwrap_or(0) << 8,
                    writable: d[0] & 1 != 0,
                }),
                0x85 if len >= 17 => out.push(Resource::Memory {
                    min: le(d, 1, 4).unwrap_or(0),
                    max: le(d, 5, 4).unwrap_or(0),
                    length: le(d, 13, 4).unwrap_or(0),
                    writable: d[0] & 1 != 0,
                }),
                0x86 if len >= 9 => {
                    let base = le(d, 1, 4).unwrap_or(0);
                    out.push(Resource::Memory { min: base, max: base, length: le(d, 5, 4).unwrap_or(0), writable: d[0] & 1 != 0 })
                }
                0x87 | 0x88 | 0x8A | 0x8B => {
                    // Word, DWord, QWord and Extended address spaces: the same fields, of 2, 4 or 8 bytes.
                    let (n, first) = match tag {
                        0x88 => (2, 3),
                        0x87 => (4, 3),
                        0x8A => (8, 3),
                        _ => (8, 5),
                    };
                    if len < first + 5 * n {
                        out.push(Resource::Other(tag));
                    } else {
                        let kind = match d[0] {
                            0 => RangeKind::Memory,
                            1 => RangeKind::Io,
                            2 => RangeKind::BusNumbers,
                            k => RangeKind::Other(k),
                        };
                        let field = |i: usize| le(d, first + i * n, n).unwrap_or(0);
                        out.push(Resource::Range { kind, min: field(1), max: field(2), translation: field(3), length: field(4) });
                    }
                }
                0x89 if len >= 2 => {
                    let flags = d[0];
                    let count = d[1] as usize;
                    let numbers = (0..count).filter_map(|i| le(d, 2 + i * 4, 4).map(|n| n as u32)).collect();
                    out.push(Resource::Interrupt {
                        numbers,
                        edge: flags & 2 != 0,
                        active_low: flags & 4 != 0,
                        shared: flags & 8 != 0,
                        wake: flags & 0x10 != 0,
                    });
                }
                0x8C if len >= 20 => {
                    // GpioInt (connection type 0) and GpioIo (1); the pin table and the controller's name are found by
                    // offsets from the descriptor's start.
                    let interrupt = d[1] == 0;
                    let flags = le(d, 4, 2).unwrap_or(0);
                    let pins_at = le(d, 11, 2).unwrap_or(0) as usize;
                    let name_at = le(d, 14, 2).unwrap_or(0) as usize;
                    let vendor_at = le(d, 16, 2).unwrap_or(0) as usize;
                    let pins_end = name_at.min(whole.len());
                    let pins = (pins_at..pins_end).step_by(2).filter_map(|i| le(whole, i, 2).map(|p| p as u16)).collect();
                    let name_end = if vendor_at > name_at { vendor_at } else { whole.len() };
                    out.push(Resource::Gpio {
                        controller: source(whole, name_at, name_end),
                        pins,
                        interrupt,
                        edge: interrupt && flags & 1 != 0,
                        active_low: interrupt && flags >> 1 & 3 == 1,
                        wake: flags & 0x10 != 0,
                    });
                }
                0x8E if len >= 9 => {
                    let kind = d[2];
                    let type_flags = le(d, 4, 2).unwrap_or(0);
                    let data_len = le(d, 7, 2).unwrap_or(0) as usize;
                    let name_at = 3 + 9 + data_len;
                    let controller = source(whole, name_at, whole.len());
                    if kind == 1 && data_len >= 6 {
                        out.push(Resource::I2c {
                            controller,
                            speed: le(d, 9, 4).unwrap_or(0) as u32,
                            address: le(d, 13, 2).unwrap_or(0) as u16,
                            ten_bit: type_flags & 1 != 0,
                        });
                    } else {
                        out.push(Resource::SerialBus { kind, controller });
                    }
                }
                _ => out.push(Resource::Other(tag)),
            }
            at += 3 + len;
        }
    }
    out
}
