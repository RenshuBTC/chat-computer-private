//! What the screen shows: the conversation with Chat Computer's model, laid out as Chat Computer's own screen is
//! (chat.css, theme.css): its words on the left in one column, light on the dark background (Claude's dark mode's);
//! yours on the right in a bubble; the message box at the bottom, a card lifted off the middle with a thin line
//! round it and a soft shadow, "Type anything" in it while it's empty, and its send button, grey until there's something to send. Sizes
//! are the Claude app's (layout.rs): 16px words on 22px lines at 100%, the model's up to 865px across, yours in bubbles
//! up to 85% of the 924px conversation, the box 962px wide and 46px high for a line, 39px above the screen's bottom. What you send goes to the
//! top of the screen, with the answer coming in under it (and the screen following the answer down); Page Up and
//! Page Down, the arrow keys, or a mouse's wheel scroll. Esc clears the box, or stops an answer.
//!
//! Before the first message, the column says what this PC is, how long Chat Computer has been running on it
//! (counted by the kernel's own timer), which keyboards it found, how its model is getting ready, and how to get
//! back to Windows. If the kernel ever stops, the same place says so, with details to pass on.

use crate::assistant::{self, Event, Ready};
use crate::draw::{Canvas, Rect, BOX_GROUND, BOX_INK, BOX_LINE, BUBBLE, DIM, GROUND, INK, LINE, MUTED, PAPER};
use crate::keyboard::{Key, Press};
use crate::layout::{self, Layout};
use crate::screen::Screen;
use crate::sync::SpinLock;
use text::{Face, Ink, Style, Type};
use crate::{files, heap, memory, rich, time, usb};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::{SECURE_BOOT_OFF, SECURE_BOOT_ON};
use core::fmt;
use core::ops::Range;
use font::raster::ONE;

const BACK_TO_WINDOWS: &str = "To go back to Windows, ask Chat Computer to restart the PC, or turn it off with its power button \
                               (holding it down if a press doesn't) and on again. Chat Computer writes nothing on your \
                               disks but its own memory and log files, so either is safe.";
/// The same, once Chat Computer has stopped (nothing can be asked).
const BACK_TO_WINDOWS_STOPPED: &str = "To go back to Windows, hold the power button until the PC turns off, then turn \
                                       it on again. Chat Computer writes nothing on your disks but its own memory \
                                       and log files, so this is safe.";

/// The longest message that can be typed.
const TYPED_MAX: usize = 4000;
/// The most lines the message box grows to; past that it scrolls.
const BOX_LINES: usize = 6;
/// The soft shadow under the message box: CSS 0 0.25rem 1.25rem, black at 35%.
const SHADOW: u32 = 89;
/// Big redraws (more than a quarter of the screen) at most this often, so a fast answer doesn't take all the time.
const BIG_REDRAW_MS: u64 = 40;
/// Under a conversation brought back after a restart.
const RESTARTED: &str = "Chat Computer started again; the conversation above was kept.";
/// After the model starts a new conversation.
const NEW_CONVERSATION: &str = "A new conversation starts here: Chat Computer no longer sees what's above, and it won't be kept.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The start screen's first line, larger.
    Title,
    /// The start screen's other paragraphs.
    Text,
    /// What you sent.
    You,
    /// The model's answer.
    Answer,
    /// Chat Computer itself saying something: what went wrong, mostly.
    Note,
    /// A line across, then words: where the conversation was picked up after a restart, or a new one starts.
    Divider,
    /// A picture the model showed (show_picture).
    Picture,
}

/// The most pictures the conversation keeps the pixels of (older ones become a line saying one was there).
const MOST_PICTURES: usize = 12;
/// What's left of a picture whose pixels went.
const PICTURE_GONE: &str = "(A picture was shown here.)";

/// What a start-screen paragraph is, for the ones that change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Live {
    No,
    Uptime,
    Keyboards,
    Model,
}

/// A line of a block, or (in the model's answers) one run of words in one face on a line.
struct Line {
    text: String,
    face: Face,
    style: Style,
    /// From the column's left edge, and from the block's top, in pixels.
    x: i32,
    y: i32,
    ink: Option<Ink>,
}

struct Block {
    kind: Kind,
    live: Live,
    text: String,
    lines: Vec<Line>,
    /// Lines and frames in the model's answers (rules, code blocks' frames, quotes' bars, tables' lines, underlines),
    /// from the column's left edge and the block's top.
    marks: Vec<Rect>,
    /// Where it is in the whole conversation (from its top), and how tall, in pixels.
    top: i32,
    height: i32,
    /// Your message's bubble, from the column's left edge and the block's top.
    bubble: Rect,
    /// The model's answer with no words yet: the pulsing dot.
    thinking: bool,
    /// What the model is doing before it answers ("Reading a file"), shown by the dot.
    activity: Option<String>,
    /// Its text changed since it was laid out.
    changed: bool,
    /// A picture's pixels.
    picture: Option<picture::Picture>,
    /// In the grey of the words that matter less (what the model looked up, as the main Chat Computer shows what it
    /// does).
    quiet: bool,
}

impl Block {
    fn new(kind: Kind, live: Live, text: String) -> Block {
        Block {
            kind,
            live,
            text,
            lines: Vec::new(),
            marks: Vec::new(),
            top: 0,
            height: 0,
            bubble: Rect::default(),
            thinking: false,
            activity: None,
            changed: true,
            picture: None,
            quiet: false,
        }
    }
}

