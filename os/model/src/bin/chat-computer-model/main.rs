//! Chat Computer's model, as a program: it serves the model to this computer's own programs (Chat Computer, its
//! desktop app's page, its Chrome extension) over HTTP on 127.0.0.1, as OpenAI's chat completions API, and makes
//! model files from the published ones.
//!
//!     chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--allow-origin ORIGIN]...
//!     chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]
//!
//! Only this computer can reach it (it listens on 127.0.0.1), and of web pages only Chat Computer's own (its site
//! and its app, and any origin given with --allow-origin, such as the Chrome extension's): any other page a browser
//! shows is refused, so no site can use the model behind the person's back. The model only writes: it runs nothing
//! and opens nothing (what Chat Computer does with its answers is Chat Computer's, with its own permissions).

mod api;
mod convert;
mod http;
mod map;

use model::chat::Style;
use model::net::Pool;
use model::sample::Sampling;
use model::session::{Ended, Request, Session};
use model::threads::Threads;
use model::{File, Model};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Pages that may use the model: Chat Computer's site, and its desktop app's own page (Tauri's origins).
const ORIGINS: [&str; 5] = ["https://chatcomputer.ai", "https://www.chatcomputer.ai", "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

struct Engine {
    session: Session<'static>,
    pool: Threads,
}

struct Server {
    engine: Mutex<Engine>,
    name: String,
    style_name: String,
    origins: Vec<String>,
    port: u16,
    ids: AtomicU64,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("chat-computer-model serve MODEL.ccm [--port 8765] [--threads N] [--allow-origin ORIGIN]...\n\
                   chat-computer-model convert MODEL_FOLDER OUT.ccm [--bits 8|4|16]");
        std::process::exit(2);
    }
    let command = args.remove(0);
    let code = match command.as_str() {
        "serve" => serve(args),
        "convert" => convert::main_with(args),
        other => {
            eprintln!("no command \"{other}\" (serve or convert)");
            2
        }
    };
    std::process::exit(code);
}

fn serve(args: Vec<String>) -> i32 {
    let (mut path, mut port, mut threads, mut extra) = (None, 8765u16, None, Vec::new());
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--port" => port = value.parse().unwrap_or(port),
            "--threads" => threads = value.parse().ok(),
            "--allow-origin" => extra.push(value),
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
    let bytes = match map::map(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("serve: {e}");
            return 1;
        }
    };
    let file: &'static File = Box::leak(Box::new(match File::open(bytes) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("serve: {path} isn't a model file ({e:?})");
            return 1;
        }
    }));
    let model: &'static Model<'static> = match Model::load(file) {
        Ok(m) => Box::leak(Box::new(m)),
        Err(e) => {
            eprintln!("serve: the model in {path} can't be run ({e:?})");
            return 1;
        }
    };
    let config = file.config().ok();
    let style_name = config.as_ref().and_then(|c| c.str("chat.style")).unwrap_or("qwen3.5").to_string();
    let Some(style) = Style::named(&style_name) else {
        eprintln!("serve: the model's chat style \"{style_name}\" isn't one this program knows");
        return 1;
    };
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
    let server: &'static Server = Box::leak(Box::new(Server {
        engine: Mutex::new(Engine { session: Session::new(model, style), pool }),
        name,
        style_name,
        origins,
        port,
        ids: AtomicU64::new(1),
    }));
    eprintln!(
        "serving {} ({} layers, {} wide, {:?}) on http://127.0.0.1:{port}, ready in {:.1} s",
        server.name, model.settings.layers, model.settings.hidden, model::cpu::level(), started.elapsed().as_secs_f64()
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
            let body = format!("{{\"ok\":true,\"model\":{},\"style\":{}}}", quoted(&server.name), quoted(&server.style_name));
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
        return match result {
            Ok((a, st)) => {
                let calls: Vec<_> = a.calls.iter().enumerate().map(|(k, c)| api::call_json(c, k, &call_id(k), &asked.tools)).collect();
                let finish = finish_reason(st.ended, !calls.is_empty());
                let body = api::whole(&id, &server.name, &a, calls, finish, api::usage(st.prompt, st.read, st.generated));
                http::respond(stream, 200, "application/json", body.as_bytes(), cors)
            }
            Err(e) => http::respond(stream, 400, "application/json", api::error(&format!("{e:?}")).as_bytes(), cors),
        };
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
    events.send("[DONE]")
}

fn finish_reason(ended: Ended, calls: bool) -> &'static str {
    match ended {
        Ended::Length => "length",
        _ if calls => "tool_calls",
        _ => "stop",
    }
}
