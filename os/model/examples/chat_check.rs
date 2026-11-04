//! Writes conversations out as chat.rs does, for checking against the models' own Jinja templates
//! (scratchpad/refcheck/chat_reference.py does the other side).
//!
//!     cargo run --release --example chat_check -- MODEL.ccm CASES.json > OURS.jsonl
//!
//! CASES.json: a list of {"style", "tools", "messages" (as OpenAI-style APIs give them), "think", "keep", "effort"};
//! each line out: {"text", "ids"}.

use model::chat::{self, Answer, Call, Effort, Message, Options, Style};
use model::json::{self, Json};
use model::{File, load_tokenizer};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0]).expect("the model file");
    let file = File::open(&bytes).expect("a model file");
    let tok = load_tokenizer(&file).expect("its tokenizer");
    let cases = json::parse(&std::fs::read_to_string(&args[1]).unwrap()).expect("the cases");
    for case in cases.array().unwrap() {
        let style = Style::named(case.get("style").and_then(Json::str).unwrap()).expect("a style");
        let tools: Vec<Json> = case.get("tools").and_then(Json::array).map(|a| a.to_vec()).unwrap_or_default();
        let messages: Vec<Message> = case.get("messages").and_then(Json::array).unwrap().iter().map(message).collect();
        let o = Options {
            think: case.get("think").and_then(Json::bool),
            keep_thinking: case.get("keep").and_then(Json::bool),
            effort: match case.get("effort").and_then(Json::str) {
                Some("medium") => Effort::Medium,
                Some("low") => Effort::Low,
                _ => Effort::Most,
            },
        };
        let out = match chat::render(style, &tools, &messages, &o) {
            Ok(p) => {
                let mut ids = Vec::new();
                p.tokens(&tok, &mut ids).unwrap();
                Json::Object(vec![
                    ("text".into(), Json::Str(p.to_text())),
                    ("ids".into(), Json::Array(ids.iter().map(|&i| Json::Number(i as f64)).collect())),
                ])
            }
            Err(e) => Json::Object(vec![("error".into(), Json::Str(format!("{e:?}")))]),
        };
        println!("{}", json::compact(&out));
    }
}

fn message(m: &Json) -> Message {
    let text = |k: &str| m.get(k).and_then(Json::str).unwrap_or("").to_string();
    match m.get("role").and_then(Json::str).unwrap() {
        "system" => Message::System(text("content")),
        "user" => Message::User(text("content")),
        "tool" => Message::Tool(text("content")),
        "assistant" => Message::Assistant(Answer {
            thinking: text("reasoning_content"),
            text: text("content"),
            calls: m
                .get("tool_calls")
                .and_then(Json::array)
                .unwrap_or(&[])
                .iter()
                .map(|c| {
                    let f = c.get("function").unwrap_or(c);
                    Call {
                        name: f.get("name").and_then(Json::str).unwrap().to_string(),
                        args: f
                            .get("arguments")
                            .and_then(Json::object)
                            .unwrap_or(&[])
                            .iter()
                            .map(|(k, v)| (k.clone(), v.str().unwrap().to_string()))
                            .collect(),
                    }
                })
                .collect(),
        }),
        r => panic!("role {r}"),
    }
}
