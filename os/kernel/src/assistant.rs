//! Chat Computer's own model, answering on this PC's own processor: an open model of Qwen's (published by Alibaba
//! under the Apache 2.0 licence), run by Chat Computer's own engine (os/model), from its file on Windows' drive: the
//! one Chat Computer's app got ready there (or the one the settings name), read into memory at start-up. Nothing
//! anyone types leaves this PC, and nothing is paid for.
//!
//! The model answers a step at a time: part of what it's asked read (64 tokens at once, so each of its weights read
//! from memory serves many: what makes reading fast), or one token written, a step. Each step runs on the processor's
//! other cores (cores::away: the second core takes it, and shares its arithmetic with the rest), the first core going
//! on with its loop meanwhile, so the screen, the mouse and the keyboard never wait for the model. The main loop looks
//! for the step's end, takes what came of it, and sends the next. With one core, a step is done in the loop itself,
//! kept short.
//!
//! Before anything's asked, it reads ahead what the next question will start with: its instructions (the system
//! prompt, with its tools), the conversation so far (after a restart, the one kept from before), and the words being
//! typed in the message box as far as they can't change (all but the last word), so a question sent has little left
//! to read. Each message it's given starts with when it was sent.
//!
//! It acts on this computer through its tools (tools.json): it sees how the computer's doing, changes the text size,
//! sets timers, reminders and alarms, reads the files on its drives (and sees their pictures, scanned pages among
//! them, when its file has the vision part: a picture a tool gives it goes with what the tool said, and its vision
//! part looks at it a step at a time), shows a picture, notes things about the person,
//! starts a new conversation, restarts the computer or turns it off. When an answer calls them, they run here and what
//! they say goes back to the model, until it has finished. The conversation is kept across restarts in Chat
//! Computer's memory file (keep.rs), with the model's notes about the person, which it's shown in every conversation.
//!
//! Or Claude answers, through Anthropic's API with the person's own key (remote.rs), when they choose it with the
//! spark beside + (chat.rs): the same conversation, tools and notes; its own model, effort and key kept in the memory
//! file with the conversation (the key never logged, nor shown). Chat Computer's own model stays the default.
//!
//! A test build made with CHATCOMPUTER_TEST=chat (by check.py) answers from a script instead (check.py's, built in:
//! what its tests have the tools do), so the screen, the tools and the memory file are checked without a model; it
//! logs what the model would have been asked and what the tools said. One made with CHATCOMPUTER_TEST=model answers
//! with the model, always with its likeliest token, and logs what it was asked and said, for check.py --model to
//! compare with what the engine says to the same on Linux.

use crate::sync::SpinLock;
use crate::{clock, files, heap, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use model::chat::{Answer, Call, Content, Message, Options, Picture, Said as Saying, Style};
use model::cpu::Level;
use model::json::{self, Json};
use model::refusal::Found;
use model::sample::Sampling;
use model::session::{Ahead, Ended, Request, Session, Step, Turn};
use model::{File, Model};

/// What came of the question.
pub enum Event {
    /// More of the answer.
    Text(String),
    /// The answer is complete.
    Done,
    /// It failed: why, for the person.
    Failed(String),
    /// The model set the text size (percent of the size chosen for this screen).
    TextSize(u32),
    /// The PC restarts in this many seconds.
    Restarting(u64),
    /// The PC turns off in this many seconds.
    TurningOff(u64),
    /// The model noted something about the person (its words), or forgot this many notes.
    Noted(String),
    Forgot(usize),
    /// What it's doing before it answers ("Reading a file").
    Activity(String),
    /// A picture the model shows the person (show_picture), made to fit the conversation's column.
    Picture(picture::Picture),
    /// The conversation so far is over: the next question starts a new one.
    NewConversation,
    /// A line from Chat Computer itself under the answer: that it was cut short, or that the model's tools were
    /// used up.
    Note(String),
    /// What the model looked up on the web (a search, or a site read): a line where it happens in the answer, so the
    /// person sees what left the computer.
    Looked(String),
    /// An app the model made (or changed, or was asked to open): its name and its page, to open in the middle.
    App(String, String),
    /// The apps there are now (their names), for the Apps list.
    Apps(Vec<String>),
}

/// An app the model made for the person (make_app): its name and its page (one HTML file); the sites it says it
/// needs, and those of them the person let it reach (it reaches no others); and what the person asked that it was
/// made (or last changed) for, which its actions are traced to.
#[derive(Clone, PartialEq)]
pub struct App {
    pub name: String,
    pub html: String,
    pub sites: Vec<String>,
    pub allowed: Vec<String>,
    pub because: String,
}

/// The most sites an app may say it needs.
const MOST_APP_SITES: usize = 8;

/// A site an app says it needs, as a host name ("api.example.com", from "https://api.example.com/v1" too), if
/// it's one.
fn app_site(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let host = s.split(['/', '?', '#']).next().unwrap_or("").trim_end_matches('.').to_ascii_lowercase();
    let ok = host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-') && !l.ends_with('-'));
    ok.then_some(host)
}

/// Whether the model can be asked now.
#[derive(Clone, PartialEq, Eq)]
pub enum Ready {
    /// It answers: its name.
    Yes(String),
    /// It's getting ready: how far, for the person.
    Getting(String),
    /// It can't answer: why, and what to do, for the person.
    No(String),
}

/// Said when there's no model.
pub const NO_MODEL: &str = "Chat Computer's own system needs its model, which it gets on Windows: go back there (restart the PC) \
and double-click Chat Computer Own System, which gets the model ready, then starts Chat Computer again.";

/// Said when Windows' drive, where the model is, is locked.
const LOCKED: &str = "Windows' drive is locked with BitLocker, and Chat Computer's model is on it: type the drive's recovery \
key below to unlock it (48 digits, which Windows keeps in your Microsoft account: https://aka.ms/myrecoverykey).";

/// The tools, as the model's template writes them into its instructions (OpenAI's form).
const TOOLS: &str = include_str!("tools.json");

const SYSTEM: &str = "You are Chat Computer: you run this computer for the person using it, who talks to you by typing \
here. You are an AI model running on this computer itself ({model}, an open model made by Qwen, run by Chat Computer's \
own engine on Chat Computer's own operating system), so you work without the internet, and nothing they say leaves the \
computer unless you look something up on the web, which they see.

With your tools you can see how this computer is doing, change the text size, set timers, reminders and alarms (and \
how loud they ring), read the files on its drives (Windows' C: and the others, USB sticks and memory cards included; \
nothing can be changed){sees}, search the web and read web pages (when this computer is connected), show the person a \
picture, note things about them to remember in later conversations, make them small apps they'll use again (a budget, \
a log, a planner) and open those, start a new conversation, and restart this computer (to go back to Windows) or turn \
it off. Use a tool rather than saying \
how; don't say you're about to use one, just use it.

Each message from the person starts with when they sent it, like [Tue 3 Nov 2026 19:02]. Answer in their language, \
briefly unless they ask for more. The screen shows Markdown (headings, bold and italic, lists, code, quotes and simple \
tables) but no links, and no pictures but the ones you show with show_picture, so write an address out in full. Don't \
make things up: say so when you don't know.";
/// Said when the oldest part of a long conversation is no longer given to the model.
const SHORTENED: &str = "\n\nThis conversation began earlier: its oldest part is no longer shown to you, to keep it short \
enough for you, but the person can still scroll back to it.";

/// Bigger models answered too slowly when Chat Computer's own system ran the model on one processor core (it uses every
/// core now, not yet measured on a PC): of the models Chat Computer's app got ready, the biggest whose file is at most
/// this is chosen (the smallest, if none is).
const ONE_CORE_MOST: u64 = 3_600_000_000;
/// What the model needs beyond its file, at the least: its state, a few snapshots of it, what reading needs.
const RUN_ROOM: u64 = 256 << 20;
/// The longest conversation given to the model, in tokens, whatever the memory (past it, its oldest part is left out),
/// and the shortest it may be.
const MOST_TOKENS: usize = 16_384;
const LEAST_TOKENS: usize = 2_048;
/// The most one round of an answer says, in tokens (a model test build's: a few, for the same reason as its tools).
const ROUND_TOKENS: usize = 2_048;
const TEST_ROUND_TOKENS: usize = 24;
/// Rounds of tool calls in one answer, at most. What the last of them says back ends by telling the model so
/// (last_round), so that the round after it is its answer, in words: tools it calls then aren't run, and what it did
/// and found stays in the conversation, for the next message to carry on from, rather than the whole question
/// being lost when it would rather go on using them.
const TOOL_ROUNDS: u32 = 8;
/// Told the model after what its last round of tools said.
fn last_round() -> String {
    format!(
        "(That was the last of the {TOOL_ROUNDS} rounds of tools you can use for one message: now answer the person in \
         words, with what you found, and say what you couldn't find out or do. Tools called now aren't run.)"
    )
}
/// What a tool called after the last round says back (not run), and the line under that answer.
const NOT_RUN: &str = "Not run: you had used all your rounds of tools for this message. Answer with what you found.";
const USED_UP: &str = "Chat Computer's model had used its tools as many times as it can for one message, so it \
stopped there: ask it to carry on.";
/// What a tool call the model was still writing when its round reached the most it says (ROUND_TOKENS) says back:
/// its arguments may be cut short, so it isn't run.
const CUT_CALL: &str = "Not run: you reached the most you can write at once while you were still writing this call, so \
it may be cut short. Call it again, shorter.";
/// The line under an answer that reached the most a round says.
const CUT_SHORT: &str = "Chat Computer's answer stops here: that's the most its model writes at once. Ask it to go on.";
/// About how long a step of work done in the main loop takes: the screen and the keyboard go on between steps.
const STEP_MS: u64 = 40;
/// About how long a step of the model's sent to the other cores takes (the main loop goes on meanwhile; a step that
/// long is still soon stopped): a picture's patches a step are suited to it.
const AWAY_MS: u64 = 300;
/// Tokens of a prompt read a step, on the other cores: a whole batch of the engine's at least (each weight read from
/// memory serving all of them), and more while steps are quick.
const READ_LEAST: usize = model::net::BATCH;
const READ_MOST: usize = 4 * model::net::BATCH;
/// With no model found, looking again this often (Windows' drive may be unlocked meanwhile).
const LOOK_AGAIN_MS: u64 = 3_000;
/// Reading the model's file: how much a read takes to start with, and at most (it suits itself to the drive).
const FIRST_CHUNK: usize = 1 << 20;
const LEAST_CHUNK: usize = 64 << 10;
const MOST_CHUNK: usize = 32 << 20;
/// The biggest file of the refusal direction beside the model's (it's a few hundred kilobytes).
const MOST_REFUSAL: u64 = 64 << 20;
/// How many characters of a test script's answer are said a step.
const SCRIPT_PIECE: usize = 7;

/// The most notes the model keeps, the longest one, and the most of them all together, in characters.
const MAX_NOTES: usize = 40;
const NOTE_CHARS: usize = 400;
const NOTES_CHARS: usize = 8000;

fn test_build() -> Option<&'static str> {
    option_env!("CHATCOMPUTER_TEST")
}

/// A test build's script (CHATCOMPUTER_TEST=chat; empty in every other build: kernel/build.rs): JSON, each question
/// (or "else") with the rounds of its answer, each round {"text": words, "calls": [[tool, {arguments}], ...]} (with
/// "cut": true, a round that reached the most a round says) or {"fail": why}.
static TEST_SCRIPT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_script.json"));

/// The most rows a picture the model sees becomes for it (about a million pixels: a photo of 1,152 by 864, a page of
/// 832 by 1,216), each read as a token is.
const SEEN_ROWS: usize = 1024;

/// How the model sees a picture: by its vision part, or (a test's script) as Qwen 3.5's does.
#[derive(Clone, Copy)]
enum Sight {
    Model(&'static model::vision::Vision<'static>),
    Qwen,
    /// Claude: at most 1,568 pixels a side and about 1.15 million in all (Anthropic's advice for its vision).
    Claude,
}

impl Sight {
    /// The size (width, height) it reads a picture of `w` by `h` at, with at most SEEN_ROWS rows.
    fn size(self, w: usize, h: usize) -> (usize, usize) {
        let (h, w) = match self {
            Sight::Model(v) => v.size_for(h, w, SEEN_ROWS),
            Sight::Qwen => model::vision::smart_resize(h, w, 32, 65_536, SEEN_ROWS * 32 * 32),
            Sight::Claude => {
                let (mut w, mut h) = (w.max(1), h.max(1));
                let long = w.max(h);
                if long > 1568 {
                    (w, h) = ((w * 1568 / long).max(1), (h * 1568 / long).max(1));
                }
                while w * h > 1_150_000 {
                    (w, h) = ((w * 9 / 10).max(1), (h * 9 / 10).max(1));
                }
                (h, w)
            }
        };
        (w, h)
    }
}

/// A part of the conversation.
#[derive(Clone)]
enum Part {
    /// What the person typed, and when (seconds since 1970, when the clock was known).
    You { text: String, at: Option<i64> },
    /// A round of the model's answer: its words, and the tools it called; and, when Claude gave it, its content
    /// blocks as they came (a JSON array: its thinking with them, which the API asks to have back).
    Said(Answer, Option<Arc<String>>),
    /// What a tool said back, and a picture it gave the model to see (read_file's).
    Tool(String, Option<Arc<Picture>>),
}

/// The model's file, being read into memory.
struct Reading {
    file: files::Opened,
    name: String,
    bytes: Vec<u8>,
    /// How much a read takes now.
    chunk: usize,
    started: u64,
}

/// The model, ready.
struct Loaded {
    name: String,
    /// Where its file is.
    path: String,
    /// The conversation it has read (not touched while one of its steps is away on the other cores: `Assistant::away`).
    session: Session<'static>,
    /// The model itself (the session's), for what's asked of it meanwhile.
    model: &'static Model<'static>,
    style_name: String,
    /// Its refusals are taken out (its .refusal file beside it).
    uncensored: bool,
    /// The longest conversation it's given, in tokens.
    context: usize,
    /// What's read ahead of the next question (Session::prompt_ahead's: the instructions, the conversation so far, and
    /// the words typed so far that can't change), what it was made from, how much of it is read, and whether it's all
    /// read; whether the instructions were (said in the log once).
    ahead: Option<Arc<Ahead>>,
    ahead_of: Option<AheadOf>,
    ahead_done: usize,
    ahead_read: bool,
    instructions_read: bool,
    /// How many tokens a reading step takes, and how many of a picture's patches a step of seeing it (each suited
    /// to the processor's speed).
    read: usize,
    see: usize,
    /// Tokens written a second, in its last answer.
    speed: Option<f64>,
}

/// What reading ahead was made from: the conversation as far as it's given (how many parts, from where, with which
/// instructions), the words typed that can't change, and the minute the message would be sent in.
#[derive(Clone, PartialEq, Eq)]
struct AheadOf {
    parts: usize,
    from: usize,
    system: u64,
    words: String,
    minute: i64,
}

impl Loaded {
    /// The instructions' reading ahead, while it isn't done: how far it's got, of how far.
    fn instructions_left(&self) -> Option<(usize, usize)> {
        let a = self.ahead.as_ref()?.anchor?;
        (!self.instructions_read && self.ahead_done < a).then_some((self.ahead_done, a))
    }
}

/// One of the model's steps, away on the other cores: what it's for (reading ahead, made from `of`; or the answer
/// `job`'s turn's step), and what comes of it.
struct Away {
    errand: crate::cores::Errand<Back>,
    of: Option<AheadOf>,
    job: Option<u64>,
}

/// What came of a step away: a turn's step (whether the turn was reading, how much it had read before, whether it's
/// over after it), or what's read ahead (worked out anew, if it was), how much of it is read, and whether it's all
/// read; and how long it took.
enum Back {
    Turn { step: Step, reading: bool, before: usize, over: bool, took: u64 },
    Ahead { ahead: Option<Arc<Ahead>>, done: usize, all: bool, took: u64 },
}

/// A pointer lent to the other cores with a step (what it points to is touched by nothing else until the step's back).
struct Lent<T>(*mut T);

unsafe impl<T> Send for Lent<T> {}

impl<T> Lent<T> {
    /// (A method, so a closure takes the whole of it, Send, rather than its pointer.)
    fn get(self) -> *mut T {
        self.0
    }
}

/// The model, as far as it's got.
enum Brain {
    /// No model: why, whether to look again (a drive may be unlocked, or the model got ready), and since when.
    None { why: String, again: bool, since: u64 },
    Reading(Box<Reading>),
    Ready(Box<Loaded>),
    /// A test build's script, in place of a model.
    Script(Json),
}

/// A question being answered.
struct Job {
    /// Which question this is (each its own number), and where it is in the conversation (a failed or stopped
    /// question is taken back out).
    id: u64,
    base: usize,
    rounds: u32,
    /// Whether any words have been shown, and whether the next ones start a new paragraph (after tools ran).
    said: bool,
    separate: bool,
    /// Restart the PC, or turn it off, once the answer is over.
    restart: bool,
    turn_off: bool,
    /// Start a new conversation once the answer is over.
    new_conversation: bool,
    /// The model's turn in the round being answered, once it's begun (where it is in memory stays the same while one
    /// of its steps is away).
    turn: Option<Box<Turn>>,
    /// How many of the round's tool calls the model finished writing before the round was over (one it was still
    /// writing when the round reached the most it says is read as far as it got: it isn't run).
    closed: usize,
    /// A test script's round: its words still to say, the answer they're part of, and whether the round reached the
    /// most a round says.
    script: Option<(String, Answer, bool)>,
    /// What it has said (the log has it only in a test build), and the activity last shown.
    text: String,
    shown: String,
    /// Tokens read and written, pictures seen, and the time each took, for the log.
    read: usize,
    read_ms: u64,
    written: usize,
    write_ms: u64,
    seen: usize,
    see_ms: u64,
    /// The round's web tools' calls still being fetched, in order: the next round waits for them.
    web: Vec<WebCall>,
    /// Claude's answer to the round, on its way (when Claude answers), and its content blocks once whole, for the
    /// conversation.
    call: Option<crate::remote::Call>,
    raw: Option<Arc<String>>,
    /// Claude answered (not Chat Computer's own model): which.
    by: Option<String>,
}

/// One of the round's calls of the web's tools, being fetched: what it was, and where its result goes in the
/// conversation (a place kept for it, in order with the round's other tools).
struct WebCall {
    part: usize,
    what: WebWhat,
    fetch: crate::web::Fetch,
    begun: bool,
}

enum WebWhat {
    Search(String),
    Read { url: String, offset: usize },
}

impl WebCall {
    fn name(&self) -> &'static str {
        match self.what {
            WebWhat::Search(_) => "search_web",
            WebWhat::Read { .. } => "read_web_page",
        }
    }
}

