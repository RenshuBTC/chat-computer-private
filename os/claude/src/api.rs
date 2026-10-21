//! Claude's Messages API (POST /v1/messages), streamed: the request, and the answer as it comes, from the
//! server-sent events (event: and data: lines, a blank line after each). The answer's words come out as they
//! arrive; the whole answer (its content blocks: thinking, words and their citations, tool calls, and the tools
//! Anthropic runs itself, like web search, with their results) is kept, to go back in the conversation, and when
//! Claude calls tools, to go back with their results. The API asks for thinking blocks, and the web tools' encrypted
//! results, to come back complete and unmodified, so they are kept exactly as they came.

use crate::http::{self, Response};
use crate::json::{self, Json};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const HOST: &str = "api.anthropic.com";
/// The model and effort a Chat Computer uses (chatcomputer/config.py): Opus 5.5 at its highest effort.
pub const MODEL: &str = "claude-opus-5-5";
pub const EFFORT: &str = "max";
/// Room for its thinking and its answer.
pub const MAX_TOKENS: u32 = 64000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    User,
    Assistant,
}

/// What a turn of the conversation says.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Words.
    Text(String),
    /// Content blocks, as JSON objects: an answer as it came (thinking, words, tool calls), or tools' results.
    Blocks(Vec<Json>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub role: Role,
    pub content: Content,
}

impl Turn {
    pub fn text(role: Role, text: &str) -> Turn {
        Turn { role, content: Content::Text(String::from(text)) }
    }
}

/// A tool Claude may call: its name, what it does, and its input's JSON schema (as JSON text).
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: &'static str,
}

/// A tool's result, to go back to Claude.
pub fn tool_result(id: &str, content: &str, is_error: bool) -> Json {
    Json::Obj(alloc::vec![
        (String::from("type"), Json::Str(String::from("tool_result"))),
        (String::from("tool_use_id"), Json::Str(String::from(id))),
        (String::from("content"), Json::Str(String::from(content))),
        (String::from("is_error"), Json::Bool(is_error)),
    ])
}

/// A tool's result with more than words: its words, then pictures or a document (`image`, `pdf`) for Claude to see.
pub fn tool_result_with(id: &str, content: &str, is_error: bool, more: Vec<Json>) -> Json {
    let mut blocks = alloc::vec![object(&[("type", Json::Str(String::from("text"))), ("text", Json::Str(String::from(content)))])];
    blocks.extend(more);
    object(&[
        ("type", Json::Str(String::from("tool_result"))),
        ("tool_use_id", Json::Str(String::from(id))),
        ("content", Json::Arr(blocks)),
        ("is_error", Json::Bool(is_error)),
    ])
}

/// A picture for Claude to see, as the API takes one in a tool's result: JPEG, PNG, GIF or WebP (`media_type`, such
/// as "image/jpeg"), its bytes in base64 (at most 10 MB of it).
pub fn image(media_type: &str, data: &[u8]) -> Json {
    attachment("image", media_type, data)
}

/// A PDF for Claude to read, its pages' words and pictures both (at most 32 MB of request, with the rest).
pub fn pdf(data: &[u8]) -> Json {
    attachment("document", "application/pdf", data)
}

fn attachment(kind: &str, media_type: &str, data: &[u8]) -> Json {
    let source = object(&[
        ("type", Json::Str(String::from("base64"))),
        ("media_type", Json::Str(String::from(media_type))),
        ("data", Json::Str(base64(data))),
    ]);
    object(&[("type", Json::Str(String::from(kind))), ("source", source)])
}

/// The pictures and documents in a turn's tool results: each one's size, as base64, in order.
pub fn attachments(turn: &Turn) -> Vec<usize> {
    let Content::Blocks(blocks) = &turn.content else { return Vec::new() };
    let mut sizes = Vec::new();
    for b in blocks {
        if let Some(Json::Arr(inner)) = b.get("content") {
            sizes.extend(inner.iter().filter_map(|i| i.path(&["source", "data"]).and_then(|d| d.str())).map(|d| d.len()));
        }
    }
    sizes
}

