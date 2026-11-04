//! Chat Computer's own TLS 1.3 client, with its own certificate checks: what lets it talk to Claude's API, and to
//! any other site, privately and knowing who it's talking to. No code from other projects (its cryptography is
//! Chat Computer's own `crypto` crate). The certificate authorities it trusts are Mozilla's (roots.txt).
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod client;
pub mod der;
pub mod x509;
#[cfg(test)]
mod tests;

use alloc::vec::Vec;

pub use client::{Client, Error, Suite};
pub use x509::{Cert, Problem};

/// Mozilla's certificate authorities (made by make_roots.py), one after another.
pub static ROOTS: &[u8] = include_bytes!("roots.der");

/// The trusted authorities, read (those of a kind Chat Computer can't use are left out).
pub fn roots() -> Vec<Cert<'static>> {
    let mut out = Vec::new();
    let mut d = der::Der::new(ROOTS);
    while let Some(t) = d.next() {
        if let Ok(c) = x509::parse_root(t.whole) {
            out.push(c);
        }
    }
    out
}