/// A page's words given to the model at a time, at most (read_web_page's offset reads on).
const PAGE_PART: usize = 12_000;

impl Job {
    fn new(id: u64, base: usize) -> Job {
        Job {
            id,
            base,
            rounds: 0,
            said: false,
            separate: false,
            restart: false,
            turn_off: false,
            new_conversation: false,
            turn: None,
            closed: 0,
            script: None,
            text: String::new(),
            shown: String::new(),
            read: 0,
            read_ms: 0,
            written: 0,
            write_ms: 0,
            seen: 0,
            see_ms: 0,
            web: Vec::new(),
            call: None,
            raw: None,
            by: None,
        }
    }
}

struct Assistant {
    brain: Brain,
    /// The last web page read: its address, title and words, and whether it was cut at the most read (so reading on
    /// from an offset doesn't fetch it again).
    page: Option<(String, String, String, bool)>,
    /// The model's file, if the settings name one (else the app's are looked for).
    setting: Option<String>,
    /// The tools, as the model is given them.
    tools: Arc<Vec<Json>>,
    /// The conversation, and where the part the model's given starts (the oldest is left out once it's too long).
    parts: Vec<Part>,
    from: usize,
    /// What the model noted about the person (its remember tool), shown to it in every conversation.
    notes: Vec<String>,
    /// The system prompt for this conversation: with the notes as they were when it began, so it stays the same, and
    /// is read once.
    system: String,
    job: Option<Job>,
    /// How many questions have been asked (each job's number).
    jobs: u64,
    /// One of the model's steps, away on the other cores; and a turn whose question was stopped while its step was
    /// away (ended once the step's back).
    away: Option<Away>,
    orphan: Option<Box<Turn>>,
    /// The conversation changed since it was last kept (keep.rs).
    unkept: bool,
    /// Your own key for Claude, if there's one (never logged), and the mark of the one the settings gave (so a new
    /// one pasted in Windows' setup takes the place of the one kept).
    claude_key: Option<String>,
    setup_key: u64,
    /// Claude answers, rather than Chat Computer's own model: its model and effort.
    claude: Option<Claude>,
    /// The tools, as Claude's API takes them.
    claude_tools: Vec<claude::api::Tool>,
    /// Whether each sidebar shows (Messages, then Actions and Chats), as their switches left them.
    sides: [bool; 2],
    /// The apps the model made for the person, kept with the conversation (and each one's data, its localStorage).
    apps: Vec<App>,
    /// When the conversation was last kept (uptime ms), and whether an app's data changed since: kept again, at most
    /// every APP_DATA_MS.
    kept_at: u64,
    app_data: bool,
}

/// The most apps, the biggest an app's page may be, and how often an app's data is kept while it's used.
const MOST_APPS: usize = 40;
const APP_BYTES: usize = 256 << 10;
const APP_DATA_MS: u64 = 3000;

/// Claude, as chosen: its model ("claude-opus-5-5") and its effort ("max"; None: the model's default).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Claude {
    pub model: String,
    pub effort: Option<String>,
}

/// Claude's models the spark offers (Anthropic's newest, as its documentation listed them in November 2026), each
/// with what it's called and the effort it takes by default (Opus 5.5 and Haiku 5.5: medium; the others: high).
pub const CLAUDE_MODELS: [(&str, &str, &str); 4] = [
    ("claude-opus-5-5", "Claude Opus 5.5", "medium"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5", "high"),
    ("claude-haiku-5-5", "Claude Haiku 5.5", "medium"),
    ("claude-fable-5-1", "Claude Fable 5.1", "high"),
];

/// The efforts, as the spark names them.
pub const EFFORT_NAMES: [(&str, &str); 5] = [("low", "Low"), ("medium", "Medium"), ("high", "High"), ("xhigh", "Extra high"), ("max", "Max")];

/// A Claude model's name ("Claude Opus 5.5"), or its id if it isn't one of those listed.
pub fn claude_name(model: &str) -> String {
    CLAUDE_MODELS.iter().find(|(id, _, _)| *id == model).map_or_else(|| String::from(model), |(_, name, _)| String::from(*name))
}

/// The effort a model takes when none is chosen.
pub fn default_effort(model: &str) -> &'static str {
    CLAUDE_MODELS.iter().find(|(id, _, _)| *id == model).map_or("high", |(_, _, e)| e)
}

pub fn effort_name(effort: &str) -> &'static str {
    EFFORT_NAMES.iter().find(|(e, _)| *e == effort).map_or("Default", |(_, n)| n)
}

/// A mark of a key (FNV-1a), to tell one from another without keeping it twice.
fn key_mark(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The settings' "ai = claude [model] [effort]" as a choice.
fn claude_from_setting(ai: &str) -> Option<Claude> {
    let mut words = ai.split_ascii_whitespace();
    if words.next()? != "claude" {
        return None;
    }
    let mut c = Claude { model: String::from(CLAUDE_MODELS[0].0), effort: Some(String::from(claude::api::EFFORT)) };
    for w in words {
        if EFFORT_NAMES.iter().any(|(e, _)| *e == w) {
            c.effort = Some(String::from(w));
        } else {
            let id = if w.starts_with("claude-") { String::from(w) } else { format!("claude-{w}") };
            if id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
                c.model = id;
            }
        }
    }
    Some(c)
}

/// What the rest of the kernel sees of it while it works (it's taken out of its lock then).
struct Shown {
    ready: Ready,
    /// Its line in the computer's status.
    status: String,
    /// An answer is on its way.
    busy: bool,
    /// It has work to do (so the main loop doesn't wait for an interrupt).
    working: bool,
}

static ASSISTANT: SpinLock<Option<Assistant>> = SpinLock::new(None);
static SHOWN: SpinLock<Option<Shown>> = SpinLock::new(None);

/// Starts: finds the model's file (the one `setting` names, if it names one), and starts reading it; with your own
/// key for Claude, if the settings bring one, and who answers at start, if the settings say (`ai`).
pub fn init(setting: Option<String>, settings_key: Option<String>, ai: Option<String>) {
    // (A model test build gives the model no tools: in a virtual PC without hardware virtualization, the processor
    // is emulated, and their descriptions would take minutes to read.)
    let tools = match json::parse(TOOLS) {
        Some(Json::Array(t)) if test_build() != Some("model") => t,
        Some(Json::Array(_)) => Vec::new(),
        _ => {
            // (Without them it could only talk, and a test's script, which calls the tools itself, wouldn't notice.)
            crate::log!("Chat Computer kernel: model: its tools can't be read: tools.json isn't a list of them");
            Vec::new()
        }
    };
    let names: Vec<&str> = tools.iter().filter_map(|t| t.get("function").and_then(|f| f.get("name")).and_then(Json::str)).collect();
    crate::log!("Chat Computer kernel: model: its tools: {}", if names.is_empty() { String::from("none") } else { names.join(", ") });
    // The model's notes and the conversation from before the last restart, if they were kept; and who answers, with
    // Claude's key.
    let kept = crate::keep::load().and_then(|bytes| String::from_utf8(bytes).ok());
    let (notes, parts) = kept.as_deref().and_then(from_kept).unwrap_or_default();
    let kept_json = kept.as_deref().and_then(json::parse);
    let mut ai_kept = kept_json.as_ref().and_then(|j| j.get("ai").cloned());
    let kept_sides = kept_json.as_ref().and_then(|j| j.get("sides").cloned());
    if let Some(k) = kept {
        let mut k = k.into_bytes();
        crate::remote::wipe(&mut k);
    }
    let parts = if parts.first().is_none_or(|p| matches!(p, Part::You { .. })) { parts } else { Vec::new() };
    if !parts.is_empty() {
        crate::log!("Chat Computer kernel: model: the conversation from before the restart ({} parts)", parts.len());
    }
    if !notes.is_empty() {
        let n = notes.len();
        crate::log!("Chat Computer kernel: model: {n} note{} from before", if n == 1 { "" } else { "s" });
    }
    let claude_tools = tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            Some(claude::api::Tool {
                name: String::from(f.get("name")?.str()?),
                description: String::from(f.get("description").and_then(Json::str).unwrap_or("")),
                input_schema: json::compact(f.get("parameters")?),
            })
        })
        .collect();
    let mut a = Assistant {
        brain: Brain::None { why: String::new(), again: false, since: 0 },
        page: None,
        setting,
        tools: Arc::new(tools),
        parts,
        from: 0,
        notes,
        system: String::new(),
        job: None,
        jobs: 0,
        away: None,
        orphan: None,
        unkept: false,
        claude_key: None,
        setup_key: 0,
        claude: None,
        claude_tools,
        sides: [true, true],
        apps: Vec::new(),
        kept_at: 0,
        app_data: false,
    };
    // The apps made before, and their data (each one's site storage).
    if let Some(Json::Array(list)) = kept_json.as_ref().and_then(|j| j.get("apps")) {
        for app in list {
            let (Some(name), Some(html)) = (app.get("name").and_then(Json::str), app.get("html").and_then(Json::str)) else { continue };
            if let Some(Json::Object(data)) = app.get("data") {
                let map = data.iter().filter_map(|(k, v)| Some((k.clone(), String::from(v.str()?)))).collect();
                crate::site::restore_storage(&crate::site::app_origin(name), map);
            }
            let list = |key: &str| -> Vec<String> {
                match app.get(key) {
                    Some(Json::Array(v)) => v.iter().filter_map(|x| x.str().and_then(app_site)).take(MOST_APP_SITES).collect(),
                    _ => Vec::new(),
                }
            };
            let sites = list("sites");
            let allowed = list("allowed").into_iter().filter(|h| sites.contains(h)).collect();
            let because = String::from(app.get("because").and_then(Json::str).unwrap_or(""));
            a.apps.push(App { name: String::from(name), html: String::from(html), sites, allowed, because });
        }
        crate::log!("Chat Computer kernel: model: {} app{} from before", a.apps.len(), if a.apps.len() == 1 { "" } else { "s" });
    }
    if let Some(Json::Array(s)) = kept_sides.as_ref() {
        for (k, v) in s.iter().take(2).enumerate() {
            if let Json::Bool(b) = v {
                a.sides[k] = *b;
            }
        }
    }
    // Claude's key: the one kept, unless the settings bring a new one (pasted in Windows' setup since).
    if let Some(ai) = ai_kept.take() {
        a.claude_key = ai.get("key").and_then(Json::str).map(String::from);
        a.setup_key = ai.get("setup").and_then(Json::str).and_then(|m| u64::from_str_radix(m, 16).ok()).unwrap_or(0);
        a.claude = ai.get("claude").and_then(|c| {
            Some(Claude { model: String::from(c.get("model")?.str()?), effort: c.get("effort").and_then(Json::str).map(String::from) })
        });
    }
    if let Some(key) = settings_key {
        let mark = key_mark(&key);
        if mark != a.setup_key || a.claude_key.is_none() {
            a.claude_key = Some(key);
            a.setup_key = mark;
            a.unkept = true;
        }
    }
    if a.claude.is_none()
        && let Some(c) = ai.as_deref().and_then(claude_from_setting)
    {
        a.claude = Some(c);
    }
    if a.claude_key.is_none() {
        a.claude = None;
    }
    match (&a.claude, &a.claude_key) {
        (Some(c), _) => crate::log!(
            "Chat Computer kernel: model: Claude answers ({}, {} effort), with your key",
            c.model,
            c.effort.as_deref().unwrap_or("its default")
        ),
        (None, Some(_)) => crate::log!("Chat Computer kernel: model: your key for Claude is here (the spark can choose Claude)"),
        _ => {}
    }
    if test_build() == Some("chat") {
        let script = core::str::from_utf8(TEST_SCRIPT).ok().and_then(json::parse).unwrap_or(Json::Object(Vec::new()));
        a.brain = Brain::Script(script);
        a.system = a.system_prompt();
        crate::log!("Chat Computer kernel: model: a test's script answers, in place of a model");
    } else {
        a.look();
    }
    a.show();
    *ASSISTANT.lock() = Some(a);
}

