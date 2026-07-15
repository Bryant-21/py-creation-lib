"""Tests for Plugin.copy_record / copy_override — cross-plugin record copy.

Guards the native code path: it must add the source plugin as a master (matching
the pure-Python path) so an override keeps the source FormID retargeted at the
source master instead of being silently treated as a new target-local record.

Sources round-trip through disk (save + reload) to mirror the real CLI flow,
where local records carry their owner index rather than the in-memory sentinel.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.esp.plugin import Plugin


def _le4(form_id: int) -> bytes:
    return (int(form_id) & 0xFFFFFFFF).to_bytes(4, "little")


def _build_source(tmp_path: Path) -> Path:
    """B21_Src.esp on disk holding recA and recB, where recB.CNAM -> recA."""
    path = tmp_path / "B21_Src.esp"
    src = Plugin.new("B21_Src.esp", game="fo4", masters=["Fallout4.esm"])
    rec_a = src.new_record("WEAP")
    src.add_record(rec_a)
    rec_b = src.new_record("WEAP")
    rec_b.add_subrecord("EDID", b"B21_CopyMe\x00")
    rec_b.add_subrecord("CNAM", _le4(rec_a.form_id), semantic_type="formid")
    src.add_record(rec_b)
    src.save(path)
    src.close()
    return path


def _empty_target(tmp_path: Path) -> Path:
    path = tmp_path / "B21_Tgt.esp"
    tgt = Plugin.new("B21_Tgt.esp", game="fo4", masters=["Fallout4.esm"])
    tgt.save(path)
    tgt.close()
    return path


def _resolve(plugin: Plugin, editor_id: str):
    fid = int(plugin.eid_index()[editor_id][0].split(":")[-1], 16)
    return plugin.get_record_by_form_id(fid)


def test_copy_override_adds_source_master_and_retargets_formid(tmp_path: Path) -> None:
    src = Plugin.load(_build_source(tmp_path))
    tgt = Plugin.load(_empty_target(tmp_path))
    try:
        rec = _resolve(src, "b21_copyme")
        source_object_id = rec.form_id & 0x00FFFFFF

        copied = tgt.copy_override(rec, src)

        assert copied is not None
        assert "b21_src.esp" in [m.lower() for m in tgt.header.masters]
        ref = tgt.normalize_form_id(copied.form_id)
        assert ref.object_id == source_object_id
        assert (ref.plugin_name or "").lower() == "b21_src.esp"
    finally:
        src.close()
        tgt.close()


def test_copy_record_allocates_local_id_and_adds_source_master(tmp_path: Path) -> None:
    src = Plugin.load(_build_source(tmp_path))
    tgt = Plugin.load(_empty_target(tmp_path))
    try:
        rec = _resolve(src, "b21_copyme")
        source_object_id = rec.form_id & 0x00FFFFFF

        copied = tgt.copy_record(rec, src)

        assert copied is not None
        # Source added as a master so the record's own FormID subrecords resolve.
        assert "b21_src.esp" in [m.lower() for m in tgt.header.masters]
        ref = tgt.normalize_form_id(copied.form_id)
        # New record is owned by the target, not an override of the source.
        assert (ref.plugin_name or "").lower() != "b21_src.esp"
        assert (copied.form_id & 0x00FFFFFF) != source_object_id
    finally:
        src.close()
        tgt.close()
