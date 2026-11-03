//! Your ChatGPT plan, in Chat Computer: OpenAI's "Sign in with ChatGPT" with its plan usage, which OpenAI offers
//! open-source apps that run on the person's own computer (developers.openai.com/siwc; T3 Code, pingdotgg/t3code,
//! signs in the same way). The person signs in on OpenAI's own page, in their own browser, and allows the app to use
//! their plan; OpenAI's page then comes back to this app on this computer (http://127.0.0.1, at a port of its own,
//! once), and the app trades what it brings for tokens over Chat Computer's own HTTPS (os/tls, by way of the
//! engine's fetch.rs, built in here too). The tokens stay in the app's own folder, for this user alone, and the page
//! never sees them: it asks the app (chatgpt_ask), and the app asks OpenAI's Responses API with them and hands the
//! answer back as it streams in. Signed out, they're given back to OpenAI (revoked) and forgotten; what OpenAI
//! registered for this app on this computer (its client id) is kept, so signing in again needs no new registration.

// (The engine's own fetcher, built in here as it is: the app sends requests with it, and doesn't download files.)
#[path = "../../../../os/model/src/bin/chat-computer-model/fetch.rs"]
#[allow(dead_code)]
mod fetch;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// OpenAI's sign-in, and where its tokens are good for.
const ISSUER: &str = "https://auth.openai.com";
const API: &str = "https://api.openai.com/v1";
/// What's asked for: who you are, staying signed in, and using your ChatGPT plan for the app's questions.
const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN: &str = "chatgpt.tokens.use.direct";
/// The app's name on OpenAI's page, the first time.
const AGENT: &str = "Chat Computer";
/// How long OpenAI's page has to come back.
const WAIT: Duration = Duration::from_secs(600);

/// What's kept, in the app's folder (chatgpt.json).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Saved {
    /// This app on this computer, to OpenAI: chosen once, opaque (urn:uuid).
    #[serde(default)]
    pub host_id: String,
    /// What OpenAI registered the app as here (oaiapp_...), at the first sign-in.
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// When the access token runs out (seconds since 1970).
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// What the page is told: signed in or not, as whom. (Never the tokens.)
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub signed_in: bool,
    pub email: Option<String>,
}

fn file(dir: &Path) -> PathBuf {
    dir.join("chatgpt.json")
}