/// Sizes, in pixels (the text's own sizes in 1/256 pixel), from the text size this screen gets (layout.rs).
#[derive(Clone, Copy)]
struct Metrics {
    rem: f64,
    body: Style,
    title: Style,
    line: i32,
    column_x: i32,
    column_w: i32,
    /// How wide the model's words (and the start screen's) go, from the column's left edge.
    text_w: i32,
    /// The message box: its left edge and width.
    box_x: i32,
    box_w: i32,
}

impl Metrics {
    fn px(&self, rem: f64) -> i32 {
        (rem * self.rem + 0.5) as i32
    }
}

/// The message box, as laid out for what's typed.
#[derive(Clone, Default)]
struct BoxLayout {
    /// The box's card, on the screen.
    card: Rect,
    /// Where the typed lines start.
    text_x: i32,
    text_y: i32,
    lines: Vec<Range<usize>>,
    /// The first line shown (when there are more than fit), and how many are shown.
    first: usize,
    shown: usize,
    /// The send button's centre and radius, in 1/8 pixel.
    send: (i32, i32, i32),
}

pub struct Chat {
    screen: Screen,
    canvas: Canvas,
    ty: Type,
    layout: Layout,
    m: Metrics,
    blocks: Vec<Block>,
    /// Whether the conversation has begun (the start screen is gone).
    started: bool,
    /// The kernel stopped: the screen says so, and there's no message box.
    stopped: bool,
    /// The monitor's size, and the text size asked for (percent of the size chosen for this screen).
    size_mm: Option<(u32, u32)>,
    percent: u32,
    /// The block the model's answer is going into.
    asking: Option<usize>,
    typed: String,
    /// Where in `typed` the caret is (a byte index on a character boundary).
    cursor: usize,
    composer: BoxLayout,
    /// Where the latest thing you sent starts (the screen keeps it in view), in pixels from the conversation's top.
    anchor: i32,
    /// Where the person scrolled to (the conversation's top line on the screen's top), or None: following.
    scroll: Option<i32>,
    /// The part of the screen to draw again.
    dirty: Rect,
    /// Blocks need laying out again.
    relayout: bool,
    last_big_redraw: u64,
    uptime_shown: Option<u64>,
    model_checked: u64,
    dot_alpha: u32,
}

static CHAT: SpinLock<Option<Chat>> = SpinLock::new(None);

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

/// What the start screen says about the model.
fn model_text(ready: Ready) -> String {
    match ready {
        Ready::Yes(name) if crate::assistant::remote() => format!(
            "Meta's {name} is ready: it answers over the internet, through Meta's Model API, so what you type goes to \
             Meta. Type anything below and press Enter."
        ),
        Ready::Yes(name) => format!(
            "Chat Computer's own model, {name}, is ready, here on this PC: nothing you type leaves it, unless it looks \
             something up on the web for you, which you'll see. Type anything below and press Enter."
        ),
        Ready::Getting(how) => how,
        Ready::No(why) => why,
    }
}

/// How opaque the thinking dot is `ms` into its pulse (as chat.css: 0.35 to 1 and back in 1.2 s, easing), of 255.
fn pulse(ms: u64) -> u32 {
    let p = (ms % 1200) as i64; // 0..1200
    let tri = 1000 - (2 * p * 1000 / 1200 - 1000).abs(); // 0..1000..0
    let smooth = tri * tri * (3000 - 2 * tri) / 1_000_000; // smoothstep, 0..1000
    (255 * (350 + 650 * smooth / 1000) / 1000) as u32
}

impl Chat {
    fn metrics(layout: &Layout, width: usize) -> Metrics {
        let rem = layout.font_px;
        let line = (rem * layout::LINE_HEIGHT + 0.5) as i32;
        let title_line = (rem * 1.5 * 1.3 + 0.5) as i32;
        let column_w = layout.column_px as i32;
        let box_w = ((layout::BOX_REM * rem) as i32).min(width as i32 - (2.0 * rem) as i32).max(column_w.min(width as i32));
        Metrics {
            rem,
            body: Style { size: fx(rem), line_height: line * ONE },
            title: Style { size: fx(rem * 1.5), line_height: title_line * ONE },
            line,
            column_x: (width as i32 - column_w) / 2,
            column_w,
            text_w: ((layout::TEXT_REM * rem + 0.5) as i32).min(column_w),
            box_x: (width as i32 - box_w) / 2,
            box_w,
        }
    }

    /// The start screen.
    pub fn show(screen: Screen, size_mm: Option<(u32, u32)>, cpu: &str, secure_boot: u32) -> Layout {
        let layout = layout::compute(screen.width as u32, screen.height as u32, size_mm);
        let m = Self::metrics(&layout, screen.width);
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
        let blocks = alloc::vec![
            Block::new(Kind::Title, Live::No, String::from("Chat Computer is running on its own kernel.")),
            Block::new(Kind::Text, Live::No, facts),
            Block::new(Kind::Text, Live::Uptime, uptime_text(0)),
            Block::new(Kind::Text, Live::Keyboards, String::from("Looking for keyboards.")),
            Block::new(Kind::Text, Live::Model, String::from("Looking for Chat Computer's model\u{2026}")),
            Block::new(Kind::Text, Live::No, String::from(BACK_TO_WINDOWS)),
        ];
        let canvas = Canvas::new(screen.width, screen.height);
        let mut c = Chat {
            screen,
            canvas,
            ty: Type::builtin(),
            layout,
            m,
            blocks,
            started: false,
            stopped: false,
            size_mm,
            percent: 100,
            asking: None,
            // (Room for the longest message from the start, so what's typed is never copied elsewhere as it grows: a
            // BitLocker recovery key typed is overwritten where it is.)
            typed: String::with_capacity(TYPED_MAX * 4),
            cursor: 0,
            composer: BoxLayout::default(),
            anchor: 0,
            scroll: None,
            dirty: Rect::default(),
            relayout: true,
            last_big_redraw: 0,
            uptime_shown: Some(0),
            model_checked: 0,
            dot_alpha: 0,
        };
        c.lay_out_box();
        let card = c.composer.card;
        crate::log!("Chat Computer kernel: message box at rows {} to {}", card.y, card.bottom());
        c.draw(true);
        *CHAT.lock() = Some(c);
        layout
    }

