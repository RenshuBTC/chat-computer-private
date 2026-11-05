//! Chat Computer's model, as a program: it serves the model to this computer's own programs (Chat Computer, its
//! desktop app's page) over HTTP on 127.0.0.1, as OpenAI's chat completions API, and makes
//! model files from the published ones.
//!
//!     chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--cache DIR] [--allow-origin ORIGIN]...
//!                                         [--refusal FILE | --no-refusal]
//!     chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]
//!     chat-computer-model find-refusal MODEL.ccm [--out FILE] [--threads N]
//!     chat-computer-model models DIR [--catalogue FILE]
//!     chat-computer-model prepare DIR [--model NAME|auto] [--catalogue FILE] [--threads N]
//!
//! find-refusal finds the direction the model refuses along (the library's refusal.rs), and keeps it beside the
//! model's file (MODEL.refusal); serve then takes it out as the model runs, so the model answers what it's asked.
//! models and prepare choose the model for this computer and get it ready: downloaded over Chat Computer's own
//! HTTPS, checked, converted and uncensored (prepare.rs).
//!
//! With --exit-with-stdin (any command), the program ends when its standard input closes: an app that starts it
//! keeps that open, so a model left running by an app that quit, or crashed, doesn't keep its memory and its port.
//!
//! Only this computer can reach it (it listens on 127.0.0.1), and of web pages only Chat Computer's own (its site
//! and its app, and any origin given with --allow-origin): any other page a browser
//! shows is refused, so no site can use the model behind the person's back. The model only writes: it runs nothing
//! and opens nothing (what Chat Computer does with its answers is Chat Computer's, with its own permissions).

mod api;
mod convert;
mod fetch;
mod http;
mod map;
mod prepare;
mod sys;

use model::chat::Style;
use model::net::Pool;
use model::sample::Sampling;
use model::session::{Ended, Request, Session};
use model::threads::Threads;
use model::refusal::{self, Found};
use model::{File, Model};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Whether it ends with its standard input (--exit-with-stdin): what it starts of itself does too.
pub static WITH_STDIN: AtomicBool = AtomicBool::new(false);

/// Pages that may use the model: Chat Computer's site, and its desktop app's own page (Tauri's origins).
const ORIGINS: [&str; 5] = ["https://chatcomputer.ai", "https://www.chatcomputer.ai", "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

struct Engine {
    session: Session<'static>,
    pool: Threads,
    /// Where the anchor (the system prompt read, with the tools) is kept on disk, and the last one kept there.
    cache: Option<std::path::PathBuf>,
    kept: u64,
}

struct Server {
    engine: Mutex<Engine>,
    /// The model file's own mark: a state kept on disk is used only with the file it was read by.
    fingerprint: u64,
    name: String,
    style_name: String,
    /// What's taken out as the model runs (its refusals), if anything.
    uncensored: Option<Found>,
    origins: Vec<String>,
    port: u16,
    ids: AtomicU64,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--cache DIR] [--allow-origin ORIGIN]... \
                   [--refusal FILE | --no-refusal]\n\
                   chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]\n\
                   chat-computer-model find-refusal MODEL.ccm [--out FILE] [--threads N]\n\
                   chat-computer-model models DIR [--catalogue FILE]\n\
                   chat-computer-model prepare DIR [--model NAME|auto] [--catalogue FILE] [--threads N]");
        std::process::exit(2);
    }
    let command = args.remove(0);
    if let Some(i) = args.iter().position(|a| a == "--exit-with-stdin") {
        args.remove(i);
        WITH_STDIN.store(true, Ordering::Relaxed);
        std::thread::spawn(|| {
            let (mut b, mut input) = ([0u8; 256], std::io::stdin());
            loop {
                if matches!(input.read(&mut b), Ok(0) | Err(_)) {
                    std::process::exit(0);
                }
            }
        });
    }
    let code = match command.as_str() {
        "serve" => serve(args),
        "convert" => convert::main_with(args),
        "find-refusal" => find_refusal(args),
        "models" => prepare::models_main(args),
        "prepare" => prepare::prepare_main(args),
        other => {
            eprintln!("no command \"{other}\" (serve, convert, find-refusal, models or prepare)");
            2
        }
    };
    std::process::exit(code);
}

