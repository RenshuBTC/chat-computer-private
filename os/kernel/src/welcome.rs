//! The first screen ClaudeOS's own kernel shows: that it is running, what it found, how long it has
//! been up (counted by its own timer interrupts), and how to get back to Windows. Laid out like
//! ClaudeOS's welcome screen: a larger first line, then text, in one column in the middle, black on
//! white. If the kernel ever stops, the same place says so, with details to pass on.

use crate::layout::{self, Layout};
use crate::screen::Screen;
use crate::sync::SpinLock;
use crate::text::{Style, Type};
use crate::{heap, memory, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::{SECURE_BOOT_OFF, SECURE_BOOT_ON};
use core::fmt;
use font::raster::ONE;

const BACK_TO_WINDOWS: &str = "To go back to Windows, hold the power button until the PC turns off, then turn it on \
                               again. ClaudeOS doesn't write to your disks, so this is safe.";

pub struct Welcome {
    screen: Screen,
    ty: Type,
    layout: Layout,
    body: Style,
    column_x: usize,
    /// Where the live part starts (the "Up for" line, then how to go back), in 1/256 pixel.
    live_top: i32,
    shown: Option<u64>,
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
            + body.line_height * back.len() as i32;
        let mut top = ((screen.height as i32 * ONE) - total) / 2;

        let mut w = Welcome { screen, ty, layout, body, column_x, live_top: 0, shown: None };
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
