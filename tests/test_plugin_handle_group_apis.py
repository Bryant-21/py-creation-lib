"""Smoke tests for native group summary APIs."""
from __future__ import annotations

from types import SimpleNamespace
from typing import Any

import pytest

import creation_lib.esp.native_runtime as native_runtime


@pytest.fixture(autouse=True)
def _reset_native_cache(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", False)


def _make_fake_module(**fns: Any) -> SimpleNamespace:
    return SimpleNamespace(**fns)


def test_group_signatures_returns_label_count_pairs(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    raw_pairs = [("WEAP", 42), ("ARMO", 100)]

    def fake_plugin_handle_group_signatures(handle_id: int) -> list[tuple[str, int]]:
        assert handle_id == 99
        return raw_pairs

    module = _make_fake_module(plugin_handle_group_signatures=fake_plugin_handle_group_signatures)
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: module)

    result = native_runtime.plugin_handle_group_signatures(99)

    assert result == [("WEAP", 42), ("ARMO", 100)]
    for label, count in result:
        assert isinstance(label, str)
        assert isinstance(count, int)


def test_group_record_summaries_return_record_summary_objects(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    raw_items = [(0x12345, "WEAP", "NativeWeapon")]

    def fake_plugin_handle_group_record_summaries(handle_id: int, sig: str) -> list[tuple[int, str, str]]:
        assert handle_id == 7
        assert sig == "WEAP"
        return raw_items

    module = _make_fake_module(plugin_handle_group_record_summaries=fake_plugin_handle_group_record_summaries)
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: module)

    result = native_runtime.plugin_handle_group_record_summaries(7, "WEAP")

    assert result == [native_runtime.RecordSummary(0x12345, "WEAP", "NativeWeapon")]


def test_group_record_summaries_empty_for_unknown_signature(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fake_plugin_handle_group_record_summaries(handle_id: int, sig: str) -> list[Any]:
        return []

    module = _make_fake_module(plugin_handle_group_record_summaries=fake_plugin_handle_group_record_summaries)
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: module)

    result = native_runtime.plugin_handle_group_record_summaries(1, "XXXX")
    assert result == []


def _try_load_native() -> Any | None:
    try:
        return native_runtime.load_native_module()
    except Exception:
        return None


_NATIVE_AVAILABLE = _try_load_native() is not None


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_integration_group_signatures_empty_on_new_plugin() -> None:
    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    handle = native_runtime.plugin_handle_new("GroupSigTest", "fo4")
    try:
        assert native_runtime.plugin_handle_group_signatures(handle) == []
    finally:
        native_runtime.plugin_handle_close(handle)


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_integration_group_record_summaries_empty_on_new_plugin() -> None:
    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    handle = native_runtime.plugin_handle_new("GroupRecTest", "fo4")
    try:
        assert native_runtime.plugin_handle_group_record_summaries(handle, "WEAP") == []
    finally:
        native_runtime.plugin_handle_close(handle)
