//! The client against OpenSSL's own server (`openssl s_server`), with certificate authorities made for each
//! test: every cipher, ECDSA (P-256, P-384) and RSA keys, a chain with an authority in the middle, a big
//! download; and the ways it must refuse: the wrong name, an authority it doesn't trust, an expired
//! certificate, a forged signature. Also: Mozilla's authorities all read.

use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn urandom(b: &mut [u8]) {
    std::fs::File::open("/dev/urandom").unwrap().read_exact(b).unwrap();
}

fn openssl(dir: &Path, args: &[&str]) {
    let out = Command::new("openssl").args(args).current_dir(dir).output().expect("openssl");
    assert!(out.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A key's options for `openssl req -newkey`.
fn key(kind: &str) -> Vec<&'static str> {
    match kind {
        "p256" => vec!["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256"],
        "p384" => vec!["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-384"],
        "rsa" => vec!["-newkey", "rsa:2048"],
        _ => unreachable!(),
    }
}

/// A test authority (`root`), perhaps one in the middle, and a server certificate for `names`, in a new folder.
fn pki(name: &str, root: &str, middle: Option<&str>, leaf: &str, names: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cc-tls-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = vec!["req", "-x509", "-nodes", "-keyout", "ca.key", "-out", "ca.pem", "-days", "30", "-subj", "/CN=Test Root",
                     "-addext", "basicConstraints=critical,CA:true", "-addext", "keyUsage=critical,keyCertSign,cRLSign"];
    a.extend(key(root));
    openssl(&dir, &a);
    std::fs::write(dir.join("ca.ext"), "basicConstraints=critical,CA:true,pathlen:0\nkeyUsage=critical,keyCertSign\n").unwrap();
    std::fs::write(dir.join("leaf.ext"), format!(
        "subjectAltName={names}\nextendedKeyUsage=serverAuth\nkeyUsage=critical,digitalSignature\nbasicConstraints=critical,CA:false\n")).unwrap();
    let (signer, signer_key) = match middle {
        Some(kind) => {
            let mut a = vec!["req", "-nodes", "-keyout", "mid.key", "-out", "mid.csr", "-subj", "/CN=Test Middle"];
            a.extend(key(kind));
            openssl(&dir, &a);
            openssl(&dir, &["x509", "-req", "-in", "mid.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "mid.pem",
                            "-days", "30", "-extfile", "ca.ext", "-sha384"]);
            ("mid.pem", "mid.key")
        }
        None => ("ca.pem", "ca.key"),
    };
    let mut a = vec!["req", "-nodes", "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", "/CN=localhost"];
    a.extend(key(leaf));
    openssl(&dir, &a);
    openssl(&dir, &["x509", "-req", "-in", "leaf.csr", "-CA", signer, "-CAkey", signer_key, "-CAcreateserial", "-out", "leaf.pem",
                    "-days", "30", "-extfile", "leaf.ext"]);
    std::fs::write(dir.join("chain.pem"), if middle.is_some() { std::fs::read(dir.join("mid.pem")).unwrap() } else { Vec::new() }).unwrap();
    dir
}

fn pem_to_der(path: &Path) -> Vec<u8> {
    let out = Command::new("openssl").args(["x509", "-outform", "DER", "-in"]).arg(path).output().unwrap();
    out.stdout
}

/// The test authority, as trusted roots (kept for the rest of the run, as the kernel keeps its own).
fn trusting(dir: &Path) -> &'static [Cert<'static>] {
    let der: &'static [u8] = Box::leak(pem_to_der(&dir.join("ca.pem")).into_boxed_slice());
    Box::leak(vec![x509::parse_root(der).unwrap()].into_boxed_slice())
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(dir: &Path, suite: &str) -> Server {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut cmd = Command::new("openssl");
    cmd.args(["s_server", "-accept", &format!("127.0.0.1:{port}"), "-cert", "leaf.pem", "-key", "leaf.key", "-tls1_3",
              "-ciphersuites", suite, "-WWW", "-quiet", "-naccept", "1"]);
    if std::fs::metadata(dir.join("chain.pem")).unwrap().len() > 0 {
        cmd.args(["-cert_chain", "chain.pem"]);
    }
    let child = cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).map(drop).is_ok() {
            // (that connection used up -naccept 1: start it again for real)
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut s = Server { child, port };
    let _ = s.child.kill();
    let _ = s.child.wait();
    let child = cmd.spawn().unwrap();
    s.child = child;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(30));
        if std::process::Command::new("sh").arg("-c").arg(format!("ss -ltn | grep -q ':{port} '")).status().map(|s| s.success()).unwrap_or(false) {
            break;
        }
    }
    s
}

