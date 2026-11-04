//! Certificates (X.509, RFC 5280) and checking a server's: its name, its dates, and a chain of signatures up to
//! one of the authorities Chat Computer trusts (roots.rs). Only what the web's certificates use: RSA and
//! ECDSA (P-256, P-384) keys and signatures with SHA-256, -384 or -512, DNS names. An extension marked critical
//! that isn't understood makes a certificate unusable, as RFC 5280 says.

use crate::der::{self, Der, Tlv};
use crypto::ecdsa::{self, Curve};
use crypto::rsa;
use crypto::{Hash, Sha256, Sha384, Sha512};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    /// Not a certificate that could be read.
    Damaged,
    /// Its time hasn't come, or has passed.
    Expired,
    /// It isn't for the name connected to.
    WrongName,
    /// No chain of signatures leads to a trusted authority.
    Untrusted,
    /// A signature in the chain is wrong.
    BadSignature,
    /// A certificate in the chain may not be used this way (not an authority, not for servers, too deep, a name
    /// outside what the authority may vouch for).
    NotAllowed,
    /// A key or algorithm Chat Computer doesn't handle.
    Unsupported,
}

impl core::fmt::Display for Problem {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(match self {
            Problem::Damaged => "its certificate couldn't be read",
            Problem::Expired => "its certificate has expired, or isn't valid yet (or this computer's clock is wrong)",
            Problem::WrongName => "its certificate is for another name",
            Problem::Untrusted => "its certificate isn't from an authority Chat Computer trusts",
            Problem::BadSignature => "a signature on its certificate is wrong",
            Problem::NotAllowed => "its certificate chain breaks the rules",
            Problem::Unsupported => "its certificate uses a key or algorithm Chat Computer doesn't handle",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigAlg {
    RsaSha256,
    RsaSha384,
    RsaSha512,
    EcdsaSha256,
    EcdsaSha384,
    EcdsaSha512,
}

#[derive(Clone, Copy, Debug)]
pub enum Key<'a> {
    Rsa { n: &'a [u8], e: &'a [u8] },
    Ec { curve: Curve, point: &'a [u8] },
}

#[derive(Clone, Debug)]
pub struct Cert<'a> {
    pub tbs: &'a [u8],
    pub sig_alg: SigAlg,
    pub signature: &'a [u8],
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    pub not_before: i64,
    pub not_after: i64,
    pub key: Key<'a>,
    pub ca: bool,
    pub path_len: Option<u32>,
    pub key_usage: Option<u16>,
    /// None: no extended key usage (any use); Some(whether servers' use is among them).
    pub for_servers: Option<bool>,
    pub dns_names: Vec<&'a [u8]>,
    /// Name constraints: the DNS names it may vouch for, and those it may not.
    pub permitted: Vec<&'a [u8]>,
    pub excluded: Vec<&'a [u8]>,
}

const RSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
const RSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0C];
const RSA_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0D];
const ECDSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
const ECDSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x03];
const ECDSA_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x04];
const RSA_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
const EC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const P256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
const P384: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x22];
const BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1D, 0x13];
const KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x0F];
const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1D, 0x11];
const EXT_KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x25];
const NAME_CONSTRAINTS: &[u8] = &[0x55, 0x1D, 0x1E];
const CERT_POLICIES: &[u8] = &[0x55, 0x1D, 0x20];
const SERVER_AUTH: &[u8] = &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
const ANY_EKU: &[u8] = &[0x55, 0x1D, 0x25, 0x00];

pub const KEY_CERT_SIGN: u16 = 1 << 5;
pub const DIGITAL_SIGNATURE: u16 = 1 << 0;

fn sig_alg(t: &Tlv) -> Option<SigAlg> {
    let mut r = t.reader();
    let oid = r.expect(der::OID)?.value;
    Some(match oid {
        RSA_SHA256 => SigAlg::RsaSha256,
        RSA_SHA384 => SigAlg::RsaSha384,
        RSA_SHA512 => SigAlg::RsaSha512,
        ECDSA_SHA256 => SigAlg::EcdsaSha256,
        ECDSA_SHA384 => SigAlg::EcdsaSha384,
        ECDSA_SHA512 => SigAlg::EcdsaSha512,
        _ => return None,
    })
}

