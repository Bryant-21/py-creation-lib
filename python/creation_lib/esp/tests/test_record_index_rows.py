from __future__ import annotations

from creation_lib.esp.model import Record
from creation_lib.esp.plugin import Plugin


def _record(signature: str, form_id: int, editor_id: str) -> Record:
    record = Record(signature, form_id)
    record.add_subrecord("EDID", editor_id.encode("cp1252") + b"\x00")
    return record


def test_record_index_rows_supports_signature_and_form_key_filters() -> None:
    plugin = Plugin.new("B21_Index.esp", game="fo4")
    try:
        plugin.add_record(_record("MISC", 0xFF000800, "B21_Misc"))
        plugin.add_record(_record("STAT", 0xFF000801, "B21_Static"))

        rows = plugin.record_index_rows(signatures=["stat"])
        assert rows == [
            ("B21_Index.esp:000801", "B21_Static", "STAT", 0x801, 0xFF000801)
        ]

        rows = plugin.record_index_rows(
            form_keys=["B21_Index.esp:000801", "B21_Index.esp:000800"]
        )
        assert [row[1] for row in rows] == ["B21_Static", "B21_Misc"]
    finally:
        plugin.close()
