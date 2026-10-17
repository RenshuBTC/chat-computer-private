//! The first screen ClaudeOS's own kernel shows: that it is running, what it found, how long it has
//! been up (counted by its own timer interrupts), a line to type on with whatever keyboard is plugged
//! in, and how to get back to Windows. Laid out like ClaudeOS's welcome screen: a larger first line,
//! then text, in one column in the middle, black on white. If the kernel ever stops, the same place
//! says so, with details to pass on.

use crate::keyboard::{Key, Press};
use crate::layout::{self, Layout};
use crate::screen::Screen;
use crate::sync::SpinLock;
use crate::text::{Style, Type};
use crate::{heap, memory, time, usb};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::{SECURE_BOOT_OFF, SECURE_BOOT_ON};
use core::fmt;
use font::raster::ONE;

const BACK_TO_WINDOWS: &str = "To go back to Windows, hold the power button until the PC turns off, then turn it on \
                               again. ClaudeOS doesn't write to your disks, so this is safe.";

/// The longest line that can be typed.
const TYPED_MAX: usize = 1000;

pub struct Welcome {
    screen: Screen,
    ty: Type,
    layout: Layout,
    body: Style,
    column_x: usize,
    /// Where the live part starts (the "Up for" line, the keyboard lines, then how to go back), in
    /// 1/256 pixel.
    live_top: i32,
    shown: Option<u64>,
    /// The line about keyboards, and the line typed on.
    keyboard_top: i32,
    typing_top: i32,
    typed: String,
    /// Where in `typed` the caret is (a byte index on a character boundary).
    cursor: usize,
}

static WELCOME: SpinLock<Option<Welcome>> = SpinLock::new(None);

fn fx(px: f64) -> i32 {
    (px * ONE as f64) as i32
}

fn plural(n: u64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

fn uptime_text(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("Up for {} and {}.", plural(h, "hour"), plural(m, "minute"))
    } else if m > 0 {
        format!("Up for {} and {}.", plural(m, "minute"), plural(s, "second"))
    } else {
        format!("Up for {}.", plural(s, "second"))
    }
}

fn memory_text(bytes: u64) -> String {
    let mib = (bytes + (1 << 19)) >> 20;
    if mib < 1024 {
        return format!("{mib} MB");
    }
    let tenths = (bytes * 10 + (1 << 29)) >> 30;
    if tenths < 100 { format!("{}.{} GB", tenths / 10, tenths % 10) } else { format!("{} GB", (tenths + 5) / 10) }
}

impl Welcome {
    /// Shows the first screen. Afterwards, `tick` keeps the "Up for" line current.
    pub fn show(screen: Screen, size_mm: Option<(u32, u32)>, cpu: &str, secure_boot: u32) -> Layout {
        let layout = layout::compute(screen.width as u32, screen.height as u32, size_mm);
        let ty = Type::atkinson();
        let rem = layout.font_px;
        let body = Style { size: fx(rem), line_height: fx(rem * 1.5) };
        let greeting = Style { size: fx(rem * 1.5), line_height: fx(rem * 1.5 * 1.3) };
        let column = layout.column_px as i32;
        let column_x = (screen.width.saturating_sub(column as usize)) / 2;

        let article = if cpu.starts_with(['A', 'E', 'I', 'O', 'a', 'e', 'i', 'o']) { "an" } else { "a" };
        let size = if layout.size_known {
            format!("{}-inch, {} by {}", (layout.diagonal_in + 0.5) as u32, screen.width, screen.height)
        } else {
            format!("{} by {}", screen.width, screen.height)
        };
        let memory = memory_text(memory::stats().total_ram);
        let secure = match secure_boot {
            SECURE_BOOT_ON => " Secure Boot is on.",
            SECURE_BOOT_OFF => " Secure Boot is off.",
            _ => "",
        };
        let facts = format!("This PC has {article} {cpu}, {memory} of memory and a {size} screen.{secure}");

        let title = ty.wrap("ClaudeOS is running on its own kernel.", greeting.size, column * ONE);
        let lines: Vec<&str> = ty.wrap(&facts, body.size, column * ONE);
        let back = ty.wrap(BACK_TO_WINDOWS, body.size, column * ONE);
        // Heights, as on ClaudeOS's welcome screen: the greeting (1.5 rem, line height 1.3) with 0.25 em
        // below it, then paragraphs (line height 1.5) 1 em apart.
        let gap_small = fx(rem * 1.5 * 0.25);
        let gap = fx(rem);
        let total = greeting.line_height * title.len() as i32
            + gap_small
            + body.line_height * lines.len() as i32
            + gap
            + body.line_height
            + gap
            + body.line_height * 2
            + gap
            + body.line_height * back.len() as i32;
        let mut top = ((screen.height as i32 * ONE) - total) / 2;

        let mut w = Welcome {
            screen,
            ty,
            layout,
            body,
            column_x,
            live_top: 0,
            shown: None,
            keyboard_top: 0,
            typing_top: 0,
            typed: String::new(),
            cursor: 0,
        };
        w.screen.fill(0, 0, w.screen.width, w.screen.height, 255);
        for line in &title {
            w.put(line, greeting, top);
            top += greeting.line_height;
        }
        top += gap_small;
        for line in &lines {
            w.put(line, body, top);
            top += body.line_height;
        }
        top += gap;
        w.live_top = top;
        w.tick();
        top += body.line_height + gap;
        w.keyboard_top = top;
        w.put("Looking for keyboards.", body, top);
        top += body.line_height;
        w.typing_top = top;
        w.draw_typing();
        top += body.line_height + gap;
        for line in &back {
            w.put(line, body, top);
            top += body.line_height;
        }
        let layout = w.layout;
        *WELCOME.lock() = Some(w);
        layout
    }