    // ------------------------------------------------------------------------------------------- laying out

    fn style(&self, kind: Kind) -> Style {
        if kind == Kind::Title { self.m.title } else { self.m.body }
    }

    /// One of the theme's pixels (its CSS px) on this screen, at least one: the width of its thin lines.
    fn hair(&self) -> i32 {
        ((self.layout.scale + 0.5) as i32).max(1)
    }

    /// The message box for what's typed now. Changes `dirty` if its size changed.
    fn lay_out_box(&mut self) {
        let m = self.m;
        let border = self.hair();
        let (pad_top, pad_left, pad_right, gap) = (m.px(0.6875), m.px(0.9375), m.px(0.75), m.px(0.5));
        let send_d = m.line;
        let text_w = m.box_w - 2 * border - pad_left - pad_right - gap - send_d;
        let lines = self.ty.breaks(&self.typed, m.body.size, text_w * ONE);
        let shown = lines.len().clamp(1, BOX_LINES);
        let caret_line = self.caret_line(&lines);
        let first = (caret_line + 1).saturating_sub(shown).min(lines.len().saturating_sub(shown));
        let h = 2 * border + 2 * pad_top + shown as i32 * m.line;
        let y = self.screen.height as i32 - m.px(2.4375) - h;
        let card = Rect::new(m.box_x, y, m.box_w, h);
        let row_bottom = y + border + pad_top + shown as i32 * m.line;
        let send = (
            (card.right() - border - pad_right) * 8 - send_d * 4,
            row_bottom * 8 - send_d * 4,
            send_d * 4,
        );
        if card != self.composer.card && self.composer.card != Rect::default() {
            self.relayout = true; // the conversation's room changed
        }
        self.composer = BoxLayout { card, text_x: m.box_x + border + pad_left, text_y: y + border + pad_top, lines, first, shown, send };
    }

    /// Which line of the box the caret is on.
    fn caret_line(&self, lines: &[Range<usize>]) -> usize {
        lines.iter().rposition(|l| l.start <= self.cursor).unwrap_or(0)
    }

    /// The bottom of the part of the screen the conversation shows in (the message box and its margin below).
    fn log_bottom(&self) -> i32 {
        self.composer.card.y - self.m.px(0.5)
    }

    /// How far down the conversation's last words may come when following it.
    fn view_bottom(&self) -> i32 {
        self.log_bottom() - self.m.px(1.25)
    }

    /// Lays a block out. `hair`: one of the theme's pixels on this screen (its thin lines).
    fn lay_out_block(ty: &Type, m: &Metrics, style: Style, hair: i32, b: &mut Block) {
        let shown = b.text.clone();
        let mut old = core::mem::take(&mut b.lines);
        let mut lines = Vec::new();
        // (A line drawn before, the same words in the same face and size, keeps its ink.)
        let mut reuse = |text: &str, face: Face, style: Style, x: i32, y: i32, i: usize| -> Line {
            let ink = match old.get_mut(i) {
                Some(l) if l.text == text && l.face == face && l.style == style => l.ink.take(),
                _ => None,
            };
            Line { text: String::from(text), face, style, x, y, ink }
        };
        b.thinking = b.kind == Kind::Answer && shown.trim().is_empty();
        b.marks.clear();
        let line = style.line_height / ONE;
        if let Some(p) = b.picture.as_mut() {
            // (Narrower than it was shown at, after the text was made bigger: shrunk to the column.)
            if p.width as i32 > m.column_w {
                *p = picture::shrink(p, m.column_w.max(1) as u32, p.height);
            }
            b.height = p.height as i32;
        } else if b.kind == Kind::Answer && !b.thinking {
            // The model's words, as Markdown (rich.rs).
            let laid = rich::lay_out(ty, &shown, m.rem, style, m.text_w, hair);
            for (i, run) in laid.runs.into_iter().enumerate() {
                lines.push(reuse(&run.text, run.face, run.style, run.x, run.y, i));
            }
            b.marks = laid.marks;
            b.height = laid.height.max(line);
        } else if b.kind == Kind::You {
            let (pad_x, pad_y) = (m.px(1.0), m.px(0.75));
            let inner = m.column_w * 85 / 100 - 2 * pad_x;
            let ranges = ty.breaks(&shown, style.size, inner * ONE);
            let widest = ranges.iter().map(|r| (ty.width(&shown[r.clone()], style.size) + ONE - 1) / ONE).max().unwrap_or(0);
            let w = widest + 2 * pad_x;
            let x = m.column_w - w;
            for (i, r) in ranges.iter().enumerate() {
                lines.push(reuse(&shown[r.clone()], Face::Regular, style, x + pad_x, pad_y + i as i32 * line, i));
            }
            b.height = ranges.len() as i32 * line + 2 * pad_y;
            b.bubble = Rect::new(x, 0, w, b.height);
        } else if b.thinking {
            b.height = line;
        } else {
            let ranges = ty.breaks(&shown, style.size, m.text_w * ONE);
            // (A divider's line, then 1 em, then its words.)
            let mut y = if b.kind == Kind::Divider { m.px(1.0) } else { 0 };
            for r in &ranges {
                let text = &shown[r.clone()];
                if text.trim().is_empty() {
                    // A blank line between paragraphs: their usual space (1 em), not a whole line.
                    y += m.px(1.0);
                    continue;
                }
                let i = lines.len();
                lines.push(reuse(text, Face::Regular, style, 0, y, i));
                y += line;
            }
            b.height = y.max(line);
        }
        b.lines = lines;
        b.changed = false;
    }

