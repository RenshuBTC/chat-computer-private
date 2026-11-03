//! Checks a session's reuse: each request read only as far as it's new, and every answer the same as a new session
//! (that knows the same earlier answers) gives. Greedy, so answers are the same.
//!
//!     cargo run --release --features std --example session_check -- MODEL.ccm
use model::chat::{Message, Options, Style};
use model::sample::Sampling;
use model::session::{Request, Session};
use model::threads::Threads;
use model::{File, Model};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let file = File::open(&bytes).unwrap();
    let m = Model::load(&file).unwrap();
    let style = Style::named(file.config().unwrap().str("chat.style").unwrap_or("qwen3.5-small")).unwrap();
    let pool = Threads::for_this_computer();
    let mut s = Session::new(&m, style);
    let o = Options { think: Some(false), keep_thinking: Some(true), ..Options::default() };
    let ask = |s: &mut Session, msgs: &[Message]| {
        let r = Request { tools: &[], messages: msgs, options: o, sampling: Sampling::GREEDY, max_tokens: 24, seed: 1 };
        let (a, st) = s.ask(&r, &pool, &mut |_, _| true).unwrap();
        (a, st)
    };
    let user = |t: &str| Message::User(t.to_string());
    let mut msgs = vec![Message::System("Answer in one short sentence.".into()), user("What is the capital of France?")];
    let (a1, st1) = ask(&mut s, &msgs);
    println!("1: read {} of {} ({:?}): {:?}", st1.read, st1.prompt, st1.ended, a1.text);
    msgs.push(Message::Assistant(a1.clone()));
    msgs.push(user("And of Italy?"));
    let (a2, st2) = ask(&mut s, &msgs);
    println!("2: read {} of {}: {:?}", st2.read, st2.prompt, a2.text);
    // A new session that knows answer 1 (as the first one does): the same answer 2.
    let mut fresh = Session::new(&m, style);
    let (_, _) = ask(&mut fresh, &msgs[..2]);
    let (b2, _) = ask(&mut fresh, &msgs);
    assert_eq!(a2, b2, "answer 2 differs from a new session's");
    // The second question changed: back to where the first prompt ended.
    let mut changed = msgs.clone();
    *changed.last_mut().unwrap() = user("And of Spain?");
    let (a3, st3) = ask(&mut s, &changed);
    println!("3: read {} of {}: {:?}", st3.read, st3.prompt, a3.text);
    let mut fresh = Session::new(&m, style);
    let (_, _) = ask(&mut fresh, &msgs[..2]);
    let (b3, _) = ask(&mut fresh, &changed);
    assert_eq!(a3, b3, "answer 3 differs from a new session's");
    // The same again: nothing read.
    let (a4, st4) = ask(&mut s, &changed);
    println!("4: read {} of {}: {:?}", st4.read, st4.prompt, a4.text);
    assert_eq!(a3, a4);
    assert_eq!(st4.read, 0);
    // An earlier request again (the first): from its snapshot, or from the start.
    let (a5, st5) = ask(&mut s, &msgs[..2]);
    println!("5: read {} of {}: {:?}", st5.read, st5.prompt, a5.text);
    assert_eq!(a5, a1);
    println!("session: {:.1} MB", s.bytes() as f64 / 1e6);
}