/// A conversation: the handshake, then `request`; everything the server sends back, or what went wrong.
fn talk(port: u16, host: &str, roots: &'static [Cert<'static>], when: i64, request: &[u8]) -> Result<(Vec<u8>, Suite), Error> {
    talk_offering(port, host, roots, when, request, &[Suite::ChaCha20Poly1305, Suite::Aes128Gcm, Suite::Aes256Gcm])
}

fn talk_offering(port: u16, host: &str, roots: &'static [Cert<'static>], when: i64, request: &[u8], suites: &[Suite])
    -> Result<(Vec<u8>, Suite), Error> {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut c = Client::offering(host, when, roots, &mut urandom, suites);
    let (mut sent, mut got) = (false, Vec::new());
    let mut buf = vec![0u8; 32768];
    loop {
        if c.is_open() && !sent {
            assert!(c.write(request));
            sent = true;
        }
        let out = c.outgoing();
        if !out.is_empty() {
            sock.write_all(&out).unwrap();
        }
        if let Some(e) = c.error() {
            return Err(e);
        }
        loop {
            let n = c.read(&mut buf);
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        if c.finished() {
            break;
        }
        let n = match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        c.incoming(&buf[..n]);
    }
    if let Some(e) = c.error() {
        return Err(e);
    }
    Ok((got, c.suite().unwrap()))
}

#[test]
fn mozillas_authorities_all_read() {
    let roots = roots();
    assert!(roots.len() >= 110, "only {} of Mozilla's roots read", roots.len());
}

#[test]
fn every_cipher_with_ecdsa_rsa_and_a_chain() {
    let cases = [
        ("p256", "p256", None, "p256", "TLS_AES_128_GCM_SHA256", Suite::Aes128Gcm),
        ("p256", "p256", None, "p256", "TLS_AES_256_GCM_SHA384", Suite::Aes256Gcm),
        ("p256", "p256", None, "p256", "TLS_CHACHA20_POLY1305_SHA256", Suite::ChaCha20Poly1305),
        ("rsa", "rsa", Some("rsa"), "rsa", "TLS_AES_128_GCM_SHA256", Suite::Aes128Gcm),
        ("mixed", "p384", Some("rsa"), "p384", "TLS_CHACHA20_POLY1305_SHA256", Suite::ChaCha20Poly1305),
        ("p384", "p384", Some("p256"), "p384", "TLS_AES_256_GCM_SHA384", Suite::Aes256Gcm),
    ];
    for (name, root, middle, leaf, suite, want) in cases {
        let dir = pki(name, root, middle, leaf, "DNS:localhost,DNS:*.chatcomputer.test");
        std::fs::write(dir.join("hello.txt"), "hello, Chat Computer\n").unwrap();
        let server = serve(&dir, suite);
        let (got, used) = talk(server.port, "localhost", trusting(&dir), now(), b"GET /hello.txt HTTP/1.0\r\n\r\n")
            .unwrap_or_else(|e| panic!("{name} {suite}: {e}"));
        let text = String::from_utf8_lossy(&got);
        assert!(text.starts_with("HTTP/1.0 200 ok") && text.ends_with("hello, Chat Computer\n"), "{name} {suite}: {text:?}");
        assert_eq!(used, want);
    }
}

#[test]
fn only_the_ciphers_offered() {
    let dir = pki("offer", "p256", None, "p256", "DNS:localhost");
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    let chacha = [Suite::ChaCha20Poly1305];
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256:TLS_CHACHA20_POLY1305_SHA256");
    let (_, used) = talk_offering(server.port, "localhost", trusting(&dir), now(), b"GET /a.txt HTTP/1.0\r\n\r\n", &chacha).unwrap();
    assert_eq!(used, Suite::ChaCha20Poly1305);
    // A server without it says so (handshake_failure), rather than using another.
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    let got = talk_offering(server.port, "localhost", trusting(&dir), now(), b"GET /a.txt HTTP/1.0\r\n\r\n", &chacha);
    assert!(matches!(got, Err(Error::Alert(40))), "{got:?}");
}

#[test]
fn a_big_download_arrives_whole() {
    let dir = pki("big", "p256", None, "p256", "DNS:localhost");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join("big.bin"), &data).unwrap();
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    let (got, _) = talk(server.port, "localhost", trusting(&dir), now(), b"GET /big.bin HTTP/1.0\r\n\r\n").unwrap();
    let body = &got[got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4..];
    assert!(body == data, "{} bytes of {}", body.len(), data.len());
}

