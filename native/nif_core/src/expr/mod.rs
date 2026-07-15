use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExprError {
    #[error("tokenizer error at position {0}: {1}")]
    Tokenize(usize, String),
    #[error("parse error: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    NumInt(i64),
    NumFloat(f64),
    Then,
    Else,
    Len(String),
    Len2(String),
    Op2(String),
    Op1(char),
    LParen,
    RParen,
    Ident(String),
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Null,
}

impl Value {
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Bool(b) => *b,
            Value::Null => false,
        }
    }
    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Float(f) => *f as i64,
            Value::Bool(b) => {
                if *b {
                    1
                } else {
                    0
                }
            }
            Value::Null => 0,
        }
    }
    pub fn as_float(&self) -> f64 {
        match self {
            Value::Int(i) => *i as f64,
            Value::Float(f) => *f,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Null => 0.0,
        }
    }
    fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        if self.is_float() || other.is_float() {
            self.as_float() == other.as_float()
        } else {
            self.as_int() == other.as_int()
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.is_float() || other.is_float() {
            self.as_float().partial_cmp(&other.as_float())
        } else {
            self.as_int().partial_cmp(&other.as_int())
        }
    }
}

pub trait EvalContext {
    fn get_field(&self, path: &str) -> Value;
    fn get_field_len(&self, _path: &str) -> Option<usize> {
        None
    }
    fn get_field_len2(&self, path: &str) -> Option<usize> {
        self.get_field_len(path)
    }
}

pub struct MapContext(pub HashMap<String, Value>);

impl MapContext {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
    pub fn from_pairs(pairs: &[(&str, Value)]) -> Self {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), v.clone());
        }
        Self(m)
    }
}

impl Default for MapContext {
    fn default() -> Self {
        Self::new()
    }
}

impl EvalContext for MapContext {
    fn get_field(&self, path: &str) -> Value {
        self.0.get(path).cloned().unwrap_or(Value::Null)
    }
}

pub struct VersionContext {
    pub version: u32,
    pub user_version: u32,
    pub bs_version: u32,
    pub fields: HashMap<String, Value>,
    pub field_lens: HashMap<String, usize>,
    pub field_lens2: HashMap<String, usize>,
    pub arg: Value,
}

impl VersionContext {
    pub fn new(version: u32, user_version: u32, bs_version: u32) -> Self {
        Self {
            version,
            user_version,
            bs_version,
            fields: HashMap::new(),
            field_lens: HashMap::new(),
            field_lens2: HashMap::new(),
            arg: Value::Null,
        }
    }
}

impl EvalContext for VersionContext {
    fn get_field(&self, path: &str) -> Value {
        match path {
            "Version" => Value::Int(self.version as i64),
            "User Version" => Value::Int(self.user_version as i64),
            "BS Header\\BS Version" => Value::Int(self.bs_version as i64),
            "ARG" => self.arg.clone(),
            "INFINITY" => Value::Float(f64::INFINITY),
            _ => self.fields.get(path).cloned().unwrap_or(Value::Null),
        }
    }
    fn get_field_len(&self, path: &str) -> Option<usize> {
        self.field_lens.get(path).copied()
    }
    fn get_field_len2(&self, path: &str) -> Option<usize> {
        self.field_lens2
            .get(path)
            .copied()
            .or_else(|| self.get_field_len(path))
    }
}

// --- Tokenizer ---

