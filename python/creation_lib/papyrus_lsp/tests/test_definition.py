"""Tests for get_definition()."""
import os
import pytest
from unittest.mock import MagicMock


class MockScriptDB:
    def __init__(self, scripts: dict):
        """scripts: {name: {"path": str, "source": str, "hierarchy": [str]}}"""
        self._scripts = {k.lower(): v for k, v in scripts.items()}

    def script_exists(self, name):
        return name.lower() in self._scripts

    def get_script_path(self, name):
        return self._scripts.get(name.lower(), {}).get("path")

    def get_source(self, name):
        return self._scripts.get(name.lower(), {}).get("source")

    def get_hierarchy(self, name):
        return self._scripts.get(name.lower(), {}).get("hierarchy", [name])

    def get_functions(self, name):
        return self._scripts.get(name.lower(), {}).get("functions", [])

    def get_properties(self, name):
        return self._scripts.get(name.lower(), {}).get("properties", [])


def test_definition_on_script_name_returns_path(tmp_path):
    """Clicking 'Actor' (a type name) should jump to Actor.psc."""
    from creation_lib.papyrus_lsp.definition import get_definition

    actor_psc = tmp_path / "Actor.psc"
    actor_psc.write_text("ScriptName Actor\n")

    db = MockScriptDB({"Actor": {"path": str(actor_psc), "source": "ScriptName Actor\n"}})
    text = "ScriptName TestScript\nActor akActor"
    result = get_definition(text, line=1, col=2, db=db)
    assert result is not None
    assert result.path == str(actor_psc)
    assert result.line == 0


def test_definition_on_unknown_word_returns_none():
    from creation_lib.papyrus_lsp.definition import get_definition

    db = MockScriptDB({})
    text = "ScriptName TestScript\nUnknownThing"
    result = get_definition(text, line=1, col=3, db=db)
    assert result is None


def test_definition_on_whitespace_returns_none():
    from creation_lib.papyrus_lsp.definition import get_definition

    db = MockScriptDB({})
    text = "ScriptName TestScript\n   "
    result = get_definition(text, line=1, col=1, db=db)
    assert result is None
