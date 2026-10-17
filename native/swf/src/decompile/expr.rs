//! AS3 syntax tree recovered from AVM2 code, and its printer.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Lit {
    Int(i64),
    Num(f64),
    Str(String),
    True,
    False,
    Null,
    Undefined,
    NaN,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Ushr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
    InstanceOf,
    Is,
    As,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    BitNot,
    TypeOf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Name(String),
    Attr(String),
    Descendants(String),
    Index(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Lit(Lit),
    This,
    Local(u32),
    /// A lexical name: `getlex`, or a `findprop` object combined with its property.
    Name(String),
    /// The object `findpropstrict`/`findproperty` resolved for a name; its property prints bare.
    Scope(String),
    Global,
    Prop(Box<Expr>, Key),
    Super(Key),
    Call(Box<Expr>, Vec<Expr>),
    New(Box<Expr>, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Update { target: Box<Expr>, inc: bool, prefix: bool },
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Array(Vec<Expr>),
    Object(Vec<(Expr, Expr)>),
    Delete(Box<Expr>),
    /// A closure, already printed with its own locals.
    Function(String),
    TypeArgs(Box<Expr>, Vec<Expr>),
    /// `new <T>[a, b]`: the element type and items.
    Vector(Box<Expr>, Vec<Expr>),
    /// The E4X filter `list.(condition)`.
    Filter(Box<Expr>, Box<Expr>),
    /// An implicit conversion: printed transparently, its type names a local's declaration.
    Coerce(String, Box<Expr>),
    /// The value a block found on the operand stack at entry, until a predecessor supplies it.
    In(usize),
    HasNext(u32, u32),
    NextName(Box<Expr>, Box<Expr>),
    NextValue(Box<Expr>, Box<Expr>),
    Activation,
    CatchScope(u32),
    Raw(String),
}

impl Expr {
    pub fn int(value: i64) -> Self {
        Expr::Lit(Lit::Int(value))
    }

    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Self {
        Expr::Binary(op, Box::new(a), Box::new(b))
    }

    pub fn stripped(&self) -> &Expr {
        match self {
            Expr::Coerce(_, inner) => inner.stripped(),
            other => other,
        }
    }

    pub fn has_effects(&self) -> bool {
        match self {
            Expr::Call(..) | Expr::New(..) | Expr::Assign(..) | Expr::Update { .. } | Expr::Delete(_) => true,
            Expr::Coerce(_, e) | Expr::Unary(_, e) => e.has_effects(),
            Expr::Binary(_, a, b) => a.has_effects() || b.has_effects(),
            Expr::Prop(o, k) => o.has_effects() || matches!(k, Key::Index(i) if i.has_effects()),
            Expr::Ternary(c, a, b) => c.has_effects() || a.has_effects() || b.has_effects(),
            Expr::Array(items) | Expr::Vector(_, items) => items.iter().any(Expr::has_effects),
            Expr::Object(pairs) => pairs.iter().any(|(k, v)| k.has_effects() || v.has_effects()),
            _ => false,
        }
    }

    /// Replaces each block-entry placeholder with the value its predecessor left.
    pub fn substitute(&self, values: &[Expr]) -> Expr {
        self.map(&|e| match e {
            Expr::In(i) => values.get(*i).cloned(),
            _ => None,
        })
    }

    pub fn map(&self, f: &dyn Fn(&Expr) -> Option<Expr>) -> Expr {
        if let Some(replaced) = f(self) {
            return replaced;
        }
        let m = |e: &Expr| Box::new(e.map(f));
        let key = |k: &Key| match k {
            Key::Index(i) => Key::Index(m(i)),
            other => other.clone(),
        };
        match self {
            Expr::Prop(o, k) => Expr::Prop(m(o), key(k)),
            Expr::Super(k) => Expr::Super(key(k)),
            Expr::Call(c, a) => Expr::Call(m(c), a.iter().map(|e| e.map(f)).collect()),
            Expr::New(c, a) => Expr::New(m(c), a.iter().map(|e| e.map(f)).collect()),
            Expr::Unary(op, e) => Expr::Unary(*op, m(e)),
            Expr::Binary(op, a, b) => Expr::Binary(*op, m(a), m(b)),
            Expr::Assign(t, v) => Expr::Assign(m(t), m(v)),
            Expr::Update { target, inc, prefix } => Expr::Update { target: m(target), inc: *inc, prefix: *prefix },
            Expr::Ternary(c, a, b) => Expr::Ternary(m(c), m(a), m(b)),
            Expr::Array(items) => Expr::Array(items.iter().map(|e| e.map(f)).collect()),
            Expr::Object(pairs) => Expr::Object(pairs.iter().map(|(k, v)| (k.map(f), v.map(f))).collect()),
            Expr::Delete(e) => Expr::Delete(m(e)),
            Expr::TypeArgs(b, a) => Expr::TypeArgs(m(b), a.iter().map(|e| e.map(f)).collect()),
            Expr::Vector(t, items) => Expr::Vector(m(t), items.iter().map(|e| e.map(f)).collect()),
            Expr::Filter(list, cond) => Expr::Filter(m(list), m(cond)),
            Expr::Coerce(t, e) => Expr::Coerce(t.clone(), m(e)),
            Expr::NextName(o, i) => Expr::NextName(m(o), m(i)),
            Expr::NextValue(o, i) => Expr::NextValue(m(o), m(i)),
            other => other.clone(),
        }
    }

    /// Whether `f` holds for this expression or any part of it.
    pub fn any(&self, f: &dyn Fn(&Expr) -> bool) -> bool {
        let found = std::cell::Cell::new(false);
        self.map(&|e| {
            if !found.get() && f(e) {
                found.set(true);
            }
            None
        });
        found.get()
    }
}

/// `c ? a : b`, or `&&`/`||` when a branch is the condition's own value.
pub fn ternary(c: Expr, a: Expr, b: Expr) -> Expr {
    let same = |x: &Expr, y: &Expr| x.stripped() == y.stripped();
    let opposite = |x: &Expr, y: &Expr| same(&negate(x.clone()), y) || same(x, &negate(y.clone()));
    if same(&c, &a) {
        Expr::bin(BinOp::Or, a, b)
    } else if opposite(&c, &a) {
        Expr::bin(BinOp::And, a, b)
    } else if same(&c, &b) {
        Expr::bin(BinOp::And, b, a)
    } else if opposite(&c, &b) {
        Expr::bin(BinOp::Or, b, a)
    } else {
        Expr::Ternary(Box::new(c), Box::new(a), Box::new(b))
    }
}

/// The type a value plainly has, for declaring the local it initializes.
pub fn value_type(e: &Expr) -> Option<String> {
    match e {
        Expr::Coerce(kind, _) => Some(kind.clone()),
        Expr::Lit(Lit::NaN) => Some("Number".into()),
        Expr::Lit(Lit::Num(_)) => Some("Number".into()),
        Expr::Lit(Lit::Str(_)) => Some("String".into()),
        Expr::Lit(Lit::True | Lit::False) | Expr::Unary(UnOp::Not, _) => Some("Boolean".into()),
        Expr::Binary(op, ..) if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe | BinOp::Lt | BinOp::Le
            | BinOp::Gt | BinOp::Ge | BinOp::In | BinOp::InstanceOf | BinOp::Is) => Some("Boolean".into()),
        _ => None,
    }
}