    /// Lays the blocks out: the start screen in the middle of the room above the message box; the conversation
    /// from the top.
    fn lay_out(&mut self) {
        let m = self.m;
        let hair = self.hair();
        for i in 0..self.blocks.len() {
            if self.blocks[i].changed {
                let style = self.style(self.blocks[i].kind);
                Self::lay_out_block(&self.ty, &m, style, hair, &mut self.blocks[i]);
            }
        }
        if self.started {
            let mut top = m.px(2.0);
            for b in &mut self.blocks {
                b.top = top;
                top += b.height + m.px(1.25);
            }
        } else {
            // As Chat Computer's welcome screen: the first line, a quarter of its size below it, then paragraphs
            // 1 em apart.
            let gaps: Vec<i32> = self
                .blocks
                .iter()
                .enumerate()
                .map(|(i, b)| match i {
                    0 => 0,
                    _ if self.blocks[i - 1].kind == Kind::Title => (m.rem * 1.5 * 0.25 + 0.5) as i32,
                    _ => m.px(1.0) + if b.kind == Kind::Title { m.px(0.5) } else { 0 },
                })
                .collect();
            let total: i32 = self.blocks.iter().map(|b| b.height).sum::<i32>() + gaps.iter().sum::<i32>();
            let room = if self.stopped { self.screen.height as i32 } else { self.log_bottom() };
            let mut top = ((room - total) / 2).max(m.px(1.0));
            for (b, gap) in self.blocks.iter_mut().zip(gaps) {
                top += gap;
                b.top = top;
                top += b.height;
            }
        }
        self.relayout = false;
    }

    /// Where the screen's top is in the conversation.
    fn view_top(&self) -> i32 {
        if !self.started {
            return 0;
        }
        let end = self.blocks.last().map_or(0, |b| b.top + b.height);
        let follow = (end - self.view_bottom()).max(self.anchor).max(0);
        match self.scroll {
            Some(t) => t.clamp(0, follow),
            None => follow,
        }
    }

    fn scroll_by(&mut self, delta: i32) {
        if !self.started {
            return;
        }
        let end = self.blocks.last().map_or(0, |b| b.top + b.height);
        let follow = (end - self.view_bottom()).max(self.anchor).max(0);
        let to = (self.view_top() + delta).max(0);
        self.scroll = if to >= follow { None } else { Some(to) };
        self.dirty = self.dirty.union(&self.canvas.whole());
    }

    // ---------------------------------------------------------------------------------------------- drawing

    /// Draws `clip` of the screen into the canvas: the background, the conversation (or start screen), and the
    /// message box.
    fn render(&mut self, clip: Rect) {
        let view_top = self.view_top();
        let log = Rect::new(0, 0, self.screen.width as i32, if self.stopped { self.screen.height as i32 } else { self.log_bottom() });
        let m = self.m;
        let rule = ((2.0 * self.layout.scale + 0.5) as i32).max(1); // (theme.css's 2px lines)
        let now = time::uptime_ms();
        let Chat { canvas, blocks, ty, .. } = self;
        canvas.set_clip(clip);
        canvas.fill(clip, GROUND);
        canvas.set_clip(clip.intersect(&log));
        let area = canvas.clip();
        let screen_h = canvas.height as i32;
        for b in blocks.iter_mut() {
            let y = b.top - view_top;
            if y + b.height < -screen_h || y > 2 * screen_h {
                // Far off the screen: its lines' ink can go (it's drawn again if it comes back).
                for l in &mut b.lines {
                    l.ink = None;
                }
                continue;
            }
            if y + b.height <= area.y || y >= area.bottom() {
                continue;
            }
            if b.kind == Kind::You {
                let r = Rect::new(m.column_x + b.bubble.x, y + b.bubble.y, b.bubble.w, b.bubble.h);
                canvas.round_rect(r, m.px(0.75), BUBBLE);
            }
            if b.kind == Kind::Divider {
                canvas.fill(Rect::new(m.column_x, y, m.column_w, rule), LINE);
            }
            if let Some(p) = &b.picture {
                canvas.picture(m.column_x, y, p.width as usize, p.height as usize, &p.rgb);
                canvas.round_corners(Rect::new(m.column_x, y, p.width as i32, p.height as i32), m.px(0.6), GROUND);
            }
            for r in &b.marks {
                canvas.fill(Rect::new(m.column_x + r.x, y + r.y, r.w, r.h), INK);
            }
            for l in &mut b.lines {
                let ly = y + l.y;
                if ly + l.style.line_height / ONE <= area.y || ly >= area.bottom() {
                    continue;
                }
                let ink = l.ink.get_or_insert_with(|| ty.ink_in(&l.text, l.face, l.style, 0));
                canvas.ink(m.column_x + l.x - ink.left as i32, ly, ink, if b.quiet { MUTED } else { INK });
            }
            if b.thinking {
                let d = m.px(0.5);
                let baseline = ty.baseline(m.body) / ONE;
                canvas.circle((m.column_x * 2 + d) * 4, (y + baseline) * 8 - d * 4, d * 4, INK, pulse(now));
                if let Some(doing) = &b.activity {
                    let ink = ty.ink(doing, m.body, 0);
                    canvas.ink(m.column_x + d + m.px(0.6) - ink.left as i32, y, &ink, MUTED);
                }
            }
        }
        if !self.stopped {
            self.canvas.set_clip(clip);
            self.render_box();
        }
    }