fn public_key<'a>(spki: &Tlv<'a>) -> Option<Key<'a>> {
    let mut r = spki.reader();
    let alg = r.expect(der::SEQUENCE)?;
    let key = der::bits(r.expect(der::BIT_STRING)?.value)?;
    let mut a = alg.reader();
    match a.expect(der::OID)?.value {
        RSA_KEY => {
            let rsa = Der::new(key).expect(der::SEQUENCE)?;
            let mut k = rsa.reader();
            let n = der::unsigned(k.expect(der::INTEGER)?.value);
            let e = der::unsigned(k.expect(der::INTEGER)?.value);
            Some(Key::Rsa { n, e })
        }
        EC_KEY => {
            let curve = match a.expect(der::OID)?.value {
                P256 => Curve::P256,
                P384 => Curve::P384,
                _ => return None,
            };
            Some(Key::Ec { curve, point: key })
        }
        _ => None,
    }
}

/// Reads a certificate. Damaged, or using something not handled: an error saying which.
pub fn parse(data: &[u8]) -> Result<Cert<'_>, Problem> {
    parse_inner(data, false).ok_or(Problem::Damaged)?
}

/// Reads a trusted authority's own certificate: how it signed itself doesn't matter (it's trusted as it is), so
/// old ones signed with SHA-1, or of version 1, are taken.
pub fn parse_root(data: &[u8]) -> Result<Cert<'_>, Problem> {
    parse_inner(data, true).ok_or(Problem::Damaged)?
}

