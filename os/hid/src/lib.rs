//! USB HID report descriptors (the HID specification 1.11, section 6.2.2): where in a mouse's reports its buttons,
//! movement and wheel are. A HID device describes its own reports this way, rather than using one fixed format, so
//! that each mouse can say how many buttons it has, how fine its movement is, and whether it has a wheel. (Keyboards
//! are read in the fixed "boot" format every keyboard supports, so they don't need this.)
//!
//! A descriptor is a list of items. Global items (the usage page, the size and count of the values that follow, their
//! logical range, the report they're in) stay in force until changed; local items (usages) say what the next main
//! item's values are; main items say what the reports hold (Input), and group them (Collection). A mouse's values are
//! in an application collection whose usage is Mouse or Pointer. Usages take their page the way Linux's parser gives
//! it to them: the page in force where each is declared, then, at the main item, those not on the last page declared
//! take that one (some devices declare a usage before its page). Nothing in a descriptor can make this read past its
//! end, loop, or take more memory than a few entries per item.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec::Vec;

#[cfg(test)]
mod tests;

// Usages: the page in the high 16 bits.
const POINTER: u32 = 0x0001_0001;
const MOUSE: u32 = 0x0001_0002;
const X: u32 = 0x0001_0030;
const Y: u32 = 0x0001_0031;
const WHEEL: u32 = 0x0001_0038;
const BUTTON_1: u32 = 0x0009_0001;
/// The consumer page's "AC Pan": a horizontal wheel, or a wheel that tilts.
const AC_PAN: u32 = 0x000C_0238;
/// More buttons than any mouse has (and than fit in a report's first bytes).
const BUTTONS_MAX: u32 = 32;
/// Nested collections and pushed global states beyond these are ignored: real descriptors use a handful.
const DEPTH_MAX: usize = 32;
/// Local usages kept for one main item; a mouse's are a few.
const USAGES_MAX: usize = 256;

/// One value in a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// The report it is in: its ID, or 0 if the device numbers none.
    pub report: u8,
    /// Where it starts, in bits from the start of the report's data (after its ID).
    pub bit: u32,
    /// Its size in bits, 1 to 32.
    pub size: u8,
    /// Whether it can be negative (its logical minimum is).
    pub signed: bool,
    /// Whether it is a change since the last report (movement, a wheel's turn) rather than a position.
    pub relative: bool,
}

impl Field {
    /// Its value in `report`, as it came from the device (with its ID first if `numbered`); None if the report is
    /// another one, or too short to have it.
    pub fn value(&self, report: &[u8], numbered: bool) -> Option<i32> {
        let data = if numbered {
            let (&id, rest) = report.split_first()?;
            if id != self.report {
                return None;
            }
            rest
        } else {
            report
        };
        let (bit, size) = (self.bit as usize, self.size as usize);
        if size == 0 || size > 32 || bit.checked_add(size)? > data.len().checked_mul(8)? {
            return None;
        }
        let mut v: u64 = 0;
        for i in 0..size {
            let b = bit + i;
            v |= ((data[b / 8] >> (b % 8)) as u64 & 1) << i;
        }
        let negative = self.signed && v >> (size - 1) & 1 != 0;
        Some(if negative { (v as i64 - (1i64 << size)) as i32 } else { v as i32 })
    }
}

/// Where a mouse's controls are in its reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pointer {
    /// Whether the device's reports start with a report ID (if one report has one, all do).
    pub numbered: bool,
    /// The first button (the left one) and how many follow it, one bit each.
    pub buttons: Option<(Field, u8)>,
    pub x: Option<Field>,
    pub y: Option<Field>,
    /// The scroll wheel: positive turned away from you.
    pub wheel: Option<Field>,
    /// The horizontal wheel: positive to the right.
    pub pan: Option<Field>,
}

impl Pointer {
    /// How many buttons it has.
    pub fn button_count(&self) -> u8 {
        self.buttons.map_or(0, |b| b.1)
    }

