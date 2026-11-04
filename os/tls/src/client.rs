//! TLS 1.3 (RFC 8446), the client side: the handshake (X25519 for the shared secret; the server's certificate
//! chain and its signature over the handshake checked; both Finished messages), then records both ways,
//! encrypted with AES-128-GCM, AES-256-GCM or ChaCha20-Poly1305, whichever the server picks.
//!
//! It knows nothing of sockets: bytes from the server go in (`incoming`), bytes for the server come out
//! (`outgoing`), and the program's own data goes in and out with `write` and `read`.

use crate::x509::{self, Cert, Key, Problem};
use crypto::chacha20poly1305::ChaCha20Poly1305;
use crypto::ecdsa::Curve;
use crypto::gcm::AesGcm;
use crypto::{hkdf_expand, hkdf_extract, hmac, x25519, Hash, Sha256, Sha384};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The server's certificate didn't pass.
    Certificate(Problem),
    /// The server sent an alert (its code, RFC 8446 6.2).
    Alert(u8),
    /// The server broke the protocol.
    Protocol(&'static str),
    /// The server wants something Chat Computer doesn't do.
    Unsupported(&'static str),
    /// A record that didn't decrypt: damaged, or not from the server.
    BadRecord,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Certificate(p) => write!(f, "{p}"),
            Error::Alert(a) => write!(f, "the server ended the connection (TLS alert {a})"),
            Error::Protocol(why) => write!(f, "the server broke the TLS protocol: {why}"),
            Error::Unsupported(what) => write!(f, "the server wants {what}, which Chat Computer doesn't do"),
            Error::BadRecord => f.write_str("a message from the server was damaged or forged"),
        }
    }
}