/// Where a model's refusal direction is kept: beside its file, as MODEL.refusal.
fn refusal_path(model: &str) -> std::path::PathBuf {
    std::path::Path::new(model).with_extension("refusal")
}

/// The model file, mapped and read: the file's table, and the model (whose weights stay in the file's bytes).
fn open(path: &str, command: &str) -> Result<(&'static File<'static>, Model<'static>), i32> {
    let bytes = map::map(path).map_err(|e| {
        eprintln!("{command}: {e}");
        1
    })?;
    let file: &'static File = Box::leak(Box::new(File::open(bytes).map_err(|e| {
        eprintln!("{command}: {path} isn't a model file ({e:?})");
        1
    })?));
    let model = Model::load(file).map_err(|e| {
        eprintln!("{command}: the model in {path} can't be run ({e:?})");
        1
    })?;
    Ok((file, model))
}

fn style_of(file: &File) -> Result<(String, Style), i32> {
    let config = file.config().ok();
    let name = config.as_ref().and_then(|c| c.str("chat.style")).unwrap_or("qwen3.5").to_string();
    match Style::named(&name) {
        Some(style) => Ok((name, style)),
        None => {
            eprintln!("the model's chat style \"{name}\" isn't one this program knows");
            Err(1)
        }
    }
}

fn find_refusal(args: Vec<String>) -> i32 {
    let (mut path, mut out, mut threads) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--out" => out = Some(std::path::PathBuf::from(value)),
            "--threads" => threads = value.parse().ok(),
            p if path.is_none() && !p.starts_with("--") => {
                path = Some(p.to_string());
                i += 1;
                continue;
            }
            other => {
                eprintln!("find-refusal: {other}?");
                return 2;
            }
        }
        i += 2;
    }
    let Some(path) = path else {
        eprintln!("find-refusal: which model file?");
        return 2;
    };
    let out = out.unwrap_or_else(|| refusal_path(&path));
    let (file, mut model) = match open(&path, "find-refusal") {
        Ok(m) => m,
        Err(code) => return code,
    };
    let style = match style_of(file) {
        Ok((_, s)) => s,
        Err(code) => return code,
    };
    let pool = match threads {
        Some(n) => Threads::new(usize::max(n, 1) - 1),
        None => Threads::for_this_computer(),
    };
    let started = std::time::Instant::now();
    let mut last = String::new();
    let found = refusal::find(&mut model, style, &pool, &mut |done, total, what| {
        // (A line a step: what the step is, and how far it's got.)
        if what != last || done == total || done % 8 == 0 {
            eprintln!("{done}/{total} {what}");
            last = what.to_string();
        }
    });
    let found = match found {
        Ok(f) => f,
        Err(e) => {
            eprintln!("find-refusal: {e}");
            return 1;
        }
    };
    let part = out.with_extension("refusal.part");
    if let Err(e) = std::fs::write(&part, found.to_bytes(file.fingerprint())).and_then(|_| std::fs::rename(&part, &out)) {
        eprintln!("find-refusal: couldn't keep it in {}: {e}", out.display());
        return 1;
    }
    println!(
        "{{\"layer\":{},\"strength\":{},\"checked\":{},\"refused_before\":{},\"refused_after\":{},\"kl\":{:.4},\
         \"everyday\":{},\"everyday_same\":{},\"everyday_refused\":{},\"seconds\":{:.0}}}",
        found.layer + 1, found.strength, found.checked, found.before, found.after, found.kl, found.everyday, found.same,
        found.everyday_refused, started.elapsed().as_secs_f64()
    );
    0
}

