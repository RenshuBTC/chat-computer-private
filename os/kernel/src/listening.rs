//! Listening: what the person says, in words. The talk key (Windows+H, as Windows' own voice typing has it, or
//! Ctrl+Space) starts it: the microphone is recorded (sound.rs), brought down to 16 kHz, and once the person pauses,
//! what they said is heard by Chat Computer's own speech recognition (OpenAI's Whisper, run by the engine:
//! model/src/whisper.rs), a step at a time beside the screen and the keyboard, and sent as their message.
//!
//! The model is the one Chat Computer's app got ready on Windows, in its models folder's speech folder
//! (whisper-small.en.ccm, say), read into memory the first time it's needed. When they've begun to speak and then
//! pause for 0.8 seconds, what they said is heard (with a little of the quiet before it and after); if they say
//! nothing for 8 seconds, it stops. What it heard is let go of as soon as it's words: nothing is kept or sent
//! anywhere. A transcript that's no words, or one of the phrases Whisper makes up from noise, isn't sent (the same
//! rules as Chat Computer's app's).

use crate::{files, heap, sound, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use model::whisper::{Hearing, Transcript, Whisper};

/// How long each step of reading the model's file or hearing takes, about.
const STEP_MS: u64 = 40;
/// A frame of the voice detector: 20 ms at 16 kHz.
const FRAME: usize = 320;
/// The voice has begun after this many frames of it in a row (60 ms), when it was quiet for this many before (0.1
/// s: so a recording that starts in the middle of something said waits for its end).
const START_FRAMES: usize = 3;
const QUIET_BEFORE: usize = 5;
/// The voice is over after this many quiet frames (0.8 s).
const PAUSE_FRAMES: usize = 40;
/// Kept before the voice began (0.3 s) and after it was last heard (0.3 s), as Chat Computer's app keeps them.
const BEFORE: usize = 15 * FRAME;
const AFTER: usize = 15 * FRAME;
/// Nothing said for this long: it stops. The most it records: Whisper hears 30 seconds.
const NOTHING_MS: u64 = 8_000;
const MOST_SAMPLES: usize = 29 * 16_000;
/// A frame is the voice when it's this many times as loud as the quiet so far, and at least this loud (root mean
/// square, of 1).
const OVER_QUIET: f32 = 3.0;
const LEAST_VOICE: f32 = 0.004;
/// Memory the rest of the system keeps, beyond the model's file and its work.
const ROOM: u64 = 256 << 20;

/// What it says to the rest of the kernel.
pub enum Heard {
    /// It's listening.
    Listening,
    /// It heard the voice end, and is hearing what was said.
    Hearing,
    /// The words said.
    Words(String),
    /// Nothing to send: why.
    Nothing(String),
}

/// The speech recognition model, as far as it's got.
enum Model {
    /// None (why).
    None(String),
    /// Found (where), not read yet.
    Found(String),
    Reading { file: files::Opened, name: String, bytes: Vec<u8>, chunk: usize },
    Ready { whisper: &'static Whisper<'static>, name: String },
}

enum Now {
    Off,
    Recording(Box<Recording>),
    Hearing { hearing: Box<Hearing>, step: usize, seconds: f32, started: u64 },
}

struct Recording {
    started: u64,
    /// What the microphone gave since last asked (48 kHz), and the 48 kHz samples' tail the filter still needs.
    raw: Vec<i16>,
    tail: Vec<f32>,
    /// The recording at 16 kHz.
    samples: Vec<i16>,
    /// How far the voice detector has looked (in samples), the quiet's loudness so far, frames of voice and of quiet
    /// in a row, and where the voice began and was last heard.
    looked: usize,
    floor: f32,
    voiced: usize,
    quiet: usize,
    began: Option<usize>,
    last: usize,
    /// The talk key pressed again: what was said so far is heard now.
    enough: bool,
}

struct Listening {
    model: Model,
    now: Now,
    /// Whether a start was asked for while the model was being read.
    wanted: bool,
}

static LISTENING: crate::sync::SpinLock<Option<Listening>> = crate::sync::SpinLock::new(None);

fn test_build() -> bool {
    option_env!("CHATCOMPUTER_TEST") == Some("listening")
}

/// Looks for the speech recognition model (it's read when it's first needed).
pub fn init() {
    let model = match choose() {
        Ok(path) => {
            crate::log!("Chat Computer kernel: listening: speech recognition's model: {path}");
            Model::Found(path)
        }
        Err(why) => {
            crate::log!("Chat Computer kernel: listening: {why}");
            Model::None(why)
        }
    };
    *LISTENING.lock() = Some(Listening { model, now: Now::Off, wanted: false });
}

/// The speech recognition model to use: the app's (in its models folder's speech folder), the biggest that fits.
fn choose() -> Result<String, String> {
    let found = files::app_speech_models().unwrap_or_default();
    let room = room();
    let fits = found.iter().filter(|(_, size)| size + ROOM <= room).max_by_key(|(_, size)| *size);
    match fits {
        Some((path, _)) => Ok(path.clone()),
        None if found.is_empty() => Err(String::from(
            "Chat Computer's own speech recognition needs its model, which Chat Computer's app gets ready on Windows",
        )),
        None => Err(String::from("this PC's memory isn't enough for Chat Computer's speech recognition as well as its model")),
    }
}

fn room() -> u64 {
    heap::could_grow() + heap::largest_free() as u64
}

/// Whether it's recording or hearing (the screen says so, and the main loop doesn't wait).
pub fn state() -> Option<bool> {
    match LISTENING.lock().as_ref().map(|l| &l.now) {
        Some(Now::Recording(_)) => Some(false),
        Some(Now::Hearing { .. }) => Some(true),
        _ => None,
    }
}

/// Whether there's work to do between interrupts.
pub fn working() -> bool {
    LISTENING.lock().as_ref().is_some_and(|l| !matches!(l.now, Now::Off) || matches!(l.model, Model::Reading { .. }))
}

/// Whether it can listen: a speech recognition model found, and a microphone.
pub fn can() -> bool {
    let has_model = LISTENING.lock().as_ref().is_some_and(|l| !matches!(l.model, Model::None(_)));
    has_model && sound::has_input()
}

/// The talk key: starts listening, or (while it records) hears what was said so far.
pub fn talk() -> Result<(), String> {
    let mut guard = LISTENING.lock();
    let Some(l) = guard.as_mut() else { return Err(String::from("Chat Computer isn't ready to listen yet")) };
    match &mut l.now {
        Now::Recording(r) => {
            // (Pressed again: what was said so far is heard now.)
            r.enough = true;
            Ok(())
        }
        Now::Hearing { .. } => Ok(()),
        Now::Off => {
            if let Model::None(_) = &l.model {
                // (Looked for again: a drive may have been unlocked, or the app got it ready since.)
                match choose() {
                    Ok(path) => l.model = Model::Found(path),
                    Err(why) => return Err(format!("{why}.")),
                }
            }
            l.wanted = true;
            Ok(())
        }
    }
}

/// Stops listening, without hearing what was said.
pub fn cancel() {
    if let Some(l) = LISTENING.lock().as_mut() {
        if matches!(l.now, Now::Recording(_)) {
            sound::stop_listening();
        }
        l.now = Now::Off;
        l.wanted = false;
    }
}

/// Its line in the computer's status.
pub fn status() -> String {
    match LISTENING.lock().as_ref().map(|l| &l.model) {
        Some(Model::Ready { name, .. }) => format!("Speech recognition: {name}, Chat Computer's own (Windows+H to talk)"),
        Some(Model::Reading { name, .. }) => format!("Speech recognition: {name}, being read"),
        Some(Model::Found(path)) => format!("Speech recognition: {}, read when first needed (Windows+H to talk)", name_of(path)),
        Some(Model::None(why)) => format!("Speech recognition: none ({why})"),
        None => String::from("Speech recognition: not looked for"),
    }
}

fn name_of(path: &str) -> String {
    let file = path.rsplit('\\').next().unwrap_or(path);
    String::from(file.strip_suffix(".ccm").unwrap_or(file))
}

/// A step: the model's file read, the microphone's samples taken and listened to, or a step of hearing.
pub fn work() -> Vec<Heard> {
    let mut out = Vec::new();
    let mut guard = LISTENING.lock();
    let Some(l) = guard.as_mut() else { return out };
    let until = time::uptime_ms() + STEP_MS;
    // The model, read the first time it's wanted.
    if l.wanted {
        match &mut l.model {
            Model::Found(path) => {
                let path = path.clone();
                match start_reading(&path) {
                    Ok(m) => l.model = m,
                    Err(why) => {
                        l.wanted = false;
                        l.model = Model::None(why.clone());
                        out.push(Heard::Nothing(why));
                    }
                }
            }
            Model::None(why) => {
                l.wanted = false;
                out.push(Heard::Nothing(why.clone()));
            }
            Model::Reading { .. } => {
                if let Err(why) = read_more(&mut l.model, until) {
                    l.wanted = false;
                    crate::log!("Chat Computer kernel: listening: {why}");
                    l.model = Model::None(why.clone());
                    out.push(Heard::Nothing(why));
                }
            }
            Model::Ready { .. } => {
                l.wanted = false;
                match sound::listen() {
                    Ok(what) => {
                        crate::log!("Chat Computer kernel: listening: to {what}");
                        l.now = Now::Recording(Box::new(Recording {
                            started: time::uptime_ms(),
                            raw: Vec::new(),
                            tail: Vec::new(),
                            samples: Vec::new(),
                            looked: 0,
                            floor: f32::INFINITY,
                            voiced: 0,
                            quiet: 0,
                            began: None,
                            last: 0,
                            enough: false,
                        }));
                        out.push(Heard::Listening);
                    }
                    Err(why) => out.push(Heard::Nothing(format!("Chat Computer can't listen: {why}."))),
                }
            }
        }
    }
    let Model::Ready { whisper, .. } = &l.model else { return out };
    let whisper: &'static Whisper<'static> = whisper;
    match &mut l.now {
        Now::Off => {}
        Now::Recording(r) => {
            sound::heard(&mut r.raw);
            let raw = core::mem::take(&mut r.raw);
            down_to_16k(&raw, &mut r.tail, &mut r.samples);
            match detect(r) {
                Detected::Going => {}
                Detected::Nothing => {
                    sound::stop_listening();
                    l.now = Now::Off;
                    out.push(Heard::Nothing(String::from("Chat Computer didn't hear anything said, so it stopped listening.")));
                }
                Detected::Said(from, to) => {
                    sound::stop_listening();
                    let said = &r.samples[from..to];
                    if test_build() {
                        test_log_samples(said);
                    }
                    let floats: Vec<f32> = said.iter().map(|&s| s as f32 / 32768.0).collect();
                    let seconds = floats.len() as f32 / 16_000.0;
                    let hearing = whisper.begin(&floats, Some("en"));
                    crate::log!("Chat Computer kernel: listening: heard {seconds:.1} s said; hearing it");
                    l.now = Now::Hearing { hearing: Box::new(hearing), step: 16, seconds, started: time::uptime_ms() };
                    out.push(Heard::Hearing);
                }
            }
        }
        Now::Hearing { hearing, step, seconds, started } => {
            while time::uptime_ms() < until {
                let t = time::uptime_ms();
                let done = whisper.advance(hearing, *step, &crate::cores::Cores);
                let took = time::uptime_ms() - t;
                if took < STEP_MS / 4 {
                    *step = (*step * 2).min(1500);
                } else if took > STEP_MS {
                    *step = (*step / 2).max(1);
                }
                if let Some(t) = done {
                    let ms = time::uptime_ms() - *started;
                    crate::log!(
                        "Chat Computer kernel: listening: heard {} tokens in {:.1} s ({:.1} s said){}",
                        t.tokens.len(),
                        ms as f64 / 1000.0,
                        seconds,
                        if test_build() { format!(": {}", test_json(&t)) } else { String::new() }
                    );
                    let noise = is_noise(&t, *seconds);
                    if test_build() {
                        crate::log!("Chat Computer kernel: test: {}", if noise { "not sent: no words" } else { "sent as the message" });
                    }
                    l.now = Now::Off;
                    if noise {
                        out.push(Heard::Nothing(String::from("Chat Computer didn't hear any words in that.")));
                    } else {
                        out.push(Heard::Words(String::from(t.text.trim())));
                    }
                    break;
                }
            }
        }
    }
    out
}

fn start_reading(path: &str) -> Result<Model, String> {
    let file = files::open(path).map_err(|why| format!("Chat Computer's speech recognition can't be read: {why}."))?;
    let name = name_of(&file.path);
    if file.size + ROOM > room() {
        return Err(String::from("This PC's memory isn't enough for Chat Computer's speech recognition as well as its model."));
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(file.size as usize).is_err() {
        return Err(String::from("This PC's memory isn't enough for Chat Computer's speech recognition as well as its model."));
    }
    crate::log!("Chat Computer kernel: listening: reading {name} ({} MB)", file.size >> 20);
    Ok(Model::Reading { file, name, bytes, chunk: 1 << 20 })
}

/// Reads more of the model's file, until `until`; once it's all read, the model made ready.
fn read_more(model: &mut Model, until: u64) -> Result<(), String> {
    let Model::Reading { file, name, bytes, chunk } = model else { return Ok(()) };
    let total = file.size as usize;
    while bytes.len() < total && time::uptime_ms() < until {
        let at = bytes.len();
        let want = (total - at).min(*chunk);
        bytes.resize(at + want, 0);
        let t = time::uptime_ms();
        let n = files::read_at(file, at as u64, &mut bytes[at..])?;
        bytes.truncate(at + n);
        if n == 0 {
            return Err(format!("{}: the file ended sooner than it said", file.path));
        }
        let took = time::uptime_ms() - t;
        if took < STEP_MS / 4 {
            *chunk = (*chunk * 2).min(32 << 20);
        } else if took > STEP_MS * 2 {
            *chunk = (*chunk / 2).max(64 << 10);
        }
    }
    if bytes.len() < total {
        return Ok(());
    }
    let name = core::mem::take(name);
    let path = file.path.clone();
    let all: &'static [u8] = Vec::leak(core::mem::take(bytes));
    let bad = |why: String| format!("{path} isn't a speech recognition model Chat Computer's engine can run ({why}).");
    let file = model::File::open(all).map_err(|e| bad(format!("{e:?}")))?;
    let file: &'static model::File<'static> = Box::leak(Box::new(file));
    let whisper = Whisper::load(file).map_err(|e| bad(format!("{e:?}")))?;
    let whisper: &'static Whisper<'static> = Box::leak(Box::new(whisper));
    crate::log!(
        "Chat Computer kernel: listening: {name} ready ({} layers, {} wide{}), its arithmetic {}",
        whisper.settings.encoder_layers,
        whisper.settings.width,
        if whisper.multilingual() { ", many languages" } else { ", English" },
        crate::assistant::level_name()
    );
    *model = Model::Ready { whisper, name };
    Ok(())
}

/// 48 kHz to 16 kHz: a low-pass filter (a windowed sinc, its cut-off at 7.2 kHz), every third sample kept. `tail`:
/// the 48 kHz samples kept from before (as f32), which the filter still needs; and how far into them the next
/// sample is.
fn down_to_16k(raw: &[i16], tail: &mut Vec<f32>, out: &mut Vec<i16>) {
    const TAPS: usize = 63;
    static COEFFS: crate::sync::SpinLock<Option<[f32; TAPS]>> = crate::sync::SpinLock::new(None);
    let coeffs = {
        let mut c = COEFFS.lock();
        *c.get_or_insert_with(|| {
            let mut c = [0.0f32; TAPS];
            let fc = 7200.0 / 48000.0;
            let mid = (TAPS / 2) as f64;
            let mut sum = 0.0f64;
            for (i, v) in c.iter_mut().enumerate() {
                let x = i as f64 - mid;
                let sinc = if x == 0.0 { 2.0 * fc } else { model::math::sin_cos(2.0 * core::f64::consts::PI * fc * x).0 / (core::f64::consts::PI * x) };
                // (A Blackman window.)
                let t = 2.0 * core::f64::consts::PI * i as f64 / (TAPS - 1) as f64;
                let w = 0.42 - 0.5 * model::math::sin_cos(t).1 + 0.08 * model::math::sin_cos(2.0 * t).1;
                *v = (sinc * w) as f32;
                sum += sinc * w;
            }
            for v in c.iter_mut() {
                *v = (*v as f64 / sum) as f32;
            }
            c
        })
    };
    tail.extend(raw.iter().map(|&s| s as f32));
    // (The filter's output at every third sample, from where its taps fit.)
    let mut at = 0;
    while at + TAPS <= tail.len() {
        let v: f32 = coeffs.iter().zip(&tail[at..at + TAPS]).map(|(c, s)| c * s).sum();
        let v = v.clamp(-32768.0, 32767.0);
        out.push(if v >= 0.0 { (v + 0.5) as i32 } else { (v - 0.5) as i32 }.clamp(-32768, 32767) as i16);
        at += 3;
    }
    tail.drain(..at);
}

enum Detected {
    Going,
    /// Nothing was said in time.
    Nothing,
    /// The voice was over: what was said (its samples, at 16 kHz).
    Said(usize, usize),
}

/// The voice detector, over the frames not looked at yet.
fn detect(r: &mut Recording) -> Detected {
    while r.looked + FRAME <= r.samples.len() {
        let frame = &r.samples[r.looked..r.looked + FRAME];
        let rms = model::math::sqrt(frame.iter().map(|&s| (s as f32 / 32768.0) * (s as f32 / 32768.0)).sum::<f32>() / FRAME as f32);
        let floor = if r.floor.is_finite() { r.floor } else { rms };
        let voice = rms > (floor * OVER_QUIET).max(LEAST_VOICE);
        // (The quiet's loudness: the quietest frame lately, rising slowly so it follows a room getting louder.)
        r.floor = if voice { floor * 1.002 } else { rms.min(floor * 1.02).max(1e-5) };
        r.looked += FRAME;
        if voice {
            r.voiced += 1;
            if r.began.is_none() && r.voiced >= START_FRAMES && r.quiet >= QUIET_BEFORE {
                r.began = Some(r.looked.saturating_sub(START_FRAMES * FRAME + BEFORE));
            }
            if r.began.is_some() {
                r.quiet = 0;
                r.last = r.looked;
            }
        } else {
            r.voiced = 0;
            r.quiet += 1;
        }
        if let Some(began) = r.began
            && (r.quiet >= PAUSE_FRAMES || r.looked - began >= MOST_SAMPLES)
        {
            let end = (r.last + AFTER).min(r.samples.len());
            return Detected::Said(began, end);
        }
    }
    if r.enough {
        // (Pressed again while it recorded: what was said so far, if anything was.)
        return match r.began {
            Some(began) => Detected::Said(began, r.samples.len()),
            None => Detected::Nothing,
        };
    }
    if r.began.is_none() && time::uptime_ms() >= r.started + NOTHING_MS {
        return Detected::Nothing;
    }
    Detected::Going
}

/// Phrases Whisper makes up from silence or noise, as Chat Computer's app lists them (voice/stt.py).
const MADE_UP: [&str; 22] = [
    "thank you", "thank you.", "thanks for watching", "thanks for watching!", "thank you for watching", "you", "bye",
    "bye.", "bye-bye", "okay.", "so", "uh", "um", "hmm", "mm", "the end", ".", "...",
    "subtitles by the amara.org community", "please subscribe", "like and subscribe", "i'm sorry",
];

/// Whether a transcript is noise rather than words (Chat Computer's app's rules: voice/stt.py's is_noise).
fn is_noise(t: &Transcript, seconds: f32) -> bool {
    let text = t.text.trim().to_lowercase();
    if text.is_empty() || !text.chars().any(|c| c.is_alphanumeric() || c == '_') {
        return true;
    }
    if MADE_UP.contains(&text.as_str()) && (t.no_speech > 0.3 || seconds < 1.2 || t.avg_logprob < -0.7) {
        return true;
    }
    t.no_speech > 0.75 && t.avg_logprob < -0.8
}

/// A test build's log of what was heard (check.py --listen compares it with the engine on Linux).
fn test_json(t: &Transcript) -> String {
    let tokens: Vec<String> = t.tokens.iter().map(|v| format!("{v}")).collect();
    format!("{{\"tokens\": [{}], \"no_speech\": {}, \"avg_logprob\": {}}}", tokens.join(", "), t.no_speech, t.avg_logprob)
}

/// A test build's log of the samples heard (16 kHz, 16 bits), in base64, for check.py to hear them on Linux too.
fn test_log_samples(s: &[i16]) {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = s.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(ABC[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    crate::log!("Chat Computer kernel: test: said {} samples: {out}", s.len());
}
