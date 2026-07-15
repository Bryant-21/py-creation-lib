from __future__ import annotations

from pathlib import Path

from creation_lib.esp import native_runtime
from creation_lib.esp.model import Record
from creation_lib.esp.plugin import Plugin


def _record(form_id: int, editor_id: str, full_name: str | None = None) -> Record:
    record = Record("MISC", form_id)
    record.add_subrecord("EDID", editor_id.encode("cp1252") + b"\x00")
    if full_name is not None:
        record.add_subrecord("FULL", full_name.encode("cp1252") + b"\x00")
    return record


def test_search_records_uses_native_batch_call(monkeypatch) -> None:
    plugin = Plugin.new("B21_Search.esp", game="fo4", masters=[])
    try:
        plugin.add_record(_record(0xFF000800, "B21_PlasmaGun", "Plasma Weapon"))
        plugin.add_record(_record(0xFF000801, "B21_LaserGun", "Laser Weapon"))

        monkeypatch.setattr(
            native_runtime,
            "plugin_handle_record_form_ids",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("serial form-id scan")),
        )
        monkeypatch.setattr(
            native_runtime,
            "plugin_handle_record_summary",
            lambda *_args, **_kwargs: (_ for _ in ()).throw(AssertionError("per-record summary")),
        )

        matches = plugin.search_records("*plasma*", match_full=True)
        assert [match["editor_id"] for match in matches] == ["B21_PlasmaGun"]
        assert matches[0]["full_name"] == "Plasma Weapon"
    finally:
        plugin.close()


def test_parallel_parse_and_save_are_byte_stable(tmp_path: Path) -> None:
    source = tmp_path / "B21_ParallelIo.esp"
    roundtrip = tmp_path / "B21_ParallelIoRoundtrip.esp"
    plugin = Plugin.new(source.name, game="fo4", masters=[])
    try:
        for index in range(64):
            record = _record(0xFF000800 + index, f"B21_Record{index:02d}")
            record.add_subrecord("DATA", bytes([index]) * (20 * 1024))
            plugin.add_record(record)
        plugin.save(source)
    finally:
        plugin.close()

    loaded = Plugin.load(source)
    try:
        assert loaded.record_count == 64
        loaded.save(roundtrip)
    finally:
        loaded.close()

    assert roundtrip.read_bytes() == source.read_bytes()