/// Whether each sidebar shows (Messages, then Actions and Chats), as kept.
pub fn sides() -> [bool; 2] {
    ASSISTANT.lock().as_ref().map_or([true, true], |a| a.sides)
}

/// The sidebars switched off or on: kept with the conversation.
pub fn set_sides(sides: [bool; 2]) {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    if a.sides != sides {
        a.sides = sides;
        a.unkept = true;
    }
}

/// Who answers, if it's Claude: its model and effort.
pub fn claude() -> Option<Claude> {
    ASSISTANT.lock().as_ref().and_then(|a| a.claude.clone())
}

/// Whether there's a key for Claude.
pub fn has_claude_key() -> bool {
    ASSISTANT.lock().as_ref().is_some_and(|a| a.claude_key.is_some())
}

/// Your own key for Claude, typed (kept in the memory file, never logged). Its words are wiped from `key`.
pub fn set_claude_key(key: &mut String) {
    let k = String::from(key.trim());
    crate::files::wipe(key);
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    a.claude_key = Some(k);
    a.unkept = true;
    crate::log!("Chat Computer kernel: model: your key for Claude was typed (it isn't logged)");
}

/// Forgets your key for Claude (Chat Computer's own model answers then).
pub fn forget_claude_key() {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    if let Some(k) = a.claude_key.take() {
        let mut k = k.into_bytes();
        crate::remote::wipe(&mut k);
    }
    a.claude = None;
    a.unkept = true;
    crate::log!("Chat Computer kernel: model: your key for Claude is forgotten");
    a.show();
}

/// Who answers from now on: Claude (its model and effort: there must be a key), or Chat Computer's own model (None).
pub fn choose_ai(who: Option<Claude>) {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    if who.is_some() && a.claude_key.is_none() {
        return;
    }
    if a.claude != who {
        match &who {
            Some(c) => crate::log!(
                "Chat Computer kernel: model: Claude answers now ({}, {} effort)",
                c.model,
                c.effort.as_deref().unwrap_or("its default")
            ),
            None => crate::log!("Chat Computer kernel: model: Chat Computer's own model answers now"),
        }
        a.claude = who;
        a.unkept = true;
        a.show();
    }
}

/// The file of the model answering (or being read now), if there is one.
pub fn model_path() -> Option<String> {
    let a = ASSISTANT.lock();
    match &a.as_ref()?.brain {
        Brain::Ready(l) => Some(l.path.clone()),
        Brain::Reading(r) => Some(r.file.path.clone()),
        _ => None,
    }
}

/// Whether the model can be asked now.
pub fn ready() -> Ready {
    SHOWN.lock().as_ref().map_or_else(|| Ready::No(String::from(NO_MODEL)), |s| s.ready.clone())
}

/// The model's line in the computer's status.
pub fn status() -> String {
    SHOWN.lock().as_ref().map_or_else(|| String::from("Model: none"), |s| s.status.clone())
}

/// Whether an answer is on its way.
pub fn busy() -> bool {
    SHOWN.lock().as_ref().is_some_and(|s| s.busy)
}

/// Whether there's work to do now (the model's file being read, its instructions read ahead, or an answer): the main
/// loop goes straight on rather than waiting for an interrupt.
pub fn working() -> bool {
    SHOWN.lock().as_ref().is_some_and(|s| s.working)
}

/// The conversation so far, as the screen shows it: what was typed, and the model's words for each question (its
/// rounds' words together, a blank line between them).
pub fn conversation() -> Vec<(bool, String)> {
    let guard = ASSISTANT.lock();
    let Some(a) = guard.as_ref() else { return Vec::new() };
    let mut shown: Vec<(bool, String)> = Vec::new();
    for p in &a.parts {
        match p {
            Part::You { text, .. } => shown.push((true, text.clone())),
            Part::Said(answer, _) => {
                let words = answer.text.trim();
                match shown.last_mut() {
                    Some((false, said)) if !words.is_empty() => {
                        said.push_str("\n\n");
                        said.push_str(words);
                    }
                    _ if !words.is_empty() => shown.push((false, String::from(words))),
                    _ => {}
                }
            }
            Part::Tool(..) => {}
        }
    }
    shown
}

/// From the main loop, when an answer is over: the conversation kept (as much of it as the memory file holds, the
/// oldest questions left out first).
pub fn keep() {
    let text = {
        let mut guard = ASSISTANT.lock();
        let Some(a) = guard.as_mut() else { return };
        let now = crate::time::uptime_ms();
        let data_due = a.app_data && now.saturating_sub(a.kept_at) >= APP_DATA_MS;
        if !(a.unkept || data_due) || a.job.is_some() {
            return;
        }
        a.unkept = false;
        a.app_data = false;
        a.kept_at = now;
        let capacity = crate::keep::capacity();
        if capacity == 0 {
            return;
        }
        let apps = kept_apps(&a.apps);
        let mut start = 0;
        loop {
            let text = kept_text(&a.notes, &a.parts[start..], a.ai_kept(), a.sides, &apps);
            if text.len() <= capacity || start >= a.parts.len() {
                break text;
            }
            start += 1;
            while start < a.parts.len() && !matches!(a.parts[start], Part::You { .. }) {
                start += 1;
            }
        }
    };
    match crate::keep::save(text.as_bytes()) {
        Ok(()) => crate::log!("Chat Computer kernel: memory: the conversation kept ({} bytes)", text.len()),
        Err(e) => crate::log!("Chat Computer kernel: memory: the conversation couldn't be kept: {e}"),
    }
}

/// Asks the model to answer what the person typed (after the conversation so far). The answer comes from `work`.
/// Err: why it can't be asked, for the person.
pub fn ask(text: &str) -> Result<(), String> {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return Err(String::from(NO_MODEL)) };
    if a.job.is_some() {
        return Err(String::from("Chat Computer is still answering."));
    }
    if let Brain::None { why, .. } = &a.brain
        && a.claude.is_none()
    {
        return Err(why.clone());
    }
    // (A model test build's "look at": a conversation of its own, as check.py asks the engine on Linux it: an earlier
    // answer of random weights isn't text that would read back as the same tokens.)
    if test_build() == Some("model") && text.starts_with("look at ") {
        a.parts.clear();
    }
    // A new conversation's instructions have the notes as they are now.
    if a.parts.is_empty() && matches!(a.brain, Brain::Ready(_) | Brain::Script(_)) {
        a.from = 0;
        let system = a.system_prompt();
        if system != a.system {
            a.system = system;
            a.start_ahead();
        }
    }
    let base = a.parts.len();
    a.parts.push(Part::You { text: String::from(text), at: clock::now() });
    a.jobs += 1;
    a.job = Some(Job::new(a.jobs, base));
    a.show();
    Ok(())
}

/// Stops the answer on its way (the question taken back out).
pub fn stop() {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    let Some(mut job) = a.job.take() else { return };
    if let Some(mut call) = job.call.take() {
        call.stop();
    }
    if let Some(mut turn) = job.turn.take() {
        if a.away.as_ref().is_some_and(|w| w.job == Some(job.id)) {
            // (Its step is away: the turn is ended once the step's back.)
            a.orphan = Some(turn);
        } else if let Brain::Ready(m) = &mut a.brain {
            m.session.stop(&mut turn);
            m.session.end(*turn);
        }
    }
    a.parts.truncate(job.base);
    a.show();
    crate::log!("Chat Computer kernel: model: stopped");
}

/// From the main loop: moves things on for about `STEP_MS` (the model's file read, its instructions read ahead, the
/// answer written). Returns what came of it.
pub fn work() -> Vec<Event> {
    let mut events = Vec::new();
    // (Out of its lock while it works, so interrupts stay on.)
    let Some(mut a) = ASSISTANT.lock().take() else { return events };
    a.work(&mut events);
    a.show();
    *ASSISTANT.lock() = Some(a);
    events
}

impl Assistant {
    fn work(&mut self, events: &mut Vec<Event>) {
        let now = time::uptime_ms();
        let until = now + STEP_MS;
        if self.claude.is_some() && self.job.is_some() {
            return self.claude_work(until, events);
        }
        match &mut self.brain {
            Brain::None { why, again, since } => {
                if let Some(job) = self.job.take() {
                    self.parts.truncate(job.base);
                    fail(events, why.clone());
                }
                if *again && now.saturating_sub(*since) >= LOOK_AGAIN_MS {
                    self.look();
                }
            }
            Brain::Reading(r) => {
                let done = read_more(r, until);
                let pct = percent(r.bytes.len(), r.file.size as usize);
                match done {
                    Ok(false) => {
                        if let Some(job) = self.job.as_mut() {
                            activity(job, format!("Getting ready ({pct}%)"), events);
                        }
                    }
                    Ok(true) => self.loaded(events),
                    Err(why) => {
                        let why = format!("Chat Computer's model couldn't be read: {why}");
                        crate::log!("Chat Computer kernel: model: {why}");
                        self.brain = Brain::None { why, again: true, since: now };
                    }
                }
            }
            Brain::Ready(_) => self.model_work(until, events),
            Brain::Script(_) => {
                if let Some(mut job) = self.job.take() {
                    let over = loop {
                        if self.script_step(&mut job, events) {
                            break true;
                        }
                        if job.script.as_ref().is_some_and(|(left, ..)| !left.is_empty())
                            || time::uptime_ms() >= until
                            || !job.web.is_empty()
                        {
                            break false;
                        }
                    };
                    if !over {
                        self.job = Some(job);
                    }
                }
            }
        }
    }

    /// What the memory file keeps of who answers: Claude's key (with the mark of the settings' one), and Claude's
    /// model and effort when it answers.
    fn ai_kept(&self) -> Option<Json> {
        let key = self.claude_key.as_ref()?;
        let mut ai = vec![
            (String::from("key"), Json::Str(key.clone())),
            (String::from("setup"), Json::Str(format!("{:016x}", self.setup_key))),
        ];
        if let Some(c) = &self.claude {
            let mut who = vec![(String::from("model"), Json::Str(c.model.clone()))];
            if let Some(e) = &c.effort {
                who.push((String::from("effort"), Json::Str(e.clone())));
            }
            ai.push((String::from("claude"), Json::Object(who)));
        }
        Some(Json::Object(ai))
    }

    /// Moves Claude's answer on (Claude answers): its rounds asked one after another, the tools it calls run between
    /// them, as Chat Computer's own model's are.
    fn claude_work(&mut self, until: u64, events: &mut Vec<Event>) {
        let Some(mut job) = self.job.take() else { return };
        if !self.claude_job(&mut job, until, events) {
            self.job = Some(job);
        }
    }

    /// True when the answer is over (its last event given).
    fn claude_job(&mut self, job: &mut Job, until: u64, events: &mut Vec<Event>) -> bool {
        loop {
            if !self.web_step(job, events) {
                return false;
            }
            if job.call.is_none() {
                match self.begin_claude(job) {
                    Ok(call) => job.call = Some(call),
                    Err(why) => {
                        self.parts.truncate(job.base);
                        fail(events, why);
                        return true;
                    }
                }
            }
            let mut got = Vec::new();
            let over = job.call.as_mut().is_none_or(|c| c.step(&mut got));
            for g in got {
                match g {
                    crate::remote::Got::Words(w) => say(job, w, events),
                    crate::remote::Got::Thinking => {
                        if !job.said {
                            activity(job, String::from("Thinking"), events);
                        }
                    }
                    crate::remote::Got::Over(Err(why)) => {
                        job.call = None;
                        self.parts.truncate(job.base);
                        fail(events, why);
                        return true;
                    }
                    crate::remote::Got::Over(Ok(answered)) => {
                        job.call = None;
                        let (answer, raw) = claude_answer(&answered);
                        job.raw = Some(Arc::new(raw));
                        job.closed = answer.calls.len();
                        let cut = answered.stop.as_deref() == Some("max_tokens");
                        if self.round_over(job, answer, cut, events) {
                            return true;
                        }
                    }
                }
            }
            if !over || time::uptime_ms() >= until {
                return false;
            }
        }
    }

