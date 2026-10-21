//! Talking to Claude: Opus 5.5 at its highest effort, through Anthropic's Messages API at api.anthropic.com, over
//! Chat Computer's own network stack (net), its own TLS 1.3 (tls) and its own HTTP and JSON (claude), with the
//! person's own API key from the settings. One question at a time: find the address, connect, check who answers
//! (its certificate, against Mozilla's authorities), send the conversation, and pass the answer's words on as they
//! come. Nothing of the conversation goes in the log; a test build logs the answer, which a test checks.
//!
//! Claude can act on this computer through tools (`TOOLS`): see how it's doing, change the text size, restart it.
//! When an answer calls them, they run here, and their results go back in a new request with the answer as it
//! came (its thinking included, unchanged, as the API requires), until Claude has finished. Claude can also search
//! the web and read web pages: Anthropic runs those tools itself (`api::WEB_SEARCH`, `api::WEB_FETCH`), so this
//! computer needs no browser for them; their results come back in the answer, and go back in the conversation as
//! they came. The conversation (every turn, as the API had it) is kept here.
//!
//! A test build (CHATCOMPUTER_TEST=claude, by check.py --claude) talks to a stand-in on the computer running the
//! virtual PC instead (10.0.2.2, as "localhost"), and trusts only that test's own certificate authority.