fn default_value(kind: &str) -> &'static str {
    match kind {
        "int" | "uint" => "0",
        "Number" => "NaN",
        "Boolean" => "false",
        "*" => "undefined",
        _ => "null",
    }
}

/// The condition that is true exactly when `e` is false, kept readable.
pub fn negate(e: Expr) -> Expr {
    match e {
        Expr::Unary(UnOp::Not, inner) => *inner,
        Expr::Lit(Lit::True) => Expr::Lit(Lit::False),
        Expr::Lit(Lit::False) => Expr::Lit(Lit::True),
        Expr::Binary(op, a, b) => {
            let flipped = match op {
                BinOp::Eq => Some(BinOp::Ne),
                BinOp::Ne => Some(BinOp::Eq),
                BinOp::StrictEq => Some(BinOp::StrictNe),
                BinOp::StrictNe => Some(BinOp::StrictEq),
                _ => None,
            };
            match flipped {
                Some(op) => Expr::Binary(op, a, b),
                None => Expr::Unary(UnOp::Not, Box::new(Expr::Binary(op, a, b))),
            }
        }
        Expr::Coerce(_, inner) if matches!(*inner, Expr::Binary(..) | Expr::Unary(UnOp::Not, _)) => negate(*inner),
        other => Expr::Unary(UnOp::Not, Box::new(other)),
    }
}