    /// The request for Claude's next round: the conversation so far as Anthropic's API has it.
    fn begin_claude(&mut self, job: &mut Job) -> Result<crate::remote::Call, String> {
        let Some(who) = self.claude.clone() else { return Err(String::from(NO_MODEL)) };
        let Some(key) = self.claude_key.clone() else {
            return Err(String::from("There's no key for Claude: choose Claude again with the spark beside + to give one."));
        };
        job.by = Some(claude_name(&who.model));
        let mut turns = self.claude_turns(job.base);
        // (Very long, the oldest questions aren't sent: as Chat Computer's own model's conversation is shortened.)
        while claude::api::messages(&turns).len() > CLAUDE_SENT_BYTES && self.shorten(job.base) {
            turns = self.claude_turns(job.base);
        }
        let system = self.claude_system(&who);
        if test_build().is_some() {
            crate::log!("Chat Computer kernel: test: asked Claude {}", claude::api::messages(&turns));
        }
        let ask = claude::api::Ask { model: &who.model, effort: who.effort.as_deref(), max_tokens: claude::api::MAX_TOKENS };
        let request = claude::api::request(claude::api::HOST, &key, &ask, &system, &turns, &self.claude_tools, &[]);
        Ok(crate::remote::Call::new(request))
    }

    /// Claude's instructions: Chat Computer's, as Claude, with the notes about the person.
    fn claude_system(&self, who: &Claude) -> String {
        let rest = SYSTEM.split_once("\n\nWith your tools").map_or("", |(_, r)| r);
        let mut system = format!(
            "You are Chat Computer: you run this computer for the person using it, who talks to you by typing here. You \
             are Claude ({}, made by Anthropic), answering through Anthropic's API with the person's own key, which they \
             chose with the spark beside + (Chat Computer's own model, an open model running on this computer, answers \
             when they choose it there instead): what they say goes to Anthropic.\n\nWith your tools{}",
            claude_name(&who.model),
            rest.replace("{sees}", " and see the pictures among them (scanned pages too)")
        );
        if self.from > 0 {
            system.push_str(SHORTENED);
        }
        if !self.notes.is_empty() {
            system.push_str("\n\nYour notes about the person, from earlier conversations (made with remember):");
            for n in &self.notes {
                system.push_str("\n\u{2022} ");
                system.push_str(n);
            }
        }
        system.push_str(&self.apps_line());
        system
    }

    /// The apps made for the person, for the model to know (and open, or change).
    fn apps_line(&self) -> String {
        if self.apps.is_empty() {
            return String::new();
        }
        let names: Vec<String> = self
            .apps
            .iter()
            .map(|a| match (a.sites.is_empty(), a.allowed.len() == a.sites.len()) {
                (true, _) => a.name.clone(),
                (false, true) => format!("{} (reaches {})", a.name, a.sites.join(", ")),
                (false, false) => format!("{} (needs {}; the person hasn't allowed it all yet)", a.name, a.sites.join(", ")),
            })
            .collect();
        format!("\n\nThe apps you've made for the person (make_app; open_app opens one): {}.", names.join(", "))
    }