use crate::sync::SpinLock;
use crate::{clock, network, random, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use claude::api::{self, Content, Role, Stream, ToolUse, Turn};
use claude::json::{self, Json};
use net::tcp::Socket;
use net::{Ipv4, Stack, TcpState};
use tls::Cert;

/// What came of the question.
pub enum Event {
    /// More of the answer.
    Text(String),
    /// The answer is complete.
    Done,
    /// It failed: why, for the person.
    Failed(String),
    /// Claude set the text size (percent of the size chosen for this screen).
    TextSize(u32),
    /// Claude is restarting the PC, in this many seconds.
    Restarting(u64),
    /// Claude is turning the PC off, in this many seconds.
    TurningOff(u64),
    /// Claude noted something about the person (its words), or forgot this many notes.
    Noted(String),
    Forgot(usize),
    /// What Claude is doing before it answers ("Searching the web").
    Activity(&'static str),
    /// A picture Claude shows the person (show_picture), made to fit the conversation's column.
    Picture(picture::Picture),
    /// The conversation so far is over: the next question starts a new one.
    NewConversation,
}

/// Whether Claude can be asked now, and if not, why not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    Yes,
    NoKey,
    BadKey(&'static str),
    NoClock,
    NoCard,
    NoCable,
    NoAddress,
}

/// Where the key comes from.
pub const GIVE_KEY: &str =
    "On Windows, double-click \u{201C}Chat Computer Own System\u{201D} and give it your Claude API key when it asks.";
pub const NO_KEY: &str = "To talk to Claude here, Chat Computer needs your Claude API key.";
pub const NO_CARD: &str = "Claude is on the internet, and this PC's network chip isn't one Chat Computer has a driver \
                           for yet. A USB network adapter works instead, or an Android phone sharing its connection \
                           over USB (USB tethering).";
const NO_CLOCK: &str = "This PC's clock couldn't be read, so Chat Computer can't check it's really talking to Claude.";

/// Waiting for a network address; finding Claude's, and connecting; the secure connection being set up.
const NETWORK_MS: u64 = 60_000;
const CONNECT_MS: u64 = 30_000;
const HANDSHAKE_MS: u64 = 30_000;
/// Silence from Claude's API, once asked, before giving up. At its highest effort Claude may think for minutes,
/// but the stream carries its thinking, or a ping, all along.
const SILENCE_MS: u64 = 180_000;
/// How long api.anthropic.com's address is used before it's looked up again.
const ADDRESS_MS: u64 = 300_000;
/// A closing connection still not closed after this is broken off.
const CLOSING_MS: u64 = 30_000;

const SYSTEM: &str = "You are Claude, talking with a person on Chat Computer: a computer whose screen is a \
conversation with you. It is running Chat Computer's own operating system, which is new: here you can talk, search the \
web and read web pages, and look after this computer with the tools you have (its status, the text size, timers and \
reminders, how loud their chimes are, restarting it or turning it off, starting a new conversation), and read (not \
change) the files on its drives (Windows' C: and the others, USB sticks and memory cards included). The conversation is \
kept across restarts, and you can note things about the person to remember in later ones. This screen shows Markdown formatted: headings, bold and italic, lists, code, quotes and simple \
tables. Nothing can be opened from it (no links), and the only pictures it shows are the ones you show with \
show_picture, so write an address out in full when the person needs it.";
const SYSTEM_NO_WEB: &str = "You are Claude, talking with a person on Chat Computer: a computer whose screen is a \
conversation with you. It is running Chat Computer's own operating system, which is new: here you can talk, and look \
after this computer with the tools you have (its status, the text size, timers and reminders, how loud their chimes \
are, restarting it or turning it off, starting a new conversation, reading the files on its drives), but you \
can't use the web here (it isn't enabled for this API key). The \
conversation is kept across restarts, and you can note things about the person to remember in later ones. This screen shows Markdown formatted: headings, bold and italic, lists, code, \
quotes and simple tables. Nothing can be opened from it (no links), and the only pictures it shows are the ones you \
show with show_picture, so write an address out in full when the person needs it.";
/// Said when the oldest part of a long conversation is no longer sent (see `SENT_BYTES`).
const SYSTEM_SHORTENED: &str = " This conversation began earlier: its oldest part isn't sent to you any more, to keep \
it short, but the person can still scroll back to it.";
/// The tools Anthropic runs: searching the web, reading web pages.
const WEB_TOOLS: [&str; 2] = [api::WEB_SEARCH, api::WEB_FETCH];

/// The most of the conversation sent with each question, in bytes of the API's JSON (very roughly 150,000 tokens):
/// beyond it the oldest questions and their answers are left out (they stay on the screen, and in the memory file). A
/// third goes at a time, so the part that's sent keeps the same start for a while, and Claude's API can go on reading
/// it from its cache (api::request).
const SENT_BYTES: usize = 600_000;

/// The most of pictures and PDFs (as base64) sent with a question, and how many: newest first, older ones taken out
/// (a line says each was there). Under the API's 32 MB a request, with the rest of the conversation; and at most 20,
/// as more than that would make the API take only smaller pictures.
const ATTACHED_BYTES: usize = 28_000_000;
const MOST_ATTACHED: usize = 20;

/// The most notes Claude keeps, the longest one, and the most of them all together, in characters.
const MAX_NOTES: usize = 40;
const NOTE_CHARS: usize = 400;
const NOTES_CHARS: usize = 8000;

/// What Claude can do on this computer.
const TOOLS: [api::Tool; 13] = [
    api::Tool {
        name: "computer_status",
        description: "What this computer is and how it's doing: its processor, memory, screen, how long Chat Computer's \
                      own system has been running, the date and time by its clock, its network connection and its \
                      keyboards.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    api::Tool {
        name: "set_text_size",
        description: "Makes the text on the screen bigger or smaller. 100 is the size Chat Computer chose for this \
                      screen and how far away it's likely seen from; 150 is half as big again.",
        input_schema: r#"{"type":"object","properties":{"percent":{"type":"integer","minimum":50,"maximum":300,"description":"The new size, in percent of the size chosen for this screen."}},"required":["percent"]}"#,
    },
    api::Tool {
        name: "restart_computer",
        description: "Restarts this PC a few seconds after you finish answering. Chat Computer's own system was started \
                      just once, so the PC then starts what it normally does (Windows, usually); this conversation is \
                      kept for the next time Chat Computer starts. Use it only when the person asks to restart, or to \
                      go back to Windows.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    api::Tool {
        name: "turn_off_computer",
        description: "Turns this PC off a few seconds after you finish answering. When it's turned on again, it starts \
                      what it normally does (Windows, usually); this conversation is kept for the next time Chat \
                      Computer starts. Use it only when the person asks to turn the computer off or shut it down.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
    api::Tool {
        name: "remember",
        description: "Notes something about the person to know in every later conversation, after restarts and new \
                      conversations too: their name, what they like, what they're working on. Use it when they ask \
                      you to remember something, or tell you something lasting about themselves that they'd want you \
                      to know later. Your notes are shown to you at the start of every conversation. One short \
                      sentence a note.",
        input_schema: r#"{"type":"object","properties":{"note":{"type":"string","description":"The note: one short sentence."}},"required":["note"]}"#,
    },
    api::Tool {
        name: "forget",
        description: "Forgets the notes you made with remember that contain these words. Use it when the person asks \
                      you to forget something.",
        input_schema: r#"{"type":"object","properties":{"words":{"type":"string","description":"Words from the note or notes to forget."}},"required":["words"]}"#,
    },
    api::Tool {
        name: "reminders",
        description: "Timers, reminders and alarms. They ring here: a line in the conversation, and a chime on this \
                      computer's speakers or headphones (where its sound works with Chat Computer's own system), \
                      again and again (a timer for a minute, an alarm for ten) until a key is pressed; a reminder \
                      rings once. timer: minutes and/or seconds, with an optional label; reminder: text, and \
                      in_minutes or at (a time of day like 18:00 or 6pm: the next time it comes, when this computer's \
                      local time is known); alarm: at, and an optional label; list; cancel: by id, or 'all' (or \
                      'timers', 'reminders', 'alarms'); stop: silences one that's ringing. They last until the computer \
                      restarts.",
        input_schema: r#"{"type":"object","properties":{"action":{"type":"string","enum":["timer","reminder","alarm","list","cancel","stop"]},"minutes":{"type":"number"},"seconds":{"type":"number"},"text":{"type":"string"},"label":{"type":"string"},"in_minutes":{"type":"number"},"at":{"type":"string"},"id":{"type":"string"}},"required":["action"]}"#,
    },
    api::Tool {
        name: "set_volume",
        description: "Sets how loud the chimes of timers and reminders are on this computer's speakers or headphones, \
                      0 (silent) to 100 (as loud as Chat Computer makes them).",
        input_schema: r#"{"type":"object","properties":{"percent":{"type":"integer","minimum":0,"maximum":100}},"required":["percent"]}"#,
    },
    api::Tool {
        name: "list_files",
        description: "Lists a folder on one of this computer's drives (C:, where Windows is, and its others, D: and on, \
                      USB sticks and memory cards included, as computer_status lists them): its folders, and its \
                      files with their size and when each was \
                      last changed. The person's own folders are in C:\\Users\\<their name> \
                      (Desktop, Documents, Downloads, Pictures...); where OneDrive backs them up, their Desktop, \
                      Documents and Pictures are in C:\\Users\\<their name>\\OneDrive instead, and a file that's \
                      online only (not downloaded to this PC) can't be read here. Links are followed. Read only: \
                      nothing here can be changed.",
        input_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"The folder, like C:\\Users or C:\\Users\\Sam\\Documents."}},"required":["path"]}"#,
    },
    api::Tool {
        name: "read_file",
        description: "Reads a file on one of this computer's drives (C:, D:...). A text file (.txt, .md, .csv, code...) as it \
                      is; a Word, Excel or PowerPoint file (or OpenDocument's) as its words: a document's paragraphs, \
                      headings, lists and tables, a workbook's sheets as CSV with row numbers and column letters \
                      (formulas after their values), a presentation's slides with their notes. Up to 60,000 bytes at \
                      a time (offset reads further on). A picture (JPEG, PNG, GIF or WebP) or a PDF (up to 20 MB) \
                      comes to you, for you to see (a picture bigger than you'd see it made smaller). A ZIP archive's \
                      files are listed. Office's files \
                      from before 2007 (.doc, .xls, .ppt) and HEIC pictures can't be read yet.",
        input_schema: r#"{"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0,"description":"Where to start, in bytes (0: the start)."}},"required":["path"]}"#,
    },
    api::Tool {
        name: "show_picture",
        description: "Shows the person a picture from one of this computer's drives (C:, D:...) on the screen, in the \
                      conversation where your answer is: a photo, a screenshot, a drawing (JPEG, PNG or GIF). Use it \
                      when they ask to see a picture, or when seeing one helps them. It shows the picture to the \
                      person, not to you: read_file shows it to you.",
        input_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"The picture, like C:\\Users\\Sam\\Pictures\\Beach.jpg."}},"required":["path"]}"#,
    },
    api::Tool {
        name: "find_files",
        description: "Finds the files and folders on one of this computer's drives whose names have all of these words \
                      in them, from a folder down: by default C:\\Users, where the person's files are (hidden folders \
                      like AppData are left out). It looks in a few thousand folders at most.",
        input_schema: r#"{"type":"object","properties":{"words":{"type":"string"},"in":{"type":"string","description":"The folder to search from (default C:\\Users)."}},"required":["words"]}"#,
    },
    api::Tool {
        name: "start_new_conversation",
        description: "Ends this conversation once you finish answering: the next message starts a new one, and this one \
                      is no longer sent to you or kept across restarts (it stays on the screen, above a line, until \
                      the computer restarts). Use it only when the person asks to start over, to start a new \
                      conversation or chat, or to forget this one.",
        input_schema: r#"{"type":"object","properties":{}}"#,
    },
];
/// Rounds of tool calls (and of continuing a paused answer) in one answer, at most.
const TOOL_ROUNDS: u32 = 8;

