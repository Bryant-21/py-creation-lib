use crate::error::FnvScriptError;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Keyword(String),
    Ident(String),
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    Punct(char),
    Op(String),
    Newline,
}

const KEYWORDS: &[&str] = &[
    "ScriptName",
    "scn",
    "Begin",
    "End",
    "if",
    "elseif",
    "else",
    "endif",
    "Set",
    "to",
    "Return",
    "int",
    "float",
    "ref",
    "short",
    "long",
    "string_var",
];

pub fn tokenize(src: &str) -> Result<Vec<Token>, FnvScriptError> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    let mut line = 1usize;
    let mut col = 1usize;

    while let Some(&c) = chars.peek() {
        match c {
            ';' => {
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                    col += 1;
                }
            }
            '\r' => {
                chars.next();
            }
            '\n' => {
                out.push(Token {
                    kind: TokenKind::Newline,
                    line,
                    col,
                });
                chars.next();
                line += 1;
                col = 1;
            }
            c if c.is_whitespace() => {
                chars.next();
                col += 1;
            }
            '"' => {
                let start_col = col;
                chars.next();
                col += 1;
                let mut s = String::new();
                let mut terminated = false;
                while let Some(&n) = chars.peek() {
                    match n {
                        '"' => {
                            chars.next();
                            col += 1;
                            terminated = true;
                            break;
                        }
                        '\n' => {
                            return Err(FnvScriptError::Parse {
                                line,
                                col,
                                msg: "unterminated string literal".into(),
                            });
                        }
                        _ => {
                            s.push(n);
                            chars.next();
                            col += 1;
                        }
                    }
                }
                if !terminated {
                    return Err(FnvScriptError::Parse {
                        line,
                        col: start_col,
                        msg: "unterminated string literal".into(),
                    });
                }
                out.push(Token {
                    kind: TokenKind::StringLit(s),
                    line,
                    col: start_col,
                });
            }
            c if c.is_ascii_digit() || (c == '-' && matches_digit_lookahead(&chars)) => {
                let start_col = col;
                let mut s = String::new();
                let mut is_float = false;
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() || n == '.' || (n == '-' && s.is_empty()) {
                        if n == '.' {
                            is_float = true;
                        }
                        s.push(n);
                        chars.next();
                        col += 1;
                    } else {
                        break;
                    }
                }
                let kind = if is_float {
                    TokenKind::FloatLit(s.parse().map_err(|_| FnvScriptError::Parse {
                        line,
                        col: start_col,
                        msg: format!("bad float {s}"),
                    })?)
                } else {
                    TokenKind::IntLit(s.parse().map_err(|_| FnvScriptError::Parse {
                        line,
                        col: start_col,
                        msg: format!("bad int {s}"),
                    })?)
                };
                out.push(Token {
                    kind,
                    line,
                    col: start_col,
                });
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start_col = col;
                let mut s = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        s.push(n);
                        chars.next();
                        col += 1;
                    } else {
                        break;
                    }
                }
                let kind = match KEYWORDS
                    .iter()
                    .find(|keyword| keyword.eq_ignore_ascii_case(&s))
                {
                    Some(keyword) => TokenKind::Keyword((*keyword).to_string()),
                    None => TokenKind::Ident(s),
                };
                out.push(Token {
                    kind,
                    line,
                    col: start_col,
                });
            }
            '=' | '!' | '<' | '>' | '&' | '|' | '+' | '-' | '*' | '/' | '%' => {
                let start_col = col;
                let mut op = String::new();
                op.push(c);
                chars.next();
                col += 1;
                if let Some(&n) = chars.peek() {
                    let combo = format!("{op}{n}");
                    if matches!(combo.as_str(), "==" | "!=" | "<=" | ">=" | "&&" | "||") {
                        op.push(n);
                        chars.next();
                        col += 1;
                    }
                }
                out.push(Token {
                    kind: TokenKind::Op(op),
                    line,
                    col: start_col,
                });
            }
            '(' | ')' | ',' | '.' => {
                out.push(Token {
                    kind: TokenKind::Punct(c),
                    line,
                    col,
                });
                chars.next();
                col += 1;
            }
            other => {
                return Err(FnvScriptError::Parse {
                    line,
                    col,
                    msg: format!("unexpected char '{other}'"),
                });
            }
        }
    }

    Ok(out)
}

fn matches_digit_lookahead(chars: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let mut cloned = chars.clone();
    cloned.next();
    matches!(cloned.peek(), Some(d) if d.is_ascii_digit())
}