    /// The buttons held down in `report` (bit 0: the first button), if the report has them.
    pub fn buttons_in(&self, report: &[u8]) -> Option<u32> {
        let (first, count) = self.buttons?;
        let mut down = 0;
        for i in 0..count as u32 {
            let f = Field { bit: first.bit + i, ..first };
            down |= (f.value(report, self.numbered)? as u32 & 1) << i;
        }
        Some(down)
    }
}

/// The boot protocol's mouse report (the HID specification's appendix B.2), for a mouse whose report descriptor can't
/// be used: three buttons, then X and Y, a byte each. Most mice add their wheel as a fourth byte; a report without
/// one has no wheel value.
pub fn boot_mouse() -> Pointer {
    let byte = |bit| Some(Field { report: 0, bit, size: 8, signed: true, relative: true });
    Pointer {
        numbered: false,
        buttons: Some((Field { report: 0, bit: 0, size: 1, signed: false, relative: false }, 3)),
        x: byte(8),
        y: byte(16),
        wheel: byte(24),
        pan: None,
    }
}

/// The global items' state, pushed and popped whole.
#[derive(Clone, Copy, Default)]
struct Globals {
    page: u32,
    /// The logical minimum (signed), and the logical maximum as it came (it is read as signed only when the minimum
    /// is negative: many devices write 255 as one byte, 0xFF).
    minimum: i64,
    size: u32,
    count: u32,
    report: u8,
}

/// A usage, or a range of them (a Usage Minimum and Maximum), from the local items.
#[derive(Clone, Copy)]
struct Usages {
    first: u32,
    last: u32,
    /// Given as a whole 32-bit usage, page included: never given another page.
    extended: bool,
}

impl Usages {
    fn len(&self) -> u64 {
        self.last as u64 - self.first as u64 + 1
    }
    fn page(&self) -> u32 {
        self.first >> 16
    }
}

/// The `n`th usage of a main item's values: in turn from the list; past its end, the last one again.
fn usage_at(usages: &[Usages], n: u64) -> Option<u32> {
    let mut n = n;
    for u in usages {
        if n < u.len() {
            return Some(u.first + n as u32);
        }
        n -= u.len();
    }
    usages.last().map(|u| u.last)
}

/// A usage from an item of `size` bytes: 4 bytes are a whole usage; fewer take the usage page in force.
fn complete(data: u32, size: usize, page: u32) -> (u32, bool) {
    if size == 4 { (data, true) } else { (page << 16 | (data & 0xFFFF), false) }
}

