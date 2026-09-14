//! ActionScript 3 tokenizer.
//!
//! Written from the ECMA-262 3rd edition lexical grammar and Adobe's AS3
//! language reference, not from another compiler's scanner.
//!
//! `public`, `internal` and `native` are reserved words. `static`, `dynamic`,
//! `final`, `override`, `get`, `set` and `each` are contextual and lex as plain
//! identifiers, so `var dynamic:int` is accepted as the language allows.

use crate::diag::{Diagnostic, Pos, Result, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    As,
    Break,
    Case,
    Catch,
    Class,
    Const,
    Continue,
    Default,
    Delete,
    Do,
    Else,
    Extends,
    False,
    Finally,
    For,
    Function,
    If,
    Implements,
    Import,
    In,
    Instanceof,
    Interface,
    Internal,
    Is,
    Native,
    New,
    Null,
    Package,
    Private,
    Protected,
    Public,
    Return,
    Super,
    Switch,
    This,
    Throw,
    True,
    Try,
    Typeof,
    Use,
    Var,
    Void,
    While,
    With,
}

impl Keyword {
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::As => "as",
            Keyword::Break => "break",
            Keyword::Case => "case",
            Keyword::Catch => "catch",
            Keyword::Class => "class",
            Keyword::Const => "const",
            Keyword::Continue => "continue",
            Keyword::Default => "default",
            Keyword::Delete => "delete",
            Keyword::Do => "do",
            Keyword::Else => "else",
            Keyword::Extends => "extends",
            Keyword::False => "false",
            Keyword::Finally => "finally",
            Keyword::For => "for",
            Keyword::Function => "function",
            Keyword::If => "if",
            Keyword::Implements => "implements",
            Keyword::Import => "import",
            Keyword::In => "in",
            Keyword::Instanceof => "instanceof",
            Keyword::Interface => "interface",
            Keyword::Internal => "internal",
            Keyword::Is => "is",
            Keyword::Native => "native",
            Keyword::New => "new",
            Keyword::Null => "null",
            Keyword::Package => "package",
            Keyword::Private => "private",
            Keyword::Protected => "protected",
            Keyword::Public => "public",
            Keyword::Return => "return",
            Keyword::Super => "super",
            Keyword::Switch => "switch",
            Keyword::This => "this",
            Keyword::Throw => "throw",
            Keyword::True => "true",
            Keyword::Try => "try",
            Keyword::Typeof => "typeof",
            Keyword::Use => "use",
            Keyword::Var => "var",
            Keyword::Void => "void",
            Keyword::While => "while",
            Keyword::With => "with",
        }
    }

    fn from_str(text: &str) -> Option<Keyword> {
        Some(match text {
            "as" => Keyword::As,
            "break" => Keyword::Break,
            "case" => Keyword::Case,
            "catch" => Keyword::Catch,
            "class" => Keyword::Class,
            "const" => Keyword::Const,
            "continue" => Keyword::Continue,
            "default" => Keyword::Default,
            "delete" => Keyword::Delete,
            "do" => Keyword::Do,
            "else" => Keyword::Else,
            "extends" => Keyword::Extends,
            "false" => Keyword::False,
            "finally" => Keyword::Finally,
            "for" => Keyword::For,
            "function" => Keyword::Function,
            "if" => Keyword::If,
            "implements" => Keyword::Implements,
            "import" => Keyword::Import,
            "in" => Keyword::In,
            "instanceof" => Keyword::Instanceof,
            "interface" => Keyword::Interface,
            "internal" => Keyword::Internal,
            "is" => Keyword::Is,
            "native" => Keyword::Native,
            "new" => Keyword::New,
            "null" => Keyword::Null,
            "package" => Keyword::Package,
            "private" => Keyword::Private,
            "protected" => Keyword::Protected,
            "public" => Keyword::Public,
            "return" => Keyword::Return,
            "super" => Keyword::Super,
            "switch" => Keyword::Switch,
            "this" => Keyword::This,
            "throw" => Keyword::Throw,
            "true" => Keyword::True,
            "try" => Keyword::Try,
            "typeof" => Keyword::Typeof,
            "use" => Keyword::Use,
            "var" => Keyword::Var,
            "void" => Keyword::Void,
            "while" => Keyword::While,
            "with" => Keyword::With,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Punct {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    DotDotDot,
    Colon,
    ColonColon,
    Question,
    At,
    Assign,
    Eq,
    StrictEq,
    Ne,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusPlus,
    MinusMinus,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    AndAnd,
    OrOr,
    Not,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    UShr,
}

impl Punct {
    pub fn as_str(self) -> &'static str {
        match self {
            Punct::LBrace => "{",
            Punct::RBrace => "}",
            Punct::LParen => "(",
            Punct::RParen => ")",
            Punct::LBracket => "[",
            Punct::RBracket => "]",
            Punct::Semi => ";",
            Punct::Comma => ",",
            Punct::Dot => ".",
            Punct::DotDotDot => "...",
            Punct::Colon => ":",
            Punct::ColonColon => "::",
            Punct::Question => "?",
            Punct::At => "@",
            Punct::Assign => "=",
            Punct::Eq => "==",
            Punct::StrictEq => "===",
            Punct::Ne => "!=",
            Punct::StrictNe => "!==",
            Punct::Lt => "<",
            Punct::Gt => ">",
            Punct::Le => "<=",
            Punct::Ge => ">=",
            Punct::Plus => "+",
            Punct::Minus => "-",
            Punct::Star => "*",
            Punct::Slash => "/",
            Punct::Percent => "%",
            Punct::PlusPlus => "++",
            Punct::MinusMinus => "--",
            Punct::PlusAssign => "+=",
            Punct::MinusAssign => "-=",
            Punct::StarAssign => "*=",
            Punct::SlashAssign => "/=",
            Punct::PercentAssign => "%=",
            Punct::AndAnd => "&&",
            Punct::OrOr => "||",
            Punct::Not => "!",
            Punct::Amp => "&",
            Punct::Pipe => "|",
            Punct::Caret => "^",
            Punct::Tilde => "~",
            Punct::Shl => "<<",
            Punct::Shr => ">>",
            Punct::UShr => ">>>",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Ident(String),
    Keyword(Keyword),
    /// An integer literal that fits in `i64`. Whether it becomes an ABC `int`,
    /// `uint` or `double` constant is a codegen decision, not a lexing one.
    Int(i64),
    Number(f64),
    Str(String),
    Punct(Punct),
    Eof,
}

impl TokenKind {
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Ident(name) => format!("identifier `{name}`"),
            TokenKind::Keyword(k) => format!("`{}`", k.as_str()),
            TokenKind::Int(v) => format!("integer `{v}`"),
            TokenKind::Number(v) => format!("number `{v}`"),
            TokenKind::Str(_) => "string literal".to_string(),
            TokenKind::Punct(p) => format!("`{}`", p.as_str()),
            TokenKind::Eof => "end of file".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

struct Cursor<'a> {
    src: &'a [u8],
    offset: usize,
    line: u32,
    col: u32,
}

impl<'a> Cursor<'a> {
    fn pos(&self) -> Pos {
        Pos {
            offset: self.offset as u32,
            line: self.line,
            col: self.col,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.offset).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.offset + ahead).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.offset += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            // Continuation bytes of a UTF-8 sequence do not advance the column,
            // so `col` counts characters rather than bytes.
            if b & 0xC0 != 0x80 {
                self.col += 1;
            }
        }
        Some(b)
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.bump();
            true
        } else {
            false
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_ident_continue(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Tokenize an entire source file. The returned vector always ends with an
/// [`TokenKind::Eof`] token so the parser never has to bounds-check.
pub fn tokenize(src: &str) -> Result<Vec<Token>> {
    // A UTF-8 BOM is not whitespace and would otherwise lex as an identifier
    // start; HUDFramework's own `IHUDWidget.as` ships with one.
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);

    let mut cur = Cursor {
        src: src.as_bytes(),
        offset: 0,
        line: 1,
        col: 1,
    };
    let mut tokens = Vec::new();

    loop {
        skip_trivia(&mut cur)?;
        let start = cur.pos();
        let Some(b) = cur.peek() else {
            tokens.push(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
            });
            return Ok(tokens);
        };

        let kind = if is_ident_start(b) {
            lex_word(&mut cur)
        } else if b.is_ascii_digit()
            || (b == b'.' && cur.peek_at(1).is_some_and(|c| c.is_ascii_digit()))
        {
            lex_number(&mut cur, start)?
        } else if b == b'"' || b == b'\'' {
            lex_string(&mut cur, start)?
        } else {
            lex_punct(&mut cur, start)?
        };

        tokens.push(Token {
            kind,
            span: Span::new(start, cur.pos()),
        });
    }
}

fn skip_trivia(cur: &mut Cursor<'_>) -> Result<()> {
    loop {
        match cur.peek() {
            Some(b) if b.is_ascii_whitespace() => {
                cur.bump();
            }
            Some(b'/') if cur.peek_at(1) == Some(b'/') => {
                while let Some(b) = cur.peek() {
                    if b == b'\n' {
                        break;
                    }
                    cur.bump();
                }
            }
            Some(b'/') if cur.peek_at(1) == Some(b'*') => {
                let start = cur.pos();
                cur.bump();
                cur.bump();
                loop {
                    match cur.peek() {
                        None => {
                            return Err(Diagnostic::lex(
                                "unterminated block comment",
                                Span::new(start, cur.pos()),
                            ));
                        }
                        Some(b'*') if cur.peek_at(1) == Some(b'/') => {
                            cur.bump();
                            cur.bump();
                            break;
                        }
                        _ => {
                            cur.bump();
                        }
                    }
                }
            }
            _ => return Ok(()),
        }
    }
}

fn lex_word(cur: &mut Cursor<'_>) -> TokenKind {
    let start = cur.offset;
    while cur.peek().is_some_and(is_ident_continue) {
        cur.bump();
    }
    // Identifier bytes are ASCII or complete UTF-8 sequences copied out of a
    // `&str`, so this slice is always valid UTF-8.
    let text = std::str::from_utf8(&cur.src[start..cur.offset]).expect("identifier is valid UTF-8");
    match Keyword::from_str(text) {
        Some(k) => TokenKind::Keyword(k),
        None => TokenKind::Ident(text.to_string()),
    }
}

fn lex_number(cur: &mut Cursor<'_>, start: Pos) -> Result<TokenKind> {
    let begin = cur.offset;

    if cur.peek() == Some(b'0') && matches!(cur.peek_at(1), Some(b'x') | Some(b'X')) {
        cur.bump();
        cur.bump();
        let digits_at = cur.offset;
        while cur.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
            cur.bump();
        }
        if cur.offset == digits_at {
            return Err(Diagnostic::lex(
                "hexadecimal literal has no digits",
                Span::new(start, cur.pos()),
            ));
        }
        let text = &cur.src[digits_at..cur.offset];
        let text = std::str::from_utf8(text).expect("hex digits are ASCII");
        // Hex literals up to 0xFFFFFFFF are the common case (colour and flag
        // constants); anything wider is a double in AS3 too.
        return match u64::from_str_radix(text, 16) {
            Ok(v) if v <= i64::MAX as u64 => Ok(TokenKind::Int(v as i64)),
            _ => Err(Diagnostic::lex(
                "hexadecimal literal does not fit in 64 bits",
                Span::new(start, cur.pos()),
            )),
        };
    }

    let mut is_float = false;
    while cur.peek().is_some_and(|b| b.is_ascii_digit()) {
        cur.bump();
    }
    if cur.peek() == Some(b'.') && cur.peek_at(1).is_some_and(|b| b.is_ascii_digit()) {
        is_float = true;
        cur.bump();
        while cur.peek().is_some_and(|b| b.is_ascii_digit()) {
            cur.bump();
        }
    } else if cur.peek() == Some(b'.') && !cur.peek_at(1).is_some_and(is_ident_start) {
        // A trailing dot with no fraction digits, as in `1.` — still a float,
        // but `1.toString()` must not consume the dot.
        is_float = true;
        cur.bump();
    }
    if matches!(cur.peek(), Some(b'e') | Some(b'E')) {
        let save = (cur.offset, cur.line, cur.col);
        cur.bump();
        if matches!(cur.peek(), Some(b'+') | Some(b'-')) {
            cur.bump();
        }
        if cur.peek().is_some_and(|b| b.is_ascii_digit()) {
            is_float = true;
            while cur.peek().is_some_and(|b| b.is_ascii_digit()) {
                cur.bump();
            }
        } else {
            // Not an exponent after all (`1eight`); give the bytes back.
            cur.offset = save.0;
            cur.line = save.1;
            cur.col = save.2;
        }
    }

    let text = std::str::from_utf8(&cur.src[begin..cur.offset]).expect("number literal is ASCII");
    if is_float {
        text.parse::<f64>()
            .map(TokenKind::Number)
            .map_err(|_| Diagnostic::lex("malformed number literal", Span::new(start, cur.pos())))
    } else {
        match text.parse::<i64>() {
            Ok(v) => Ok(TokenKind::Int(v)),
            // Decimal literals wider than i64 are doubles in AS3, not errors.
            Err(_) => text.parse::<f64>().map(TokenKind::Number).map_err(|_| {
                Diagnostic::lex("malformed number literal", Span::new(start, cur.pos()))
            }),
        }
    }
}

fn lex_string(cur: &mut Cursor<'_>, start: Pos) -> Result<TokenKind> {
    let quote = cur.bump().expect("caller checked the quote");
    let mut out = String::new();
    loop {
        let Some(b) = cur.peek() else {
            return Err(Diagnostic::lex(
                "unterminated string literal",
                Span::new(start, cur.pos()),
            ));
        };
        if b == quote {
            cur.bump();
            return Ok(TokenKind::Str(out));
        }
        if b == b'\n' {
            return Err(Diagnostic::lex(
                "newline in string literal",
                Span::new(start, cur.pos()),
            ));
        }
        if b == b'\\' {
            cur.bump();
            let Some(esc) = cur.bump() else {
                return Err(Diagnostic::lex(
                    "unterminated escape sequence",
                    Span::new(start, cur.pos()),
                ));
            };
            match esc {
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'v' => out.push('\u{b}'),
                b'0' => out.push('\0'),
                b'\\' => out.push('\\'),
                b'\'' => out.push('\''),
                b'"' => out.push('"'),
                b'u' | b'x' => {
                    let want = if esc == b'u' { 4 } else { 2 };
                    let mut value: u32 = 0;
                    for _ in 0..want {
                        let Some(d) = cur.peek().filter(u8::is_ascii_hexdigit) else {
                            return Err(Diagnostic::lex(
                                "malformed unicode escape",
                                Span::new(start, cur.pos()),
                            ));
                        };
                        cur.bump();
                        value = value * 16 + (d as char).to_digit(16).expect("hex digit");
                    }
                    match char::from_u32(value) {
                        Some(c) => out.push(c),
                        None => {
                            return Err(Diagnostic::lex(
                                "escape is not a valid code point",
                                Span::new(start, cur.pos()),
                            ));
                        }
                    }
                }
                other => out.push(other as char),
            }
            continue;
        }
        // Copy the byte through; multi-byte UTF-8 sequences pass one byte at a
        // time and reassemble correctly because the source was a `&str`.
        let byte_start = cur.offset;
        cur.bump();
        while cur.peek().is_some_and(|c| c & 0xC0 == 0x80) {
            cur.bump();
        }
        out.push_str(
            std::str::from_utf8(&cur.src[byte_start..cur.offset]).expect("source is valid UTF-8"),
        );
    }
}

fn lex_punct(cur: &mut Cursor<'_>, start: Pos) -> Result<TokenKind> {
    let b = cur.bump().expect("caller checked there is a byte");
    let p = match b {
        b'{' => Punct::LBrace,
        b'}' => Punct::RBrace,
        b'(' => Punct::LParen,
        b')' => Punct::RParen,
        b'[' => Punct::LBracket,
        b']' => Punct::RBracket,
        b';' => Punct::Semi,
        b',' => Punct::Comma,
        b'?' => Punct::Question,
        b'@' => Punct::At,
        b'~' => Punct::Tilde,
        b'.' => {
            if cur.peek() == Some(b'.') && cur.peek_at(1) == Some(b'.') {
                cur.bump();
                cur.bump();
                Punct::DotDotDot
            } else {
                Punct::Dot
            }
        }
        b':' => {
            if cur.eat(b':') {
                Punct::ColonColon
            } else {
                Punct::Colon
            }
        }
        b'=' => {
            if cur.eat(b'=') {
                if cur.eat(b'=') {
                    Punct::StrictEq
                } else {
                    Punct::Eq
                }
            } else {
                Punct::Assign
            }
        }
        b'!' => {
            if cur.eat(b'=') {
                if cur.eat(b'=') {
                    Punct::StrictNe
                } else {
                    Punct::Ne
                }
            } else {
                Punct::Not
            }
        }
        b'<' => {
            if cur.eat(b'=') {
                Punct::Le
            } else if cur.eat(b'<') {
                Punct::Shl
            } else {
                Punct::Lt
            }
        }
        b'>' => {
            if cur.eat(b'=') {
                Punct::Ge
            } else if cur.eat(b'>') {
                if cur.eat(b'>') {
                    Punct::UShr
                } else {
                    Punct::Shr
                }
            } else {
                Punct::Gt
            }
        }
        b'+' => {
            if cur.eat(b'+') {
                Punct::PlusPlus
            } else if cur.eat(b'=') {
                Punct::PlusAssign
            } else {
                Punct::Plus
            }
        }
        b'-' => {
            if cur.eat(b'-') {
                Punct::MinusMinus
            } else if cur.eat(b'=') {
                Punct::MinusAssign
            } else {
                Punct::Minus
            }
        }
        b'*' => {
            if cur.eat(b'=') {
                Punct::StarAssign
            } else {
                Punct::Star
            }
        }
        b'/' => {
            if cur.eat(b'=') {
                Punct::SlashAssign
            } else {
                Punct::Slash
            }
        }
        b'%' => {
            if cur.eat(b'=') {
                Punct::PercentAssign
            } else {
                Punct::Percent
            }
        }
        b'&' => {
            if cur.eat(b'&') {
                Punct::AndAnd
            } else {
                Punct::Amp
            }
        }
        b'|' => {
            if cur.eat(b'|') {
                Punct::OrOr
            } else {
                Punct::Pipe
            }
        }
        b'^' => Punct::Caret,
        other => {
            return Err(Diagnostic::lex(
                format!("unexpected character {:?}", other as char),
                Span::new(start, cur.pos()),
            ));
        }
    };
    Ok(TokenKind::Punct(p))
}
