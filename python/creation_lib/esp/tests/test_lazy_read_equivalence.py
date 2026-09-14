from __future__ import annotations

import os
import random
from pathlib import Path

import pytest

from creation_lib.esp import native_runtime as nr
from creation_lib.esp.model import Record
from creation_lib.esp.plugin import Plugin

# Overridable so a quick correctness pass can run a small sample; the default is
# what a full verification run uses.
SAMPLE_SIZE = int(os.environ.get("ESP_EQUIV_SAMPLE", "2000"))
SAMPLE_SEED = 20260903


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


def _assert_equivalent(path: Path, game: str, sample: int | None) -> None:
    # eager_compressed must match what the lazy path does when it materializes a
    # record (it always inflates), and True is what Plugin.load defaults to.
    # Loading the full side with False leaves COMPRESSED records as raw payload
    # with no decoded fields, which is a difference in the comparison rather than
    # in the code under test.
    full = nr.plugin_handle_load(str(path), game=game, eager_compressed=True)
    lazy = nr.plugin_handle_load_index(str(path), game=game)
    try:
        form_ids = list(nr.plugin_handle_call(full, "record_form_ids", None))
        assert form_ids, f"no records found in {path}"
        if sample is not None and len(form_ids) > sample:
            form_ids = random.Random(SAMPLE_SEED).sample(form_ids, sample)

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


def test_lazy_handle_reads_match_full_handle_on_fixture(fixture_plugin: Path) -> None:
    _assert_equivalent(fixture_plugin, "fo4", sample=None)


def _game_data_dir(var: str) -> Path | None:
    configured = os.environ.get(var, "").strip().strip('"')
    if not configured:
        env_file = Path(__file__).resolve().parents[5] / ".env"
        if env_file.is_file():
            for line in env_file.read_text(encoding="utf-8").splitlines():
                if line.startswith(f"{var}="):
                    configured = line.split("=", 1)[1].strip().strip('"')
                    break
    if not configured:
        return None
    data = Path(configured) / "Data"
    return data if data.is_dir() else None


@pytest.mark.integration
@pytest.mark.parametrize(
    ("var", "plugin_name", "game"),
    [
        ("FO4_DIR", "Fallout4.esm", "fo4"),
        ("FO76_DIR", "SeventySix.esm", "fo76"),
    ],
)
def test_lazy_handle_reads_match_full_handle_on_shipped_esm(
    var: str, plugin_name: str, game: str
) -> None:
    data_dir = _game_data_dir(var)
    if data_dir is None:
        pytest.skip(f"{var} not configured")
    plugin_path = data_dir / plugin_name
    if not plugin_path.is_file():
        pytest.skip(f"{plugin_path} not present")
    _assert_equivalent(plugin_path, game, sample=SAMPLE_SIZE)


@pytest.mark.integration
def test_single_record_read_does_not_deep_clone_the_tree() -> None:
    """A read must not scale with plugin size.

    Cloning slot.parsed per read copies every Vec<ParsedItem> and
    Vec<ParsedSubrecord> spine in the plugin: 2.6 s for one record on
    SeventySix.esm. Wall-clock is a proxy for that copy, so the bound is loose:
    the clone costs seconds, not milliseconds.
    """
    import time

    data_dir = _game_data_dir("FO76_DIR")
    if data_dir is None:
        pytest.skip("FO76_DIR not configured")
    plugin_path = data_dir / "SeventySix.esm"
    if not plugin_path.is_file():
        pytest.skip(f"{plugin_path} not present")

    handle = nr.plugin_handle_load(str(plugin_path), game="fo76", eager_compressed=True)
    try:
        form_ids = list(nr.plugin_handle_call(handle, "record_form_ids", ["WEAP"]))
        assert form_ids
        nr.plugin_handle_call(handle, "export_record_text", form_ids[0], "json")

        start = time.perf_counter()
        for form_id in form_ids[1:21]:
            nr.plugin_handle_call(handle, "export_record_text", form_id, "json")
        per_read = (time.perf_counter() - start) / 20
    finally:
        nr.plugin_handle_close(handle)

    assert per_read < 0.100, f"{per_read * 1000:.0f} ms per read; expected well under 100 ms"


@pytest.mark.integration
def test_repeated_lazy_reads_do_not_scale_with_plugin_size() -> None:
    """A lazy handle probed once per record must index itself, not rescan.

    plugin_handle_load_index defers the form_id -> offset map and serves the first
    lookup by scanning; the second probe builds the map. Rescanning every probe is
    quadratic for BACUP, which probes its read-only lazy masters once per record.

    Measured on SeventySix.esm: ~25 ms per read rescanning, under 1 ms indexed.
    The bound guards the complexity, not the constant.
    """
    import time

    plugin_path = _fo76_esm()
    handle = nr.plugin_handle_load_index(str(plugin_path), game="fo76")
    try:
        form_ids = list(nr.plugin_handle_call(handle, "record_form_ids", ["WEAP"]))
        assert len(form_ids) > 200
        sample = random.Random(SAMPLE_SEED).sample(form_ids, 200)

        start = time.perf_counter()
        for form_id in sample:
            nr.plugin_handle_call(handle, "export_record_text", form_id, "json")
        per_read = (time.perf_counter() - start) / len(sample)
    finally:
        nr.plugin_handle_close(handle)

    assert per_read < 0.005, (
        f"{per_read * 1000:.1f} ms per lazy read; expected well under 5 ms. "
        "Above this, each lookup is rescanning the plugin instead of using the index."
    )