fn test_build() -> bool {
    option_env!("CHATCOMPUTER_TEST") == Some("claude")
}

/// The stand-in's address and port, in a test build.
const TEST_SERVER: Ipv4 = Ipv4([10, 0, 2, 2]);
pub const TEST_PORT: u16 = 8472;
/// The test's certificate authority (empty in every other build: kernel/build.rs).
static TEST_CA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_ca.der"));

fn host() -> &'static str {
    if test_build() { "localhost" } else { api::HOST }
}

fn port() -> u16 {
    if test_build() { TEST_PORT } else { 443 }
}

fn roots() -> &'static [Cert<'static>] {
    let roots: Vec<Cert<'static>> = if test_build() {
        tls::x509::parse_root(TEST_CA).into_iter().collect()
    } else {
        tls::roots()
    };
    Box::leak(roots.into_boxed_slice())
}

struct Talk {
    socket: Socket,
    tls: Box<tls::Client>,
    stream: Stream,
    /// When the server last sent anything.
    heard: u64,
    /// For the server, not yet taken by TCP.
    out: Vec<u8>,
    /// Whether the request has been handed to TLS.
    sent: bool,
    chars: usize,
    /// The whole answer, once it has come.
    answer: Option<Answer>,
    /// The API refused the web tools (not enabled for this key): ask again without them.
    again_without_web: bool,
}

/// An answer that came whole: its content blocks, why it stopped, and the tools it called.
struct Answer {
    content: Vec<Json>,
    stop: Option<String>,
    calls: Vec<ToolUse>,
}

enum Phase {
    /// Waiting for a network address, since.
    Network(u64),
    Looking(net::Lookup, u64),
    Connecting(Socket, u64),
    Talking(Box<Talk>),
}

struct Job {
    /// The request, key and all, until it's handed to TLS (then wiped).
    request: Vec<u8>,
    phase: Phase,
    /// How long the conversation was before this question (a failed question is taken back out).
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
    /// The last answer was paused (during a web search, say): the next one carries on with it.
    continuing: bool,
    /// The answer's words, kept only by a test build, for its log.
    text: String,
}

struct Assistant {
    key: Option<String>,
    key_problem: Option<&'static str>,
    roots: &'static [Cert<'static>],
    job: Option<Job>,
    /// Connections being closed, and since when.
    closing: Vec<(Socket, u64)>,
    /// api.anthropic.com's address, and when it was found.
    address: Option<(Ipv4, u64)>,
    /// The conversation, as the API has it.
    history: Vec<Turn>,
    /// Where the part of it that's sent starts (a question; see `SENT_BYTES`).
    sent_from: usize,
    /// What Claude noted about the person (its remember tool), shown to it in every conversation.
    notes: Vec<String>,
    /// Whether to offer the web tools (until the API says they aren't enabled for this key).
    web: bool,
    /// The conversation changed since it was last kept (keep.rs).
    unkept: bool,
}

static ASSISTANT: SpinLock<Option<Assistant>> = SpinLock::new(None);

