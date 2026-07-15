"""Tests for Plugin.set_masters — replace/reorder masters, remap FormIDs by name.

Reordering masters must remap every in-plugin FormID by master *name*, so a record
overriding a moved master keeps resolving to it (only the high-byte master index
changes). Round-trips through disk to mirror the real CLI flow.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.esp.plugin import Plugin


def _build(tmp_path: Path) -> Path:
    """Plugin with masters [Fallout4, DLCRobot] and a record overriding DLCRobot:000ABC."""
    path = tmp_path / "B21_Reorder.esp"
    p = Plugin.new("B21_Reorder.esp", game="fo4", masters=["Fallout4.esm", "DLCRobot.esm"])
    rec = p.new_record("WEAP", form_id=0x01000ABC)  # master index 1 -> DLCRobot.esm
    rec.add_subrecord("EDID", b"B21_Override\x00")
    p.add_record(rec)
    p.save(path)
    p.close()
    return path


def _summary(plugin: Plugin, editor_id: str):
    idx = plugin.eid_index()
    hit = idx.get(editor_id) or next((v for k, v in idx.items() if k.lower() == editor_id.lower()), None)
    object_id = int(hit[0].split(":")[-1], 16)
    return plugin.get_record_by_form_id(object_id)


def test_set_masters_reorder_remaps_record_index_by_name(tmp_path: Path) -> None:
    p = Plugin.load(_build(tmp_path))
    try:
        before = _summary(p, "B21_Override")
        assert before.form_id >> 24 == 1  # DLCRobot is master index 1
        assert (p.normalize_form_id(before.form_id).plugin_name or "").lower() == "dlcrobot.esm"

        p.set_masters([("DLCRobot.esm", 0), ("Fallout4.esm", 0)])
        out = tmp_path / "B21_Reorder_out.esp"
        p.save(out)
    finally:
        p.close()

    rp = Plugin.load(out)
    try:
        assert [m.lower() for m in rp.header.masters] == ["dlcrobot.esm", "fallout4.esm"]
        after = _summary(rp, "B21_Override")
        assert after.form_id >> 24 == 0  # DLCRobot moved to index 0 — record followed by name
        assert (after.form_id & 0x00FFFFFF) == 0xABC
        assert (rp.normalize_form_id(after.form_id).plugin_name or "").lower() == "dlcrobot.esm"
    finally:
        rp.close()
