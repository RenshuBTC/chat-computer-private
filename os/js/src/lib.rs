//! Chat Computer's own JavaScript engine, for the web pages its own system shows (os/browser): a script parsed
//! (lexer.rs, parser.rs: ECMAScript 2024's grammar), compiled to bytecode, and run by a virtual machine whose frames
//! are its own (generators and async functions are frames set aside and taken up again), with a garbage collector
//! that finds what's still reachable, and the language's built-in objects. Written from the language's
//! specification, with nothing from other engines; `no_std`, so it runs in Chat Computer's own kernel.

#![no_std]

extern crate alloc;

pub mod ast;
pub mod bigint;
pub mod builtins;
pub mod code;
pub mod compiler;
mod convert;
mod generators;
pub mod lexer;
pub mod math;
pub mod normalize;
pub mod module;
pub mod names;
pub mod object;
pub mod parser;
mod props;
pub mod regexp;
pub mod scope;
pub mod string;
mod unicode_gc;
mod unicode_id;
pub mod value;
pub mod vm;

#[cfg(test)]
mod tests;
