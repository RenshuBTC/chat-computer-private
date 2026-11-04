//! Chat Computer's own model engine (os/model) in a web page: chatcomputer.ai's chat answers with it, free and with
//! no account, the model running in the visitor's browser as WebAssembly, with no other program or library (no
//! ONNX Runtime, no llama.cpp: the engine is the one Chat Computer runs on a PC, its kernel included).
//!
//! The page's Web Worker (docs/demo/engine/worker.js) downloads Qwen's published files from Qwen's own repository at
//! the revision Chat Computer pins (chatcomputer/brain/models.json), checks each against its pinned SHA-256 as it
//! comes (os/crypto's), makes the model's file from them as they come (convert.rs: the same file the engine's
//! program makes), keeps it in the browser, and runs it: a conversation as OpenAI's chat completions API has it
//! (the engine's program's own reading of a request, src/bin/chat-computer-model/api.rs), answered a step at a
//! time, each step's words, thinking or tool call given to the page as the program streams them.
//!
//! This is the module's side: plain functions for JavaScript (no generated glue), on numbers and on bytes in the
//! module's memory. JavaScript asks for room (`cc_alloc`), puts bytes there and passes where and how many; what comes
//! back is in one place (`cc_out_ptr`, `cc_out_len`): JSON, or bytes. A function that fails returns -1 with
//! {"error": "..."} there. One processor (a web page's worker can't share memory with another without headers
//! GitHub Pages doesn't send), so the arithmetic runs on one (net::Serial), with WebAssembly's 128-bit SIMD.

pub mod convert;
#[cfg(not(target_arch = "wasm32"))]
pub mod fixture;

// The engine's program's own reading of OpenAI's requests, and its way of writing a tool call and a step of an
// answer, so a page's request means here what it means to the program on a PC, and its answers stream the same.
#[allow(dead_code)]
#[path = "../../model/src/bin/chat-computer-model/api.rs"]
mod api;

// The engine's program's own converter, for the tests: a page must make the same file, byte for byte.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../model/src/bin/chat-computer-model/convert.rs"]
mod program_convert;

use crypto::Hash;
use model::chat::{Answer, Said, Style};
use model::json::{self, Json};
use model::refusal::Found;
use model::sample::Sampling;
use model::session::{Ended, Request, Session, Step, Turn};
use model::{File, Model, Serial};
use std::cell::RefCell;

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
    /// What's given back to JavaScript: JSON, or bytes (kept apart from the engine, so it's readable whatever state
    /// the engine was left in).
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// What the last panic said (the module stops there: JavaScript sees a trap, then reads this).
    static PANIC: RefCell<String> = const { RefCell::new(String::new()) };
}

#[derive(Default)]
struct Engine {
    hashes: Vec<Option<crypto::Sha256>>,
    converter: Option<convert::Converter>,
    /// The model's file, made here or read from the browser's store, before it's loaded.
    file: Option<Vec<u8>>,
    loaded: Option<Loaded>,
    #[cfg(feature = "bench")]
    bench: Option<(&'static Model<'static>, model::State)>,
}

struct Loaded {
    session: Session<'static>,
    style_name: String,
    /// The model file's own mark (with what's taken out as it runs): a conversation's reading kept in the browser is
    /// used only with the model it was read by.
    fingerprint: u64,
    answering: Option<Answering>,
    /// The first message's tokens being read ahead of the question.
    ahead: Vec<u32>,
    /// Requests answered, for their ids (as the program numbers them).
    asked: u64,
}

/// An answer being written: its turn, the tools it was given (its tool calls' arguments are typed by them), and its
/// id.
struct Answering {
    turn: Turn,
    tools: Vec<Json>,
    id: String,
}

fn put_out(bytes: Vec<u8>) {
    OUT.with_borrow_mut(|o| *o = bytes);
}

fn put_json(j: Json) {
    put_out(json::compact(&j).into_bytes());
}

fn obj(items: Vec<(&str, Json)>) -> Json {
    Json::Object(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn num(n: impl Into<f64>) -> Json {
    Json::Number(n.into())
}

/// A function's result for JavaScript: 0 (or what it returns) when it went well, -1 with the reason given back.
fn done(r: Result<i32, String>) -> i32 {
    match r {
        Ok(v) => v,
        Err(e) => {
            put_json(obj(vec![("error", Json::Str(e))]));
            -1
        }
    }
}

/// Bytes JavaScript put in the module's memory.
///
/// # Safety
/// `ptr` and `len` must be a part of the module's memory that JavaScript wrote (from cc_alloc).
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(ptr, len) } }
}