    /// The message box: its shadow, the card, what's typed (or the hint) and the caret, the send button.
    fn render_box(&mut self) {
        let m = self.m;
        let b = &self.composer;
        let canvas = &mut self.canvas;
        canvas.shadow(b.card, m.px(0.25), m.px(1.25), SHADOW);
        // (A thin line round it: the card in the line's colour, the box's own inside it.)
        let hair = ((self.layout.scale + 0.5) as i32).max(1);
        canvas.round_rect(b.card, m.px(1.0), BOX_LINE);
        canvas.round_rect(Rect::new(b.card.x + hair, b.card.y + hair, b.card.w - 2 * hair, b.card.h - 2 * hair), m.px(1.0) - hair, PAPER);
        let caret_w = ((2.0 * self.layout.scale + 0.5) as i32).max(1);
        if self.typed.is_empty() {
            let hint = match crate::listening::state() {
                Some(false) => "Listening\u{2026} say it, then pause (Esc stops)",
                Some(true) => "Hearing what you said\u{2026}",
                None if assistant::busy() => "Esc stops the answer",
                None if crate::listening::can() => "Type anything, or press Windows+H to talk",
                None => "Type anything",
            };
            let ink = self.ty.ink(hint, m.body, 0);
            canvas.ink(b.text_x - ink.left as i32, b.text_y, &ink, DIM);
        }
        for (i, r) in b.lines.iter().enumerate().skip(b.first).take(b.shown) {
            let text = &self.typed[r.clone()];
            if text.is_empty() {
                continue;
            }
            let ink = self.ty.ink(text, m.body, 0);
            canvas.ink(b.text_x - ink.left as i32, b.text_y + (i - b.first) as i32 * m.line, &ink, BOX_INK);
        }
        // The caret.
        let li = b.lines.iter().rposition(|l| l.start <= self.cursor).unwrap_or(0);
        if (b.first..b.first + b.shown).contains(&li) {
            let start = b.lines.get(li).map_or(0, |l| l.start);
            let line_end = b.lines.get(li).map_or(0, |l| l.end).max(start);
            let end = self.cursor.min(line_end).max(start);
            // (Where the next letter starts: kerned closer to the one before, the caret is too.)
            let x = b.text_x + self.ty.x_at(&self.typed[start..line_end], end - start, m.body.size) / ONE;
            let y = b.text_y + (li - b.first) as i32 * m.line;
            canvas.fill(Rect::new(x, y + m.line * 18 / 100, caret_w, m.line * 64 / 100), BOX_INK);
        }
        // The send button: a circle with an arrow up (chat.html's arrow), drawn at 62% of the button.
        let (cx, cy, r) = b.send;
        let ready = !self.typed.trim().is_empty() && !assistant::busy();
        canvas.circle(cx, cy, r, if ready { BOX_INK } else { DIM }, 255);
        let unit = r * 2 * 62 / 100; // the icon's size, in 1/8 pixel; its drawing is 24 units across
        let at = |u: i32, v: i32| (cx - unit / 2 + u * unit / 240, cy - unit / 2 + v * unit / 240);
        let segments = [(at(120, 190), at(120, 50)), (at(55, 115), at(120, 50)), (at(120, 50), at(185, 115))];
        canvas.strokes(&segments, unit * 26 / 240, BOX_GROUND);
    }

    /// Draws what changed (everything if `all`) and puts it on the screen.
    fn draw(&mut self, all: bool) {
        if self.relayout {
            self.lay_out();
            self.dirty = self.canvas.whole();
        }
        if all {
            self.dirty = self.canvas.whole();
        }
        let dirty = self.dirty.intersect(&self.canvas.whole());
        if dirty.is_empty() {
            return;
        }
        let now = time::uptime_ms();
        let big = dirty.w as i64 * dirty.h as i64 * 4 > self.canvas.width as i64 * self.canvas.height as i64;
        if big && !all && now.saturating_sub(self.last_big_redraw) < BIG_REDRAW_MS {
            return; // soon
        }
        if big {
            self.last_big_redraw = now;
        }
        self.dirty = Rect::default();
        self.render(dirty);
        let width = self.canvas.width;
        self.screen.copy(dirty.x as usize, dirty.y as usize, dirty.w as usize, dirty.h as usize, self.canvas.pixels(), width);
    }

    /// The part of the screen a block takes now.
    fn block_area(&self, i: usize) -> Rect {
        let b = &self.blocks[i];
        Rect::new(0, b.top - self.view_top(), self.screen.width as i32, b.height)
    }

    /// The message box's part of the screen, shadow and all.
    fn box_area(&self) -> Rect {
        let reach = self.m.px(2.2);
        let card = self.composer.card;
        Rect::new(card.x - reach, card.y - reach, card.w + 2 * reach, self.screen.height as i32 - (card.y - reach))
    }

    // ------------------------------------------------------------------------------------------- what happens

    fn tick(&mut self) {
        if self.stopped {
            return;
        }
        let now = time::uptime_ms();
        if !self.started {
            let seconds = now / 1000;
            if self.uptime_shown != Some(seconds) {
                self.uptime_shown = Some(seconds);
                self.set_live(Live::Uptime, uptime_text(seconds));
            }
            if now.saturating_sub(self.model_checked) >= 500 {
                self.model_checked = now;
                self.set_live(Live::Model, model_text(assistant::ready()));
            }
        }
        if let Some(i) = self.asking
            && self.blocks.get(i).is_some_and(|b| b.thinking)
        {
            let alpha = pulse(now);
            if alpha.abs_diff(self.dot_alpha) >= 8 {
                self.dot_alpha = alpha;
                let area = self.block_area(i);
                self.dirty = self.dirty.union(&Rect::new(area.x, area.y, area.w, self.m.line));
            }
        }
        self.draw(false);
    }