fn pack_version(a: u32, b: u32, c: u32, d: u32) -> u32 {
    (a << 24) | (b << 16) | (c << 8) | d
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, ExprError> {
    let bytes = src.as_bytes();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // whitespace
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // parens
        if c == b'(' {
            out.push(Token::LParen);
            i += 1;
            continue;
        }
        if c == b')' {
            out.push(Token::RParen);
            i += 1;
            continue;
        }
        // #THEN#, #ELSE#, #LEN[...]#, #LEN2[...]#
        if c == b'#' {
            // Try fixed tokens first
            if bytes.len() >= i + 6 && &bytes[i..i + 6] == b"#THEN#" {
                out.push(Token::Then);
                i += 6;
                continue;
            }
            if bytes.len() >= i + 6 && &bytes[i..i + 6] == b"#ELSE#" {
                out.push(Token::Else);
                i += 6;
                continue;
            }
            // #LEN2[name]# and #LEN[name]#
            let is_len2 = bytes.len() >= i + 6 && &bytes[i..i + 6] == b"#LEN2[";
            let is_len = bytes.len() >= i + 5 && &bytes[i..i + 5] == b"#LEN[";
            if is_len2 || is_len {
                let start = i + if is_len2 { 6 } else { 5 };
                let mut j = start;
                while j < bytes.len() && bytes[j] != b']' {
                    j += 1;
                }
                if j >= bytes.len()
                    || bytes[j] != b']'
                    || j + 2 > bytes.len()
                    || bytes[j + 1] != b'#'
                {
                    return Err(ExprError::Tokenize(
                        i,
                        format!("unterminated #LEN[...]# at {}", i),
                    ));
                }
                let name = std::str::from_utf8(&bytes[start..j])
                    .map_err(|e| ExprError::Tokenize(i, e.to_string()))?
                    .trim()
                    .to_string();
                if is_len2 {
                    out.push(Token::Len2(name));
                } else {
                    out.push(Token::Len(name));
                }
                i = j + 2;
                continue;
            }
            // Generic #IDENT# — treat as bare identifier lookup (matches Python evaluator,
            // which silently skips unrecognized '#' chars and parses the inner IDENT).
            let mut j = i + 1;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_alphanumeric() || b == b'_' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j < bytes.len() && bytes[j] == b'#' && j > i + 1 {
                let name = std::str::from_utf8(&bytes[i + 1..j]).unwrap().to_string();
                out.push(Token::Ident(name));
                i = j + 1;
                continue;
            }
            return Err(ExprError::Tokenize(i, format!("unexpected '#' at {}", i)));
        }
        // hex number
        if c == b'0' && i + 1 < bytes.len() && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            let start = i;
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            let slice = std::str::from_utf8(&bytes[start + 2..j]).unwrap();
            let parsed = u64::from_str_radix(slice, 16)
                .map_err(|e| ExprError::Tokenize(start, format!("invalid hex literal: {}", e)))?;
            out.push(Token::NumInt(parsed as i64));
            i = j;
            continue;
        }
        // numeric literal (int / float / version)
        if c.is_ascii_digit() {
            let start = i;
            // Collect leading digits
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            // Check for "a.b.c.d" version (4-part dotted)
            // Peek: must be '.' digits '.' digits '.' digits
            let mut ver_parts: Vec<u32> = Vec::new();
            let mut k = j;
            let mut tmp_parts = 1;
            let head: u32 = std::str::from_utf8(&bytes[i..j]).unwrap().parse().unwrap();
            ver_parts.push(head);
            let save_k = k;
            let mut ok = true;
            while tmp_parts < 4 {
                if k >= bytes.len() || bytes[k] != b'.' {
                    ok = false;
                    break;
                }
                let nstart = k + 1;
                let mut ne = nstart;
                while ne < bytes.len() && bytes[ne].is_ascii_digit() {
                    ne += 1;
                }
                if ne == nstart {
                    ok = false;
                    break;
                }
                let part: u32 = match std::str::from_utf8(&bytes[nstart..ne]).unwrap().parse() {
                    Ok(v) => v,
                    Err(_) => {
                        ok = false;
                        break;
                    }
                };
                ver_parts.push(part);
                k = ne;
                tmp_parts += 1;
            }
            if ok && ver_parts.len() == 4 {
                let packed = pack_version(ver_parts[0], ver_parts[1], ver_parts[2], ver_parts[3]);
                out.push(Token::NumInt(packed as i64));
                i = k;
                continue;
            }
            // Not a 4-part version. Check for float: `.digits` and/or exponent
            let _ = save_k;
            let mut is_float = false;
            let mut fend = j;
            if fend < bytes.len() && bytes[fend] == b'.' {
                let ds = fend + 1;
                let mut de = ds;
                while de < bytes.len() && bytes[de].is_ascii_digit() {
                    de += 1;
                }
                if de > ds {
                    is_float = true;
                    fend = de;
                }
            }
            if fend < bytes.len() && (bytes[fend] == b'e' || bytes[fend] == b'E') {
                let mut es = fend + 1;
                if es < bytes.len() && (bytes[es] == b'+' || bytes[es] == b'-') {
                    es += 1;
                }
                let dstart = es;
                let mut de = dstart;
                while de < bytes.len() && bytes[de].is_ascii_digit() {
                    de += 1;
                }
                if de > dstart {
                    is_float = true;
                    fend = de;
                }
            }
            if is_float {
                let s = std::str::from_utf8(&bytes[start..fend]).unwrap();
                let f: f64 = s.parse().map_err(|e: std::num::ParseFloatError| {
                    ExprError::Tokenize(start, e.to_string())
                })?;
                out.push(Token::NumFloat(f));
                i = fend;
                continue;
            }
            // Plain integer
            let s = std::str::from_utf8(&bytes[start..j]).unwrap();
            let v: i64 = s
                .parse::<i64>()
                .or_else(|_| s.parse::<u64>().map(|u| u as i64))
                .map_err(|e| ExprError::Tokenize(start, e.to_string()))?;
            out.push(Token::NumInt(v));
            i = j;
            continue;
        }
        // Two-char operators first
        if i + 1 < bytes.len() {
            let pair = &bytes[i..i + 2];
            let is_two = matches!(
                pair,
                b"==" | b"!=" | b">=" | b"<=" | b">>" | b"<<" | b"&&" | b"||"
            );
            if is_two {
                out.push(Token::Op2(std::str::from_utf8(pair).unwrap().to_string()));
                i += 2;
                continue;
            }
        }
        // Single-char operators
        if matches!(
            c,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'>' | b'<' | b'!' | b'&' | b'|' | b'^' | b'~'
        ) {
            out.push(Token::Op1(c as char));
            i += 1;
            continue;
        }
        // nif.xml uses "$Field" as a field truthiness shorthand, e.g.
        // "$Name" / "!$Name" on shader material branches. Treat the sigil as
        // syntax and let the following identifier resolve normally.
        if c == b'$' {
            i += 1;
            continue;
        }
        // Identifier: [A-Za-z_][A-Za-z0-9_ ]*(?:\[A-Za-z_]...)* with spaces + backslash paths
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            let mut j = i + 1;
            loop {
                // Consume ident chars and internal spaces
                while j < bytes.len() {
                    let b = bytes[j];
                    if b.is_ascii_alphanumeric() || b == b'_' || b == b' ' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                // Optional \ident segment
                if j < bytes.len() && bytes[j] == b'\\' {
                    // Trim trailing space in current segment
                    while j > start && bytes[j - 1] == b' ' {
                        j -= 1;
                        // shouldn't happen; restart - actually path has \ directly after letter
                        // Allow any positioning — just attempt to consume \ now
                        break;
                    }
                    // Re-examine bytes[j]
                    if j < bytes.len() && bytes[j] == b'\\' {
                        // Next char must start new ident segment
                        if j + 1 < bytes.len()
                            && (bytes[j + 1].is_ascii_alphabetic() || bytes[j + 1] == b'_')
                        {
                            j += 1; // consume '\\'
                            continue; // loop back to eat the next segment
                        }
                    }
                }
                break;
            }
            // Trim trailing whitespace from the identifier
            let mut end = j;
            while end > start && bytes[end - 1] == b' ' {
                end -= 1;
            }
            let name = std::str::from_utf8(&bytes[start..end]).unwrap().to_string();
            out.push(Token::Ident(name));
            i = j;
            continue;
        }
        return Err(ExprError::Tokenize(
            i,
            format!("unexpected char {:?}", c as char),
        ));
    }
    Ok(out)
}

