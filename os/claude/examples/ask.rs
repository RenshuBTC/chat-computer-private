//! Chat Computer's own TLS 1.3 and its own HTTP and Claude API code (the same the kernel runs), run on an ordinary
//! computer against the real api.anthropic.com: a test of the whole path, certificate checks included, that a
//! virtual PC behind a re-encrypting proxy can't do. Without an API key, the API's answer is a refusal (401), which
//! is enough to show every layer works.
//!
//!     cargo run -p claude --example ask
//!
//! Prints what happened at each step; never prints a key (none is sent).

use claude::api::{self, Role, Stream, Turn};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn urandom(b: &mut [u8]) {
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(b)).expect("/dev/urandom");
}

fn main() {
    let host = api::HOST;
    let started = Instant::now();
    let mut sock = TcpStream::connect((host, 443)).expect("connecting to api.anthropic.com:443");
    sock.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    println!("connected to {} ({:?})", sock.peer_addr().unwrap(), started.elapsed());
    let roots: &'static [tls::Cert<'static>] = Box::leak(tls::roots().into_boxed_slice());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let mut client = tls::Client::new(host, now, roots, &mut urandom);
    let request = api::request(host, "", &api::Ask { model: api::MODEL, effort: Some(api::EFFORT), max_tokens: api::MAX_TOKENS }, "Answer in one short sentence.", &[Turn::text(Role::User, "hi")], &[], &[]);
    let mut stream = Stream::new();
    let mut sent = false;
    let mut buf = vec![0u8; 16384];
    loop {
        let out = client.outgoing();
        if !out.is_empty() {
            sock.write_all(&out).unwrap();
        }
        if let Some(e) = client.error() {
            println!("TLS failed: {e}");
            std::process::exit(1);
        }
        if client.is_open() && !sent {
            println!(
                "TLS 1.3 open: {:?}, ALPN {:?}, the certificate chain checked against {} of Mozilla's authorities ({:?})",
                client.suite(),
                client.alpn().map(String::from_utf8_lossy),
                roots.len(),
                started.elapsed()
            );
            client.write(&request);
            sent = true;
            continue;
        }
        let n = client.read(&mut buf);
        if n > 0 {
            for e in stream.feed(&buf[..n]) {
                println!("HTTP {:?}: {:?}", stream.status(), e);
            }
            if stream.finished() {
                break;
            }
            continue;
        }
        match sock.read(&mut buf) {
            Ok(0) => {
                for e in stream.closed() {
                    println!("closed: {e:?}");
                }
                break;
            }
            Ok(n) => client.incoming(&buf[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => panic!("{e}"),
        }
        if started.elapsed() > Duration::from_secs(60) {
            println!("gave up after a minute");
            std::process::exit(1);
        }
    }
    println!("done ({:?})", started.elapsed());
}
