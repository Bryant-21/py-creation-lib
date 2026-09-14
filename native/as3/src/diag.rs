//! Source positions and compiler errors.
//!
//! Written from scratch for this crate; not derived from any other compiler.

use std::fmt;

/// A 1-based source position. Byte `offset` is kept so a caller can slice the
/// original text; `line`/`col` are what a human needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub offset: u32,
    pub line: u32,
    pub col: u32,
}

/// Half-open source range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    pub fn new(start: Pos, end: Pos) -> Self {
        Self { start, end }
    }
}

/// What stage rejected the input. The distinction matters to callers: an
/// `Unsupported` diagnostic means the source is valid AS3 that this phase of
/// the compiler cannot yet lower, which is a different conversation from a
/// syntax error in the user's file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Lex,
    Parse,
    /// Valid AS3, outside the implemented subset.
    Unsupported,
    Codegen,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stage::Lex => "lex error",
            Stage::Parse => "syntax error",
            Stage::Unsupported => "unsupported",
            Stage::Codegen => "codegen error",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub stage: Stage,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(stage: Stage, message: impl Into<String>, span: Span) -> Self {
        Self {
            stage,
            message: message.into(),
            span,
        }
    }

    pub fn lex(message: impl Into<String>, span: Span) -> Self {
        Self::new(Stage::Lex, message, span)
    }

    pub fn parse(message: impl Into<String>, span: Span) -> Self {
        Self::new(Stage::Parse, message, span)
    }

    pub fn unsupported(message: impl Into<String>, span: Span) -> Self {
        Self::new(Stage::Unsupported, message, span)
    }

    pub fn codegen(message: impl Into<String>, span: Span) -> Self {
        Self::new(Stage::Codegen, message, span)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}: {}",
            self.span.start.line, self.span.start.col, self.stage, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

pub type Result<T> = std::result::Result<T, Diagnostic>;