/// # Safety
/// As `bytes`.
unsafe fn text<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    std::str::from_utf8(unsafe { bytes(ptr, len) }).map_err(|_| "not UTF-8".to_string())
}

// ------------------------------------------------------------------ memory, and what's given back

/// Called once, first: a panic's words are kept for JavaScript to read.
#[unsafe(no_mangle)]
pub extern "C" fn cc_init() {
    std::panic::set_hook(Box::new(|info| PANIC.with_borrow_mut(|p| *p = info.to_string())));
}

/// Room for `len` bytes, for JavaScript to fill (0: no room).
#[unsafe(no_mangle)]
pub extern "C" fn cc_alloc(len: usize) -> *mut u8 {
    match std::alloc::Layout::array::<u8>(len.max(1)) {
        Ok(layout) => unsafe { std::alloc::alloc(layout) },
        Err(_) => core::ptr::null_mut(),
    }
}

/// Gives back room from cc_alloc.
///
/// # Safety
/// `ptr` and `len` must be what cc_alloc gave and was asked for, and not given back already.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_free(ptr: *mut u8, len: usize) {
    if let (false, Ok(layout)) = (ptr.is_null(), std::alloc::Layout::array::<u8>(len.max(1))) {
        unsafe { std::alloc::dealloc(ptr, layout) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn cc_out_ptr() -> *const u8 {
    OUT.with_borrow(|o| o.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn cc_out_len() -> usize {
    OUT.with_borrow(|o| o.len())
}

/// What the last panic said, given back (empty if there was none).
#[unsafe(no_mangle)]
pub extern "C" fn cc_panic() {
    let p = PANIC.with_borrow(|p| p.clone());
    put_out(p.into_bytes());
}

// ------------------------------------------------------------------ checking what's downloaded

/// A new SHA-256 (its number).
#[unsafe(no_mangle)]
pub extern "C" fn cc_hash_new() -> i32 {
    ENGINE.with_borrow_mut(|e| {
        e.hashes.push(Some(crypto::Sha256::new()));
        e.hashes.len() as i32 - 1
    })
}

/// # Safety
/// `ptr` and `len` as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_hash_update(h: i32, ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { bytes(ptr, len) };
    done(ENGINE.with_borrow_mut(|e| {
        let s = e.hashes.get_mut(h as usize).and_then(|s| s.as_mut()).ok_or("no such hash")?;
        s.update(data);
        Ok(0)
    }))
}

/// The hash, in hexadecimal, given back; the hash is over.
#[unsafe(no_mangle)]
pub extern "C" fn cc_hash_finish(h: i32) -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let s = e.hashes.get_mut(h as usize).and_then(|s| s.take()).ok_or("no such hash")?;
        let hex: String = s.finish().iter().map(|b| format!("{b:02x}")).collect();
        put_out(hex.into_bytes());
        Ok(0)
    }))
}

// ------------------------------------------------------------------ making the model's file

/// Starts making a model's file from Qwen's published files, its big weights in `bits` bits a number (8 or 4; 16:
/// as published).
#[unsafe(no_mangle)]
pub extern "C" fn cc_convert_new(bits: u32) -> i32 {
    ENGINE.with_borrow_mut(|e| e.converter = Some(convert::Converter::new(bits)));
    0
}

/// One of the small files: config.json, chat_template.jinja, tokenizer_config.json or tokenizer.json (its name, its
/// bytes).
///
/// # Safety
/// The pointers and lengths as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_convert_file(name: *const u8, name_len: usize, ptr: *const u8, len: usize) -> i32 {
    let (name, data) = (unsafe { text(name, name_len) }, unsafe { bytes(ptr, len) });
    done(ENGINE.with_borrow_mut(|e| {
        e.converter.as_mut().ok_or("no model's file being made")?.file(name?, data)?;
        Ok(0)
    }))
}

/// A weights file's header (its name, and the JSON after its first 8 bytes): its number, which its bytes are fed with.
///
/// # Safety
/// The pointers and lengths as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_convert_weights(name: *const u8, name_len: usize, ptr: *const u8, len: usize) -> i32 {
    let (name, header) = (unsafe { text(name, name_len) }, unsafe { bytes(ptr, len) });
    done(ENGINE.with_borrow_mut(|e| Ok(e.converter.as_mut().ok_or("no model's file being made")?.weights(name?, header)? as i32)))
}

