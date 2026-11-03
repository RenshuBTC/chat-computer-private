//! What Chat Computer's engine says to what Chat Computer's own kernel asked its model (for check.py --model): the
//! messages a test build of the kernel logged ("test: asked [...]"), with the kernel's own tools
//! (os/kernel/src/tools.json), asked as the kernel asks (no thinking first, always the likeliest token), on one
//! processor core, with the arithmetic the kernel used (its log says which). Prints the answer's first round as the
//! kernel's test build logs it ("test: round {...}"): its words and the tools it called.
//!
//!     cargo run --release --features std --example kernel_check -- MODEL.ccm ASKED.json [--tools TOOLS.json]
//!         [--max TOKENS] [--level plain|avx2|avx-vnni|avx-512-vnni]
//!
//! (--tools: the tools it was given, if not the kernel's own; --max: the most the round says, if not 2,048.)
use model::chat::{Answer, Call, Message, Options, Style};
use model::cpu::Level;
use model::json::{self, Json};
use model::sample::Sampling;
use model::session::{Request, Session};
use model::{File, Model, Serial};

const TOOLS: &str = include_str!("../../kernel/src/tools.json");

fn text(m: &Json, key: &str) -> String {
    m.get(key).and_then(Json::str).unwrap_or("").to_string()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("the model's file");
    let asked = args.next().expect("what the kernel asked (JSON)");
    let (mut tools_text, mut max_tokens) = (TOOLS.to_string(), 2048);
    while let Some(a) = args.next() {
        let value = args.next().expect("a value after each option");
        match a.as_str() {
            "--tools" => tools_text = std::fs::read_to_string(&value).expect("reading the tools"),
            "--max" => max_tokens = value.parse().expect("a number of tokens"),
            "--level" => model::cpu::at_most(match value.as_str() {
                "plain" => Level::Plain,
                "avx2" => Level::Avx2,
                "avx-vnni" => Level::AvxVnni,
                _ => Level::Avx512Vnni,
            }),
            other => panic!("{other}: not an option"),
        }
    }
    let bytes = std::fs::read(&path).expect("reading the model's file");
    let file = File::open(&bytes).expect("a model's file");
    let model = Model::load(&file).expect("a model the engine runs");
    let style_name = file.config().ok().and_then(|c| c.str("chat.style").map(String::from)).unwrap_or_else(|| "qwen3.5".into());
    let style = Style::named(&style_name).expect("a chat style the engine knows");
    let Some(Json::Array(tools)) = json::parse(&tools_text) else { panic!("the tools aren't a JSON list") };
    let asked = json::parse(&std::fs::read_to_string(&asked).expect("reading what was asked")).expect("JSON");
    let messages: Vec<Message> = asked
        .array()
        .expect("a list of messages")
        .iter()
        .map(|m| match m.get("role").and_then(Json::str) {
            Some("system") => Message::System(text(m, "content")),
            Some("user") => Message::User(text(m, "content")),
            Some("tool") => Message::Tool(text(m, "content")),
            _ => Message::Assistant(Answer {
                thinking: String::new(),
                text: text(m, "content"),
                calls: m
                    .get("calls")
                    .and_then(Json::array)
                    .unwrap_or(&[])
                    .iter()
                    .map(|c| Call {
                        name: text(c, "name"),
                        args: c.get("args").and_then(Json::object).unwrap_or(&[]).iter().map(|(k, v)| (k.clone(), v.str().unwrap_or("").to_string())).collect(),
                    })
                    .collect(),
            }),
        })
        .collect();
    let r = Request {
        tools: &tools,
        messages: &messages,
        options: Options { think: Some(false), keep_thinking: Some(true), ..Options::default() },
        sampling: Sampling::GREEDY,
        max_tokens,
        seed: 1,
    };
    let mut session = Session::new(&model, style);
    let (answer, _) = session.ask(&r, &Serial, &mut |_, _| true).expect("an answer");
    let calls = answer
        .calls
        .iter()
        .map(|c| {
            let args = c.args.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect();
            Json::Object(vec![("name".into(), Json::Str(c.name.clone())), ("args".into(), Json::Object(args))])
        })
        .collect();
    println!("{}", json::compact(&Json::Object(vec![("text".into(), Json::Str(answer.text)), ("calls".into(), Json::Array(calls))])));
}
