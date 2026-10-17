from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.esp import native_runtime as nr
from creation_lib.esp.model import Record
from creation_lib.esp.plugin import Plugin


def _record(signature: str, form_id: int, editor_id: str) -> Record:
    record = Record(signature, form_id)
    record.add_subrecord("EDID", editor_id.encode("cp1252") + b"\x00")
    record.add_subrecord("FULL", editor_id.encode("cp1252") + b"\x00")
    return record


@pytest.fixture()
def fixture_plugin(tmp_path: Path) -> Path:
    plugin = Plugin.new("B21_LazyEquiv.esp", game="fo4")
    try:
        for i in range(64):
            signature = ("MISC", "STAT", "WEAP", "ARMO")[i % 4]
            plugin.add_record(
                _record(signature, 0xFF000800 + i, f"B21_Lazy_{signature}_{i:03d}")
            )
        target = tmp_path / "B21_LazyEquiv.esp"
        plugin.save(target)
    finally:
        plugin.close()
    return target


def _read_all(handle, form_ids: list[int]) -> dict[int, str]:
    out = {}
    for form_id in form_ids:
        try:
            out[form_id] = nr.plugin_handle_call(
                handle, "export_record_text", form_id, "json"
            )
        except (KeyError, RuntimeError) as err:
            out[form_id] = f"ERROR: {type(err).__name__}: {err}"
    return out


def test_lazy_handle_reads_match_full_handle_on_fixture(fixture_plugin: Path) -> None:
    # eager_compressed must match what the lazy path does when it materializes a
    # record (it always inflates), and True is what Plugin.load defaults to.
    # Loading the full side with False leaves COMPRESSED records as raw payload
    # with no decoded fields, which is a difference in the comparison rather than
    # in the code under test.
    full = nr.plugin_handle_load(str(fixture_plugin), game="fo4", eager_compressed=True)
    lazy = nr.plugin_handle_load_index(str(fixture_plugin), game="fo4")
    try:
        form_ids = list(nr.plugin_handle_call(full, "record_form_ids", None))
        assert form_ids, f"no records found in {fixture_plugin}"

        full_out = _read_all(full, form_ids)
        lazy_out = _read_all(lazy, form_ids)

        mismatched = [f for f in form_ids if full_out[f] != lazy_out[f]]
        assert not mismatched, (
            f"{len(mismatched)}/{len(form_ids)} records differ between the full "
            f"and lazy handles. First: {mismatched[0]:08X}\n"
            f"  full: {full_out[mismatched[0]][:400]}\n"
            f"  lazy: {lazy_out[mismatched[0]][:400]}"
        )
    finally:
        nr.plugin_handle_close(full)
        nr.plugin_handle_close(lazy)


def test_lazy_search_matches_eager_and_rejects_mutation(fixture_plugin: Path) -> None:
    full = nr.plugin_handle_load(str(fixture_plugin), game="fo4", eager_compressed=True)
    lazy = nr.plugin_handle_load_index(str(fixture_plugin), game="fo4")
    try:
        for options in (
            {"pattern": "*", "signatures": ["STAT", "MISC"]},
            {"pattern": "b21_lazy_misc_000", "signatures": ["MISC"]},
            {"pattern": "B21_Lazy_MISC_000", "signatures": ["MISC"], "case_sensitive": True},
            {"pattern": "*", "signatures": ["STAT", "MISC"], "limit": 3},
            {"pattern": "*", "signatures": ["STAT"], "read_full": True},
            {"pattern": "_MISC_", "signatures": ["MISC"], "mode": "substring"},
        ):
            assert nr.plugin_handle_search_records(lazy, **options) == nr.plugin_handle_search_records(full, **options)
        match = nr.plugin_handle_search_records(lazy, "B21_Lazy_MISC_000", signatures=["MISC"])[0]
        with pytest.raises(KeyError, match="unknown record"):
            nr.plugin_handle_set_record_subrecords(
                lazy, match["form_id"], [("EDID", b"B21_Changed\0", None)],
            )
        assert nr.plugin_handle_search_records(lazy, "B21_Lazy_MISC_000", signatures=["MISC"]) == [match]
    finally:
        nr.plugin_handle_close(full)
        nr.plugin_handle_close(lazy)