fn serve(args: Vec<String>) -> i32 {
    let (mut path, mut port, mut threads, mut extra, mut cache) = (None, 8765u16, None, Vec::new(), None);
    let mut refusal_file: Option<Option<std::path::PathBuf>> = None;
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--port" => port = value.parse().unwrap_or(port),
            "--threads" => threads = value.parse().ok(),
            "--allow-origin" => extra.push(value),
            "--cache" => cache = Some(std::path::PathBuf::from(value)),
            "--refusal" => refusal_file = Some(Some(std::path::PathBuf::from(value))),
            "--no-refusal" => {
                refusal_file = Some(None);
                i += 1;
                continue;
            }
            p if path.is_none() && !p.starts_with("--") => {
                path = Some(p.to_string());
                i += 1;
                continue;
            }
            other => {
                eprintln!("serve: {other}?");
                return 2;
            }
        }
        i += 2;
    }
    let Some(path) = path else {
        eprintln!("serve: which model file?");
        return 2;
    };
    let started = std::time::Instant::now();
    let (file, mut model) = match open(&path, "serve") {
        Ok(m) => m,
        Err(code) => return code,
    };
    let (style_name, style) = match style_of(file) {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Its refusals taken out, where they've been found (beside the model's file, unless told otherwise). What's
    // kept on disk of a conversation is kept for the model as it runs: with them taken out or not.
    let mark = file.fingerprint();
    let mut fingerprint = mark;
    let mut uncensored = None;
    if let Some(at) = refusal_file.unwrap_or_else(|| Some(refusal_path(&path))) {
        match std::fs::read(&at) {
            Ok(b) => match Found::from_bytes(&b, mark, model.settings.hidden) {
                Ok(f) => {
                    model.ablation = f.ablation();
                    if model.ablation.is_some() {
                        fingerprint = model::file::fnv(mark, &b);
                    }
                    uncensored = Some(f);
                }
                Err(e) => eprintln!("serve: {} isn't used: {e}", at.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("serve: {}: {e}", at.display()),
        }
    }
    let model: &'static Model<'static> = Box::leak(Box::new(model));
    let name = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let pool = match threads {
        Some(n) => Threads::new(usize::max(n, 1) - 1),
        None => Threads::for_this_computer(),
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("serve: port {port}: {e}");
            return 1;
        }
    };
    let mut origins: Vec<String> = ORIGINS.iter().map(|s| s.to_string()).collect();
    origins.extend(extra);
    let mut session = Session::new(model, style);
    let mut kept = 0;
    if let Some(dir) = &cache
        && let Some((tokens, state)) = load_anchor(dir, fingerprint, model)
    {
        eprintln!("picked up {} tokens already read (the system prompt and tools)", tokens.len());
        kept = hash_tokens(&tokens);
        session.restore(tokens, state);
    }
    let server: &'static Server = Box::leak(Box::new(Server {
        engine: Mutex::new(Engine { session, pool, cache, kept }),
        fingerprint,
        name,
        style_name,
        uncensored,
        origins,
        port,
        ids: AtomicU64::new(1),
    }));
    eprintln!(
        "serving {} ({} layers, {} wide, {:?}{}) on http://127.0.0.1:{port}, ready in {:.1} s",
        server.name, model.settings.layers, model.settings.hidden, model::cpu::level(),
        match &server.uncensored {
            Some(f) if model.ablation.is_some() => format!(", its refusals taken out: {} of {} refused, was {}", f.after, f.checked, f.before),
            Some(_) => ", nothing to take out".to_string(),
            None => String::new(),
        },
        started.elapsed().as_secs_f64()
    );
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        std::thread::spawn(move || {
            let _ = handle(server, stream);
        });
    }
    0
}

/// Whether a request may be answered: addressed to this computer by its own name (not a site's name that a DNS
/// trick pointed here), and from no web page or from Chat Computer's own; and the page's origin, for CORS.
fn allowed<'a>(server: &Server, req: &'a http::Request) -> Result<Option<&'a str>, &'static str> {
    let host = req.header("host").unwrap_or("");
    let port = server.port;
    let ok_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")];
    if !ok_hosts.iter().any(|h| h == host) {
        return Err("only this computer's own address is answered");
    }
    match req.header("origin") {
        None => Ok(None),
        Some(o) if server.origins.iter().any(|a| a == o) => Ok(Some(o)),
        Some(_) => Err("this page isn't one of Chat Computer's"),
    }
}

