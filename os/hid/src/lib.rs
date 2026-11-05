//! USB HID report descriptors (the HID specification 1.11, section 6.2.2): where in a mouse's reports its buttons,
//! movement and wheel are, and where in a keyboard's its keys are. A HID device describes its own reports this way,
//! rather than using one fixed format, so that each mouse can say how many buttons it has, how fine its movement is,
//! and whether it has a wheel, and each keyboard how many keys it tells at once: six in a list, as every keyboard's
//! "boot" format has them, or every key a bit of its own (gaming keyboards' "n-key rollover"), often in another
//! interface than the boot one, which such a keyboard may leave silent.
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
const KEYBOARD: u32 = 0x0001_0006;
const KEYPAD: u32 = 0x0001_0007;
/// The keyboard's page (its keys' usages: 4 is A, 0xE0 to 0xE7 are the modifiers) and the lights'.
const KEY_PAGE: u32 = 0x07;
const LED_PAGE: u32 = 0x08;
/// The keys' usage for "too many keys at once to tell which".
pub const ROLLOVER: u16 = 1;
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

/// What a main item says its values are: input (what the device sends) or output (what it's sent: a keyboard's
/// lights).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Main {
    Input,
    Output,
}

/// A main item's values, as the walk found them: the usages listed for them, the global items in force, where they
/// start (in bits, in their report), the item's flags (bit 0: constant; 1: variable, else an array; 2: relative),
/// and the usage of the application collection they're in. A report's bits are counted for input and output apart.
struct Values<'u> {
    main: Main,
    usages: &'u [Usages],
    g: Globals,
    start: u64,
    flags: u32,
    application: Option<u32>,
}

/// Goes through a descriptor's items, telling `found` about each Input and Output item. Returns whether its reports
/// are numbered (a Report ID item anywhere: then every report starts with its ID).
fn walk(d: &[u8], mut found: impl FnMut(&Values)) -> bool {
    let mut g = Globals::default();
    let mut pushed: Vec<Globals> = Vec::new();
    let mut usages: Vec<Usages> = Vec::new();
    // A Usage Minimum waiting for its Usage Maximum, and its item's size.
    let mut minimum: Option<(u32, usize)> = None;
    // The usages of the collections open now; the first is the application's.
    let mut collections: Vec<u32> = Vec::new();
    // How many bits of each report the Input (and, apart, the Output) items so far take.
    let mut taken: Vec<(Main, u8, u64)> = Vec::new();
    let mut numbered = false;
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
                    0x8 | 0x9 => {
                        let main = if tag == 0x8 { Main::Input } else { Main::Output };
                        let report = g.report;
                        let at = match taken.iter().position(|t| t.0 == main && t.1 == report) {
                            Some(k) => k,
                            None => {
                                taken.push((main, report, 0));
                                taken.len() - 1
                            }
                        };
                        let start = taken[at].2;
                        taken[at].2 = start.saturating_add(g.size as u64 * g.count as u64);
                        found(&Values { main, usages: &usages, g, start, flags: data, application: collections.first().copied() });
                    }
                    0xA => {
                        if collections.len() < DEPTH_MAX {
                            collections.push(usage_at(&usages, 0).unwrap_or(0));
                        }
                    }
                    0xC => {
                        collections.pop();
                    }
                    _ => {} // Feature: nothing a mouse or keyboard needs
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
                    numbered = true;
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
    numbered
}

/// Where a mouse's controls are, if the descriptor has a mouse.
pub fn parse(d: &[u8]) -> Option<Pointer> {
    let mut p = Pointer::default();
    p.numbered = walk(d, |v| {
        let (constant, variable, relative) = (v.flags & 1 != 0, v.flags & 2 != 0, v.flags & 4 != 0);
        if v.main == Main::Input && matches!(v.application, Some(MOUSE | POINTER)) && !constant && variable && (1..=32).contains(&v.g.size) {
            input(&mut p, v.usages, v.g, v.start, relative);
        }
    });
    let found = p.x.is_some() || p.y.is_some() || p.wheel.is_some() || p.buttons.is_some();
    found.then_some(p)
}

