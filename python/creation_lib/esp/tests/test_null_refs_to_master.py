"""Unit test for the plugin_handle_null_refs_to_master native helper.

Backs `esp masters remove --force`: every formid/formid_array subrecord pointing at
the removed master index is set to NULL. Uses an in-memory authored plugin so the
formid semantic types are present (a disk round-trip would re-derive them from the
schema, which the helper shares with set_masters/copy).
"""
from __future__ import annotations

from creation_lib.esp import native_runtime as nr
from creation_lib.esp.plugin import Plugin


def test_null_refs_to_master_nulls_only_target_index() -> None:
    p = Plugin.new("B21_Null.esp", game="fo4", masters=["Fallout4.esm", "DLCRobot.esm"])
    rec = p.new_record("FLST")
    rec.add_subrecord("EDID", b"B21_List\x00")
    rec.add_subrecord("LNAM", (0x01000ABC).to_bytes(4, "little"), semantic_type="formid")  # -> DLCRobot (idx 1)
    rec.add_subrecord("LNAM", (0x00000111).to_bytes(4, "little"), semantic_type="formid")  # -> Fallout4 (idx 0)
    p.add_record(rec)
    handle = p._rust_handle
    try:
        assert 1 in set(nr.plugin_handle_call(handle, "used_master_indices"))
        nulled = nr.plugin_handle_null_refs_to_master(handle, 1)
        assert nulled == 1  # only the DLCRobot ref, not the Fallout4 ref
        assert 1 not in set(nr.plugin_handle_call(handle, "used_master_indices"))
        assert 0 in set(nr.plugin_handle_call(handle, "used_master_indices"))  # Fallout4 ref intact
    finally:
        p.close()
