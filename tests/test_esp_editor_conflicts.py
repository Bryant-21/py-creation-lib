"""Tests for load_folder / import_load_order helpers.

The conflict scanner itself is now implemented in Rust
(`py_creation_lib/native/esp/src/conflicts.rs`) and consumes parsed plugin data directly
from the native handle store. Scanner behavior is exercised end-to-end
by integration tests that load real plugins; the previous fake-record
monkeypatch suite was deleted alongside the Python scanner.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.esp.editor import EditorSession
from creation_lib.esp.editor import session as session_module


def _plugin(handle: int, name: str, lo_index: int):
    return session_module.LoadedPlugin(
        handle=handle,
        path=f"/fake/{name}",
        game="fo4",
        is_master=name.lower().endswith(".esm"),
        load_order_index=lo_index,
        plugin_name=name,
    )


def test_parse_load_order_file_handles_star_and_comments(tmp_path: Path):
    p = tmp_path / "loadorder.txt"
    p.write_text("# comment\n*Fallout4.esm\nDLCRobot.esm\n\n*ModA.esp\n", encoding="utf-8")
    out = session_module._parse_load_order_file(p)
    assert out == ["Fallout4.esm", "DLCRobot.esm", "ModA.esp"]


def test_order_plugins_alpha_with_esm_first(tmp_path: Path):
    a = tmp_path / "ModB.esp"; a.write_bytes(b"")
    b = tmp_path / "ModA.esp"; b.write_bytes(b"")
    c = tmp_path / "Master.esm"; c.write_bytes(b"")
    ordered = session_module._order_plugins([a, b, c], None)
    assert [p.name for p in ordered] == ["Master.esm", "ModA.esp", "ModB.esp"]


def test_order_plugins_uses_explicit_ordering(tmp_path: Path):
    a = tmp_path / "ModA.esp"; a.write_bytes(b"")
    b = tmp_path / "ModB.esp"; b.write_bytes(b"")
    c = tmp_path / "Master.esm"; c.write_bytes(b"")
    ordered = session_module._order_plugins([a, b, c], ["ModB.esp", "Master.esm", "ModA.esp"])
    assert [p.name for p in ordered] == ["ModB.esp", "Master.esm", "ModA.esp"]


def test_import_load_order_reorders_existing_plugins(tmp_path: Path):
    s = EditorSession(default_game="fo4", auto_scan_conflicts=False)
    s._plugins = [
        _plugin(1, "ModA.esp", 0),
        _plugin(2, "Master.esm", 1),
        _plugin(3, "ModB.esp", 2),
    ]
    p = tmp_path / "loadorder.txt"
    p.write_text("Master.esm\nModB.esp\nModA.esp\n", encoding="utf-8")
    s.import_load_order(p)
    assert [pl.plugin_name for pl in s._plugins] == ["Master.esm", "ModB.esp", "ModA.esp"]
    assert [pl.load_order_index for pl in s._plugins] == [0, 1, 2]