/// Plans the file, once the small files and every header have come: {"size": its bytes} given back.
#[unsafe(no_mangle)]
pub extern "C" fn cc_convert_plan() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let size = e.converter.as_mut().ok_or("no model's file being made")?.plan()?;
        put_json(obj(vec![("size", num(size as f64))]));
        Ok(0)
    }))
}

/// A weights file's next bytes, from `offset` in it (a number of bytes, which can be past 4 GB).
///
/// # Safety
/// `ptr` and `len` as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_convert_feed(file: i32, offset: f64, ptr: *const u8, len: usize) -> i32 {
    let data = unsafe { bytes(ptr, len) };
    done(ENGINE.with_borrow_mut(|e| {
        e.converter.as_mut().ok_or("no model's file being made")?.feed(file as usize, offset as u64, data)?;
        Ok(0)
    }))
}

/// Every weight has come: the file is made, ready to keep (cc_model_ptr) and to load (cc_load).
#[unsafe(no_mangle)]
pub extern "C" fn cc_convert_finish() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let c = e.converter.take().ok_or("no model's file being made")?;
        c.finish()?;
        e.file = Some(c.out);
        Ok(0)
    }))
}

// ------------------------------------------------------------------ the model

/// Room for a model's file of `len` bytes read from the browser's store (0: no room for it).
#[unsafe(no_mangle)]
pub extern "C" fn cc_model_alloc(len: usize) -> *mut u8 {
    ENGINE.with_borrow_mut(|e| {
        e.file = None;
        let mut v = Vec::new();
        if v.try_reserve_exact(len).is_err() {
            return core::ptr::null_mut();
        }
        v.resize(len, 0u8);
        let p = v.as_mut_ptr();
        e.file = Some(v);
        p
    })
}

/// Where the model's file is (made here, or room for it), and its size.
#[unsafe(no_mangle)]
pub extern "C" fn cc_model_ptr() -> *const u8 {
    ENGINE.with_borrow(|e| e.file.as_ref().map_or(core::ptr::null(), |f| f.as_ptr()))
}

#[unsafe(no_mangle)]
pub extern "C" fn cc_model_len() -> usize {
    ENGINE.with_borrow(|e| e.file.as_ref().map_or(0, |f| f.len()))
}

/// Loads the model from its file (cc_convert_finish's, or cc_model_alloc's once filled), with its refusal direction
/// if one is given (MODEL.refusal: taken out as it runs, as the program does). Its settings given back: {"layers",
/// "hidden", "vocab", "context", "style", "bytes", "uncensored"}.
///
/// # Safety
/// `refusal` and `refusal_len` as in `bytes` (or 0 and 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_load(refusal: *const u8, refusal_len: usize) -> i32 {
    let refusal = unsafe { bytes(refusal, refusal_len) };
    done(ENGINE.with_borrow_mut(|e| {
        if e.loaded.is_some() {
            return Err("a model is loaded already".into());
        }
        let bytes = e.file.take().ok_or("no model's file")?;
        let size = bytes.len();
        // (The model is the page's for good: its file's bytes are its weights. If they aren't a model, they're given
        // back: nothing else holds them then.)
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let opened = File::open(bytes).map_err(|e| format!("not a model file ({e:?})")).and_then(|file| {
            let model = Model::load(&file).map_err(|e| format!("a model the engine can't run ({e:?})"))?;
            Ok((file, model))
        });
        let (file, mut model) = match opened {
            Ok(m) => m,
            Err(why) => {
                drop(unsafe { Box::from_raw(bytes as *const [u8] as *mut [u8]) });
                return Err(why);
            }
        };
        let style_name = file.config().ok().and_then(|c| c.str("chat.style").map(String::from)).unwrap_or_else(|| "qwen3.5".into());
        let style = Style::named(&style_name).ok_or_else(|| format!("the model's chat style \"{style_name}\" isn't one the engine knows"))?;
        let mark = file.fingerprint();
        let mut fingerprint = mark;
        let mut uncensored = Json::Null;
        if !refusal.is_empty() {
            let found = Found::from_bytes(refusal, mark, model.settings.hidden).map_err(|e| format!("its refusal file isn't used: {e}"))?;
            model.ablation = found.ablation();
            if model.ablation.is_some() {
                fingerprint = model::file::fnv(mark, refusal);
            }
            uncensored = obj(vec![
                ("taken_out", Json::Bool(model.ablation.is_some())),
                ("checked", num(found.checked)),
                ("refused_before", num(found.before)),
                ("refused_after", num(found.after)),
            ]);
        }
        let model: &'static Model<'static> = Box::leak(Box::new(model));
        let s = &model.settings;
        put_json(obj(vec![
            ("layers", num(s.layers as f64)),
            ("hidden", num(s.hidden as f64)),
            ("vocab", num(s.vocab as f64)),
            ("context", num(s.context as f64)),
            ("style", Json::Str(style_name.clone())),
            ("bytes", num(size as f64)),
            ("uncensored", uncensored),
        ]));
        e.loaded = Some(Loaded { session: Session::new(model, style), style_name, fingerprint, answering: None, ahead: Vec::new(), asked: 0 });
        Ok(0)
    }))
}