impl Error {
    /// The alert we send the server about it.
    fn alert(self) -> u8 {
        match self {
            Error::Certificate(Problem::Expired) => 45,
            Error::Certificate(Problem::Untrusted) => 48,
            Error::Certificate(Problem::Unsupported) => 43,
            Error::Certificate(_) => 42,
            Error::Alert(_) => 0,
            Error::Protocol(_) => 10,
            Error::Unsupported(_) => 40,
            Error::BadRecord => 20,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Suite {
    Aes128Gcm,
    Aes256Gcm,
    ChaCha20Poly1305,
}

impl Suite {
    fn from_code(c: u16) -> Option<Suite> {
        match c {
            0x1301 => Some(Suite::Aes128Gcm),
            0x1302 => Some(Suite::Aes256Gcm),
            0x1303 => Some(Suite::ChaCha20Poly1305),
            _ => None,
        }
    }

    fn code(self) -> u16 {
        match self {
            Suite::Aes128Gcm => 0x1301,
            Suite::Aes256Gcm => 0x1302,
            Suite::ChaCha20Poly1305 => 0x1303,
        }
    }

    fn sha384(self) -> bool {
        self == Suite::Aes256Gcm
    }

    fn hash_len(self) -> usize {
        if self.sha384() { 48 } else { 32 }
    }

    fn hash(self, data: &[u8]) -> Vec<u8> {
        if self.sha384() { Sha384::digest(data).to_vec() } else { Sha256::digest(data).to_vec() }
    }

    fn extract(self, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
        if self.sha384() { hkdf_extract::<Sha384>(salt, ikm).to_vec() } else { hkdf_extract::<Sha256>(salt, ikm).to_vec() }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        if self.sha384() { hmac::<Sha384>(key, data).to_vec() } else { hmac::<Sha256>(key, data).to_vec() }
    }

    /// HKDF-Expand-Label (RFC 8446 7.1).
    fn expand_label(self, secret: &[u8], label: &str, context: &[u8], len: usize) -> Vec<u8> {
        let mut info = Vec::with_capacity(4 + 6 + label.len() + context.len());
        info.extend_from_slice(&(len as u16).to_be_bytes());
        info.push((6 + label.len()) as u8);
        info.extend_from_slice(b"tls13 ");
        info.extend_from_slice(label.as_bytes());
        info.push(context.len() as u8);
        info.extend_from_slice(context);
        let mut out = vec![0u8; len];
        if self.sha384() { hkdf_expand::<Sha384>(secret, &info, &mut out) } else { hkdf_expand::<Sha256>(secret, &info, &mut out) }
        out
    }

    fn derive(self, secret: &[u8], label: &str, transcript_hash: &[u8]) -> Vec<u8> {
        self.expand_label(secret, label, transcript_hash, self.hash_len())
    }
}

#[allow(clippy::large_enum_variant)]
enum Aead {
    Gcm(AesGcm),
    ChaCha(ChaCha20Poly1305),
}

/// One direction's record protection: its cipher, its IV and how many records it has done.
struct Protection {
    aead: Aead,
    iv: [u8; 12],
    seq: u64,
}

impl Protection {
    fn new(suite: Suite, secret: &[u8]) -> Protection {
        let key = suite.expand_label(secret, "key", &[], if suite == Suite::Aes128Gcm { 16 } else { 32 });
        let mut iv = [0u8; 12];
        iv.copy_from_slice(&suite.expand_label(secret, "iv", &[], 12));
        let aead = if suite == Suite::ChaCha20Poly1305 { Aead::ChaCha(ChaCha20Poly1305::new(&key)) } else { Aead::Gcm(AesGcm::new(&key)) };
        Protection { aead, iv, seq: 0 }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        for (b, s) in n[4..].iter_mut().zip(self.seq.to_be_bytes()) {
            *b ^= s;
        }
        n
    }

    /// One record: `content`, then its type, encrypted, behind its header.
    fn seal(&mut self, kind: u8, content: &[u8]) -> Vec<u8> {
        let mut inner = Vec::with_capacity(content.len() + 1 + 16 + 5);
        inner.extend_from_slice(content);
        inner.push(kind);
        let len = inner.len() + 16;
        let header = [23, 3, 3, (len >> 8) as u8, len as u8];
        let nonce = self.nonce();
        let tag = match &self.aead {
            Aead::Gcm(g) => g.seal(&nonce, &header, &mut inner),
            Aead::ChaCha(c) => c.seal(&nonce, &header, &mut inner),
        };
        self.seq += 1;
        let mut record = header.to_vec();
        record.extend_from_slice(&inner);
        record.extend_from_slice(&tag);
        record
    }

    /// Decrypts a record's body: its type and content.
    fn open(&mut self, header: &[u8], body: &[u8]) -> Option<(u8, Vec<u8>)> {
        if body.len() < 17 {
            return None;
        }
        let (data, tag) = body.split_at(body.len() - 16);
        let mut data = data.to_vec();
        let nonce = self.nonce();
        let ok = match &self.aead {
            Aead::Gcm(g) => g.open(&nonce, header, &mut data, tag),
            Aead::ChaCha(c) => c.open(&nonce, header, &mut data, tag),
        };
        if !ok {
            return None;
        }
        self.seq += 1;
        // The type is the last byte that isn't padding (zeros).
        let end = data.iter().rposition(|&b| b != 0)?;
        let kind = data[end];
        data.truncate(end);
        Some((kind, data))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    ServerHello,
    EncryptedExtensions,
    Certificate,
    CertificateVerify,
    Finished,
    Open,
    Closed,
}

#[allow(clippy::large_enum_variant)]
enum ServerKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    Ec { curve: Curve, point: Vec<u8> },
}

const HRR: [u8; 32] = [
    0xCF, 0x21, 0xAD, 0x74, 0xE5, 0x9A, 0x61, 0x11, 0xBE, 0x1D, 0x8C, 0x02, 0x1E, 0x65, 0xB8, 0x91, 0xC2, 0xA2, 0x11, 0x16, 0x7A, 0xBB,
    0x8C, 0x5E, 0x07, 0x9E, 0x09, 0xE2, 0xC8, 0xA8, 0x33, 0x9C,
];
const MAX_RECORD: usize = 16384;
const MAX_HANDSHAKE: usize = 128 * 1024;

pub struct Client {
    host: String,
    now: i64,
    /// How far off `now` may be, in seconds.
    clock_error: i64,
    roots: &'static [Cert<'static>],
    step: Step,
    private: [u8; 32],
    session_id: [u8; 32],
    transcript: Vec<u8>,
    suite: Option<Suite>,
    /// The ciphers offered, in the order preferred.
    offered: Vec<Suite>,
    handshake_secret: Vec<u8>,
    shared: Option<Vec<u8>>,
    client_secret: Vec<u8>,
    server_secret: Vec<u8>,
    read: Option<Protection>,
    write: Option<Protection>,
    inbuf: Vec<u8>,
    hsbuf: Vec<u8>,
    out: Vec<u8>,
    app: VecDeque<u8>,
    server_key: Option<ServerKey>,
    alpn: Option<Vec<u8>>,
    error: Option<Error>,
    peer_closed: bool,
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { core::ptr::write_volatile(&mut self.private, [0; 32]) };
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// A list of extensions: (type, data) each.
fn extensions(b: &[u8]) -> Option<Vec<(u16, &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let kind = u16_at(b, at)?;
        let len = u16_at(b, at + 2)? as usize;
        out.push((kind, b.get(at + 4..at + 4 + len)?));
        at += 4 + len;
    }
    Some(out)
}

impl Client {
    /// Starts a connection to `host` (the name checked against its certificate), at `now` (seconds since 1970),
    /// trusting `roots`. `random` fills buffers with hard-to-guess bytes. ClientHello is then in `outgoing`.
    pub fn new(host: &str, now: i64, roots: &'static [Cert<'static>], random: &mut dyn FnMut(&mut [u8])) -> Client {
        Client::offering(host, now, roots, random, &[Suite::ChaCha20Poly1305, Suite::Aes128Gcm, Suite::Aes256Gcm])
    }

    /// The same, offering only these ciphers, in this order (a download that's to be fast offers ChaCha20-Poly1305
    /// alone: AES here is the constant-time kind written without the processor's AES instructions, and slow).
    pub fn offering(host: &str, now: i64, roots: &'static [Cert<'static>], random: &mut dyn FnMut(&mut [u8]),
                    suites: &[Suite]) -> Client {
        let mut c = Client {
            host: String::from(host),
            now,
            clock_error: 0,
            roots,
            step: Step::ServerHello,
            private: [0; 32],
            session_id: [0; 32],
            transcript: Vec::new(),
            suite: None,
            offered: suites.to_vec(),
            handshake_secret: Vec::new(),
            shared: None,
            client_secret: Vec::new(),
            server_secret: Vec::new(),
            read: None,
            write: None,
            inbuf: Vec::new(),
            hsbuf: Vec::new(),
            out: Vec::new(),
            app: VecDeque::new(),
            server_key: None,
            alpn: None,
            error: None,
            peer_closed: false,
        };
        let mut client_random = [0u8; 32];
        random(&mut client_random);
        random(&mut c.private);
        random(&mut c.session_id);
        let hello = c.client_hello(&client_random);
        c.transcript.extend_from_slice(&hello);
        c.plain_record(22, &hello, 0x0301);
        c
    }