/// Starts, with the key from the settings (or why there's none that works).
pub fn init(key: Option<String>, key_problem: Option<&'static str>) {
    let roots = roots();
    crate::log!(
        "Chat Computer kernel: Claude: {} at {} with {}, {} certificate authorities{}",
        api::MODEL,
        host(),
        if key.is_some() { "the API key from the settings" } else { "no API key yet" },
        roots.len(),
        if test_build() { " (the test's own)" } else { " (Mozilla's)" }
    );
    // Claude's notes and the conversation from before the last restart, if they were kept.
    let (notes, history) = crate::keep::load()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| from_kept(&text))
        .unwrap_or_default();
    let history = if history.first().is_none_or(is_question) { history } else { Vec::new() };
    if !history.is_empty() {
        crate::log!("Chat Computer kernel: Claude: the conversation from before the restart ({} turns)", history.len());
    }
    if !notes.is_empty() {
        let n = notes.len();
        crate::log!("Chat Computer kernel: Claude: {n} note{} from before", if n == 1 { "" } else { "s" });
    }
    *ASSISTANT.lock() = Some(Assistant {
        key,
        key_problem,
        roots,
        job: None,
        closing: Vec::new(),
        address: None,
        history,
        sent_from: 0,
        notes,
        web: true,
        unkept: false,
    });
}

/// What the memory file keeps: Claude's notes and the conversation, as {"notes": [...], "messages": [...]} (the
/// messages as the API has them, without pictures and PDFs: `without_attachments`).
fn kept_text(notes: &[String], history: &[Turn]) -> String {
    let notes = Json::Arr(notes.iter().map(|n| Json::Str(n.clone())).collect()).write();
    format!("{{\"notes\":{notes},\"messages\":{}}}", api::messages(history))
}

/// The conversation without its pictures and PDFs (a line says where each was): what the memory file keeps, as
/// they're on the PC already, and too big for it.
fn without_attachments(history: &[Turn]) -> Vec<Turn> {
    history
        .iter()
        .map(|t| {
            let mut t = t.clone();
            api::unattach(&mut t, 0);
            t
        })
        .collect()
}

/// The notes and conversation in the memory file's text (an older system kept the messages alone).
fn from_kept(text: &str) -> Option<(Vec<String>, Vec<Turn>)> {
    match json::parse(text)? {
        Json::Arr(_) => Some((Vec::new(), api::parse_messages(text)?)),
        kept @ Json::Obj(_) => {
            let notes = match kept.get("notes") {
                Some(Json::Arr(items)) => items.iter().filter_map(|n| n.str().map(String::from)).collect(),
                _ => Vec::new(),
            };
            Some((notes, api::parse_messages(&kept.get("messages")?.write())?))
        }
        _ => None,
    }
}

/// Whether a turn is a question: what the person typed (not tools' results).
fn is_question(t: &Turn) -> bool {
    matches!(t, Turn { role: Role::User, content: Content::Text(_) })
}

pub fn ready() -> Ready {
    let (has_key, problem) = match ASSISTANT.lock().as_ref() {
        Some(a) => (a.key.is_some(), a.key_problem),
        None => (false, None),
    };
    let status = network::status();
    match (has_key, problem) {
        (false, Some(p)) => Ready::BadKey(p),
        (false, None) => Ready::NoKey,
        _ if clock::now().is_none() => Ready::NoClock,
        _ if !status.card => Ready::NoCard,
        _ if status.address.is_none() && !status.link => Ready::NoCable,
        _ if status.address.is_none() => Ready::NoAddress,
        _ => Ready::Yes,
    }
}

/// The conversation so far, as the screen shows it: what was typed, and Claude's words for each question (its
/// answers' words together, a blank line between rounds).
pub fn conversation() -> Vec<(bool, String)> {
    let guard = ASSISTANT.lock();
    let Some(a) = guard.as_ref() else { return Vec::new() };
    let mut shown: Vec<(bool, String)> = Vec::new();
    for turn in &a.history {
        match (turn.role, &turn.content) {
            (Role::User, Content::Text(t)) => shown.push((true, t.clone())),
            (Role::User, Content::Blocks(_)) => {} // tools' results
            (Role::Assistant, Content::Blocks(blocks)) => {
                for b in blocks {
                    if b.get("type").and_then(|t| t.str()) != Some("text") {
                        continue;
                    }
                    let words = b.get("text").and_then(|t| t.str()).unwrap_or("");
                    match shown.last_mut() {
                        Some((false, said)) if !words.is_empty() => {
                            said.push_str("\n\n");
                            said.push_str(words);
                        }
                        _ if !words.is_empty() => shown.push((false, String::from(words))),
                        _ => {}
                    }
                }
            }
            (Role::Assistant, Content::Text(t)) => shown.push((false, t.clone())),
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
        let history = without_attachments(&a.history);
        let mut start = 0;
        loop {
            let text = kept_text(&a.notes, &history[start..]);
            if text.len() <= capacity || start >= history.len() {
                break text;
            }
            start += 1;
            while start < history.len() && !is_question(&history[start]) {
                start += 1;
            }
        }
    };
    match crate::keep::save(text.as_bytes()) {
        Ok(()) => crate::log!("Chat Computer kernel: memory: the conversation kept ({} bytes)", text.len()),
        Err(e) => crate::log!("Chat Computer kernel: memory: the conversation couldn't be kept: {e}"),
    }
}

/// Whether an answer is on its way.
pub fn busy() -> bool {
    ASSISTANT.lock().as_ref().is_some_and(|a| a.job.is_some())
}

/// Asks Claude to answer what the person typed (after the conversation so far). The answer comes from `work`.
/// Err: why it can't be asked, for the person.
pub fn ask(text: &str) -> Result<(), String> {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return Err(format!("{NO_KEY} {GIVE_KEY}")) };
    if a.job.is_some() {
        return Err(String::from("Claude is still answering."));
    }
    match (&a.key, a.key_problem) {
        (Some(_), _) => {}
        (None, Some(p)) => return Err(format!("{}. {GIVE_KEY}", sentence(p))),
        (None, None) => return Err(format!("{NO_KEY} {GIVE_KEY}")),
    }
    if clock::now().is_none() {
        return Err(String::from(NO_CLOCK));
    }
    if !network::status().card {
        return Err(String::from(NO_CARD));
    }
    let base = a.history.len();
    a.history.push(Turn::text(Role::User, text));
    let request = a.request();
    a.job = Some(Job {
        request,
        phase: Phase::Network(time::uptime_ms()),
        base,
        rounds: 0,
        said: false,
        separate: false,
        restart: false,
        turn_off: false,
        new_conversation: false,
        continuing: false,
        text: String::new(),
    });
    Ok(())
}

