"""Header field/flag edits must survive a disk round-trip.

Regression: the TES4 header keeps a verbatim `raw_subrecords` cache that the
serializer prefers over the parsed header fields. Editing author/flags/etc. must
invalidate that cache (native `set_header_field`) or the edit is silently dropped
on save — show-mode reads the live field and looked correct while disk did not.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.esp.editor import header_flags
from creation_lib.esp.plugin import Plugin


def _roundtrip(tmp_path: Path, mutate) -> Plugin:
    path = tmp_path / "B21_HeaderEdit.esp"
    p = Plugin.new("B21_HeaderEdit.esp", game="fo4", masters=["Fallout4.esm"])
    p.save(path)
    p.close()
    p = Plugin.load(path, game="fo4")
    try:
        mutate(p)
        p.save(path)
    finally:
        p.close()
    return Plugin.load(path, game="fo4")


def test_field_edits_persist(tmp_path: Path) -> None:
    def mutate(p: Plugin) -> None:
        p.header.author = "B21"
        p.header.description = "desc"
        p.header.next_object_id = 0x901

    r = _roundtrip(tmp_path, mutate)
    try:
        assert r.header.author == "B21"
        assert r.header.description == "desc"
        assert (int(r.header.next_object_id) & 0x00FFFFFF) == 0x901
        assert [m.lower() for m in r.header.masters] == ["fallout4.esm"]  # untouched
    finally:
        r.close()


def test_flag_edit_persists(tmp_path: Path) -> None:
    r = _roundtrip(tmp_path, lambda p: header_flags.set_light(p._rust_handle, True))
    try:
        assert header_flags.is_light(r._rust_handle) is True
    finally:
        r.close()