    /// The conversation as Claude's API has it: what was typed (with when), each round of the answers (Claude's own
    /// blocks as they came; Chat Computer's own model's words and calls as Claude's would be), and what the tools said
    /// (pictures included, for the question being answered).
    fn claude_turns(&self, current: usize) -> Vec<claude::api::Turn> {
        use claude::api::{Content, Role, Turn};
        use claude::json::Json as CJson;
        let mut turns: Vec<Turn> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        let mut next_id = 0;
        let push = |turns: &mut Vec<Turn>, role: Role, blocks: Vec<CJson>| {
            if blocks.is_empty() {
                return;
            }
            match turns.last_mut() {
                Some(Turn { role: r, content: Content::Blocks(b) }) if *r == role => b.extend(blocks),
                _ => turns.push(Turn { role, content: Content::Blocks(blocks) }),
            }
        };
        let text = |t: &str| claude_object(&[("type", CJson::Str(String::from("text"))), ("text", CJson::Str(String::from(if t.is_empty() { " " } else { t })))]);
        for (i, p) in self.parts.iter().enumerate().skip(self.from.min(self.parts.len())) {
            match p {
                Part::You { text: t, at } => {
                    let words = match at {
                        Some(when) => format!("[{}] {t}", clock::short_text(*when)),
                        None => t.clone(),
                    };
                    push(&mut turns, Role::User, vec![text(&words)]);
                }
                Part::Said(answer, raw) => {
                    let blocks: Vec<CJson> = match raw.as_deref().and_then(|r| claude::json::parse(r)) {
                        Some(CJson::Arr(b)) => b,
                        _ => {
                            let mut b = Vec::new();
                            if !answer.text.trim().is_empty() {
                                b.push(text(answer.text.trim()));
                            }
                            for (k, call) in answer.calls.iter().enumerate() {
                                let tool = self.tools.iter().find(|t| t.get("function").and_then(|f| f.get("name")).and_then(Json::str) == Some(&call.name));
                                let input = claude::json::parse(&json::compact(&call.arguments(tool))).unwrap_or(CJson::Obj(Vec::new()));
                                b.push(claude_object(&[
                                    ("type", CJson::Str(String::from("tool_use"))),
                                    ("id", CJson::Str(format!("toolu_cc{i}x{k}"))),
                                    ("name", CJson::Str(call.name.clone())),
                                    ("input", input),
                                ]));
                            }
                            b
                        }
                    };
                    ids = blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.str()) == Some("tool_use"))
                        .filter_map(|b| b.get("id").and_then(|t| t.str()).map(String::from))
                        .collect();
                    next_id = 0;
                    push(&mut turns, Role::Assistant, blocks);
                }
                Part::Tool(said, picture) => {
                    let Some(id) = ids.get(next_id).cloned() else { continue };
                    next_id += 1;
                    let error = said.starts_with("Error: ");
                    let said = said.strip_prefix("Error: ").unwrap_or(said);
                    let block = match picture {
                        Some(pic) if i >= current => {
                            let p = picture::Picture { width: pic.width as u32, height: pic.height as u32, rgb: pic.rgb.clone() };
                            let jpeg = picture::encode_jpeg(&p, 85);
                            claude::api::tool_result_with(&id, said, error, vec![claude::api::image("image/jpeg", &jpeg)])
                        }
                        Some(_) => claude::api::tool_result(&id, &format!("{said}\n\n[The picture shown here then isn't sent again: read_file shows it again.]"), error),
                        None => claude::api::tool_result(&id, said, error),
                    };
                    push(&mut turns, Role::User, vec![block]);
                }
            }
        }
        turns
    }

    /// What the rest of the kernel sees, brought up to date.
    fn show(&self) {
        let ready = match &self.brain {
            _ if self.claude.is_some() => Ready::Yes(claude_name(&self.claude.as_ref().map_or_else(String::new, |c| c.model.clone()))),
            Brain::None { why, .. } => Ready::No(why.clone()),
            Brain::Reading(r) => Ready::Getting(format!(
                "Getting Chat Computer's own model ready: reading {} from {} ({}%)\u{2026}",
                r.name,
                drive_of(&r.file.path),
                percent(r.bytes.len(), r.file.size as usize)
            )),
            Brain::Ready(m) if m.instructions_left().is_some() => {
                let (done, of) = m.instructions_left().unwrap_or((0, 1));
                Ready::Getting(format!(
                    "Getting Chat Computer's own model ready: {} is reading its instructions ({}%)\u{2026}",
                    m.name,
                    percent(done, of)
                ))
            }
            Brain::Ready(m) => Ready::Yes(m.name.clone()),
            Brain::Script(_) => Ready::Yes(String::from("a test's script")),
        };
        let status = match &self.brain {
            _ if self.claude.is_some() => {
                let c = self.claude.as_ref().expect("Claude");
                format!(
                    "Model: you are {} ({}), at {} effort, answering through Anthropic's API with the person's own key \
                     (they chose you with the spark beside +; Chat Computer's own model, on this PC, answers when they \
                     choose it there)",
                    claude_name(&c.model),
                    c.model,
                    c.effort.as_deref().unwrap_or("its default")
                )
            }
            Brain::None { why, .. } => format!("Model: none ({why})"),
            Brain::Reading(r) => format!("Model: {}, being read from {} into memory", r.name, r.file.path),
            Brain::Ready(m) => {
                let level = level_name();
                format!(
                    "Model: you are {} ({}), run by Chat Computer's own engine on {} ({level}){}{}; \
                     conversations up to {} tokens{}",
                    m.name,
                    m.path,
                    match crate::cores::count() {
                        1 => String::from("one of this PC's processor cores"),
                        n => format!("{n} of this PC's processor cores"),
                    },
                    if m.uncensored { ", its refusals taken out" } else { "" },
                    if m.model.vision.is_some() { ", seeing pictures" } else { ", its file without the part that sees pictures" },
                    m.context,
                    m.speed.map(|s| format!("; lately {s:.1} tokens a second")).unwrap_or_default()
                )
            }
            Brain::Script(_) => String::from("Model: a test's script, in place of a model"),
        };
        // (The main loop goes straight on while there's work for it; while a step of the model's is away on the other
        // cores, it waits: the step's end wakes it.)
        let ahead = matches!(&self.brain, Brain::Ready(m) if !m.ahead_read && self.claude.is_none());
        let away = self.away.as_ref().is_some_and(|a| !a.errand.done());
        let working = !away && (self.job.is_some() || matches!(&self.brain, Brain::Reading(_)) || ahead);
        *SHOWN.lock() = Some(Shown { ready, status, busy: self.job.is_some(), working });
    }

    /// The system prompt for a conversation: the instructions, and the model's notes about the person.
    fn system_prompt(&self) -> String {
        let name = match &self.brain {
            Brain::Ready(m) => m.name.clone(),
            Brain::Reading(r) => r.name.clone(),
            _ => String::from("Qwen"),
        };
        let sees = match &self.brain {
            Brain::Ready(m) => m.model.vision.is_some(),
            Brain::Script(_) => true,
            _ => false,
        };
        let mut system = SYSTEM.replace("{model}", &name).replace(
            "{sees}",
            if sees { " and see the pictures among them (scanned pages too)" } else { "" },
        );
        if self.from > 0 {
            system.push_str(SHORTENED);
        }
        if !self.notes.is_empty() {
            system.push_str("\n\nYour notes about the person, from earlier conversations (made with remember):");
            for n in &self.notes {
                system.push_str("\n\u{2022} ");
                system.push_str(n);
            }
        }
        system.push_str(&self.apps_line());
        system
    }

    /// What the model's given: the system prompt, then the conversation from where it's given.
    fn messages(&self) -> Vec<Message> {
        let mut out = vec![Message::System(self.system.clone())];
        for p in &self.parts[self.from.min(self.parts.len())..] {
            out.push(match p {
                Part::You { text, at: Some(t) } => Message::User(format!("[{}] {text}", clock::short_text(*t))),
                Part::You { text, at: None } => Message::User(text.clone()),
                Part::Said(a, _) => Message::Assistant(a.clone()),
                Part::Tool(t, None) => Message::Tool(t.clone()),
                Part::Tool(t, Some(picture)) => Message::ToolParts(vec![Content::Text(t.clone()), Content::Picture(picture.clone())]),
            });
        }
        out
    }

    /// Finds the model's file and starts reading it, or says why there's none.
    fn look(&mut self) {
        let now = time::uptime_ms();
        let found = match &self.setting {
            Some(path) => Ok(path.clone()),
            None => choose(),
        };
        match found.and_then(|path| start_reading(&path)) {
            Ok(r) => {
                crate::log!("Chat Computer kernel: model: reading {} ({}) from {}", r.name, size_text(r.file.size), r.file.path);
                self.brain = Brain::Reading(Box::new(r));
            }
            Err((why, again)) => {
                // (Logged once, not each time it looks again.)
                if !matches!(&self.brain, Brain::None { why: before, .. } if *before == why) {
                    crate::log!("Chat Computer kernel: model: none: {why}");
                }
                self.brain = Brain::None { why, again, since: now };
            }
        }
    }

    /// The model's file is read: the model made ready from it, and its instructions read ahead.
    fn loaded(&mut self, events: &mut Vec<Event>) {
        let Brain::Reading(r) = core::mem::replace(&mut self.brain, Brain::None { why: String::new(), again: false, since: 0 })
        else {
            return;
        };
        let seconds = (time::uptime_ms() - r.started) as f64 / 1000.0;
        let size = r.file.size;
        match ready_model(*r) {
            Ok(m) => {
                crate::log!(
                    "Chat Computer kernel: model: {} ready ({} read in {seconds:.1} s){}, conversations up to {} tokens, its \
                     arithmetic {}",
                    m.name,
                    size_text(size),
                    if m.uncensored { ", its refusals taken out" } else { "" },
                    m.context,
                    level_name()
                );
                self.brain = Brain::Ready(Box::new(m));
                self.system = self.system_prompt();
                self.start_ahead();
            }
            Err(why) => {
                crate::log!("Chat Computer kernel: model: {why}");
                if let Some(job) = self.job.take() {
                    self.parts.truncate(job.base);
                    fail(events, why.clone());
                }
                self.brain = Brain::None { why, again: false, since: time::uptime_ms() };
            }
        }
    }

    /// Reading ahead starts again (the instructions changed, or the conversation): what it reads is worked out anew.
    fn start_ahead(&mut self) {
        if let Brain::Ready(m) = &mut self.brain {
            m.ahead_of = None;
            m.ahead_read = false;
        }
    }

    /// Chat Computer's own model's work: its step away on the other cores looked at (back: what came of it taken), a
    /// turn whose question was stopped meanwhile ended, and the next step sent (the answer's, or reading ahead). With
    /// one core, steps are done here, one after another until `until`.
    fn model_work(&mut self, until: u64, events: &mut Vec<Event>) {
        loop {
            if let Some(away) = &self.away {
                if !away.errand.done() {
                    return;
                }
                let away = self.away.take().expect("a step away");
                self.step_back(away, events);
            }
            if let Some(mut turn) = self.orphan.take()
                && let Brain::Ready(m) = &mut self.brain
            {
                m.session.stop(&mut turn);
                m.session.end(*turn);
            }
            if self.job.is_some() {
                self.answer(events);
            } else {
                self.read_ahead_step();
            }
            match &self.away {
                Some(away) if away.errand.done() && time::uptime_ms() < until => {}
                _ => return,
            }
        }
    }

    /// What came of a step that was away.
    fn step_back(&mut self, mut away: Away, events: &mut Vec<Event>) {
        let Some(back) = away.errand.take() else { return };
        let Brain::Ready(m) = &mut self.brain else { return };
        let target = if crate::cores::count() > 1 { AWAY_MS } else { STEP_MS };
        let (least, most) = if crate::cores::count() > 1 { (READ_LEAST, READ_MOST) } else { (1, model::net::BATCH) };
        match back {
            Back::Ahead { ahead, done, all, took } => {
                if away.of.is_some() && away.of == m.ahead_of {
                    if ahead.is_some() {
                        m.ahead = ahead;
                    }
                    m.ahead_done = done;
                    m.ahead_read = all;
                }
                if took > 0 {
                    m.read = suited_to(m.read, took, target, most).max(least);
                }
                if let Some(anchor) = m.ahead.as_ref().and_then(|a| a.anchor)
                    && !m.instructions_read
                    && m.ahead_done >= anchor
                {
                    m.instructions_read = true;
                    crate::log!("Chat Computer kernel: model: read its instructions ahead ({anchor} tokens)");
                }
            }
            Back::Turn { step, reading, before, over, took } => {
                // (Its question stopped meanwhile: the turn is the orphan's, ended already.)
                let Some(job) = self.job.as_mut().filter(|j| away.job == Some(j.id)) else { return };
                match step {
                    Step::Read { done, total } => {
                        job.read += done.saturating_sub(before);
                        job.read_ms += took;
                        m.read = suited_to(m.read, took, target, most).max(least);
                        if !job.said && job.read_ms > 1500 {
                            activity(job, format!("Reading ({}%)", percent(done, total)), events);
                        }
                    }
                    Step::Seeing { done, total } => {
                        job.see_ms += took;
                        m.see = suited_to(m.see, took, target, 1 << 16);
                        if done == total {
                            job.seen += 1;
                        }
                        if !job.said {
                            activity(job, format!("Looking at the picture ({}%)", percent(done, total)), events);
                        }
                    }
                    Step::Said(s) => {
                        if !reading {
                            job.written += 1;
                            job.write_ms += took;
                        }
                        // (A call that comes as the turn ends was never closed: the engine reads it as far as it got.
                        // If the round reached the most it says, it was cut off as it was written, and isn't run.)
                        match s {
                            Saying::Text(words) => say(job, words, events),
                            Saying::Thinking(_) => activity(job, String::from("Thinking"), events),
                            Saying::Call(_) if !over => job.closed += 1,
                            _ => {}
                        }
                    }
                    Step::Over => {}
                }
            }
        }
    }

    /// Reads ahead, a step: what the next question will start with (the instructions, the conversation so far, and
    /// the words typed so far that can't change) worked out again if any of that changed, and a step of reading it
    /// sent away. Not while Claude answers (its own model isn't asked then).
    fn read_ahead_step(&mut self) {
        if self.claude.is_some() {
            return;
        }
        let words = sure_words(&TYPED.lock());
        let now = clock::now();
        let of = AheadOf {
            parts: self.parts.len(),
            from: self.from,
            system: model::file::fnv(0xcbf2_9ce4_8422_2325, self.system.as_bytes()),
            words,
            minute: now.map_or(-1, |t| t.div_euclid(60)),
        };
        let fresh = match &self.brain {
            Brain::Ready(m) if m.ahead_of.as_ref() == Some(&of) => {
                if m.ahead_read {
                    return;
                }
                false
            }
            Brain::Ready(_) => true,
            _ => return,
        };
        // (Worked out anew on the other core, with the step: the conversation's words made into tokens there.)
        let messages = if fresh {
            let mut all = self.messages();
            all.push(Message::User(match now {
                Some(t) if of.words.is_empty() => format!("[{}]", clock::short_text(t)),
                Some(t) => format!("[{}] {}", clock::short_text(t), of.words),
                None => of.words.clone(),
            }));
            Some(all)
        } else {
            None
        };
        let tools = self.tools.clone();
        let Brain::Ready(m) = &mut self.brain else { return };
        if fresh {
            m.ahead_of = Some(of.clone());
            m.ahead_read = false;
        }
        let known = m.ahead.clone();
        let (session, read, context) = (Lent(&mut m.session as *mut Session<'static>), m.read, m.context);
        let errand = crate::cores::away(move || {
            let session = unsafe { &mut *session.get() };
            let t = time::uptime_ms();
            let ahead = match messages {
                Some(messages) => match session.prompt_ahead(&request(&tools, &messages, 0, Sampling::GREEDY)) {
                    Ok(a) => Arc::new(a),
                    // (Nothing to read ahead: it's all read, as far as reading ahead goes.)
                    Err(_) => return Back::Ahead { ahead: None, done: 0, all: true, took: 0 },
                },
                None => match known {
                    Some(a) => a,
                    None => return Back::Ahead { ahead: None, done: 0, all: true, took: 0 },
                },
            };
            // (Longer than the model's given: the question shortens the conversation, and reads it then.)
            if ahead.tokens.len() > context {
                return Back::Ahead { done: 0, all: true, took: 0, ahead: Some(ahead) };
            }
            let all = session.read_on(&ahead, read, &crate::cores::Cores);
            Back::Ahead { done: session.read_of(&ahead), all, took: time::uptime_ms() - t, ahead: Some(ahead) }
        });
        self.away = Some(Away { errand, of: Some(of), job: None });
    }

    /// Moves the answer on: as far as its next step, sent away.
    fn answer(&mut self, events: &mut Vec<Event>) {
        let Some(mut job) = self.job.take() else { return };
        if !self.answer_job(&mut job, events) {
            self.job = Some(job);
        }
    }

    /// True when the answer is over (its last event given).
    fn answer_job(&mut self, job: &mut Job, events: &mut Vec<Event>) -> bool {
        loop {
            // (The round's web tools still being fetched: the next round waits for them, the main loop going on.)
            if !self.web_step(job, events) {
                return false;
            }
            // (A model test build, asked "look at PATH [page N]": its first round read_file's call of it, as if the
            // model had made it, for check.py to see the model see a picture, which a model of random weights won't
            // ask for.)
            if job.turn.is_none()
                && job.rounds == 0
                && test_build() == Some("model")
                && let Some(Part::You { text, .. }) = self.parts.get(job.base)
                && let Some(rest) = text.strip_prefix("look at ")
            {
                let (path, page) = match rest.rsplit_once(" page ") {
                    Some((path, page)) if page.parse::<u32>().is_ok() => (path, Some(page)),
                    _ => (rest, None),
                };
                let mut args = vec![(String::from("path"), String::from(path))];
                if let Some(page) = page {
                    args.push((String::from("page"), String::from(page)));
                }
                let answer = Answer { calls: vec![Call { name: String::from("read_file"), args }], ..Answer::default() };
                job.closed = 1;
                if self.round_over(job, answer, false, events) {
                    return true;
                }
                continue;
            }
            if job.turn.is_none()
                && let Err(why) = self.begin_round(job)
            {
                self.parts.truncate(job.base);
                fail(events, why);
                return true;
            }
            let Brain::Ready(m) = &mut self.brain else { return true };
            let Some(turn) = job.turn.as_mut() else { return true };
            if turn.finished() {
                let turn = job.turn.take().expect("the round's turn");
                let (answer, stats) = m.session.end(*turn);
                if self.round_over(job, answer, stats.ended == Ended::Length, events) {
                    return true;
                }
                continue;
            }
            // The turn's next step, away on the other cores (the session and the turn aren't touched here meanwhile).
            let (session, turn) = (Lent(&mut m.session as *mut Session<'static>), Lent(&mut **turn as *mut Turn));
            let (read, see) = (m.read, m.see);
            let errand = crate::cores::away(move || {
                let (session, turn) = (unsafe { &mut *session.get() }, unsafe { &mut *turn.get() });
                let (reading, before) = (turn.reading(), session.read_so_far());
                session.see = see;
                let t = time::uptime_ms();
                let step = session.step(turn, read, &crate::cores::Cores);
                Back::Turn { step, reading, before, over: turn.finished(), took: time::uptime_ms() - t }
            });
            self.away = Some(Away { errand, of: None, job: Some(job.id) });
            return false;
        }
    }

    /// Begins a round of the answer: the conversation given to the model (its oldest part left out, if it's too long).
    fn begin_round(&mut self, job: &mut Job) -> Result<(), String> {
        loop {
            let messages = self.messages();
            let greedy = test_build() == Some("model");
            let Brain::Ready(m) = &mut self.brain else { return Err(String::from(NO_MODEL)) };
            let sampling = if greedy { Sampling::GREEDY } else { Sampling::recommended(&m.style_name, false) };
            let seed = if greedy { 1 } else { crate::random::u64() };
            let r = request(&self.tools, &messages, seed, sampling);
            let turn = m.session.begin(&r).map_err(|e| format!("Chat Computer's model couldn't be asked ({e:?})."))?;
            if turn.prompt_tokens() <= m.context {
                if test_build().is_some() {
                    crate::log!("Chat Computer kernel: test: asked {}", json::compact(&messages_json(&messages)));
                }
                job.turn = Some(Box::new(turn));
                job.closed = 0;
                return Ok(());
            }
            if !self.shorten(job.base) {
                return Err(String::from(
                    "That's more than Chat Computer's model can read at once here: ask it to start a new conversation, or \
                     send less.",
                ));
            }
        }
    }

    /// Leaves out the oldest part of the conversation given (about a third of it, whole questions, never the one
    /// being answered, at `upto`); false if there's nothing left to leave out.
    fn shorten(&mut self, upto: usize) -> bool {
        let target = self.from + (self.parts.len() - self.from) / 3;
        match (target.max(self.from + 1)..=upto.min(self.parts.len().saturating_sub(1))).find(|&i| matches!(self.parts[i], Part::You { .. })) {
            Some(i) => {
                self.from = i;
                self.system = self.system_prompt();
                crate::log!("Chat Computer kernel: model: the conversation's oldest part is no longer given to it");
                true
            }
            None => false,
        }
    }

    /// A round of the answer is over (`cut`: at the most a round says, ROUND_TOKENS): kept in the conversation; if it
    /// called tools, they run, and what they say goes back to the model in the next round (false); else the answer's
    /// over (true).
    fn round_over(&mut self, job: &mut Job, answer: Answer, cut: bool, events: &mut Vec<Event>) -> bool {
        let calls = answer.calls.clone();
        if test_build().is_some() {
            let round = Json::Object(vec![
                (String::from("text"), Json::Str(answer.text.clone())),
                (String::from("calls"), calls_json(&calls)),
            ]);
            crate::log!("Chat Computer kernel: test: round {}", json::compact(&round));
        }
        if answer.text.trim().is_empty() && calls.is_empty() && !job.said {
            self.parts.truncate(job.base);
            fail(events, String::from("Chat Computer's model didn't say anything."));
            return true;
        }
        let raw = job.raw.take();
        self.parts.push(Part::Said(answer, raw));
        if calls.is_empty() {
            if cut {
                // (The person can't tell by looking that it stopped mid-way: a line says so.)
                crate::log!("Chat Computer kernel: model: its answer reached the most a round says, and stops there");
                events.push(Event::Note(String::from(CUT_SHORT)));
            }
            self.finish(job, events);
            return true;
        }
        if job.rounds >= TOOL_ROUNDS {
            // Its tools used up (what they said last told it so): these aren't run, but each is answered, and the
            // answer ends with what it said, all it did kept for the next message to carry on from.
            crate::log!("Chat Computer kernel: model: its rounds of tools were used up: {} more not run", calls.len());
            for call in &calls {
                self.tool_said(&call.name, String::from(NOT_RUN), true, None);
            }
            events.push(Event::Note(String::from(USED_UP)));
            self.finish(job, events);
            return true;
        }
        for (i, call) in calls.iter().enumerate() {
            if cut && i >= job.closed {
                crate::log!("Chat Computer kernel: model: its call of {} was cut off as it wrote it, so isn't run", call.name);
                self.tool_said(&call.name, String::from(CUT_CALL), true, None);
                continue;
            }
            let tool = self.tools.iter().find(|t| t.get("function").and_then(|f| f.get("name")).and_then(Json::str) == Some(&call.name));
            let input = call.arguments(tool);
            if matches!(call.name.as_str(), "search_web" | "read_web_page") {
                if let Some((said, error)) = self.start_web(&call.name, &input, job) {
                    self.tool_said(&call.name, said, error, None);
                }
                continue;
            }
            let notes = self.notes.clone();
            let (sight, blind) = (self.sight(), self.blind());
            let apps = self.apps.len();
            let because = match self.parts.get(job.base) {
                Some(Part::You { text, .. }) => text.chars().take(300).collect(),
                _ => String::new(),
            };
            let (said, error, picture) = run_tool(&call.name, &input, job, &mut self.notes, &mut self.apps, &because, events, sight, blind);
            self.unkept |= self.notes != notes || matches!(call.name.as_str(), "make_app") || self.apps.len() != apps;
            self.tool_said(&call.name, said, error, picture);
        }
        job.rounds += 1;
        if job.rounds == TOOL_ROUNDS
            && let Some(Part::Tool(said, _)) = self.parts.last_mut()
        {
            said.push_str("\n\n");
            said.push_str(&last_round());
        }
        job.separate = job.said;
        false
    }

    /// A call of one of the web's tools: its fetch begun, a place kept for what it says in the conversation (None), or
    /// what it says at once (an address that can't be read, nothing to search for, or the last page read, read on).
    fn start_web(&mut self, name: &str, input: &Json, job: &mut Job) -> Option<(String, bool)> {
        crate::log!("Chat Computer kernel: model: uses {name}");
        let text = |key: &str| input.get(key).and_then(Json::str).map(str::trim).unwrap_or("");
        let (what, url) = if name == "search_web" {
            let query: String = text("query").chars().take(400).collect();
            if query.is_empty() {
                return Some((String::from("No words to search for were given."), true));
            }
            let url = web::Url::parse(&web::search::address(&query)).expect("the search's own address");
            (WebWhat::Search(query), url)
        } else {
            let url = match web::Url::parse(text("url")) {
                Ok(u) => u,
                Err(why) => return Some((format!("That address can't be read: {why}."), true)),
            };
            let offset = input.get("offset").and_then(number).unwrap_or(0.0).max(0.0) as usize;
            if let Some((at, title, words, cut)) = &self.page
                && *at == url.to_string()
            {
                return Some((page_part(at, title, words, *cut, offset), false));
            }
            (WebWhat::Read { url: url.to_string(), offset }, url)
        };
        self.parts.push(Part::Tool(String::new(), None));
        job.web.push(WebCall { part: self.parts.len() - 1, what, fetch: crate::web::Fetch::new(url), begun: false });
        None
    }

    /// Moves the round's web calls on: each in turn, said to the person as it begins, its result put in its place
    /// once fetched. True once there are none left.
    fn web_step(&mut self, job: &mut Job, events: &mut Vec<Event>) -> bool {
        while let Some(call) = job.web.first_mut() {
            if !call.begun {
                call.begun = true;
                let (doing, note) = match &call.what {
                    WebWhat::Search(q) => (
                        String::from("Searching the web"),
                        format!("Searched the web (DuckDuckGo) for \u{201C}{q}\u{201D}"),
                    ),
                    WebWhat::Read { .. } => (
                        format!("Reading {}", call.fetch.host()),
                        format!("Read a page on {}", call.fetch.host()),
                    ),
                };
                crate::log!("Chat Computer kernel: web: {note}");
                events.push(Event::Looked(note));
                activity(job, doing, events);
            }
            let Some(got) = job.web[0].fetch.step() else { return false };
            let call = job.web.remove(0);
            let (said, error) = self.web_said(&call.what, got);
            let name = call.name();
            if test_build().is_some() {
                let logged = Json::Object(vec![
                    (String::from("name"), Json::Str(String::from(name))),
                    (String::from("error"), Json::Bool(error)),
                    (String::from("said"), Json::Str(said.clone())),
                ]);
                crate::log!("Chat Computer kernel: test: tool {}", json::compact(&logged));
            }
            // (Anything already added after its place, such as the note that the rounds of tools are used up, stays.)
            if let Some(Part::Tool(place, _)) = self.parts.get_mut(call.part) {
                let after = core::mem::take(place);
                *place = format!("{}{said}{after}", if error { "Error: " } else { "" });
            }
        }
        true
    }

    /// What a web call says to the model, from what was fetched: the search's results, or the page's words (kept for
    /// reading on). The bool: it's an error.
    fn web_said(&mut self, what: &WebWhat, got: Result<crate::web::Page, String>) -> (String, bool) {
        match (what, got) {
            (WebWhat::Search(q), Ok(page)) => {
                let found = web::search::results(&String::from_utf8_lossy(&page.bytes));
                crate::log!("Chat Computer kernel: web: {} results", found.len());
                if found.is_empty() {
                    (String::from("The search found nothing (or DuckDuckGo's results page has changed)."), true)
                } else {
                    (format!("Results for \u{201C}{q}\u{201D} (DuckDuckGo):\n{}", web::search::listed(&found)), false)
                }
            }
            (WebWhat::Search(_), Err(why)) => {
                crate::log!("Chat Computer kernel: web: the search failed: {why}");
                (format!("Couldn't search the web: {why}."), true)
            }
            (WebWhat::Read { url, offset }, Ok(page)) => {
                let at = page.url.to_string();
                // (A PDF read as one on the PC's drives is, as far as the most fetched goes.)
                let words = if page.kind.to_ascii_lowercase().contains("application/pdf") || page.bytes.starts_with(b"%PDF-") {
                    match pdf::read(&page.bytes) {
                        Ok(d) if d.text.is_empty() => Err(String::from("a PDF whose pages are pictures (scanned, perhaps), with no words to read")),
                        Ok(d) => Ok(web::page::Page { title: d.title.unwrap_or_default(), text: d.text }),
                        Err(pdf::Error::Encrypted) => Err(String::from("a PDF a password protects")),
                        Err(_) if page.cut => Err(String::from("a PDF too big to read here (more than 3 MB)")),
                        Err(_) => Err(String::from("a PDF that can't be read (it may be damaged)")),
                    }
                } else {
                    web::page::words(&page.bytes, &page.kind)
                };
                match words {
                    Ok(p) => {
                        crate::log!(
                            "Chat Computer kernel: web: read {} ({} bytes, {} characters of words)",
                            page.url.host,
                            page.bytes.len(),
                            p.text.chars().count()
                        );
                        let said = page_part(&at, &p.title, &p.text, page.cut, *offset);
                        self.page = Some((at, p.title, p.text, page.cut));
                        // (Read on from the address asked for, too, if it was sent on elsewhere.)
                        if *url != self.page.as_ref().map_or(String::new(), |p| p.0.clone()) {
                            crate::log!("Chat Computer kernel: web: {url} was sent on to {}", page.url);
                        }
                        (said, false)
                    }
                    Err(kind) => (format!("{at} is {kind}, which can't be read here."), true),
                }
            }
            (WebWhat::Read { url, .. }, Err(why)) => {
                crate::log!("Chat Computer kernel: web: couldn't read {url}: {why}");
                (format!("Couldn't read {url}: {why}."), true)
            }
        }
    }

    /// What a tool said back (`error`: why it couldn't), and a picture it gives the model to see, for the model in
    /// the next round.
    fn tool_said(&mut self, name: &str, said: String, error: bool, picture: Option<picture::Picture>) {
        let picture = picture.map(|p| Arc::new(Picture::new(p.width as usize, p.height as usize, p.rgb)));
        if test_build().is_some() {
            let mut logged = vec![
                (String::from("name"), Json::Str(String::from(name))),
                (String::from("error"), Json::Bool(error)),
                (String::from("said"), Json::Str(said.clone())),
            ];
            if let Some(p) = &picture {
                logged.push((String::from("picture"), Json::Str(picture_text(p))));
            }
            crate::log!("Chat Computer kernel: test: tool {}", json::compact(&Json::Object(logged)));
        }
        self.parts.push(Part::Tool(if error { format!("Error: {said}") } else { said }, picture));
    }

    /// How the model sees, if it can (its file has a vision part; a test's script sees as Qwen 3.5 does).
    fn sight(&self) -> Option<Sight> {
        if self.claude.is_some() {
            return Some(Sight::Claude);
        }
        match &self.brain {
            Brain::Ready(m) => {
                let model: &'static model::Model<'static> = m.model;
                model.vision.as_ref().map(Sight::Model)
            }
            Brain::Script(_) => Some(Sight::Qwen),
            _ => None,
        }
    }

    /// Why the model can't see, when it can't: its file has no vision part; one made before the engine kept that
    /// part (by the converter from before) is made again by Chat Computer's app on Windows.
    fn blind(&self) -> &'static str {
        match &self.brain {
            Brain::Ready(m) if m.model.vision.is_none() && m.model.settings.converter < model::convert::CONVERTER => {
                "its file was made before Chat Computer's engine kept the part of the model that sees. Chat Computer's \
                 app on Windows makes it again (downloading the model again, in the background) when it's next opened \
                 there."
            }
            _ => "its file has no vision part.",
        }
    }

    /// The answer is complete.
    fn finish(&mut self, job: &mut Job, events: &mut Vec<Event>) {
        let chars = job.text.chars().count();
        let speed = (job.write_ms > 0).then(|| job.written as f64 * 1000.0 / job.write_ms as f64);
        let seen = match job.seen {
            0 => String::new(),
            1 => format!("; a picture seen in {:.1} s", job.see_ms as f64 / 1000.0),
            n => format!("; {n} pictures seen in {:.1} s", job.see_ms as f64 / 1000.0),
        };
        match &job.by {
            Some(who) => crate::log!("Chat Computer kernel: model: answered ({chars} characters, by {who}{seen})"),
            None => crate::log!(
                "Chat Computer kernel: model: answered ({chars} characters; {} tokens read in {:.1} s, {} written in {:.1} s{seen}{})",
                job.read,
                job.read_ms as f64 / 1000.0,
                job.written,
                job.write_ms as f64 / 1000.0,
                crate::cores::shared()
            ),
        }
        if let (Brain::Ready(m), Some(s)) = (&mut self.brain, speed) {
            m.speed = Some(s);
        }
        if test_build().is_some() {
            crate::log!("Chat Computer kernel: test: said {}", json::compact(&Json::Str(job.text.clone())));
        }
        if job.new_conversation {
            self.parts.clear();
            self.from = 0;
            self.system = self.system_prompt();
            self.start_ahead();
            crate::log!("Chat Computer kernel: model: a new conversation starts");
        }
        self.unkept = true;
        events.push(Event::Done);
        if job.new_conversation {
            events.push(Event::NewConversation);
        }
        if job.restart {
            events.push(Event::Restarting(5));
            crate::power::restart_in(5000);
        } else if job.turn_off {
            events.push(Event::TurningOff(5));
            crate::power::turn_off_in(5000);
        }
    }

    /// A step of a test script's answer: a round begun, a few of its words said, or the round over. True when the
    /// answer's over.
    fn script_step(&mut self, job: &mut Job, events: &mut Vec<Event>) -> bool {
        if !self.web_step(job, events) {
            return false;
        }
        if job.script.is_none() {
            let messages = self.messages();
            crate::log!("Chat Computer kernel: test: asked {}", json::compact(&messages_json(&messages)));
            let question = match self.parts.get(job.base) {
                Some(Part::You { text, .. }) => text.clone(),
                _ => String::new(),
            };
            let Brain::Script(script) = &self.brain else { return true };
            let rounds = script.get(&question).or_else(|| script.get("else"));
            let round = rounds.and_then(Json::array).and_then(|r| r.get(job.rounds as usize));
            if let Some(why) = round.and_then(|r| r.get("fail")).and_then(Json::str) {
                let why = String::from(why);
                self.parts.truncate(job.base);
                fail(events, why);
                return true;
            }
            let text = round.and_then(|r| r.get("text")).and_then(Json::str).unwrap_or(if round.is_some() { "" } else { "Done." });
            let calls = round.and_then(|r| r.get("calls")).and_then(Json::array).unwrap_or(&[]);
            let calls = calls
                .iter()
                .filter_map(|c| {
                    let c = c.array()?;
                    let name = String::from(c.first()?.str()?);
                    let args = c.get(1).and_then(Json::object).unwrap_or(&[]);
                    let args = args
                        .iter()
                        .map(|(k, v)| (k.clone(), v.str().map_or_else(|| json::compact(v), String::from)))
                        .collect();
                    Some(Call { name, args })
                })
                .collect();
            let answer = Answer { thinking: String::new(), text: String::from(text), calls };
            // ("cut": the round reached the most a round says, its last call, if it has calls, still being written.)
            let cut = round.and_then(|r| r.get("cut")).and_then(Json::bool).unwrap_or(false);
            job.closed = if cut { answer.calls.len().saturating_sub(1) } else { answer.calls.len() };
            job.script = Some((String::from(text), answer, cut));
        }
        let left = &mut job.script.as_mut().expect("the script's round").0;
        if !left.is_empty() {
            let cut = left.char_indices().nth(SCRIPT_PIECE).map_or(left.len(), |(i, _)| i);
            let piece: String = left.drain(..cut).collect();
            say(job, piece, events);
            return false;
        }
        let (_, answer, cut) = job.script.take().expect("the script's round");
        self.round_over(job, answer, cut, events)
    }
}

/// A page's words for the model, from character `offset`, at most PAGE_PART of them: from the start, with its title
/// and address; else its address and where; and how much more there is, or that it was cut at the most read.
fn page_part(url: &str, title: &str, words: &str, cut: bool, offset: usize) -> String {
    let total = words.chars().count();
    if total == 0 {
        return format!("{url}\n\n(The page has no words: it may be all pictures, or made by programs that run in a browser.)");
    }
    if offset >= total {
        return format!("{url}: that's past its end (it has {total} characters of words).");
    }
    let part: String = words.chars().skip(offset).take(PAGE_PART).collect();
    let end = offset + part.chars().count();
    let mut out = if offset == 0 {
        if title.is_empty() { format!("{url}\n\n") } else { format!("{title}\n{url}\n\n") }
    } else {
        format!("{url}, from character {offset}:\n\n")
    };
    out.push_str(&part);
    if end < total {
        out.push_str(&format!("\n\n(The page goes on for {} more characters: read_web_page with offset {end} reads on.)", total - end));
    } else if cut {
        out.push_str("\n\n(The page was longer than Chat Computer reads: that was all of its first 3 MB.)");
    }
    out
}

/// The request for a round: the tools, the messages, no thinking first (on one processor core, a model that thought
/// first took minutes to answer; on all of them, still to be measured on a PC).
fn request<'a>(tools: &'a [Json], messages: &'a [Message], seed: u64, sampling: Sampling) -> Request<'a> {
    Request {
        tools,
        messages,
        options: Options { think: Some(false), keep_thinking: Some(true), ..Options::default() },
        sampling,
        max_tokens: if test_build() == Some("model") { TEST_ROUND_TOKENS } else { ROUND_TOKENS },
        seed,
    }
}

/// Words the model said, for the screen.
fn say(job: &mut Job, words: String, events: &mut Vec<Event>) {
    if words.is_empty() {
        return;
    }
    if job.separate {
        job.separate = false;
        job.text.push_str("\n\n");
        events.push(Event::Text(String::from("\n\n")));
    }
    job.said = true;
    job.text.push_str(&words);
    events.push(Event::Text(words));
}

/// What it's doing, shown while it hasn't said anything yet (each change once).
fn activity(job: &mut Job, doing: String, events: &mut Vec<Event>) {
    if !job.said && job.shown != doing {
        job.shown = doing.clone();
        events.push(Event::Activity(doing));
    }
}

/// How much a step does next (at most `most`): more while steps are quick, less when they're slow (than `target`
/// milliseconds).
fn suited_to(n: usize, took_ms: u64, target: u64, most: usize) -> usize {
    if took_ms < target / 2 {
        (n * 2).min(most)
    } else if took_ms > target * 2 {
        (n / 2).max(1)
    } else {
        n
    }
}

/// What's typed in the message box now (the chat says, as it changes), for reading ahead.
static TYPED: SpinLock<String> = SpinLock::new(String::new());

/// The message box's words changed (what's sent would be them, trimmed): read ahead as far as they can't change.
pub fn typing(text: &str) {
    let mut t = TYPED.lock();
    if *t != text {
        t.clear();
        t.push_str(text);
    }
}

/// The words typed that can't change as more is typed: all but the last (the words before the last space), from the
/// first that isn't a space. None of a search ("/"), nor what may be a BitLocker recovery key (which is never given to
/// the model): its groups of six digits.
fn sure_words(typed: &str) -> String {
    let t = typed.trim_start();
    if t.starts_with('/') {
        return String::new();
    }
    let digits = t.as_bytes().windows(7).any(|w| w[..6].iter().all(u8::is_ascii_digit) && w[6] == b'-');
    if digits {
        return String::new();
    }
    match t.rfind(char::is_whitespace) {
        Some(i) => String::from(t[..i].trim_end()),
        None => String::new(),
    }
}

/// The arithmetic the engine uses on this processor (model::cpu): "AVX2", say.
pub fn level_name() -> &'static str {
    match model::cpu::level() {
        Level::Plain => "plain",
        Level::Avx2 => "AVX2",
        Level::AvxVnni => "AVX-VNNI",
        Level::Avx512Vnni => "AVX-512 VNNI",
    }
}

fn percent(done: usize, total: usize) -> usize {
    if total == 0 { 100 } else { (done.min(total) * 100 / total).min(99) }
}

fn fail(events: &mut Vec<Event>, why: String) {
    crate::log!("Chat Computer kernel: model: failed: {why}");
    events.push(Event::Failed(why));
}

/// "1.2 GB", "620 MB".
fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{}.{} GB", bytes / 1_000_000_000, bytes / 100_000_000 % 10)
    } else {
        format!("{} MB", bytes.div_ceil(1_000_000))
    }
}

