//! Chat Computer's own model, answering on this PC's own processor: an open model of Qwen's (published by Alibaba
//! under the Apache 2.0 licence), run by Chat Computer's own engine (os/model), from its file on Windows' drive: the
//! one Chat Computer's app got ready there (or the one the settings name), read into memory at start-up. Nothing
//! anyone types leaves this PC, and nothing is paid for.
//!
//! The model answers a step at a time, from the main loop: part of what it's asked read, or one token written, a step
//! (a few hundredths of a second, as far as the processor allows), so the screen and the keyboard go on while it reads
//! and writes. Before anyone asks anything, it reads its instructions (the system prompt, with its tools) ahead, so
//! the first question has only itself to be read. Each message it's given starts with when it was sent.
//!
//! It acts on this computer through its tools (tools.json): it sees how the computer's doing, changes the text size,
//! sets timers, reminders and alarms, reads the files on its drives (and sees their pictures, scanned pages among
//! them, when its file has the vision part: a picture a tool gives it goes with what the tool said, and its vision
//! part looks at it a step at a time), shows a picture, notes things about the person,
//! starts a new conversation, restarts the computer or turns it off. When an answer calls them, they run here and what
//! they say goes back to the model, until it has finished. The conversation is kept across restarts in Chat
//! Computer's memory file (keep.rs), with the model's notes about the person, which it's shown in every conversation.
//!
//! With a key for Meta's Model API in the settings (meta-api-key), Meta's Muse Spark answers instead, over the
//! internet: each round of the answer is a POST of the conversation (OpenAI's form of it, with the same tools) to
//! https://api.meta.ai/v1/chat/completions, made by Chat Computer's own network stack and TLS 1.3 (web.rs), and its
//! words and tool calls come back the same way; the tools run here as they do for the model on the PC. What's said
//! goes to Meta then, and the status says so.
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
use model::session::{Ended, Request, Session, Step, Turn};
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
pub const NO_MODEL: &str = "Chat Computer's own system needs its model, which Chat Computer's app gets ready on Windows: \
open the app there once (it gets its model by itself), then start Chat Computer's own system again.";

/// Said when Windows' drive, where the model is, is locked.
const LOCKED: &str = "Windows' drive is locked with BitLocker, and Chat Computer's model is on it: type the drive's recovery \
key below to unlock it (48 digits, which Windows keeps in your Microsoft account: https://aka.ms/myrecoverykey).";

/// The tools, as the model's template writes them into its instructions (OpenAI's form).
const TOOLS: &str = include_str!("tools.json");

const SYSTEM: &str = "You are Chat Computer: you run this computer for the person using it, who talks to you by typing \
here. {who}

