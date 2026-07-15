//! papyrus_core — pure-Rust Papyrus parser, AST, resolver, emitter, and script DB.
//!
//! Design invariant: core modules (`lexer`, `parser`, `ast`, `resolver`, `emitter`,
//! `script_db`, `session`) MUST NOT reference `pyo3`. The PyO3 surface lives in
//! `bindings.rs` and wraps every entry point in `Python::detach` (PyO3 0.28's
//! renamed `allow_threads`) so the GIL is never held during parse, resolve, or
//! DB work.

pub mod ast;
pub mod codegen;
pub mod compiler;
pub mod emitter;
pub mod lexer;
pub mod parser;
pub mod pex;
pub mod pex_writer;
pub mod profile;
pub mod resolver;
pub mod script_db;
pub mod session;
pub mod source_resolver;
pub mod typeck;

mod bindings;

pub use bindings::register_module;