// --- Parser (recursive descent) ---

struct Parser<'a, C: EvalContext> {
    tokens: &'a [Token],
    pos: usize,
    ctx: &'a C,
}

impl<'a, C: EvalContext> Parser<'a, C> {
    fn new(tokens: &'a [Token], ctx: &'a C) -> Self {
        Self {
            tokens,
            pos: 0,
            ctx,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat_op2(&mut self, s: &str) -> bool {
        matches!(self.peek(), Some(Token::Op2(x)) if x == s) && {
            self.pos += 1;
            true
        }
    }
    fn eat_op1(&mut self, c: char) -> bool {
        matches!(self.peek(), Some(Token::Op1(x)) if *x == c) && {
            self.pos += 1;
            true
        }
    }

    fn parse_ternary(&mut self) -> Value {
        let cond = self.parse_logical_or();
        if matches!(self.peek(), Some(Token::Then)) {
            self.pos += 1;
            let then_v = self.parse_logical_or();
            if matches!(self.peek(), Some(Token::Else)) {
                self.pos += 1;
                let else_v = self.parse_logical_or();
                return if cond.as_bool() { then_v } else { else_v };
            }
            return if cond.as_bool() { then_v } else { Value::Null };
        }
        cond
    }

    fn parse_logical_or(&mut self) -> Value {
        let mut left = self.parse_logical_and();
        while self.eat_op2("||") {
            let right = self.parse_logical_and();
            left = Value::Bool(left.as_bool() || right.as_bool());
        }
        left
    }

    fn parse_logical_and(&mut self) -> Value {
        let mut left = self.parse_bitwise_or();
        while self.eat_op2("&&") {
            let right = self.parse_bitwise_or();
            left = Value::Bool(left.as_bool() && right.as_bool());
        }
        left
    }

    fn parse_bitwise_or(&mut self) -> Value {
        let mut left = self.parse_bitwise_xor();
        while self.eat_op1('|') {
            let right = self.parse_bitwise_xor();
            left = Value::Int(left.as_int() | right.as_int());
        }
        left
    }

    fn parse_bitwise_xor(&mut self) -> Value {
        let mut left = self.parse_bitwise_and();
        while self.eat_op1('^') {
            let right = self.parse_bitwise_and();
            left = Value::Int(left.as_int() ^ right.as_int());
        }
        left
    }

    fn parse_bitwise_and(&mut self) -> Value {
        let mut left = self.parse_equality();
        while self.eat_op1('&') {
            let right = self.parse_equality();
            left = Value::Int(left.as_int() & right.as_int());
        }
        left
    }

    fn parse_equality(&mut self) -> Value {
        let mut left = self.parse_relational();
        loop {
            if self.eat_op2("==") {
                let right = self.parse_relational();
                left = Value::Bool(left == right);
            } else if self.eat_op2("!=") {
                let right = self.parse_relational();
                left = Value::Bool(left != right);
            } else {
                break;
            }
        }
        left
    }

    fn parse_relational(&mut self) -> Value {
        let mut left = self.parse_shift();
        loop {
            if self.eat_op2(">=") {
                let right = self.parse_shift();
                left = Value::Bool(left >= right);
            } else if self.eat_op2("<=") {
                let right = self.parse_shift();
                left = Value::Bool(left <= right);
            } else if self.eat_op1('>') {
                let right = self.parse_shift();
                left = Value::Bool(left > right);
            } else if self.eat_op1('<') {
                let right = self.parse_shift();
                left = Value::Bool(left < right);
            } else {
                break;
            }
        }
        left
    }

    fn parse_shift(&mut self) -> Value {
        let mut left = self.parse_additive();
        loop {
            if self.eat_op2(">>") {
                let right = self.parse_additive();
                let l = left.as_int();
                let r = right.as_int();
                left = Value::Int(((l as u64) >> (r as u64 & 63)) as i64);
            } else if self.eat_op2("<<") {
                let right = self.parse_additive();
                let l = left.as_int();
                let r = right.as_int();
                left = Value::Int(((l as u64) << (r as u64 & 63)) as i64);
            } else {
                break;
            }
        }
        left
    }

    fn parse_additive(&mut self) -> Value {
        let mut left = self.parse_multiplicative();
        loop {
            if self.eat_op1('+') {
                let right = self.parse_multiplicative();
                left = numeric_binop(&left, &right, |a, b| a + b, |a, b| a + b);
            } else if self.eat_op1('-') {
                let right = self.parse_multiplicative();
                left = numeric_binop(&left, &right, |a, b| a - b, |a, b| a - b);
            } else {
                break;
            }
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Value {
        let mut left = self.parse_unary();
        loop {
            if self.eat_op1('*') {
                let right = self.parse_unary();
                left = numeric_binop(&left, &right, |a, b| a * b, |a, b| a * b);
            } else if self.eat_op1('/') {
                let right = self.parse_unary();
                if left.is_float() || right.is_float() {
                    left = Value::Float(left.as_float() / right.as_float());
                } else {
                    let r = right.as_int();
                    if r == 0 {
                        left = Value::Int(0);
                    } else {
                        left = Value::Int(left.as_int() / r);
                    }
                }
            } else if self.eat_op1('%') {
                let right = self.parse_unary();
                let r = right.as_int();
                left = Value::Int(if r == 0 { 0 } else { left.as_int() % r });
            } else {
                break;
            }
        }
        left
    }

    fn parse_unary(&mut self) -> Value {
        if self.eat_op1('!') {
            let v = self.parse_unary();
            return Value::Bool(!v.as_bool());
        }
        if self.eat_op1('-') {
            let v = self.parse_unary();
            return if v.is_float() {
                Value::Float(-v.as_float())
            } else {
                Value::Int(-v.as_int())
            };
        }
        if self.eat_op1('~') {
            let v = self.parse_unary();
            return Value::Int(!v.as_int());
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Value {
        let tok = match self.bump() {
            Some(t) => t.clone(),
            None => return Value::Null,
        };
        match tok {
            Token::LParen => {
                let v = self.parse_ternary();
                if matches!(self.peek(), Some(Token::RParen)) {
                    self.pos += 1;
                }
                v
            }
            Token::NumInt(i) => Value::Int(i),
            Token::NumFloat(f) => Value::Float(f),
            Token::Ident(name) => {
                if name == "true" {
                    Value::Bool(true)
                } else if name == "false" {
                    Value::Bool(false)
                } else {
                    self.ctx.get_field(&name)
                }
            }
            Token::Len(name) => match self.ctx.get_field_len(&name) {
                Some(n) => Value::Int(n as i64),
                None => {
                    // Fallback: if the field itself is an Int, reuse as length.
                    let v = self.ctx.get_field(&name);
                    match v {
                        Value::Int(i) => Value::Int(i),
                        _ => Value::Int(0),
                    }
                }
            },
            Token::Len2(name) => match self.ctx.get_field_len2(&name) {
                Some(n) => Value::Int(n as i64),
                None => {
                    let v = self.ctx.get_field(&name);
                    match v {
                        Value::Int(i) => Value::Int(i),
                        _ => Value::Int(0),
                    }
                }
            },
            _ => Value::Null,
        }
    }
}

fn numeric_binop(
    a: &Value,
    b: &Value,
    f_int: fn(i64, i64) -> i64,
    f_float: fn(f64, f64) -> f64,
) -> Value {
    if a.is_float() || b.is_float() {
        Value::Float(f_float(a.as_float(), b.as_float()))
    } else {
        Value::Int(f_int(a.as_int(), b.as_int()))
    }
}

// --- Public API ---

#[derive(Debug, Clone)]
pub struct NifExpr {
    pub source: String,
    tokens: Vec<Token>,
}

static EXPR_CACHE: Lazy<Mutex<HashMap<String, Arc<NifExpr>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static THREAD_EXPR_CACHE: RefCell<HashMap<String, Arc<NifExpr>>> = RefCell::new(HashMap::new());
}

impl NifExpr {
    pub fn parse(src: &str) -> Result<Self, ExprError> {
        let trimmed = src.trim();
        let tokens = tokenize(trimmed)?;
        Ok(Self {
            source: trimmed.to_string(),
            tokens,
        })
    }

    pub fn cached(src: &str) -> Result<Arc<Self>, ExprError> {
        let trimmed = src.trim();
        if let Some(expr) = THREAD_EXPR_CACHE.with(|cache| cache.borrow().get(trimmed).cloned()) {
            return Ok(expr);
        }

        if let Some(expr) = EXPR_CACHE.lock().unwrap().get(trimmed).cloned() {
            THREAD_EXPR_CACHE.with(|cache| {
                cache.borrow_mut().insert(trimmed.to_string(), expr.clone());
            });
            return Ok(expr);
        }

        let expr = Arc::new(Self::parse(trimmed)?);
        EXPR_CACHE
            .lock()
            .unwrap()
            .insert(trimmed.to_string(), expr.clone());
        THREAD_EXPR_CACHE.with(|cache| {
            cache.borrow_mut().insert(trimmed.to_string(), expr.clone());
        });
        Ok(expr)
    }

    pub fn evaluate<C: EvalContext>(&self, ctx: &C) -> Value {
        let mut p = Parser::new(&self.tokens, ctx);
        p.parse_ternary()
    }

    pub fn evaluate_bool<C: EvalContext>(&self, ctx: &C) -> bool {
        self.evaluate(ctx).as_bool()
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fields: &[(&str, i64)]) -> MapContext {
        MapContext::from_pairs(
            &fields
                .iter()
                .map(|(k, v)| (*k, Value::Int(*v)))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn version_compare_packed_literal() {
        // "Version >= 20.2.0.7" — version literal packs to 0x14020007
        let expr = NifExpr::parse("Version >= 20.2.0.7").unwrap();
        let c = ctx(&[("Version", 0x14020007)]);
        assert!(expr.evaluate_bool(&c));
        let c2 = ctx(&[("Version", 0x14020006)]);
        assert!(!expr.evaluate_bool(&c2));
    }

    #[test]
    fn version_compare_explicit() {
        let expr = NifExpr::parse("Version == 335675399").unwrap();
        let c = ctx(&[("Version", 0x14020007)]);
        assert!(expr.evaluate_bool(&c));
    }

    #[test]
    fn logical_and_with_backslash_path() {
        let expr =
            NifExpr::parse("(BS Header\\BS Version >= 130) && (BS Header\\BS Version <= 139)")
                .unwrap();
        let c = ctx(&[("BS Header\\BS Version", 130)]);
        assert!(expr.evaluate_bool(&c));
        let c2 = ctx(&[("BS Header\\BS Version", 140)]);
        assert!(!expr.evaluate_bool(&c2));
    }

    #[test]
    fn logical_or() {
        let expr = NifExpr::parse("(User Version == 11) || (User Version == 12)").unwrap();
        let c = ctx(&[("User Version", 12)]);
        assert!(expr.evaluate_bool(&c));
        let c2 = ctx(&[("User Version", 0)]);
        assert!(!expr.evaluate_bool(&c2));
    }

    #[test]
    fn bitshift_calc() {
        let expr = NifExpr::parse("Vertex Desc >> 44").unwrap();
        let c = ctx(&[("Vertex Desc", 1i64 << 44)]);
        assert_eq!(expr.evaluate(&c).as_int(), 1);
    }

    #[test]
    fn bitand_calc() {
        // Mimic BSTriShape Data Size calc fragment: (Vertex Desc & 0xF)
        let expr = NifExpr::parse("Vertex Desc & 0xF").unwrap();
        let c = ctx(&[("Vertex Desc", 0x5Ai64)]);
        assert_eq!(expr.evaluate(&c).as_int(), 0xA);
    }

    #[test]
    fn arithmetic_precedence() {
        let expr = NifExpr::parse("2 + 3 * 4").unwrap();
        let c = MapContext::new();
        assert_eq!(expr.evaluate(&c).as_int(), 14);
    }

    #[test]
    fn parens_override_precedence() {
        let expr = NifExpr::parse("(2 + 3) * 4").unwrap();
        let c = MapContext::new();
        assert_eq!(expr.evaluate(&c).as_int(), 20);
    }

    #[test]
    fn ternary_true_branch() {
        let expr = NifExpr::parse("Num Verts #THEN# Num Verts #ELSE# 0").unwrap();
        let c = ctx(&[("Num Verts", 5)]);
        assert_eq!(expr.evaluate(&c).as_int(), 5);
    }

    #[test]
    fn ternary_false_branch() {
        let expr = NifExpr::parse("Num Verts #THEN# Num Verts #ELSE# 42").unwrap();
        let c = ctx(&[("Num Verts", 0)]);
        assert_eq!(expr.evaluate(&c).as_int(), 42);
    }

    #[test]
    fn missing_field_is_null_eq_zero() {
        let expr = NifExpr::parse("NonExistent == 0").unwrap();
        let c = MapContext::new();
        assert!(expr.evaluate_bool(&c));
    }

    #[test]
    fn not_operator() {
        let expr = NifExpr::parse("!Flag").unwrap();
        let c = ctx(&[("Flag", 0)]);
        assert!(expr.evaluate_bool(&c));
        let c2 = ctx(&[("Flag", 1)]);
        assert!(!expr.evaluate_bool(&c2));
    }

    #[test]
    fn dollar_field_truthiness_sigil_is_ignored() {
        let expr = NifExpr::parse("!$Name").unwrap();
        let c = ctx(&[("Name", 0)]);
        assert!(expr.evaluate_bool(&c));
        let c2 = ctx(&[("Name", 1)]);
        assert!(!expr.evaluate_bool(&c2));
    }

    #[test]
    fn unary_minus() {
        let expr = NifExpr::parse("-5 + 10").unwrap();
        let c = MapContext::new();
        assert_eq!(expr.evaluate(&c).as_int(), 5);
    }

    #[test]
    fn hex_literal() {
        let expr = NifExpr::parse("Flags & 0xFF").unwrap();
        let c = ctx(&[("Flags", 0x1234)]);
        assert_eq!(expr.evaluate(&c).as_int(), 0x34);
    }

    #[test]
    fn version_context_resolves_globals() {
        let mut c = VersionContext::new(0x14020007, 12, 130);
        c.fields.insert("Num Vertices".to_string(), Value::Int(5));
        let expr = NifExpr::parse(
            "(BS Header\\BS Version >= 130) && (Version == 20.2.0.7) && (Num Vertices > 0)",
        )
        .unwrap();
        assert!(expr.evaluate_bool(&c));
    }

    #[test]
    fn tokenize_backslash_path() {
        let toks = tokenize("BS Header\\BS Version >= 130").unwrap();
        // First token should be the full path identifier.
        match &toks[0] {
            Token::Ident(s) => assert_eq!(s, "BS Header\\BS Version"),
            other => panic!("expected Ident, got {:?}", other),
        }
    }

    #[test]
    fn tokenize_version_literal_packed() {
        let toks = tokenize("20.2.0.7").unwrap();
        assert_eq!(toks, vec![Token::NumInt(0x14020007)]);
    }

    #[test]
    fn tokenize_float_vs_version() {
        // 3.14 is float (only 2 parts)
        let toks = tokenize("3.14").unwrap();
        match &toks[0] {
            Token::NumFloat(f) => assert!((f - 3.14).abs() < 1e-9),
            t => panic!("expected float, got {:?}", t),
        }
    }

    #[test]
    fn len_token() {
        let expr = NifExpr::parse("#LEN[Children]# > 0").unwrap();
        let mut c = VersionContext::new(0, 0, 0);
        c.field_lens.insert("Children".to_string(), 3);
        assert!(expr.evaluate_bool(&c));
    }

    #[test]
    fn len2_token_uses_second_dimension_length() {
        let mut c = VersionContext::new(0, 0, 0);
        c.field_lens.insert("Strips".to_string(), 1);
        c.field_lens2.insert("Strips".to_string(), 5);

        let len = NifExpr::parse("#LEN[Strips]#").unwrap();
        let len2 = NifExpr::parse("#LEN2[Strips]#").unwrap();

        assert_eq!(len.evaluate(&c).as_int(), 1);
        assert_eq!(len2.evaluate(&c).as_int(), 5);
    }
}