fn handle(server: &Server, mut stream: TcpStream) -> std::io::Result<()> {
    let req = match http::read(&stream) {
        Ok(r) => r,
        Err(e) => return http::respond(&mut stream, 400, "application/json", api::error(&e).as_bytes(), None),
    };
    let cors = match allowed(server, &req) {
        Ok(c) => c,
        Err(e) => return http::respond(&mut stream, 403, "application/json", api::error(e).as_bytes(), None),
    };
    let path = req.path.split('?').next().unwrap_or("");
    match (req.method.as_str(), path) {
        ("OPTIONS", _) => match cors {
            Some(o) => http::preflight(&mut stream, o),
            None => http::respond(&mut stream, 204, "text/plain", b"", None),
        },
        ("GET", "/v1/models") => {
            let body = format!(
                "{{\"object\":\"list\",\"data\":[{{\"id\":{},\"object\":\"model\",\"owned_by\":\"this computer\"}}]}}",
                quoted(&server.name)
            );
            http::respond(&mut stream, 200, "application/json", body.as_bytes(), cors)
        }
        ("GET", "/health") => {
            // (Uncensored: whether its refusals were looked for, and then how it measured.)
            let uncensored = match &server.uncensored {
                Some(f) => format!(
                    "{{\"taken_out\":{},\"checked\":{},\"refused_before\":{},\"refused_after\":{}}}",
                    f.ablation().is_some(), f.checked, f.before, f.after
                ),
                None => "null".to_string(),
            };
            let body = format!(
                "{{\"ok\":true,\"model\":{},\"style\":{},\"uncensored\":{uncensored}}}",
                quoted(&server.name), quoted(&server.style_name)
            );
            http::respond(&mut stream, 200, "application/json", body.as_bytes(), cors)
        }
        ("POST", "/v1/chat/completions") => complete(server, &mut stream, &req, cors),
        (_, "/v1/models" | "/health" | "/v1/chat/completions") => {
            http::respond(&mut stream, 405, "application/json", api::error("not that method").as_bytes(), cors)
        }
        _ => http::respond(&mut stream, 404, "application/json", api::error("nothing here").as_bytes(), cors),
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::new();
    model::json::quote(s, &mut out);
    out
}

fn complete(server: &Server, stream: &mut TcpStream, req: &http::Request, cors: Option<&str>) -> std::io::Result<()> {
    let asked = match api::parse(&req.body) {
        Ok(a) => a,
        Err(e) => return http::respond(stream, 400, "application/json", api::error(&e).as_bytes(), cors),
    };
    let id = format!("chatcmpl-{}", server.ids.fetch_add(1, Ordering::Relaxed));
    let mut engine = server.engine.lock().unwrap_or_else(|p| p.into_inner());
    let engine = &mut *engine;
    let think = asked.options.think.unwrap_or(engine.session.style.thinks);
    // Earlier answers keep their thinking unless asked not to: the conversation then only grows, and what was read
    // is read once (see session.rs).
    let mut options = asked.options;
    options.keep_thinking = Some(options.keep_thinking.unwrap_or(true));
    let r = Request {
        tools: &asked.tools,
        messages: &asked.messages,
        options,
        sampling: asked.sampling(Sampling::recommended(&server.style_name, think)),
        max_tokens: asked.max_tokens,
        seed: asked.seed,
    };
    let pool: &dyn Pool = &engine.pool;
    let call_id = |k: usize| format!("call_{}_{k}", &id[9..]);
    if !asked.stream {
        let result = engine.session.ask(&r, pool, &mut |_, _| true);
        let sent = match result {
            Ok((a, st)) => {
                let calls: Vec<_> = a.calls.iter().enumerate().map(|(k, c)| api::call_json(c, k, &call_id(k), &asked.tools)).collect();
                let finish = finish_reason(st.ended, !calls.is_empty());
                let body = api::whole(&id, &server.name, &a, calls, finish, api::usage(st.prompt, st.read, st.generated));
                http::respond(stream, 200, "application/json", body.as_bytes(), cors)
            }
            Err(e) => http::respond(stream, 400, "application/json", api::error(&format!("{e:?}")).as_bytes(), cors),
        };
        save_anchor(server, engine);
        return sent;
    }
    let mut events = http::Events::start(stream, cors)?;
    let mut alive = events.send(&api::chunk(&id, &server.name, api::delta("role", "assistant"), None, None)).is_ok();
    let result = engine.session.ask(&r, pool, &mut |said, answer| {
        if !alive {
            return false;
        }
        let d = match said {
            model::chat::Said::Thinking(t) => Some(api::delta("reasoning_content", t)),
            model::chat::Said::Text(t) => Some(api::delta("content", t)),
            model::chat::Said::Call(k) => Some(api::calls_delta(vec![api::call_json(&answer.calls[*k], *k, &call_id(*k), &asked.tools)])),
            _ => None,
        };
        if let Some(d) = d {
            // A page that went away (the person stopped it): the answer stops too.
            alive = events.send(&api::chunk(&id, &server.name, d, None, None)).is_ok();
        }
        alive
    });
    match result {
        Ok((a, st)) => {
            let finish = finish_reason(st.ended, !a.calls.is_empty());
            let usage = api::usage(st.prompt, st.read, st.generated);
            let _ = events.send(&api::chunk(&id, &server.name, api::delta("content", ""), Some(finish), Some(usage)));
        }
        Err(e) => {
            let _ = events.send(&api::error(&format!("{e:?}")));
        }
    }
    let done = events.send("[DONE]");
    save_anchor(server, engine);
    done
}

fn hash_tokens(t: &[u32]) -> u64 {
    let bytes: Vec<u8> = t.iter().flat_map(|x| x.to_le_bytes()).collect();
    model::file::fnv(0xcbf2_9ce4_8422_2325, &bytes)
}

const STATE_MAGIC: &[u8; 8] = b"CCSTATE1";

/// The anchor kept on disk for this model file, if there is one: its tokens, and the state after them.
fn load_anchor(dir: &std::path::Path, fingerprint: u64, model: &Model) -> Option<(Vec<u32>, model::State)> {
    let prefix = format!("{fingerprint:016x}-");
    let newest = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).filter(|e| {
        let n = e.file_name().to_string_lossy().into_owned();
        n.starts_with(&prefix) && n.ends_with(".ccstate")
    }).max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let bytes = std::fs::read(newest.path()).ok()?;
    if bytes.len() < 20 || &bytes[..8] != STATE_MAGIC || u64::from_le_bytes(bytes[8..16].try_into().ok()?) != fingerprint {
        return None;
    }
    let n = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
    let tokens: Vec<u32> = bytes.get(20..20 + 4 * n)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let state = model.import(&bytes[20 + 4 * n..])?;
    (state.pos == tokens.len()).then_some((tokens, state))
}