/// Where a mouse's controls are, if the descriptor has a mouse.
pub fn parse(d: &[u8]) -> Option<Pointer> {
    let mut g = Globals::default();
    let mut pushed: Vec<Globals> = Vec::new();
    let mut usages: Vec<Usages> = Vec::new();
    // A Usage Minimum waiting for its Usage Maximum, and its item's size.
    let mut minimum: Option<(u32, usize)> = None;
    // The usages of the collections open now; the first is the application's.
    let mut collections: Vec<u32> = Vec::new();
    // How many bits of each report the Input items so far take.
    let mut taken: Vec<(u8, u64)> = Vec::new();
    let mut p = Pointer::default();
    let mut i = 0;
    while i < d.len() {
        let prefix = d[i];
        if prefix == 0xFE {
            // A long item: its data's size, its tag, then the data. None is defined.
            let Some(&size) = d.get(i + 1) else { break };
            i += 3 + size as usize;
            continue;
        }
        let size = [0, 1, 2, 4][(prefix & 3) as usize];
        let Some(bytes) = d.get(i + 1..i + 1 + size) else { break };
        let data = bytes.iter().rev().fold(0u32, |a, &b| a << 8 | b as u32);
        let signed = match size {
            1 => data as u8 as i8 as i64,
            2 => data as u16 as i16 as i64,
            4 => data as i32 as i64,
            _ => 0,
        };
        i += 1 + size;
        let tag = prefix >> 4;
        match (prefix >> 2) & 3 {
            // Main items.
            0 => {
                // Usages declared before their page take the last page declared (as Linux has it).
                for u in usages.iter_mut().rev().filter(|u| !u.extended) {
                    if u.page() == g.page {
                        break;
                    }
                    u.first = g.page << 16 | (u.first & 0xFFFF);
                    u.last = g.page << 16 | (u.last & 0xFFFF);
                }
                match tag {
                    0x8 => {
                        let report = g.report;
                        let at = match taken.iter().position(|t| t.0 == report) {
                            Some(k) => k,
                            None => {
                                taken.push((report, 0));
                                taken.len() - 1
                            }
                        };
                        let start = taken[at].1;
                        taken[at].1 = start.saturating_add(g.size as u64 * g.count as u64);
                        let (constant, variable, relative) = (data & 1 != 0, data & 2 != 0, data & 4 != 0);
                        let mouse = matches!(collections.first(), Some(&MOUSE | &POINTER));
                        if mouse && !constant && variable && (1..=32).contains(&g.size) {
                            input(&mut p, &usages, g, start, relative);
                        }
                    }
                    0xA => {
                        if collections.len() < DEPTH_MAX {
                            collections.push(usage_at(&usages, 0).unwrap_or(0));
                        }
                    }
                    0xC => {
                        collections.pop();
                    }
                    _ => {} // Output and Feature: nothing a mouse's movement needs
                }
                usages.clear();
                minimum = None;
            }
            // Global items.
            1 => match tag {
                0x0 => g.page = data & 0xFFFF,
                0x1 => g.minimum = signed,
                0x7 => g.size = data,
                0x8 => {
                    g.report = data as u8;
                    p.numbered = true;
                }
                0x9 => g.count = data,
                0xA if pushed.len() < DEPTH_MAX => pushed.push(g),
                0xB => {
                    if let Some(top) = pushed.pop() {
                        g = top;
                    }
                }
                _ => {} // the logical maximum, physical range and units: not needed
            },
            // Local items.
            2 => match tag {
                0x0 if usages.len() < USAGES_MAX => {
                    let (u, extended) = complete(data, size, g.page);
                    usages.push(Usages { first: u, last: u, extended });
                }
                0x1 => minimum = Some((data, size)),
                0x2 => {
                    if let Some((min, min_size)) = minimum.take()
                        && usages.len() < USAGES_MAX
                    {
                        let (first, _) = complete(min, min_size, g.page);
                        let (last, extended) = complete(data, size, g.page);
                        if last >= first {
                            usages.push(Usages { first, last, extended });
                        }
                    }
                }
                _ => {} // designators, strings, delimiters
            },
            _ => {} // reserved
        }
    }
    let found = p.x.is_some() || p.y.is_some() || p.wheel.is_some() || p.buttons.is_some();
    found.then_some(p)
}

/// An Input item's values (variables, in a mouse's collection): the controls a mouse has, each the first found.
fn input(p: &mut Pointer, usages: &[Usages], g: Globals, start: u64, relative: bool) {
    // Values past the end of the usages listed have the last one again, which can't be a control not seen yet; and
    // no mouse has a thousand controls.
    let listed: u64 = usages.iter().map(|u| u.len()).sum();
    for n in 0..(g.count as u64).min(listed).min(1024) {
        let Some(usage) = usage_at(usages, n) else { return };
        let bit = start + n * g.size as u64;
        let Ok(bit) = u32::try_from(bit) else { return };
        let field = Field { report: g.report, bit, size: g.size as u8, signed: g.minimum < 0, relative };
        match usage {
            X => {
                p.x.get_or_insert(field);
            }
            Y => {
                p.y.get_or_insert(field);
            }
            WHEEL => {
                p.wheel.get_or_insert(field);
            }
            AC_PAN => {
                p.pan.get_or_insert(field);
            }
            b if (BUTTON_1..BUTTON_1 + BUTTONS_MAX).contains(&b) && g.size == 1 => {
                let k = b - BUTTON_1;
                match &mut p.buttons {
                    None if k == 0 => p.buttons = Some((field, 1)),
                    // The next button, in the next bit of the same report.
                    Some((first, count)) if k == *count as u32 && first.report == field.report && first.bit + k == bit => *count += 1,
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