fn load(dir: &Path) -> Saved {
    std::fs::read(file(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Written whole (to a new file, then moved over the old), readable by this user alone.
fn save(dir: &Path, s: &Saved) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("the app's folder: {e}"))?;
    let tmp = dir.join("chatgpt.json.new");
    let body = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("keeping the sign-in: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        f.write_all(&body).map_err(|e| format!("keeping the sign-in: {e}"))?;
    }
    std::fs::rename(&tmp, file(dir)).map_err(|e| format!("keeping the sign-in: {e}"))
}

pub fn status(dir: &Path) -> Status {
    let s = load(dir);
    Status { signed_in: signed_in(&s), email: if signed_in(&s) { s.email } else { None } }
}

fn signed_in(s: &Saved) -> bool {
    s.client_id.is_some() && (s.access_token.is_some() || s.refresh_token.is_some()) && s.scopes.iter().any(|x| x == PLAN)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn random(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the system's random bytes");
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    random(&mut b);
    b
}

pub fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..c.len() + 1 {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// PKCE's challenge for a verifier: its SHA-256, base64url (RFC 7636).
pub fn challenge(verifier: &str) -> String {
    b64url(&crypto::sha256(verifier.as_bytes()))
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push((hex(b[i + 1]).unwrap_or(0) * 16 + hex(b[i + 2]).unwrap_or(0)) as u8);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

fn query(q: &str) -> Vec<(String, String)> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        (decode(k), decode(v))
    }).collect()
}

/// A request over the own HTTPS: its status and body.
fn https(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let mut status = 0;
    let mut out = Vec::new();
    fetch::request(method, url, headers, body, &mut random, &mut |p| {
        match p {
            fetch::Part::Head { status: s, .. } => status = s,
            fetch::Part::Body(b) => {
                if out.len() + b.len() > 8 << 20 {
                    return Err("an answer too long".into());
                }
                out.extend_from_slice(b);
            }
        }
        Ok(true)
    })?;
    Ok((status, out))
}

fn api_error(body: &[u8]) -> String {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let e = &v["error"];
    let text = e["message"].as_str().or(e.as_str()).or(v["error_description"].as_str()).or(v["message"].as_str()).unwrap_or("");
    text.chars().take(200).collect()
}

/// OpenAI's sign-in's own addresses, as it publishes them, all of them its own.
#[derive(Clone, Debug)]
struct Endpoints {
    authorization: String,
    token: String,
    revocation: Option<String>,
}

static ENDPOINTS: Mutex<Option<Endpoints>> = Mutex::new(None);

fn endpoints() -> Result<Endpoints, String> {
    if let Some(e) = ENDPOINTS.lock().map_err(|_| "a lock".to_string())?.clone() {
        return Ok(e);
    }
    let (status, body) = https("GET", &format!("{ISSUER}/.well-known/openid-configuration"), &[], b"")
        .map_err(|e| format!("ChatGPT's sign-in can't be reached ({e})."))?;
    if status != 200 {
        return Err(format!("ChatGPT's sign-in can't be reached (it said {status})."));
    }
    let v: Value = serde_json::from_slice(&body).map_err(|_| "ChatGPT's sign-in said something unreadable.".to_string())?;
    let own = |k: &str| v[k].as_str().filter(|u| u.starts_with(&format!("{ISSUER}/"))).map(str::to_string);
    if v["issuer"].as_str() != Some(ISSUER) {
        return Err("ChatGPT's sign-in couldn't be verified.".into());
    }
    let e = Endpoints {
        authorization: own("authorization_endpoint").ok_or("ChatGPT's sign-in couldn't be verified.")?,
        token: own("token_endpoint").ok_or("ChatGPT's sign-in couldn't be verified.")?,
        revocation: own("revocation_endpoint"),
    };
    *ENDPOINTS.lock().map_err(|_| "a lock".to_string())? = Some(e.clone());
    Ok(e)
}

/// The address of OpenAI's page to sign in at, for `s`, coming back to `redirect`.
pub fn authorize_url(authorization: &str, s: &Saved, redirect: &str, state: &str, nonce: &str, challenge: &str) -> String {
    let mut q: Vec<(&str, &str)> = Vec::new();
    match &s.client_id {
        Some(id) => {
            q.push(("client_id", id));
            if let Some(e) = &s.email {
                q.push(("login_hint", e));
            }
            if !s.scopes.iter().any(|x| x == PLAN) {
                q.push(("prompt", "consent"));
            }
        }
        None => {
            q.push(("client_id", "dynamic_agent_client"));
            q.push(("agent_name_hint", AGENT));
        }
    }
    q.extend([
        ("ext_agent_host_id", s.host_id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", redirect),
        ("scope", SCOPE),
        ("resource", API),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge),
    ]);
    format!("{authorization}?{}", form(&q))
}

/// What OpenAI's page brought back to /auth/callback: the code and the client id, if it's the answer to `state`.
pub fn callback(target: &str, state: &str) -> Result<(String, Option<String>), String> {
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" {
        return Err("not here".into());
    }
    let q = query(q);
    let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if get("state").as_deref() != Some(state) {
        return Err("This sign-in isn't the one Chat Computer started: start it again from Chat Computer.".into());
    }
    if let Some(e) = get("error") {
        return Err(if e == "access_denied" {
            "ChatGPT's sign-in was declined. Sign in again when you're ready.".into()
        } else {
            format!("ChatGPT's sign-in didn't finish ({e}).")
        });
    }
    let code = get("code").filter(|c| !c.is_empty()).ok_or("ChatGPT's sign-in came back without a code.")?;
    Ok((code, get("client_id")))
}

