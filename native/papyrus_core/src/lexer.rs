//! Hand-written Papyrus tokenizer.
//!
//! Pure char dispatch, no regex engine. Keywords are matched case-insensitively.
//! Positions are 1-based line / 1-based col, matching `Pos` in `ast_nodes.py`.
//!
//! Negative literals are not pre-classified: the parser handles unary `-` by
//! context, so `Minus` is always its own token.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Identifiers / literals
    Name(String),
    Int(i64),
    Float(f64),
    Str(String),

    // Keywords (matched case-insensitively in source)
    KwScriptname,
    KwExtends,
    KwImport,
    KwProperty,
    KwEndProperty,
    KwFunction,
    KwEndFunction,
    KwEvent,
    KwEndEvent,
    KwState,
    KwEndState,
    KwAuto,
    KwAutoReadOnly,
    KwIf,
    KwElseIf,
    KwElse,
    KwEndIf,
    KwWhile,
    KwEndWhile,
    KwReturn,
    KwNative,
    KwGlobal,
    KwHidden,
    KwConditional,
    KwConst,
    KwMandatory,
    KwDefault,
    KwBetaOnly,
    KwDebugOnly,
    KwNew,
    KwAs,
    KwIs,
    KwSelf,
    KwParent,
    KwNone,
    KwTrue,
    KwFalse,
    KwVar,
    KwLength,
    KwStruct,
    KwEndStruct,
    KwGroup,
    KwEndGroup,
    KwCollapsed,
    KwCollapsedOnRef,
    KwCollapsedOnBase,
    KwCustomEvent,

    // Operators / punctuation
    Eq,          // ==
    Neq,         // !=
    Lte,         // <=
    Gte,         // >=
    Lt,          // <
    Gt,          // >
    OrOr,        // ||
    AndAnd,      // &&
    PlusAssign,  // +=
    MinusAssign, // -=
    MulAssign,   // *=
    DivAssign,   // /=
    ModAssign,   // %=
    Assign,      // =
    Plus,        // +
    Minus,       // -
    Star,        // *
    Slash,       // /
    Percent,     // %
    Bang,        // !
    Dot,         // .
    Comma,       // ,
    Colon,       // :
    LParen,      // (
    RParen,      // )
    LBracket,    // [
    RBracket,    // ]

    Newline,
    Eof,
}

impl TokenKind {
    /// True for tokens that introduce or are part of a Papyrus identifier-like
    /// name (used by tests/diagnostics; not part of the parsing algorithm).
    pub fn is_name_like(&self) -> bool {
        matches!(self, TokenKind::Name(_))
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub pos: Pos,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "lex error at {}:{}: {}",
            self.pos.line, self.pos.col, self.message
        )
    }
}

impl std::error::Error for LexError {}

