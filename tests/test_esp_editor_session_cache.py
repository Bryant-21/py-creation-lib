"""Tests for conflict_status caching and invalidate_form_id in EditorSession."""
from __future__ import annotations

from creation_lib.esp.editor.session import ConflictStatus, EditorSession


def _make_session() -> EditorSession:
    return EditorSession(default_game="fo4")


def test_invalidate_form_id_clears_only_the_given_entry_in_both_caches():
    session = _make_session()
    session._conflict_cache[0x100] = ConflictStatus.NO_CONFLICT
    session._conflict_cache[0x200] = ConflictStatus.CONFLICT
    session._resolve_cache[0x100] = (1, object())

    session.invalidate_form_id(0x100)

    assert 0x100 not in session._conflict_cache
    assert 0x100 not in session._resolve_cache
    assert 0x200 in session._conflict_cache

    session.invalidate_form_id(0xDEADBEEF)  # missing key is a no-op, does not raise


def test_invalidate_cache_clears_everything():
    session = _make_session()
    session._conflict_cache[0x1] = ConflictStatus.CONFLICT
    session._resolve_cache[0x1] = (1, object())

    session._invalidate_cache()

    assert len(session._conflict_cache) == 0
    assert len(session._resolve_cache) == 0


def test_conflict_status_with_no_plugins_is_only_one_and_gets_cached():
    session = _make_session()

    assert session.conflict_status(0xABC) == ConflictStatus.ONLY_ONE
    assert session._conflict_cache[0xABC] == ConflictStatus.ONLY_ONE

    # A cached value short-circuits recomputation.
    session._conflict_cache[0xABC] = ConflictStatus.CONFLICT
    assert session.conflict_status(0xABC) == ConflictStatus.CONFLICT