fn parse_inner(data: &[u8], root: bool) -> Option<Result<Cert<'_>, Problem>> {
    let mut outer = Der::new(data);
    let cert = outer.expect(der::SEQUENCE)?;
    if !outer.is_empty() {
        return None;
    }
    let mut c = cert.reader();
    let tbs = c.expect(der::SEQUENCE)?;
    let alg = c.expect(der::SEQUENCE)?;
    let signature = der::bits(c.expect(der::BIT_STRING)?.value)?;
    let mut t = tbs.reader();
    if let Some(v) = t.optional(0xA0) {
        let version = v.reader().expect(der::INTEGER)?;
        if version.value != [2] && !root {
            return None; // (only version 3 certificates have the extensions that make them safe to use)
        }
    } else if !root {
        return None;
    }
    t.expect(der::INTEGER)?; // serial number
    let inner_alg = t.expect(der::SEQUENCE)?;
    let issuer = t.expect(der::SEQUENCE)?;
    let validity = t.expect(der::SEQUENCE)?;
    let subject = t.expect(der::SEQUENCE)?;
    let spki = t.expect(der::SEQUENCE)?;
    let sig_alg = match (sig_alg(&alg), inner_alg.whole == alg.whole) {
        (Some(a), true) => a,
        _ if root => SigAlg::RsaSha256, // (never checked)
        _ => return Some(Err(Problem::Unsupported)),
    };
    let mut v = validity.reader();
    let not_before = der::time(&v.next()?)?;
    let not_after = der::time(&v.next()?)?;
    let Some(key) = public_key(&spki) else { return Some(Err(Problem::Unsupported)) };
    let mut cert = Cert {
        tbs: tbs.whole,
        sig_alg,
        signature,
        issuer: issuer.whole,
        subject: subject.whole,
        not_before,
        not_after,
        key,
        ca: false,
        path_len: None,
        key_usage: None,
        for_servers: None,
        dns_names: Vec::new(),
        permitted: Vec::new(),
        excluded: Vec::new(),
    };
    t.optional(0x81);
    t.optional(0x82);
    if let Some(exts) = t.optional(0xA3) {
        let mut list = exts.reader().expect(der::SEQUENCE)?.reader();
        while !list.is_empty() {
            let ext = list.expect(der::SEQUENCE)?;
            let mut e = ext.reader();
            let oid = e.expect(der::OID)?.value;
            let critical = e.optional(der::BOOLEAN).is_some_and(|b| b.value == [0xFF]);
            let value = e.expect(der::OCTET_STRING)?.value;
            let mut v = Der::new(value);
            match oid {
                BASIC_CONSTRAINTS => {
                    let mut bc = v.expect(der::SEQUENCE)?.reader();
                    cert.ca = bc.optional(der::BOOLEAN).is_some_and(|b| b.value == [0xFF]);
                    if let Some(p) = bc.optional(der::INTEGER) {
                        let p = der::unsigned(p.value);
                        cert.path_len = Some(if p.len() > 2 { u32::MAX } else { p.iter().fold(0u32, |a, &b| a << 8 | b as u32) });
                    }
                }
                KEY_USAGE => {
                    let bits = v.expect(der::BIT_STRING)?.value;
                    // The first byte counts unused bits; the flags read from the most significant bit.
                    let mut u = 0u16;
                    for (i, &b) in bits.iter().skip(1).take(2).enumerate() {
                        for j in 0..8 {
                            if b & (0x80 >> j) != 0 {
                                u |= 1 << (8 * i + j);
                            }
                        }
                    }
                    cert.key_usage = Some(u);
                }
                SUBJECT_ALT_NAME => {
                    let mut names = v.expect(der::SEQUENCE)?.reader();
                    while !names.is_empty() {
                        let n = names.next()?;
                        if n.tag == 0x82 {
                            cert.dns_names.push(n.value);
                        }
                    }
                }
                EXT_KEY_USAGE => {
                    let mut list = v.expect(der::SEQUENCE)?.reader();
                    let mut server = false;
                    while !list.is_empty() {
                        let o = list.expect(der::OID)?.value;
                        server |= o == SERVER_AUTH || o == ANY_EKU;
                    }
                    cert.for_servers = Some(server);
                }
                NAME_CONSTRAINTS => {
                    let mut nc = v.expect(der::SEQUENCE)?.reader();
                    while !nc.is_empty() {
                        let part = nc.next()?;
                        let mut subtrees = part.reader();
                        while !subtrees.is_empty() {
                            let st = subtrees.expect(der::SEQUENCE)?;
                            let base = st.reader().next()?;
                            if base.tag == 0x82 {
                                match part.tag {
                                    0xA0 => cert.permitted.push(base.value),
                                    0xA1 => cert.excluded.push(base.value),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                CERT_POLICIES => {}
                _ if critical => return Some(Err(Problem::Unsupported)),
                _ => {}
            }
        }
    }
    Some(Ok(cert))
}

/// Whether `cert` was signed by the holder of `key`.
pub fn signed_by(cert: &Cert, key: &Key) -> bool {
    match (cert.sig_alg, key) {
        (SigAlg::RsaSha256, Key::Rsa { n, e }) => rsa::verify_pkcs1(n, e, rsa::Digest::Sha256, cert.tbs, cert.signature),
        (SigAlg::RsaSha384, Key::Rsa { n, e }) => rsa::verify_pkcs1(n, e, rsa::Digest::Sha384, cert.tbs, cert.signature),
        (SigAlg::RsaSha512, Key::Rsa { n, e }) => rsa::verify_pkcs1(n, e, rsa::Digest::Sha512, cert.tbs, cert.signature),
        (alg, Key::Ec { curve, point }) => {
            let hash = match alg {
                SigAlg::EcdsaSha256 => Sha256::digest(cert.tbs),
                SigAlg::EcdsaSha384 => Sha384::digest(cert.tbs),
                SigAlg::EcdsaSha512 => Sha512::digest(cert.tbs),
                _ => return false,
            };
            ecdsa_der(*curve, point, &hash, cert.signature)
        }
        _ => false,
    }
}

/// An ECDSA signature as DER (SEQUENCE of r and s), checked.
pub fn ecdsa_der(curve: Curve, point: &[u8], hash: &[u8], sig: &[u8]) -> bool {
    let size = match curve {
        Curve::P256 => 32,
        Curve::P384 => 48,
    };
    let Some(seq) = Der::new(sig).expect(der::SEQUENCE) else { return false };
    let mut r = seq.reader();
    let (Some(rr), Some(ss)) = (r.expect(der::INTEGER), r.expect(der::INTEGER)) else { return false };
    let (rr, ss) = (der::unsigned(rr.value), der::unsigned(ss.value));
    if rr.len() > size || ss.len() > size {
        return false;
    }
    let mut rb = [0u8; 48];
    let mut sb = [0u8; 48];
    rb[size - rr.len()..size].copy_from_slice(rr);
    sb[size - ss.len()..size].copy_from_slice(ss);
    ecdsa::verify(curve, point, hash, &rb[..size], &sb[..size])
}

fn lower(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

fn eq_name(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}

/// Whether a certificate's DNS name (perhaps "*.example.com") covers `host`.
pub fn name_matches(pattern: &[u8], host: &[u8]) -> bool {
    if let Some(rest) = pattern.strip_prefix(b"*.") {
        // The wildcard is one whole label, the leftmost, and never for a bare top-level name.
        let Some(dot) = host.iter().position(|&c| c == b'.') else { return false };
        return dot > 0 && rest.contains(&b'.') && eq_name(rest, &host[dot + 1..]);
    }
    eq_name(pattern, host)
}

/// Whether `host` is inside a name constraint's subtree ("example.com" covers it and anything under it).
fn within(base: &[u8], host: &[u8]) -> bool {
    let base = base.strip_prefix(b".").unwrap_or(base);
    if base.is_empty() {
        return true;
    }
    eq_name(base, host) || (host.len() > base.len() && host[host.len() - base.len() - 1] == b'.' && eq_name(base, &host[host.len() - base.len()..]))
}

/// Checks the chain a server sent (its own certificate first) for `host`, at `now` (seconds since 1970), up to
/// one of `roots`. Returns the server's key, for checking its handshake signature.
pub fn verify<'a>(chain: &[&'a [u8]], host: &str, now: i64, roots: &[Cert]) -> Result<Key<'a>, Problem> {
    verify_around(chain, host, now, 0, roots)
}

/// `verify`, when the time is only known to within `error` seconds of `now` (a clock whose time zone isn't
/// known): each certificate must be valid at some moment in that span.
pub fn verify_around<'a>(chain: &[&'a [u8]], host: &str, now: i64, error: i64, roots: &[Cert]) -> Result<Key<'a>, Problem> {
    let error = error.max(0);
    let valid = |c: &Cert| c.not_before <= now.saturating_add(error) && now.saturating_sub(error) <= c.not_after;
    let host = host.trim_end_matches('.').as_bytes();
    let first = chain.first().ok_or(Problem::Damaged)?;
    let leaf = parse(first)?;
    if !valid(&leaf) {
        return Err(Problem::Expired);
    }
    if !leaf.dns_names.iter().any(|n| name_matches(n, host)) {
        return Err(Problem::WrongName);
    }
    if leaf.for_servers == Some(false) || leaf.key_usage.is_some_and(|u| u & DIGITAL_SIGNATURE == 0) {
        return Err(Problem::NotAllowed);
    }
    let mut others: Vec<Cert> = Vec::new();
    for c in chain.iter().skip(1).take(8) {
        if let Ok(c) = parse(c) {
            others.push(c);
        }
    }
    let mut current = leaf.clone();
    for depth in 0..8 {
        // A trusted authority that signed it: done. (One of the same name that didn't is someone else.)
        for root in roots.iter().filter(|r| r.subject == current.issuer) {
            if signed_by(&current, &root.key) {
                return if allowed(root, &leaf, depth) { Ok(leaf.key) } else { Err(Problem::NotAllowed) };
            }
        }
        // Else an authority the server sent along.
        let Some(next) = others.iter().find(|c| c.subject == current.issuer && c.subject != current.subject && signed_by(&current, &c.key)) else {
            if others.iter().any(|c| c.subject == current.issuer && c.subject != current.subject) {
                return Err(Problem::BadSignature);
            }
            return Err(Problem::Untrusted);
        };
        if !valid(next) {
            return Err(Problem::Expired);
        }
        if !next.ca || next.key_usage.is_some_and(|u| u & KEY_CERT_SIGN == 0) || next.for_servers == Some(false) || !allowed(next, &leaf, depth) {
            return Err(Problem::NotAllowed);
        }
        current = next.clone();
    }
    Err(Problem::Untrusted)
}

/// Whether authority `ca`, `depth` authorities above the server's certificate, may vouch for it: its path length,
/// and its name constraints on the server's names.
fn allowed(ca: &Cert, leaf: &Cert, depth: usize) -> bool {
    if ca.path_len.is_some_and(|p| (depth as u32) > p) {
        return false;
    }
    leaf.dns_names.iter().all(|name| {
        let name = name.strip_prefix(b"*.").unwrap_or(name);
        (ca.permitted.is_empty() || ca.permitted.iter().any(|b| within(b, name))) && !ca.excluded.iter().any(|b| within(b, name))
    })
}