/// What the ID token says, read from it (it came straight from OpenAI's token address, over HTTPS: that's what
/// vouches for it, as OpenID Connect allows): whose it is, and that it's the answer to this sign-in.
pub fn identity(id_token: &str, client_id: &str, nonce: &str) -> Result<(String, Option<String>), String> {
    let payload = id_token.split('.').nth(1).and_then(b64url_decode).ok_or("ChatGPT gave an unreadable identity.")?;
    let v: Value = serde_json::from_slice(&payload).map_err(|_| "ChatGPT gave an unreadable identity.".to_string())?;
    let aud_ok = match &v["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if v["iss"].as_str() != Some(ISSUER) || !aud_ok || v["nonce"].as_str() != Some(nonce) {
        return Err("ChatGPT's sign-in couldn't be verified. Start again.".into());
    }
    let sub = v["sub"].as_str().ok_or("ChatGPT gave an identity without whose it is.")?.to_string();
    Ok((sub, v["email"].as_str().map(str::to_string)))
}

/// Tokens from the token address into `s`.
fn take_tokens(s: &mut Saved, body: &[u8]) -> Result<Value, String> {
    let v: Value = serde_json::from_slice(body).map_err(|_| "ChatGPT's sign-in said something unreadable.".to_string())?;
    let access = v["access_token"].as_str().filter(|t| !t.is_empty()).ok_or("ChatGPT's sign-in gave no token.")?;
    if !v["token_type"].as_str().is_some_and(|t| t.eq_ignore_ascii_case("bearer")) {
        return Err("ChatGPT's sign-in gave a token of a kind Chat Computer doesn't use.".into());
    }
    s.access_token = Some(access.to_string());
    if let Some(r) = v["refresh_token"].as_str().filter(|t| !t.is_empty()) {
        s.refresh_token = Some(r.to_string());
    }
    s.expires_at = now() + v["expires_in"].as_i64().unwrap_or(3600).max(60);
    if let Some(sc) = v["scope"].as_str() {
        s.scopes = sc.split_whitespace().map(str::to_string).collect();
    }
    Ok(v)
}

fn answer_browser(mut stream: &TcpStream, status: &str, words: &str) {
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Chat Computer</title>\
         <body style=\"margin:0;display:grid;place-items:center;height:100vh;background:#191919;color:#F0F0F0;\
         font:18px/1.4 Lora,Georgia,serif\"><p style=\"max-width:30em;padding:1em\">{words}</p>"
    );
    let _ = stream.write_all(
        format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
                 Connection: close\r\n\r\n{page}", page.len()).as_bytes(),
    );
}

