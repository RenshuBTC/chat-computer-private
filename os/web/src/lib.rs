//! Chat Computer's own reading of the web, for its own system's model (kernel/src/web.rs does the fetching, over its
//! own TCP and TLS): web addresses (`url`), HTTP/1.1's requests and responses (`http`), a page's words (`page`) and
//! a search's results (`search`), as the main Chat Computer's tools have them (chatcomputer/brain/builtin.py: a
//! DuckDuckGo search, and a page read as its words), so the model reads the web the same way on both.
//!
//! Nothing here runs anything a page holds: its scripts, styles and pictures are dropped, and only its words kept.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod http;
pub mod page;
pub mod search;
pub mod url;

#[cfg(test)]
mod tests;

pub use url::Url;

/// How Chat Computer says who it is to the sites it reads (as the main Chat Computer does).
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) ChatComputer/1.0";
/// The most of a page's bytes read (as the main Chat Computer reads).
pub const MOST_BYTES: usize = 3 << 20;
