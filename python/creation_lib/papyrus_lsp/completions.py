"""Completion item generation for the Papyrus editor."""
from __future__ import annotations
from dataclasses import dataclass


@dataclass
class CompletionItem:
    label: str
    kind: str    # "function" | "property" | "type" | "keyword"
    detail: str


def _find_dot_context(text: str, line: int, col: int) -> tuple[str, str] | None:
    """If cursor is after 'receiver.member', return (receiver_word, member_word).
    If right after dot, member is empty string.
    Returns None if cursor is not in a dot expression.
    """
    lines = text.split("\n")
    if line >= len(lines):
        return None
    src_line = lines[line]
    pos = min(col, len(src_line))
    # Walk back over partial member word
    word_start = pos
    while word_start > 0 and (src_line[word_start - 1].isalnum() or src_line[word_start - 1] == '_'):
        word_start -= 1
    member = src_line[word_start:pos]
    # Check for dot before the member
    dot_pos = word_start - 1
    while dot_pos >= 0 and src_line[dot_pos] == ' ':
        dot_pos -= 1
    if dot_pos < 0 or src_line[dot_pos] != '.':
        return None
    # Get receiver word before dot
    recv_end = dot_pos
    recv_start = recv_end
    while recv_start > 0 and (src_line[recv_start - 1].isalnum() or src_line[recv_start - 1] == '_'):
        recv_start -= 1
    receiver = src_line[recv_start:recv_end]
    if not receiver:
        return None
    return receiver, member


def _infer_type_from_script(ast, receiver_word: str, db) -> str | None:
    """Infer receiver type from AST scope or treat as script name."""
    if ast is None:
        return None
    word_lower = receiver_word.lower()
    if word_lower == "self":
        return ast.name
    if word_lower == "parent":
        return ast.parent
    for prop in ast.properties:
        if prop.name.lower() == word_lower:
            return prop.type
    for var in ast.variables:
        if var.name.lower() == word_lower:
            return var.type
    if db.script_exists(receiver_word):
        return receiver_word
    return None


def get_completions(text: str, line: int, col: int, db) -> list[CompletionItem]:
    """Return completion items for cursor at (line, col) in text.

    If cursor is after '.', resolves left-hand type and returns its
    functions/properties from db. Otherwise returns all known script
    type names matching the current prefix. On parse failure or no match,
    falls back to all known script type names (never returns []).

    Args:
        text: Full script text.
        line: 0-based line index.
        col: 0-based column index (cursor position).
        db: ScriptDB instance.

    Returns:
        List of CompletionItem — never empty if db has any scripts.
    """
    from . import parse_script

    items: list[CompletionItem] = []

    # Try dot completion first
    dot_ctx = _find_dot_context(text, line, col)
    if dot_ctx:
        receiver_word, _member = dot_ctx
        # Try to parse; if full text fails (incomplete dot expr), strip the current line
        parse_result = parse_script(text=text)
        ast = parse_result.ast
        if ast is None:
            text_lines = text.split("\n")
            clean_text = "\n".join(text_lines[:line] + text_lines[line + 1:])
            parse_result = parse_script(text=clean_text)
            ast = parse_result.ast
        receiver_type = _infer_type_from_script(ast, receiver_word, db)
        if receiver_type:
            members = db.get_all_members(receiver_type)
            for f in members["functions"]:
                params = f.get("params", "")
                items.append(CompletionItem(
                    label=f["name"],
                    kind="function",
                    detail=f"{f['return_type']} {f['name']}({params})",
                ))
            for p in members["properties"]:
                items.append(CompletionItem(
                    label=p["name"],
                    kind="property",
                    detail=f"{p['type']} {p['name']}",
                ))
            for e in members.get("events", []):
                items.append(CompletionItem(
                    label=e["name"],
                    kind="function",
                    detail=f"Event {e['name']}({e.get('params', '')})",
                ))
            if items:
                return items
        # Fall through to script name fallback if type not resolved

    # Determine prefix from current word
    lines = text.split("\n")
    prefix = ""
    if line < len(lines):
        src_line = lines[line]
        pos = min(col, len(src_line))
        word_start = pos
        while word_start > 0 and (src_line[word_start - 1].isalnum() or src_line[word_start - 1] == '_'):
            word_start -= 1
        prefix = src_line[word_start:pos].lower()

    # Add local scope from AST (best-effort)
    parse_result = parse_script(text=text)
    if parse_result.ast is not None:
        ast = parse_result.ast
        for prop in ast.properties:
            if prop.name.lower().startswith(prefix):
                items.append(CompletionItem(label=prop.name, kind="property",
                                            detail=f"{prop.type} Property"))
        for var in ast.variables:
            if var.name.lower().startswith(prefix):
                items.append(CompletionItem(label=var.name, kind="type",
                                            detail=var.type))
        for func in ast.functions:
            if func.name.lower().startswith(prefix):
                param_str = ", ".join(f"{p.type} {p.name}" for p in func.params)
                detail = f"({param_str})"
                if func.return_type != "None":
                    detail += f" -> {func.return_type}"
                items.append(CompletionItem(label=func.name, kind="function", detail=detail))

    # Script name search from DB
    if len(prefix) >= 2:
        for script_name in db.search_scripts(prefix):
            items.append(CompletionItem(label=script_name, kind="type", detail="Script"))
    elif not prefix:
        # No prefix — offer common globals
        for global_script in ["Game", "Debug", "Utility", "Math", "Actor", "ObjectReference", "Form"]:
            items.append(CompletionItem(label=global_script, kind="type", detail="Script"))

    # Fallback: ensure we never return []
    if not items:
        for script_name in db.search_scripts(""):
            items.append(CompletionItem(label=script_name, kind="type", detail="Script"))
            if len(items) >= 20:
                break

    return items
