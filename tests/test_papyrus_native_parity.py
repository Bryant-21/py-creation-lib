"""Native parser shape tests — verifies the Rust parser produces the expected
ScriptNode dataclass shape across a range of inputs.

After the Python→Rust cutover, the Python lark-based parser is gone; these
tests now run native-only. The original parity comparison vs. the Python
backend is preserved in git history.
"""
from __future__ import annotations

import pytest

from creation_lib.papyrus_lsp import native_runtime as nr


def _native_ready() -> bool:
    """True iff the loaded papyrus_core has the full bindings set."""
    if not nr.is_available():
        return False
    mod = nr.load_native_module()
    if mod is None:
        return False
    return hasattr(mod, "session_open") and hasattr(mod, "session_diagnostics")


pytestmark = pytest.mark.skipif(
    not _native_ready(),
    reason="papyrus_core native build is missing/stub — rebuild via "
    "`uv sync --reinstall-package modbox21-native`",
)


def parse_via_facade(text: str):
    from creation_lib.papyrus_lsp import parse_script as facade_parse
    return facade_parse(text=text)


CASES = [
    ("ScriptName Foo extends Bar\n", "Foo", "Bar", 0, 0),
    ("Scriptname Game Native Hidden\n", "Game", None, 0, 0),
    ("ScriptName B21:Mod extends ObjectReference\n", "B21:Mod", "ObjectReference", 0, 0),
    ("ScriptName Foo\nInt Property MyProp Auto\n", "Foo", None, 1, 0),
    ("ScriptName Foo\nInt Property X Auto\nFunction DoIt()\nEndFunction\n", "Foo", None, 1, 1),
    ("ScriptName Foo\nImport Game\n", "Foo", None, 0, 0),
    ("ScriptName Foo\nFunction Bar(Int x = 1, Float y = 2.5)\nEndFunction\n", "Foo", None, 0, 1),
]


@pytest.mark.parametrize("src,exp_name,exp_parent,exp_props,exp_funcs", CASES)
def test_native_parser_shape(src, exp_name, exp_parent, exp_props, exp_funcs):
    nat = parse_via_facade(src)
    assert nat.ast is not None, f"native failed: {nat.errors}"
    assert nat.ast.name == exp_name
    assert nat.ast.parent == exp_parent
    assert len(nat.ast.properties) == exp_props
    assert len(nat.ast.functions) == exp_funcs


def test_native_filename_validation():
    from creation_lib.papyrus_lsp import validate_filename as facade_validate

    nat = facade_validate("/path/WrongName.psc", "MyScript")
    assert nat is not None
    assert "WrongName" in nat.message

    assert facade_validate("/path/B21/TestScript.psc", "B21:TestScript") is None


def test_native_emit_round_trip():
    """Parse, emit via native, ensure the result re-parses."""
    src = (
        "ScriptName Foo extends Bar\n"
        "Int Property X Auto\n"
        "Function DoIt()\n"
        "  Int y = X + 1\n"
        "EndFunction\n"
    )
    nat = parse_via_facade(src)
    assert nat.ast is not None

    rendered = nr.emit_script_native(nat.ast)
    assert "Scriptname Foo Extends Bar" in rendered
    assert "Int Property X" in rendered
    assert "Function DoIt" in rendered

    again = parse_via_facade(rendered)
    assert again.ast is not None
    assert again.ast.name == "Foo"
    assert len(again.ast.functions) == 1