/// `negate` that also distributes over `&&` and `||`, for conditions joined from jumps.
pub fn invert(e: Expr) -> Expr {
    match e {
        Expr::Binary(BinOp::And, a, b) => Expr::bin(BinOp::Or, invert(*a), invert(*b)),
        Expr::Binary(BinOp::Or, a, b) => Expr::bin(BinOp::And, invert(*a), invert(*b)),
        other => negate(other),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Expr(Expr),
    Return(Option<Expr>),
    Throw(Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    DoWhile(Vec<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Vec<Stmt>, Vec<Stmt>),
    ForIn { each: bool, var: Expr, object: Expr, body: Vec<Stmt> },
    /// Each case: its labels (`None` is `default`) and body.
    Switch(Expr, Vec<(Vec<Option<Expr>>, Vec<Stmt>)>),
    Break(Option<String>),
    Continue(Option<String>),
    Labeled(String, Box<Stmt>),
    Try(Vec<Stmt>, Vec<Catch>),
    Comment(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    pub name: String,
    pub kind: String,
    pub body: Vec<Stmt>,
}

fn binary_precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Mul | BinOp::Div | BinOp::Mod => 14,
        BinOp::Add | BinOp::Sub => 13,
        BinOp::Shl | BinOp::Shr | BinOp::Ushr => 12,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In | BinOp::InstanceOf | BinOp::Is | BinOp::As => 11,
        BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe => 10,
        BinOp::BitAnd => 9,
        BinOp::BitXor => 8,
        BinOp::BitOr => 7,
        BinOp::And => 6,
        BinOp::Or => 5,
    }
}

fn binary_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Ushr => ">>>",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::StrictEq => "===",
        BinOp::StrictNe => "!==",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::In => "in",
        BinOp::InstanceOf => "instanceof",
        BinOp::Is => "is",
        BinOp::As => "as",
    }
}

fn precedence(e: &Expr) -> u8 {
    match e {
        Expr::Coerce(_, inner) => precedence(inner),
        Expr::Prop(..) | Expr::Super(_) | Expr::Call(..) | Expr::TypeArgs(..) | Expr::New(..) | Expr::Filter(..) => 19,
        Expr::Vector(..) => 18,
        Expr::Update { prefix: false, .. } => 17,
        Expr::Update { prefix: true, .. } | Expr::Unary(..) | Expr::Delete(_) => 16,
        Expr::Binary(op, ..) => binary_precedence(*op),
        Expr::Ternary(..) => 4,
        Expr::Assign(..) => 3,
        Expr::Lit(Lit::Int(v)) if *v < 0 => 16,
        Expr::Lit(Lit::Num(v)) if *v < 0.0 => 16,
        _ => 20,
    }
}

