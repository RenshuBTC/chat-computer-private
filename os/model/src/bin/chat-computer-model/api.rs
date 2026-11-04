//! The server's requests and answers, as OpenAI's chat completions API has them (the way most programs talk to a
//! model, so any of them can talk to this one): messages with their roles, tools, tool calls and their results,
//! the model's thinking as "reasoning_content", and answers whole or streamed a piece at a time.

use model::chat::{Answer, Call, Effort, Message, Options};
use model::json::{self, Json};
use model::sample::Sampling;

pub struct Asked {
    pub tools: Vec<Json>,
    pub messages: Vec<Message>,
    pub options: Options,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<usize>,
    pub presence_penalty: Option<f32>,
    pub max_tokens: usize,
    pub stream: bool,
    pub seed: u64,
}

impl Asked {
    /// The sampling asked for, over the model's makers' recommendation.
    pub fn sampling(&self, recommended: Sampling) -> Sampling {
        Sampling {
            temperature: self.temperature.unwrap_or(recommended.temperature),
            top_p: self.top_p.unwrap_or(recommended.top_p),
            top_k: self.top_k.unwrap_or(recommended.top_k),
            presence_penalty: self.presence_penalty.unwrap_or(recommended.presence_penalty),
        }
    }
}

fn text_of(content: Option<&Json>) -> Result<String, String> {
    match content {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Str(s)) => Ok(s.clone()),
        Some(Json::Array(parts)) => {
            let mut out = String::new();
            for p in parts {
                match p.get("type").and_then(Json::str) {
                    Some("text") => out.push_str(p.get("text").and_then(Json::str).unwrap_or("")),
                    Some(other) => return Err(format!("this model reads text only, not {other}")),
                    None => return Err("a part of a message without a type".into()),
                }
            }
            Ok(out)
        }
        _ => Err("a message's content isn't text".into()),
    }
}

/// A tool call's arguments as the model writes them: text as it is, anything else as JSON.
fn raw_args(args: Option<&Json>) -> Result<Vec<(String, String)>, String> {
    let parsed;
    let object = match args {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Str(s)) if s.trim().is_empty() => return Ok(Vec::new()),
        Some(Json::Str(s)) => {
            parsed = json::parse(s).ok_or("a tool call's arguments aren't JSON")?;
            &parsed
        }
        Some(o) => o,
    };
    let items = object.object().ok_or("a tool call's arguments aren't an object")?;
    Ok(items
        .iter()
        .map(|(k, v)| (k.clone(), match v {
            Json::Str(s) => s.clone(),
            other => json::dumps(other),
        }))
        .collect())
}

pub fn parse(body: &[u8]) -> Result<Asked, String> {
    let text = std::str::from_utf8(body).map_err(|_| "the request isn't UTF-8")?;
    let j = json::parse(text).ok_or("the request isn't JSON")?;
    let mut messages = Vec::new();
    for m in j.get("messages").and_then(Json::array).ok_or("no messages")? {
        let content = text_of(m.get("content"))?;
        messages.push(match m.get("role").and_then(Json::str).ok_or("a message without a role")? {
            "system" | "developer" => Message::System(content),
            "user" => Message::User(content),
            "tool" => Message::Tool(content),
            "assistant" => {
                let thinking = m.get("reasoning_content").or_else(|| m.get("reasoning")).and_then(Json::str).unwrap_or("").to_string();
                let mut calls = Vec::new();
                for c in m.get("tool_calls").and_then(Json::array).unwrap_or(&[]) {
                    let f = c.get("function").unwrap_or(c);
                    let name = f.get("name").and_then(Json::str).ok_or("a tool call without a name")?.to_string();
                    calls.push(Call { name, args: raw_args(f.get("arguments"))? });
                }
                Message::Assistant(Answer { thinking, text: content, calls })
            }
            other => return Err(format!("a message from \"{other}\", which isn't a role")),
        });
    }
    let tools = j.get("tools").and_then(Json::array).map(|a| a.to_vec()).unwrap_or_default();
    let kwargs = j.get("chat_template_kwargs");
    let flag = |name: &str| kwargs.and_then(|k| k.get(name)).and_then(Json::bool);
    let options = Options {
        think: flag("enable_thinking").or_else(|| j.get("think").and_then(Json::bool)),
        keep_thinking: flag("preserve_thinking"),
        effort: match j.get("reasoning_effort").and_then(Json::str) {
            Some("medium") => Effort::Medium,
            Some("low") | Some("minimal") => Effort::Low,
            _ => Effort::Most,
        },
    };
    let num = |k: &str| j.get(k).and_then(Json::num);
    Ok(Asked {
        tools,
        messages,
        options,
        temperature: num("temperature").map(|v| v as f32),
        top_p: num("top_p").map(|v| v as f32),
        top_k: j.get("top_k").and_then(Json::usize),
        presence_penalty: num("presence_penalty").map(|v| v as f32),
        max_tokens: j.get("max_completion_tokens").or_else(|| j.get("max_tokens")).and_then(Json::usize).unwrap_or(32768).max(1),
        stream: j.get("stream").and_then(Json::bool).unwrap_or(false),
        seed: j.get("seed").and_then(Json::usize).map(|s| s as u64).unwrap_or(0x5eed),
    })
}