    fn put(&mut self, text: &str, style: Style, top: i32) {
        let row = top.div_euclid(ONE);
        let ink = self.ty.ink(text, style, top - row * ONE);
        let x = self.column_x as isize - ink.left as isize;
        self.screen.draw_ink(x, row as isize, ink.width, ink.height, &ink.coverage);
    }

    /// Like `put`, but nothing past `max_px` pixels from the column's left edge.
    fn put_clipped(&mut self, text: &str, style: Style, top: i32, max_px: usize) {
        let row = top.div_euclid(ONE);
        let ink = self.ty.ink(text, style, top - row * ONE);
        let keep = (max_px + ink.left).min(ink.width);
        let mut coverage = Vec::with_capacity(keep * ink.height);
        for r in 0..ink.height {
            coverage.extend_from_slice(&ink.coverage[r * ink.width..r * ink.width + keep]);
        }
        let x = self.column_x as isize - ink.left as isize;
        self.screen.draw_ink(x, row as isize, keep, ink.height, &coverage);
    }

    /// The line about keyboards: shorter forms if the long one doesn't fit on one line.
    /// `other`: a keyboard found another way (the PS/2 controller), by name.
    fn show_keyboards(&mut self, status: &usb::Status, other: Option<&str>) {
        let mut all: Vec<&str> = status.keyboards.iter().map(|k| k.as_str()).collect();
        all.extend(other);
        // Keyboards of the same name once, with how many: "QEMU USB Keyboard (2)".
        let mut names: Vec<(&str, usize)> = Vec::new();
        for k in &all {
            match names.iter_mut().find(|(n, _)| n == k) {
                Some((_, count)) => *count += 1,
                None => names.push((k, 1)),
            }
        }
        let named: Vec<String> =
            names.iter().map(|(n, count)| if *count > 1 { format!("{n} ({count})") } else { String::from(*n) }).collect();
        let long = match all.as_slice() {
            [] if status.controllers == 0 => String::from("No keyboard ClaudeOS can use was found, so typing doesn't work yet."),
            [] if status.running == 0 => String::from("The USB controllers didn't start, so typing doesn't work yet."),
            [] => String::from("Plug in a USB keyboard to type here."),
            [one] => format!("Type here with the {one}."),
            _ => format!("Type here with any keyboard: {}.", named.join(", ")),
        };
        let column = self.layout.column_px as i32 * ONE;
        let text = if self.ty.width(&long, self.body.size) <= column {
            long
        } else if all.len() == 1 {
            String::from("Type here with the keyboard.")
        } else {
            format!("Type here with any of the {} keyboards.", all.len())
        };
        self.clear(self.keyboard_top, 1);
        self.put(&text, self.body, self.keyboard_top);
    }

    /// The line typed on, with the caret. When it is longer than the column, it scrolls so the caret
    /// stays in view.
    fn draw_typing(&mut self) {
        let style = self.body;
        let column_px = self.layout.column_px as usize;
        let caret_w = ((2.0 * self.layout.scale + 0.5) as usize).max(1);
        let room = (column_px.saturating_sub(caret_w + 2) as i32) * ONE;
        let mut start = 0;
        while start < self.cursor && self.ty.width(&self.typed[start..self.cursor], style.size) > room {
            start += self.typed[start..].chars().next().map_or(1, |c| c.len_utf8());
        }
        self.clear(self.typing_top, 1);
        if start < self.typed.len() {
            let visible = String::from(&self.typed[start..]);
            self.put_clipped(&visible, style, self.typing_top, column_px);
        }
        let x = self.column_x + (self.ty.width(&self.typed[start..self.cursor], style.size) / ONE) as usize;
        let row = self.typing_top.div_euclid(ONE) as usize;
        let line = (style.line_height / ONE) as usize;
        self.screen.fill(x, row + line * 18 / 100, caret_w, line * 64 / 100, 0);
    }

