"""The host app can own the game selection instead of the workspace."""
import pytest

from creation_lib.ui.workspaces.voice_browser import VoiceBrowserWorkspace


def _workspace(**kwargs):
    return VoiceBrowserWorkspace(None, **kwargs)


def test_game_selector_defaults_and_set_game_selects_ignores_unknown_and_is_noop_for_current():
    # Phase 1: game_selector constructor default and override
    assert _workspace().game_selector is True
    assert _workspace(game_selector=False).game_selector is False

    # Phase 2: set_game() selects a known game, ignores unknown ones, and is a no-op for the current game
    ws = _workspace(game_selector=False)
    ws.set_game("skyrimse")
    assert ws._selected_game == "skyrimse"

    before = ws._selected_game
    ws.set_game("not-a-game")
    assert ws._selected_game == before

    sentinel = object()
    ws._index = sentinel
    ws.set_game(ws._selected_game)
    assert ws._index is sentinel


def test_set_game_invalidates_the_loaded_index():
    ws = _workspace(game_selector=False)
    ws._index = object()
    ws._selected_group = "Some Voice"
    ws._selected_plugin = "skyrim.esm"
    ws._selected_line_idx = 7
    ws._load_attempt_key = ("fo4", "English")

    ws.set_game("fnv")

    assert ws._selected_game == "fnv"
    assert ws._index is None
    assert ws._selected_group == ""
    assert ws._selected_plugin == ""
    assert ws._selected_line_idx == -1
    assert ws._load_attempt_key is None
    assert ws._dirty_filter is True