/// Stops the answer on its way (its connection broken off).
pub fn stop() {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    let Some(mut job) = a.job.take() else { return };
    wipe(&mut job.request);
    a.history.truncate(job.base);
    let socket = match job.phase {
        Phase::Connecting(s, _) => Some(s),
        Phase::Talking(t) => Some(t.socket),
        _ => None,
    };
    if let Some(s) = socket {
        network::with(|stack| stack.abort(s));
    }
    crate::log!("Chat Computer kernel: Claude: stopped");
}

/// From the main loop: moves the question on. Returns what came of it.
pub fn work() -> Vec<Event> {
    let now = time::uptime_ms();
    let mut events = Vec::new();
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return events };
    if a.job.is_none() && a.closing.is_empty() {
        return events;
    }
    let mut over = None;
    let ran = network::with(|stack| {
        a.tidy(now, stack);
        if let Some(mut job) = a.job.take() {
            if a.step(now, stack, &mut job, &mut events) {
                over = Some(job);
            } else {
                a.job = Some(job);
            }
        }
    });
    if ran.is_none()
        && let Some(job) = a.job.take()
    {
        a.history.truncate(job.base);
        events.push(Event::Failed(String::from(NO_CARD)));
    }
    if let Some(mut job) = over {
        wipe(&mut job.request);
        if matches!(&job.phase, Phase::Talking(t) if t.again_without_web) {
            job.request = a.request();
            job.phase = Phase::Network(now);
            a.job = Some(job);
            return events;
        }
        let answer = match &mut job.phase {
            Phase::Talking(t) => t.answer.take(),
            _ => None,
        };
        match answer {
            Some(answer) => a.answered(now, job, answer, &mut events),
            None => a.history.truncate(job.base), // it failed (and said why)
        }
    }
    events
}

