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


def test_no_dot_and_parse_failure_both_fall_back_to_script_names():
    db = MockScriptDB()

    # Phase 1: no dot in the expression falls back to prefix-matched script names
    text = "ScriptName TestScript\nAc"
    items = get_completions(text, line=1, col=2, db=db)
    labels = [i.label for i in items]
    # Prefix "Ac" should match "Actor", "ActorBase"
    assert any("Actor" in l for l in labels)

    text2 = "ScriptName TestScript\nGame"
    items2 = get_completions(text2, line=1, col=4, db=db)
    labels2 = [i.label for i in items2]
    assert "Game" in labels2

    # Phase 2: a parse failure mid-expression falls back to all known script names (never [])
    text3 = "ScriptName TestScript\nActor akActor = ("
    items3 = get_completions(text3, line=1, col=17, db=db)
    assert len(items3) > 0