/// "C:" from "C:\Users\...".
fn drive_of(path: &str) -> &str {
    path.get(..2).filter(|d| d.ends_with(':')).unwrap_or("its drive")
}

/// The model's name from its file's ("qwen3.5-4b.ccm": "Qwen3.5 4B").
fn name_of(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".ccm").or_else(|| file.strip_suffix(".CCM")).unwrap_or(file);
    match stem.split_once('-') {
        Some((family, size)) => format!("{} {}", family.replacen("qwen", "Qwen", 1), size.to_uppercase()),
        None => String::from(stem),
    }
}

/// The memory a model may have: what's free in the heap now, and what it could still grow by.
fn room() -> u64 {
    heap::could_grow() + heap::largest_free() as u64
}

fn too_big(name: &str, size: u64) -> String {
    format!(
        "This PC's memory isn't enough for Chat Computer's model, {name} ({}): it needs about {}, and {} is free. On \
         Windows, Chat Computer's app can get a smaller model ready.",
        size_text(size),
        size_text(size + RUN_ROOM),
        size_text(room())
    )
}

/// Which of the models Chat Computer's app got ready to run: the biggest that's quick enough (ONE_CORE_MOST) and fits
/// in memory (else the smallest that fits). Err: why there's none, and whether to look again later.
fn choose() -> Result<String, (String, bool)> {
    let found = files::app_models().map_err(|_| {
        let why = if files::windows_locked() { LOCKED } else { NO_MODEL };
        (String::from(why), true)
    })?;
    let room = room();
    let fits: Vec<&(String, u64)> = found.iter().filter(|(_, size)| size + RUN_ROOM <= room).collect();
    let quick = fits.iter().filter(|(_, size)| *size <= ONE_CORE_MOST).max_by_key(|(_, size)| *size);
    match quick.or_else(|| fits.iter().min_by_key(|(_, size)| *size)) {
        Some((path, _)) => Ok(path.clone()),
        None => match found.iter().min_by_key(|(_, size)| *size) {
            Some((path, size)) => Err((too_big(&name_of(path), *size), false)),
            None => Err((String::from(NO_MODEL), true)),
        },
    }
}

