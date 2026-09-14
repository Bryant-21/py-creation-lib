#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub name: Option<String>,
    pub variables: Vec<VarDecl>,
    pub blocks: Vec<Block>,
    pub script_kind: ScriptKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Object,
    Quest,
    Effect,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub name: String,
    pub ty: VarType,
    pub initial: Option<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    Int,
    Long,
    Short,
    Float,
    Ref,
    StringVar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub event: String,
    pub args: Vec<Expr>,
    pub statements: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Set {
        target: LValue,
        value: Expr,
    },
    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        elif_branches: Vec<(Expr, Vec<Stmt>)>,
        else_branch: Vec<Stmt>,
    },
    Call(FunctionCall),
    Return,
    ScriptBlockEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Float(f64),
    String(String),
    Ident(String),
    Member {
        receiver: Box<Expr>,
        name: String,
    },
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Call(FunctionCall),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub receiver: Option<Box<Expr>>,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LValue {
    Var(String),
    Member { receiver: Expr, name: String },
}