    /// A start-screen paragraph that changes: its new words.
    fn set_live(&mut self, live: Live, text: String) {
        let Some(i) = self.blocks.iter().position(|b| b.live == live) else { return };
        if self.blocks[i].text == text {
            return;
        }
        let before = self.blocks[i].lines.len();
        self.blocks[i].text = text;
        let (style, hair) = (self.style(self.blocks[i].kind), self.hair());
        Self::lay_out_block(&self.ty, &self.m, style, hair, &mut self.blocks[i]);
        if self.blocks[i].lines.len() == before {
            let area = self.block_area(i);
            self.dirty = self.dirty.union(&area);
        } else {
            self.relayout = true; // it took more or fewer lines: everything moves
        }
    }

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
        let text = match all.as_slice() {
            [] if status.controllers == 0 => String::from("No keyboard Chat Computer can use was found, so typing doesn't work yet."),
            [] if status.running == 0 => String::from("The USB controllers didn't start, so typing doesn't work yet."),
            [] => String::from("Plug in a USB keyboard to type."),
            [one] => format!("Type with the {one}."),
            _ => format!("Type with any keyboard: {}.", named.join(", ")),
        };
        self.set_live(Live::Keyboards, text);
    }

    fn edit(&mut self, p: Press) {
        let before = |s: &str, at: usize| s[..at].chars().next_back().map_or(at, |c| at - c.len_utf8());
        let after = |s: &str, at: usize| s[at..].chars().next().map_or(at, |c| at + c.len_utf8());
        let page = (self.view_bottom() * 8 / 10).max(self.m.line);
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
            Key::Enter => return self.send(),
            Key::Escape => {
                if !self.typed.is_empty() {
                    self.typed.clear();
                    self.cursor = 0;
                } else if self.asking.is_some() {
                    assistant::stop();
                    self.answer_over(Some(String::from("Stopped.")));
                    return;
                }
            }
            Key::PageUp => return self.scroll_by(-page),
            Key::PageDown => return self.scroll_by(page),
            Key::Up => return self.scroll_by(-3 * self.m.line),
            Key::Down => return self.scroll_by(3 * self.m.line),
            _ => return,
        }
        self.lay_out_box();
        let area = self.box_area();
        self.dirty = self.dirty.union(&area);
    }

    /// The talk key: listening starts (or, while it records, what was said so far is heard).
    fn talk(&mut self) {
        if let Err(why) = crate::listening::talk() {
            self.blocks.push(Block::new(Kind::Note, Live::No, why));
            self.relayout = true;
        }
        self.box_changed();
    }

    /// What came of listening: the words said are sent as if typed.
    fn heard(&mut self, h: crate::listening::Heard) {
        use crate::listening::Heard;
        match h {
            Heard::Listening | Heard::Hearing => self.box_changed(),
            Heard::Words(words) => {
                if self.asking.is_some() {
                    // (An answer is still coming: the words wait in the box, to be sent with Enter.)
                    self.typed = words;
                    self.cursor = self.typed.len();
                    self.lay_out_box();
                    self.box_changed();
                } else {
                    self.typed = words;
                    self.cursor = self.typed.len();
                    self.send();
                }
            }
            Heard::Nothing(why) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, why));
                self.relayout = true;
                self.box_changed();
            }
        }
    }

    /// The message box needs drawing again (its hint changed).
    fn box_changed(&mut self) {
        let area = self.box_area();
        self.dirty = self.dirty.union(&area);
    }

    /// Enter: what's typed goes to the model.
    fn send(&mut self) {
        // A test build of the keyboard (check.py --keyboard) writes the line in the log instead, so the test can
        // check what reached the screen; no other build ever logs what's typed.
        if option_env!("CHATCOMPUTER_TEST") == Some("keyboard") {
            crate::log!("Chat Computer kernel: test: typed {:?}", self.typed);
            self.typed.clear();
            self.cursor = 0;
            self.lay_out_box();
            let area = self.box_area();
            self.dirty = self.dirty.union(&area);
            return;
        }
        if self.typed.trim().is_empty() || self.asking.is_some() {
            return;
        }
        if !self.started {
            self.started = true;
            self.blocks.clear();
        }
        // A BitLocker recovery key unlocks Windows' drive here (files::work): it isn't shown, given to the model or
        // kept, nor is anything typed with it.
        if let Some(key) = bitlocker::find_recovery_key(&self.typed) {
            let alone = key.len() == self.typed.trim().len();
            files::unlock_later(String::from(key));
            files::wipe(&mut self.typed);
            self.cursor = 0;
            let note = if alone {
                "A BitLocker recovery key: Chat Computer unlocks Windows' drive with it here, without giving it to the \
                 model or keeping it. Unlocking\u{2026}"
            } else {
                "Your message has a BitLocker recovery key in it, so it wasn't given to the model: Chat Computer \
                 unlocks Windows' drive with the key here instead, and doesn't keep it. Unlocking\u{2026}"
            };
            self.blocks.push(Block::new(Kind::Note, Live::No, String::from(note)));
            self.lay_out_box();
            self.lay_out();
            self.anchor = self.blocks[self.blocks.len() - 1].top - self.m.px(2.0);
            self.scroll = None;
            // (On the screen before the unlocking starts, as the screen waits while it's done.)
            self.draw(true);
            return;
        }
        let text = String::from(self.typed.trim());
        self.typed.clear();
        self.cursor = 0;
        self.blocks.push(Block::new(Kind::You, Live::No, text.clone()));
        let you = self.blocks.len() - 1;
        match assistant::ask(&text) {
            Ok(()) => {
                self.blocks.push(Block::new(Kind::Answer, Live::No, String::new()));
                self.asking = Some(self.blocks.len() - 1);
            }
            Err(why) => self.blocks.push(Block::new(Kind::Note, Live::No, why)),
        }
        self.lay_out_box();
        self.lay_out();
        self.anchor = self.blocks[you].top - self.m.px(2.0);
        self.scroll = None;
        self.dirty = self.canvas.whole();
    }

    /// The answer is over: complete (None), or not (why).
    fn answer_over(&mut self, failed: Option<String>) {
        let Some(i) = self.asking.take() else { return };
        let empty = self.blocks[i].text.trim().is_empty();
        match failed {
            Some(why) if empty => self.blocks[i] = Block::new(Kind::Note, Live::No, why),
            Some(why) => self.blocks.push(Block::new(Kind::Note, Live::No, why)),
            None if empty => {
                self.blocks.remove(i);
            }
            None => {
                // The conversation stays as it is: only the message box changes (its hint and send button).
                self.lay_out_box();
                let area = self.box_area();
                self.dirty = self.dirty.union(&area);
                return;
            }
        }
        self.relayout = true;
        self.lay_out_box(); // (the send button and the hint change)
    }

    /// The text `percent` of the size chosen for this screen: everything laid out again.
    fn set_text_size(&mut self, percent: u32) {
        self.percent = percent.clamp(50, 300);
        let (w, h) = (self.screen.width as u32, self.screen.height as u32);
        let base = layout::compute(w, h, self.size_mm);
        self.layout = layout::scaled(base, w, self.percent as f64 / 100.0);
        self.m = Self::metrics(&self.layout, self.screen.width);
        for b in &mut self.blocks {
            b.lines.clear();
            b.changed = true;
        }
        self.composer = BoxLayout::default();
        self.lay_out_box();
        self.relayout = true;
        crate::log!("Chat Computer kernel: text size {}%: {:.1} px", self.percent, self.layout.font_px);
    }

    fn answer(&mut self, e: Event) {
        match e {
            Event::Text(s) => {
                let Some(i) = self.asking else { return };
                let top_before = self.view_top();
                let old = self.block_area(i);
                self.blocks[i].text.push_str(&s);
                self.blocks[i].changed = true;
                self.lay_out();
                let new = self.block_area(i);
                if self.view_top() != top_before {
                    let log = Rect::new(0, 0, self.screen.width as i32, self.log_bottom());
                    self.dirty = self.dirty.union(&log).union(&self.box_area());
                } else {
                    self.dirty = self.dirty.union(&old.union(&new)).union(&self.box_area());
                }
            }
            Event::Done => self.answer_over(None),
            Event::Failed(why) => self.answer_over(Some(why)),
            Event::TextSize(percent) => self.set_text_size(percent),
            Event::Activity(doing) => {
                let Some(i) = self.asking else { return };
                self.blocks[i].activity = Some(doing);
                let area = self.block_area(i);
                self.dirty = self.dirty.union(&area);
            }
            Event::Picture(p) => self.show_picture(p),
            Event::Restarting(seconds) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, format!("Restarting in {seconds} seconds\u{2026}")));
                self.relayout = true;
            }
            Event::TurningOff(seconds) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, format!("Turning off in {seconds} seconds\u{2026}")));
                self.relayout = true;
            }
            Event::Noted(note) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, format!("Chat Computer will remember: {note}")));
                self.relayout = true;
            }
            Event::Forgot(n) => {
                let what = if n == 1 { String::from("a note") } else { format!("{n} notes") };
                self.blocks.push(Block::new(Kind::Note, Live::No, format!("Chat Computer forgot {what}.")));
                self.relayout = true;
            }
            Event::NewConversation => {
                self.blocks.push(Block::new(Kind::Divider, Live::No, String::from(NEW_CONVERSATION)));
                self.relayout = true;
            }
            Event::Note(text) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, text));
                self.relayout = true;
            }
            Event::Looked(text) => self.looked(text),
        }
    }

    /// A line saying what the model looked up on the web, where it happens in the answer: before its words if it has
    /// none yet, else after them (the answer going on below it), as a picture is placed.
    fn looked(&mut self, text: String) {
        let mut block = Block::new(Kind::Note, Live::No, text);
        block.quiet = true;
        match self.asking {
            Some(i) if self.blocks[i].text.trim().is_empty() => {
                self.blocks.insert(i, block);
                self.asking = Some(i + 1);
            }
            Some(i) => {
                self.blocks.insert(i + 1, block);
                self.blocks.insert(i + 2, Block::new(Kind::Answer, Live::No, String::new()));
                self.asking = Some(i + 2);
            }
            None => self.blocks.push(block),
        }
        self.relayout = true;
    }

    /// A picture the model shows: where the answer is, before its words if it hasn't any yet, else after them (and the
    /// answer goes on below it). Only the last `MOST_PICTURES` keep their pixels.
    fn show_picture(&mut self, p: picture::Picture) {
        let Some(i) = self.asking else { return };
        let mut block = Block::new(Kind::Picture, Live::No, String::new());
        block.picture = Some(p);
        if self.blocks[i].text.trim().is_empty() {
            self.blocks.insert(i, block);
            self.asking = Some(i + 1);
        } else {
            self.blocks.insert(i + 1, block);
            self.blocks.insert(i + 2, Block::new(Kind::Answer, Live::No, String::new()));
            self.asking = Some(i + 2);
        }
        let mut kept = 0;
        for b in self.blocks.iter_mut().rev() {
            if b.picture.is_some() {
                kept += 1;
                if kept > MOST_PICTURES {
                    *b = Block::new(Kind::Note, Live::No, String::from(PICTURE_GONE));
                }
            }
        }
        self.relayout = true;
    }

    /// The most room a picture gets: the column's width, and two thirds of the conversation's height.
    fn picture_box(&self) -> (u32, u32) {
        let room = (self.log_bottom() - self.m.px(4.0)).max(1);
        (self.m.column_w.max(1) as u32, (room * 2 / 3).max(1) as u32)
    }

    /// The conversation from before the last restart, in place of the start screen, scrolled to its end.
    fn restore(&mut self, turns: &[(bool, String)]) {
        if turns.is_empty() || self.stopped {
            return;
        }
        self.started = true;
        self.blocks.clear();
        for (you, text) in turns {
            self.blocks.push(Block::new(if *you { Kind::You } else { Kind::Answer }, Live::No, text.clone()));
        }
        self.blocks.push(Block::new(Kind::Divider, Live::No, String::from(RESTARTED)));
        self.lay_out_box();
        self.lay_out();
        self.anchor = 0;
        self.scroll = None;
        self.dirty = self.canvas.whole();
    }

    /// Replaces everything with what stopped the kernel, the details, and how to go back.
    fn stop(&mut self, what: &str, details: &str) {
        self.stopped = true;
        self.started = false;
        self.blocks = alloc::vec![
            Block::new(Kind::Title, Live::No, format!("Chat Computer stopped: {what}.")),
            Block::new(Kind::Text, Live::No, format!("Details: {details}")),
            Block::new(Kind::Text, Live::No, String::from(BACK_TO_WINDOWS_STOPPED)),
        ];
        self.relayout = true;
        self.draw(true);
    }
}

