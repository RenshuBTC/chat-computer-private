//! Claude's Messages API (POST /v1/messages), streamed: the request, and the answer's text as it comes, from the
//! server-sent events (event: and data: lines, a blank line after each).

use crate::http::{self, Response};
use crate::json;
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

/// The whole request, ready to send: the conversation so far, newest last.
pub fn request(host: &str, key: &str, system: &str, conversation: &[(Role, String)]) -> Vec<u8> {
    let mut body = format!(
        "{{\"model\":{},\"max_tokens\":{},\"stream\":true,\"output_config\":{{\"effort\":{}}},\"system\":{},\"messages\":[",
        json::quote(MODEL),
        MAX_TOKENS,
        json::quote(EFFORT),
        json::quote(system)
    );
    for (i, (role, text)) in conversation.iter().enumerate() {
        if i > 0 {
            body.push(',');
        }
        let role = match role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        body.push_str(&format!("{{\"role\":\"{role}\",\"content\":{}}}", json::quote(text)));
    }
    body.push_str("]}");
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

/// What the answer brought.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// More of the answer's words.
    Text(String),
    /// The answer is complete.
    Done,
    /// It failed: why, in Anthropic's words or ours.
    Failed(String),
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

    fn dispatch(&mut self, event: &str, data: &str, out: &mut Vec<Event>) {
        let Some(j) = json::parse(data) else { return };
        let kind = j.get("type").and_then(|t| t.str()).unwrap_or(event);
        match kind {
            "content_block_delta" => {
                if j.path(&["delta", "type"]).and_then(|t| t.str()) == Some("text_delta")
                    && let Some(text) = j.path(&["delta", "text"]).and_then(|t| t.str())
                {
                    out.push(Event::Text(String::from(text)));
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
            _ => {} // message_start, content_block_start/stop (thinking too), message_delta, ping
        }
    }
}