fn loaded(e: &mut Engine) -> Result<&mut Loaded, String> {
    e.loaded.as_mut().ok_or_else(|| "no model is loaded".to_string())
}

/// A request as the program has it, made ready to ask: (its tools and messages, options, sampling, the most tokens
/// to write, the seed). Earlier answers keep their thinking unless asked not to, as the program does: the
/// conversation then only grows, and what was read is read once.
fn request(l: &Loaded, body: &[u8]) -> Result<(api::Asked, model::chat::Options, Sampling), String> {
    let asked = api::parse(body)?;
    let think = asked.options.think.unwrap_or(l.session.style.thinks);
    let mut options = asked.options;
    options.keep_thinking = Some(options.keep_thinking.unwrap_or(true));
    let sampling = asked.sampling(Sampling::recommended(&l.style_name, think));
    Ok((asked, options, sampling))
}

// ------------------------------------------------------------------ answering

/// Starts answering a request (OpenAI's chat completions request, as JSON: "messages", "tools", "max_tokens",
/// "think", "temperature" and the rest as the program reads them; "stream" is ignored: it's always step by step).
/// Gives back {"prompt_tokens", "reading"}: how long its prompt is, and whether any of it is still to be read (the
/// steps say how far the reading has got).
///
/// # Safety
/// `ptr` and `len` as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_chat(ptr: *const u8, len: usize) -> i32 {
    let body = unsafe { bytes(ptr, len) };
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        if l.answering.is_some() {
            return Err("an answer is being written already".into());
        }
        let (asked, options, sampling) = request(l, body)?;
        let r = Request { tools: &asked.tools, messages: &asked.messages, options, sampling, max_tokens: asked.max_tokens, seed: asked.seed };
        let turn = l.session.begin(&r).map_err(|p| format!("{p:?}"))?;
        // (Past the model's context its positions mean nothing it was taught; the page keeps conversations short.)
        if turn.prompt_tokens() >= l.session.model.settings.context {
            l.session.end(turn);
            return Err(format!("the conversation is longer than the model reads ({} tokens)", l.session.model.settings.context));
        }
        l.asked += 1;
        put_json(obj(vec![("prompt_tokens", num(turn.prompt_tokens() as f64)), ("reading", Json::Bool(turn.reading()))]));
        l.answering = Some(Answering { turn, tools: asked.tools, id: format!("chatcmpl-{}", l.asked) });
        Ok(0)
    }))
}

/// A step of the answer: up to `read` more of the prompt read (at least one), or, once it's read, one token written.
/// Returns 1 while the answer goes on, with what the step did given back: {"read": [done, total]}, or {"delta": ...}
/// (as the program streams it: {"content"}, {"reasoning_content"} or {"tool_calls": [...]}), or {} (a token that
/// added nothing yet); 0 once it's over, with {"delta"} if the last step said something and {"end": {"message",
/// "finish_reason", "usage"}}.
#[unsafe(no_mangle)]
pub extern "C" fn cc_step(read: u32) -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        let a = l.answering.as_mut().ok_or("no answer is being written")?;
        let step = l.session.step(&mut a.turn, read.max(1) as usize, &Serial);
        let delta = match step {
            Step::Read { done, total } => {
                put_json(obj(vec![("read", Json::Array(vec![num(done as f64), num(total as f64)]))]));
                return Ok(1);
            }
            Step::Seeing { .. } => {
                put_json(obj(vec![]));
                return Ok(1);
            }
            Step::Said(s) => delta(&s, a),
            Step::Over => None,
        };
        if !a.turn.finished() {
            put_json(match delta {
                Some(d) => obj(vec![("delta", d)]),
                None => obj(vec![]),
            });
            return Ok(1);
        }
        end(l, delta);
        Ok(0)
    }))
}