    fn client_hello(&self, random: &[u8; 32]) -> Vec<u8> {
        let mut body = vec![3, 3];
        body.extend_from_slice(random);
        body.push(32);
        body.extend_from_slice(&self.session_id);
        body.extend_from_slice(&((2 * self.offered.len()) as u16).to_be_bytes());
        for s in &self.offered {
            body.extend_from_slice(&s.code().to_be_bytes());
        }
        body.extend_from_slice(&[1, 0]);
        let mut ext = Vec::new();
        let mut add = |kind: u16, data: &[u8]| {
            ext.extend_from_slice(&kind.to_be_bytes());
            ext.extend_from_slice(&(data.len() as u16).to_be_bytes());
            ext.extend_from_slice(data);
        };
        let host = self.host.as_bytes();
        let mut sni = Vec::new();
        sni.extend_from_slice(&((host.len() + 3) as u16).to_be_bytes());
        sni.push(0);
        sni.extend_from_slice(&(host.len() as u16).to_be_bytes());
        sni.extend_from_slice(host);
        add(0, &sni);
        add(10, &[0, 2, 0x00, 0x1D]); // groups: X25519
        add(13, &[0, 16, 4, 3, 5, 3, 8, 4, 8, 5, 8, 6, 4, 1, 5, 1, 6, 1]); // signatures
        add(16, &[0, 9, 8, b'h', b't', b't', b'p', b'/', b'1', b'.', b'1']); // ALPN: HTTP/1.1
        add(43, &[2, 3, 4]); // TLS 1.3 only
        let public = x25519::public(&self.private);
        let mut share = vec![0, 36, 0x00, 0x1D, 0, 32];
        share.extend_from_slice(&public);
        add(51, &share);
        body.extend_from_slice(&(ext.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext);
        let mut m = vec![1, 0, 0, 0];
        m[1..4].copy_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        m.extend_from_slice(&body);
        m
    }

    fn plain_record(&mut self, kind: u8, data: &[u8], version: u16) {
        for chunk in data.chunks(MAX_RECORD) {
            self.out.push(kind);
            self.out.extend_from_slice(&version.to_be_bytes());
            self.out.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
            self.out.extend_from_slice(chunk);
        }
    }

    fn sealed(&mut self, kind: u8, data: &[u8]) {
        let Some(w) = self.write.as_mut() else { return };
        for chunk in data.chunks(MAX_RECORD) {
            let r = w.seal(kind, chunk);
            self.out.extend_from_slice(&r);
        }
    }

    /// Says the time given to `new` is only known to within `seconds` (before the server's certificate arrives):
    /// certificates valid at any moment in that span are accepted.
    pub fn allow_clock_error(&mut self, seconds: i64) {
        self.clock_error = seconds.max(0);
    }

    /// Bytes to send to the server.
    pub fn outgoing(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    /// Whether the handshake is done and data can go both ways.
    pub fn is_open(&self) -> bool {
        self.step == Step::Open && self.error.is_none()
    }

    /// Why it failed, if it did.
    pub fn error(&self) -> Option<Error> {
        self.error
    }

    /// Whether the server has said it's done (close_notify), and everything it sent has been read.
    pub fn finished(&self) -> bool {
        (self.peer_closed || self.step == Step::Closed) && self.app.is_empty()
    }

    pub fn suite(&self) -> Option<Suite> {
        self.suite
    }

    /// The application protocol the server chose ("http/1.1"), if it said.
    pub fn alpn(&self) -> Option<&[u8]> {
        self.alpn.as_deref()
    }

    /// Hands over data to send, encrypted, once open. Returns false if it isn't open.
    pub fn write(&mut self, data: &[u8]) -> bool {
        if !self.is_open() {
            return false;
        }
        self.sealed(23, data);
        true
    }

    /// The server's data that has arrived: how many bytes went into `buf`.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.app.len());
        for (b, v) in buf.iter_mut().zip(self.app.drain(..n)) {
            *b = v;
        }
        n
    }

    /// Says goodbye (close_notify).
    pub fn close(&mut self) {
        if self.write.is_some() && self.step != Step::Closed {
            self.sealed(21, &[1, 0]);
        }
        self.step = Step::Closed;
    }

    fn fail(&mut self, e: Error) {
        if self.error.is_none() {
            self.error = Some(e);
            let alert = e.alert();
            if alert != 0 {
                if self.write.is_some() {
                    self.sealed(21, &[2, alert]);
                } else {
                    self.plain_record(21, &[2, alert], 0x0303);
                }
            }
        }
        self.step = Step::Closed;
    }

    /// Bytes from the server.
    pub fn incoming(&mut self, data: &[u8]) {
        if self.error.is_some() || self.step == Step::Closed && self.peer_closed {
            return;
        }
        self.inbuf.extend_from_slice(data);
        let mut at = 0;
        while self.error.is_none() && self.inbuf.len() >= at + 5 {
            let kind = self.inbuf[at];
            let len = u16::from_be_bytes([self.inbuf[at + 3], self.inbuf[at + 4]]) as usize;
            if len > MAX_RECORD + 256 || self.inbuf[at + 1] != 3 {
                self.fail(Error::Protocol("a record too big, or not TLS"));
                break;
            }
            if self.inbuf.len() < at + 5 + len {
                break;
            }
            let header: Vec<u8> = self.inbuf[at..at + 5].to_vec();
            let body: Vec<u8> = self.inbuf[at + 5..at + 5 + len].to_vec();
            at += 5 + len;
            if let Err(e) = self.record(kind, &header, &body) {
                self.fail(e);
            }
        }
        self.inbuf.drain(..at.min(self.inbuf.len()));
    }

    fn record(&mut self, kind: u8, header: &[u8], body: &[u8]) -> Result<(), Error> {
        match kind {
            20 => {
                // ChangeCipherSpec, sent for middleboxes' sake during the handshake: ignored.
                if self.step == Step::Open || body != [1] {
                    return Err(Error::Protocol("an unexpected ChangeCipherSpec"));
                }
                Ok(())
            }
            21 if self.read.is_none() => self.alert(body),
            22 if self.read.is_none() => {
                self.hsbuf.extend_from_slice(body);
                self.handshake()
            }
            23 => {
                let read = self.read.as_mut().ok_or(Error::Protocol("encrypted data before the keys"))?;
                let (inner, content) = read.open(header, body).ok_or(Error::BadRecord)?;
                match inner {
                    21 => self.alert(&content),
                    22 => {
                        if content.is_empty() {
                            return Err(Error::Protocol("an empty handshake record"));
                        }
                        self.hsbuf.extend_from_slice(&content);
                        if self.hsbuf.len() > MAX_HANDSHAKE {
                            return Err(Error::Protocol("a handshake message too big"));
                        }
                        self.handshake()
                    }
                    23 if self.step == Step::Open => {
                        self.app.extend(content);
                        Ok(())
                    }
                    _ => Err(Error::Protocol("an unexpected record")),
                }
            }
            _ => Err(Error::Protocol("an unexpected record")),
        }
    }

    fn alert(&mut self, body: &[u8]) -> Result<(), Error> {
        match body {
            [_, 0] => {
                self.peer_closed = true;
                self.step = Step::Closed;
                Ok(())
            }
            [_, 90] => Ok(()), // user_canceled: a close_notify follows
            [_, desc] => Err(Error::Alert(*desc)),
            _ => Err(Error::Protocol("a damaged alert")),
        }
    }

    fn handshake(&mut self) -> Result<(), Error> {
        while self.hsbuf.len() >= 4 {
            let len = u32::from_be_bytes([0, self.hsbuf[1], self.hsbuf[2], self.hsbuf[3]]) as usize;
            if len > MAX_HANDSHAKE {
                return Err(Error::Protocol("a handshake message too big"));
            }
            if self.hsbuf.len() < 4 + len {
                break;
            }
            let whole: Vec<u8> = self.hsbuf.drain(..4 + len).collect();
            self.message(whole[0], &whole)?;
        }
        Ok(())
    }

    fn th(&self) -> Vec<u8> {
        self.suite.map_or_else(Vec::new, |s| s.hash(&self.transcript))
    }

    fn message(&mut self, kind: u8, whole: &[u8]) -> Result<(), Error> {
        let body = &whole[4..];
        match (self.step, kind) {
            (Step::ServerHello, 2) => {
                self.server_hello(body)?;
                self.transcript.extend_from_slice(whole);
                let suite = self.suite.unwrap();
                let shared = self.shared.take().ok_or(Error::Protocol("no key share"))?;
                let zeros = vec![0u8; suite.hash_len()];
                let early = suite.extract(&[], &zeros);
                let derived = suite.derive(&early, "derived", &suite.hash(&[]));
                self.handshake_secret = suite.extract(&derived, &shared);
                let th = self.th();
                self.client_secret = suite.derive(&self.handshake_secret, "c hs traffic", &th);
                self.server_secret = suite.derive(&self.handshake_secret, "s hs traffic", &th);
                self.read = Some(Protection::new(suite, &self.server_secret));
                if !self.hsbuf.is_empty() {
                    return Err(Error::Protocol("handshake messages across the change of keys"));
                }
                self.step = Step::EncryptedExtensions;
                Ok(())
            }
            (Step::EncryptedExtensions, 8) => {
                let len = u16_at(body, 0).ok_or(Error::Protocol("damaged EncryptedExtensions"))? as usize;
                let list = extensions(body.get(2..2 + len).ok_or(Error::Protocol("damaged EncryptedExtensions"))?)
                    .ok_or(Error::Protocol("damaged EncryptedExtensions"))?;
                for (kind, data) in list {
                    if kind == 16 {
                        // ALPN: one protocol, one we offered.
                        let p = data.get(3..).ok_or(Error::Protocol("damaged ALPN"))?;
                        if p != b"http/1.1" {
                            return Err(Error::Protocol("a protocol we didn't offer"));
                        }
                        self.alpn = Some(p.to_vec());
                    }
                }
                self.transcript.extend_from_slice(whole);
                self.step = Step::Certificate;
                Ok(())
            }
            (Step::Certificate, 13) => Err(Error::Unsupported("a client certificate")),
            (Step::Certificate, 11) => {
                let key = self.certificate(body)?;
                self.server_key = Some(key);
                self.transcript.extend_from_slice(whole);
                self.step = Step::CertificateVerify;
                Ok(())
            }
            (Step::CertificateVerify, 15) => {
                let scheme = u16_at(body, 0).ok_or(Error::Protocol("damaged CertificateVerify"))?;
                let len = u16_at(body, 2).ok_or(Error::Protocol("damaged CertificateVerify"))? as usize;
                let sig = body.get(4..4 + len).ok_or(Error::Protocol("damaged CertificateVerify"))?;
                let mut signed = vec![0x20u8; 64];
                signed.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
                signed.extend_from_slice(&self.th());
                if !self.check_signature(scheme, &signed, sig) {
                    return Err(Error::Certificate(Problem::BadSignature));
                }
                self.transcript.extend_from_slice(whole);
                self.step = Step::Finished;
                Ok(())
            }
            (Step::Finished, 20) => {
                let suite = self.suite.unwrap();
                let key = suite.expand_label(&self.server_secret, "finished", &[], suite.hash_len());
                let want = suite.hmac(&key, &self.th());
                if !crypto::equal(&want, body) {
                    return Err(Error::Protocol("the server's Finished is wrong"));
                }
                self.transcript.extend_from_slice(whole);
                // The application's keys, from everything up to the server's Finished.
                let th = self.th();
                let zeros = vec![0u8; suite.hash_len()];
                let derived = suite.derive(&self.handshake_secret, "derived", &suite.hash(&[]));
                let master = suite.extract(&derived, &zeros);
                let client_app = suite.derive(&master, "c ap traffic", &th);
                let server_app = suite.derive(&master, "s ap traffic", &th);
                // Our Finished, under the handshake keys (after a ChangeCipherSpec, for middleboxes).
                let ckey = suite.expand_label(&self.client_secret, "finished", &[], suite.hash_len());
                let verify = suite.hmac(&ckey, &th);
                let mut fin = vec![20, 0, 0, verify.len() as u8];
                fin.extend_from_slice(&verify);
                self.plain_record(20, &[1], 0x0303);
                self.write = Some(Protection::new(suite, &self.client_secret));
                self.sealed(22, &fin);
                self.transcript.extend_from_slice(&fin);
                self.read = Some(Protection::new(suite, &server_app));
                self.write = Some(Protection::new(suite, &client_app));
                self.client_secret = client_app;
                self.server_secret = server_app;
                self.handshake_secret.clear();
                self.step = Step::Open;
                Ok(())
            }
            (Step::Open, 4) => Ok(()), // NewSessionTicket: Chat Computer doesn't resume sessions
            (Step::Open, 24) => {
                // KeyUpdate: their keys move on; if they ask, ours too.
                let suite = self.suite.unwrap();
                let request = *body.first().ok_or(Error::Protocol("damaged KeyUpdate"))?;
                self.server_secret = suite.expand_label(&self.server_secret, "traffic upd", &[], suite.hash_len());
                self.read = Some(Protection::new(suite, &self.server_secret));
                if request == 1 {
                    self.sealed(22, &[24, 0, 0, 1, 0]);
                    self.client_secret = suite.expand_label(&self.client_secret, "traffic upd", &[], suite.hash_len());
                    self.write = Some(Protection::new(suite, &self.client_secret));
                }
                Ok(())
            }
            _ => Err(Error::Protocol("an unexpected handshake message")),
        }
    }
}

impl Client {
    fn server_hello(&mut self, b: &[u8]) -> Result<(), Error> {
        let bad = Error::Protocol("a damaged ServerHello");
        if b.len() < 38 {
            return Err(bad);
        }
        if b[2..34] == HRR {
            return Err(Error::Unsupported("a key exchange other than X25519"));
        }
        let sid_len = b[34] as usize;
        let sid = b.get(35..35 + sid_len).ok_or(bad)?;
        if sid != self.session_id {
            return Err(Error::Protocol("the session id came back different"));
        }
        let at = 35 + sid_len;
        let suite = Suite::from_code(u16_at(b, at).ok_or(bad)?)
            .filter(|s| self.offered.contains(s))
            .ok_or(Error::Protocol("a cipher we didn't offer"))?;
        if b.get(at + 2) != Some(&0) {
            return Err(bad);
        }
        let len = u16_at(b, at + 3).ok_or(bad)? as usize;
        let list = extensions(b.get(at + 5..at + 5 + len).ok_or(bad)?).ok_or(bad)?;
        let mut version = false;
        let mut theirs = None;
        for (kind, data) in list {
            match kind {
                43 => version = data == [3, 4],
                51 => {
                    if u16_at(data, 0) != Some(0x001D) || u16_at(data, 2) != Some(32) || data.len() != 36 {
                        return Err(Error::Unsupported("a key exchange other than X25519"));
                    }
                    let mut k = [0u8; 32];
                    k.copy_from_slice(&data[4..36]);
                    theirs = Some(k);
                }
                _ => {}
            }
        }
        if !version {
            return Err(Error::Unsupported("a TLS version older than 1.3"));
        }
        let theirs = theirs.ok_or(Error::Protocol("no key share"))?;
        let shared = x25519::shared(&self.private, &theirs).ok_or(Error::Protocol("a bad key share"))?;
        self.suite = Some(suite);
        self.shared = Some(shared.to_vec());
        Ok(())
    }

