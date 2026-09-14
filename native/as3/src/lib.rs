//! An ActionScript 3 compiler targeting AVM2 ABC bytecode.
//!
//! # Provenance and licence
//!
//! Every file in this crate is written from published specifications: Adobe's
//! *ActionScript Virtual Machine 2 Overview* for ABC, the *SWF File Format
//! Specification* for the container, and Adobe's ActionScript 3 language
//! reference for the grammar. **No file here is a port of, or derived from,
//! swftools or any other GPL compiler**, so this crate carries the parent
//! repository's licence with no additional obligation.
//!
//! swftools' `lib/as3` was surveyed for architecture only; modules that diverge
//! from it say why. Code ported from swftools must live in a separate crate
//! with a GPL-2.0-or-later header attributing swftools, not in this one.
//!
//! # Pipeline
//!
//! [`lexer`] → [`parser`] → [`ast`] → [`codegen`] → [`abc`], with [`diag`]
//! carrying positions and errors across all of them.
//!
//! ```
//! let abc = as3_native::compile_source(
//!     "package { public class Foo extends Object { public function Foo() {} } }",
//! )
//! .unwrap();
//! assert_eq!(&abc[..4], &[0x10, 0x00, 0x2E, 0x00]); // ABC 46.16
//! ```

pub mod abc;
pub mod ast;
pub mod codegen;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod types;

pub use diag::{Diagnostic, Result, Stage};

/// Compile AS3 source text to a raw ABC block.
pub fn compile_source(src: &str) -> Result<Vec<u8>> {
    compile_sources(&[src])
}

/// Compile several AS3 source files into one ABC block.
///
/// AS3 allows one package per file, so a widget whose document class is in the
/// unnamed package and whose interface is in `hudframework` is necessarily two
/// files. Order does not matter: types are sorted so that a base class or
/// interface is defined before whatever depends on it.
pub fn compile_sources(sources: &[&str]) -> Result<Vec<u8>> {
    let units = sources
        .iter()
        .map(|s| parser::parse(s))
        .collect::<Result<Vec<_>>>()?;
    codegen::compile_units(&units)
}

/// Compile AS3 source text to a `DoABCDefine` (tag 82) tag *body*, ready for a
/// SWF packer to write a tag header in front of. The tag must be placed before
/// the `SymbolClass` that binds the classes it defines.
pub fn compile_to_do_abc(src: &str) -> Result<Vec<u8>> {
    Ok(abc::file::do_abc_define_body(&compile_source(src)?))
}
