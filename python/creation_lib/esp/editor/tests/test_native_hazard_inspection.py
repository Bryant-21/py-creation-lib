from __future__ import annotations

import json
import struct
import zlib
from types import SimpleNamespace

import pytest

from creation_lib.esp import native_runtime as nr
from creation_lib.esp.editor import runtime_hazards as hazards


def _binary_record(signature, form_id, subrecords, *, compressed=False, version=131):
    payload = b"".join(
        sig.encode("ascii") + struct.pack("<H", len(data)) + data
        for sig, data in subrecords
    )
    flags = 0x40000 if compressed else 0
    if compressed:
        payload = struct.pack("<I", len(payload)) + zlib.compress(payload)
    return (
        struct.pack(
            "<4sIIIIHH",
            signature.encode("ascii"),
            len(payload),
            flags,
            form_id,
            0,
            version,
            0,
        )
        + payload
    )


def _group(label, children, group_type=0):
    payload = b"".join(children)
    return (
        struct.pack(
            "<4sI4si8s", b"GRUP", len(payload) + 24, label, group_type, b"\0" * 8
        )
        + payload
    )


@pytest.fixture
def inspection_plugin(tmp_path):
    records = [
        _binary_record(
            "MISC",
            0x02000800,
            [("EDID", b"B21_BadMisc\0"), ("DATA", b"\1" * 4)],
            compressed=True,
        ),
        _binary_record("MISC", 0x801, [("DATA", b"\0" * 8)]),
        _binary_record("MISC", 0x800, [("DATA", b"\0"), ("DATA", b"\0" * 12)]),
        _binary_record("LGTM", 0x802, [("EDID", b"B21_MissingDalc\0")]),
        _binary_record("IMAD", 0x803, [("DNAM", b"\0" * 252), ("TNAM", b"\0")]),
        _binary_record("PROJ", 0x804, [("DATA", b"")]),
        _binary_record("TERM", 0x805, [("SNAM", b"\0" * 4)], version=124),
        _binary_record("TERM", 0x806, [("SNAM", b"\0" * 4)], version=131),
        _binary_record("ACTI", 0x807, [("MNAM", b"\0" * 1040)], compressed=True),
        _binary_record("STAT", 0x808, [("MODT", b"\0" * 16)]),
        _binary_record(
            "QUST",
            0x809,
            [("ANAM", b"\0" * 4), ("ALST", b"\1\0\0\0"), ("ALFD", b"\0" * 4)],
        ),
        _binary_record("REFR", 0x810, [("XLOC", b"\0" * 12)]),
        _binary_record("EFSH", 0x811, [("DNAM", b"\0" * 395)]),
        _binary_record("WTHR", 0x812, [("MNAM", b"\0" * 4)]),
        _binary_record(
            "NPC_",
            0x02000813,
            [("EDID", b"B21_Npc\0"), ("TPTA", (0x813).to_bytes(4, "little"))],
        ),
        _binary_record("NAVI", 0x814, [("NVER", (11).to_bytes(4, "little"))]),
        _binary_record("NAVM", 0x815, [("NVNM", b"\0" * 4)]),
        _binary_record("KYWD", 0x816, [("EDID", b"B21_Unrelated\0")]),
        _binary_record("MISC", 0xFF000817, [("DATA", b"bad")]),
        _binary_record("MISC", 0x09000818, [("DATA", b"bad")]),
    ]
    header = _binary_record(
        "TES4",
        0,
        [
            ("HEDR", struct.pack("<fII", 1.0, len(records), 0x900)),
            ("MAST", b"B21_Base0.esm\0"),
            ("DATA", b"\0" * 8),
            ("MAST", b"B21_Base1.esm\0"),
            ("DATA", b"\0" * 8),
        ],
    )
    data = (
        header
        + _group(b"MISC", records[:3])
        + _group(b"CELL", [_group(struct.pack("<I", 0x900), records[3:], group_type=6)])
    )
    path = tmp_path / "B21_HazardInspection.esp"
    path.write_bytes(data)
    return path