/// Stops the answer where it is (what it was saying is finished as it is): gives back as cc_step does at the end.
#[unsafe(no_mangle)]
pub extern "C" fn cc_stop() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        let a = l.answering.as_mut().ok_or("no answer is being written")?;
        let s = l.session.stop(&mut a.turn);
        let d = delta(&s, a);
        end(l, d);
        Ok(0)
    }))
}

/// What a token added, as the program streams it.
fn delta(s: &Said, a: &Answering) -> Option<Json> {
    match s {
        Said::Thinking(t) => Some(api::delta("reasoning_content", t)),
        Said::Text(t) => Some(api::delta("content", t)),
        Said::Call(k) => {
            let call = &a.turn.answer().calls[*k];
            Some(api::calls_delta(vec![api::call_json(call, *k, &call_id(&a.id, *k), &a.tools)]))
        }
        _ => None,
    }
}

/// A tool call's id, as the program makes it: "call_" with the request's number and the call's.
fn call_id(id: &str, k: usize) -> String {
    format!("call_{}_{k}", &id[9..])
}

/// The answer's end: its turn ended (its tokens kept, to be put back as they were), and the whole message given back
/// as the program's whole answer has it, with why it ended and the tokens it took.
fn end(l: &mut Loaded, delta: Option<Json>) {
    let a = l.answering.take().expect("an answer being written");
    let (answer, st) = l.session.end(a.turn);
    let calls: Vec<Json> = answer.calls.iter().enumerate().map(|(k, c)| api::call_json(c, k, &call_id(&a.id, k), &a.tools)).collect();
    let finish = match st.ended {
        Ended::Length => "length",
        _ if !calls.is_empty() => "tool_calls",
        _ => "stop",
    };
    let mut items = vec![];
    if let Some(d) = delta {
        items.push(("delta", d));
    }
    items.push(("end", obj(vec![
        ("message", message(&answer, calls)),
        ("finish_reason", Json::Str(finish.into())),
        ("usage", api::usage(st.prompt, st.read, st.generated)),
    ])));
    put_json(obj(items));
}

/// The assistant's message, as the program's whole answer has it: its words (none, if it only called tools), its
/// thinking, its tool calls.
fn message(a: &Answer, calls: Vec<Json>) -> Json {
    let mut m = vec![
        ("role", Json::Str("assistant".into())),
        ("content", if a.text.is_empty() && !calls.is_empty() { Json::Null } else { Json::Str(a.text.clone()) }),
    ];
    if !a.thinking.is_empty() {
        m.push(("reasoning_content", Json::Str(a.thinking.clone())));
    }
    if !calls.is_empty() {
        m.push(("tool_calls", Json::Array(calls)));
    }
    obj(m)
}

// ------------------------------------------------------------------ reading ahead, and keeping what's read

/// Starts reading a request's first message (the system prompt, with the tools) ahead of the question, while the
/// person is still typing: every request that starts the same way then reads only what follows it. The request is
/// as for cc_chat (its messages may be the system prompt alone). Gives back {"tokens"}: how many there are to read.
///
/// # Safety
/// `ptr` and `len` as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_ahead(ptr: *const u8, len: usize) -> i32 {
    let body = unsafe { bytes(ptr, len) };
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        if l.answering.is_some() {
            return Err("an answer is being written".into());
        }
        let (mut asked, options, sampling) = request(l, body)?;
        // (A question to follow, as the kernel reads ahead: only the first message is read.)
        if !asked.messages.iter().any(|m| matches!(m, model::chat::Message::User(_))) {
            asked.messages.push(model::chat::Message::User(".".into()));
        }
        let r = Request { tools: &asked.tools, messages: &asked.messages, options, sampling, max_tokens: 1, seed: 1 };
        l.ahead = l.session.ahead(&r).map_err(|p| format!("{p:?}"))?;
        put_json(obj(vec![("tokens", num(l.ahead.len() as f64))]));
        Ok(0)
    }))
}

/// Reads up to `read` more of the first message (at least one): 1 while there's more, 0 once it's all read (kept
/// for good where it ends); gives back {"over", "tokens"}.
#[unsafe(no_mangle)]
pub extern "C" fn cc_ahead_step(read: u32) -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        if l.answering.is_some() {
            return Err("an answer is being written".into());
        }
        let tokens = core::mem::take(&mut l.ahead);
        let over = l.session.read_ahead(&tokens, read.max(1) as usize, &Serial);
        put_json(obj(vec![("over", Json::Bool(over)), ("tokens", num(tokens.len() as f64))]));
        l.ahead = tokens;
        Ok(if over { 0 } else { 1 })
    }))
}

