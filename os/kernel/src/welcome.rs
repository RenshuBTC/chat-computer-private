//! The first screen ClaudeOS's own kernel shows: that it is running, what it found, and how long it
//! has been up (counted by its own timer interrupts). Laid out like ClaudeOS's welcome screen: a
//! larger first line, then text, in one column in the middle, black on white.

use crate::layout::{self, Layout};
use crate::screen::Screen;
use crate::sync::SpinLock;
use crate::text::{Style, Type};
use crate::{memory, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use font::raster::ONE;

pub struct Welcome {
    screen: Screen,
    ty: Type,
    layout: Layout,
    body: Style,
    column_x: usize,
    /// Where the "Up for ..." line goes: its top, in 1/256 pixel.
    uptime_top: i32,
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
    pub fn show(screen: Screen, size_mm: Option<(u32, u32)>, cpu: &str) -> layout::Layout {
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
        let facts = format!("This PC has {article} {cpu}, {memory} of memory and a {size} screen.");

        let title = ty.wrap("ClaudeOS is running on its own kernel.", greeting.size, column * ONE);
        let lines: Vec<&str> = ty.wrap(&facts, body.size, column * ONE);
        // Heights, as on ClaudeOS's welcome screen: greeting (1.5 rem, line height 1.3) with 0.25 em
        // below it, the text (line height 1.5), a paragraph gap of 1 em, then the "Up for" line.
        let title_h = greeting.line_height * title.len() as i32;
        let gap_small = fx(rem * 1.5 * 0.25);
        let gap = fx(rem);
        let total = title_h + gap_small + body.line_height * lines.len() as i32 + gap + body.line_height;
        let mut top = ((screen.height as i32 * ONE) - total) / 2;

        let mut w = Welcome { screen, ty, layout, body, column_x, uptime_top: 0, shown: None };
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
        w.uptime_top = top;
        w.tick();
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

    fn tick(&mut self) {
        let seconds = time::uptime_ms() / 1000;
        if self.shown == Some(seconds) {
            return;
        }
        self.shown = Some(seconds);
        let row = self.uptime_top.div_euclid(ONE) as usize;
        let height = (self.body.line_height / ONE + 2) as usize;
        let width = self.layout.column_px as usize + 8;
        self.screen.fill(self.column_x.saturating_sub(4), row, width, height, 255);
        let text = uptime_text(seconds);
        self.put(&text, self.body, self.uptime_top);
    }
}

/// Called after every interrupt: updates the "Up for" line when a second has passed.
pub fn tick() {
    if let Some(w) = WELCOME.lock().as_mut() {
        w.tick();
    }
}

/// The last words on the screen after a crash, in place of the "Up for" line.
pub fn show_stop(what: &str) {
    if let Some(mut guard) = WELCOME.try_lock() {
        if let Some(w) = guard.as_mut() {
            let row = w.uptime_top.div_euclid(ONE) as usize;
            let height = (w.body.line_height / ONE + 2) as usize;
            let width = w.layout.column_px as usize + 8;
            w.screen.fill(w.column_x.saturating_sub(4), row, width, height, 255);
            let text = format!("ClaudeOS stopped: {what}.");
            let (style, top) = (w.body, w.uptime_top);
            w.put(&text, style, top);
        }
    }
}