#[test]
fn wildcards_cover_one_label() {
    let dir = pki("wild", "p256", None, "p256", "DNS:*.chatcomputer.test");
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    // (the server doesn't care which name was asked for; the client checks it)
    assert!(talk(server.port, "api.chatcomputer.test", trusting(&dir), now(), b"GET /a.txt HTTP/1.0\r\n\r\n").is_ok());
    assert!(x509::name_matches(b"*.chatcomputer.test", b"API.ChatComputer.TEST"));
    assert!(!x509::name_matches(b"*.chatcomputer.test", b"a.b.chatcomputer.test"));
    assert!(!x509::name_matches(b"*.chatcomputer.test", b"chatcomputer.test"));
    assert!(!x509::name_matches(b"*.test", b"chatcomputer.test"));
}

#[test]
fn the_wrong_name_an_unknown_authority_and_an_expired_certificate_are_refused() {
    let dir = pki("refuse", "p256", Some("p256"), "p256", "DNS:localhost");
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    let roots = trusting(&dir);
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    assert_eq!(talk(server.port, "api.anthropic.com", roots, now(), b"").err(), Some(Error::Certificate(Problem::WrongName)));
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    let other = pki("other", "p256", None, "p256", "DNS:localhost");
    assert_eq!(talk(server.port, "localhost", trusting(&other), now(), b"").err(), Some(Error::Certificate(Problem::Untrusted)));
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    assert_eq!(talk(server.port, "localhost", roots, now() + 60 * 86_400, b"").err(), Some(Error::Certificate(Problem::Expired)));
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    assert_eq!(talk(server.port, "localhost", roots, now() - 86_400, b"").err(), Some(Error::Certificate(Problem::Expired)));
    // Mozilla's authorities don't vouch for a test certificate.
    let server = serve(&dir, "TLS_AES_128_GCM_SHA256");
    let mozilla: &'static [Cert<'static>] = Box::leak(crate::roots().into_boxed_slice());
    assert_eq!(talk(server.port, "localhost", mozilla, now(), b"").err(), Some(Error::Certificate(Problem::Untrusted)));
}

#[test]
fn a_forged_certificate_signature_is_caught() {
    let dir = pki("forged", "p256", None, "p256", "DNS:localhost");
    let leaf = pem_to_der(&dir.join("leaf.pem"));
    let roots = trusting(&dir);
    assert!(x509::verify(&[&leaf], "localhost", now(), roots).is_ok());
    // A byte of the signed part changed (the serial number): the signature no longer matches.
    let mut forged = leaf.clone();
    let serial_at = 15;
    forged[serial_at] ^= 1;
    assert!(matches!(x509::verify(&[&forged], "localhost", now(), roots), Err(Problem::BadSignature) | Err(Problem::Untrusted) | Err(Problem::Damaged)));
    // Not an authority: a server certificate can't vouch for another.
    let other = pki("forged2", "p256", None, "p256", "DNS:localhost");
    let leaf2 = pem_to_der(&other.join("leaf.pem"));
    assert!(x509::verify(&[&leaf2, &leaf], "localhost", now(), roots).is_err());
}

#[test]
fn a_clock_hours_off_passes_only_when_that_much_error_is_allowed() {
    let dir = pki("clock", "p256", None, "p256", "DNS:localhost");
    let leaf = pem_to_der(&dir.join("leaf.pem"));
    let roots = trusting(&dir);
    // Ten hours before the certificate was made: a clock in local time, east of UTC, read as UTC.
    let early = now() - 10 * 3600;
    assert_eq!(x509::verify(&[&leaf], "localhost", early, roots).err(), Some(Problem::Expired));
    assert!(x509::verify_around(&[&leaf], "localhost", early, 14 * 3600, roots).is_ok());
    assert_eq!(x509::verify_around(&[&leaf], "localhost", now() - 20 * 3600, 14 * 3600, roots).err(), Some(Problem::Expired));
    assert_eq!(x509::verify_around(&[&leaf], "localhost", now() + 31 * 86_400, 14 * 3600, roots).err(), Some(Problem::Expired));
}

