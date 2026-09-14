"""Papyrus language library: parser, resolver, completions, definition.

Python facade over the Rust `papyrus_core` crate (`creation_lib._native.papyrus_core`)
that round-trips dataclasses ↔ JSON. Entry points: parse_script,
validate_filename, resolve, get_completions, get_definition, ScriptDB.
"""
from __future__ import annotations
from dataclasses import dataclass, field
from typing import Optional

from . import native_runtime as _native
from .ast_nodes import ScriptNode, ParseResult, ParseError
from .native_runtime import (
    Diagnostic as _ResolverDiagnostic,
    DiagnosticSeverity,
    ScriptDB,
    resolve,
)


def parse_script(path: Optional[str] = None, text: Optional[str] = None) -> ParseResult:
    """Parse a .psc file or text. Native (Rust) implementation only.

    Preprocessing (CRLF normalization, line-continuation collapse, doc-comment
    stripping) runs inside Rust under a released GIL.
    """
    if path is not None:
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as f:
                text = f.read()
        except OSError as e:
            return ParseResult(errors=[ParseError(1, 0, f"Read error: {e}")])
    if not text or not text.strip():
        return ParseResult(errors=[ParseError(1, 0, "Empty script")])
    payload = _native.parse_text_native(text)
    ast_dict = payload.get("ast")
    ast = _native.script_node_from_json(ast_dict) if ast_dict else None
    errors = [
        ParseError(line=int(e["line"]), col=int(e["col"]), message=str(e["message"]))
        for e in payload.get("errors", [])
    ]
    return ParseResult(ast=ast, errors=errors)


def validate_filename(path: Optional[str], script_name: Optional[str]) -> Optional[ParseError]:
    """Filename ↔ Scriptname validation via the native implementation."""
    if not path or not script_name:
        return None
    payload = _native.validate_filename_native(path, script_name)
    if payload is None:
        return None
    return ParseError(
        line=int(payload["line"]),
        col=int(payload["col"]),
        message=str(payload["message"]),
    )


# Convenience alias matching the original public surface.
parse = parse_script


# Completions & definition — pure Python; they only orchestrate the parser
# and the DB.
from .completions import get_completions, CompletionItem
from .definition import get_definition, DefinitionResult


# --- Shared types used across the toolkit ---

@dataclass
class Diagnostic:
    """Editor diagnostic (syntax error or symbol resolution warning)."""
    path: str
    line: int        # 0-based
    col: int         # 0-based
    end_line: int
    end_col: int
    message: str
    severity: str    # "error" | "warning"


@dataclass
class CompletionResult:
    """Wraps completion items with request context for routing."""
    path: str
    items: list[CompletionItem] = field(default_factory=list)