def _private_gb() -> float:
    import psutil

    return psutil.Process().memory_full_info().private / 1e9


def _fo76_esm() -> Path:
    data_dir = _game_data_dir("FO76_DIR")
    if data_dir is None:
        pytest.skip("FO76_DIR not configured")
    plugin_path = data_dir / "SeventySix.esm"
    if not plugin_path.is_file():
        pytest.skip(f"{plugin_path} not present")
    return plugin_path


def _large_unlocalized_plugin() -> Path:
    """A big plugin with the localized flag clear.

    A localized plugin hydrates its whole string table at open on both load paths
    (293 MB for Fallout4.esm, ~1 GB for SeventySix.esm), which would swamp the
    measurement.
    """
    candidate = Path(__file__).resolve().parents[5] / "mods" / "FNV_FO3" / "FalloutNV.esm"
    if not candidate.is_file():
        pytest.skip(f"{candidate} not present")
    return candidate


@pytest.mark.integration
@pytest.mark.isolated
def test_lazy_load_does_not_materialize_the_plugin() -> None:
    """Opening a plugin lazily must cost nothing proportional to its size.

    Measured on this 523 MB plugin: 1 MB and under 10 ms, because only the TES4
    header is parsed and the body stays a shared file mapping. A failure means the
    record tree, or an index over it, is built at open.
    """
    plugin_path = _large_unlocalized_plugin()

    baseline = _private_gb()
    handle = nr.plugin_handle_load_index(str(plugin_path), game="fo4")
    try:
        after_load = _private_gb() - baseline
        assert after_load < 0.10, (
            f"opening a {plugin_path.stat().st_size / 1e9:.2f} GB plugin committed "
            f"{after_load:.2f} GB; expected < 0.10"
        )
    finally:
        nr.plugin_handle_close(handle)


@pytest.mark.integration
@pytest.mark.isolated
def test_direct_form_id_read_costs_far_less_than_a_full_load() -> None:
    """A direct FormID read on a real, localized master.

    Measured on SeventySix.esm: 5.98 GB for a full load versus 0.29 GB here -
    68 MB of string-table directories, about 1 MB of plugin, and the handful of
    archive tables this record's names actually needed.

    Must run in a fresh process: mimalloc retains freed pages, so a delta taken
    after another test has loaded and closed a master reads far too low.
    """
    plugin_path = _fo76_esm()

    baseline = _private_gb()
    handle = nr.plugin_handle_load_index(str(plugin_path), game="fo76")
    try:
        nr.plugin_handle_call(handle, "export_record_text", 0x0090B305, "json")
        committed = _private_gb() - baseline
        assert committed < 0.50, (
            f"a direct FormID read committed {committed:.2f} GB; expected < 0.50. "
            "Above this, an index is being built for a lookup that needs one record."
        )
    finally:
        nr.plugin_handle_close(handle)


@pytest.mark.integration
@pytest.mark.isolated
def test_full_index_build_stays_below_both_previous_loads() -> None:
    """Ceiling: a whole-plugin query still beats loading the tree.

    Measured on SeventySix.esm in a fresh process: 7.89 GB when the index-only
    handle parsed the whole tree to build CoreSection and then dropped it, 5.98 GB
    for a plain full load, 2.54 GB here, almost all of it CoreSection.

    A full CoreSection over 5,635,950 records costs ~1.8 GB: a RecordIndexEntry
    each in by_form_key, the same FormKeys again in by_signature_form_keys, and a
    Vec per object id in form_ids_by_object_id. Going lower needs those as separate
    lazily-populated sections so a signature query skips the object-id map.

    Must run in a fresh process; see
    test_direct_form_id_read_costs_far_less_than_a_full_load.
    """
    plugin_path = _fo76_esm()

    baseline = _private_gb()
    handle = nr.plugin_handle_load_index(str(plugin_path), game="fo76")
    try:
        form_ids = list(nr.plugin_handle_call(handle, "record_form_ids", ["WEAP"]))
        assert len(form_ids) > 1000
        committed = _private_gb() - baseline
        assert committed < 3.0, f"index build committed {committed:.2f} GB; expected < 3.0"
    finally:
        nr.plugin_handle_close(handle)


@pytest.mark.integration
@pytest.mark.isolated
def test_reading_one_record_does_not_decode_every_string_table() -> None:
    """Opening a localized master must not decode its whole string corpus.

    SeventySix.esm ships 207 MB of loose tables across 13 languages, plus a
    sibling archive holding another 209 MB that backfills ids the loose extract
    lacks - here 50,765 of them, so it cannot be skipped. Decoding all of it at
    open costs about 1.00 GB, even to read a single record.

    The record must still name itself in every language it has: resolving one
    language would also cut the measurement, and the lazy-vs-full comparison
    would not catch it because both handles would lose the same languages.

    Must run in a fresh process; see
    test_direct_form_id_read_costs_far_less_than_a_full_load.
    """
    plugin_path = _fo76_esm()

    baseline = _private_gb()
    handle = nr.plugin_handle_load_index(str(plugin_path), game="fo76")
    try:
        text = nr.plugin_handle_call(handle, "export_record_text", 0x0090B305, "json")
        # The record must still resolve its localized name in every language.
        assert "Unarmed Protectron" in text
        assert "Unbewaffneter Protektron" in text
        committed = _private_gb() - baseline
        assert committed < 0.35, (
            f"reading one record committed {committed:.2f} GB of string tables; "
            "expected < 0.35"
        )
    finally:
        nr.plugin_handle_close(handle)