/// The chain api.anthropic.com sent on 21 October 2026, fetched outside this computer's network (whose proxy
/// re-encrypts everything): its certificate, from Google Trust Services' WE1; WE1, from GTS Root R4; and GTS Root R4
/// signed by GlobalSign as well. It must check out against Mozilla's authorities at the time it was fetched, and
/// not for another name, after it expires, or without WE1.
#[test]
fn the_real_api_anthropic_com_chain_checks_out_against_mozillas_authorities() {
    let chain: [&[u8]; 3] = [
        include_bytes!("testdata/api.anthropic.com/0.der"),
        include_bytes!("testdata/api.anthropic.com/1.der"),
        include_bytes!("testdata/api.anthropic.com/2.der"),
    ];
    let mozilla = crate::roots();
    let fetched = 1_792_546_958;
    let key = x509::verify(&chain, "api.anthropic.com", fetched, &mozilla).expect("the real chain");
    assert!(matches!(key, x509::Key::Ec { .. }));
    assert!(x509::verify(&chain[..2], "api.anthropic.com", fetched, &mozilla).is_ok());
    assert_eq!(x509::verify(&chain, "anthropic.com", fetched, &mozilla).err(), Some(Problem::WrongName));
    assert_eq!(x509::verify(&chain, "api.anthropic.com", 1_797_800_000, &mozilla).err(), Some(Problem::Expired));
    assert_eq!(x509::verify(&chain[..1], "api.anthropic.com", fetched, &mozilla).err(), Some(Problem::Untrusted));
    // And with a clock that keeps local time somewhere, read as UTC.
    assert!(x509::verify_around(&chain, "api.anthropic.com", fetched + 13 * 3600, 14 * 3600, &mozilla).is_ok());
}

/// Another TLS 1.3 server than OpenSSL's: the network proxy of the computer this runs on, if it has one that
/// re-encrypts (HTTPS_PROXY, CA_BUNDLE: its own authority), asked through it for Claude's API. Run by hand:
///     CA_BUNDLE=/path/to/bundle.crt cargo test -p tls -- --ignored through_a_proxy
#[test]
#[ignore]
fn through_a_proxy() {
    let proxy = std::env::var("HTTPS_PROXY").expect("HTTPS_PROXY");
    let addr = proxy.trim_start_matches("http://").trim_end_matches('/').to_string();
    let bundle = std::fs::read_to_string(std::env::var("CA_BUNDLE").expect("CA_BUNDLE")).unwrap();
    let mut roots = Vec::new();
    for part in bundle.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let b64: String = part.split("-----END CERTIFICATE-----").next().unwrap().split_whitespace().collect();
        let out = Command::new("base64").arg("-d").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().and_then(|mut c| {
            c.stdin.take().unwrap().write_all(b64.as_bytes())?;
            c.wait_with_output()
        }).unwrap();
        let der: &'static [u8] = Box::leak(out.stdout.into_boxed_slice());
        if let Ok(c) = x509::parse_root(der) {
            roots.push(c);
        }
    }
    let roots: &'static [Cert<'static>] = Box::leak(roots.into_boxed_slice());
    let mut sock = TcpStream::connect(&addr).unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    sock.write_all(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        sock.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&head).contains(" 200"), "{}", String::from_utf8_lossy(&head));
    let mut c = Client::new("api.anthropic.com", now(), roots, &mut urandom);
    sock.write_all(&c.outgoing()).unwrap();
    let mut buf = vec![0u8; 32768];
    let mut got = Vec::new();
    let mut sent = false;
    while !c.finished() && c.error().is_none() {
        if c.is_open() && !sent {
            c.write(b"GET /v1/models HTTP/1.1\r\nHost: api.anthropic.com\r\nanthropic-version: 2023-06-01\r\nConnection: close\r\n\r\n");
            sent = true;
        }
        let out = c.outgoing();
        if !out.is_empty() {
            sock.write_all(&out).unwrap();
        }
        let n = match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        c.incoming(&buf[..n]);
        let n = c.read(&mut buf);
        got.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&got);
    eprintln!("suite {:?}, alpn {:?}, error {:?}\n{}", c.suite(), c.alpn().map(String::from_utf8_lossy), c.error(), &text[..text.len().min(600)]);
    assert!(text.starts_with("HTTP/1.1 401") || text.starts_with("HTTP/1.1 4"), "{text}");
}
