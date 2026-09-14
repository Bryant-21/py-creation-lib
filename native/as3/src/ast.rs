//! Abstract syntax tree.
//!
//! swftools emits bytecode during grammar reduction and needs two passes over
//! the source text for forward references. A whole-program tree lets later
//! passes resolve names by walking it instead of re-lexing.

use crate::diag::Span;

/// A dotted name as written: `flash.display.MovieClip` is `["flash",
/// "display", "MovieClip"]`. Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DottedName {
    pub parts: Vec<String>,
    pub span: Span,
}

impl DottedName {
    /// The last component — the class or member being named.
    pub fn last(&self) -> &str {
        self.parts.last().expect("dotted name is never empty")
    }

    /// Everything before the last component, joined with dots. Empty for a bare
    /// name, which is how the unnamed (top-level) package is spelled.
    pub fn package(&self) -> String {
        self.parts[..self.parts.len() - 1].join(".")
    }

    pub fn joined(&self) -> String {
        self.parts.join(".")
    }
}

/// A type annotation. `*` (the any type) and a missing annotation are distinct
/// in source but identical in ABC — both are multiname index 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Any,
    Void,
    Named(DottedName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub visibility: Option<Visibility>,
    pub is_static: bool,
    pub is_final: bool,
    pub is_dynamic: bool,
    pub is_override: bool,
    pub is_native: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_ref: TypeRef,
    /// Present for `function f(x:int = 3)`. Rest parameters (`...rest`) set
    /// [`Param::is_rest`] instead.
    pub default: Option<Expr>,
    pub is_rest: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSig {
    pub params: Vec<Param>,
    pub return_type: TypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accessor {
    None,
    Getter,
    Setter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub modifiers: Modifiers,
    pub accessor: Accessor,
    pub name: String,
    pub sig: FunctionSig,
    /// `None` for an interface method or a `native` declaration — a signature
    /// with no body.
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub modifiers: Modifiers,
    pub is_const: bool,
    pub name: String,
    pub type_ref: TypeRef,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Member {
    Function(FunctionDecl),
    Var(VarDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub modifiers: Modifiers,
    pub is_interface: bool,
    pub name: String,
    pub extends: Option<DottedName>,
    /// An interface's `extends A, B` list lands here too: both spell "the
    /// interfaces this type conforms to", which is one `interface_count` list
    /// in ABC.
    pub implements: Vec<DottedName>,
    pub members: Vec<Member>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub name: DottedName,
    /// `import flash.display.*` — the trailing component was a `*`.
    pub wildcard: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// Empty for `package { ... }`, the unnamed package.
    pub name: String,
    pub imports: Vec<ImportDecl>,
    pub classes: Vec<ClassDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompilationUnit {
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Empty,
    Expr(Expr),
    /// A `var`/`const` declaration inside a function body.
    Var(Box<VarDecl>),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then: Box<Stmt>,
        otherwise: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    Block(Block),
    Break(Option<String>),
    Continue(Option<String>),
    /// A statement the parser recognises but this phase declines to lower. It
    /// carries its span so codegen can point at it.
    Unsupported {
        what: &'static str,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    StrictEq,
    Ne,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    /// `as` and `is`, kept as binary operators the way the grammar reads.
    As,
    Is,
    InstanceOf,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    TypeOf,
    Delete,
    Void,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Null(Span),
    Bool(bool, Span),
    Int(i64, Span),
    Number(f64, Span),
    Str(String, Span),
    /// A bare identifier, unresolved. Deciding whether this is a local, a
    /// member, a class or a package is a name-resolution job, not a parse job.
    Ident(String, Span),
    This(Span),
    Super(Span),
    Member {
        object: Box<Expr>,
        name: String,
        span: Span,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    New {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Assign {
        target: Box<Expr>,
        /// `None` for plain `=`; `Some(op)` for a compound assignment.
        op: Option<BinOp>,
        value: Box<Expr>,
        span: Span,
    },
    Conditional {
        cond: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
        span: Span,
    },
    ArrayLit {
        items: Vec<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Null(s)
            | Expr::Bool(_, s)
            | Expr::Int(_, s)
            | Expr::Number(_, s)
            | Expr::Str(_, s)
            | Expr::Ident(_, s)
            | Expr::This(s)
            | Expr::Super(s) => *s,
            Expr::Member { span, .. }
            | Expr::Index { span, .. }
            | Expr::Call { span, .. }
            | Expr::New { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Assign { span, .. }
            | Expr::Conditional { span, .. }
            | Expr::ArrayLit { span, .. } => *span,
        }
    }
}