/// Opens the model's file, with the memory to read it into. Err: why not, and whether to look again later.
fn start_reading(path: &str) -> Result<Reading, (String, bool)> {
    let file = files::open(path).map_err(|why| (format!("Chat Computer's model can't be read: {why}."), true))?;
    let name = name_of(&file.path);
    if file.size + RUN_ROOM > room() {
        return Err((too_big(&name, file.size), false));
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(file.size as usize).is_err() {
        return Err((too_big(&name, file.size), false));
    }
    Ok(Reading { file, name, bytes, chunk: FIRST_CHUNK, started: time::uptime_ms() })
}

/// Reads more of the model's file, until `until`: true once it's all read.
fn read_more(r: &mut Reading, until: u64) -> Result<bool, String> {
    let total = r.file.size as usize;
    loop {
        let at = r.bytes.len();
        if at >= total {
            return Ok(true);
        }
        let want = (total - at).min(r.chunk);
        // (Within what was reserved: the bytes don't move.)
        r.bytes.resize(at + want, 0);
        let t = time::uptime_ms();
        let n = files::read_at(&r.file, at as u64, &mut r.bytes[at..])?;
        r.bytes.truncate(at + n);
        if n == 0 {
            return Err(format!("{}: the file ended sooner than it said", r.file.path));
        }
        let took = time::uptime_ms() - t;
        if took < STEP_MS / 4 {
            r.chunk = (r.chunk * 2).min(MOST_CHUNK);
        } else if took > STEP_MS * 2 {
            r.chunk = (r.chunk / 2).max(LEAST_CHUNK);
        }
        if time::uptime_ms() >= until {
            return Ok(r.bytes.len() >= total);
        }
    }
}

/// The model, from its file's bytes (kept in memory for good once it's ready; given back if it can't be).
fn ready_model(r: Reading) -> Result<Loaded, String> {
    let Reading { file, name, bytes, .. } = r;
    let (len, cap) = (bytes.len(), bytes.capacity());
    let ptr = Vec::leak(bytes).as_mut_ptr();
    let all: &'static [u8] = unsafe { core::slice::from_raw_parts(ptr, len) };
    match load(all, &file, name) {
        Ok(m) => Ok(m),
        Err(why) => {
            // (Nothing else holds the bytes: what load made from them is gone.)
            drop(unsafe { Vec::from_raw_parts(ptr, len, cap) });
            Err(why)
        }
    }
}

fn load(all: &'static [u8], opened: &files::Opened, name: String) -> Result<Loaded, String> {
    let bad = |why: String| format!("{} isn't a model Chat Computer's engine can run ({why})", opened.path);
    let file = File::open(all).map_err(|e| bad(format!("{e:?}")))?;
    let mut model = Model::load(&file).map_err(|e| bad(format!("{e:?}")))?;
    let style_name = file.config().ok().and_then(|c| c.str("chat.style").map(String::from)).unwrap_or_else(|| String::from("qwen3.5"));
    let style = Style::named(&style_name).ok_or_else(|| bad(format!("its chat style, {style_name}, isn't one it knows")))?;
    // Its refusals taken out, where they've been found (the .refusal file beside it, made for this very file).
    let refusal = match opened.path.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.refusal"),
        None => format!("{}.refusal", opened.path),
    };
    if let Ok(o) = files::open(&refusal)
        && o.size <= MOST_REFUSAL
    {
        let mut b = vec![0u8; o.size as usize];
        match files::read_at(&o, 0, &mut b) {
            Ok(n) if n == b.len() => match Found::from_bytes(&b, file.fingerprint(), model.settings.hidden) {
                Ok(found) => model.ablation = found.ablation(),
                Err(e) => crate::log!("Chat Computer kernel: model: {refusal} isn't used: {e}"),
            },
            _ => crate::log!("Chat Computer kernel: model: {refusal} couldn't be read"),
        }
    }
    // The longest conversation it's given: what the memory holds of the attention layers' keys and values (four bytes
    // a number; a growing list may hold up to twice what's in it, and needs room to move), after the state, the
    // snapshots session.rs keeps (and their scores), and what reading needs.
    let s = &model.settings;
    let full = (0..s.layers).filter(|&l| s.full_attention(l)).count().max(1);
    let per_token = 2 * full * s.kv_heads * s.head_dim * 4 * 3;
    let snapshot = model.state().snapshot().bytes() + 4 * s.vocab;
    let fixed = 7 * snapshot + (128 << 20);
    let room = (room() as usize).saturating_sub(fixed);
    let context = (room / per_token.max(1)).min(s.context).min(MOST_TOKENS);
    if context < LEAST_TOKENS {
        return Err(too_big(&name, opened.size));
    }
    let uncensored = model.ablation.is_some();
    let model: &'static Model<'static> = Box::leak(Box::new(model));
    Ok(Loaded {
        name,
        path: opened.path.clone(),
        session: Session::new(model, style),
        model,
        style_name,
        uncensored,
        context,
        ahead: None,
        ahead_of: None,
        ahead_done: 0,
        ahead_read: false,
        instructions_read: false,
        // (With other cores, a whole batch of the engine's a step: each of its weights read from memory serving them
        // all. With one, a few, so the screen goes on between steps.)
        read: if crate::cores::count() > 1 { READ_LEAST } else { 8 },
        see: 64,
        speed: None,
    })
}

/// The messages as JSON, for a test build's log.
fn messages_json(messages: &[Message]) -> Json {
    let s = |v: &str| Json::Str(String::from(v));
    Json::Array(
        messages
            .iter()
            .map(|m| match m {
                Message::System(t) => Json::Object(vec![(String::from("role"), s("system")), (String::from("content"), s(t))]),
                Message::User(t) => Json::Object(vec![(String::from("role"), s("user")), (String::from("content"), s(t))]),
                Message::Tool(t) => Json::Object(vec![(String::from("role"), s("tool")), (String::from("content"), s(t))]),
                // (Pictures as their size and mark: a test's log has no room for their pixels.)
                Message::UserParts(parts) | Message::ToolParts(parts) => {
                    let role = if matches!(m, Message::UserParts(_)) { "user" } else { "tool" };
                    let text: String = parts
                        .iter()
                        .map(|p| match p {
                            Content::Text(t) => t.clone(),
                            Content::Picture(pic) => format!("[a picture, {}]", picture_text(pic)),
                        })
                        .collect();
                    Json::Object(vec![(String::from("role"), s(role)), (String::from("content"), s(&text))])
                }
                Message::Assistant(a) => Json::Object(vec![
                    (String::from("role"), s("assistant")),
                    (String::from("content"), s(&a.text)),
                    (String::from("calls"), calls_json(&a.calls)),
                ]),
            })
            .collect(),
    )
}

/// A picture as a test's log has it: its size, and its mark (a hash of its size and pixels).
fn picture_text(p: &Picture) -> String {
    format!("{}x{} {:016x}", p.width, p.height, p.mark)
}

/// Tool calls as JSON, for a test build's log: each {"name": ..., "args": {name: value as the model wrote it}}.
fn calls_json(calls: &[Call]) -> Json {
    Json::Array(
        calls
            .iter()
            .map(|c| {
                let args = c.args.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect();
                Json::Object(vec![(String::from("name"), Json::Str(c.name.clone())), (String::from("args"), Json::Object(args))])
            })
            .collect(),
    )
}

/// The most of the conversation sent to Claude at once (its JSON), the oldest questions left out past it.
const CLAUDE_SENT_BYTES: usize = 900_000;

fn claude_object(fields: &[(&str, claude::json::Json)]) -> claude::json::Json {
    claude::json::Json::Obj(fields.iter().map(|(k, v)| (String::from(*k), v.clone())).collect())
}

/// Claude's whole answer to a round as the conversation keeps it: its words, and the tools it called (each argument
/// as a string, as Chat Computer's own model writes them: Call::arguments reads them back by the tool's types), and
/// its content blocks as they came.
fn claude_answer(a: &crate::remote::Answered) -> (Answer, String) {
    let text: String = a
        .content
        .iter()
        .filter(|b| b.get("type").and_then(|t| t.str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|t| t.str()))
        .collect::<Vec<_>>()
        .join("");
    let calls = a
        .calls
        .iter()
        .map(|u| Call {
            name: u.name.clone(),
            args: match &u.input {
                claude::json::Json::Obj(fields) => fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.str().map_or_else(|| v.write(), String::from)))
                    .collect(),
                _ => Vec::new(),
            },
        })
        .collect();
    let raw = claude::json::Json::Arr(a.content.clone()).write();
    (Answer { text, calls, ..Answer::default() }, raw)
}

/// What the memory file keeps: the model's notes and the conversation, as {"notes": [...], "chat": [{"you": words,
/// "at": seconds since 1970}, {"said": words}, ...]}: each question, and the words of its answer (not the tools it
/// used along the way).
/// The apps as they're kept: each one's name, page and data (its localStorage now).
fn kept_apps(apps: &[App]) -> Json {
    Json::Array(
        apps.iter()
            .map(|app| {
                let data = crate::site::storage_of(&crate::site::app_origin(&app.name)).unwrap_or_default();
                let list = |v: &[String]| Json::Array(v.iter().map(|h| Json::Str(h.clone())).collect());
                Json::Object(vec![
                    (String::from("name"), Json::Str(app.name.clone())),
                    (String::from("html"), Json::Str(app.html.clone())),
                    (String::from("data"), Json::Object(data.into_iter().map(|(k, v)| (k, Json::Str(v))).collect())),
                    (String::from("sites"), list(&app.sites)),
                    (String::from("allowed"), list(&app.allowed)),
                    (String::from("because"), Json::Str(app.because.clone())),
                ])
            })
            .collect(),
    )
}

fn kept_text(notes: &[String], parts: &[Part], ai: Option<Json>, sides: [bool; 2], apps: &Json) -> String {
    let mut chat: Vec<Json> = Vec::new();
    for p in parts {
        match p {
            Part::You { text, at } => {
                let mut item = vec![(String::from("you"), Json::Str(text.clone()))];
                if let Some(t) = at {
                    item.push((String::from("at"), Json::Number(*t as f64)));
                }
                chat.push(Json::Object(item));
            }
            Part::Said(answer, _) if !answer.text.trim().is_empty() => {
                let words = answer.text.trim();
                match chat.last_mut() {
                    Some(Json::Object(item)) if item.first().is_some_and(|(k, _)| k == "said") => {
                        if let Some((_, Json::Str(said))) = item.first_mut() {
                            said.push_str("\n\n");
                            said.push_str(words);
                        }
                    }
                    _ => chat.push(Json::Object(vec![(String::from("said"), Json::Str(String::from(words)))])),
                }
            }
            _ => {}
        }
    }
    let notes = Json::Array(notes.iter().map(|n| Json::Str(n.clone())).collect());
    let mut kept = vec![(String::from("notes"), notes), (String::from("chat"), Json::Array(chat))];
    if let Some(ai) = ai {
        kept.push((String::from("ai"), ai));
    }
    if sides != [true, true] {
        kept.push((String::from("sides"), Json::Array(vec![Json::Bool(sides[0]), Json::Bool(sides[1])])));
    }
    if matches!(apps, Json::Array(list) if !list.is_empty()) {
        kept.push((String::from("apps"), apps.clone()));
    }
    json::compact(&Json::Object(kept))
}

