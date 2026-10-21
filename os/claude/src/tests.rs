//! JSON, HTTP and the streamed answer, with what Claude's API really sends (its event stream, chunked, cut
//! anywhere), and its errors.

use super::api::{self, Event, Role, Stream};
use super::http::{self, Response};
use super::json::{self, Json};

#[test]
fn json_reads_what_the_api_sends_and_writes_strings_safely() {
    let j = json::parse(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello \"there\"\n\u00e9\ud83d\ude00"}}"#).unwrap();
    assert_eq!(j.path(&["delta", "text"]).and_then(|t| t.str()), Some("Hello \"there\"\né😀"));
    assert_eq!(j.get("index"), Some(&Json::Num("0".into())));
    assert_eq!(json::parse("[1, -2.5e3, true, false, null, {}, []]").map(|j| matches!(j, Json::Arr(ref a) if a.len() == 7)), Some(true));
    for bad in ["", "{", "{\"a\":}", "[1,]", "\"\\x\"", "nul", "{\"a\":1} x", "\"\u{1}\""] {
        assert_eq!(json::parse(bad), None, "{bad:?}");
    }
    // Too deep: refused, not a crash.
    assert_eq!(json::parse(&"[".repeat(100_000)), None);
    let q = json::quote("say \"hi\"\n\t\\ \u{1} é");
    assert_eq!(q, "\"say \\\"hi\\\"\\n\\t\\\\ \\u0001 é\"");
    assert_eq!(json::parse(&q).unwrap(), Json::Str("say \"hi\"\n\t\\ \u{1} é".into()));
}

#[test]
fn the_request_is_opus_at_max_effort_streamed() {
    let r = api::request("api.anthropic.com", "sk-ant-test", "Be brief.", &[(Role::User, "Hi \"Claude\"".into())]);
    let text = String::from_utf8(r).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/messages HTTP/1.1\r\nHost: api.anthropic.com\r\n"));
    assert!(head.contains("\r\nx-api-key: sk-ant-test\r\n") && head.contains("\r\nanthropic-version: 2023-06-01\r\n"));
    assert!(head.contains(&format!("\r\nContent-Length: {}", body.len())));
    let j = json::parse(body).unwrap();
    assert_eq!(j.get("model").and_then(|m| m.str()), Some("claude-opus-5-5"));
    assert_eq!(j.path(&["output_config", "effort"]).and_then(|m| m.str()), Some("max"));
    assert_eq!(j.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(j.get("messages").map(|m| format!("{m:?}")).unwrap().matches("Hi \\\"Claude\\\"").count(), 1);
}

/// A streamed answer as the API sends it: thinking first (its text left out), then the words.
fn answer() -> Vec<u8> {
    let events = [
        ("message_start", r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-opus-5-5"}}"#),
        ("content_block_start", r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#),
        ("content_block_delta", r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#),
        ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#),
        ("ping", r#"{"type": "ping"}"#),
        ("content_block_start", r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#),
        ("content_block_delta", r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello"}}"#),
        ("content_block_delta", r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":", world: é ✓"}}"#),
        ("content_block_stop", r#"{"type":"content_block_stop","index":1}"#),
        ("message_delta", r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}"#),
        ("message_stop", r#"{"type":"message_stop"}"#),
    ];
    let mut sse = String::new();
    for (e, d) in events {
        sse.push_str(&format!("event: {e}\ndata: {d}\n\n"));
    }
    // Chunked, in uneven chunks, as servers send it.
    let mut body = Vec::new();
    for piece in sse.as_bytes().chunks(77) {
        body.extend_from_slice(format!("{:x}\r\n", piece.len()).as_bytes());
        body.extend_from_slice(piece);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let mut r = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nrequest-id: req_1\r\n\r\n".to_vec();
    r.extend_from_slice(&body);
    r
}

fn run(bytes: &[u8], cut: usize) -> Vec<Event> {
    let mut s = Stream::new();
    let mut out = Vec::new();
    for piece in bytes.chunks(cut) {
        out.extend(s.feed(piece));
    }
    out
}

#[test]
fn a_streamed_answer_comes_out_word_by_word_however_it_is_cut() {
    for cut in [1, 2, 3, 7, 64, 1000, 100_000] {
        let events = run(&answer(), cut);
        assert_eq!(events, vec![Event::Text("Hello".into()), Event::Text(", world: é ✓".into()), Event::Done], "cut {cut}");
    }
}

#[test]
fn errors_say_what_anthropic_said() {
    let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"},"request_id":"req_2"}"#;
    let r = format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    assert_eq!(run(r.as_bytes(), 5), vec![Event::Failed("invalid x-api-key".into())]);
    // Overloaded partway through: an error event in the stream.
    let r = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
    assert_eq!(run(r.as_bytes(), 9), vec![Event::Text("Hi".into()), Event::Failed("Overloaded".into())]);
    // Cut off before the end.
    let full = answer();
    let mut s = Stream::new();
    let mut events = s.feed(&full[..full.len() / 2]);
    events.extend(s.closed());
    assert!(matches!(events.last(), Some(Event::Failed(_))));
    // Not HTTP at all.
    assert!(matches!(run(b"SSH-2.0-OpenSSH\r\n\r\n", 4).as_slice(), [Event::Failed(_)]));
}

#[test]
fn http_bodies_by_length_chunks_or_until_closed() {
    let mut r = Response::new();
    r.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel");
    assert_eq!((r.status(), r.take_body(), r.done()), (Some(200), b"hel".to_vec(), false));
    r.feed(b"loEXTRA");
    assert_eq!((r.take_body(), r.done()), (b"lo".to_vec(), true));
    let mut r = Response::new();
    r.feed(b"HTTP/1.0 200 ok\r\n\r\nall of it");
    r.closed();
    assert_eq!((r.take_body(), r.done()), (b"all of it".to_vec(), true));
    let mut r = Response::new();
    r.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;ext=1\r\nWiki\r\n5\r\npedia\r\n0\r\nTrailer: x\r\n\r\n");
    assert_eq!((r.take_body(), r.done()), (b"Wikipedia".to_vec(), true));
    let req = http::request("GET", "example.com", "/", &[("Accept", "*/*")], b"");
    assert_eq!(req, b"GET / HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\n\r\n".to_vec());
}
