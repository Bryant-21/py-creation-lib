use pyo3::prelude::*;

pub mod ast;
mod bindings;
pub mod context;
pub mod decompile;
pub mod emit;
pub mod error;
pub mod function_map;
pub mod lexer;
pub mod lower;
pub mod parser;

pub fn register_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    bindings::register_module(m)
}