With your tools you can see how this computer is doing, change the text size, set timers, reminders and alarms (and \
how loud they ring), read the files on its drives (Windows' C: and the others, USB sticks and memory cards included; \
nothing can be changed){sees}, search the web and read web pages (when this computer is connected), show the person a \
picture, note things about them to remember in later conversations, start a new conversation, and restart this \
computer (to go back to Windows) or turn it off. Chat Computer's own system can't yet show websites, play any sound \
but its chimes (no music, videos or speech), use Wi-Fi or Bluetooth, change the screen's brightness, print or scan, \
install or run programs, or change, copy, move or delete files: for those, say plainly that it can't, and that \
Windows can (restart_computer goes back to it); don't suggest a way to do them here.

Use a tool rather than saying how. When you call tools, write nothing else in that message: every word you write is \
on the screen, so say what you found once, in your answer, when you're done. Look a file up with find_files, by a word \
of its name, rather than folder by folder; use computer_status only when they ask about this computer itself (the \
date and time are in their message). When the answer may be in their own papers (a letter, a bill, a lease, their \
notes), look in their files before asking them; for what changes (timetables, prices, news, the weather), search the \
web. If an answer depends on something they didn't say (which city, which currency), say what you took it to be. When \
they ask you to restart or turn off the computer, call the tool that does it.

Each message from the person starts with when they sent it, like [Tue 3 Nov 2026 19:02]. Answer in their language, \
briefly unless they ask for more. The screen shows Markdown (headings, bold and italic, lists, code, quotes and simple \
tables) but no links, and no pictures but the ones you show with show_picture, so write an address out in full. Don't \
make things up: say so when you don't know.";
/// Who answers, in the instructions: the model on this PC...
const LOCAL_WHO: &str = "You are an AI model running on this computer itself ({model}, an open model made by Qwen, run by \
Chat Computer's own engine on Chat Computer's own operating system), so you work without the internet, and nothing \
they say leaves the computer unless you look something up on the web, which they see.";
/// ...or Meta's, over the internet.
const META_WHO: &str = "You are {model}, Meta's AI model, answering over the internet through Meta's Model API: Chat \
Computer's own operating system reaches you with its own network stack and TLS, so what they say goes to Meta to be \
answered, and you can't answer while this computer isn't connected.";
/// Meta's Model API, and the model it asks when the settings don't say.
const META_API: &str = "https://api.meta.ai/v1/chat/completions";
const META_MODEL: &str = "muse-spark-1.3";
/// The most a round of Meta's model's answer says (its thinking included).
const META_TOKENS: usize = 8192;
/// How many characters of its words are shown at a time, so they appear as they would be written.
const META_PIECE: usize = 12;
/// How many times a round is asked again when Meta's Model API answers with what may pass (now and then it says it
/// has no such model, or that it's busy).
const META_TRIES: u32 = 4;
/// The most of the words written with a round's tool calls shown as what the model is doing (Meta's model).
const MOST_DOING: usize = 90;

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
/// About how long a step of work takes: the screen and the keyboard go on between steps.
const STEP_MS: u64 = 40;
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
}

impl Sight {
    /// The size (width, height) it reads a picture of `w` by `h` at, with at most SEEN_ROWS rows.
    fn size(self, w: usize, h: usize) -> (usize, usize) {
        let (h, w) = match self {
            Sight::Model(v) => v.size_for(h, w, SEEN_ROWS),
            Sight::Qwen => model::vision::smart_resize(h, w, 32, 65_536, SEEN_ROWS * 32 * 32),
        };
        (w, h)
    }
}

/// A part of the conversation.
#[derive(Clone)]
enum Part {
    /// What the person typed, and when (seconds since 1970, when the clock was known).
    You { text: String, at: Option<i64> },
    /// A round of the model's answer: its words, and the tools it called.
    Said(Answer),
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
    session: Session<'static>,
    style_name: String,
    /// Its refusals are taken out (its .refusal file beside it).
    uncensored: bool,
    /// The longest conversation it's given, in tokens.
    context: usize,
    /// Its instructions' tokens, to read ahead of the first question (empty once read), and how many are read.
    ahead: Vec<u32>,
    ahead_done: usize,
    /// How many tokens a reading step takes, and how many of a picture's patches a step of seeing it (each suited
    /// to the processor's speed).
    read: usize,
    see: usize,
    /// Tokens written a second, in its last answer.
    speed: Option<f64>,
}

/// The model, as far as it's got.
enum Brain {
    /// No model: why, whether to look again (a drive may be unlocked, or the model got ready), and since when.
    None { why: String, again: bool, since: u64 },
    Reading(Box<Reading>),
    Ready(Box<Loaded>),
    /// A test build's script, in place of a model.
    Script(Json),
    /// Meta's model, over Meta's Model API.
    Meta(Box<Meta>),
}

/// Meta's model: the key for Meta's Model API (never logged), which model, and its name.
struct Meta {
    key: String,
    model: String,
    name: String,
    /// How long its last answer's round took to come.
    took_ms: Option<u64>,
}

