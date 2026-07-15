"""Typed AST node definitions for Papyrus scripts.

Every node carries a Pos (position) for diagnostic reporting.
Nodes are plain dataclasses -- no methods, no tree walking here.
"""

from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional


@dataclass
class Pos:
    """Source position: line/col are 1-based for LSP compatibility."""
    line: int
    col: int
    end_line: int
    end_col: int


# --- Expressions ---

@dataclass
class NameExpr:
    """Simple identifier reference: akActor, Self, parent, etc."""
    name: str
    pos: Pos


@dataclass
class LiteralExpr:
    """Literal value: int, float, string, bool, None."""
    value: object
    type: str  # "int", "float", "string", "bool", "none"
    pos: Pos


@dataclass
class DotExpr:
    """Dot-access: object.member (e.g., Game.GetPlayer)."""
    object: object  # expression node
    member: str
    pos: Pos


@dataclass
class CallExpr:
    """Function call: func(args) or expr.func(args)."""
    function: str
    args: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class DotCallExpr:
    """Method call on an expression: expr.Method(args)."""
    object: object  # expression node
    method: str
    args: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class BinaryExpr:
    """Binary operation: left op right."""
    left: object
    op: str  # "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&", "||"
    right: object
    pos: Pos


@dataclass
class UnaryExpr:
    """Unary operation: -expr, !expr."""
    op: str  # "-", "!"
    operand: object
    pos: Pos


@dataclass
class CastExpr:
    """Type cast: expr as Type."""
    expr: object
    target_type: str
    pos: Pos


@dataclass
class ArrayAccessExpr:
    """Array subscript: expr[index]."""
    array: object
    index: object
    pos: Pos


@dataclass
class NewArrayExpr:
    """Array creation: new Type[size]."""
    element_type: str
    size: object
    pos: Pos


@dataclass
class ParentExpr:
    """Parent keyword access."""
    pos: Pos


# --- Statements ---

@dataclass
class ExprStmt:
    """Expression used as a statement (function call, etc.)."""
    expr: object
    pos: Pos


@dataclass
class AssignStmt:
    """Assignment: target = value, target += value, etc."""
    target: object
    op: str  # "=", "+=", "-=", "*=", "/="
    value: object
    pos: Pos


@dataclass
class ReturnStmt:
    """Return statement, optionally with a value."""
    value: Optional[object] = None
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class IfStmt:
    """If/ElseIf/Else/EndIf block."""
    condition: object
    body: list = field(default_factory=list)
    elseif_clauses: list = field(default_factory=list)  # list of (condition, body) tuples
    else_body: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class WhileStmt:
    """While/EndWhile loop."""
    condition: object
    body: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class LocalVarStmt:
    """Local variable declaration: Type varName = expr."""
    name: str
    type: str
    value: Optional[object] = None
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


# --- Definitions ---

@dataclass
class Parameter:
    """Function/event parameter."""
    name: str
    type: str
    default: Optional[object] = None
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class FunctionDef:
    """Function definition."""
    name: str
    return_type: str  # "None" if void
    params: list[Parameter] = field(default_factory=list)
    is_native: bool = False
    is_global: bool = False
    is_beta_only: bool = False
    docstring: str = ""
    body: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class EventDef:
    """Event definition."""
    name: str
    params: list[Parameter] = field(default_factory=list)
    is_native: bool = False
    docstring: str = ""
    body: list = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class PropertyDef:
    """Property definition."""
    name: str
    type: str
    flags: list[str] = field(default_factory=list)  # Auto, Const, Mandatory, Hidden, etc.
    docstring: str = ""
    default: Optional[object] = None
    getter: Optional[FunctionDef] = None
    setter: Optional[FunctionDef] = None
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class VariableDef:
    """Script-level variable (not local)."""
    name: str
    type: str
    value: Optional[object] = None
    flags: list[str] = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class StructMemberDef:
    """Member inside a Papyrus struct definition."""
    name: str
    type: str
    value: Optional[object] = None
    flags: list[str] = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class StructDef:
    """Papyrus struct definition."""
    name: str
    members: list[StructMemberDef] = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class ImportNode:
    """Import statement."""
    script_name: str
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class StateDef:
    """State definition."""
    name: str
    is_auto: bool = False
    functions: list[FunctionDef] = field(default_factory=list)
    events: list[EventDef] = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


@dataclass
class ScriptNode:
    """Top-level script node -- root of the AST."""
    name: str
    parent: Optional[str] = None
    flags: list[str] = field(default_factory=list)  # Native, Hidden, Conditional, Const, etc.
    imports: list[ImportNode] = field(default_factory=list)
    properties: list[PropertyDef] = field(default_factory=list)
    variables: list[VariableDef] = field(default_factory=list)
    structs: list[StructDef] = field(default_factory=list)
    functions: list[FunctionDef] = field(default_factory=list)
    events: list[EventDef] = field(default_factory=list)
    states: list[StateDef] = field(default_factory=list)
    pos: Pos = field(default_factory=lambda: Pos(0, 0, 0, 0))


# --- Parse result types ---

@dataclass
class ParseError:
    """A syntax error with location info."""
    line: int
    col: int
    message: str


@dataclass
class ParseResult:
    """Result of parsing a .psc file."""
    ast: Optional[ScriptNode] = None
    errors: list[ParseError] = field(default_factory=list)