/// The anchor on disk (when it's new): the next start picks it up instead of reading the system prompt again.
fn save_anchor(server: &Server, engine: &mut Engine) {
    let Some(dir) = engine.cache.clone() else { return };
    let Some((tokens, state, snap)) = engine.session.anchor() else { return };
    let h = hash_tokens(tokens);
    if h == engine.kept {
        return;
    }
    let mut out = Vec::new();
    out.extend_from_slice(STATE_MAGIC);
    out.extend_from_slice(&server.fingerprint.to_le_bytes());
    out.extend_from_slice(&(tokens.len() as u32).to_le_bytes());
    for t in tokens {
        out.extend_from_slice(&t.to_le_bytes());
    }
    out.extend_from_slice(&state.export(snap));
    let name = format!("{:016x}-{h:016x}.ccstate", server.fingerprint);
    let tmp = dir.join(format!("{name}.part"));
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&tmp, &out).is_err() || std::fs::rename(&tmp, dir.join(&name)).is_err() {
        eprintln!("couldn't keep the system prompt's reading in {}", dir.display());
        return;
    }
    engine.kept = h;
    // (The ones kept before, for this model, are of no more use.)
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let prefix = format!("{:016x}-", server.fingerprint);
        for e in entries.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with(&prefix) && n != name {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn finish_reason(ended: Ended, calls: bool) -> &'static str {
    match ended {
        Ended::Length => "length",
        _ if calls => "tool_calls",
        _ => "stop",
    }
}
