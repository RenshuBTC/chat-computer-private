//! Chat Computer's own web browser engine, for its own system: a page's HTML read into a tree (html.rs, dom.rs), its
//! style sheets read (css.rs), each element's style worked out (style.rs, values.rs), and the page laid out and drawn
//! (layout.rs) in the middle of the screen, as the website and the app show a site; its scripts run by Chat Computer's
//! own JavaScript engine, with the DOM and the rest of what a page's scripts use (script/).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod address;
pub mod animate;
pub mod boxes;
pub mod css;
pub mod dom;
pub mod effects;
pub mod first_letter;
pub mod html;
pub mod layout;
pub mod num;
pub mod page;
pub mod paint;
pub mod script;
pub mod srcset;
pub mod style;
pub mod values;

pub use page::{Hit, Page, Request};

#[cfg(test)]
mod tests;
