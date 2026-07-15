"""Tests for conflict_status caching and invalidate_form_id in EditorSession."""
from __future__ import annotations

from unittest.mock import MagicMock, patch

import pytest

from creation_lib.esp.editor.session import ConflictStatus, EditorSession


def _make_session() -> EditorSession:
    return EditorSession(default_game="fo4")


def _make_record(payloads: list[bytes]):
    """Build a fake record whose subrecords produce the given payloads."""
    subrecords = []
    for p in payloads:
        sub = MagicMock()
        sub.data = p
        subrecords.append(sub)
    record = MagicMock()
    record.subrecords = subrecords
    return record


class TestConflictStatusCache:
    def test_result_is_cached_on_second_call(self):
        session = _make_session()
        call_count = 0

        original_conflict = session.conflict_status

        def _patched(form_id):
            nonlocal call_count
            call_count += 1
            return ConflictStatus.ONLY_ONE

        # Manually warm the cache as the method would
        session._conflict_cache[0x12345] = ConflictStatus.ONLY_ONE

        # Second call should hit the cache without touching _plugins
        result = session.conflict_status(0x12345)
        assert result == ConflictStatus.ONLY_ONE

    def test_invalidate_form_id_clears_single_entry(self):
        session = _make_session()
        session._conflict_cache[0x100] = ConflictStatus.NO_CONFLICT
        session._conflict_cache[0x200] = ConflictStatus.CONFLICT

        session.invalidate_form_id(0x100)

        assert 0x100 not in session._conflict_cache
        assert 0x200 in session._conflict_cache

    def test_invalidate_form_id_clears_resolve_cache_too(self):
        session = _make_session()
        session._resolve_cache[0x100] = (1, object())
        session._conflict_cache[0x100] = ConflictStatus.ONLY_ONE

        session.invalidate_form_id(0x100)

        assert 0x100 not in session._resolve_cache
        assert 0x100 not in session._conflict_cache

    def test_invalidate_form_id_missing_key_is_noop(self):
        session = _make_session()
        # Should not raise even if form_id is not cached
        session.invalidate_form_id(0xDEADBEEF)

    def test_invalidate_cache_clears_conflict_cache(self):
        session = _make_session()
        session._conflict_cache[0x1] = ConflictStatus.CONFLICT
        session._conflict_cache[0x2] = ConflictStatus.OVERRIDE

        session._invalidate_cache()

        assert len(session._conflict_cache) == 0
        assert len(session._resolve_cache) == 0

    def test_conflict_status_only_one_when_no_plugins(self):
        session = _make_session()
        # No plugins loaded — should return ONLY_ONE and cache it
        with patch.object(session, "_call", return_value=None):
            result = session.conflict_status(0xABC)
        assert result == ConflictStatus.ONLY_ONE
        assert session._conflict_cache.get(0xABC) == ConflictStatus.ONLY_ONE

    def test_conflict_status_uses_cached_value_on_repeat_call(self):
        session = _make_session()
        session._conflict_cache[0x999] = ConflictStatus.CONFLICT

        # Even though there are no plugins, cached value should be returned
        result = session.conflict_status(0x999)
        assert result == ConflictStatus.CONFLICT

    def test_invalidate_form_id_then_recompute(self):
        session = _make_session()
        # Prime the cache
        session._conflict_cache[0x50] = ConflictStatus.CONFLICT
        # Invalidate it
        session.invalidate_form_id(0x50)
        assert 0x50 not in session._conflict_cache
        # Next call with no plugins produces ONLY_ONE and caches it
        with patch.object(session, "_call", return_value=None):
            result = session.conflict_status(0x50)
        assert result == ConflictStatus.ONLY_ONE
        assert session._conflict_cache[0x50] == ConflictStatus.ONLY_ONE