pub fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn number(value: f64) -> String {
    if value.is_nan() {
        "NaN".into()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else {
        format!("{value}")
    }
}

pub fn literal(lit: &Lit) -> String {
    match lit {
        Lit::Int(v) => v.to_string(),
        Lit::Num(v) => number(*v),
        Lit::Str(s) => quote(s),
        Lit::True => "true".into(),
        Lit::False => "false".into(),
        Lit::Null => "null".into(),
        Lit::Undefined => "undefined".into(),
        Lit::NaN => "NaN".into(),
    }
}

/// `new RegExp("p", "f")` as the literal `/p/f` source wrote.
fn regex_literal(ctor: &Expr, args: &[Expr]) -> Option<String> {
    if *ctor.stripped() != Expr::Name("RegExp".into()) {
        return None;
    }
    let text = |e: &Expr| match e.stripped() {
        Expr::Lit(Lit::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let (pattern, flags) = match args {
        [p] => (text(p)?, String::new()),
        [p, f] => (text(p)?, text(f)?),
        _ => return None,
    };
    if pattern.is_empty() || pattern.contains(['\n', '\r']) || !flags.chars().all(|c| "gimsx".contains(c)) {
        return None;
    }
    let mut out = String::from("/");
    let mut escaped = false;
    for c in pattern.chars() {
        if c == '/' && !escaped {
            out.push('\\');
        }
        escaped = c == '\\' && !escaped;
        out.push(c);
    }
    Some(format!("{out}/{flags}"))
}

/// Prints one method body. Locals are declared with `var` where first assigned as a statement;
/// the rest at the top.
pub struct Printer<'a> {
    locals: &'a Locals,
    /// Registers that are parameters, `this` or otherwise never declared.
    pub predeclared: HashSet<u32>,
    declared: HashSet<u32>,
    referenced: BTreeSet<u32>,
    declared_vars: HashSet<String>,
    referenced_vars: BTreeSet<String>,
    out: String,
    depth: usize,
    base: usize,
}

/// What a method body's locals are called and declared as.
#[derive(Clone, Default)]
pub struct Locals {
    pub names: HashMap<u32, String>,
    /// Declared types of registers, from the conversions their stores carry.
    pub types: HashMap<u32, String>,
    /// Variables kept in the activation object (methods with closures), with their types.
    pub vars: HashMap<String, String>,
}

impl<'a> Printer<'a> {
    pub fn new(locals: &'a Locals, predeclared: HashSet<u32>, depth: usize) -> Self {
        Self {
            locals,
            predeclared,
            declared: HashSet::new(),
            referenced: BTreeSet::new(),
            declared_vars: HashSet::new(),
            referenced_vars: BTreeSet::new(),
            out: String::new(),
            depth,
            base: depth,
        }
    }

    pub fn finish(self) -> String {
        let mut head = String::new();
        let indent = "    ".repeat(self.base);
        for r in &self.referenced {
            if !self.declared.contains(r) && !self.predeclared.contains(r) {
                let kind = self.locals.types.get(r).map_or("*", String::as_str);
                let _ = writeln!(head, "{indent}var {}:{kind} = {};", self.local_name(*r), default_value(kind));
            }
        }
        for name in &self.referenced_vars {
            if !self.declared_vars.contains(name) {
                let kind = &self.locals.vars[name];
                let _ = writeln!(head, "{indent}var {name}:{kind} = {};", default_value(kind));
            }
        }
        head + &self.out
    }

    fn local_name(&self, register: u32) -> String {
        self.locals.names.get(&register).cloned().unwrap_or_else(|| format!("_loc{register}_"))
    }

    fn local(&mut self, register: u32) -> String {
        self.referenced.insert(register);
        self.local_name(register)
    }

    fn key(&mut self, key: &Key) -> String {
        match key {
            Key::Name(n) if is_identifier(n) => format!(".{n}"),
            Key::Name(n) => format!("[{}]", quote(n)),
            Key::Attr(n) => format!(".@{n}"),
            Key::Descendants(n) => format!("..{n}"),
            Key::Index(e) => format!("[{}]", self.expr(e, 0)),
        }
    }

    fn operand(&mut self, e: &Expr, min: u8) -> String {
        let text = self.expr(e, 0);
        if precedence(e) < min { format!("({text})") } else { text }
    }

    fn args(&mut self, args: &[Expr]) -> String {
        args.iter().map(|a| self.expr(a, 3)).collect::<Vec<_>>().join(", ")
    }

    /// `min` is the lowest precedence that can stand here without parentheses.
    pub fn expr(&mut self, e: &Expr, min: u8) -> String {
        let text = match e {
            Expr::Coerce(_, inner) => return self.expr(inner, min),
            Expr::Lit(lit) => literal(lit),
            Expr::This => "this".into(),
            Expr::Local(r) => self.local(*r),
            Expr::Name(n) => {
                if self.locals.vars.contains_key(n) {
                    self.referenced_vars.insert(n.clone());
                }
                n.clone()
            }
            Expr::Scope(n) => n.clone(),
            Expr::Global => "global".into(),
            Expr::Prop(obj, key) => match (obj.stripped(), key) {
                (Expr::Scope(_) | Expr::Global, Key::Name(n)) => n.clone(),
                (Expr::Activation | Expr::CatchScope(_), Key::Name(n)) => n.clone(),
                _ => {
                    let target = self.operand(obj, 19);
                    let target = if matches!(obj.stripped(), Expr::Lit(Lit::Int(_) | Lit::Num(_))) {
                        format!("({target})")
                    } else {
                        target
                    };
                    format!("{target}{}", self.key(key))
                }
            },
            Expr::Super(key) => format!("super{}", self.key(key)),
            Expr::Call(callee, args) => {
                let target = if matches!(callee.stripped(), Expr::Function(_)) {
                    format!("({})", self.expr(callee, 0))
                } else {
                    self.operand(callee, 19)
                };
                format!("{target}({})", self.args(args))
            }
            Expr::New(ctor, args) if let Some(regex) = regex_literal(ctor, args) => regex,
            Expr::New(ctor, args) => {
                let target = if matches!(ctor.stripped(), Expr::Call(..)) {
                    format!("({})", self.expr(ctor, 0))
                } else {
                    self.operand(ctor, 19)
                };
                format!("new {target}({})", self.args(args))
            }
            Expr::Unary(op, inner) => {
                let symbol = match op {
                    UnOp::Not => "!",
                    UnOp::Neg => "-",
                    UnOp::BitNot => "~",
                    UnOp::TypeOf => "typeof ",
                };
                format!("{symbol}{}", self.operand(inner, 16))
            }
            Expr::Binary(op, a, b) => {
                let p = binary_precedence(*op);
                // && and || are associative, so a right operand of the same operator needs no parentheses.
                let right = if matches!(op, BinOp::And | BinOp::Or)
                    && matches!(b.stripped(), Expr::Binary(inner, ..) if inner == op)
                {
                    p
                } else {
                    p + 1
                };
                format!("{} {} {}", self.operand(a, p), binary_symbol(*op), self.operand(b, right))
            }
            Expr::Assign(target, value) => self.assignment(target, value, false),
            Expr::Update { target, inc, prefix } => {
                let symbol = if *inc { "++" } else { "--" };
                let t = self.operand(target, 19);
                if *prefix { format!("{symbol}{t}") } else { format!("{t}{symbol}") }
            }
            // A nested conditional reads better in parentheses.
            Expr::Ternary(c, a, b) => format!("{} ? {} : {}", self.operand(c, 5), self.operand(a, 5), self.operand(b, 5)),
            Expr::Array(items) => format!("[{}]", self.args(items)),
            Expr::Object(pairs) => {
                let fields: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| {
                        let key = match k.stripped() {
                            Expr::Lit(Lit::Str(s)) if is_identifier(s) => s.clone(),
                            Expr::Lit(Lit::Str(s)) => quote(s),
                            other => self.expr(other, 20),
                        };
                        format!("{key}:{}", self.expr(v, 3))
                    })
                    .collect();
                format!("{{{}}}", fields.join(", "))
            }
            Expr::Delete(target) => format!("delete {}", self.operand(target, 19)),
            Expr::Function(text) => text.clone(),
            Expr::TypeArgs(base, args) => format!("{}.<{}>", self.operand(base, 19), self.args(args)),
            Expr::Vector(kind, items) => format!("new <{}>[{}]", self.expr(kind, 0), self.args(items)),
            Expr::Filter(list, cond) => format!("{}.({})", self.operand(list, 19), self.expr(cond, 0)),
            Expr::In(i) => format!("§§stack[{i}]"),
            Expr::HasNext(o, i) => format!("hasnext({}, {})", self.local(*o), self.local(*i)),
            Expr::NextName(o, i) => format!("nextname({}, {})", self.expr(o, 3), self.expr(i, 3)),
            Expr::NextValue(o, i) => format!("nextvalue({}, {})", self.expr(o, 3), self.expr(i, 3)),
            Expr::Activation => "§§activation".into(),
            Expr::CatchScope(_) => "§§catch".into(),
            Expr::Raw(text) => text.clone(),
        };
        if precedence(e) < min { format!("({text})") } else { text }
    }

    fn declaration(&mut self, target: &Expr, value: Option<&Expr>) -> Option<String> {
        if let Expr::Name(n) = target
            && let Some(kind) = self.locals.vars.get(n)
            && self.declared_vars.insert(n.clone())
        {
            return Some(format!("var {n}:{kind}"));
        }
        let Expr::Local(r) = target else { return None };
        if self.predeclared.contains(r) || !self.declared.insert(*r) {
            return None;
        }
        let _ = value;
        let kind = self.locals.types.get(r).cloned().unwrap_or_else(|| "*".into());
        Some(format!("var {}:{kind}", self.local(*r)))
    }

    fn assignment(&mut self, target: &Expr, value: &Expr, statement: bool) -> String {
        if statement && let Some(declared) = self.declaration(target, Some(value)) {
            return format!("{declared} = {}", self.expr(value, 3));
        }
        if let Expr::Binary(op, left, right) = value.stripped()
            && left.stripped() == target.stripped()
            && !matches!(op, BinOp::Eq | BinOp::Ne | BinOp::StrictEq | BinOp::StrictNe
                | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::In | BinOp::InstanceOf | BinOp::Is | BinOp::As)
        {
            let t = self.operand(target, 19);
            if matches!(op, BinOp::Add | BinOp::Sub) && matches!(right.stripped(), Expr::Lit(Lit::Int(1))) {
                let symbol = if *op == BinOp::Add { "++" } else { "--" };
                // Statements keep the compiler's spelling: `i++` on locals, `++obj.p` on properties.
                let postfix = statement && matches!(target.stripped(), Expr::Local(_));
                return if postfix { format!("{t}{symbol}") } else { format!("{symbol}{t}") };
            }
            return format!("{t} {}= {}", binary_symbol(*op), self.expr(right, 3));
        }
        format!("{} = {}", self.operand(target, 19), self.expr(value, 3))
    }

    fn line(&mut self, text: &str) {
        for _ in 0..self.depth {
            self.out.push_str("    ");
        }
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn block(&mut self, head: &str, body: &[Stmt]) {
        self.line(head);
        self.line("{");
        self.depth += 1;
        self.stmts(body);
        self.depth -= 1;
        self.line("}");
    }

    pub fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn simple(&mut self, e: &Expr) -> String {
        match e {
            Expr::Assign(t, v) => self.assignment(t, v, true),
            Expr::Coerce(_, inner) => self.simple(inner),
            other => self.expr(other, 0),
        }
    }

    pub fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Expr(e) => {
                let text = self.simple(e);
                self.line(&format!("{text};"));
            }
            Stmt::Return(None) => self.line("return;"),
            Stmt::Return(Some(e)) => {
                let text = self.expr(e, 0);
                self.line(&format!("return {text};"));
            }
            Stmt::Throw(e) => {
                let text = self.expr(e, 0);
                self.line(&format!("throw {text};"));
            }
            Stmt::If(cond, then, otherwise) => {
                let head = format!("if({})", self.expr(cond, 0));
                self.block(&head, then);
                match otherwise.as_slice() {
                    [] => {}
                    [Stmt::If(..)] => {
                        // `else if` keeps chains flat.
                        let mut nested = Printer::new(self.locals, self.predeclared.clone(), self.depth);
                        nested.declared = std::mem::take(&mut self.declared);
                        nested.declared_vars = std::mem::take(&mut self.declared_vars);
                        nested.stmt(&otherwise[0]);
                        self.declared = nested.declared;
                        self.declared_vars = nested.declared_vars;
                        self.referenced.extend(nested.referenced);
                        self.referenced_vars.extend(nested.referenced_vars);
                        let text = nested.out;
                        let indent = "    ".repeat(self.depth);
                        let trimmed = text.strip_prefix(&indent).unwrap_or(&text);
                        self.out.push_str(&indent);
                        self.out.push_str("else ");
                        self.out.push_str(trimmed);
                    }
                    _ => self.block("else", otherwise),
                }
            }
            Stmt::While(cond, body) => {
                let head = format!("while({})", self.expr(cond, 0));
                self.block(&head, body);
            }
            Stmt::DoWhile(body, cond) => {
                self.line("do");
                self.line("{");
                self.depth += 1;
                self.stmts(body);
                self.depth -= 1;
                self.line("}");
                let text = self.expr(cond, 0);
                self.line(&format!("while({text});"));
            }
            Stmt::For(init, cond, update, body) => {
                let init = match init.as_deref() {
                    Some(Stmt::Expr(e)) => self.simple(e),
                    _ => String::new(),
                };
                let cond = cond.as_ref().map(|c| self.expr(c, 0)).unwrap_or_default();
                let update: Vec<String> = update
                    .iter()
                    .filter_map(|s| if let Stmt::Expr(e) = s { Some(self.simple(e)) } else { None })
                    .collect();
                let head = format!("for({init}; {cond}; {})", update.join(", "));
                self.block(&head, body);
            }
            Stmt::ForIn { each, var, object, body } => {
                let var = match var {
                    Expr::Local(r) if !self.predeclared.contains(r) && self.declared.insert(*r) => {
                        format!("var {}", self.local(*r))
                    }
                    other => self.expr(other, 19),
                };
                let head = format!("for{}({var} in {})", if *each { " each" } else { "" }, self.expr(object, 0));
                self.block(&head, body);
            }
            Stmt::Switch(subject, cases) => {
                let head = format!("switch({})", self.expr(subject, 0));
                self.line(&head);
                self.line("{");
                self.depth += 1;
                for (labels, body) in cases {
                    for label in labels {
                        let text = match label {
                            Some(v) => format!("case {}:", self.expr(v, 0)),
                            None => "default:".into(),
                        };
                        self.line(&text);
                    }
                    self.depth += 1;
                    self.stmts(body);
                    self.depth -= 1;
                }
                self.depth -= 1;
                self.line("}");
            }
            Stmt::Break(None) => self.line("break;"),
            Stmt::Break(Some(l)) => self.line(&format!("break {l};")),
            Stmt::Continue(None) => self.line("continue;"),
            Stmt::Continue(Some(l)) => self.line(&format!("continue {l};")),
            Stmt::Labeled(label, inner) => {
                self.line(&format!("{label}:"));
                self.stmt(inner);
            }
            Stmt::Try(body, catches) => {
                self.block("try", body);
                for c in catches {
                    let head = format!("catch({}:{})", c.name, c.kind);
                    self.block(&head, &c.body);
                }
            }
            Stmt::Comment(text) => {
                for line in text.lines() {
                    self.line(&format!("// {line}"));
                }
            }
        }
    }
}