const STATE_MAGIC: &[u8; 8] = b"CCSTATE1";

/// The reading of the first message (the anchor), to keep in the browser, as the program keeps it on disk
/// (--cache): "CCSTATE1", the model's mark, the tokens, the state there. Given back as bytes (none if there's
/// nothing read to keep).
#[unsafe(no_mangle)]
pub extern "C" fn cc_anchor_export() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        let Some((tokens, state, snap)) = l.session.anchor() else {
            put_out(Vec::new());
            return Ok(0);
        };
        let mut out = Vec::new();
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&l.fingerprint.to_le_bytes());
        out.extend_from_slice(&(tokens.len() as u32).to_le_bytes());
        for t in tokens {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out.extend_from_slice(&state.export(snap));
        put_out(out);
        Ok(0)
    }))
}

/// The anchor's mark (the model's, and its tokens': what cc_anchor_export would give is the same while this is),
/// given back as text; none if nothing's read to keep.
#[unsafe(no_mangle)]
pub extern "C" fn cc_anchor_mark() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        let mark = l.session.anchor().map(|(tokens, _, _)| {
            let bytes: Vec<u8> = tokens.iter().flat_map(|t| t.to_le_bytes()).collect();
            format!("{:016x}-{:016x}-{}", l.fingerprint, model::file::fnv(0xcbf2_9ce4_8422_2325, &bytes), tokens.len())
        });
        put_out(mark.unwrap_or_default().into_bytes());
        Ok(0)
    }))
}

/// Starts from a reading kept by cc_anchor_export (for this same model): 0 if it's used, 1 if it isn't one for this
/// model.
///
/// # Safety
/// `ptr` and `len` as in `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cc_anchor_import(ptr: *const u8, len: usize) -> i32 {
    let b = unsafe { bytes(ptr, len) };
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        if l.answering.is_some() {
            return Err("an answer is being written".into());
        }
        let read = || -> Option<(Vec<u32>, model::State)> {
            if b.len() < 20 || &b[..8] != STATE_MAGIC || u64::from_le_bytes(b[8..16].try_into().ok()?) != l.fingerprint {
                return None;
            }
            let n = u32::from_le_bytes(b[16..20].try_into().ok()?) as usize;
            let tokens: Vec<u32> = b.get(20..20 + 4 * n)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
            let state = l.session.model.import(&b[20 + 4 * n..])?;
            (state.pos == tokens.len()).then_some((tokens, state))
        };
        match read() {
            Some((tokens, state)) => {
                l.session.restore(tokens, state);
                Ok(0)
            }
            None => Ok(1),
        }
    }))
}

/// Forgets the conversation read so far (the model stays loaded).
#[unsafe(no_mangle)]
pub extern "C" fn cc_clear() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let l = loaded(e)?;
        l.answering = None;
        l.session.clear();
        Ok(0)
    }))
}

// ------------------------------------------------------------------ timing (the bench feature only)

#[cfg(feature = "bench")]
mod bench;

/// Makes a model of Qwen3.5 0.8B's shape with random weights in memory (8 bits a weight): the size of its file
/// given back.
#[cfg(feature = "bench")]
#[unsafe(no_mangle)]
pub extern "C" fn cc_bench_make() -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let bytes: &'static [u8] = Box::leak(bench::qwen35_08b().into_boxed_slice());
        let file: &'static File<'static> = Box::leak(Box::new(File::open(bytes).map_err(|e| format!("{e:?}"))?));
        let model: &'static Model<'static> = Box::leak(Box::new(Model::load(file).map_err(|e| format!("{e:?}"))?));
        e.bench = Some((model, model.state()));
        put_json(obj(vec![("bytes", num(bytes.len() as f64))]));
        Ok(0)
    }))
}

/// Reads `n` tokens into the bench model's conversation (`n` = 1: writing a token, as an answer does).
#[cfg(feature = "bench")]
#[unsafe(no_mangle)]
pub extern "C" fn cc_bench_forward(n: u32) -> i32 {
    done(ENGINE.with_borrow_mut(|e| {
        let (model, state) = e.bench.as_mut().ok_or("no bench model")?;
        let tokens: Vec<u32> = (0..n).map(|i| (i * 7919 + state.pos as u32 * 31) % model.settings.vocab as u32).collect();
        let mut logits = vec![0.0f32; model.settings.vocab];
        model.forward(state, &tokens, Some(&mut logits), &Serial);
        Ok(0)
    }))
}
