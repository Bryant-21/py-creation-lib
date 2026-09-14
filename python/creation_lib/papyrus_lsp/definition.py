"""Go-to-definition resolution for the Papyrus editor."""
from __future__ import annotations
from dataclasses import dataclass
import os


@dataclass
class DefinitionResult:
    path: str   # absolute path to .psc file
    line: int   # 0-based line number


def _word_at_position(text: str, line: int, col: int) -> str | None:
    """Extract the identifier word at (line, col). Returns None if not on a word."""
    lines = text.split("\n")
    if line >= len(lines):
        return None
    src_line = lines[line]
    if col > len(src_line):
        return None
    start = col
    end = col
    while start > 0 and (src_line[start - 1].isalnum() or src_line[start - 1] == '_'):
        start -= 1
    while end < len(src_line) and (src_line[end].isalnum() or src_line[end] == '_'):
        end += 1
    word = src_line[start:end]
    return word if word else None


def _find_definition_in_source(source: str, symbol: str) -> int:
    """Find the 0-based line where `symbol` is defined in source text.

    Searches for function/event/property definitions. Returns 0 if not found.
    """
    import re
    patterns = [
        re.compile(r'\bfunction\s+' + re.escape(symbol) + r'\b', re.IGNORECASE),
        re.compile(r'\bevent\s+' + re.escape(symbol) + r'\b', re.IGNORECASE),
        re.compile(r'\bproperty\s+' + re.escape(symbol) + r'\b', re.IGNORECASE),
    ]
    for i, line in enumerate(source.split("\n")):
        for pat in patterns:
            if pat.search(line):
                return i
    return 0


def get_definition(text: str, line: int, col: int, db) -> DefinitionResult | None:
    """Resolve the symbol under the cursor (0-based line, col) to its definition.

    Checks local definitions in the parsed AST, then known script names in db,
    then ``receiver.member`` in the receiver's script. Returns
    DefinitionResult(path, line), or None when the cursor isn't on a word or
    nothing resolves.
    """
    from . import parse_script

    word = _word_at_position(text, line, col)
    if not word:
        return None

    word_lower = word.lower()

    # Parse AST for local definitions
    parse_result = parse_script(text=text)
    ast = parse_result.ast

    # 1. Local AST definitions
    if ast is not None:
        local_src_line = _search_local_definition(ast, text, word_lower)
        if local_src_line is not None:
            # Definition is in current (unsaved) buffer — no path to return,
            # so return the path as empty string with the line number.
            # The caller (LspService/PapyrusEditorApp) handles the open-file case.
            return DefinitionResult(path="", line=local_src_line)

    # 2. Script name -> jump to its .psc file
    script_path = db.get_script_path(word)
    if script_path and os.path.exists(script_path):
        return DefinitionResult(path=script_path, line=0)

    # 3. Dot context: receiver.member
    lines = text.split("\n")
    if line < len(lines):
        src_line = lines[line]
        pos = min(col, len(src_line))
        # Walk back over word
        word_start = pos
        while word_start > 0 and (src_line[word_start - 1].isalnum() or src_line[word_start - 1] == '_'):
            word_start -= 1
        dot_pos = word_start - 1
        while dot_pos >= 0 and src_line[dot_pos] == ' ':
            dot_pos -= 1
        if dot_pos >= 0 and src_line[dot_pos] == '.':
            recv_end = dot_pos
            recv_start = recv_end
            while recv_start > 0 and (src_line[recv_start - 1].isalnum() or src_line[recv_start - 1] == '_'):
                recv_start -= 1
            receiver_word = src_line[recv_start:recv_end]
            if receiver_word and ast is not None:
                # Infer receiver type
                receiver_type = _infer_receiver_type(ast, receiver_word, db)
                if receiver_type:
                    src = db.get_source(receiver_type)
                    script_path2 = db.get_script_path(receiver_type)
                    if script_path2 and os.path.exists(script_path2):
                        def_line = _find_definition_in_source(src or "", word) if src else 0
                        return DefinitionResult(path=script_path2, line=def_line)

    return None


def _search_local_definition(ast, text: str, word_lower: str) -> int | None:
    """Search AST for a local definition matching word_lower. Returns 0-based line or None."""
    import re
    all_names = (
        [(f.name, "function") for f in ast.functions]
        + [(e.name, "event") for e in ast.events]
        + [(p.name, "property") for p in ast.properties]
        + [(v.name, "variable") for v in ast.variables]
        + [(s.name, "state") for s in ast.states]
    )
    for name, kind in all_names:
        if name.lower() == word_lower:
            patterns = {
                "function": re.compile(r'\bfunction\s+' + re.escape(name) + r'\b', re.IGNORECASE),
                "event": re.compile(r'\bevent\s+' + re.escape(name) + r'\b', re.IGNORECASE),
                "property": re.compile(r'\bproperty\s+' + re.escape(name) + r'\b', re.IGNORECASE),
                "state": re.compile(r'\bstate\s+' + re.escape(name) + r'\b', re.IGNORECASE),
                "variable": re.compile(r'\b' + re.escape(name) + r'\b', re.IGNORECASE),
            }
            pat = patterns.get(kind, patterns["variable"])
            for i, line in enumerate(text.split("\n")):
                if pat.search(line):
                    return i
    return None


def _infer_receiver_type(ast, receiver_word: str, db) -> str | None:
    """Infer type of receiver from script scope."""
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