/// Source preprocessor.
///
/// - Normalize CRLF / CR → LF.
/// - Collapse line continuations (`\` followed by spaces/tabs and a newline → one space).
/// - Strip doc comments `{ ... }` (replace with spaces, preserve newlines so
///   line numbers stay correct).
/// - Strip block comments `;/ ... /;` (same: replace with spaces, preserve newlines).
/// - Ensure trailing newline.
///
/// Line comments `;...` are handled inline by the tokenizer (they end at LF).
pub fn preprocess(src: &str) -> (String, Vec<DocComment>) {
    let mut out = String::with_capacity(src.len() + 1);
    let bytes = src.as_bytes();
    let mut i = 0;
    // Doc comments `{ ... }` are stripped from `out` (replaced with whitespace),
    // but their content is captured here keyed by the byte offset in `out` where
    // the `{` sat — the line is computed after `out` is built so it matches the
    // tokenizer's newline-counted line numbers exactly.
    let mut docs_raw: Vec<(usize, String)> = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];

        // CRLF → LF, lone CR → LF.
        if b == b'\r' {
            out.push('\n');
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
            continue;
        }

        // Line continuation: `\` (optional spaces/tabs) `\n` → single space.
        if b == b'\\' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'\n' || bytes[j] == b'\r') {
                out.push(' ');
                i = j + 1;
                if bytes[j] == b'\r' && i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                continue;
            }
            // Not a line continuation — fall through.
        }

        // Doc comment `{ ... }` — replace with spaces in `out`, capture content.
        if b == b'{' {
            let offset = out.len();
            let content_start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] != b'}' {
                if bytes[i] == b'\n' {
                    out.push('\n');
                } else if bytes[i] == b'\r' {
                    out.push('\n');
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                        i += 1;
                    }
                } else {
                    out.push(' ');
                }
                i += 1;
            }
            let content_end = i;
            if i < bytes.len() {
                i += 1; // skip closing '}'
            }
            let raw = String::from_utf8_lossy(&bytes[content_start..content_end]);
            let text = raw.replace("\r\n", "\n").replace('\r', "\n");
            let text = text.trim().to_string();
            if !text.is_empty() {
                docs_raw.push((offset, text));
            }
            continue;
        }

        // Block comment `;/ ... /;` — replace with spaces, preserve newlines.
        if b == b';' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'/' && bytes[i + 1] == b';') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                } else if bytes[i] == b'\r' {
                    out.push('\n');
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                        i += 1;
                    }
                } else {
                    out.push(' ');
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2; // skip closing '/;'
            }
            continue;
        }

        out.push(b as char);
        i += 1;
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }

    // Resolve each captured doc comment's 1-based line by counting newlines in
    // `out` up to its offset — identical to how the tokenizer numbers lines.
    let out_bytes = out.as_bytes();
    let docs = docs_raw
        .into_iter()
        .map(|(offset, text)| {
            let line = 1 + out_bytes[..offset.min(out_bytes.len())]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32;
            DocComment { line, text }
        })
        .collect();
    (out, docs)
}