/// A list of the keys held, in a keyboard's report: each element a key's index among its usages (from `minimum`),
/// or out of their range for none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyArray {
    pub first: Field,
    pub count: u16,
    pub minimum: i64,
    /// The usages the elements index, in ranges (on the keyboard's page).
    pub usages: Vec<(u16, u16)>,
}

/// Where a keyboard's keys are in its reports, and its lights in what it's sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// Whether the device's reports start with a report ID.
    pub numbered: bool,
    /// Keys one bit each, in runs: the first one's place, its usage, and how many follow (the modifiers are such a
    /// run, and so is a keyboard's every key when it tells them all at once).
    pub bits: Vec<(Field, u16, u16)>,
    /// Lists of the keys held (six, in the boot format).
    pub arrays: Vec<KeyArray>,
    /// Its lights, each a bit of an output report: (Num Lock 1, Caps Lock 2, Scroll Lock 3, as the LED page numbers
    /// them), and the output report's length in bytes.
    pub lights: Vec<(Field, u8)>,
    pub lights_bytes: u16,
}

/// What a keyboard's report says: the modifiers held (bit 0: left Ctrl, as the boot format has them), the other
/// keys held, and whether it said it had too many keys held at once to tell which.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub modifiers: u8,
    pub keys: Vec<u16>,
    pub rollover: bool,
}

impl Keys {
    /// The keys held, as `report` says (with its ID first if the reports are numbered); None if the report isn't one
    /// of the keyboard's.
    pub fn held(&self, report: &[u8]) -> Option<Held> {
        let mut h = Held::default();
        let mut any = false;
        let key = |h: &mut Held, usage: u16| match usage {
            0xE0..=0xE7 => h.modifiers |= 1 << (usage - 0xE0),
            ROLLOVER => h.rollover = true,
            u if u >= 4 && !h.keys.contains(&u) && h.keys.len() < 64 => h.keys.push(u),
            _ => {}
        };
        for (first, usage, count) in &self.bits {
            for i in 0..*count as u32 {
                let f = Field { bit: first.bit + i, ..*first };
                let Some(v) = f.value(report, self.numbered) else { break };
                any = true;
                if v & 1 != 0 {
                    key(&mut h, usage + i as u16);
                }
            }
        }
        for a in &self.arrays {
            let total: i64 = a.usages.iter().map(|(f, l)| (*l - *f) as i64 + 1).sum();
            for i in 0..a.count as u32 {
                let f = Field { bit: a.first.bit + i * a.first.size as u32, ..a.first };
                let Some(v) = f.value(report, self.numbered) else { break };
                any = true;
                let mut index = v as i64 - a.minimum;
                if index < 0 || index >= total {
                    continue; // no key
                }
                for (f, l) in &a.usages {
                    let len = (*l - *f) as i64 + 1;
                    if index < len {
                        key(&mut h, f + index as u16);
                        break;
                    }
                    index -= len;
                }
            }
        }
        any.then_some(h)
    }

    /// Whether it has lights to set.
    pub fn has_lights(&self) -> bool {
        !self.lights.is_empty()
    }

    /// The output report that sets its lights (`lights`: bit 0 Num Lock, 1 Caps Lock, 2 Scroll Lock, as the boot
    /// format has them): its ID (0 if none) and its bytes, the ID first if the reports are numbered.
    pub fn lights_report(&self, lights: u8) -> Option<(u8, Vec<u8>)> {
        let (first, _) = self.lights.first()?;
        let report = first.report;
        let mut data = alloc::vec![0u8; self.lights_bytes.clamp(1, 64) as usize];
        for (f, led) in self.lights.iter().filter(|(f, _)| f.report == report) {
            let on = (1..=3).contains(led) && lights >> (led - 1) & 1 != 0;
            let byte = (f.bit / 8) as usize;
            if on && byte < data.len() {
                data[byte] |= 1 << (f.bit % 8);
            }
        }
        if self.numbered {
            data.insert(0, report);
        }
        Some((report, data))
    }
}

