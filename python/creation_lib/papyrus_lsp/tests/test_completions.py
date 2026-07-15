"""Tests for get_completions()."""
import os
import pytest
from unittest.mock import MagicMock, patch

from creation_lib.papyrus_lsp.completions import get_completions
from creation_lib.papyrus_lsp.completions import CompletionItem


class MockScriptDB:
    """Minimal ScriptDB mock for completion tests."""
    def script_exists(self, name):
        return name.lower() in {"actor", "game", "objectreference"}

    def get_all_members(self, name):
        if name.lower() == "actor":
            return {
                "functions": [{"name": "GetActorValue", "return_type": "Float", "params": "ActorValue akValue"}],
                "properties": [{"name": "Race", "type": "Race"}],
                "events": [],
            }
        return {"functions": [], "properties": [], "events": []}

    def search_scripts(self, prefix):
        all_scripts = ["Actor", "ActorBase", "Game", "ObjectReference"]
        return [s for s in all_scripts if s.lower().startswith(prefix.lower())]


def test_dot_completion_returns_members():
    db = MockScriptDB()
    script = "ScriptName TestScript\nActor akActor\n"
    # Cursor after "akActor." on line 2 (0-based)
    text = "ScriptName TestScript\nActor akActor\nakActor."
    items = get_completions(text, line=2, col=8, db=db)
    labels = [i.label for i in items]
    assert "GetActorValue" in labels
    assert "Race" in labels


def test_no_dot_returns_script_names_as_fallback():
    db = MockScriptDB()
    text = "ScriptName TestScript\nAc"
    items = get_completions(text, line=1, col=2, db=db)
    labels = [i.label for i in items]
    # Prefix "Ac" should match "Actor", "ActorBase"
    assert any("Actor" in l for l in labels)


def test_partial_script_name_completion():
    db = MockScriptDB()
    text = "ScriptName TestScript\nGame"
    items = get_completions(text, line=1, col=4, db=db)
    labels = [i.label for i in items]
    assert "Game" in labels


def test_parse_failure_fallback_returns_script_names():
    """On parse failure mid-expression, fall back to all known script names (never [])."""
    db = MockScriptDB()
    # Incomplete expression that will fail to parse
    text = "ScriptName TestScript\nActor akActor = ("
    items = get_completions(text, line=1, col=17, db=db)
    # Should not return empty list even on parse failure
    assert len(items) > 0