def _legacy_records(payload):
    if isinstance(payload, dict):
        if "signature" in payload and "subrecords" in payload:
            yield payload
        for value in payload.values():
            yield from _legacy_records(value)
    elif isinstance(payload, list):
        for item in payload:
            yield from _legacy_records(item)


@pytest.mark.parametrize("profile", hazards.SUPPORTED_PROFILES)
@pytest.mark.parametrize("mode", ["eager", "compressed_deferred", "index"])
def test_native_inspection_matches_lossless_reports(inspection_plugin, profile, mode):
    before = inspection_plugin.read_bytes()
    baseline = nr.plugin_handle_load(str(inspection_plugin), game="fo4")
    try:
        payload = json.loads(
            nr.plugin_handle_call(baseline, "export_plugin_text", "lossless", "json")
        )
        by_sig = {}
        for record in _legacy_records(payload):
            by_sig.setdefault(record["signature"], []).append(record)
        expected = hazards.scan_runtime_hazard_records(
            by_sig,
            plugin_name=inspection_plugin.name,
            game="fo4",
            profile=profile,
        )
    finally:
        nr.plugin_handle_close(baseline)
    handle = (
        nr.plugin_handle_load_index(str(inspection_plugin), game="fo4")
        if mode == "index"
        else nr.plugin_handle_load(
            str(inspection_plugin), game="fo4", eager_compressed=mode == "eager"
        )
    )
    try:
        active = SimpleNamespace(
            handle=handle, game="fo4", plugin_name=inspection_plugin.name
        )
        actual = hazards.scan_runtime_hazards(
            SimpleNamespace(active=active), profile=profile
        )
        assert actual.to_dict() == expected.to_dict()
        assert actual.hazards
        rows = hazards._record_payloads(handle, profile)
        assert not any(record["signature"] == "KYWD" for record in rows)
        assert sum(record["form_id"] == 0x800 for record in rows) == 2
        assert any(record["raw_form_id"] == 0x02000800 for record in rows)
        assert any(record["form_id"] == 0x817 for record in rows)
        assert any(record["form_id"] == 0x09000818 for record in rows)
        if profile == hazards.FO4_TARGET_SHAPE_PROFILE:
            assert {record["signature"] for record in rows} == {"IMAD", "LGTM", "MISC"}
        assert (
            hazards.scan_runtime_hazards_path(
                inspection_plugin,
                game="fo4",
                profile=profile,
            ).to_dict()
            == expected.to_dict()
        )
    finally:
        nr.plugin_handle_close(handle)
    assert inspection_plugin.read_bytes() == before


def test_inspection_reads_current_subrecords(inspection_plugin):
    handle = nr.plugin_handle_load(str(inspection_plugin), game="fo4")
    try:
        assert nr.plugin_handle_set_record_subrecords(
            handle, 0x801, [("DATA", b"bad", None)]
        )
        rows = nr.plugin_handle_inspection_records(handle, ["MISC"], [])
        changed = next(row for row in rows if row["form_id"] == 0x801)
        assert changed["subrecords"] == [{"signature": "DATA", "data": b"bad"}]
    finally:
        nr.plugin_handle_close(handle)


def test_failed_native_inspection_does_not_report_success():
    active = SimpleNamespace(handle=2**63, game="fo4", plugin_name="B21_Missing.esp")
    with pytest.raises(KeyError, match="unknown plugin handle"):
        hazards.scan_runtime_hazards(SimpleNamespace(active=active))


def test_index_inspection_rejects_truncated_group(inspection_plugin):
    inspection_plugin.write_bytes(inspection_plugin.read_bytes()[:-1])
    handle = nr.plugin_handle_load_index(str(inspection_plugin), game="fo4")
    try:
        with pytest.raises(ValueError, match="truncated plugin"):
            nr.plugin_handle_inspection_records(handle, ["MISC"], [])
    finally:
        nr.plugin_handle_close(handle)