/// The most room a picture gets on the screen (show_picture).
pub fn picture_box() -> (u32, u32) {
    CHAT.lock().as_ref().map_or((800, 600), |c| c.picture_box())
}

/// Before one of the start's steps (the main loop, which draws, isn't going yet): which, on the start screen, in the
/// model's line until the model's own news replaces it; drawn at once, so a PC that stops in the middle of a step
/// shows which.
pub fn starting(what: &str) {
    if let Some(c) = CHAT.lock().as_mut()
        && !c.stopped
        && !c.started
    {
        let seconds = time::uptime_ms() / 1000;
        c.uptime_shown = Some(seconds);
        c.set_live(Live::Uptime, uptime_text(seconds));
        c.set_live(Live::Model, format!("Starting {what}\u{2026}"));
        c.draw(false);
    }
}

/// Called after every interrupt: the start screen's live lines, the thinking dot, and whatever changed.
pub fn tick() {
    if let Some(c) = CHAT.lock().as_mut() {
        c.tick();
    }
}

/// Says which keyboards there are: USB ones, and `other` (found another way), by name.
pub fn keyboards(status: &usb::Status, other: Option<&str>) {
    if let Some(c) = CHAT.lock().as_mut() {
        c.show_keyboards(status, other);
    }
}

/// A mouse's wheel turned this many notches (positive away from you): the conversation scrolls three lines a notch,
/// back for a turn away from you, as Windows' default is.
pub fn wheel(notches: i32) {
    // A test build of the keyboard (check.py --keyboard) says so in the log, so the test can check the turns arrive.
    if option_env!("CHATCOMPUTER_TEST") == Some("keyboard") {
        crate::log!("Chat Computer kernel: test: wheel {notches}");
    }
    if let Some(c) = CHAT.lock().as_mut() {
        let lines = 3 * c.m.line;
        c.scroll_by(notches.saturating_mul(lines).saturating_neg());
    }
}