/// A question being answered.
struct Job {
    /// Where it is in the conversation (a failed or stopped question is taken back out).
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
    /// The model's turn in the round being answered, once it's begun.
    turn: Option<Turn>,
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
    /// Meta's model's round, being asked: the request on its way, and since when; and how many times it's been asked.
    meta: Option<(crate::web::Fetch, u64)>,
    meta_tries: u32,
    /// Meta's model was asked again once already, its answer having said it turns off or restarts the PC without
    /// the tool that does it.
    reasked: bool,
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
    fn new(base: usize) -> Job {
        Job {
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
            meta: None,
            meta_tries: 0,
            reasked: false,
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
    tools: Vec<Json>,
    /// The conversation, and where the part the model's given starts (the oldest is left out once it's too long).
    parts: Vec<Part>,
    from: usize,
    /// What the model noted about the person (its remember tool), shown to it in every conversation.
    notes: Vec<String>,
    /// The system prompt for this conversation: with the notes as they were when it began, so it stays the same, and
    /// is read once.
    system: String,
    job: Option<Job>,
    /// The conversation changed since it was last kept (keep.rs).
    unkept: bool,
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
    /// Meta's model answers, over the internet (not the model on this PC).
    remote: bool,
}

static ASSISTANT: SpinLock<Option<Assistant>> = SpinLock::new(None);
static SHOWN: SpinLock<Option<Shown>> = SpinLock::new(None);

/// Starts: finds the model's file (the one `setting` names, if it names one), and starts reading it; or, given a key
/// for Meta's Model API (and which model, if the settings say), Meta's model answers.
pub fn init(setting: Option<String>, meta: Option<(String, Option<String>)>) {
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
    // The model's notes and the conversation from before the last restart, if they were kept.
    let (notes, parts) = crate::keep::load()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| from_kept(&text))
        .unwrap_or_default();
    let parts = if parts.first().is_none_or(|p| matches!(p, Part::You { .. })) { parts } else { Vec::new() };
    if !parts.is_empty() {
        crate::log!("Chat Computer kernel: model: the conversation from before the restart ({} parts)", parts.len());
    }
    if !notes.is_empty() {
        let n = notes.len();
        crate::log!("Chat Computer kernel: model: {n} note{} from before", if n == 1 { "" } else { "s" });
    }
    let mut a = Assistant {
        brain: Brain::None { why: String::new(), again: false, since: 0 },
        page: None,
        setting,
        tools,
        parts,
        from: 0,
        notes,
        system: String::new(),
        job: None,
        unkept: false,
    };
    if test_build() == Some("chat") {
        let script = core::str::from_utf8(TEST_SCRIPT).ok().and_then(json::parse).unwrap_or(Json::Object(Vec::new()));
        a.brain = Brain::Script(script);
        a.system = a.system_prompt();
        crate::log!("Chat Computer kernel: model: a test's script answers, in place of a model");
    } else if let Some((key, model)) = meta {
        let model = model.unwrap_or_else(|| String::from(META_MODEL));
        let name = if model.starts_with("muse-spark") { String::from("Muse Spark") } else { model.clone() };
        crate::log!("Chat Computer kernel: model: Meta's {name} ({model}) answers, over Meta's Model API");
        a.brain = Brain::Meta(Box::new(Meta { key, model, name, took_ms: None }));
        a.system = a.system_prompt();
    } else {
        a.look();
    }
    a.show();
    *ASSISTANT.lock() = Some(a);
}

/// Whether the model can be asked now.
pub fn ready() -> Ready {
    SHOWN.lock().as_ref().map_or_else(|| Ready::No(String::from(NO_MODEL)), |s| s.ready.clone())
}

/// The model's line in the computer's status.
pub fn status() -> String {
    SHOWN.lock().as_ref().map_or_else(|| String::from("Model: none"), |s| s.status.clone())
}

/// Whether Meta's model answers, over the internet (what's typed goes to Meta), rather than the model on this PC.
pub fn remote() -> bool {
    SHOWN.lock().as_ref().is_some_and(|s| s.remote)
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
            Part::Said(answer) => {
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
        if !a.unkept || a.job.is_some() {
            return;
        }
        a.unkept = false;
        let capacity = crate::keep::capacity();
        if capacity == 0 {
            return;
        }
        let mut start = 0;
        loop {
            let text = kept_text(&a.notes, &a.parts[start..]);
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
    if let Brain::None { why, .. } = &a.brain {
        return Err(why.clone());
    }
    // (A model test build's "look at": a conversation of its own, as check.py asks the engine on Linux it: an earlier
    // answer of random weights isn't text that would read back as the same tokens.)
    if test_build() == Some("model") && text.starts_with("look at ") {
        a.parts.clear();
    }
    // A new conversation's instructions have the notes as they are now.
    if a.parts.is_empty() && matches!(a.brain, Brain::Ready(_) | Brain::Script(_) | Brain::Meta(_)) {
        a.from = 0;
        let system = a.system_prompt();
        if system != a.system {
            a.system = system;
            a.start_ahead();
        }
    }
    let base = a.parts.len();
    a.parts.push(Part::You { text: String::from(text), at: clock::now() });
    a.job = Some(Job::new(base));
    a.show();
    Ok(())
}

/// Stops the answer on its way (the question taken back out).
pub fn stop() {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    let Some(mut job) = a.job.take() else { return };
    if let (Some(mut turn), Brain::Ready(m)) = (job.turn.take(), &mut a.brain) {
        m.session.stop(&mut turn);
        m.session.end(turn);
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
            Brain::Ready(m) => {
                if self.job.is_some() {
                    m.ahead.clear();
                    self.answer(until, events);
                } else if !m.ahead.is_empty() {
                    while !m.ahead.is_empty() && time::uptime_ms() < until {
                        let t = time::uptime_ms();
                        if m.session.read_ahead(&m.ahead, m.read, &crate::cores::Cores) {
                            crate::log!("Chat Computer kernel: model: read its instructions ahead ({} tokens)", m.ahead.len());
                            m.ahead.clear();
                        } else {
                            m.ahead_done = (m.ahead_done + m.read).min(m.ahead.len());
                            m.read = suited(m.read, time::uptime_ms() - t);
                        }
                    }
                }
            }
            Brain::Meta(_) => {
                if let Some(mut job) = self.job.take() {
                    let over = loop {
                        if self.meta_step(&mut job, events) {
                            break true;
                        }
                        if job.script.as_ref().is_some_and(|(left, ..)| !left.is_empty())
                            || job.meta.is_some()
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

    /// What the rest of the kernel sees, brought up to date.
    fn show(&self) {
        let ready = match &self.brain {
            Brain::None { why, .. } => Ready::No(why.clone()),
            Brain::Reading(r) => Ready::Getting(format!(
                "Getting Chat Computer's own model ready: reading {} from {} ({}%)\u{2026}",
                r.name,
                drive_of(&r.file.path),
                percent(r.bytes.len(), r.file.size as usize)
            )),
            Brain::Ready(m) if !m.ahead.is_empty() => Ready::Getting(format!(
                "Getting Chat Computer's own model ready: {} is reading its instructions ({}%)\u{2026}",
                m.name,
                percent(m.ahead_done, m.ahead.len())
            )),
            Brain::Ready(m) => Ready::Yes(m.name.clone()),
            Brain::Script(_) => Ready::Yes(String::from("a test's script")),
            Brain::Meta(m) => Ready::Yes(m.name.clone()),
        };
        let status = match &self.brain {
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
                    if m.session.model.vision.is_some() { ", seeing pictures" } else { ", its file without the part that sees pictures" },
                    m.context,
                    m.speed.map(|s| format!("; lately {s:.1} tokens a second")).unwrap_or_default()
                )
            }
            Brain::Script(_) => String::from("Model: a test's script, in place of a model"),
            Brain::Meta(m) => format!(
                "Model: you are Meta's {} ({}), answering over Meta's Model API (api.meta.ai), which Chat Computer's own \
                 system reaches over its own network stack and TLS: what's said goes to Meta{}",
                m.name,
                m.model,
                m.took_ms.map(|t| format!("; its last answer took {:.1} s to come", t as f64 / 1000.0)).unwrap_or_default()
            ),
        };
        let working = self.job.is_some()
            || matches!(&self.brain, Brain::Reading(_))
            || matches!(&self.brain, Brain::Ready(m) if !m.ahead.is_empty());
        let remote = matches!(self.brain, Brain::Meta(_));
        *SHOWN.lock() = Some(Shown { ready, status, busy: self.job.is_some(), working, remote });
    }

    /// The system prompt for a conversation: the instructions, and the model's notes about the person.
    fn system_prompt(&self) -> String {
        let name = match &self.brain {
            Brain::Ready(m) => m.name.clone(),
            Brain::Reading(r) => r.name.clone(),
            Brain::Meta(m) => m.name.clone(),
            _ => String::from("Qwen"),
        };
        let who = if matches!(self.brain, Brain::Meta(_)) { META_WHO } else { LOCAL_WHO };
        let sees = match &self.brain {
            Brain::Ready(m) => m.session.model.vision.is_some(),
            Brain::Script(_) => true,
            _ => false,
        };
        let mut system = SYSTEM.replace("{who}", who).replace("{model}", &name).replace(
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
        system
    }

    /// What the model's given: the system prompt, then the conversation from where it's given.
    fn messages(&self) -> Vec<Message> {
        let mut out = vec![Message::System(self.system.clone())];
        for p in &self.parts[self.from.min(self.parts.len())..] {
            out.push(match p {
                Part::You { text, at: Some(t) } => Message::User(format!("[{}] {text}", clock::short_text(*t))),
                Part::You { text, at: None } => Message::User(text.clone()),
                Part::Said(a) => Message::Assistant(a.clone()),
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

    /// Starts reading the instructions ahead (the system prompt, with the tools), before anything's asked.
    fn start_ahead(&mut self) {
        let messages = vec![Message::System(self.system.clone()), Message::User(String::from("."))];
        let Brain::Ready(m) = &mut self.brain else { return };
        let r = request(&self.tools, &messages, 0, Sampling::GREEDY);
        m.ahead = m.session.ahead(&r).unwrap_or_default();
        m.ahead_done = 0;
    }

    /// Moves the answer on, for a step.
    fn answer(&mut self, until: u64, events: &mut Vec<Event>) {
        let Some(mut job) = self.job.take() else { return };
        if !self.answer_job(&mut job, until, events) {
            self.job = Some(job);
        }
    }

    /// True when the answer is over (its last event given).
    fn answer_job(&mut self, job: &mut Job, until: u64, events: &mut Vec<Event>) -> bool {
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
                let (answer, stats) = m.session.end(turn);
                if self.round_over(job, answer, stats.ended == Ended::Length, events) {
                    return true;
                }
            } else {
                let reading = turn.reading();
                let t = time::uptime_ms();
                m.session.see = m.see;
                let step = m.session.step(turn, m.read, &crate::cores::Cores);
                let took = time::uptime_ms() - t;
                // (A call that comes as the turn ends was never closed: the engine reads it as far as it got. If the
                // round reached the most it says, it was cut off as it was written, and isn't run.)
                let over = turn.finished();
                match step {
                    Step::Read { done, total } => {
                        job.read += m.read.min(total);
                        job.read_ms += took;
                        m.read = suited(m.read, took);
                        if !job.said && job.read_ms > 1500 {
                            activity(job, format!("Reading ({}%)", percent(done, total)), events);
                        }
                    }
                    Step::Seeing { done, total } => {
                        job.see_ms += took;
                        m.see = suited_to(m.see, took, 1 << 16);
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
            if time::uptime_ms() >= until {
                return false;
            }
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
                job.turn = Some(turn);
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
        self.parts.push(Part::Said(answer));
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
            let (said, error, picture) = run_tool(&call.name, &input, job, &mut self.notes, events, sight, blind);
            self.unkept |= self.notes != notes;
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
        match &self.brain {
            Brain::Ready(m) => {
                let model: &'static model::Model<'static> = m.session.model;
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
            Brain::Ready(m) if m.session.model.vision.is_none() && m.session.model.settings.converter < model::convert::CONVERTER => {
                "its file was made before Chat Computer's engine kept the part of the model that sees. Chat Computer's \
                 app on Windows makes it again (downloading the model again, in the background) when it's next opened \
                 there."
            }
            Brain::Meta(_) => "Chat Computer doesn't send pictures to Meta's model yet.",
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
        crate::log!(
            "Chat Computer kernel: model: answered ({chars} characters; {} tokens read in {:.1} s, {} written in {:.1} s{seen}{})",
            job.read,
            job.read_ms as f64 / 1000.0,
            job.written,
            job.write_ms as f64 / 1000.0,
            crate::cores::shared()
        );
        if let (Brain::Ready(m), Some(s)) = (&mut self.brain, speed) {
            m.speed = Some(s);
        }
        if let Brain::Meta(m) = &mut self.brain
            && job.write_ms > 0
        {
            m.took_ms = Some(job.write_ms);
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

    /// A step of Meta's model's answer: a round asked of Meta's Model API, its answer come, a few of its words said, or
    /// the round over. True when the answer's over.
    fn meta_step(&mut self, job: &mut Job, events: &mut Vec<Event>) -> bool {
        if !self.web_step(job, events) {
            return false;
        }
        if job.script.is_none() {
            if job.meta.is_none() {
                let messages = self.messages();
                if test_build().is_some() {
                    crate::log!("Chat Computer kernel: test: asked {}", json::compact(&messages_json(&messages)));
                }
                let Brain::Meta(m) = &self.brain else { return true };
                let body = meta_body(&m.model, &messages, &self.tools);
                let headers = vec![
                    (String::from("Authorization"), format!("Bearer {}", m.key)),
                    (String::from("Content-Type"), String::from("application/json")),
                    (String::from("Accept"), String::from("application/json")),
                ];
                let url = web::Url::parse(META_API).expect("Meta's Model API's own address");
                crate::log!("Chat Computer kernel: model: asks Meta's {} ({} bytes)", m.name, body.len());
                job.meta_tries += 1;
                if test_build().is_some() {
                    crate::log!("Chat Computer kernel: test: body {body}");
                }
                job.meta = Some((crate::web::Fetch::post(url, headers, body.into_bytes()), time::uptime_ms()));
                let doing = format!("Asking {} (Meta)", m.name);
                activity(job, doing, events);
            }
            let Some((fetch, since)) = job.meta.as_mut() else { return true };
            let since = *since;
            let Some(got) = fetch.step() else { return false };
            job.meta = None;
            let took = time::uptime_ms() - since;
            match meta_answer(got) {
                Ok((answer, cut, read, written)) => {
                    job.read += read;
                    job.written += written;
                    job.write_ms += took;
                    crate::log!(
                        "Chat Computer kernel: model: Meta's answer came in {:.1} s ({read} tokens read, {written} written{})",
                        took as f64 / 1000.0,
                        if cut { ", cut at the most it says" } else { "" }
                    );
                    // Muse Spark sometimes answers "Turning it off now." without calling turn_off_computer (6% of
                    // 50 power requests in evals/meta, whatever the tool's description says), and the PC stays on:
                    // such an answer isn't taken, and it's asked again, once.
                    if answer.calls.is_empty() && !job.reasked && !job.restart && !job.turn_off && claims_power(&answer.text) {
                        job.reasked = true;
                        crate::log!("Chat Computer kernel: model: Meta's answer says the PC turns off or restarts, without the tool: asks again");
                        return false;
                    }
                    job.closed = answer.calls.len();
                    job.meta_tries = 0;
                    // Words written with tool calls that will run ("I'll look for it...") aren't the answer: they'd
                    // sit on the screen in the middle of it, so they're shown as what it's doing instead, and the
                    // answer is what it says once it's done (evals/meta found Muse Spark writing them in two
                    // answers of three, whatever its instructions say). They stay in the conversation it's given.
                    let shown = if !answer.calls.is_empty() && job.rounds < TOOL_ROUNDS {
                        let line = answer.text.split('\n').map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
                        if !line.is_empty() {
                            let mut doing: String = line.chars().take(MOST_DOING).collect();
                            if line.chars().count() > MOST_DOING {
                                doing.push('\u{2026}');
                            }
                            activity(job, doing, events);
                        }
                        String::new()
                    } else {
                        answer.text.clone()
                    };
                    job.script = Some((shown, answer, cut));
                }
                Err((_, true)) if job.meta_tries < META_TRIES => {
                    crate::log!("Chat Computer kernel: model: asks again");
                    return false;
                }
                Err((why, _)) => {
                    self.parts.truncate(job.base);
                    fail(events, why);
                    return true;
                }
            }
        }
        let left = &mut job.script.as_mut().expect("Meta's round").0;
        if !left.is_empty() {
            let cut = left.char_indices().nth(META_PIECE).map_or(left.len(), |(i, _)| i);
            let piece: String = left.drain(..cut).collect();
            say(job, piece, events);
            return false;
        }
        let (_, answer, cut) = job.script.take().expect("Meta's round");
        self.round_over(job, answer, cut, events)
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

/// Whether an answer says the PC is turning off or restarting now (in English or Chinese), and not that it won't.
fn claims_power(text: &str) -> bool {
    let t = text.to_lowercase();
    let says = ["shutting down", "shutting it down", "turning off", "turning it off", "turning the computer off", "turning this off",
        "turning the pc off", "restarting now", "restarting the", "restarting it", "restarting this", "restarting to", "restarting into",
        "restarting back", "restarting in a", "正在关机", "马上关机", "正在重启"]
    .iter()
    .any(|w| t.contains(w));
    let not = ["can't", "cannot", "won't", "not ", "don't", "no ", "\u{4e0d}"].iter().any(|w| t.contains(w));
    says && !not
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

/// How many tokens a reading step takes next: more while steps are quick, fewer when they're slow.
fn suited(read: usize, took_ms: u64) -> usize {
    suited_to(read, took_ms, model::net::BATCH)
}

/// How much a step does next (at most `most`): more while steps are quick, less when they're slow.
fn suited_to(n: usize, took_ms: u64, most: usize) -> usize {
    if took_ms < STEP_MS / 2 {
        (n * 2).min(most)
    } else if took_ms > STEP_MS * 2 {
        (n / 2).max(1)
    } else {
        n
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
        style_name,
        uncensored,
        context,
        ahead: Vec::new(),
        ahead_done: 0,
        read: 8,
        see: 64,
        speed: None,
    })
}

/// The request for a round of Meta's model's answer, as Meta's Model API reads it (OpenAI's chat completions): the
/// model, the messages (each tool's result after the call it answers, their ids made up here), and the tools.
fn meta_body(model: &str, messages: &[Message], tools: &[Json]) -> String {
    let s = |v: &str| Json::Str(String::from(v));
    let field = |k: &str, v: Json| (String::from(k), v);
    let mut out: Vec<Json> = Vec::new();
    let mut made = 0;
    let mut waiting: Vec<String> = Vec::new();
    let text_of = |parts: &[Content]| -> String {
        parts
            .iter()
            .map(|p| match p {
                Content::Text(t) => t.clone(),
                Content::Picture(_) => String::from("[a picture, which Chat Computer doesn't send to Meta's model]"),
            })
            .collect()
    };
    for m in messages {
        out.push(match m {
            Message::System(t) => Json::Object(vec![field("role", s("system")), field("content", s(t))]),
            Message::User(t) => Json::Object(vec![field("role", s("user")), field("content", s(t))]),
            Message::UserParts(parts) => Json::Object(vec![field("role", s("user")), field("content", s(&text_of(parts)))]),
            Message::Assistant(a) => {
                let mut fields = vec![field("role", s("assistant")), field("content", s(&a.text))];
                if !a.calls.is_empty() {
                    let calls = a
                        .calls
                        .iter()
                        .map(|c| {
                            made += 1;
                            let id = format!("call_{made}");
                            waiting.push(id.clone());
                            let tool = tools.iter().find(|t| t.get("function").and_then(|f| f.get("name")).and_then(Json::str) == Some(&c.name));
                            Json::Object(vec![
                                field("id", Json::Str(id)),
                                field("type", s("function")),
                                field(
                                    "function",
                                    Json::Object(vec![field("name", s(&c.name)), field("arguments", Json::Str(json::compact(&c.arguments(tool))))]),
                                ),
                            ])
                        })
                        .collect();
                    fields.push(field("tool_calls", Json::Array(calls)));
                }
                Json::Object(fields)
            }
            Message::Tool(_) | Message::ToolParts(_) => {
                let text = match m {
                    Message::Tool(t) => t.clone(),
                    Message::ToolParts(parts) => text_of(parts),
                    _ => String::new(),
                };
                let id = if waiting.is_empty() { format!("call_{made}") } else { waiting.remove(0) };
                Json::Object(vec![field("role", s("tool")), field("tool_call_id", Json::Str(id)), field("content", Json::Str(text))])
            }
        });
    }
    let mut body = vec![field("model", s(model)), field("messages", Json::Array(out))];
    if !tools.is_empty() {
        body.push(field("tools", Json::Array(tools.to_vec())));
    }
    body.push(field("max_completion_tokens", Json::Number(META_TOKENS as f64)));
    json::compact(&Json::Object(body))
}

/// Meta's model's round, from Meta's Model API's answer: its words and tool calls, whether it reached the most it
/// says, and the tokens it read and wrote; or why there's none, for the person, and whether asking again may help.
fn meta_answer(got: Result<crate::web::Page, String>) -> Result<(Answer, bool, usize, usize), (String, bool)> {
    let page = got.map_err(|why| (format!("Meta's model couldn't be reached: {why}."), false))?;
    let text = String::from_utf8_lossy(&page.bytes);
    let answer = json::parse(&text);
    if page.status != 200 {
        let said = answer.as_ref().and_then(|a| a.get("error")).and_then(|e| e.get("message").or(Some(e))).and_then(Json::str);
        let said = said.map(|m| format!(": \u{201C}{}\u{201D}", m.chars().take(300).collect::<String>())).unwrap_or_default();
        crate::log!("Chat Computer kernel: model: Meta's Model API answered {}{said}", page.status);
        let code = answer.as_ref().and_then(|a| a.get("error")).and_then(|e| e.get("code")).and_then(Json::str);
        let again = page.status == 429 || page.status >= 500 || code == Some("model_not_found");
        return Err((match page.status {
            401 | 403 => format!(
                "Meta's Model API didn't accept the key in Chat Computer's settings (meta-api-key){said}. A key comes from \
                 https://dev.meta.ai."
            ),
            429 => String::from("Meta's Model API says it's had too many requests: try again in a minute."),
            s if s >= 500 => format!("Meta's Model API had a problem ({s}): try again in a minute."),
            s => format!("Meta's Model API refused the question ({s}){said}."),
        }, again));
    }
    let Some(answer) = answer else { return Err((String::from("Meta's Model API's answer couldn't be read."), true)) };
    let Some(choice) = answer.get("choices").and_then(Json::array).and_then(|c| c.first()) else {
        return Err((String::from("Meta's Model API's answer had nothing in it."), true));
    };
    let message = choice.get("message");
    let words = message.and_then(|m| m.get("content")).and_then(Json::str).unwrap_or("");
    let calls = message
        .and_then(|m| m.get("tool_calls"))
        .and_then(Json::array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|c| {
            let f = c.get("function")?;
            let name = String::from(f.get("name")?.str()?);
            let args = f.get("arguments").and_then(Json::str).and_then(json::parse);
            let args = args
                .as_ref()
                .and_then(Json::object)
                .unwrap_or(&[])
                .iter()
                .map(|(k, v)| (k.clone(), v.str().map_or_else(|| json::compact(v), String::from)))
                .collect();
            Some(Call { name, args })
        })
        .collect();
    let cut = choice.get("finish_reason").and_then(Json::str) == Some("length");
    let usage = answer.get("usage");
    let tokens = |k: &str| usage.and_then(|u| u.get(k)).and_then(Json::usize).unwrap_or(0);
    let answer = Answer { thinking: String::new(), text: String::from(words.trim_start()), calls };
    Ok((answer, cut, tokens("prompt_tokens"), tokens("completion_tokens")))
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

/// What the memory file keeps: the model's notes and the conversation, as {"notes": [...], "chat": [{"you": words,
/// "at": seconds since 1970}, {"said": words}, ...]}: each question, and the words of its answer (not the tools it
/// used along the way).
fn kept_text(notes: &[String], parts: &[Part]) -> String {
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
            Part::Said(answer) if !answer.text.trim().is_empty() => {
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
    json::compact(&Json::Object(vec![(String::from("notes"), notes), (String::from("chat"), Json::Array(chat))]))
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
            Some(Part::Said(a)) => {
                a.text.push_str("\n\n");
                a.text.push_str(words);
            }
            _ => parts.push(Part::Said(Answer { text: String::from(words), ..Answer::default() })),
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
fn run_tool(
    name: &str,
    input: &Json,
    job: &mut Job,
    notes: &mut Vec<String>,
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
    let (said, error) = run_other_tool(name, input, job, notes, events);
    (said, error, None)
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