/// Signs in: OpenAI's page, opened in the person's own browser by `open`, and back here. Blocks until it's done
/// (or WAIT passes). What's kept: the tokens and who signed in.
pub fn sign_in(dir: &Path, open: &dyn Fn(&str) -> Result<(), String>) -> Result<Status, String> {
    let mut s = load(dir);
    if s.host_id.is_empty() {
        let mut u = random_bytes(16);
        u[6] = u[6] & 0x0f | 0x40;
        u[8] = u[8] & 0x3f | 0x80;
        let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
        s.host_id = format!("urn:uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]);
        save(dir, &s)?;
    }
    let ends = endpoints()?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("The sign-in can't come back to Chat Computer ({e})."))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let verifier = b64url(&random_bytes(64));
    let (state, nonce) = (b64url(&random_bytes(32)), b64url(&random_bytes(32)));
    open(&authorize_url(&ends.authorization, &s, &redirect, &state, &nonce, &challenge(&verifier)))?;
    // OpenAI's page, back: one request to /auth/callback (anything else asked of this port is turned away).
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let (code, given_id, stream) = loop {
        if started.elapsed() > WAIT {
            return Err("ChatGPT's sign-in took too long. Start it again.".into());
        }
        let mut stream = match listener.accept() {
            Ok((st, _)) => st,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut head = Vec::new();
        let mut b = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16384 {
            match stream.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => head.extend_from_slice(&b[..n]),
            }
        }
        let line = String::from_utf8_lossy(&head).lines().next().unwrap_or("").to_string();
        let mut parts = line.split_whitespace();
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if method != "GET" || !target.starts_with("/auth/callback") {
            answer_browser(&stream, "404 Not Found", "Nothing here.");
            continue;
        }
        match callback(target, &state) {
            Ok((code, id)) => break (code, id, stream),
            Err(e) => {
                answer_browser(&stream, "400 Bad Request", &e);
                return Err(e);
            }
        }
    };
    let finish = |ok: bool, words: &str| answer_browser(&stream, if ok { "200 OK" } else { "400 Bad Request" }, words);
    let client_id = match s.client_id.clone().or(given_id) {
        Some(id) if id.starts_with("oaiapp_") => id,
        _ => {
            let e = "ChatGPT's sign-in didn't register Chat Computer. Start it again.";
            finish(false, e);
            return Err(e.into());
        }
    };
    let body = form(&[
        ("grant_type", "authorization_code"), ("client_id", &client_id), ("code", &code), ("code_verifier", &verifier),
        ("redirect_uri", &redirect), ("resource", API),
    ]);
    let done = (|| -> Result<Status, String> {
        let (st, answer) = https("POST", &ends.token, &[("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")],
                                 body.as_bytes()).map_err(|e| format!("ChatGPT's sign-in can't be reached ({e})."))?;
        if st != 200 {
            return Err(format!("ChatGPT's sign-in didn't finish: {}", api_error(&answer)));
        }
        let mut next = s.clone();
        next.client_id = Some(client_id.clone());
        let v = take_tokens(&mut next, &answer)?;
        let (_, email) = identity(v["id_token"].as_str().ok_or("ChatGPT gave no identity.")?, &client_id, &nonce)?;
        next.email = email.or(next.email);
        if !next.scopes.iter().any(|x| x == PLAN) {
            // (Registered, but not let use the plan: kept registered, asked again next time.)
            let kept = Saved { client_id: Some(client_id.clone()), email: next.email.clone(), ..s.clone() };
            save(dir, &kept)?;
            return Err("Using your ChatGPT plan wasn't allowed. Sign in again and allow it to use Chat Computer.".into());
        }
        save(dir, &next)?;
        Ok(Status { signed_in: true, email: next.email })
    })();
    match &done {
        Ok(_) => finish(true, "Chat Computer is signed in with ChatGPT. You can close this tab and go back to Chat Computer."),
        Err(e) => finish(false, e),
    }
    done
}

/// The tokens, renewed when they've run out (or `force`d, after OpenAI turned the access token away).
fn fresh(dir: &Path, force: bool) -> Result<Saved, String> {
    let mut s = load(dir);
    if !signed_in(&s) {
        return Err("Chat Computer isn't signed in with ChatGPT.".into());
    }
    if !force && s.access_token.is_some() && s.expires_at - 60 > now() {
        return Ok(s);
    }
    let (Some(refresh), Some(client_id)) = (s.refresh_token.clone(), s.client_id.clone()) else {
        return Err("Your ChatGPT sign-in has run out: sign in again.".into());
    };
    let ends = endpoints()?;
    let body = form(&[("grant_type", "refresh_token"), ("client_id", &client_id), ("refresh_token", &refresh), ("resource", API)]);
    let (st, answer) = https("POST", &ends.token, &[("Content-Type", "application/x-www-form-urlencoded"), ("Accept", "application/json")],
                             body.as_bytes()).map_err(|e| format!("ChatGPT can't be reached ({e})."))?;
    if st == 400 || st == 401 {
        s.access_token = None;
        s.refresh_token = None;
        save(dir, &s)?;
        return Err("Your ChatGPT sign-in has run out: sign in again.".into());
    }
    if st != 200 {
        return Err(format!("ChatGPT couldn't renew the sign-in: {}", api_error(&answer)));
    }
    take_tokens(&mut s, &answer)?;
    save(dir, &s)?;
    Ok(s)
}