/// Takes the pictures and documents out of a turn's tool results (the first `keep` of them stay), each replaced by
/// a line saying it was there. How many were taken out.
pub fn unattach(turn: &mut Turn, keep: usize) -> usize {
    let Content::Blocks(blocks) = &mut turn.content else { return 0 };
    let (mut seen, mut taken) = (0, 0);
    for b in blocks.iter_mut() {
        if let Some(Json::Arr(inner)) = b.get_mut("content") {
            for i in inner.iter_mut() {
                if i.path(&["source", "data"]).is_none() {
                    continue;
                }
                seen += 1;
                if seen > keep {
                    let what = if i.get("type").and_then(|t| t.str()) == Some("image") { "picture" } else { "document" };
                    let note = alloc::format!("[The {what} read here isn't sent any more: read_file shows it again.]");
                    *i = object(&[("type", Json::Str(String::from("text"))), ("text", Json::Str(note))]);
                    taken += 1;
                }
            }
        }
    }
    taken
}

fn object(fields: &[(&str, Json)]) -> Json {
    Json::Obj(fields.iter().map(|(k, v)| (String::from(*k), v.clone())).collect())
}

/// Base64 (RFC 4648, with padding).
pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(ABC[(n >> 18) as usize & 63]);
        out.push(ABC[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 { ABC[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if chunk.len() > 2 { ABC[n as usize & 63] } else { b'=' });
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Tools Anthropic runs itself (server tools), as their definitions' JSON: Claude searches the web, and reads web
/// pages, without the computer doing either.
pub const WEB_SEARCH: &str = r#"{"type":"web_search_20250305","name":"web_search","max_uses":5}"#;
pub const WEB_FETCH: &str = r#"{"type":"web_fetch_20250910","name":"web_fetch","max_uses":5,"max_content_tokens":25000}"#;

/// The whole request, ready to send: the conversation so far, newest last, the tools this computer runs, and the
/// server tools (`WEB_SEARCH`...). With the API's automatic prompt caching: it caches the request up to its last
/// block, so the next request (the conversation one answer longer, or a tool's result added) reads all that from the
/// cache, for a twentieth of the price and faster, as long as it comes within 5 minutes and starts the same way (the
/// same tools and system prompt).
pub fn request(host: &str, key: &str, system: &str, conversation: &[Turn], tools: &[Tool], server_tools: &[&str]) -> Vec<u8> {
    let mut body = format!(
        "{{\"model\":{},\"max_tokens\":{},\"stream\":true,\"output_config\":{{\"effort\":{}}},\
         \"cache_control\":{{\"type\":\"ephemeral\"}},\"system\":{}",
        json::quote(MODEL),
        MAX_TOKENS,
        json::quote(EFFORT),
        json::quote(system)
    );
    if !tools.is_empty() || !server_tools.is_empty() {
        let mut all: Vec<String> = tools
            .iter()
            .map(|t| {
                format!(
                    "{{\"name\":{},\"description\":{},\"input_schema\":{}}}",
                    json::quote(t.name),
                    json::quote(t.description),
                    t.input_schema
                )
            })
            .collect();
        all.extend(server_tools.iter().map(|t| String::from(*t)));
        body.push_str(",\"tools\":[");
        body.push_str(&all.join(","));
        body.push(']');
    }
    body.push_str(",\"messages\":");
    let room: usize = conversation.iter().map(turn_size).sum::<usize>() + 16;
    body.reserve(room);
    write_messages(&mut body, conversation);
    body.push('}');
    http::request(
        "POST",
        host,
        "/v1/messages",
        &[
            ("x-api-key", key),
            ("anthropic-version", "2023-06-01"),
            ("content-type", "application/json"),
            ("accept", "text/event-stream"),
            ("user-agent", "ChatComputer-own-kernel/0.1"),
        ],
        body.as_bytes(),
    )
}

/// The conversation as the API's "messages" array (JSON text): what goes in a request, and what's kept across restarts.
pub fn messages(conversation: &[Turn]) -> String {
    let mut out = String::with_capacity(conversation.iter().map(turn_size).sum::<usize>() + 2);
    write_messages(&mut out, conversation);
    out
}

/// About how long a turn's JSON is.
pub fn turn_size(turn: &Turn) -> usize {
    32 + match &turn.content {
        Content::Text(t) => t.len() + 2,
        Content::Blocks(blocks) => blocks.iter().map(|b| b.size() + 1).sum(),
    }
}

/// The "messages" array, at the end of `out` (written in place: a picture's base64 isn't copied more than once).
fn write_messages(out: &mut String, conversation: &[Turn]) {
    out.push('[');
    for (i, turn) in conversation.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let role = match turn.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        out.push_str("{\"role\":\"");
        out.push_str(role);
        out.push_str("\",\"content\":");
        match &turn.content {
            Content::Text(text) => json::quote_into(out, text),
            Content::Blocks(blocks) => {
                out.push('[');
                for (k, b) in blocks.iter().enumerate() {
                    if k > 0 {
                        out.push(',');
                    }
                    b.write_to(out);
                }
                out.push(']');
            }
        }
        out.push('}');
    }
    out.push(']');
}

/// A conversation from its "messages" JSON (as `messages` wrote it); None if it isn't one.
pub fn parse_messages(text: &str) -> Option<Vec<Turn>> {
    let Json::Arr(items) = json::parse(text)? else { return None };
    items
        .into_iter()
        .map(|m| {
            let role = match m.get("role")?.str()? {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            let content = match m.get("content")? {
                Json::Str(s) => Content::Text(s.clone()),
                Json::Arr(blocks) => Content::Blocks(blocks.clone()),
                _ => return None,
            };
            Some(Turn { role, content })
        })
        .collect()
}

/// What the answer brought.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// More of the answer's words.
    Text(String),
    /// Claude started using a server tool (its name: "web_search", "web_fetch"); Anthropic runs it.
    Tool(String),
    /// The answer is complete (`Stream::stop_reason` says why it stopped: "tool_use" when Claude called tools,
    /// "pause_turn" when a server tool's turn was paused, to be sent back to go on).
    Done,
    /// It failed: why, in Anthropic's words or ours.
    Failed(String),
}

/// A tool call in an answer.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Json,
}

/// Reads one streamed answer.
#[derive(Default)]
pub struct Stream {
    http: Response,
    line: Vec<u8>,
    event: String,
    data: String,
    error_body: Vec<u8>,
    finished: bool,
    /// The answer's content blocks so far, by their index, and the tool calls' input as it comes (JSON text).
    blocks: Vec<Option<Json>>,
    inputs: Vec<String>,
    stop_reason: Option<String>,
}

impl Stream {
    pub fn new() -> Stream {
        Stream::default()
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The HTTP status the API answered with, once it has.
    pub fn status(&self) -> Option<u16> {
        self.http.status()
    }

    /// Why the answer stopped ("end_turn", "tool_use", "max_tokens"...), once it has.
    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }

    /// The answer's content blocks, as they came: to go back to the API in the next request.
    pub fn content(&self) -> Vec<Json> {
        self.blocks.iter().flatten().cloned().collect()
    }

    /// The tools the answer called.
    pub fn tool_uses(&self) -> Vec<ToolUse> {
        self.blocks
            .iter()
            .flatten()
            .filter(|b| b.get("type").and_then(|t| t.str()) == Some("tool_use"))
            .map(|b| ToolUse {
                id: String::from(b.get("id").and_then(|v| v.str()).unwrap_or("")),
                name: String::from(b.get("name").and_then(|v| v.str()).unwrap_or("")),
                input: b.get("input").cloned().unwrap_or(Json::Obj(Vec::new())),
            })
            .collect()
    }

    /// Bytes from the connection; the events they complete.
    pub fn feed(&mut self, data: &[u8]) -> Vec<Event> {
        let mut out = Vec::new();
        if self.finished {
            return out;
        }
        self.http.feed(data);
        if self.http.broken() {
            self.finished = true;
            out.push(Event::Failed(String::from("the answer wasn't HTTP")));
            return out;
        }
        let body = self.http.take_body();
        match self.http.status() {
            None => {}
            Some(200) => {
                for &b in &body {
                    if b == b'\n' {
                        let mut line = core::mem::take(&mut self.line);
                        if line.last() == Some(&b'\r') {
                            line.pop();
                        }
                        self.line_done(&line, &mut out);
                        if self.finished {
                            break;
                        }
                    } else if self.line.len() < 1 << 20 {
                        self.line.push(b);
                    }
                }
                if self.http.done() && !self.finished {
                    self.finished = true;
                    out.push(Event::Failed(String::from("the answer stopped partway")));
                }
            }
            Some(code) => {
                self.error_body.extend_from_slice(&body);
                if self.http.done() || self.error_body.len() > 64 * 1024 {
                    self.finished = true;
                    let text = String::from_utf8_lossy(&self.error_body).into_owned();
                    let message = json::parse(&text)
                        .and_then(|j| j.path(&["error", "message"]).and_then(|m| m.str().map(String::from)))
                        .unwrap_or_else(|| format!("Claude's API answered {code}"));
                    out.push(Event::Failed(message));
                }
            }
        }
        out
    }

    /// The connection closed.
    pub fn closed(&mut self) -> Vec<Event> {
        self.http.closed();
        let mut out = self.feed(&[]);
        if !self.finished {
            self.finished = true;
            out.push(Event::Failed(String::from("the connection closed before the answer was complete")));
        }
        out
    }

    fn line_done(&mut self, line: &[u8], out: &mut Vec<Event>) {
        let line = String::from_utf8_lossy(line);
        if line.is_empty() {
            let (event, data) = (core::mem::take(&mut self.event), core::mem::take(&mut self.data));
            if !data.is_empty() || !event.is_empty() {
                self.dispatch(&event, &data, out);
            }
            return;
        }
        if line.starts_with(':') {
            return; // a comment
        }
        let (field, value) = line.split_once(':').map(|(f, v)| (f, v.strip_prefix(' ').unwrap_or(v))).unwrap_or((&line, ""));
        match field {
            "event" => self.event = String::from(value),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
    }

    fn block(&mut self, index: usize) -> Option<&mut Json> {
        self.blocks.get_mut(index).and_then(|b| b.as_mut())
    }

    fn dispatch(&mut self, event: &str, data: &str, out: &mut Vec<Event>) {
        let Some(j) = json::parse(data) else { return };
        let kind = j.get("type").and_then(|t| t.str()).unwrap_or(event);
        let index = j.get("index").and_then(|i| match i {
            Json::Num(n) => n.parse::<usize>().ok(),
            _ => None,
        });
        match kind {
            "content_block_start" => {
                if let (Some(i), Some(block)) = (index, j.get("content_block"))
                    && i < 4096
                {
                    if self.blocks.len() <= i {
                        self.blocks.resize(i + 1, None);
                        self.inputs.resize(i + 1, String::new());
                    }
                    self.blocks[i] = Some(block.clone());
                    if block.get("type").and_then(|t| t.str()) == Some("server_tool_use") {
                        let name = block.get("name").and_then(|n| n.str()).unwrap_or("");
                        out.push(Event::Tool(String::from(name)));
                    }
                }
            }
            "content_block_delta" => {
                let Some(delta) = j.get("delta") else { return };
                let text = |key: &str| delta.get(key).and_then(|t| t.str()).unwrap_or("");
                match delta.get("type").and_then(|t| t.str()) {
                    Some("text_delta") => {
                        let words = String::from(text("text"));
                        if let Some(b) = index.and_then(|i| self.block(i)) {
                            b.append("text", &words);
                        }
                        out.push(Event::Text(words));
                    }
                    Some("thinking_delta") => {
                        let more = String::from(text("thinking"));
                        if let Some(b) = index.and_then(|i| self.block(i)) {
                            b.append("thinking", &more);
                        }
                    }
                    Some("signature_delta") => {
                        let more = String::from(text("signature"));
                        if let Some(b) = index.and_then(|i| self.block(i)) {
                            b.append("signature", &more);
                        }
                    }
                    Some("citations_delta") => {
                        if let (Some(c), Some(b)) = (delta.get("citation").cloned(), index.and_then(|i| self.block(i))) {
                            b.push("citations", c);
                        }
                    }
                    Some("input_json_delta") => {
                        let more = String::from(text("partial_json"));
                        if let Some(input) = index.and_then(|i| self.inputs.get_mut(i)) {
                            input.push_str(&more);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(i) = index {
                    let input = self.inputs.get(i).cloned().unwrap_or_default();
                    if let Some(b) = self.block(i)
                        && matches!(b.get("type").and_then(|t| t.str()), Some("tool_use" | "server_tool_use"))
                    {
                        let parsed = if input.trim().is_empty() { Some(Json::Obj(Vec::new())) } else { json::parse(&input) };
                        b.set("input", parsed.unwrap_or(Json::Obj(Vec::new())));
                    }
                }
            }
            "message_delta" => {
                if let Some(reason) = j.path(&["delta", "stop_reason"]).and_then(|r| r.str()) {
                    self.stop_reason = Some(String::from(reason));
                }
            }
            "message_stop" => {
                self.finished = true;
                out.push(Event::Done);
            }
            "error" => {
                self.finished = true;
                let message = j.path(&["error", "message"]).and_then(|m| m.str()).unwrap_or("Claude's API reported an error");
                out.push(Event::Failed(String::from(message)));
            }
            _ => {} // message_start, ping
        }
    }
}
