"""Thin Python boundary for native Papyrus core entrypoints.

Mirrors the loader pattern in `py_creation_lib/python/creation_lib/esp/native_runtime.py`. The native module is
optional — callers should gate use behind `is_available()` or the environment
flag `MODKIT_PAPYRUS_NATIVE=1`.

GIL discipline: every native function we call here releases the GIL inside Rust
(see `py_creation_lib/native/papyrus_core/src/bindings.rs`). The Python boundary only does
JSON unmarshalling, which is cheap.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from enum import IntEnum
from importlib import import_module
from typing import Any, Optional

from .ast_nodes import (
    ArrayAccessExpr, AssignStmt, BinaryExpr, CallExpr, CastExpr,
    DotCallExpr, DotExpr, EventDef, ExprStmt, FunctionDef, IfStmt,
    ImportNode, LiteralExpr, LocalVarStmt, NameExpr, NewArrayExpr,
    Parameter, ParentExpr, Pos, PropertyDef, ReturnStmt, ScriptNode,
    StateDef, StructDef, StructMemberDef, UnaryExpr, VariableDef, WhileStmt,
)

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_ENABLED: bool | None = None


def _load_umbrella_submodule() -> Any:
    umbrella = import_module("creation_lib._native")
    native = getattr(umbrella, "papyrus_core", None)
    if native is None:
        raise ImportError("creation_lib._native.papyrus_core is missing")
    return native


def load_native_module() -> Any | None:
    """Load the native Papyrus core module. Returns None if unavailable."""
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        return _NATIVE_MODULE
    _NATIVE_IMPORT_ATTEMPTED = True
    try:
        _NATIVE_MODULE = _load_umbrella_submodule()
    except ImportError:
        try:
            _NATIVE_MODULE = import_module("papyrus_core")
        except ImportError:
            _NATIVE_MODULE = None
    return _NATIVE_MODULE


def is_available() -> bool:
    return load_native_module() is not None


def configure_native_enabled(enabled: bool | None) -> None:
    global _NATIVE_ENABLED
    _NATIVE_ENABLED = enabled


def is_enabled() -> bool:
    """True when the native backend should be used.

    Native is the default. Set `MODKIT_PAPYRUS_NATIVE=0` to force the
    Python fallback (e.g. for debugging a parity regression).
    """
    if not is_available():
        return False
    if _NATIVE_ENABLED is False:
        return False
    return True


# ---------------------------------------------------------------------------
# Stateless calls
# ---------------------------------------------------------------------------

def parse_text_native(text: str) -> dict[str, Any]:
    """Call native parse_text and parse the JSON envelope."""
    mod = load_native_module()
    if mod is None:
        raise RuntimeError("papyrus_core native module is not loaded")
    raw = mod.parse_text(text)
    return json.loads(raw)


def emit_script_native(ast: ScriptNode) -> str:
    """Render an AST back to Papyrus source via the native emitter."""
    mod = load_native_module()
    if mod is None:
        raise RuntimeError("papyrus_core native module is not loaded")
    return mod.emit_script_json(json.dumps(_dump_script(ast)))


def validate_filename_native(path: str | None, script_name: str | None) -> dict[str, Any] | None:
    if not path or not script_name:
        return None
    mod = load_native_module()
    if mod is None:
        raise RuntimeError("papyrus_core native module is not loaded")
    raw = mod.validate_filename(path, script_name)
    if raw == "null":
        return None
    return json.loads(raw)


# ---------------------------------------------------------------------------
# AST JSON ↔ dataclass round-trip
# ---------------------------------------------------------------------------

def _pos_from(d: dict | None) -> Pos:
    if not d:
        return Pos(0, 0, 0, 0)
    return Pos(
        line=int(d.get("line", 0) or 0),
        col=int(d.get("col", 0) or 0),
        end_line=int(d.get("end_line", 0) or 0),
        end_col=int(d.get("end_col", 0) or 0),
    )


def _expr_from(d: Any) -> Any:
    if d is None:
        return None
    node = d.get("node")
    pos = _pos_from(d.get("pos"))
    if node == "NameExpr":
        return NameExpr(name=d["name"], pos=pos)
    if node == "LiteralExpr":
        return LiteralExpr(value=d.get("value"), type=d.get("type", "none"), pos=pos)
    if node == "DotExpr":
        return DotExpr(object=_expr_from(d["object"]), member=d["member"], pos=pos)
    if node == "CallExpr":
        return CallExpr(
            function=d["function"],
            args=[_expr_from(a) for a in d.get("args", [])],
            pos=pos,
        )
    if node == "DotCallExpr":
        return DotCallExpr(
            object=_expr_from(d["object"]),
            method=d["method"],
            args=[_expr_from(a) for a in d.get("args", [])],
            pos=pos,
        )
    if node == "BinaryExpr":
        return BinaryExpr(
            left=_expr_from(d["left"]),
            op=d["op"],
            right=_expr_from(d["right"]),
            pos=pos,
        )
    if node == "UnaryExpr":
        return UnaryExpr(op=d["op"], operand=_expr_from(d["operand"]), pos=pos)
    if node == "CastExpr":
        return CastExpr(expr=_expr_from(d["expr"]), target_type=d["target_type"], pos=pos)
    if node == "ArrayAccessExpr":
        return ArrayAccessExpr(
            array=_expr_from(d["array"]),
            index=_expr_from(d["index"]),
            pos=pos,
        )
    if node == "NewArrayExpr":
        return NewArrayExpr(
            element_type=d["element_type"],
            size=_expr_from(d["size"]),
            pos=pos,
        )
    if node == "ParentExpr":
        return ParentExpr(pos=pos)
    raise ValueError(f"unknown expr node: {node!r}")


def _stmt_from(d: dict) -> Any:
    node = d.get("node")
    pos = _pos_from(d.get("pos"))
    if node == "ExprStmt":
        return ExprStmt(expr=_expr_from(d["expr"]), pos=pos)
    if node == "AssignStmt":
        return AssignStmt(
            target=_expr_from(d["target"]),
            op=d["op"],
            value=_expr_from(d["value"]),
            pos=pos,
        )
    if node == "ReturnStmt":
        return ReturnStmt(value=_expr_from(d.get("value")), pos=pos)
    if node == "IfStmt":
        return IfStmt(
            condition=_expr_from(d["condition"]),
            body=[_stmt_from(s) for s in d.get("body", [])],
            elseif_clauses=[
                (_expr_from(c["condition"]), [_stmt_from(s) for s in c.get("body", [])])
                for c in d.get("elseif_clauses", [])
            ],
            else_body=[_stmt_from(s) for s in d.get("else_body", [])],
            pos=pos,
        )
    if node == "WhileStmt":
        return WhileStmt(
            condition=_expr_from(d["condition"]),
            body=[_stmt_from(s) for s in d.get("body", [])],
            pos=pos,
        )
    if node == "LocalVarStmt":
        return LocalVarStmt(
            name=d["name"],
            type=d["type"],
            value=_expr_from(d.get("value")),
            pos=pos,
        )
    raise ValueError(f"unknown stmt node: {node!r}")


def _param_from(d: dict) -> Parameter:
    return Parameter(
        name=d["name"],
        type=d["type"],
        default=_expr_from(d.get("default")),
        pos=_pos_from(d.get("pos")),
    )


def _function_from(d: dict) -> FunctionDef:
    return FunctionDef(
        name=d["name"],
        return_type=d.get("return_type", "None"),
        params=[_param_from(p) for p in d.get("params", [])],
        is_native=d.get("is_native", False),
        is_global=d.get("is_global", False),
        is_beta_only=d.get("is_beta_only", False),
        docstring=d.get("docstring", ""),
        body=[_stmt_from(s) for s in d.get("body", [])],
        pos=_pos_from(d.get("pos")),
    )


def _event_from(d: dict) -> EventDef:
    return EventDef(
        name=d["name"],
        params=[_param_from(p) for p in d.get("params", [])],
        is_native=d.get("is_native", False),
        docstring=d.get("docstring", ""),
        body=[_stmt_from(s) for s in d.get("body", [])],
        pos=_pos_from(d.get("pos")),
    )


def script_node_from_json(d: dict) -> ScriptNode:
    """Reconstruct a Python ScriptNode from the JSON returned by parse_text."""
    return ScriptNode(
        name=d["name"],
        parent=d.get("parent"),
        flags=list(d.get("flags", [])),
        imports=[
            ImportNode(script_name=i["script_name"], pos=_pos_from(i.get("pos")))
            for i in d.get("imports", [])
        ],
        properties=[
            PropertyDef(
                name=p["name"],
                type=p["type"],
                flags=list(p.get("flags", [])),
                docstring=p.get("docstring", ""),
                default=_expr_from(p.get("default")),
                getter=_function_from(p["getter"]) if p.get("getter") else None,
                setter=_function_from(p["setter"]) if p.get("setter") else None,
                pos=_pos_from(p.get("pos")),
            )
            for p in d.get("properties", [])
        ],
        variables=[
            VariableDef(
                name=v["name"],
                type=v["type"],
                value=_expr_from(v.get("value")),
                flags=list(v.get("flags", [])),
                pos=_pos_from(v.get("pos")),
            )
            for v in d.get("variables", [])
        ],
        structs=[
            StructDef(
                name=s["name"],
                members=[
                    StructMemberDef(
                        name=m["name"],
                        type=m["type"],
                        value=_expr_from(m.get("value")),
                        flags=list(m.get("flags", [])),
                        pos=_pos_from(m.get("pos")),
                    )
                    for m in s.get("members", [])
                ],
                pos=_pos_from(s.get("pos")),
            )
            for s in d.get("structs", [])
        ],
        functions=[_function_from(f) for f in d.get("functions", [])],
        events=[_event_from(e) for e in d.get("events", [])],
        states=[
            StateDef(
                name=s["name"],
                is_auto=s.get("is_auto", False),
                functions=[_function_from(f) for f in s.get("functions", [])],
                events=[_event_from(e) for e in s.get("events", [])],
                pos=_pos_from(s.get("pos")),
            )
            for s in d.get("states", [])
        ],
        pos=_pos_from(d.get("pos")),
    )


# ---------------------------------------------------------------------------
# AST → JSON (for the round-trip emitter)
# ---------------------------------------------------------------------------

def _dump_pos(p: Pos | None) -> dict:
    if p is None:
        return {"line": 0, "col": 0, "end_line": 0, "end_col": 0}
    return {"line": p.line, "col": p.col, "end_line": p.end_line, "end_col": p.end_col}


def _dump_expr(e: Any) -> Any:
    if e is None:
        return None
    if isinstance(e, NameExpr):
        return {"node": "NameExpr", "name": e.name, "pos": _dump_pos(e.pos)}
    if isinstance(e, LiteralExpr):
        return {
            "node": "LiteralExpr",
            "value": e.value,
            "type": e.type,
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, DotExpr):
        return {
            "node": "DotExpr",
            "object": _dump_expr(e.object),
            "member": e.member,
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, CallExpr):
        return {
            "node": "CallExpr",
            "function": e.function,
            "args": [_dump_expr(a) for a in e.args],
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, DotCallExpr):
        return {
            "node": "DotCallExpr",
            "object": _dump_expr(e.object),
            "method": e.method,
            "args": [_dump_expr(a) for a in e.args],
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, BinaryExpr):
        return {
            "node": "BinaryExpr",
            "left": _dump_expr(e.left),
            "op": e.op,
            "right": _dump_expr(e.right),
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, UnaryExpr):
        return {
            "node": "UnaryExpr",
            "op": e.op,
            "operand": _dump_expr(e.operand),
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, CastExpr):
        return {
            "node": "CastExpr",
            "expr": _dump_expr(e.expr),
            "target_type": e.target_type,
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, ArrayAccessExpr):
        return {
            "node": "ArrayAccessExpr",
            "array": _dump_expr(e.array),
            "index": _dump_expr(e.index),
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, NewArrayExpr):
        return {
            "node": "NewArrayExpr",
            "element_type": e.element_type,
            "size": _dump_expr(e.size),
            "pos": _dump_pos(e.pos),
        }
    if isinstance(e, ParentExpr):
        return {"node": "ParentExpr", "pos": _dump_pos(e.pos)}
    raise ValueError(f"unknown expr type: {type(e).__name__}")


def _dump_stmt(s: Any) -> dict:
    if isinstance(s, ExprStmt):
        return {"node": "ExprStmt", "expr": _dump_expr(s.expr), "pos": _dump_pos(s.pos)}
    if isinstance(s, AssignStmt):
        return {
            "node": "AssignStmt",
            "target": _dump_expr(s.target),
            "op": s.op,
            "value": _dump_expr(s.value),
            "pos": _dump_pos(s.pos),
        }
    if isinstance(s, ReturnStmt):
        return {
            "node": "ReturnStmt",
            "value": _dump_expr(s.value),
            "pos": _dump_pos(s.pos),
        }
    if isinstance(s, IfStmt):
        return {
            "node": "IfStmt",
            "condition": _dump_expr(s.condition),
            "body": [_dump_stmt(x) for x in s.body],
            "elseif_clauses": [
                {
                    "condition": _dump_expr(c[0]),
                    "body": [_dump_stmt(x) for x in c[1]],
                    "pos": _dump_pos(Pos(0, 0, 0, 0)),
                }
                for c in s.elseif_clauses
            ],
            "else_body": [_dump_stmt(x) for x in s.else_body],
            "pos": _dump_pos(s.pos),
        }
    if isinstance(s, WhileStmt):
        return {
            "node": "WhileStmt",
            "condition": _dump_expr(s.condition),
            "body": [_dump_stmt(x) for x in s.body],
            "pos": _dump_pos(s.pos),
        }
    if isinstance(s, LocalVarStmt):
        return {
            "node": "LocalVarStmt",
            "name": s.name,
            "type": s.type,
            "value": _dump_expr(s.value),
            "pos": _dump_pos(s.pos),
        }
    raise ValueError(f"unknown stmt type: {type(s).__name__}")


def _dump_param(p: Parameter) -> dict:
    return {
        "name": p.name,
        "type": p.type,
        "default": _dump_expr(p.default),
        "pos": _dump_pos(p.pos),
    }


def _dump_function(f: FunctionDef) -> dict:
    return {
        "name": f.name,
        "return_type": f.return_type,
        "params": [_dump_param(p) for p in f.params],
        "is_native": f.is_native,
        "is_global": f.is_global,
        "is_beta_only": f.is_beta_only,
        "docstring": f.docstring,
        "body": [_dump_stmt(s) for s in f.body],
        "pos": _dump_pos(f.pos),
    }


def _dump_struct(s: StructDef) -> dict:
    return {
        "name": s.name,
        "members": [
            {
                "name": m.name,
                "type": m.type,
                "value": _dump_expr(m.value),
                "flags": list(m.flags),
                "pos": _dump_pos(m.pos),
            }
            for m in s.members
        ],
        "pos": _dump_pos(s.pos),
    }


def _dump_event(e: EventDef) -> dict:
    return {
        "name": e.name,
        "params": [_dump_param(p) for p in e.params],
        "is_native": e.is_native,
        "docstring": e.docstring,
        "body": [_dump_stmt(s) for s in e.body],
        "pos": _dump_pos(e.pos),
    }


def _dump_script(node: ScriptNode) -> dict:
    return {
        "name": node.name,
        "parent": node.parent,
        "flags": list(node.flags),
        "imports": [
            {"script_name": i.script_name, "pos": _dump_pos(i.pos)} for i in node.imports
        ],
        "properties": [
            {
                "name": p.name,
                "type": p.type,
                "flags": list(p.flags),
                "docstring": p.docstring,
                "default": _dump_expr(p.default),
                "getter": _dump_function(p.getter) if p.getter else None,
                "setter": _dump_function(p.setter) if p.setter else None,
                "pos": _dump_pos(p.pos),
            }
            for p in node.properties
        ],
        "variables": [
            {
                "name": v.name,
                "type": v.type,
                "value": _dump_expr(v.value),
                "flags": list(v.flags),
                "pos": _dump_pos(v.pos),
            }
            for v in node.variables
        ],
        "structs": [_dump_struct(s) for s in node.structs],
        "functions": [_dump_function(f) for f in node.functions],
        "events": [_dump_event(e) for e in node.events],
        "states": [
            {
                "name": s.name,
                "is_auto": s.is_auto,
                "functions": [_dump_function(f) for f in s.functions],
                "events": [_dump_event(e) for e in s.events],
                "pos": _dump_pos(s.pos),
            }
            for s in node.states
        ],
        "pos": _dump_pos(node.pos),
    }


# ---------------------------------------------------------------------------
# Diagnostic & resolver wrapper
# ---------------------------------------------------------------------------

class DiagnosticSeverity(IntEnum):
    ERROR = 1
    WARNING = 2
    INFO = 3
    HINT = 4


@dataclass
class Diagnostic:
    """Resolver diagnostic — shape-compatible with the old Python resolver."""
    line: int
    col: int
    end_line: int
    end_col: int
    message: str
    severity: DiagnosticSeverity = DiagnosticSeverity.ERROR


_SEVERITY_FROM_RUST = {
    1: DiagnosticSeverity.ERROR,
    2: DiagnosticSeverity.WARNING,
    3: DiagnosticSeverity.INFO,
    4: DiagnosticSeverity.HINT,
    "Error": DiagnosticSeverity.ERROR,
    "Warning": DiagnosticSeverity.WARNING,
    "Info": DiagnosticSeverity.INFO,
    "Hint": DiagnosticSeverity.HINT,
}


def _diagnostic_from_json(d: dict) -> Diagnostic:
    sev_raw = d.get("severity", 1)
    severity = _SEVERITY_FROM_RUST.get(sev_raw, DiagnosticSeverity.ERROR)
    return Diagnostic(
        line=int(d.get("line", 0) or 0),
        col=int(d.get("col", 0) or 0),
        end_line=int(d.get("end_line", 0) or 0),
        end_col=int(d.get("end_col", 0) or 0),
        message=str(d.get("message", "")),
        severity=severity,
    )


def resolve(ast: ScriptNode, db: "ScriptDB | None") -> list[Diagnostic]:
    """Run the native resolver against a ScriptNode and a ScriptDB.

    Mirrors `creation_lib.papyrus_lsp.resolver.resolve` but delegates to the Rust
    resolver via `resolve_ast`. `db` may be None (uses an empty DB).
    """
    mod = load_native_module()
    if mod is None:
        raise RuntimeError("papyrus_core native module is not loaded")
    ast_json = json.dumps(_dump_script(ast))
    db_id = db._db_id if db is not None else None
    raw = mod.resolve_ast(ast_json, db_id)
    payload = json.loads(raw)
    return [_diagnostic_from_json(d) for d in payload]


# ---------------------------------------------------------------------------
# ScriptDB — Python wrapper over the native db_* bindings
# ---------------------------------------------------------------------------

class ScriptDB:
    """Native-backed ScriptDB.

    Mirrors the call surface of the old `creation_lib.papyrus_lsp.script_db.ScriptDB`
    so completions/definition/resolver and the UI service work unchanged.
    The handle (`_db_id`) is allocated by the Rust session pool; close()
    releases it.
    """

    def __init__(self, db_path: str, source_dirs: Optional[list[str]] = None):
        mod = load_native_module()
        if mod is None:
            raise RuntimeError("papyrus_core native module is not loaded")
        self._mod = mod
        self._db_id: Optional[int] = mod.db_open(db_path, list(source_dirs or []))

    def close(self) -> None:
        if self._db_id is not None and self._mod is not None:
            self._mod.db_close(self._db_id)
            self._db_id = None

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass

    def _require(self) -> int:
        if self._db_id is None:
            raise RuntimeError("ScriptDB handle is closed")
        return self._db_id

    def add_source_dir(self, path: str) -> None:
        self._mod.db_add_source_dir(self._require(), path)

    def register_ast(self, ast: ScriptNode) -> None:
        ast_json = json.dumps(_dump_script(ast))
        self._mod.db_register_ast_json(self._require(), ast_json)

    def script_exists(self, name: str) -> bool:
        return bool(self._mod.db_script_exists(self._require(), name))

    def get_extends(self, name: str) -> Optional[str]:
        return self._mod.db_get_extends(self._require(), name)

    def get_functions(self, name: str) -> list[dict]:
        return json.loads(self._mod.db_get_functions(self._require(), name))

    def get_events(self, name: str) -> list[dict]:
        return json.loads(self._mod.db_get_events(self._require(), name))

    def get_properties(self, name: str) -> list[dict]:
        return json.loads(self._mod.db_get_properties(self._require(), name))

    def get_hierarchy(self, name: str) -> list[str]:
        return json.loads(self._mod.db_get_hierarchy(self._require(), name))

    def has_function(self, script_name: str, func_name: str) -> bool:
        return bool(self._mod.db_has_function(self._require(), script_name, func_name))

    def has_event(self, script_name: str, event_name: str) -> bool:
        return bool(self._mod.db_has_event(self._require(), script_name, event_name))

    def has_property(self, script_name: str, prop_name: str) -> bool:
        return bool(self._mod.db_has_property(self._require(), script_name, prop_name))

    def get_function_return_type(self, script_name: str, func_name: str) -> Optional[str]:
        return self._mod.db_get_function_return_type(self._require(), script_name, func_name)

    def get_script_path(self, name: str) -> Optional[str]:
        return self._mod.db_get_script_path(self._require(), name)

    def get_source(self, name: str) -> Optional[str]:
        return self._mod.db_get_source(self._require(), name)

    def search_scripts(self, prefix: str) -> list[str]:
        return json.loads(self._mod.db_search_scripts(self._require(), prefix))

    def get_all_members(self, name: str) -> dict:
        return json.loads(self._mod.db_get_all_members(self._require(), name))
