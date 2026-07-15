//! Papyrus AST node types — mirror of `py_creation_lib/python/creation_lib/papyrus_lsp/ast_nodes.py`.
//!
//! All nodes carry a `Pos` (1-based line/col + end_line/end_col, matching the
//! Python definition) and are `serde::Serialize` so `bindings.rs` can hand
//! them to Python as JSON without holding the GIL.
//!
//! The shapes intentionally match the Python dataclasses field-for-field so
//! the Python facade in `py_creation_lib/python/creation_lib/papyrus_lsp/__init__.py` can reconstruct the
//! original dataclass instances without translation logic. Field names use
//! `serde(rename = "type")` where the Python name shadows a Rust keyword.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Pos {
    pub const ZERO: Pos = Pos {
        line: 0,
        col: 0,
        end_line: 0,
        end_col: 0,
    };

    pub fn span(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            line: start_line,
            col: start_col,
            end_line,
            end_col,
        }
    }
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// `tag` is set so Python (and downstream tools) can discriminate variants
/// from the JSON without inspecting field shapes. The names match the Python
/// dataclass class names exactly (`NameExpr`, `LiteralExpr`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "node")]
pub enum Expr {
    NameExpr {
        name: String,
        pos: Pos,
    },
    LiteralExpr {
        value: LiteralValue,
        #[serde(rename = "type")]
        ty: String, // "int" | "float" | "string" | "bool" | "none"
        pos: Pos,
    },
    DotExpr {
        object: Box<Expr>,
        member: String,
        pos: Pos,
    },
    CallExpr {
        function: String,
        args: Vec<Expr>,
        /// Parallel to `args`: the explicit `name =` for each argument, or `None`
        /// for a positional argument. Lets codegen bind out-of-order / gap-skipping
        /// named arguments to their parameters.
        #[serde(default)]
        arg_names: Vec<Option<String>>,
        pos: Pos,
    },
    DotCallExpr {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        pos: Pos,
    },
    BinaryExpr {
        left: Box<Expr>,
        op: String,
        right: Box<Expr>,
        pos: Pos,
    },
    UnaryExpr {
        op: String,
        operand: Box<Expr>,
        pos: Pos,
    },
    CastExpr {
        expr: Box<Expr>,
        target_type: String,
        pos: Pos,
    },
    ArrayAccessExpr {
        array: Box<Expr>,
        index: Box<Expr>,
        pos: Pos,
    },
    NewArrayExpr {
        element_type: String,
        size: Box<Expr>,
        pos: Pos,
    },
    ParentExpr {
        pos: Pos,
    },
}

impl Expr {
    pub fn pos(&self) -> Pos {
        match self {
            Expr::NameExpr { pos, .. }
            | Expr::LiteralExpr { pos, .. }
            | Expr::DotExpr { pos, .. }
            | Expr::CallExpr { pos, .. }
            | Expr::DotCallExpr { pos, .. }
            | Expr::BinaryExpr { pos, .. }
            | Expr::UnaryExpr { pos, .. }
            | Expr::CastExpr { pos, .. }
            | Expr::ArrayAccessExpr { pos, .. }
            | Expr::NewArrayExpr { pos, .. }
            | Expr::ParentExpr { pos } => *pos,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LiteralValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "node")]
pub enum Stmt {
    ExprStmt {
        expr: Expr,
        pos: Pos,
    },
    AssignStmt {
        target: Expr,
        op: String,
        value: Expr,
        pos: Pos,
    },
    ReturnStmt {
        value: Option<Expr>,
        pos: Pos,
    },
    IfStmt {
        condition: Expr,
        body: Vec<Stmt>,
        elseif_clauses: Vec<ElseIfClause>,
        else_body: Vec<Stmt>,
        pos: Pos,
    },
    WhileStmt {
        condition: Expr,
        body: Vec<Stmt>,
        pos: Pos,
    },
    LocalVarStmt {
        name: String,
        #[serde(rename = "type")]
        ty: String,
        value: Option<Expr>,
        pos: Pos,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElseIfClause {
    pub condition: Expr,
    pub body: Vec<Stmt>,
    pub pos: Pos,
}

// ---------------------------------------------------------------------------
// Definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub default: Option<Expr>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub return_type: String,
    pub params: Vec<Parameter>,
    pub is_native: bool,
    pub is_global: bool,
    pub is_beta_only: bool,
    #[serde(default)]
    pub docstring: String,
    pub body: Vec<Stmt>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventDef {
    pub name: String,
    pub params: Vec<Parameter>,
    pub is_native: bool,
    #[serde(default)]
    pub docstring: String,
    pub body: Vec<Stmt>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PropertyDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub flags: Vec<String>,
    #[serde(default)]
    pub docstring: String,
    #[serde(default)]
    pub default: Option<Expr>,
    #[serde(default)]
    pub getter: Option<FunctionDef>,
    #[serde(default)]
    pub setter: Option<FunctionDef>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VariableDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub value: Option<Expr>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StructMemberDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub value: Option<Expr>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StructDef {
    pub name: String,
    #[serde(default)]
    pub members: Vec<StructMemberDef>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportNode {
    pub script_name: String,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateDef {
    pub name: String,
    pub is_auto: bool,
    pub functions: Vec<FunctionDef>,
    pub events: Vec<EventDef>,
    #[serde(default)]
    pub pos: Pos,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptNode {
    pub name: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub flags: Vec<String>,
    pub imports: Vec<ImportNode>,
    pub properties: Vec<PropertyDef>,
    pub variables: Vec<VariableDef>,
    #[serde(default)]
    pub structs: Vec<StructDef>,
    pub functions: Vec<FunctionDef>,
    pub events: Vec<EventDef>,
    pub states: Vec<StateDef>,
    #[serde(default)]
    pub pos: Pos,
}