/// Where a keyboard's keys are, if the descriptor has a keyboard (or a keypad).
pub fn parse_keys(d: &[u8]) -> Option<Keys> {
    let mut k = Keys::default();
    let mut lights_bits: Vec<(u8, u64)> = Vec::new();
    k.numbered = walk(d, |v| {
        if !matches!(v.application, Some(KEYBOARD | KEYPAD)) || v.flags & 1 != 0 || !(1..=32).contains(&v.g.size) {
            return;
        }
        let listed: u64 = v.usages.iter().map(|u| u.len()).sum();
        match v.main {
            Main::Input if v.flags & 2 != 0 => {
                // Keys a bit each.
                if v.g.size != 1 {
                    return;
                }
                for n in 0..(v.g.count as u64).min(listed).min(1024) {
                    let Some(usage) = usage_at(v.usages, n) else { return };
                    if usage >> 16 != KEY_PAGE || usage & 0xFFFF > 0xFFFF {
                        continue;
                    }
                    let Ok(bit) = u32::try_from(v.start + n) else { return };
                    let usage = (usage & 0xFFFF) as u16;
                    let field = Field { report: v.g.report, bit, size: 1, signed: false, relative: false };
                    let runs = k.bits.len();
                    match k.bits.last_mut() {
                        Some((first, u, count)) if first.report == field.report && first.bit + *count as u32 == bit && *u + *count == usage => *count += 1,
                        _ if runs < 64 => k.bits.push((field, usage, 1)),
                        _ => {}
                    }
                }
            }
            Main::Input => {
                // A list of the keys held.
                let ranges: Vec<(u16, u16)> = v
                    .usages
                    .iter()
                    .filter(|u| u.page() == KEY_PAGE)
                    .map(|u| ((u.first & 0xFFFF) as u16, (u.last & 0xFFFF) as u16))
                    .take(16)
                    .collect();
                if ranges.is_empty() || k.arrays.len() >= 8 {
                    return;
                }
                let Ok(bit) = u32::try_from(v.start) else { return };
                let first = Field { report: v.g.report, bit, size: v.g.size as u8, signed: v.g.minimum < 0, relative: false };
                k.arrays.push(KeyArray { first, count: v.g.count.min(256) as u16, minimum: v.g.minimum, usages: ranges });
            }
            Main::Output if v.flags & 2 != 0 && v.g.size == 1 => {
                for n in 0..(v.g.count as u64).min(listed).min(64) {
                    let Some(usage) = usage_at(v.usages, n) else { return };
                    let Ok(bit) = u32::try_from(v.start + n) else { return };
                    if usage >> 16 == LED_PAGE && (1..=3).contains(&(usage & 0xFFFF)) && k.lights.len() < 8 {
                        k.lights.push((Field { report: v.g.report, bit, size: 1, signed: false, relative: false }, usage as u8));
                    }
                }
                let end = v.start + v.g.size as u64 * v.g.count as u64;
                match lights_bits.iter_mut().find(|(r, _)| *r == v.g.report) {
                    Some((_, e)) => *e = (*e).max(end),
                    None => lights_bits.push((v.g.report, end)),
                }
            }
            Main::Output => {
                let end = v.start + v.g.size as u64 * v.g.count as u64;
                match lights_bits.iter_mut().find(|(r, _)| *r == v.g.report) {
                    Some((_, e)) => *e = (*e).max(end),
                    None => lights_bits.push((v.g.report, end)),
                }
            }
        }
    });
    if let Some((f, _)) = k.lights.first() {
        let bits = lights_bits.iter().find(|(r, _)| *r == f.report).map_or(8, |(_, e)| *e);
        k.lights_bytes = bits.div_ceil(8).min(64) as u16;
    }
    // A keyboard tells keys, not only lights.
    let keys = k.bits.iter().any(|(_, u, c)| (*u as u32 + *c as u32) > 4 && *u < 0xE0) || !k.arrays.is_empty();
    keys.then_some(k)
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