    fn edit(&mut self, p: Press) {
        let before = |s: &str, at: usize| s[..at].chars().next_back().map_or(at, |c| at - c.len_utf8());
        let after = |s: &str, at: usize| s[at..].chars().next().map_or(at, |c| at + c.len_utf8());
        match p.key {
            Key::Char(c) if !p.ctrl && !p.alt && !p.logo => {
                if self.typed.chars().count() < TYPED_MAX {
                    self.typed.insert(self.cursor, c);
                    self.cursor += c.len_utf8();
                }
            }
            Key::Backspace if self.cursor > 0 => {
                let from = before(&self.typed, self.cursor);
                self.typed.replace_range(from..self.cursor, "");
                self.cursor = from;
            }
            Key::Delete if self.cursor < self.typed.len() => {
                let to = after(&self.typed, self.cursor);
                self.typed.replace_range(self.cursor..to, "");
            }
            Key::Left => self.cursor = before(&self.typed, self.cursor),
            Key::Right => self.cursor = after(&self.typed, self.cursor),
            Key::Home => self.cursor = 0,
            Key::End => self.cursor = self.typed.len(),
            Key::Enter => {
                // Typed lines go nowhere yet. A test build writes them in the log, so a test can check
                // what reached the screen; a normal build never logs what is typed.
                if option_env!("CLAUDEOS_TEST") == Some("keyboard") {
                    crate::log!("ClaudeOS kernel: test: typed {:?}", self.typed);
                }
                self.typed.clear();
                self.cursor = 0;
            }
            Key::Escape => {
                self.typed.clear();
                self.cursor = 0;
            }
            _ => return,
        }
        self.draw_typing();
    }

    /// Clears `lines` lines of the column, starting at `top`.
    fn clear(&mut self, top: i32, lines: usize) {
        let row = top.div_euclid(ONE) as usize;
        let height = (self.body.line_height as usize * lines) / ONE as usize + 2;
        let width = self.layout.column_px as usize + 8;
        self.screen.fill(self.column_x.saturating_sub(4), row, width, height, 255);
    }

    fn tick(&mut self) {
        let seconds = time::uptime_ms() / 1000;
        if self.shown == Some(seconds) {
            return;
        }
        self.shown = Some(seconds);
        self.clear(self.live_top, 1);
        let text = uptime_text(seconds);
        self.put(&text, self.body, self.live_top);
    }

    /// Replaces the live part with what stopped the kernel, the details, and how to go back.
    fn stop(&mut self, what: &str, details: &str) {
        let column = self.layout.column_px as i32 * ONE;
        let gap = fx(self.layout.font_px);
        let rows = (self.screen.height as i32 * ONE - self.live_top) / self.body.line_height;
        self.clear(self.live_top, rows.max(0) as usize);
        let mut top = self.live_top;
        let message = format!("ClaudeOS stopped: {what}.");
        let note = format!("Details: {details}");
        for (i, paragraph) in [message.as_str(), note.as_str(), BACK_TO_WINDOWS].iter().enumerate() {
            if i > 0 {
                top += gap;
            }
            for line in self.ty.wrap(paragraph, self.body.size, column) {
                self.put(line, self.body, top);
                top += self.body.line_height;
            }
        }
    }
}

/// Called after every interrupt: updates the "Up for" line when a second has passed.
pub fn tick() {
    if let Some(w) = WELCOME.lock().as_mut() {
        w.tick();
    }
}

/// Says which keyboards there are: USB ones, and `other` (found another way), by name.
pub fn keyboards(status: &usb::Status, other: Option<&str>) {
    if let Some(w) = WELCOME.lock().as_mut() {
        w.show_keyboards(status, other);
    }
}

/// A key was typed.
pub fn typed(p: Press) {
    if let Some(w) = WELCOME.lock().as_mut() {
        w.edit(p);
    }
}

/// The last words on the screen after a crash. Drawing needs the heap, so if the crash happened while
/// the heap was busy, only the log gets them.
pub fn show_stop(what: &str, details: fmt::Arguments) {
    if heap::busy() {
        return;
    }
    let details = format!("{details}");
    if let Some(mut guard) = WELCOME.try_lock()
        && let Some(w) = guard.as_mut()
    {
        w.stop(what, &details);
    }
}