/// Runs a tool Claude called: its result for Claude, and whether that's an error (and in `more`, a picture or PDF to go
/// with it).
fn run_tool(call: &ToolUse, job: &mut Job, notes: &mut Vec<String>, events: &mut Vec<Event>, more: &mut Vec<Json>) -> (String, bool) {
    crate::log!("Chat Computer kernel: Claude: uses {}", call.name);
    match call.name.as_str() {
        "computer_status" => (crate::system::status(), false),
        "set_text_size" => {
            let percent = call.input.get("percent").and_then(|p| match p {
                Json::Num(n) => n.split('.').next().and_then(|n| n.parse::<i64>().ok()),
                _ => None,
            });
            match percent {
                Some(p) if (50..=300).contains(&p) => {
                    events.push(Event::TextSize(p as u32));
                    (format!("The text is now {p}% of the size chosen for this screen."), false)
                }
                _ => (String::from("percent must be a whole number from 50 to 300"), true),
            }
        }
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
            let note = call.input.get("note").and_then(|n| n.str()).map(str::trim).unwrap_or("");
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
                (String::from("Noted: you'll see it in every conversation from now on."), false)
            }
        }
        "forget" => {
            let words = call.input.get("words").and_then(|w| w.str()).map(str::trim).unwrap_or("").to_lowercase();
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
            let text = |key: &str| call.input.get(key).and_then(|v| v.str()).map(str::trim).unwrap_or("");
            let num = |key: &str| call.input.get(key).and_then(number);
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
        "set_volume" => match call.input.get("percent").and_then(number) {
            Some(p) if (0.0..=100.0).contains(&p) => match crate::sound::set_volume(p as u32) {
                Some(v) => (format!("The chimes are at {v}% now."), false),
                None => (format!("This computer has no sound Chat Computer's own system can use: {}", crate::sound::status()), true),
            },
            _ => (String::from("percent must be from 0 to 100"), true),
        },
        "show_picture" => {
            let path = call.input.get("path").and_then(|v| v.str()).unwrap_or("");
            let (w, h) = crate::chat::picture_box();
            match crate::files::picture(path, w, h, crate::draw::GROUND) {
                Ok((said, picture)) => {
                    events.push(Event::Picture(picture));
                    (said, false)
                }
                Err(why) => (why, true),
            }
        }
        "list_files" | "read_file" | "find_files" => {
            let text = |key: &str| call.input.get(key).and_then(|v| v.str()).unwrap_or("");
            events.push(Event::Activity(if call.name == "read_file" { "Reading a file" } else { "Looking through your files" }));
            let said = match call.name.as_str() {
                "list_files" => crate::files::list(text("path")),
                "read_file" => {
                    let offset = call.input.get("offset").and_then(number).unwrap_or(0.0).max(0.0) as u64;
                    crate::files::read(text("path"), offset).map(|r| {
                        match r.attachment {
                            Some(crate::files::Attachment::Picture(media, data)) => more.push(api::image(media, &data)),
                            Some(crate::files::Attachment::Pdf(data)) => more.push(api::pdf(&data)),
                            None => {}
                        }
                        r.text
                    })
                }
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

/// A number Claude gave (3 or 2.5), if it's one.
fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Num(n) => n.parse::<f64>().ok().filter(|x| x.is_finite()),
        _ => None,
    }
}

/// `s` with a capital first letter.
pub fn sentence(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Overwrites what held the key.
fn wipe(v: &mut Vec<u8>) {
    for b in v.iter_mut() {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    v.clear();
}

fn fail(events: &mut Vec<Event>, why: String) -> bool {
    crate::log!("Chat Computer kernel: Claude: failed: {why}");
    events.push(Event::Failed(why));
    true
}

fn tcp_text(e: Option<net::TcpError>) -> String {
    match e {
        Some(e) => format!("{e}"),
        None => String::from("the connection closed"),
    }
}

/// A failed secure connection, for the person.
fn tls_text(e: tls::Error) -> String {
    match e {
        tls::Error::Certificate(tls::Problem::Expired) if clock::behind() || clock::now().is_none() => format!(
            "Chat Computer couldn't check who answered for {}: its certificate isn't valid at the time this PC's clock \
             says ({}), and that's earlier than it can be. Set the clock in the PC's setup screen, or in Windows, and \
             start again.",
            host(),
            clock::now_text().unwrap_or_default()
        ),
        tls::Error::Certificate(p) => format!("Chat Computer didn't trust the server that answered for {}: {p}.", host()),
        e => format!("The secure connection to Claude's API failed: {e}."),
    }
}

/// A failure Claude's API reported, for the person.
fn api_text(status: Option<u16>, message: &str) -> String {
    match status {
        Some(401) => format!("Claude's API didn't accept the API key ({message}). {GIVE_KEY}"),
        Some(403) => format!("Claude's API says this key may not do that ({message})."),
        Some(429) => format!("Claude's API says there have been too many requests ({message}). Try again in a minute."),
        Some(529) | Some(503) => format!("Claude is very busy right now ({message}). Try again in a moment."),
        _ => format!("Claude's API: {message}"),
    }
}

fn suite_name(s: Option<tls::Suite>) -> &'static str {
    match s {
        Some(tls::Suite::Aes128Gcm) => "AES-128-GCM",
        Some(tls::Suite::Aes256Gcm) => "AES-256-GCM",
        Some(tls::Suite::ChaCha20Poly1305) => "ChaCha20-Poly1305",
        None => "?",
    }
}

impl Assistant {
    /// The request for the conversation so far (as much of it as is sent: `SENT_BYTES`). The system prompt changes at
    /// most once a day (the date), so Claude's API can keep the request's start in its cache; the time of day is in
    /// the computer's status.
    fn request(&mut self) -> Vec<u8> {
        let from = self.sending_from();
        let mut system = String::from(if self.web { SYSTEM } else { SYSTEM_NO_WEB });
        if let Some(today) = clock::today_text() {
            system.push_str(&format!(" Today is {today}, by the computer's clock; its status has the time."));
        }
        if from > 0 {
            system.push_str(SYSTEM_SHORTENED);
        }
        if !self.notes.is_empty() {
            system.push_str("\n\nYour notes about the person, from earlier conversations (made with remember):");
            for n in &self.notes {
                system.push_str("\n\u{2022} ");
                system.push_str(n);
            }
        }
        let web: &[&str] = if self.web { &WEB_TOOLS } else { &[] };
        api::request(host(), self.key.as_deref().unwrap_or(""), &system, &self.history[from..], &TOOLS, web)
    }

    /// Where the part of the conversation that's sent starts: where it did, unless that's now more than `SENT_BYTES`;
    /// then later questions, until it's at most two thirds of that. Never past the last question.
    fn sending_from(&mut self) -> usize {
        if !self.history.get(self.sent_from).is_some_and(is_question) {
            self.sent_from = 0;
        }
        // (Pictures and PDFs don't count: they have their own limit, `ATTACHED_BYTES`.)
        let sizes: Vec<usize> =
            self.history.iter().map(|t| api::turn_size(t).saturating_sub(api::attachments(t).iter().sum::<usize>())).collect();
        let mut total: usize = sizes[self.sent_from..].iter().sum();
        if total > SENT_BYTES {
            while total > SENT_BYTES / 3 * 2 {
                let Some(next) = (self.sent_from + 1..self.history.len()).find(|&i| is_question(&self.history[i])) else {
                    break;
                };
                total -= sizes[self.sent_from..next].iter().sum::<usize>();
                self.sent_from = next;
            }
            crate::log!("Chat Computer kernel: Claude: the conversation's oldest part is no longer sent");
        }
        self.sent_from
    }

    /// Keeps the pictures and PDFs sent within `ATTACHED_BYTES` and `MOST_ATTACHED`: the newest, in the order they
    /// came; older ones are taken out, a line saying each was there.
    fn limit_attachments(&mut self) {
        let (mut bytes, mut count) = (0usize, 0usize);
        for turn in self.history.iter_mut().rev() {
            let sizes = api::attachments(turn);
            let fit = sizes
                .iter()
                .take_while(|&&b| {
                    let ok = bytes + b <= ATTACHED_BYTES && count < MOST_ATTACHED;
                    if ok {
                        bytes += b;
                        count += 1;
                    }
                    ok
                })
                .count();
            if fit < sizes.len() {
                let taken = api::unattach(turn, fit);
                crate::log!("Chat Computer kernel: Claude: {taken} older picture(s) or PDF(s) no longer sent");
                // (Nothing older is sent, once one isn't.)
                bytes = ATTACHED_BYTES;
                count = MOST_ATTACHED;
            }
        }
    }

    /// An answer came whole: kept in the conversation; if it called tools, they run and their results go back in a new
    /// request; else it's over.
    fn answered(&mut self, now: u64, mut job: Job, answer: Answer, events: &mut Vec<Event>) {
        let has_words = answer.content.iter().any(|b| b.get("type").and_then(|t| t.str()) == Some("text"));
        let calls = answer.stop.as_deref() == Some("tool_use") && !answer.calls.is_empty();
        // Paused (the API does that during its own tools, a long web search say): sent back as it is, it goes on.
        let paused = answer.stop.as_deref() == Some("pause_turn");
        if !has_words && !calls && !paused && !job.said {
            self.history.truncate(job.base);
            fail(events, String::from("Claude didn't say anything."));
            return;
        }
        // What carries on a paused answer belongs to that answer.
        match self.history.last_mut() {
            Some(Turn { role: Role::Assistant, content: Content::Blocks(blocks) }) if job.continuing => blocks.extend(answer.content),
            _ => self.history.push(Turn { role: Role::Assistant, content: Content::Blocks(answer.content) }),
        }
        job.continuing = false;
        if paused {
            if job.rounds >= TOOL_ROUNDS {
                self.history.truncate(job.base);
                fail(events, String::from("Claude's answer kept on pausing, so Chat Computer stopped it."));
                return;
            }
            job.rounds += 1;
            job.continuing = true;
            job.separate = job.said;
            job.request = self.request();
            job.phase = Phase::Network(now);
            self.job = Some(job);
            return;
        }
        if !calls {
            if test_build() {
                crate::log!("Chat Computer kernel: test: Claude said {:?}", job.text);
            }
            if job.new_conversation {
                self.history.clear();
                self.sent_from = 0;
                crate::log!("Chat Computer kernel: Claude: a new conversation starts");
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
            return;
        }
        if job.rounds >= TOOL_ROUNDS {
            self.history.truncate(job.base);
            fail(events, String::from("Claude kept on using tools, so Chat Computer stopped it."));
            return;
        }
        let mut results = Vec::new();
        for call in &answer.calls {
            let before = self.notes.len();
            let mut more = Vec::new();
            let (text, error) = run_tool(call, &mut job, &mut self.notes, events, &mut more);
            self.unkept |= self.notes.len() != before;
            results.push(if more.is_empty() { api::tool_result(&call.id, &text, error) } else { api::tool_result_with(&call.id, &text, error, more) });
        }
        self.history.push(Turn { role: Role::User, content: Content::Blocks(results) });
        self.limit_attachments();
        job.rounds += 1;
        job.separate = job.said;
        job.request = self.request();
        job.phase = Phase::Network(now);
        self.job = Some(job);
    }

    /// Lets go of closed connections; breaks off ones that won't close.
    fn tidy(&mut self, now: u64, stack: &mut Stack) {
        self.closing.retain(|&(s, since)| match stack.state(s) {
            TcpState::Closed | TcpState::TimeWait => {
                stack.release(s);
                false
            }
            _ if now.saturating_sub(since) > CLOSING_MS => {
                stack.abort(s);
                false
            }
            _ => true,
        });
    }

    fn connect(&mut self, now: u64, stack: &mut Stack, job: &mut Job, ip: Ipv4, events: &mut Vec<Event>) -> bool {
        match stack.connect(now, ip, port()) {
            Ok(s) => {
                job.phase = Phase::Connecting(s, now);
                false
            }
            Err(e) => fail(events, format!("Couldn't connect to Claude's API: {e}.")),
        }
    }

    /// Moves `job` on as far as it goes now. True when it's over (its last event given).
    fn step(&mut self, now: u64, stack: &mut Stack, job: &mut Job, events: &mut Vec<Event>) -> bool {
        loop {
            match &mut job.phase {
                Phase::Network(since) => {
                    if stack.config().is_none() {
                        if now.saturating_sub(*since) > NETWORK_MS {
                            return fail(
                                events,
                                String::from("Chat Computer has no network address yet: is the network cable plugged in?"),
                            );
                        }
                        return false;
                    }
                    if test_build() {
                        if self.connect(now, stack, job, TEST_SERVER, events) {
                            return true;
                        }
                    } else if let Some((ip, when)) = self.address
                        && now.saturating_sub(when) < ADDRESS_MS
                    {
                        if self.connect(now, stack, job, ip, events) {
                            return true;
                        }
                    } else {
                        job.phase = Phase::Looking(stack.resolve(now, host()), now);
                    }
                }
                Phase::Looking(lookup, since) => match stack.resolved(*lookup) {
                    None if now.saturating_sub(*since) > CONNECT_MS => {
                        return fail(events, format!("Couldn't find {} on the network: no answer.", host()));
                    }
                    None => return false,
                    Some(Ok(ip)) => {
                        crate::log!("Chat Computer kernel: Claude: {} is at {ip}", host());
                        self.address = Some((ip, now));
                        if self.connect(now, stack, job, ip, events) {
                            return true;
                        }
                    }
                    Some(Err(e)) => return fail(events, format!("Couldn't find {} on the network: {e}.", host())),
                },
                Phase::Connecting(s, since) => {
                    let s = *s;
                    match stack.state(s) {
                        TcpState::Established => {
                            let Some(now_s) = clock::now() else { return fail(events, String::from(NO_CLOCK)) };
                            let mut tls = Box::new(tls::Client::new(host(), now_s, self.roots, &mut random::fill));
                            tls.allow_clock_error(clock::error());
                            let out = tls.outgoing();
                            job.phase = Phase::Talking(Box::new(Talk {
                                socket: s,
                                tls,
                                stream: Stream::new(),
                                heard: now,
                                out,
                                sent: false,
                                chars: 0,
                                answer: None,
                                again_without_web: false,
                            }));
                        }
                        TcpState::Closed => {
                            let why = tcp_text(stack.error(s));
                            stack.release(s);
                            return fail(events, format!("Couldn't connect to Claude's API: {why}."));
                        }
                        _ if now.saturating_sub(*since) > CONNECT_MS => {
                            stack.abort(s);
                            return fail(events, String::from("Couldn't connect to Claude's API: no answer."));
                        }
                        _ => return false,
                    }
                }
                Phase::Talking(t) => {
                    let mut words = Vec::new();
                    let over = self.talk(now, stack, t, &mut job.request, &mut words, events);
                    for w in words {
                        if job.separate && !w.is_empty() {
                            job.separate = false;
                            if test_build() {
                                job.text.push_str("\n\n");
                            }
                            events.push(Event::Text(String::from("\n\n")));
                        }
                        job.said |= !w.trim().is_empty();
                        if test_build() {
                            job.text.push_str(&w);
                        }
                        events.push(Event::Text(w));
                    }
                    return over;
                }
            }
        }
    }

    /// The connection is open: the secure connection's set-up, the request, and the answer (its words go in
    /// `words`; when it's whole, in `t.answer`).
    fn talk(
        &mut self,
        now: u64,
        stack: &mut Stack,
        t: &mut Talk,
        request: &mut Vec<u8>,
        words: &mut Vec<String>,
        events: &mut Vec<Event>,
    ) -> bool {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = stack.recv(t.socket, &mut buf);
            if n == 0 {
                break;
            }
            t.heard = now;
            t.tls.incoming(&buf[..n]);
        }
        if let Some(e) = t.tls.error() {
            let alert = t.tls.outgoing();
            stack.send(t.socket, &alert);
            self.close(now, stack, t.socket);
            return fail(events, tls_text(e));
        }
        if t.tls.is_open() && !t.sent {
            crate::log!("Chat Computer kernel: Claude: connected (TLS 1.3, {}), asking", suite_name(t.tls.suite()));
            t.tls.write(request);
            wipe(request);
            t.sent = true;
        }
        let mut over = None;
        'reading: loop {
            let n = t.tls.read(&mut buf);
            if n == 0 {
                break;
            }
            for e in t.stream.feed(&buf[..n]) {
                match e {
                    api::Event::Text(s) => {
                        t.chars += s.chars().count();
                        words.push(s);
                    }
                    api::Event::Tool(name) => {
                        let (doing, log) = match name.as_str() {
                            "web_search" => ("Searching the web", "searches the web"),
                            "web_fetch" => ("Reading a web page", "reads a web page"),
                            _ => ("Working", "uses one of Anthropic's tools"),
                        };
                        crate::log!("Chat Computer kernel: Claude: {log}");
                        events.push(Event::Activity(doing));
                    }
                    e => {
                        over = Some(e);
                        break 'reading;
                    }
                }
            }
        }
        if over.is_none() && (stack.finished(t.socket) || t.tls.finished()) {
            for e in t.stream.closed() {
                match e {
                    api::Event::Text(s) => words.push(s),
                    api::Event::Tool(_) => {}
                    e => over = Some(e),
                }
            }
        }
        match over {
            Some(api::Event::Done) => {
                crate::log!(
                    "Chat Computer kernel: Claude: answered ({} characters, {})",
                    t.chars,
                    t.stream.stop_reason().unwrap_or("no reason given")
                );
                t.answer = Some(Answer {
                    content: t.stream.content(),
                    stop: t.stream.stop_reason().map(String::from),
                    calls: t.stream.tool_uses(),
                });
                t.tls.close();
                t.out.extend(t.tls.outgoing());
                stack.send(t.socket, &t.out);
                self.close(now, stack, t.socket);
                return true;
            }
            Some(api::Event::Failed(m)) => {
                let status = t.stream.status();
                t.tls.close();
                t.out.extend(t.tls.outgoing());
                stack.send(t.socket, &t.out);
                self.close(now, stack, t.socket);
                // An organization can switch the web tools off for its keys: then the API refuses any request that
                // offers them. Asked again without them, and not offered again.
                let lower = m.to_ascii_lowercase();
                if self.web && status == Some(400) && ["web search", "web_search", "web fetch", "web_fetch"].iter().any(|w| lower.contains(w)) {
                    crate::log!("Chat Computer kernel: Claude: the web tools aren't enabled for this API key; asking without them");
                    self.web = false;
                    t.again_without_web = true;
                    return true;
                }
                return fail(events, api_text(status, &m));
            }
            _ => {}
        }
        t.out.extend(t.tls.outgoing());
        if !t.out.is_empty() {
            let n = stack.send(t.socket, &t.out);
            t.out.drain(..n);
        }
        if stack.state(t.socket) == TcpState::Closed {
            let why = tcp_text(stack.error(t.socket));
            stack.release(t.socket);
            return fail(events, format!("The connection to Claude's API broke: {why}."));
        }
        let limit = if t.sent { SILENCE_MS } else { HANDSHAKE_MS };
        if now.saturating_sub(t.heard) > limit {
            stack.abort(t.socket);
            return fail(events, String::from("Claude's API stopped answering."));
        }
        false
    }

    fn close(&mut self, now: u64, stack: &mut Stack, s: Socket) {
        stack.close(s);
        self.closing.push((s, now));
    }
}