/// A `{ ... }` doc comment captured by `preprocess`, keyed by the 1-based line
/// of its opening brace (in tokenizer line-numbering).
#[derive(Debug, Clone)]
pub struct DocComment {
    pub line: u32,
    pub text: String,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn cur_pos(&self) -> Pos {
        Pos {
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek(0)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    /// Skip horizontal whitespace and line comments (but not newlines).
    fn skip_horiz_ws_and_line_comment(&mut self) {
        loop {
            match self.peek(0) {
                Some(b' ') | Some(b'\t') => {
                    self.advance();
                }
                Some(b';') => {
                    // Line comment runs to LF (block comments handled in preprocess).
                    while let Some(b) = self.peek(0) {
                        if b == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn read_name_or_keyword(&mut self) -> Token {
        let start = self.cur_pos();
        let begin = self.pos;
        while let Some(b) = self.peek(0) {
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.advance();
            } else {
                break;
            }
        }
        let raw = std::str::from_utf8(&self.src[begin..self.pos]).unwrap();
        let end = self.cur_pos();
        let kind = match keyword_kind(raw) {
            Some(k) => k,
            None => TokenKind::Name(raw.to_owned()),
        };
        Token { kind, start, end }
    }

    fn read_number(&mut self) -> Result<Token, LexError> {
        let start = self.cur_pos();
        let begin = self.pos;

        // Hex prefix?
        if self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'x') | Some(b'X')) {
            self.advance(); // 0
            self.advance(); // x/X
            let hex_begin = self.pos;
            while let Some(b) = self.peek(0) {
                if b.is_ascii_hexdigit() {
                    self.advance();
                } else {
                    break;
                }
            }
            if self.pos == hex_begin {
                return Err(LexError {
                    message: "expected hex digits after `0x`".into(),
                    pos: start,
                });
            }
            let raw = std::str::from_utf8(&self.src[hex_begin..self.pos]).unwrap();
            let value = i64::from_str_radix(raw, 16).map_err(|e| LexError {
                message: format!("invalid hex literal: {e}"),
                pos: start,
            })?;
            return Ok(Token {
                kind: TokenKind::Int(value),
                start,
                end: self.cur_pos(),
            });
        }

        let mut is_float = false;
        while let Some(b) = self.peek(0) {
            if b.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        if self.peek(0) == Some(b'.') && matches!(self.peek(1), Some(d) if d.is_ascii_digit()) {
            is_float = true;
            self.advance(); // .
            while let Some(b) = self.peek(0) {
                if b.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        } else if self.peek(0) == Some(b'.') && self.peek(1).is_none() {
            // Trailing `.` with no fractional part — unusual but accept it as float.
            is_float = true;
            self.advance();
        } else if self.peek(0) == Some(b'.')
            && !matches!(self.peek(1), Some(b) if b.is_ascii_alphanumeric() || b == b'_')
        {
            // `1.` followed by non-ident, non-digit — treat as float (e.g., "1.\n").
            is_float = true;
            self.advance();
        }

        let raw = std::str::from_utf8(&self.src[begin..self.pos]).unwrap();
        let kind = if is_float {
            TokenKind::Float(raw.parse::<f64>().map_err(|e| LexError {
                message: format!("invalid float literal `{raw}`: {e}"),
                pos: start,
            })?)
        } else {
            TokenKind::Int(raw.parse::<i64>().map_err(|e| LexError {
                message: format!("invalid int literal `{raw}`: {e}"),
                pos: start,
            })?)
        };
        Ok(Token {
            kind,
            start,
            end: self.cur_pos(),
        })
    }

    fn read_string(&mut self) -> Result<Token, LexError> {
        let start = self.cur_pos();
        debug_assert_eq!(self.peek(0), Some(b'"'));
        self.advance(); // opening quote
        let mut begin = self.pos;
        let mut out = String::new();
        while let Some(b) = self.peek(0) {
            if b == b'"' {
                out.push_str(
                    std::str::from_utf8(&self.src[begin..self.pos]).map_err(|e| LexError {
                        message: format!("non-utf8 string: {e}"),
                        pos: start,
                    })?,
                );
                self.advance(); // closing quote
                return Ok(Token {
                    kind: TokenKind::Str(out),
                    start,
                    end: self.cur_pos(),
                });
            }
            if b == b'\n' {
                return Err(LexError {
                    message: "unterminated string literal".into(),
                    pos: start,
                });
            }
            if b == b'\\' && self.peek(1) == Some(b't') {
                out.push_str(
                    std::str::from_utf8(&self.src[begin..self.pos]).map_err(|e| LexError {
                        message: format!("non-utf8 string: {e}"),
                        pos: start,
                    })?,
                );
                out.push('\t');
                self.advance();
                self.advance();
                begin = self.pos;
                continue;
            }
            self.advance();
        }
        Err(LexError {
            message: "unterminated string literal".into(),
            pos: start,
        })
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_horiz_ws_and_line_comment();
        let start = self.cur_pos();
        let b = match self.peek(0) {
            Some(b) => b,
            None => return Ok(None),
        };

        // Single-char/multi-char operators and punctuation.
        let two = self.peek(1);
        let kind = match (b, two) {
            (b'\n', _) => {
                self.advance();
                let end = self.cur_pos();
                return Ok(Some(Token {
                    kind: TokenKind::Newline,
                    start,
                    end,
                }));
            }
            (b'=', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::Eq
            }
            (b'!', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::Neq
            }
            (b'<', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::Lte
            }
            (b'>', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::Gte
            }
            (b'|', Some(b'|')) => {
                self.advance();
                self.advance();
                TokenKind::OrOr
            }
            (b'&', Some(b'&')) => {
                self.advance();
                self.advance();
                TokenKind::AndAnd
            }
            (b'+', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::PlusAssign
            }
            (b'-', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::MinusAssign
            }
            (b'*', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::MulAssign
            }
            (b'/', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::DivAssign
            }
            (b'%', Some(b'=')) => {
                self.advance();
                self.advance();
                TokenKind::ModAssign
            }
            (b'<', _) => {
                self.advance();
                TokenKind::Lt
            }
            (b'>', _) => {
                self.advance();
                TokenKind::Gt
            }
            (b'=', _) => {
                self.advance();
                TokenKind::Assign
            }
            (b'+', _) => {
                self.advance();
                TokenKind::Plus
            }
            (b'-', _) => {
                self.advance();
                TokenKind::Minus
            }
            (b'*', _) => {
                self.advance();
                TokenKind::Star
            }
            (b'/', _) => {
                self.advance();
                TokenKind::Slash
            }
            (b'%', _) => {
                self.advance();
                TokenKind::Percent
            }
            (b'!', _) => {
                self.advance();
                TokenKind::Bang
            }
            (b'.', _) => {
                self.advance();
                TokenKind::Dot
            }
            (b',', _) => {
                self.advance();
                TokenKind::Comma
            }
            (b':', _) => {
                self.advance();
                TokenKind::Colon
            }
            (b'(', _) => {
                self.advance();
                TokenKind::LParen
            }
            (b')', _) => {
                self.advance();
                TokenKind::RParen
            }
            (b'[', _) => {
                self.advance();
                TokenKind::LBracket
            }
            (b']', _) => {
                self.advance();
                TokenKind::RBracket
            }
            (b'"', _) => return self.read_string().map(Some),
            (d, _) if d.is_ascii_digit() => return self.read_number().map(Some),
            (c, _) if c.is_ascii_alphabetic() || c == b'_' => {
                return Ok(Some(self.read_name_or_keyword()));
            }
            (other, _) => {
                return Err(LexError {
                    message: format!("unexpected character `{}`", other as char),
                    pos: start,
                });
            }
        };
        let end = self.cur_pos();
        Ok(Some(Token { kind, start, end }))
    }
}

/// Match a raw identifier against the case-insensitive Papyrus keyword set.
/// Returns `None` if the word is a regular identifier.
fn keyword_kind(raw: &str) -> Option<TokenKind> {
    let lower = raw.to_ascii_lowercase();
    Some(match lower.as_str() {
        "scriptname" => TokenKind::KwScriptname,
        "extends" => TokenKind::KwExtends,
        "import" => TokenKind::KwImport,
        "property" => TokenKind::KwProperty,
        "endproperty" => TokenKind::KwEndProperty,
        "function" => TokenKind::KwFunction,
        "endfunction" => TokenKind::KwEndFunction,
        "event" => TokenKind::KwEvent,
        "endevent" => TokenKind::KwEndEvent,
        "state" => TokenKind::KwState,
        "endstate" => TokenKind::KwEndState,
        "auto" => TokenKind::KwAuto,
        "autoreadonly" => TokenKind::KwAutoReadOnly,
        "if" => TokenKind::KwIf,
        "elseif" => TokenKind::KwElseIf,
        "else" => TokenKind::KwElse,
        "endif" => TokenKind::KwEndIf,
        "while" => TokenKind::KwWhile,
        "endwhile" => TokenKind::KwEndWhile,
        "return" => TokenKind::KwReturn,
        "native" => TokenKind::KwNative,
        "global" => TokenKind::KwGlobal,
        "hidden" => TokenKind::KwHidden,
        "conditional" => TokenKind::KwConditional,
        "const" => TokenKind::KwConst,
        "mandatory" => TokenKind::KwMandatory,
        "default" => TokenKind::KwDefault,
        "betaonly" => TokenKind::KwBetaOnly,
        "debugonly" => TokenKind::KwDebugOnly,
        "new" => TokenKind::KwNew,
        "as" => TokenKind::KwAs,
        "is" => TokenKind::KwIs,
        "self" => TokenKind::KwSelf,
        "parent" => TokenKind::KwParent,
        "none" => TokenKind::KwNone,
        "true" => TokenKind::KwTrue,
        "false" => TokenKind::KwFalse,
        "var" => TokenKind::KwVar,
        "length" => TokenKind::KwLength,
        "struct" => TokenKind::KwStruct,
        "endstruct" => TokenKind::KwEndStruct,
        "group" => TokenKind::KwGroup,
        "endgroup" => TokenKind::KwEndGroup,
        "collapsed" => TokenKind::KwCollapsed,
        "collapsedonref" => TokenKind::KwCollapsedOnRef,
        "collapsedonbase" => TokenKind::KwCollapsedOnBase,
        "customevent" => TokenKind::KwCustomEvent,
        _ => return None,
    })
}

/// Tokenize a Papyrus source string. Runs `preprocess` first.
///
/// Trailing `Eof` token is appended so the parser can rely on a sentinel.
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    Ok(tokenize_with_docs(src)?.0)
}

/// Like `tokenize`, but also returns the doc comments captured by `preprocess`
/// (the compiler attaches them to property/function declarations).
pub fn tokenize_with_docs(src: &str) -> Result<(Vec<Token>, Vec<DocComment>), LexError> {
    let (cleaned, docs) = preprocess(src);
    let mut lex = Lexer::new(&cleaned);
    let mut out = Vec::new();
    while let Some(tok) = lex.next_token()? {
        out.push(tok);
    }
    let end = lex.cur_pos();
    out.push(Token {
        kind: TokenKind::Eof,
        start: end,
        end,
    });
    Ok((out, docs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind as K;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| !matches!(k, K::Newline | K::Eof))
            .collect()
    }

    fn name(n: &str) -> TokenKind {
        K::Name(n.into())
    }

    fn start_of(src: &str, n: &str) -> Pos {
        tokenize(src)
            .unwrap()
            .into_iter()
            .find(|t| matches!(&t.kind, K::Name(x) if x == n))
            .unwrap()
            .start
    }

    #[test]
    fn tokenizes_representative_inputs() {
        let foo_bar = vec![name("Foo"), name("Bar")];
        let cases: Vec<(&str, Vec<TokenKind>)> = vec![
            (
                "scriptname Foo EXTENDS Bar",
                vec![K::KwScriptname, name("Foo"), K::KwExtends, name("Bar")],
            ),
            ("42", vec![K::Int(42)]),
            ("3.14", vec![K::Float(3.14)]),
            ("0x1a2B", vec![K::Int(0x1a2B)]),
            // The lexer never produces negative literals; that is the parser's job.
            ("-1.5", vec![K::Minus, K::Float(1.5)]),
            (r#""hello world""#, vec![K::Str("hello world".into())]),
            (r#""a\tb""#, vec![K::Str("a\tb".into())]),
            (r#""""#, vec![K::Str(String::new())]),
            ("Foo ; this is a comment\nBar", foo_bar.clone()),
            ("Foo ;/ block /; Bar", foo_bar.clone()),
            ("Foo \\\n  Bar", foo_bar.clone()),
            // `B21:Script` is NAME ":" NAME; the parser glues namespaced names together.
            ("B21:Script", vec![name("B21"), K::Colon, name("Script")]),
            (
                "== != <= >= < > || && += -= *= /= %= = + - * / % ! . , : ( ) [ ]",
                vec![
                    K::Eq,
                    K::Neq,
                    K::Lte,
                    K::Gte,
                    K::Lt,
                    K::Gt,
                    K::OrOr,
                    K::AndAnd,
                    K::PlusAssign,
                    K::MinusAssign,
                    K::MulAssign,
                    K::DivAssign,
                    K::ModAssign,
                    K::Assign,
                    K::Plus,
                    K::Minus,
                    K::Star,
                    K::Slash,
                    K::Percent,
                    K::Bang,
                    K::Dot,
                    K::Comma,
                    K::Colon,
                    K::LParen,
                    K::RParen,
                    K::LBracket,
                    K::RBracket,
                ],
            ),
        ];
        for (src, want) in cases {
            assert_eq!(kinds(src), want, "{src:?}");
        }
    }

    #[test]
    fn edge_inputs() {
        assert!(matches!(tokenize("").unwrap().last().unwrap().kind, K::Eof));
        let err = tokenize("\"oops\n").unwrap_err();
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn positions_are_one_based_and_survive_normalization() {
        assert_eq!(tokenize("Foo\n  Bar").unwrap()[0].start, Pos { line: 1, col: 1 });
        assert_eq!(start_of("Foo\n  Bar", "Bar"), Pos { line: 2, col: 3 });
        assert_eq!(start_of("Foo\r\nBar", "Bar").line, 2);
        // A doc comment eats its internal newline but keeps line numbering.
        assert_eq!(
            kinds("Foo { docstring spanning\nmultiple lines } Bar"),
            vec![name("Foo"), name("Bar")]
        );
        assert_eq!(start_of("Foo { docstring spanning\nmultiple lines } Bar", "Bar").line, 2);
    }
}