    fn certificate(&mut self, b: &[u8]) -> Result<ServerKey, Error> {
        let bad = Error::Protocol("a damaged Certificate");
        let ctx = *b.first().ok_or(bad)? as usize;
        let mut at = 1 + ctx;
        let total = u32::from_be_bytes([0, *b.get(at).ok_or(bad)?, *b.get(at + 1).ok_or(bad)?, *b.get(at + 2).ok_or(bad)?]) as usize;
        at += 3;
        let end = at + total;
        if end != b.len() {
            return Err(bad);
        }
        let mut chain: Vec<&[u8]> = Vec::new();
        while at < end {
            let len = u32::from_be_bytes([0, *b.get(at).ok_or(bad)?, *b.get(at + 1).ok_or(bad)?, *b.get(at + 2).ok_or(bad)?]) as usize;
            chain.push(b.get(at + 3..at + 3 + len).ok_or(bad)?);
            at += 3 + len;
            let elen = u16_at(b, at).ok_or(bad)? as usize;
            at += 2 + elen;
        }
        let key = x509::verify_around(&chain, &self.host, self.now, self.clock_error, self.roots).map_err(Error::Certificate)?;
        Ok(match key {
            Key::Rsa { n, e } => ServerKey::Rsa { n: n.to_vec(), e: e.to_vec() },
            Key::Ec { curve, point } => ServerKey::Ec { curve, point: point.to_vec() },
        })
    }

    fn check_signature(&self, scheme: u16, signed: &[u8], sig: &[u8]) -> bool {
        use crypto::rsa::{verify_pss, Digest};
        match (scheme, &self.server_key) {
            (0x0403, Some(ServerKey::Ec { curve: Curve::P256, point })) => x509::ecdsa_der(Curve::P256, point, &Sha256::digest(signed), sig),
            (0x0503, Some(ServerKey::Ec { curve: Curve::P384, point })) => x509::ecdsa_der(Curve::P384, point, &Sha384::digest(signed), sig),
            (0x0804, Some(ServerKey::Rsa { n, e })) => verify_pss(n, e, Digest::Sha256, signed, sig),
            (0x0805, Some(ServerKey::Rsa { n, e })) => verify_pss(n, e, Digest::Sha384, signed, sig),
            (0x0806, Some(ServerKey::Rsa { n, e })) => verify_pss(n, e, Digest::Sha512, signed, sig),
            _ => false,
        }
    }
}
