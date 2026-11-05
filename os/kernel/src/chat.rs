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
//!
//! Where they fit (as chat.js has it: 17.5 rem each beside more than half the conversation's width), the app's two
//! sidebars, against the screen's edges, a shade lighter than the middle with a thin line between: Messages at the
//! left (the messages of the person's email and chat apps, once the own system can connect them); at the right,
//! Actions (the date and time, then what Chat Computer did for the person, each with its time: what it looked up
//! on the web, what it read or did on the PC) over Chats (this conversation's, by its first message). The
//! conversation and the message box are in the middle between them.

use crate::assistant::{self, Event, Ready};
use crate::draw::{Canvas, Point, Rect, BOX_CHOSEN, BOX_GROUND, BOX_INK, BOX_LINE, BUBBLE, CHOSEN, DIM, GROUND, INK, KNOB, LINE, MUTED, PAPER, SIDE};
use crate::keyboard::{Key, Press};
use crate::layout::{self, Layout};
use crate::screen::Screen;
use crate::sync::SpinLock;
use text::{Face, Ink, Style, Type};
use crate::site::{self, Site};
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
/// The sidebars' words while they have nothing to list.
const MESSAGES_EMPTY: &str = "Messages from your email and chat apps show up here, newest first, once Chat Computer's own \
                              system can connect them (in the app, + connects them).";
const ACTIONS_EMPTY: &str = "What I do for you shows up here: what I look up on the web, and what I read or change on this PC.";
const CHATS_EMPTY: &str = "This conversation shows up here, by its first message.";
/// The most actions the Actions sidebar keeps (the latest).
const ACTIONS_MAX: usize = 200;

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
    /// Each sidebar's width (0: they don't fit), and the middle between them.
    side_w: i32,
    mid_x: i32,
    mid_w: i32,
    /// The sidebars' words: 0.875 rem, on lines 1.4 times that.
    small: Style,
    small_line: i32,
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
    /// The round buttons under what's typed, each's centre and radius in 1/8 pixel: +, the spark, the microphone and
    /// send (TOOL_*).
    tools: [(i32, i32, i32); 4],
    /// The files attached, as chips over what's typed: each one's place (and its x's), on the screen.
    chips: Vec<(Rect, Rect)>,
}

/// The message box's buttons, in `BoxLayout::tools`.
const TOOL_ADD: usize = 0;
const TOOL_AI: usize = 1;
const TOOL_MIC: usize = 2;
const TOOL_SEND: usize = 3;

/// A list above the message box: the files + attaches, or the models the spark chooses among.
struct Menu {
    kind: MenuKind,
    title: String,
    items: Vec<MenuItem>,
    chosen: usize,
    /// Its first item shown (when there are more than fit).
    first: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    Files,
    Ai,
    /// Claude's models (after Claude is chosen), then the effort for the one picked.
    ClaudeModels,
    Efforts,
    /// "/" typed in the box: the files on this PC whose names have what's typed after it, the newest first.
    Search,
}

/// The "/" list's last item: Chat Computer asked to find it (in `MenuItem::path`).
const ASK_FIND: &str = "\u{0}ask-find";

/// The spark's list: Claude's item, and in Claude's list, giving another key and forgetting it (in `MenuItem::path`).
const CLAUDE_ITEM: &str = "\u{0}claude";
const NEW_KEY: &str = "\u{0}new-key";
const FORGET_KEY: &str = "\u{0}forget-key";

struct MenuItem {
    label: String,
    detail: String,
    /// A file's path (Files), or a model's (Ai).
    path: String,
    ticked: bool,
}

/// The most items a list above the box shows at once, and the most files + offers.
const MENU_ROWS: usize = 9;
const MENU_FILES: usize = 60;

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
    /// The Actions sidebar's list (what, and when: "2:32 pm"), the date and time it shows, and the Chats sidebar's
    /// conversation (by its first message).
    actions: Vec<(String, String)>,
    clock_shown: (String, String),
    chat_title: Option<String>,
    /// The apps the model made (their names), listed under Messages; and where each is in the list, for a click.
    apps: Vec<String>,
    app_rows: Vec<(Rect, String)>,
    /// Which of the message box's round buttons the pointer is over (it's lighter). (The pointer itself is drawn by
    /// pointer.rs, over the screen, as the mouse's reports come.)
    tool_lit: Option<usize>,
    /// The start screen's greeting (as the website's: by the time of day and the day), and when it began to fade
    /// (the first message sent).
    greeting: String,
    welcome_fading: Option<u64>,
    /// The list open above the message box, if one is; the files attached to the next message.
    menu: Option<Menu>,
    attached: Vec<(String, String)>,
    /// Your key for Claude being typed: the box shows a dot for each of its characters (`typed` holds the dots), and
    /// the key itself is here, until Enter gives it to the assistant (never shown, nor logged).
    key_typed: Option<String>,
    /// Claude's model picked, its effort still to choose.
    picked_model: Option<String>,
    /// Whether each sidebar shows (Messages at the left, Actions and Chats at the right): their switches, as the
    /// website's, put them away and bring them back; Chat Computer keeps the choice (read once it's started).
    sides: [bool; 2],
    sides_read: bool,
    /// Which switch the pointer is over, to light it up.
    switch_lit: Option<usize>,
    /// The files "/" searches (read from the drives once each time it's typed).
    search_files: Vec<files::Recent>,
    /// The main loop has started (it takes keys as they come: before, they're taken between the start's steps), and
    /// a message sent before then, to send once it has.
    looping: bool,
    send_later: bool,
    /// A website open in the middle, in place of the conversation (site.rs), until it's closed.
    site: Option<Site>,
    /// The page's place on the screen as last logged (a test build logs it, for the test to click in it).
    site_logged: Option<Rect>,
    /// Where an app's Allow button was last said to be (test builds).
    allow_logged: Option<Rect>,
    /// The send an app asks to make that the question over it is about (its number), and when it was first shown
    /// (uptime ms); where its Send button was last said to be (test builds).
    send_shown: Option<(u32, u64)>,
    send_logged: Option<(u32, Rect)>,
    /// The button over an app the keyboard has chosen (Tab), for Enter to press: drawn with a ring round it.
    pill: Option<Pill>,
}

/// The buttons over an app the keyboard can choose: its Allow button, and its question's Don't send and Send.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pill {
    Allow,
    DontSend,
    Send,
}

/// How long the start screen's word logo and greeting take to fade, as the website's (chat.css: #welcome).
const WELCOME_FADE_MS: u64 = 500;

/// The question over an app that asks to send something: its buttons press only once it's been shown this long (ms),
/// so a click meant for the page under it can't press one; and at most this many lines of what it sends.
const SEND_WAIT_MS: u64 = 600;
const SEND_LINES: usize = 10;

/// The question over an app that asks to send something (site::SendAsked), laid out: its box, its lines (each one's
/// words, face, top and colour), and its two buttons, Don't send and Send.
struct SendBox {
    id: u32,
    rect: Rect,
    lines: Vec<(String, Face, i32, u32)>,
    dont: Rect,
    send: Rect,
}

/// The website's greetings (chat.js: GREETINGS, without a name): what, from and until which hour, and on which days
/// (0 Sunday; empty: any).
const GREETINGS: &[(&str, u32, u32, &[u32])] = &[
    ("Good morning", 6, 12, &[]),
    ("Welcome", 6, 12, &[]),
    ("Hey there", 6, 12, &[]),
    ("Happy Monday", 6, 12, &[1]),
    ("Happy Tuesday", 6, 12, &[2]),
    ("Happy Wednesday", 6, 12, &[3]),
    ("Happy Thursday", 6, 12, &[4]),
    ("Happy Friday", 6, 12, &[5]),
    ("Hello to that Friday feeling", 6, 12, &[5]),
    ("Welcome to the weekend", 6, 12, &[6, 0]),
    ("Happy Saturday", 6, 12, &[6]),
    ("What's on your mind?", 6, 12, &[6, 0]),
    ("Sunday session?", 6, 12, &[0]),
    ("Happy Sunday", 6, 12, &[0]),
    ("Coffee and computer time?", 6, 12, &[]),
    ("Good afternoon", 12, 17, &[]),
    ("Hi, how are you?", 12, 17, &[]),
    ("What's new?", 12, 17, &[]),
    ("Back at it?", 12, 17, &[]),
    ("Good evening", 17, 21, &[]),
    ("How was your day?", 17, 21, &[]),
    ("How's it going?", 21, 6, &[]),
    ("What's on your mind tonight?", 21, 6, &[]),
    ("Hello, night owl", 21, 6, &[]),
    ("Greetings, whoever you are", 0, 24, &[]),
];

/// A greeting for now (by the PC's clock; the first in the list for the time of day while the clock's unknown),
/// a different one from start to start.
fn greeting_now() -> String {
    let Some(t) = crate::clock::local_now().or_else(crate::clock::now) else { return String::from("Hello") };
    let (_, _, _, h, _) = crate::clock::civil(t);
    // (1970-01-01 was a Thursday.)
    let day = (t.div_euclid(86_400) + 4).rem_euclid(7) as u32;
    let fits: Vec<&str> = GREETINGS
        .iter()
        .filter(|(_, from, until, days)| {
            let hour = if from < until { h >= *from && h < *until } else { h >= *from || h < *until };
            hour && (days.is_empty() || days.contains(&day))
        })
        .map(|(g, ..)| *g)
        .collect();
    let pick = (crate::time::now_us() ^ (t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)) as usize % fits.len().max(1);
    String::from(fits.get(pick).copied().unwrap_or("Hello"))
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
        Ready::Yes(name) => format!(
            "Chat Computer's own model, {name}, is ready, here on this PC: nothing you type leaves it, unless it looks \
             something up on the web for you, which you'll see. Type anything below and press Enter."
        ),
        Ready::Getting(how) => how,
        Ready::No(why) => why,
    }
}