/// Asks OpenAI's API on your plan: `path` is "responses" (an answer, `body` its request, streamed) or "models" (the
/// models your plan has). What comes back is handed to `on` as it comes, as text; the status is returned.
pub fn ask(dir: &Path, path: &str, body: Option<&str>, on: &mut dyn FnMut(&str)) -> Result<u16, String> {
    let (method, url) = match path {
        "responses" => ("POST", format!("{API}/responses")),
        "models" => ("GET", format!("{API}/models")),
        _ => return Err("Not something to ask ChatGPT.".into()),
    };
    let mut force = false;
    loop {
        let s = fresh(dir, force)?;
        let auth = format!("Bearer {}", s.access_token.as_deref().unwrap_or(""));
        let mut headers = vec![("Authorization", auth.as_str()), ("Accept", if method == "POST" { "text/event-stream" } else { "application/json" })];
        if method == "POST" {
            headers.push(("Content-Type", "application/json"));
        }
        let mut status = 0;
        let mut sent = false;
        let mut carry: Vec<u8> = Vec::new();
        fetch::request(method, &url, &headers, body.unwrap_or("").as_bytes(), &mut random, &mut |p| {
            match p {
                fetch::Part::Head { status: st, .. } => {
                    status = st;
                    // (Turned away: renewed once, and asked again, before anything's handed over.)
                    if st == 401 && !force {
                        return Ok(false);
                    }
                }
                fetch::Part::Body(b) => {
                    carry.extend_from_slice(b);
                    // (Only whole characters: a piece can end inside one.)
                    let upto = match std::str::from_utf8(&carry) {
                        Ok(_) => carry.len(),
                        Err(e) => e.valid_up_to(),
                    };
                    if upto > 0 {
                        on(std::str::from_utf8(&carry[..upto]).unwrap_or(""));
                        sent = true;
                        carry.drain(..upto);
                    }
                }
            }
            Ok(true)
        }).map_err(|e| format!("ChatGPT can't be reached ({e})."))?;
        if status == 401 && !force && !sent {
            force = true;
            continue;
        }
        return Ok(status);
    }
}