fn s(v: &str) -> Json {
    Json::Str(v.to_string())
}

fn obj(items: Vec<(&str, Json)>) -> Json {
    Json::Object(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A tool call as the API gives it: its arguments as JSON (typed by the tool's description) in a string.
pub fn call_json(c: &Call, index: usize, id: &str, tools: &[Json]) -> Json {
    let tool = tools.iter().find(|t| t.get("function").unwrap_or(t).get("name").and_then(Json::str) == Some(c.name.as_str()));
    obj(vec![
        ("index", Json::Number(index as f64)),
        ("id", s(id)),
        ("type", s("function")),
        ("function", obj(vec![("name", s(&c.name)), ("arguments", s(&json::compact(&c.arguments(tool))))])),
    ])
}

/// One piece of a streamed answer.
pub fn chunk(id: &str, model: &str, delta: Json, finish: Option<&str>, usage: Option<Json>) -> String {
    let mut items = vec![
        ("id", s(id)),
        ("object", s("chat.completion.chunk")),
        ("created", Json::Number(now() as f64)),
        ("model", s(model)),
        ("choices", Json::Array(vec![obj(vec![
            ("index", Json::Number(0.0)),
            ("delta", delta),
            ("finish_reason", finish.map(s).unwrap_or(Json::Null)),
        ])])),
    ];
    if let Some(u) = usage {
        items.push(("usage", u));
    }
    json::compact(&obj(items))
}

pub fn delta(kind: &str, text: &str) -> Json {
    obj(vec![(kind, s(text))])
}

pub fn calls_delta(calls: Vec<Json>) -> Json {
    obj(vec![("tool_calls", Json::Array(calls))])
}

pub fn usage(prompt: usize, read: usize, generated: usize) -> Json {
    obj(vec![
        ("prompt_tokens", Json::Number(prompt as f64)),
        ("completion_tokens", Json::Number(generated as f64)),
        ("total_tokens", Json::Number((prompt + generated) as f64)),
        ("prompt_tokens_details", obj(vec![("cached_tokens", Json::Number((prompt - read) as f64))])),
    ])
}

/// The whole answer at once.
pub fn whole(id: &str, model: &str, a: &Answer, calls: Vec<Json>, finish: &str, usage: Json) -> String {
    let mut message = vec![("role", s("assistant")), ("content", if a.text.is_empty() && !calls.is_empty() { Json::Null } else { s(&a.text) })];
    if !a.thinking.is_empty() {
        message.push(("reasoning_content", s(&a.thinking)));
    }
    if !calls.is_empty() {
        message.push(("tool_calls", Json::Array(calls)));
    }
    json::compact(&obj(vec![
        ("id", s(id)),
        ("object", s("chat.completion")),
        ("created", Json::Number(now() as f64)),
        ("model", s(model)),
        ("choices", Json::Array(vec![obj(vec![
            ("index", Json::Number(0.0)),
            ("message", obj(message)),
            ("finish_reason", s(finish)),
        ])])),
        ("usage", usage),
    ]))
}

pub fn error(message: &str) -> String {
    json::compact(&obj(vec![("error", obj(vec![("message", s(message)), ("type", s("invalid_request_error"))]))]))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