/// A key was typed.
pub fn typed(p: Press) {
    if let Some(c) = CHAT.lock().as_mut() {
        if talk_key(p) {
            c.talk();
        } else if matches!(p.key, Key::Escape) && crate::listening::state().is_some() {
            crate::listening::cancel();
            c.box_changed();
        } else {
            c.edit(p);
        }
    }
}

/// The talk key: Windows+H, as Windows' own voice typing has it, or Ctrl+Space.
fn talk_key(p: Press) -> bool {
    match p.key {
        Key::Char('h' | 'H') => p.logo && !p.ctrl && !p.alt,
        Key::Char(' ') => p.ctrl && !p.alt && !p.logo,
        _ => false,
    }
}

/// What came of listening.
pub fn heard(h: crate::listening::Heard) {
    if let Some(c) = CHAT.lock().as_mut()
        && !c.stopped
    {
        c.heard(h);
    }
}

/// The conversation from before the last restart, shown (nothing if there was none).
pub fn restore(turns: &[(bool, String)]) {
    if let Some(c) = CHAT.lock().as_mut() {
        c.restore(turns);
    }
}

/// A line from Chat Computer itself at the end of the conversation (a timer ringing).
pub fn note(text: String) {
    if let Some(c) = CHAT.lock().as_mut()
        && !c.stopped
    {
        c.blocks.push(Block::new(Kind::Note, Live::No, text));
        c.relayout = true;
    }
}

/// Something came of the question to the model.
pub fn answer(e: Event) {
    if let Some(c) = CHAT.lock().as_mut() {
        c.answer(e);
    }
}

/// The last words on the screen after a crash. Drawing needs the heap, so if the crash happened while the heap
/// was busy (or the screen was being drawn), only the log gets them.
pub fn show_stop(what: &str, details: fmt::Arguments) {
    if heap::busy() {
        return;
    }
    let details = format!("{details}");
    if let Some(mut guard) = CHAT.try_lock()
        && let Some(c) = guard.as_mut()
    {
        c.stop(what, &details);
    }
}
