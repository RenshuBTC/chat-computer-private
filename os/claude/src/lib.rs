//! Talking to Claude from Chat Computer's own kernel: its Messages API, streamed (api.rs), over HTTP/1.1
//! (http.rs), with JSON (json.rs). It knows nothing of networks or encryption: the kernel's TLS connection carries
//! the bytes. No code from other projects.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod api;
pub mod http;
pub mod json;
#[cfg(test)]
mod tests;