/// The Actions sidebar's date and time, as the app shows them in Singapore's English: "Thursday 5 November 2026",
/// "2:32 pm"; in local time when the settings gave the time zone, else in UTC (said so). Empty while the clock is
/// unknown.
fn clock_now() -> (String, String) {
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    const MONTHS: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let (t, utc) = match crate::clock::local_now() {
        Some(t) => (t, false),
        None => match crate::clock::now() {
            Some(t) => (t, true),
            None => return (String::new(), String::new()),
        },
    };
    let (y, mo, d, h, mi) = crate::clock::civil(t);
    let day = DAYS[t.div_euclid(86_400).rem_euclid(7) as usize];
    let month = MONTHS[(mo as usize).clamp(1, 12) - 1];
    let (h12, pm) = (if h % 12 == 0 { 12 } else { h % 12 }, h >= 12);
    (format!("{day} {d} {month} {y}"), format!("{h12}:{mi:02} {}{}", if pm { "pm" } else { "am" }, if utc { " UTC" } else { "" }))
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
        let width = width as i32;
        // The sidebars, where there's room for both beside more than half the conversation (chat.js's fitPanel).
        let side = (17.5 * rem + 0.5) as i32;
        let fits = width as f64 - 38.0 * rem >= 0.55 * layout.column_px;
        let side_w = if fits { side } else { 0 };
        let (mid_x, mid_w) = (side_w, width - 2 * side_w);
        let column_w = (layout.column_px as i32).min(if fits { mid_w - (3.0 * rem) as i32 } else { width }).max(1);
        let box_w = ((layout::BOX_REM * rem) as i32).min(mid_w - (2.0 * rem) as i32).max(column_w.min(mid_w));
        let small_line = (rem * 0.875 * 1.4 + 0.5) as i32;
        Metrics {
            rem,
            body: Style { size: fx(rem), line_height: line * ONE },
            title: Style { size: fx(rem * 1.5), line_height: title_line * ONE },
            line,
            column_x: mid_x + (mid_w - column_w) / 2,
            column_w,
            text_w: ((layout::TEXT_REM * rem + 0.5) as i32).min(column_w),
            box_x: mid_x + (mid_w - box_w) / 2,
            box_w,
            side_w,
            mid_x,
            mid_w,
            small: Style { size: fx(rem * 0.875), line_height: small_line * ONE },
            small_line,
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
            actions: Vec::new(),
            apps: Vec::new(),
            app_rows: Vec::new(),
            clock_shown: clock_now(),
            chat_title: None,
            tool_lit: None,
            greeting: greeting_now(),
            welcome_fading: None,
            menu: None,
            attached: Vec::new(),
            key_typed: None,
            picked_model: None,
            sides: [true, true],
            sides_read: false,
            switch_lit: None,
            search_files: Vec::new(),
            looping: false,
            send_later: false,
            site: None,
            site_logged: None,
            allow_logged: None,
            send_shown: None,
            send_logged: None,
            pill: None,
        };
        c.lay_out_box();
        let card = c.composer.card;
        crate::log!("Chat Computer kernel: message box at rows {} to {}", card.y, card.bottom());
        c.draw(true);
        // The pointer, drawn over the screen from the mouse's interrupt: what's under it comes back from the canvas
        // (whose pixels stay where they are as the chat moves into its place).
        let (raw, pixels, hair) = (c.screen.raw(), c.canvas.pixels().as_ptr(), c.hair());
        *CHAT.lock() = Some(c);
        crate::pointer::init(raw, pixels, hair);
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

    /// The message box for what's typed now, as the website's (chat.css: #box, #row): its padding, what's typed across
    /// the top (the files attached as chips over it), and under that a row of round buttons 2 em across, + and the
    /// spark at the left, the microphone and send at the right. Changes `dirty` if its size changed.
    fn lay_out_box(&mut self) {
        let m = self.m;
        let border = self.hair();
        let (pad_top, pad_left, pad_right, pad_bottom) = (m.px(0.6875), m.px(0.9375), m.px(0.75), m.px(0.6875));
        let tool = m.px(2.0);
        let row_gap = m.px(0.55);
        let text_w = m.box_w - 2 * border - pad_left - pad_right;
        let lines = self.ty.breaks(&self.typed, m.body.size, text_w.max(1) * ONE);
        let shown = lines.len().clamp(1, BOX_LINES);
        let caret_line = self.caret_line(&lines);
        let first = (caret_line + 1).saturating_sub(shown).min(lines.len().saturating_sub(shown));
        // The chips: a row of them (as many as fit; the rest on more rows).
        let chip_h = (m.small_line as f32 + m.rem as f32 * 0.875 * 0.5) as i32;
        let chip_gap = m.px(0.4);
        let mut chip_rows = 0;
        let mut chip_places: Vec<(i32, i32, i32)> = Vec::new(); // (row, x within the row, width)
        {
            let mut x = 0;
            for (_, name) in &self.attached {
                let w = ((self.ty.width(name, m.small.size) + ONE - 1) / ONE + m.px(0.65) + m.px(1.75)).min(text_w);
                if x > 0 && x + w > text_w {
                    chip_rows += 1;
                    x = 0;
                }
                chip_places.push((chip_rows, x, w));
                x += w + chip_gap;
            }
            if !self.attached.is_empty() {
                chip_rows += 1;
            }
        }
        let chips_h = if chip_rows > 0 { chip_rows * chip_h + (chip_rows - 1) * chip_gap + m.px(0.55) } else { 0 };
        let h = 2 * border + pad_top + chips_h + shown as i32 * m.line + row_gap + tool + pad_bottom;
        let y = self.screen.height as i32 - m.px(2.4375) - h;
        let card = Rect::new(m.box_x, y, m.box_w, h);
        let left = m.box_x + border + pad_left;
        let right = card.right() - border - pad_right;
        let chips_y = y + border + pad_top;
        let chips = chip_places
            .iter()
            .map(|&(row, x, w)| {
                let r = Rect::new(left + x, chips_y + row * (chip_h + chip_gap), w, chip_h);
                let xd = m.px(1.4).min(chip_h);
                (r, Rect::new(r.right() - xd - m.px(0.15), r.y + (chip_h - xd) / 2, xd, xd))
            })
            .collect();
        let text_y = chips_y + chips_h;
        let row_cy = text_y + shown as i32 * m.line + row_gap + tool / 2;
        let r8 = tool * 4;
        let at = |x: i32| (x * 8 + tool * 4, row_cy * 8, r8);
        let tools = [
            at(left - m.px(0.35)),
            at(left + tool + m.px(0.1)),
            at(right - 2 * tool - m.px(0.45)),
            at(right - tool),
        ];
        if card != self.composer.card && self.composer.card != Rect::default() {
            self.relayout = true; // the conversation's room changed
        }
        self.composer = BoxLayout { card, text_x: left, text_y, lines, first, shown, tools, chips };
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
            let mut before: Option<Kind> = None;
            for b in &mut self.blocks {
                // (A little more room before what you say after something said to you, as the app has: so pinned
                // to the top of the screen, nothing of what came before shows.)
                if b.kind == Kind::You && before.is_some_and(|k| k != Kind::You) {
                    top += m.px(0.75);
                }
                b.top = top;
                top += b.height + m.px(1.25);
                before = Some(b.kind);
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
        let log = Rect::new(self.m.mid_x, 0, self.m.mid_w, if self.stopped { self.screen.height as i32 } else { self.log_bottom() });
        let m = self.m;
        let rule = ((2.0 * self.layout.scale + 0.5) as i32).max(1); // (theme.css's 2px lines)
        let now = time::uptime_ms();
        self.canvas.set_clip(clip);
        self.canvas.fill(clip, GROUND);
        if m.side_w > 0 && !self.stopped {
            self.render_sides(clip);
        }
        // A website open: in its frame, in place of the conversation.
        if self.site.is_some() && !self.stopped {
            self.render_site(clip.intersect(&log));
            self.canvas.set_clip(clip);
            self.render_box();
            return;
        }
        // The start screen: the word logo and the greeting (fading as the first message goes).
        let welcome = if self.stopped {
            None
        } else if !self.started {
            Some(1.0f32)
        } else {
            self.welcome_fading.map(|at| 1.0 - (now.saturating_sub(at) as f32 / WELCOME_FADE_MS as f32).min(1.0)).filter(|a| *a > 0.0)
        };
        if let Some(alpha) = welcome {
            self.canvas.set_clip(clip.intersect(&log));
            self.render_welcome(alpha);
        }
        // (The start screen's own facts aren't drawn under the word logo: the computer's status has them.)
        let facts_hidden = !self.started && !self.stopped;
        let Chat { canvas, blocks, ty, .. } = self;
        canvas.set_clip(clip.intersect(&log));
        let area = canvas.clip();
        let screen_h = canvas.height as i32;
        for b in blocks.iter_mut() {
            if facts_hidden {
                break;
            }
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

    /// The word logo and the greeting under it, in the middle of the room above the message box (as the website's
    /// #welcome), at `alpha` of their colours (1: shown, 0: gone).
    fn render_welcome(&mut self, alpha: f32) {
        let m = self.m;
        let mix = |rgb: u32| -> u32 {
            let ch = |c: u32, shift: u32| -> u32 {
                let (a, b) = ((c >> shift) & 0xFF, (GROUND >> shift) & 0xFF);
                ((b as f32 + (a as f32 - b as f32) * alpha) + 0.5) as u32 & 0xFF
            };
            ch(rgb, 16) << 16 | ch(rgb, 8) << 8 | ch(rgb, 0)
        };
        let big = Style { size: fx(m.rem * 3.0), line_height: ((m.rem * 3.0 * 1.05) as i32) * ONE };
        let mark = "Chat Computer";
        let mark_h = big.line_height / ONE;
        let width = m.mid_w - m.px(2.0);
        let lines = self.ty.breaks(&self.greeting, m.body.size, width.max(1) * ONE);
        let gap = m.px(0.7);
        let total = mark_h + gap + lines.len() as i32 * m.line;
        let room = self.log_bottom();
        let mut y = ((room - total) / 2).max(0);
        let center = m.mid_x + m.mid_w / 2;
        let ink = self.ty.ink_in(mark, Face::Bold, big, 0);
        let w = (self.ty.width_in(mark, Face::Bold, big.size) + ONE - 1) / ONE;
        self.canvas.ink(center - w / 2 - ink.left as i32, y, &ink, mix(INK));
        y += mark_h + gap;
        for r in lines {
            let part = String::from(self.greeting[r].trim_end());
            let ink = self.ty.ink_in(&part, Face::Regular, m.body, 0);
            let w = (self.ty.width(&part, m.body.size) + ONE - 1) / ONE;
            self.canvas.ink(center - w / 2 - ink.left as i32, y, &ink, mix(MUTED));
            y += m.line;
        }
    }

    /// Which of the message box's round buttons is at (x, y) (a little beyond each circle counts too: they're small).
    fn tool_at(&self, x: f32, y: f32, slack: i64) -> Option<usize> {
        let (px8, py8) = ((x * 8.0) as i64, (y * 8.0) as i64);
        self.composer.tools.iter().position(|&(cx, cy, r)| {
            let (dx, dy) = (px8 - cx as i64, py8 - cy as i64);
            dx * dx + dy * dy <= (r as i64 + slack).pow(2)
        })
    }

    /// The pointer moved to `to` (pointer.rs moved it, as the mouse's reports came), or its left button was pressed
    /// there (`click`): what's under it lights up, and a click does what it's on (the send button sends what's typed,
    /// or stops the answer coming).
    fn pointer_at(&mut self, to: (f32, f32), click: bool) {
        let pressed = if click { 1 } else { 0 };
        // Under the pointer: a list's item is chosen; a button lights up.
        if let (Some(r), Some(menu)) = (self.menu_rect(), self.menu.as_ref()) {
            let row_h = self.m.line + self.m.px(0.5);
            let top = r.y + self.m.px(0.5) + self.m.small_line + self.m.px(0.35);
            let (px, py) = (to.0 as i32, to.1 as i32);
            if px >= r.x && px < r.right() && py >= top {
                let k = menu.first + ((py - top) / row_h) as usize;
                if k < menu.items.len() && k < menu.first + MENU_ROWS && self.menu.as_ref().is_some_and(|m| m.chosen != k) {
                    if let Some(m) = self.menu.as_mut() {
                        m.chosen = k;
                    }
                    self.dirty = self.dirty.union(&r);
                }
            }
        }
        // (Drawn again only when what's lit changes.)
        let tool = self.tool_at(to.0, to.1, 0).filter(|&t| t < TOOL_SEND);
        if tool != self.tool_lit {
            self.tool_lit = tool;
            let card = self.composer.card;
            self.dirty = self.dirty.union(&card);
        }
        // A sidebar's switch under the pointer lights up.
        let inside = |r: Rect| to.0 as i32 >= r.x && (to.0 as i32) < r.right() && to.1 as i32 >= r.y && (to.1 as i32) < r.bottom();
        let lit = if self.m.side_w > 0 { (0..2).find(|&k| inside(self.switch_rect(k))) } else { None };
        if lit != self.switch_lit {
            for k in [self.switch_lit, lit].into_iter().flatten() {
                let r = self.switch_rect(k);
                self.dirty = self.dirty.union(&r);
            }
            self.switch_lit = lit;
        }
        if pressed & 1 == 0 || self.stopped {
            return;
        }
        if let Some(k) = lit {
            self.switch_side(k);
            // (Where the switch is now: it moves with its sidebar.)
            self.switch_lit = inside(self.switch_rect(k)).then_some(k);
            let r = self.switch_rect(k);
            self.dirty = self.dirty.union(&r);
            return;
        }
        let (px, py) = (to.0 as i32, to.1 as i32);
        // In the list: its item picked (elsewhere, the list closes).
        if let Some(r) = self.menu_rect() {
            if px >= r.x && px < r.right() && py >= r.y && py < r.bottom() {
                let k = self.menu.as_ref().map_or(0, |m| m.chosen);
                return self.pick(k);
            }
            self.close_menu();
        }
        // An app in the Apps list: opened.
        if let Some((_, name)) = self.app_rows.iter().find(|(r, _)| px >= r.x && px < r.right() && py >= r.y && py < r.bottom()).cloned() {
            if let Some(html) = assistant::app(&name) {
                self.open_app(name, html);
            }
            return;
        }
        // In a website's frame: its x closes it; in its page, a link followed, a field to type in, a box ticked.
        if self.site.is_some() {
            let (frame, close, page) = self.site_rects();
            let inside = |r: Rect| px >= r.x && px < r.right() && py >= r.y && py < r.bottom();
            if inside(close) {
                return self.close_site();
            }
            // An app's question: its Send and Don't send buttons (only the person's pointer presses them); the rest
            // of its box does nothing.
            if let Some(b) = self.send_box() {
                if inside(b.send) || inside(b.dont) {
                    return self.answer_send(b.id, inside(b.send));
                }
                if inside(b.rect) {
                    return;
                }
            }
            if let Some((allow, _)) = self.allow_rect()
                && inside(allow)
            {
                return self.allow_app();
            }
            if inside(page) {
                if let Some(s) = self.site.as_mut() {
                    s.click((px - page.x) as f32, (py - page.y) as f32);
                }
                self.dirty = self.dirty.union(&frame);
                return;
            }
            // (Elsewhere: what's typed goes to the message box again.)
            if let Some(s) = self.site.as_mut()
                && s.focus.take().is_some()
            {
                self.dirty = self.dirty.union(&page);
            }
        }
        // A chip's x: the file taken off.
        if let Some(i) = self.composer.chips.iter().position(|(_, x)| px >= x.x && px < x.right() && py >= x.y && py < x.bottom()) {
            self.attached.remove(i);
            self.lay_out_box();
            self.dirty = self.dirty.union(&self.box_area());
            return;
        }
        // (A little beyond each circle counts too: they're small.)
        let hit = self.tool_at(to.0, to.1, 16 * self.hair() as i64);
        match hit {
            Some(TOOL_ADD) => self.open_menu(MenuKind::Files),
            Some(TOOL_AI) => self.open_menu(MenuKind::Ai),
            Some(TOOL_MIC) => self.talk(),
            Some(_) if self.key_typed.is_some() => {
                self.edit_key(Press { key: Key::Enter, ctrl: false, alt: false, logo: false });
            }
            Some(_) => {
                if self.asking.is_some() {
                    assistant::stop();
                    self.answer_over(Some(String::from("Stopped.")));
                } else if !self.typed.trim().is_empty() || !self.attached.is_empty() {
                    self.send();
                }
            }
            None => {}
        }
    }

    /// The two sidebars (where they fit), drawn in `clip`.
    fn render_sides(&mut self, clip: Rect) {
        let m = self.m;
        let (w, h) = (self.screen.width as i32, self.screen.height as i32);
        let hair = self.hair();
        let (left, right) = (Rect::new(0, 0, m.side_w, h), Rect::new(w - m.side_w, 0, m.side_w, h));
        let sides = self.sides;
        let switches = [self.switch_rect(0), self.switch_rect(1)];
        let lit = self.switch_lit;
        let Chat { canvas, ty, actions, clock_shown, chat_title, apps, app_rows, .. } = self;
        canvas.set_clip(clip);
        for (k, (side, line_x)) in [(left, left.right() - hair), (right, right.x)].into_iter().enumerate() {
            if sides[k] {
                canvas.fill(side, SIDE);
                canvas.fill(Rect::new(line_x, 0, hair, h), LINE);
            }
            draw_switch(canvas, switches[k], k, sides[k], lit == Some(k), &m, if sides[k] { SIDE } else { GROUND });
        }
        if !sides[0] && !sides[1] {
            return;
        }
        // Sizes as chat.css's: padding 0.75 rem (0.5 rem on a sidebar's right), words inset 0.4 em, a title's row 1.6
        // em high and 0.75 rem above what follows.
        let small = m.small;
        let em = m.rem * 0.875;
        let inset = (0.4 * em + 0.5) as i32;
        let (pad_l, pad_r, pad_t) = (m.px(0.75), m.px(0.5), m.px(0.75));
        let head_h = (1.6 * em + 0.5) as i32;
        let words_w = m.side_w - pad_l - pad_r - inset;
        let text = |canvas: &mut Canvas, s: &str, face: Face, x: i32, y: i32, rgb: u32| {
            let ink = ty.ink_in(s, face, small, 0);
            canvas.ink(x - ink.left as i32, y, &ink, rgb);
        };
        let wrapped = |canvas: &mut Canvas, s: &str, x: i32, mut y: i32, max: i32, rgb: u32| -> i32 {
            for r in ty.breaks(s, small.size, max * ONE) {
                let part = s[r].trim_end();
                if !part.is_empty() {
                    let ink = ty.ink_in(part, Face::Regular, small, 0);
                    canvas.ink(x - ink.left as i32, y, &ink, rgb);
                }
                y += m.small_line;
            }
            y
        };
        let head = |canvas: &mut Canvas, title: &str, x: i32, y: i32| -> i32 {
            text(canvas, title, Face::Bold, x + inset, y + (head_h - m.small_line) / 2, INK);
            y + head_h + m.px(0.75)
        };
        // Messages, over the apps the model made (when there are any): each a row to click, its name cut to fit.
        app_rows.clear();
        if sides[0] {
            let y = head(canvas, "Messages", left.x + pad_l, pad_t);
            wrapped(canvas, MESSAGES_EMPTY, left.x + pad_l + inset, y, words_w, INK);
            if !apps.is_empty() {
                let half = h / 2;
                canvas.fill(Rect::new(left.x, half, m.side_w - hair, hair), LINE);
                let x = left.x + pad_l;
                let mut y = head(canvas, "Apps", x, half + hair + pad_t);
                let pad = (0.4 * em + 0.5) as i32;
                let item_h = m.small_line + 2 * (0.5 * em) as i32;
                let row_w = m.side_w - hair - pad_l - pad_r;
                for name in apps.iter() {
                    if y + item_h > h - pad_t {
                        break;
                    }
                    let row = Rect::new(x, y, row_w, item_h);
                    canvas.round_rect(row, (0.5 * em) as i32, CHOSEN);
                    let max = row_w - 2 * pad;
                    let mut shown = name.clone();
                    if ty.width(&shown, small.size) > max * ONE {
                        while !shown.is_empty() && ty.width(&format!("{shown}\u{2026}"), small.size) > max * ONE {
                            shown.pop();
                        }
                        shown = format!("{}\u{2026}", shown.trim_end());
                    }
                    text(canvas, &shown, Face::Regular, x + pad, y + (0.5 * em) as i32, INK);
                    app_rows.push((row, name.clone()));
                    y += item_h + m.px(0.4);
                }
            }
        }
        if !sides[1] {
            return;
        }
        // Actions, over Chats: two halves of the same height, a line between them. (Its title clear of its switch,
        // as chat.css's #actions .side-head.)
        let x = right.x + hair + pad_l;
        let half = h / 2;
        let mut y = head(canvas, "Actions", x + m.px(1.95), pad_t);
        y -= m.px(0.4);
        text(canvas, &clock_shown.0, Face::Regular, x + inset, y, INK);
        text(canvas, &clock_shown.1, Face::Regular, x + inset, y + m.small_line, INK);
        y += 2 * m.small_line + m.px(0.9);
        if actions.is_empty() {
            wrapped(canvas, ACTIONS_EMPTY, x + inset, y, words_w, INK);
        } else {
            // The latest that fit, oldest at the top (as the app lists them), each with its time at its first line's
            // right, in grey.
            let room = half - m.px(0.75) - y;
            let item = |what: &str, when: &str| -> (Vec<Range<usize>>, i32) {
                let when_w = (ty.width(when, small.size) + ONE - 1) / ONE + (0.6 * em) as i32;
                let lines = ty.breaks(what, small.size, (words_w - when_w).max(1) * ONE);
                let height = lines.len() as i32 * m.small_line + (0.7 * em) as i32;
                (lines, height)
            };
            let mut first = actions.len();
            let mut used = 0;
            while first > 0 {
                let (_, height) = item(&actions[first - 1].0, &actions[first - 1].1);
                if used + height > room {
                    break;
                }
                used += height;
                first -= 1;
            }
            let mut at = y;
            for (what, when) in &actions[first..] {
                let (lines, height) = item(what, when);
                let top = at + (0.35 * em) as i32;
                for (i, r) in lines.iter().enumerate() {
                    let part = what[r.clone()].trim_end();
                    if !part.is_empty() {
                        text(canvas, part, Face::Regular, x + inset, top + i as i32 * m.small_line, INK);
                    }
                }
                let when_x = right.right() - pad_r - inset - (ty.width(when, small.size) + ONE - 1) / ONE;
                text(canvas, when, Face::Regular, when_x, top, MUTED);
                at += height;
            }
        }
        canvas.fill(Rect::new(right.x + hair, half, m.side_w - hair, hair), LINE);
        let y = head(canvas, "Chats", x, half + hair + pad_t);
        match chat_title {
            Some(title) => {
                // One line, cut short with an ellipsis if it's longer, marked as the one on the screen.
                let pad = (0.4 * em + 0.5) as i32;
                let item_h = m.small_line + 2 * (0.5 * em) as i32;
                canvas.round_rect(Rect::new(x, y, m.side_w - hair - pad_l - pad_r, item_h), (0.5 * em) as i32, CHOSEN);
                let max = m.side_w - hair - pad_l - pad_r - 2 * pad;
                let mut shown = String::from(title.lines().next().unwrap_or(""));
                if ty.width(&shown, small.size) > max * ONE {
                    while !shown.is_empty() && ty.width(&format!("{shown}\u{2026}"), small.size) > max * ONE {
                        shown.pop();
                    }
                    shown = format!("{}\u{2026}", shown.trim_end());
                }
                text(canvas, &shown, Face::Regular, x + pad, y + (0.5 * em) as i32, INK);
            }
            None => {
                wrapped(canvas, CHATS_EMPTY, x + inset, y, words_w, INK);
            }
        }
    }

    /// Where a sidebar's switch is (0: Messages', 1: Actions'): 1.6 rem square, 0.65 rem from the top, 2.2 rem in
    /// from the sidebar's edge nearest the middle while it shows, else 0.6 rem from the screen's edge (chat.css's
    /// .toggle).
    fn switch_rect(&self, k: usize) -> Rect {
        let m = self.m;
        let (w, size) = (self.screen.width as i32, m.px(1.6));
        let inset = if self.sides[k] { (m.side_w - m.px(2.2)).max(m.px(0.6)) } else { m.px(0.6) };
        let x = if k == 0 { inset } else { w - inset - size };
        Rect::new(x, m.px(0.65), size, size)
    }

    /// A sidebar switched off or on with its switch: the middle stays where it is (as the website's).
    fn switch_side(&mut self, k: usize) {
        self.sides[k] = !self.sides[k];
        assistant::set_sides(self.sides);
        let (w, h) = (self.screen.width as i32, self.screen.height as i32);
        let area = if k == 0 { Rect::new(0, 0, self.m.side_w, h) } else { Rect::new(w - self.m.side_w, 0, self.m.side_w, h) };
        self.dirty = self.dirty.union(&area);
    }

    /// The right sidebar's part of the screen (for redrawing it when what it lists changes).
    fn right_side(&self) -> Rect {
        let w = self.screen.width as i32;
        Rect::new(w - self.m.side_w, 0, self.m.side_w, self.screen.height as i32)
    }

    /// Something Chat Computer did for the person, for the Actions sidebar.
    fn action(&mut self, what: String) {
        if self.actions.last().is_some_and(|(w, _)| *w == what) {
            return;
        }
        let when = clock_now().1;
        self.actions.push((what, when));
        if self.actions.len() > ACTIONS_MAX {
            self.actions.remove(0);
        }
        let area = self.right_side();
        self.dirty = self.dirty.union(&area);
    }

    /// The conversation's first message, for the Chats sidebar.
    fn set_chat_title(&mut self, title: Option<String>) {
        if self.chat_title != title {
            self.chat_title = title;
            let area = self.right_side();
            self.dirty = self.dirty.union(&area);
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
                _ if self.key_typed.is_some() => "Your key for Claude (made at platform.claude.com/settings/keys), then Enter",
                Some(false) => "Listening\u{2026} say it, then pause (Esc stops)",
                Some(true) => "Hearing what you said\u{2026}",
                None if !self.looping => "Starting\u{2026} type now: it's sent once Chat Computer is ready",
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
        // The chips: each file attached, its name, and an x to take it off.
        for (i, (r, x)) in b.chips.iter().enumerate() {
            let Some((_, name)) = self.attached.get(i) else { continue };
            canvas.round_rect(*r, m.px(0.6), BOX_LINE);
            canvas.round_rect(Rect::new(r.x + hair, r.y + hair, r.w - 2 * hair, r.h - 2 * hair), m.px(0.6) - hair, BOX_GROUND);
            let ink = self.ty.ink(name, m.small, 0);
            canvas.ink(r.x + m.px(0.65) - ink.left as i32, r.y + (r.h - m.small_line) / 2, &ink, BOX_INK);
            let (cx, cy, u) = ((x.x * 2 + x.w) * 4, (x.y * 2 + x.h) * 4, x.w * 8 * 7 / 10);
            let p = |a: i32, b: i32| -> Point { (cx - u / 2 + a * u / 24, cy - u / 2 + b * u / 24) };
            canvas.strokes(&[(p(6, 6), p(18, 18)), (p(18, 6), p(6, 18))], u * 2 / 24, BOX_INK);
        }
        // The round buttons: +, the spark, the microphone (each in its own circle, lighter under the pointer), then
        // send (bright once there's something to send; its stop while an answer comes).
        let lit = self.tool_lit;
        let listening = crate::listening::state().is_some();
        for (i, t) in b.tools.iter().enumerate().take(TOOL_SEND) {
            let ground = if i == TOOL_MIC && listening { mix(KNOB, BOX_INK, 0.25) } else if lit == Some(i) { mix(KNOB, BOX_INK, 0.12) } else { KNOB };
            canvas.circle(t.0, t.1, t.2, ground, 255);
            let unit = t.2 * 2 * 115 / 200; // (1.15 em: the icon's 24 units, in 1/8 pixel)
            let p = |a: f32, b: f32| -> Point { (t.0 - unit / 2 + (a * unit as f32 / 24.0) as i32, t.1 - unit / 2 + (b * unit as f32 / 24.0) as i32) };
            let width = unit * 2 / 24;
            let mut segs: Vec<(Point, Point)> = Vec::new();
            let poly = |pts: &[(f32, f32)], closed: bool, segs: &mut Vec<(Point, Point)>| {
                for w in pts.windows(2) {
                    segs.push((p(w[0].0, w[0].1), p(w[1].0, w[1].1)));
                }
                if closed && pts.len() > 2 {
                    segs.push((p(pts[pts.len() - 1].0, pts[pts.len() - 1].1), p(pts[0].0, pts[0].1)));
                }
            };
            match i {
                TOOL_ADD => {
                    poly(&[(12.0, 5.0), (12.0, 19.0)], false, &mut segs);
                    poly(&[(5.0, 12.0), (19.0, 12.0)], false, &mut segs);
                }
                TOOL_AI => {
                    // (chat.html's i-spark: a four-pointed star, and a little one.)
                    poly(&[(12.0, 3.0), (13.8, 8.2), (19.0, 10.0), (13.8, 11.8), (12.0, 17.0), (10.2, 11.8), (5.0, 10.0), (10.2, 8.2)], true, &mut segs);
                    poly(&[(19.0, 15.0), (19.8, 17.2), (22.0, 18.0), (19.8, 18.8), (19.0, 21.0), (18.2, 18.8), (16.0, 18.0), (18.2, 17.2)], true, &mut segs);
                }
                _ => {
                    // The microphone: its head (a rounded bar), the arc under it, and its stand.
                    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<(f32, f32)> {
                        (0..=12).map(|k| {
                            let a = (from + (to - from) * k as f32 / 12.0) as f64;
                            let (sn, cs) = model::math::sin_cos(a);
                            (cx + r * cs as f32, cy + r * sn as f32)
                        }).collect()
                    };
                    use core::f32::consts::PI;
                    let mut head = arc(12.0, 6.5, 3.0, PI, 2.0 * PI);
                    head.extend(arc(12.0, 11.5, 3.0, 0.0, PI));
                    poly(&head, true, &mut segs);
                    poly(&arc(12.0, 11.5, 6.5, PI, 0.0), false, &mut segs);
                    poly(&[(12.0, 18.0), (12.0, 20.5)], false, &mut segs);
                }
            }
            canvas.strokes(&segs, width, BOX_INK);
        }
        let (cx, cy, r) = b.tools[TOOL_SEND];
        let busy = assistant::busy();
        let ready = !self.typed.trim().is_empty() && !busy;
        canvas.circle(cx, cy, r, if ready || busy { BOX_INK } else { DIM }, 255);
        let unit = r * 2 * 55 / 100; // the icon's size, in 1/8 pixel; its drawing is 24 units across
        let at = |u: i32, v: i32| (cx - unit / 2 + u * unit / 240, cy - unit / 2 + v * unit / 240);
        if busy {
            // (While an answer comes: a square, its stop.)
            let s = unit * 7 / 24;
            canvas.round_rect(Rect::new((cx - s) / 8, (cy - s) / 8, s / 4, s / 4), (s / 16).max(1), BOX_GROUND);
        } else {
            let segments = [(at(120, 190), at(120, 50)), (at(55, 115), at(120, 50)), (at(120, 50), at(185, 115))];
            canvas.strokes(&segments, unit * 26 / 240, BOX_GROUND);
        }
        self.render_menu();
    }

    /// The list above the message box (+'s files, or the spark's models), if it's open.
    fn render_menu(&mut self) {
        let Some(r) = self.menu_rect() else { return };
        let m = self.m;
        let hair = self.hair();
        let Some(menu) = self.menu.as_ref() else { return };
        let canvas = &mut self.canvas;
        canvas.shadow(r, m.px(0.25), m.px(1.25), SHADOW);
        canvas.round_rect(r, m.px(0.75), BOX_LINE);
        canvas.round_rect(Rect::new(r.x + hair, r.y + hair, r.w - 2 * hair, r.h - 2 * hair), m.px(0.75) - hair, PAPER);
        let pad = m.px(0.5);
        let row_h = m.line + m.px(0.5);
        let title = self.ty.ink_in(&menu.title, Face::Bold, m.small, 0);
        canvas.ink(r.x + pad + m.px(0.5) - title.left as i32, r.y + pad, &title, MUTED);
        let top = r.y + pad + m.small_line + m.px(0.35);
        if menu.items.is_empty() {
            let ink = self.ty.ink("Nothing here yet.", m.body, 0);
            canvas.ink(r.x + pad + m.px(0.5) - ink.left as i32, top + m.px(0.25), &ink, DIM);
            return;
        }
        for (k, item) in menu.items.iter().enumerate().skip(menu.first).take(MENU_ROWS) {
            let y = top + (k - menu.first) as i32 * row_h;
            let row = Rect::new(r.x + pad, y, r.w - 2 * pad, row_h);
            if k == menu.chosen {
                canvas.round_rect(row, m.px(0.5), BOX_CHOSEN);
            }
            let ty_y = y + (row_h - m.line) / 2;
            let detail_w = (self.ty.width(&item.detail, m.small.size) + ONE - 1) / ONE;
            let label_room = row.w - m.px(1.0) - detail_w - m.px(1.5);
            let mut label = item.label.clone();
            if (self.ty.width(&label, m.body.size) + ONE - 1) / ONE > label_room {
                while !label.is_empty() && (self.ty.width(&format!("{label}\u{2026}"), m.body.size) + ONE - 1) / ONE > label_room {
                    label.pop();
                }
                label.push('\u{2026}');
            }
            if item.ticked {
                label = format!("{label}  \u{2713}");
            }
            let ink = self.ty.ink(&label, m.body, 0);
            canvas.ink(row.x + m.px(0.5) - ink.left as i32, ty_y, &ink, BOX_INK);
            let ink = self.ty.ink(&item.detail, m.small, 0);
            canvas.ink(row.right() - m.px(0.5) - detail_w - ink.left as i32, ty_y + (m.line - m.small_line) / 2, &ink, MUTED);
        }
    }

    /// The list's place: above the message box, as wide as it.
    fn menu_rect(&self) -> Option<Rect> {
        let menu = self.menu.as_ref()?;
        let m = self.m;
        let rows = menu.items.len().clamp(1, MENU_ROWS) as i32;
        let h = 2 * m.px(0.5) + m.small_line + m.px(0.35) + rows * (m.line + m.px(0.5));
        let card = self.composer.card;
        Some(Rect::new(card.x, card.y - m.px(0.5) - h, card.w, h))
    }

    /// Opens a list above the box: + 's files (the newest in each person's Desktop, Documents, Downloads and Pictures),
    /// or the spark's models (those on this PC, the one answering ticked).
    fn open_menu(&mut self, kind: MenuKind) {
        if self.menu.as_ref().is_some_and(|m| m.kind == kind) {
            return self.close_menu();
        }
        let (title, items) = match kind {
            MenuKind::Files => (
                String::from("Attach a file (from Desktop, Documents, Downloads and Pictures)"),
                // (The newest first, at the top.)
                crate::files::recent(MENU_FILES)
                    .into_iter()
                    .map(|f| MenuItem { detail: format!("{}, {}", size_text(f.size), when_text(f.modified)), label: f.name, path: f.path, ticked: false })
                    .collect(),
            ),
            MenuKind::Search => {
                if self.search_files.is_empty() {
                    self.search_files = crate::files::recent(SEARCHED);
                }
                (String::from("Open a file on this PC (the newest first)"), self.search_items())
            }
            MenuKind::Ai => {
                let answering = assistant::model_path();
                let claude = assistant::claude();
                let mut items: Vec<MenuItem> = crate::files::app_models()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(path, size)| MenuItem {
                        label: model_label(&path),
                        detail: format!("on this PC, free: nothing you type leaves it ({})", size_text(size)),
                        ticked: claude.is_none() && answering.as_deref() == Some(path.as_str()),
                        path,
                    })
                    .collect();
                if items.is_empty() {
                    items.push(MenuItem { label: String::from("No model on this PC yet"), detail: String::from("setup gets one on Windows"), path: String::new(), ticked: false });
                }
                let detail = match (&claude, assistant::has_claude_key()) {
                    (Some(c), _) => format!(
                        "{}, {} effort, with your key: what you type goes to Anthropic",
                        assistant::claude_name(&c.model),
                        assistant::effort_name(c.effort.as_deref().unwrap_or(assistant::default_effort(&c.model)))
                    ),
                    (None, true) => String::from("by Anthropic, with your key: what you type then goes to Anthropic"),
                    (None, false) => String::from("by Anthropic, with your own key: what you type then goes to Anthropic"),
                };
                items.push(MenuItem { label: String::from("Claude"), detail, path: String::from(CLAUDE_ITEM), ticked: claude.is_some() });
                (String::from("Which AI answers"), items)
            }
            MenuKind::ClaudeModels => {
                let current = assistant::claude().map(|c| c.model);
                let mut items: Vec<MenuItem> = assistant::CLAUDE_MODELS
                    .iter()
                    .map(|(id, name, effort)| MenuItem {
                        label: String::from(*name),
                        detail: format!("{id}, {} effort unless you choose", assistant::effort_name(effort).to_ascii_lowercase()),
                        path: String::from(*id),
                        ticked: current.as_deref() == Some(*id),
                    })
                    .collect();
                items.push(MenuItem { label: String::from("Give another key"), detail: String::from("for Claude, in place of this one"), path: String::from(NEW_KEY), ticked: false });
                items.push(MenuItem { label: String::from("Forget your key"), detail: String::from("Chat Computer's own model answers then"), path: String::from(FORGET_KEY), ticked: false });
                (String::from("Which of Claude's models answers"), items)
            }
            MenuKind::Efforts => {
                let model = self.picked_model.clone().unwrap_or_else(|| String::from(assistant::CLAUDE_MODELS[0].0));
                let current = assistant::claude().filter(|c| c.model == model).and_then(|c| c.effort);
                let default = assistant::default_effort(&model);
                let items = assistant::EFFORT_NAMES
                    .iter()
                    .map(|(e, name)| MenuItem {
                        label: String::from(*name),
                        detail: String::from(match *e {
                            "low" => "quickest, and costs the least",
                            "medium" => "a balance of thought, speed and cost",
                            "high" => "thinks as much as each question needs",
                            "xhigh" => "more thought, for long and hard work",
                            _ => "the most thought, whatever it costs",
                        }) + if *e == default { " (its default)" } else { "" },
                        path: String::from(*e),
                        ticked: current.as_deref().unwrap_or(default) == *e,
                    })
                    .collect();
                (format!("How hard {} thinks (its effort)", assistant::claude_name(&model)), items)
            }
        };
        // (What's ticked; else the first: in the lists of files, the newest.)
        let chosen = items.iter().position(|i| i.ticked).unwrap_or(0);
        self.menu = Some(Menu { kind, title, items, chosen, first: chosen.saturating_sub(MENU_ROWS - 1) });
        self.dirty = self.dirty.union(&self.canvas.whole());
    }

    fn close_menu(&mut self) {
        if let Some(r) = self.menu_rect() {
            self.menu = None;
            self.dirty = self.dirty.union(&Rect::new(r.x - self.m.px(3.0), r.y - self.m.px(3.0), r.w + self.m.px(6.0), r.h + self.m.px(6.0)));
        }
        self.menu = None;
    }

    /// The list's item picked: a file attached, or a model chosen.
    fn pick(&mut self, k: usize) {
        let Some(menu) = self.menu.take() else { return };
        self.dirty = self.dirty.union(&self.canvas.whole());
        let Some(item) = menu.items.get(k) else { return };
        match menu.kind {
            MenuKind::Files => {
                if !self.attached.iter().any(|(p, _)| *p == item.path) {
                    self.attached.push((item.path.clone(), item.label.clone()));
                }
                self.lay_out_box();
            }
            MenuKind::Ai => {
                if item.path == CLAUDE_ITEM {
                    if assistant::has_claude_key() {
                        self.open_menu(MenuKind::ClaudeModels);
                    } else {
                        self.ask_key();
                    }
                } else if assistant::claude().is_some() && !item.path.is_empty() {
                    assistant::choose_ai(None);
                    self.note_now(String::from(
                        "Chat Computer's own model answers again, here on this PC: nothing you type leaves it.",
                    ));
                } else if !item.ticked && !item.path.is_empty() {
                    let note = format!(
                        "{} is on this PC too. Chat Computer's own system answers with one model each time it starts \
                         (the biggest that answers quickly here); choosing it here comes soon.",
                        item.label
                    );
                    self.note_now(note);
                }
            }
            MenuKind::ClaudeModels => match item.path.as_str() {
                NEW_KEY => self.ask_key(),
                FORGET_KEY => {
                    assistant::forget_claude_key();
                    self.note_now(String::from(
                        "Your key for Claude is forgotten on this PC. Chat Computer's own model answers: nothing you type \
                         leaves this PC.",
                    ));
                }
                model => {
                    self.picked_model = Some(String::from(model));
                    self.open_menu(MenuKind::Efforts);
                }
            },
            MenuKind::Search => {
                let query = String::from(self.typed.get(1..).unwrap_or("").trim());
                self.typed.clear();
                self.cursor = 0;
                self.search_files.clear();
                if item.path == ASK_FIND {
                    if !query.is_empty() {
                        self.typed = format!("Find \u{201C}{query}\u{201D} in my files.");
                        self.cursor = self.typed.len();
                        self.lay_out_box();
                        return self.send();
                    }
                } else {
                    // (Opened, in the middle: + attaches a file; "/" opens one.)
                    self.lay_out_box();
                    return self.open_file(item.path.clone());
                }
                self.lay_out_box();
            }
            MenuKind::Efforts => {
                let model = self.picked_model.take().unwrap_or_else(|| String::from(assistant::CLAUDE_MODELS[0].0));
                let effort = item.path.clone();
                assistant::choose_ai(Some(assistant::Claude { model: model.clone(), effort: Some(effort.clone()) }));
                if assistant::claude().is_some() {
                    self.note_now(format!(
                        "{} answers now, at {} effort, with your key: what you type goes to Anthropic, and is paid for \
                         from your account there. The spark beside + switches back to Chat Computer's own model.",
                        assistant::claude_name(&model),
                        assistant::effort_name(&effort)
                    ));
                }
            }
        }
    }

    /// The box asks for your key for Claude: what's typed next is it (shown as dots), Enter gives it, Esc doesn't.
    fn ask_key(&mut self) {
        files::wipe(&mut self.typed);
        self.cursor = 0;
        self.key_typed = Some(String::new());
        self.lay_out_box();
        self.dirty = self.dirty.union(&self.canvas.whole());
    }

    /// A key being typed: its characters (never shown: a dot each), Backspace, Enter, Esc. True if `p` was for it.
    fn edit_key(&mut self, p: Press) -> bool {
        let Some(key) = self.key_typed.as_mut() else { return false };
        match p.key {
            Key::Char(c) if !p.ctrl && !p.alt && !p.logo => {
                if !c.is_whitespace() && key.len() < 512 {
                    key.push(c);
                    self.typed.push('\u{2022}');
                }
            }
            Key::Backspace => {
                key.pop();
                self.typed.pop();
            }
            Key::Enter => {
                let mut key = self.key_typed.take().unwrap_or_default();
                files::wipe(&mut self.typed);
                self.cursor = 0;
                if key.trim().is_empty() {
                    files::wipe(&mut key);
                } else {
                    assistant::set_claude_key(&mut key);
                    self.lay_out_box();
                    self.open_menu(MenuKind::ClaudeModels);
                    return true;
                }
            }
            Key::Escape => {
                if let Some(mut k) = self.key_typed.take() {
                    files::wipe(&mut k);
                }
                files::wipe(&mut self.typed);
                self.cursor = 0;
            }
            _ => return true,
        }
        self.cursor = self.typed.len();
        self.lay_out_box();
        let area = self.box_area();
        self.dirty = self.dirty.union(&area);
        true
    }

    /// A note in the conversation now (the start screen's word logo fading, if it's still there).
    fn note_now(&mut self, note: String) {
        if !self.started {
            self.started = true;
            self.blocks.clear();
            self.welcome_fading = Some(time::uptime_ms());
        }
        self.blocks.push(Block::new(Kind::Note, Live::No, note));
        self.relayout = true;
        self.scroll = None;
        self.dirty = self.dirty.union(&self.canvas.whole());
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
        // (Over the pointer, if it was there: it's drawn again. Interrupts are off, the chat's lock held, so it can't
        // move meanwhile.)
        crate::pointer::over(dirty.x, dirty.y, dirty.w, dirty.h);
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
        // What's typed, for the model to read ahead (not a key for Claude being typed).
        assistant::typing(if self.key_typed.is_some() { "" } else { &self.typed });
        if !self.sides_read && self.looping {
            self.sides_read = true;
            let sides = assistant::sides();
            if sides != self.sides {
                self.sides = sides;
                self.dirty = self.dirty.union(&self.canvas.whole());
            }
        }
        // A website's page and what it asks for, fetched; drawn again when something came.
        if let Some(s) = self.site.as_mut()
            && s.work(&self.ty)
        {
            let (frame, _, _) = self.site_rects();
            self.dirty = self.dirty.union(&frame);
        }
        for what in self.site.as_mut().map(|s| s.take_actions()).unwrap_or_default() {
            self.action(what);
        }
        let shown = clock_now();
        if shown != self.clock_shown {
            self.clock_shown = shown;
            let area = self.right_side();
            self.dirty = self.dirty.union(&area);
        }
        let now = time::uptime_ms();
        if let Some(at) = self.welcome_fading {
            if now.saturating_sub(at) > WELCOME_FADE_MS + 50 {
                self.welcome_fading = None;
            }
            let log = Rect::new(self.m.mid_x, 0, self.m.mid_w, self.log_bottom());
            self.dirty = self.dirty.union(&log);
        }
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
        if self.site.is_some() && self.menu.is_none() && self.key_typed.is_none() && self.site_key(p) {
            return;
        }
        // A list open above the box: the arrows choose in it, Enter picks, Esc closes it; anything else closes it and
        // goes to the box.
        if let Some(menu) = self.menu.as_mut() {
            let n = menu.items.len();
            match p.key {
                Key::Up | Key::Down if n > 0 => {
                    menu.chosen = if p.key == Key::Up { menu.chosen.saturating_sub(1) } else { (menu.chosen + 1).min(n - 1) };
                    if menu.chosen < menu.first {
                        menu.first = menu.chosen;
                    } else if menu.chosen >= menu.first + MENU_ROWS {
                        menu.first = menu.chosen + 1 - MENU_ROWS;
                    }
                    let r = self.menu_rect().unwrap_or_default();
                    self.dirty = self.dirty.union(&r);
                    return;
                }
                Key::Enter => {
                    let k = menu.chosen;
                    return self.pick(k);
                }
                Key::Escape if menu.kind == MenuKind::Search => {
                    // (As the website's: Escape clears what was typed.)
                    self.close_menu();
                    self.typed.clear();
                    self.cursor = 0;
                    self.search_files.clear();
                    self.lay_out_box();
                    let area = self.box_area();
                    self.dirty = self.dirty.union(&area);
                    return;
                }
                Key::Escape => return self.close_menu(),
                _ if menu.kind == MenuKind::Search => {}
                _ => self.close_menu(),
            }
        }
        if self.edit_key(p) {
            return;
        }
        // (Backspace in an empty box takes off the last file attached.)
        if matches!(p.key, Key::Backspace) && self.cursor == 0 && self.typed.is_empty() && !self.attached.is_empty() {
            self.attached.pop();
            self.lay_out_box();
            self.dirty = self.dirty.union(&self.canvas.whole());
            return;
        }
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
        self.searching();
    }

    /// "/" at the start of what's typed: the list of files found for what follows it (opened, or brought up to date);
    /// gone again once the "/" is.
    fn searching(&mut self) {
        let on = self.typed.starts_with('/') && self.key_typed.is_none();
        match self.menu.as_ref().map(|m| m.kind) {
            Some(MenuKind::Search) if on => {
                let items = self.search_items();
                // (Its shadow too, as the list shrinks or grows.)
                let reach = self.m.px(3.0);
                let grown = |r: Rect| Rect::new(r.x - reach, r.y - reach, r.w + 2 * reach, r.h + 2 * reach);
                let area = self.menu_rect().map(grown).unwrap_or_default();
                if let Some(menu) = self.menu.as_mut() {
                    menu.items = items;
                    menu.chosen = 0;
                    menu.first = 0;
                }
                let after = self.menu_rect().map(grown).unwrap_or_default();
                self.dirty = self.dirty.union(&area.union(&after));
            }
            Some(MenuKind::Search) => {
                self.close_menu();
                self.search_files.clear();
            }
            None if on => self.open_menu(MenuKind::Search),
            _ => {}
        }
    }

    /// The "/" list's items: the files whose names have the words typed after "/" (all of them, for "/" alone), the
    /// newest first, and last, asking Chat Computer to find it.
    fn search_items(&self) -> Vec<MenuItem> {
        let query = self.typed.get(1..).unwrap_or("").trim().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut items: Vec<MenuItem> = self
            .search_files
            .iter()
            .filter(|f| {
                let name = f.name.to_lowercase();
                words.iter().all(|w| name.contains(w))
            })
            .take(MENU_FILES)
            .map(|f| MenuItem {
                label: f.name.clone(),
                detail: format!("{}, {}", folder_of(&f.path), when_text(f.modified)),
                path: f.path.clone(),
                ticked: false,
            })
            .collect();
        let ask = if query.is_empty() { String::from("Ask Chat Computer to find something") } else { format!("Ask Chat Computer to find \u{201C}{query}\u{201D}") };
        items.push(MenuItem { label: ask, detail: String::from("it looks through all your files"), path: String::from(ASK_FIND), ticked: false });
        items
    }

    /// The talk key: listening starts (or, while it records, what was said so far is heard).
    fn talk(&mut self) {
        if let Err(why) = crate::listening::talk() {
            self.note_now(why);
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
        if (self.typed.trim().is_empty() && self.attached.is_empty()) || self.asking.is_some() {
            return;
        }
        // (Still starting: sent once the main loop has.)
        if !self.looping {
            self.send_later = true;
            return;
        }
        if !self.started {
            self.started = true;
            self.blocks.clear();
            self.welcome_fading = Some(time::uptime_ms());
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
            self.anchor = self.pin_top(self.blocks.len() - 1);
            self.scroll = None;
            // (On the screen before the unlocking starts, as the screen waits while it's done.)
            self.draw(true);
            return;
        }
        // A site's address alone ("google.com", "open en.wikipedia.org"): the site opens, in the middle.
        if self.attached.is_empty()
            && let Some(url) = browser::address::address(&self.typed)
        {
            let text = String::from(self.typed.trim());
            self.typed.clear();
            self.cursor = 0;
            return self.open_site(text, url);
        }
        // Anything else goes to Chat Computer: the site closes, so its answer shows.
        self.close_site();
        let mut text = String::from(self.typed.trim());
        self.typed.clear();
        self.cursor = 0;
        // The files attached: named with their paths after what's typed, for the model to read (read_file).
        if !self.attached.is_empty() {
            let paths: Vec<String> = core::mem::take(&mut self.attached).into_iter().map(|(p, _)| p).collect();
            if text.is_empty() {
                text = String::from("Have a look at this.");
            }
            text = format!("{text}\n\nAttached: {}", paths.join(", "));
        }
        if self.chat_title.is_none() {
            self.set_chat_title(Some(text.clone()));
        }
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
        self.anchor = self.pin_top(you);
        self.scroll = None;
        self.dirty = self.canvas.whole();
    }

    // ---------------------------------------------------------------------------------------------- websites

    /// A website's frame (as chat.css's #frame: from the top of the screen down to the message box, between the
    /// sidebars, 0.75 rem from each), its close button (x), and where its page shows (under the frame's bar, inside its
    /// border and clear of its bottom corners' curve).
    fn site_rects(&self) -> (Rect, Rect, Rect) {
        let m = self.m;
        let gap = m.px(0.75);
        let rule = ((2.0 * self.layout.scale + 0.5) as i32).max(1);
        let (left, right) = (m.mid_x + gap, m.mid_x + m.mid_w - gap);
        let (top, bottom) = (gap, (self.composer.card.y - gap).max(gap + m.px(6.0)));
        let frame = Rect::new(left, top, (right - left).max(0), bottom - top);
        let bar = m.px(2.4);
        let size = m.px(1.75);
        let close = Rect::new(right - rule - m.px(0.45) - size, top + rule + (bar - size) / 2, size, size);
        let inset = rule + m.px(0.4);
        let page = Rect::new(left + inset, top + rule + bar, (right - left - 2 * inset).max(0), (bottom - top - rule - bar - inset).max(0));
        (frame, close, page)
    }

    /// The website open, in its frame: a 2-pixel line round it (rounded 1.1 rem), its name in bold at the left of its
    /// bar and its close button at the right, and the page under them (or, until it comes, what's happening).
    fn render_site(&mut self, clip: Rect) {
        let (frame, close, page) = self.site_rects();
        let allow = self.allow_rect();
        let m = self.m;
        let rule = ((2.0 * self.layout.scale + 0.5) as i32).max(1);
        let radius = m.px(1.1);
        self.canvas.set_clip(clip);
        self.canvas.round_rect(frame, radius, LINE);
        self.canvas.round_rect(Rect::new(frame.x + rule, frame.y + rule, frame.w - 2 * rule, frame.h - 2 * rule), (radius - rule).max(0), GROUND);
        // The bar: the page's name, cut to fit before the x.
        let Some(site) = self.site.as_mut() else { return };
        let title = site.title();
        let size = m.rem * 0.8;
        let style = Style { size: fx(size), line_height: ((size * 1.2 + 0.5) as i32) * ONE };
        let bar_h = page.y - frame.y - rule;
        let x = frame.x + rule + m.px(1.1);
        let room = (allow.as_ref().map_or(close.x, |(r, _)| r.x) - m.px(0.75) - x).max(0);
        let mut shown = title.clone();
        while !shown.is_empty() && self.ty.width_in(&shown, Face::Bold, style.size) / ONE > room {
            shown.pop();
            while shown.ends_with(' ') {
                shown.pop();
            }
            if shown.chars().count() < title.chars().count() && !shown.is_empty() {
                // (Cut: an ellipsis where the rest was.)
                let mut cut = shown.clone();
                cut.push('\u{2026}');
                if self.ty.width_in(&cut, Face::Bold, style.size) / ONE <= room {
                    shown = cut;
                    break;
                }
            }
        }
        let ink = self.ty.ink_in(&shown, Face::Bold, style, 0);
        let ty_ = frame.y + rule + (bar_h - style.line_height / ONE) / 2;
        self.canvas.set_clip(clip.intersect(&Rect::new(frame.x, frame.y, frame.w, bar_h + rule)));
        self.canvas.ink(x - ink.left as i32, ty_, &ink, INK);
        // An app's Allow button: its words in bold on a pill (a ring round it when the keyboard has chosen it).
        if let Some((r, label)) = &allow {
            if self.pill == Some(Pill::Allow) {
                let g = rule + 1;
                self.canvas.round_rect(Rect::new(r.x - g, r.y - g, r.w + 2 * g, r.h + 2 * g), r.h / 2 + g, INK);
            }
            self.canvas.round_rect(*r, r.h / 2, CHOSEN);
            let ink = self.ty.ink_in(label, Face::Bold, style, 0);
            let lw = self.ty.width_in(label, Face::Bold, style.size) / ONE;
            self.canvas.set_clip(clip.intersect(r));
            self.canvas.ink(r.x + (r.w - lw) / 2 - ink.left as i32, r.y + (r.h - style.line_height / ONE) / 2, &ink, INK);
            self.canvas.set_clip(clip.intersect(&Rect::new(frame.x, frame.y, frame.w, bar_h + rule)));
            // (A test build says where it is, for the test to press it.)
            if option_env!("CHATCOMPUTER_TEST").is_some() && self.allow_logged != Some(*r) {
                self.allow_logged = Some(*r);
                crate::log!("Chat Computer kernel: site: allow button at {},{} {}x{}", r.x, r.y, r.w, r.h);
            }
        }
        // The x: two strokes across the button's middle.
        let (cx, cy) = ((close.x * 2 + close.w) * 4, (close.y * 2 + close.h) * 4);
        let half = m.px(0.32) * 8;
        let width8 = ((1.6 * self.layout.scale * 8.0) as i32).max(8);
        self.canvas.strokes(&[((cx - half, cy - half), (cx + half, cy + half)), ((cx - half, cy + half), (cx + half, cy - half))], width8, INK);
        // The page.
        let inside = clip.intersect(&page);
        if inside.is_empty() {
            return;
        }
        let dpr = self.layout.scale as f32;
        let (w, h) = (self.canvas.width, self.canvas.height);
        if site.has_page() {
            let (pixels, ty) = (self.canvas.pixels_mut(), &self.ty);
            site.draw(ty, pixels, w, h, (page.x, page.y, page.w, page.h), (inside.x, inside.y, inside.w, inside.h), dpr);
        } else {
            site.dirty = false;
            let note = site.note.clone().unwrap_or_default();
            self.canvas.set_clip(inside);
            let lines = self.ty.breaks(&note, m.body.size, (page.w - m.px(2.0)).max(1) * ONE);
            let mut y = page.y + (page.h - lines.len() as i32 * m.line) / 2;
            for r in lines {
                let part = String::from(note[r].trim_end());
                let ink = self.ty.ink_in(&part, Face::Regular, m.body, 0);
                let lw = self.ty.width(&part, m.body.size) / ONE;
                self.canvas.ink(page.x + (page.w - lw) / 2 - ink.left as i32, y, &ink, MUTED);
                y += m.line;
            }
        }
        // An app's question, over its page: what it asks to send, and where; Don't send and Send.
        let asked = self.send_box();
        if asked.is_none() && matches!(self.pill, Some(Pill::DontSend | Pill::Send)) {
            self.pill = None;
        }
        if let Some(b) = asked {
            if self.send_shown.map(|(i, _)| i) != Some(b.id) {
                self.send_shown = Some((b.id, time::uptime_ms()));
            }
            self.canvas.set_clip(inside);
            let r = m.px(0.8);
            self.canvas.round_rect(b.rect, r, BOX_LINE);
            self.canvas.round_rect(Rect::new(b.rect.x + rule, b.rect.y + rule, b.rect.w - 2 * rule, b.rect.h - 2 * rule), (r - rule).max(0), BOX_GROUND);
            let pad = m.px(1.0);
            self.canvas.set_clip(inside.intersect(&Rect::new(b.rect.x + pad, b.rect.y, b.rect.w - 2 * pad, b.rect.h)));
            for (words, face, y, colour) in &b.lines {
                let ink = self.ty.ink_in(words, *face, m.body, 0);
                self.canvas.ink(b.rect.x + pad - ink.left as i32, *y, &ink, *colour);
            }
            for (r, label, pill) in [(b.dont, "Don't send", Pill::DontSend), (b.send, "Send", Pill::Send)] {
                self.canvas.set_clip(inside);
                if self.pill == Some(pill) {
                    let g = rule + 1;
                    self.canvas.round_rect(Rect::new(r.x - g, r.y - g, r.w + 2 * g, r.h + 2 * g), r.h / 2 + g, INK);
                }
                self.canvas.round_rect(r, r.h / 2, CHOSEN);
                let ink = self.ty.ink_in(label, Face::Bold, style, 0);
                let lw = self.ty.width_in(label, Face::Bold, style.size) / ONE;
                self.canvas.set_clip(inside.intersect(&r));
                self.canvas.ink(r.x + (r.w - lw) / 2 - ink.left as i32, r.y + (r.h - style.line_height / ONE) / 2, &ink, INK);
            }
            // (A test build says where its buttons are, for the test to press them.)
            if option_env!("CHATCOMPUTER_TEST").is_some() && self.send_logged != Some((b.id, b.send)) {
                self.send_logged = Some((b.id, b.send));
                let (s, d) = (b.send, b.dont);
                crate::log!("Chat Computer kernel: site: send button at {},{} {}x{}; don't-send button at {},{} {}x{}", s.x, s.y, s.w, s.h, d.x, d.y, d.w, d.h);
            }
        }
        // (A test build says where the page is, for the test to click in it.)
        if option_env!("CHATCOMPUTER_TEST").is_some() && self.site_logged != Some(page) {
            self.site_logged = Some(page);
            crate::log!("Chat Computer kernel: site: page at {},{} {}x{}", page.x, page.y, page.w, page.h);
        }
    }

    /// A key, with a website open: typed into its field (when one has the caret), or scrolling it (the arrows and
    /// Page Up and Down, with nothing typed in the box), Alt+Left back to the page before, Tab to its first field, and
    /// Esc (with nothing typed) closing it. True if it was the site's.
    fn site_key(&mut self, p: Press) -> bool {
        if self.pill_key(p) {
            return true;
        }
        let (frame, _, page) = self.site_rects();
        let empty = self.typed.is_empty();
        let Some(s) = self.site.as_mut() else { return false };
        if s.focus.is_some() && s.key(p) {
            self.dirty = self.dirty.union(&frame);
            return true;
        }
        // (The page's scripts see it; one they cancel goes no further.)
        if s.focus.is_none() && !matches!(p.key, Key::Escape) && !(matches!(p.key, Key::Left) && p.alt) && s.page_key(p) {
            self.dirty = self.dirty.union(&frame);
            return true;
        }
        let line = 3.0 * self.m.line as f32;
        let screen = (page.h as f32 * 0.85).max(line);
        let moved = match p.key {
            Key::Left if p.alt => {
                if s.go_back() {
                    self.dirty = self.dirty.union(&frame);
                }
                return true;
            }
            Key::Tab => {
                s.tab(false);
                self.dirty = self.dirty.union(&frame);
                return true;
            }
            Key::Escape if empty => {
                self.close_site();
                return true;
            }
            Key::PageDown => s.scroll_by(screen),
            Key::PageUp => s.scroll_by(-screen),
            Key::Down if empty => s.scroll_by(line),
            Key::Up if empty => s.scroll_by(-line),
            Key::Char(' ') if empty && p.ctrl => s.scroll_by(screen),
            _ => return false,
        };
        if moved {
            self.dirty = self.dirty.union(&page);
        }
        true
    }

    /// Opens a site (an address typed): what was typed and what's said go in the conversation, as the website's do,
    /// and the site shows in its frame, in place of the conversation (in the one open, if one is).
    fn open_site(&mut self, text: String, url: web::Url) {
        if self.chat_title.is_none() {
            self.set_chat_title(Some(text.clone()));
        }
        let name = site::name_of(&url);
        self.blocks.push(Block::new(Kind::You, Live::No, text));
        let you = self.blocks.len() - 1;
        self.blocks.push(Block::new(
            Kind::Answer,
            Live::No,
            format!("Here's {name}. Click in it to type, and scroll it as you'd scroll any page."),
        ));
        let what = format!("Opened {}", url.host);
        crate::log!("Chat Computer kernel: site: {what}");
        self.action(what);
        match self.site.as_mut() {
            Some(s) => s.go(browser::Request { url, body: None }, None, true),
            None => self.site = Some(Site::open(url)),
        }
        self.lay_out_box();
        self.lay_out();
        self.anchor = self.pin_top(you);
        self.scroll = None;
        self.dirty = self.canvas.whole();
    }

    /// A file picked with "/": opened in the middle, as a website is (files::view makes a page of it), until it's
    /// closed; or why it can't be, under the conversation.
    fn open_file(&mut self, path: String) {
        match crate::files::view(&path) {
            Ok(v) => {
                let what = format!("Opened {}", v.name);
                crate::log!("Chat Computer kernel: site: {what}");
                self.action(what);
                if let Some(mut s) = self.site.take() {
                    s.stop();
                }
                // (Its address is the file's, on this PC: a link in it, if it has any, goes to the web from there.)
                let url = web::Url { host: String::from("this.pc"), port: 443, path: format!("/{}", path.replace('\\', "/")) };
                self.site = Some(Site::local(url, &v.html));
            }
            Err(why) => self.note_now(why),
        }
        self.lay_out_box();
        self.dirty = self.canvas.whole();
    }

    /// Opens an app the model made (or opens it again, changed): in the middle, as a website is, until it's closed.
    fn open_app(&mut self, name: String, html: String) {
        let what = format!("Opened the {name} app");
        crate::log!("Chat Computer kernel: site: {what}");
        self.action(what);
        self.show_app(&name, &html);
    }

    /// An app shown in the middle, reaching the sites it needs that the person allowed it.
    fn show_app(&mut self, name: &str, html: &str) {
        if let Some(mut s) = self.site.take() {
            s.stop();
        }
        self.pill = None;
        let (sites, allowed, _) = assistant::app_sites(name);
        self.site = Some(Site::app(name, html, &sites, &allowed));
        self.lay_out_box();
        self.dirty = self.canvas.whole();
    }

    /// The Allow button over an app that needs sites the person hasn't allowed it yet: before the close button and as
    /// tall, "Allow api.example.com" (or "Allow 3 sites").
    fn allow_rect(&self) -> Option<(Rect, String)> {
        let pending = self.site.as_ref()?.app_pending();
        let label = match pending {
            [] => return None,
            [one] => format!("Allow {one}"),
            more => format!("Allow {} sites", more.len()),
        };
        let (frame, close, _) = self.site_rects();
        let size = fx(self.m.rem * 0.8);
        let w = (self.ty.width_in(&label, Face::Bold, size) / ONE + self.m.px(1.6)).min((close.x - frame.x) / 2);
        Some((Rect::new(close.x - self.m.px(0.6) - w, close.y, w, close.h), label))
    }

    /// The question over an app that asks to send something, if one does: a box at the top of its page (at most 40
    /// rem wide), "The Notes app asks to send this to api.example.com/note:" in bold, what it sends under that (at
    /// most SEND_LINES lines, and as many as the page has room for), and its two buttons at its bottom right, Don't
    /// send and Send, as the Allow button is.
    fn send_box(&self) -> Option<SendBox> {
        let ask = self.site.as_ref()?.send_asked()?;
        let (_, close, page) = self.site_rects();
        let m = self.m;
        let (margin, pad) = (m.px(1.0), m.px(1.0));
        let w = (page.w - 2 * margin).min(m.px(40.0)).max(m.px(12.0)).min(page.w);
        let x = page.x + (page.w - w) / 2;
        let y = page.y + margin.min(page.h / 8);
        let tw = (w - 2 * pad).max(1) * ONE;
        let mut lines = Vec::new();
        let mut at = y + pad;
        let mut head = if ask.deletes {
            format!("The {} app asks to delete {}.", ask.app, ask.to)
        } else {
            format!("The {} app asks to send this to {}:", ask.app, ask.to)
        };
        if ask.more > 0 {
            head.push_str(&format!(" ({} more after it.)", ask.more));
        }
        let chars: Vec<(char, Face)> = head.chars().map(|c| (c, Face::Bold)).collect();
        let rs = self.ty.breaks_styled(&chars, m.body.size, tw);
        for (i, r) in rs.iter().take(4).enumerate() {
            let mut line: String = chars[r.clone()].iter().map(|(c, _)| *c).collect();
            if i == 3 && rs.len() > 4 {
                line.push('\u{2026}');
            }
            lines.push((line, Face::Bold, at, INK));
            at += m.line;
        }
        // What it sends, in as many lines as there's room for.
        let button_h = close.h;
        let room = ((page.bottom() - at - m.px(0.4) - m.px(0.8) - button_h - pad - margin) / m.line.max(1)).max(1) as usize;
        let most = SEND_LINES.min(room);
        let (what, colour) = if !ask.what.is_empty() {
            (ask.what.as_str(), INK)
        } else if ask.deletes {
            ("", INK)
        } else {
            ("Nothing but its address.", MUTED)
        };
        if !what.is_empty() {
            at += m.px(0.4);
            let rs = self.ty.breaks(what, m.body.size, tw);
            let cut = rs.len() > most;
            for r in rs.iter().take(if cut { most.saturating_sub(1).max(1) } else { most }) {
                lines.push((String::from(what[r.clone()].trim_end()), Face::Regular, at, colour));
                at += m.line;
            }
            if cut {
                let shown = most.saturating_sub(1).max(1);
                lines.push((format!("(and {} lines more)", rs.len() - shown), Face::Regular, at, MUTED));
                at += m.line;
            }
        }
        at += m.px(0.8);
        let size = fx(m.rem * 0.8);
        let send_w = self.ty.width_in("Send", Face::Bold, size) / ONE + m.px(2.4);
        let dont_w = self.ty.width_in("Don't send", Face::Bold, size) / ONE + m.px(1.6);
        let send = Rect::new(x + w - pad - send_w, at, send_w, button_h);
        let dont = Rect::new(send.x - m.px(0.6) - dont_w, at, dont_w, button_h);
        at += button_h + pad;
        Some(SendBox { id: ask.id, rect: Rect::new(x, y, w, at - y), lines, dont, send })
    }

    /// The keyboard's way to the buttons over an app, for a person without a pointer (the model can't press keys, nor
    /// can a page's scripts: they're the person's). With a question showing: Tab chooses Don't send, then Send, and
    /// round again; Enter presses the one chosen (nothing, until one is); Esc is Don't send. With an Allow button:
    /// Tab chooses it (when no field on the page has the caret: from it, Tab goes on to the page's first field),
    /// Enter presses it, Esc or any other key lets it go. True if the key was the buttons'.
    fn pill_key(&mut self, p: Press) -> bool {
        let (frame, _, _) = self.site_rects();
        if let Some(b) = self.send_box() {
            match p.key {
                Key::Tab => {
                    self.pill = Some(if self.pill == Some(Pill::DontSend) { Pill::Send } else { Pill::DontSend });
                    self.dirty = self.dirty.union(&frame);
                    return true;
                }
                Key::Enter if matches!(self.pill, Some(Pill::DontSend | Pill::Send)) => {
                    let send = self.pill == Some(Pill::Send);
                    self.answer_send(b.id, send);
                    return true;
                }
                Key::Escape => {
                    self.answer_send(b.id, false);
                    return true;
                }
                _ => return false,
            }
        }
        if self.allow_rect().is_none() {
            self.pill = None;
            return false;
        }
        let field = self.site.as_ref().is_some_and(|s| s.focus.is_some());
        match (p.key, self.pill) {
            (Key::Tab, None) if !field => self.pill = Some(Pill::Allow),
            (Key::Enter, Some(Pill::Allow)) => {
                self.pill = None;
                self.allow_app();
            }
            (Key::Escape, Some(Pill::Allow)) => self.pill = None,
            (_, Some(_)) => {
                // (Tab goes on to the page's first field, and anything else where it would have gone.)
                self.pill = None;
                self.dirty = self.dirty.union(&frame);
                return false;
            }
            _ => return false,
        }
        self.dirty = self.dirty.union(&frame);
        true
    }

    /// The person pressed Send or Don't send over an app that asks to send something (once its question has been
    /// shown SEND_WAIT_MS): it's sent or not, and the Actions sidebar says which.
    fn answer_send(&mut self, id: u32, send: bool) {
        let ready = self.send_shown.is_some_and(|(i, at)| i == id && time::uptime_ms().saturating_sub(at) >= SEND_WAIT_MS);
        if !ready {
            return;
        }
        if let Some(s) = self.site.as_mut() {
            s.answer_send(send);
        }
        self.pill = None;
        for what in self.site.as_mut().map(|s| s.take_actions()).unwrap_or_default() {
            self.action(what);
        }
        let (frame, _, _) = self.site_rects();
        self.dirty = self.dirty.union(&frame);
    }

    /// The person pressed an app's Allow button: it reaches the sites it needs from now on (opened again, so what it
    /// fetches as it starts comes). The Actions sidebar says so, with what they'd asked it for.
    fn allow_app(&mut self) {
        let Some(name) = self.site.as_ref().and_then(|s| s.app_name()).map(String::from) else { return };
        let allowed = assistant::allow_app(&name);
        let (_, _, because) = assistant::app_sites(&name);
        let asked: String = because.chars().take(80).collect();
        let what = if asked.is_empty() {
            format!("You allowed the {name} app to reach {}", allowed.join(", "))
        } else {
            format!("You allowed the {name} app (for \u{201c}{asked}\u{201d}) to reach {}", allowed.join(", "))
        };
        crate::log!("Chat Computer kernel: site: {what}");
        self.action(what);
        if let Some(html) = assistant::app(&name) {
            self.show_app(&name, &html);
        }
    }

    /// The apps there are (their names), for the Apps list.
    fn set_apps(&mut self, apps: Vec<String>) {
        if self.apps != apps {
            self.apps = apps;
            let h = self.screen.height as i32;
            self.dirty = self.dirty.union(&Rect::new(0, 0, self.m.side_w, h));
        }
    }

    /// Closes the website open: the conversation shows again.
    fn close_site(&mut self) {
        self.pill = None;
        if let Some(mut s) = self.site.take() {
            s.stop();
            crate::log!("Chat Computer kernel: site: closed");
            self.site_logged = None;
            self.dirty = self.canvas.whole();
        }
    }

    /// Where the screen's top goes for block `i` pinned to it: 2 rem above it, but never so far up that any of what
    /// came before shows (as the app's pinTop).
    fn pin_top(&self, i: usize) -> i32 {
        let at = self.blocks[i].top;
        let top = at - self.m.px(2.0);
        match i.checked_sub(1).map(|p| &self.blocks[p]) {
            Some(b) => top.max((b.top + b.height).min(at)),
            None => top,
        }
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
                // (What it does, for the Actions sidebar: not how far it's got.)
                if !doing.contains('%') && !doing.starts_with("Searching") && !doing.starts_with("Reading ") {
                    self.action(doing.clone());
                }
                let Some(i) = self.asking else { return };
                self.blocks[i].activity = Some(doing);
                let area = self.block_area(i);
                self.dirty = self.dirty.union(&area);
            }
            Event::Picture(p) => self.show_picture(p),
            Event::App(name, html) => self.open_app(name, html),
            Event::Apps(names) => self.set_apps(names),
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
                self.set_chat_title(None);
            }
            Event::Note(text) => {
                self.blocks.push(Block::new(Kind::Note, Live::No, text));
                self.relayout = true;
            }
            Event::Looked(text) => {
                self.action(text.clone());
                self.looked(text);
            }
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
        if let Some((_, first)) = turns.iter().find(|(you, _)| *you) {
            self.set_chat_title(Some(first.clone()));
        }
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

/// The main loop has started: keys are taken as they come; a message sent before then goes now.
pub fn looping() {
    let apps = assistant::apps();
    if let Some(c) = CHAT.lock().as_mut() {
        c.looping = true;
        c.apps = apps;
        if core::mem::take(&mut c.send_later) {
            c.send();
        }
        c.box_changed();
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
        // A website open: it scrolls.
        if c.site.is_some() {
            let (_, _, page) = c.site_rects();
            if c.site.as_mut().is_some_and(|s| s.scroll_by(notches.saturating_mul(lines).saturating_neg() as f32)) {
                c.dirty = c.dirty.union(&page);
            }
            return;
        }
        c.scroll_by(notches.saturating_mul(lines).saturating_neg());
    }
}

/// The pointer moved (it's at `at`), or its left button was pressed at `click`.
pub fn pointer(at: (f32, f32), click: Option<(f32, f32)>) {
    if let Some(c) = CHAT.lock().as_mut() {
        if let Some(p) = click {
            c.pointer_at(p, true);
        }
        c.pointer_at(at, false);
        c.draw(false);
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

/// "2.5 MB", "12 KB": a size, for the lists above the box.
fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1e6),
        b if b >= 1_000 => format!("{} KB", b / 1_000),
        b => format!("{b} bytes"),
    }
}

/// A model's name from its file's: "...\\qwen3.5-4b.ccm" is "Qwen3.5 4B".
/// A sidebar's switch, as the website's (chat.html's toggle icons): the screen with its sidebar marked, filled in while
/// the sidebar shows, lighter under the pointer. `k`: 0 Messages' (marked at the left), 1 Actions' (at the right).
fn draw_switch(canvas: &mut Canvas, r: Rect, k: usize, shown: bool, lit: bool, m: &Metrics, ground: u32) {
    if lit {
        canvas.round_rect(r, m.px(0.4), CHOSEN);
    } else {
        canvas.fill(r, ground);
    }
    // The icon's 24 units, in the button less 0.15 rem round it, in 1/8 pixel.
    let pad = m.px(0.15);
    let unit8 = (r.w - 2 * pad) * 8;
    let (x8, y8) = ((r.x + pad) * 8, (r.y + pad) * 8);
    let p = |a: f32, b: f32| -> Point { (x8 + (a * unit8 as f32 / 24.0) as i32, y8 + (b * unit8 as f32 / 24.0) as i32) };
    let px = |a: f32| (r.x + pad) as f32 + a * (r.w - 2 * pad) as f32 / 24.0;
    let py = |b: f32| (r.y + pad) as f32 + b * (r.w - 2 * pad) as f32 / 24.0;
    // The sidebar's part, filled while it shows: 6 units of the 18, its outer corners round.
    let (from, to) = if k == 0 { (3.0f32, 9.0f32) } else { (15.0f32, 21.0f32) };
    if shown {
        let (x0, x1, y0, y1) = (px(from) as i32, px(to) as i32, py(5.0) as i32, py(19.0) as i32);
        let radius = (3.5 * (r.w - 2 * pad) as f32 / 24.0) as i32;
        canvas.round_rect(Rect::new(x0, y0, x1 - x0, y1 - y0), radius, INK);
        // (Its inner side square.)
        let half = (x1 - x0) / 2;
        let square = if k == 0 { Rect::new(x0 + half, y0, x1 - x0 - half, y1 - y0) } else { Rect::new(x0, y0, x1 - x0 - half, y1 - y0) };
        canvas.fill(square, INK);
    }
    // The screen: a rectangle 18 by 14 with corners of 3.5, and the line at the sidebar's edge.
    let mut segs: Vec<(Point, Point)> = Vec::new();
    let mut outline: Vec<(f32, f32)> = Vec::new();
    let corner = |cx: f32, cy: f32, a0: f32, out: &mut Vec<(f32, f32)>| {
        for i in 0..=6 {
            let a = (a0 + i as f32 * 15.0) as f64 * core::f64::consts::PI / 180.0;
            let (sn, cs) = model::math::sin_cos(a);
            out.push((cx + 3.5 * cs as f32, cy + 3.5 * sn as f32));
        }
    };
    corner(6.5, 8.5, 180.0, &mut outline);
    corner(17.5, 8.5, 270.0, &mut outline);
    corner(17.5, 15.5, 0.0, &mut outline);
    corner(6.5, 15.5, 90.0, &mut outline);
    for w in outline.windows(2) {
        segs.push((p(w[0].0, w[0].1), p(w[1].0, w[1].1)));
    }
    if let (Some(first), Some(last)) = (outline.first(), outline.last()) {
        segs.push((p(last.0, last.1), p(first.0, first.1)));
    }
    let line_x = if k == 0 { 9.0 } else { 15.0 };
    segs.push((p(line_x, 5.0), p(line_x, 19.0)));
    canvas.strokes(&segs, unit8 * 2 / 24, INK);
}

/// The most files "/" searches among (the newest on this PC: Desktop, Documents, Downloads, Pictures).
const SEARCHED: usize = 2000;

/// When a file was changed, short, as the website's list has it: the time today ("3:56 pm"), else the day
/// ("19 Oct"), with the year when it isn't this year's.
fn when_text(t: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let offset = crate::clock::local_now().zip(crate::clock::now()).map_or(0, |(l, n)| l - n);
    let local = t + offset;
    let (y, mo, d, h, mi) = crate::clock::civil(local);
    let now = crate::clock::local_now().or_else(crate::clock::now);
    let today = now.map(|n| crate::clock::civil(n));
    match today {
        Some((ty, tm, td, _, _)) if (ty, tm, td) == (y, mo, d) => {
            let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "am" } else { "pm" });
            format!("{h12}:{mi:02} {half}")
        }
        Some((ty, _, _, _, _)) if ty == y => format!("{d} {}", MONTHS[(mo as usize).clamp(1, 12) - 1]),
        _ => format!("{d} {} {y}", MONTHS[(mo as usize).clamp(1, 12) - 1]),
    }
}

/// The folder a file's in, as its path names it, the drive and Users\NAME left out ("Documents\Work").
fn folder_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('\\').collect();
    let folder = &parts[..parts.len().saturating_sub(1)];
    // (C:, Users, NAME, then the folders.)
    let start = if folder.get(1).is_some_and(|p| p.eq_ignore_ascii_case("users")) { 3 } else { 1 };
    let shown = folder.get(start..).map(|f| f.join("\\")).unwrap_or_default();
    if shown.is_empty() { String::from(folder.join("\\").as_str()) } else { shown }
}

fn model_label(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".ccm").unwrap_or(file);
    match stem.split_once('-') {
        Some((family, size)) => {
            let family = match family.strip_prefix("qwen") {
                Some(rest) => format!("Qwen{rest}"),
                None => String::from(family),
            };
            format!("{family} {}", size.to_ascii_uppercase())
        }
        None => String::from(stem),
    }
}

/// The colour `a` mixed with `b`: `t` of b.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| -> u32 {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        ((x + (y - x) * t) + 0.5) as u32 & 0xFF
    };
    ch(16) << 16 | ch(8) << 8 | ch(0)
}