/// Signs out: the tokens given back to OpenAI (where it can be reached) and forgotten here.
pub fn sign_out(dir: &Path) -> Status {
    let mut s = load(dir);
    if let (Some(r), Some(id)) = (s.refresh_token.clone(), s.client_id.clone())
        && let Ok(ends) = endpoints()
        && let Some(revoke) = ends.revocation
    {
        let body = form(&[("token", &r), ("token_type_hint", "refresh_token"), ("client_id", &id)]);
        let _ = https("POST", &revoke, &[("Content-Type", "application/x-www-form-urlencoded")], body.as_bytes());
    }
    s.access_token = None;
    s.refresh_token = None;
    s.expires_at = 0;
    s.scopes.clear();
    let _ = save(dir, &s);
    Status { signed_in: false, email: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_as_rfc_7636_has_it() {
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(b64url(b"Man"), "TWFu");
        assert_eq!(b64url(b"Ma"), "TWE");
        assert_eq!(b64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(b64url_decode("-_8").unwrap(), vec![0xfb, 0xff]);
    }

    #[test]
    fn asks_for_the_plan_the_first_time_and_after() {
        let first = Saved { host_id: "urn:uuid:1".into(), ..Saved::default() };
        let u = authorize_url("https://auth.openai.com/oauth/authorize", &first, "http://127.0.0.1:5555/auth/callback", "st", "no", "ch");
        let (base, q) = u.split_once('?').unwrap();
        assert_eq!(base, "https://auth.openai.com/oauth/authorize");
        let q = query(q);
        let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("client_id"), Some("dynamic_agent_client"));
        assert_eq!(get("agent_name_hint"), Some("Chat Computer"));
        assert_eq!(get("redirect_uri"), Some("http://127.0.0.1:5555/auth/callback"));
        assert_eq!(get("scope"), Some(SCOPE));
        assert_eq!(get("resource"), Some("https://api.openai.com/v1"));
        assert_eq!((get("code_challenge_method"), get("code_challenge"), get("state"), get("nonce")), (Some("S256"), Some("ch"), Some("st"), Some("no")));
        assert_eq!(get("ext_agent_host_id"), Some("urn:uuid:1"));
        // Registered, with the plan: the same client, the person's own address as a hint, nothing asked again.
        let later = Saved { client_id: Some("oaiapp_x".into()), email: Some("a@b.c".into()), scopes: vec![PLAN.into()], ..first.clone() };
        let q = query(authorize_url("https://x/a", &later, "r", "s", "n", "c").split_once('?').unwrap().1);
        let has = |k: &str| q.iter().any(|(n, _)| n == k);
        assert!(q.contains(&("client_id".into(), "oaiapp_x".into())) && q.contains(&("login_hint".into(), "a@b.c".into())));
        assert!(!has("agent_name_hint") && !has("prompt"));
        // Registered, but not let use the plan: asked again.
        let refused = Saved { scopes: vec![], ..later };
        assert!(query(authorize_url("https://x/a", &refused, "r", "s", "n", "c").split_once('?').unwrap().1)
            .contains(&("prompt".into(), "consent".into())));
    }

    #[test]
    fn reads_what_comes_back() {
        assert_eq!(callback("/auth/callback?code=abc%2Fd&state=s1&client_id=oaiapp_9", "s1").unwrap(),
                   ("abc/d".to_string(), Some("oaiapp_9".to_string())));
        assert!(callback("/auth/callback?code=abc&state=other", "s1").unwrap_err().contains("isn't the one"));
        assert!(callback("/auth/callback?error=access_denied&state=s1", "s1").unwrap_err().contains("declined"));
        assert!(callback("/elsewhere?code=abc&state=s1", "s1").is_err());
        assert!(callback("/auth/callback?state=s1", "s1").unwrap_err().contains("without a code"));
    }

    #[test]
    fn checks_the_identity_is_this_sign_in_s() {
        let token = |claims: &str| format!("e30.{}.sig", b64url(claims.as_bytes()));
        let good = token(r#"{"iss":"https://auth.openai.com","aud":"oaiapp_1","nonce":"n1","sub":"user-1","email":"a@b.c"}"#);
        assert_eq!(identity(&good, "oaiapp_1", "n1").unwrap(), ("user-1".to_string(), Some("a@b.c".to_string())));
        assert!(identity(&good, "oaiapp_2", "n1").is_err());
        assert!(identity(&good, "oaiapp_1", "n2").is_err());
        let elsewhere = token(r#"{"iss":"https://elsewhere","aud":["oaiapp_1"],"nonce":"n1","sub":"u"}"#);
        assert!(identity(&elsewhere, "oaiapp_1", "n1").is_err());
        assert!(identity("not-a-token", "oaiapp_1", "n1").is_err());
    }

    #[test]
    fn keeps_tokens_for_this_user_and_forgets_them() {
        let dir = std::env::temp_dir().join(format!("chatgpt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(status(&dir), Status { signed_in: false, email: None });
        let mut s = Saved { host_id: "urn:uuid:2".into(), client_id: Some("oaiapp_1".into()), email: Some("a@b.c".into()), ..Saved::default() };
        let v = take_tokens(&mut s, br#"{"access_token":"at","refresh_token":"rt","token_type":"Bearer","expires_in":3600,
                                          "scope":"openid chatgpt.tokens.use.direct"}"#).unwrap();
        assert_eq!(v["access_token"], "at");
        assert!(take_tokens(&mut s.clone(), br#"{"access_token":"x","token_type":"mac","expires_in":1}"#).is_err());
        save(&dir, &s).unwrap();
        assert_eq!(status(&dir), Status { signed_in: true, email: Some("a@b.c".into()) });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(file(&dir)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Signed out (OpenAI not reached here: forgotten all the same), its registration kept.
        let mut out = load(&dir);
        out.access_token = None;
        out.refresh_token = None;
        out.scopes.clear();
        save(&dir, &out).unwrap();
        assert_eq!(status(&dir).signed_in, false);
        assert_eq!(load(&dir).client_id.as_deref(), Some("oaiapp_1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