/// The notes and conversation in the memory file's text; also as Chat Computer's own system kept them when it talked
/// to Claude's API ({"notes": [...], "messages": [the API's messages]}, or the messages alone), their words taken.
fn from_kept(text: &str) -> Option<(Vec<String>, Vec<Part>)> {
    let kept = json::parse(text)?;
    let notes = match kept.get("notes") {
        Some(Json::Array(items)) => items.iter().filter_map(|n| n.str().map(String::from)).collect(),
        _ => Vec::new(),
    };
    let mut parts: Vec<Part> = Vec::new();
    let said = |parts: &mut Vec<Part>, words: &str| {
        let words = words.trim();
        if words.is_empty() {
            return;
        }
        match parts.last_mut() {
            Some(Part::Said(a, _)) => {
                a.text.push_str("\n\n");
                a.text.push_str(words);
            }
            _ => parts.push(Part::Said(Answer { text: String::from(words), ..Answer::default() }, None)),
        }
    };
    if let Some(chat) = kept.get("chat").and_then(Json::array) {
        for item in chat {
            if let Some(you) = item.get("you").and_then(Json::str) {
                let at = item.get("at").and_then(Json::num).map(|t| t as i64);
                parts.push(Part::You { text: String::from(you), at });
            } else if let Some(words) = item.get("said").and_then(Json::str) {
                said(&mut parts, words);
            }
        }
        return Some((notes, parts));
    }
    let messages = match &kept {
        Json::Array(m) => m.as_slice(),
        _ => kept.get("messages")?.array()?,
    };
    for m in messages {
        match (m.get("role").and_then(Json::str), m.get("content")) {
            (Some("user"), Some(Json::Str(t))) => parts.push(Part::You { text: t.clone(), at: None }),
            (Some("assistant"), Some(Json::Str(t))) => said(&mut parts, t),
            (Some("assistant"), Some(Json::Array(blocks))) => {
                for b in blocks.iter().filter(|b| b.get("type").and_then(Json::str) == Some("text")) {
                    said(&mut parts, b.get("text").and_then(Json::str).unwrap_or(""));
                }
            }
            _ => {}
        }
    }
    Some((notes, parts))
}

/// Runs a tool the model called: what it says back, whether that's an error, and a picture it gives the model to see
/// (read_file's, when the model can: `sight`; else why it can't, `blind`).
#[allow(clippy::too_many_arguments)]
fn run_tool(
    name: &str,
    input: &Json,
    job: &mut Job,
    notes: &mut Vec<String>,
    apps: &mut Vec<App>,
    because: &str,
    events: &mut Vec<Event>,
    sight: Option<Sight>,
    blind: &str,
) -> (String, bool, Option<picture::Picture>) {
    if name == "read_file" {
        activity(job, String::from("Reading a file"), events);
        crate::log!("Chat Computer kernel: model: uses {name}");
        let text = input.get("path").and_then(Json::str).map(str::trim).unwrap_or("");
        let num = |key: &str| input.get(key).and_then(number);
        let page = num("page").filter(|p| *p >= 1.0).map(|p| p as u64);
        let size = |w: usize, h: usize| sight.map_or((w, h), |s| s.size(w, h));
        return match crate::files::read(text, num("offset").unwrap_or(0.0).max(0.0) as u64, page, sight.map(|_| &size as crate::files::Sight), blind) {
            Ok(got) => (got.said, false, got.picture),
            Err(e) => (e, true, None),
        };
    }
    if matches!(name, "make_app" | "open_app") {
        let (said, error) = app_tool(name, input, apps, because, events);
        return (said, error, None);
    }
    let (said, error) = run_other_tool(name, input, job, notes, events);
    (said, error, None)
}

/// make_app and open_app: an app made (or changed) and opened, or one made before opened.
fn app_tool(name: &str, input: &Json, apps: &mut Vec<App>, because: &str, events: &mut Vec<Event>) -> (String, bool) {
    crate::log!("Chat Computer kernel: model: uses {name}");
    let text = |key: &str| input.get(key).and_then(Json::str).unwrap_or("");
    let app_name: String = text("name").trim().chars().take(40).collect();
    let names = |apps: &[App]| apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
    let found = apps.iter().position(|a| a.name.eq_ignore_ascii_case(&app_name));
    if name == "open_app" {
        return match found {
            Some(i) => {
                events.push(Event::App(apps[i].name.clone(), apps[i].html.clone()));
                (format!("The {} app is open in the middle of the screen.", apps[i].name), false)
            }
            None if apps.is_empty() => (String::from("You haven't made any apps for the person yet (make_app makes one)."), true),
            None => (format!("There's no app called {app_name:?}. The apps: {}.", names(apps)), true),
        };
    }
    let html = text("html");
    if app_name.is_empty() {
        return (String::from("name must be the app's name, a few words"), true);
    }
    if !html.to_ascii_lowercase().contains("<") {
        return (String::from("html must be the app's whole page: <!doctype html>, its <style> and its <script>"), true);
    }
    if html.len() > APP_BYTES {
        return (format!("An app's page can be at most {} KB: make it smaller.", APP_BYTES >> 10), true);
    }
    // The sites it needs (none: it reaches no site).
    let mut sites: Vec<String> = Vec::new();
    match input.get("sites") {
        None | Some(Json::Null) => {}
        Some(Json::Array(list)) => {
            for x in list {
                match x.str().and_then(app_site) {
                    Some(h) if !sites.contains(&h) => sites.push(h),
                    Some(_) => {}
                    None => return (format!("sites must be the host names the app fetches from (\"api.example.com\"); {x:?} isn't one"), true),
                }
            }
            if sites.len() > MOST_APP_SITES {
                return (format!("An app can reach at most {MOST_APP_SITES} sites."), true);
            }
        }
        Some(_) => return (String::from("sites must be a list of host names (\"api.example.com\")"), true),
    }
    let ask = if sites.is_empty() {
        String::new()
    } else {
        format!(
            " It can't reach {} until the person allows it, with the Allow button over it (only they can): tell them \
             what it fetches from there, and why.",
            sites.join(", ")
        )
    };
    let said = match found {
        Some(i) => {
            let app = &mut apps[i];
            app.html = String::from(html);
            // (Sites it already had stay allowed; new ones wait for the person.)
            app.allowed.retain(|h| sites.contains(h));
            let pending = sites.iter().any(|h| !app.allowed.contains(h));
            app.sites = sites;
            app.because = String::from(because);
            format!("The {} app is changed (its data kept), and open in the middle of the screen.{}", app.name, if pending { &ask } else { "" })
        }
        None if apps.len() >= MOST_APPS => return (format!("There's no room for more apps ({MOST_APPS} at most): change one of them instead."), true),
        None => {
            apps.push(App { name: app_name.clone(), html: String::from(html), sites, allowed: Vec::new(), because: String::from(because) });
            format!(
                "The {app_name} app is made, and open in the middle of the screen; it stays in the Apps list at the \
                 left, for the person to open again.{ask}"
            )
        }
    };
    let app = apps.iter().find(|a| a.name.eq_ignore_ascii_case(&app_name)).cloned().expect("just made");
    crate::log!("Chat Computer kernel: model: the {} app ({} bytes)", app.name, app.html.len());
    events.push(Event::App(app.name, app.html));
    events.push(Event::Apps(apps.iter().map(|a| a.name.clone()).collect()));
    (said, false)
}

/// The apps' names (for the Apps list), and one's page.
pub fn apps() -> Vec<String> {
    ASSISTANT.lock().as_ref().map(|a| a.apps.iter().map(|app| app.name.clone()).collect()).unwrap_or_default()
}

pub fn app(name: &str) -> Option<String> {
    ASSISTANT.lock().as_ref().and_then(|a| a.apps.iter().find(|app| app.name == name).map(|app| app.html.clone()))
}

/// The sites an app says it needs, those the person allowed, and what it was made for.
pub fn app_sites(name: &str) -> (Vec<String>, Vec<String>, String) {
    ASSISTANT
        .lock()
        .as_ref()
        .and_then(|a| a.apps.iter().find(|app| app.name == name).map(|app| (app.sites.clone(), app.allowed.clone(), app.because.clone())))
        .unwrap_or_default()
}

/// The person allowed an app to reach the sites it needs (they pressed its Allow button): kept, and given back.
pub fn allow_app(name: &str) -> Vec<String> {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return Vec::new() };
    let Some(app) = a.apps.iter_mut().find(|app| app.name == name) else { return Vec::new() };
    app.allowed = app.sites.clone();
    let allowed = app.allowed.clone();
    a.unkept = true;
    allowed
}

/// An app's data changed (its scripts' localStorage): kept again soon.
pub fn app_data_changed() {
    if let Some(a) = ASSISTANT.lock().as_mut() {
        a.app_data = true;
    }
}

/// Runs a tool other than read_file.
fn run_other_tool(name: &str, input: &Json, job: &mut Job, notes: &mut Vec<String>, events: &mut Vec<Event>) -> (String, bool) {
    crate::log!("Chat Computer kernel: model: uses {name}");
    let text = |key: &str| input.get(key).and_then(Json::str).map(str::trim).unwrap_or("");
    let num = |key: &str| input.get(key).and_then(number);
    match name {
        "computer_status" => (crate::system::status(), false),
        "set_text_size" => match num("percent") {
            Some(p) if (50.0..=300.0).contains(&p) && p == (p as i64) as f64 => {
                events.push(Event::TextSize(p as u32));
                (format!("The text is now {p}% of the size chosen for this screen."), false)
            }
            _ => (String::from("percent must be a whole number from 50 to 300"), true),
        },
        "restart_computer" => {
            job.restart = true;
            (String::from("The PC restarts a few seconds after your answer: tell the person."), false)
        }
        "turn_off_computer" if crate::power::can_turn_off() => {
            job.turn_off = true;
            (String::from("The PC turns off a few seconds after your answer: tell the person."), false)
        }
        "turn_off_computer" => (
            String::from(
                "Chat Computer can't turn this PC off: its firmware doesn't say how. The person can hold the power \
                 button for a few seconds instead.",
            ),
            true,
        ),
        "remember" => {
            let note = text("note");
            let total: usize = notes.iter().map(|n| n.chars().count()).sum();
            if note.is_empty() {
                (String::from("note must be a sentence about the person"), true)
            } else if note.chars().count() > NOTE_CHARS {
                (format!("A note can be at most {NOTE_CHARS} characters: make it shorter."), true)
            } else if notes.iter().any(|n| n == note) {
                (String::from("That's noted already."), false)
            } else if notes.len() >= MAX_NOTES || total + note.chars().count() > NOTES_CHARS {
                (String::from("There's no room for more notes: forget one that matters less first (ask the person)."), true)
            } else {
                notes.push(String::from(note));
                events.push(Event::Noted(String::from(note)));
                (String::from("Noted: you'll see it in every new conversation from now on."), false)
            }
        }
        "forget" => {
            let words = text("words").to_lowercase();
            let before = notes.len();
            if !words.is_empty() {
                notes.retain(|n| !n.to_lowercase().contains(&words));
            }
            match before - notes.len() {
                0 => (String::from("No note has those words."), true),
                n => {
                    events.push(Event::Forgot(n));
                    (format!("Forgot {n} note{}.", if n == 1 { "" } else { "s" }), false)
                }
            }
        }
        "reminders" => {
            let said = match text("action") {
                "timer" => {
                    let seconds = num("minutes").unwrap_or(0.0) * 60.0 + num("seconds").unwrap_or(0.0);
                    let label = if text("label").is_empty() { text("text") } else { text("label") };
                    crate::schedule::timer(seconds, label)
                }
                kind @ ("reminder" | "alarm") => {
                    let label = if text("text").is_empty() { text("label") } else { text("text") };
                    crate::schedule::reminder(kind == "alarm", label, num("in_minutes"), Some(text("at")))
                }
                "list" => Ok(crate::schedule::list()),
                "cancel" => crate::schedule::cancel(text("id")),
                "stop" => Ok(crate::schedule::stop()),
                other => Err(format!("{other:?} isn't one of timer, reminder, alarm, list, cancel or stop.")),
            };
            match said {
                Ok(s) => (s, false),
                Err(e) => (e, true),
            }
        }
        "set_volume" => match num("percent") {
            Some(p) if (0.0..=100.0).contains(&p) => match crate::sound::set_volume(p as u32) {
                Some(v) => (format!("The chimes are at {v}% now."), false),
                None => (format!("This computer has no sound Chat Computer's own system can use: {}", crate::sound::status()), true),
            },
            _ => (String::from("percent must be from 0 to 100"), true),
        },
        "show_picture" => {
            let (w, h) = crate::chat::picture_box();
            match crate::files::picture(text("path"), w, h, crate::draw::GROUND) {
                Ok((said, picture)) => {
                    events.push(Event::Picture(picture));
                    (said, false)
                }
                Err(why) => (why, true),
            }
        }
        "list_files" | "find_files" => {
            activity(job, String::from("Looking through your files"), events);
            let said = match name {
                "list_files" => crate::files::list(text("path")),
                _ => crate::files::find(text("words"), text("in")),
            };
            match said {
                Ok(s) => (s, false),
                Err(e) => (e, true),
            }
        }
        "start_new_conversation" => {
            job.new_conversation = true;
            (String::from("A new conversation starts when your answer ends: say so, in a few words."), false)
        }
        other => (format!("There's no tool called {other}."), true),
    }
}

/// A number the model gave (3, 2.5, or "3" in words), if it's one.
fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => Some(*n),
        Json::Str(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|x| x.is_finite())
}
