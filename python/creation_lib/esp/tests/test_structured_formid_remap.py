"""Structured CELL.XCRI, CTDA and PERK EPFD FormIDs follow master names across plugins.

Mirrors the Tales/SeventySix bug: a CELL copied out of a plugin whose master
list differs from the target's must have every XCRI reference (and every CTDA
parameter FormID) re-indexed by master name when its authoring record is built
into the target. That only happens when the payload decodes to FormKeys; raw
bytes keep the source plugin's master indices.
"""
from __future__ import annotations

import struct

from creation_lib.esp import native_runtime
from creation_lib.esp.plugin import Plugin

SOURCE_MASTERS = ["Fallout4.esm", "Other.esm"]  # Source.esm's own index is 02
TARGET_MASTERS = ["Other.esm", "Source.esm", "Fallout4.esm"]

SOURCE_OWN = 0x0200_0801
FALLOUT4_REF = 0x0000_0ABC
OTHER_REF = 0x0100_0123

MESH_KEYS = [0, 0x16000F31]


def _xcri(pairs: list[tuple[int, int]]) -> bytes:
    words = [len(MESH_KEYS), 2 * len(pairs), *MESH_KEYS]
    for reference, mesh in pairs:
        words += [reference, mesh]
    return struct.pack(f"<{len(words)}I", *words)


def _ctda(function: int, parameter: int) -> bytes:
    return struct.pack("<B3xfHxxIIIIi", 0, 1.0, function, parameter, 0, 0, 0, -1)


def _subrecords(plugin: Plugin, form_id: int, signature: str) -> list[bytes]:
    rows = native_runtime.plugin_handle_record_subrecords(plugin._rust_handle, form_id) or []
    return [data for sig, data, _ in rows if sig == signature]


def _field(record: dict, key: str) -> list:
    return [entry[key] for entry in record["fields"] if key in entry]


def _built_from_raw(signature: str, fields: list[dict]) -> tuple[Plugin, int]:
    source = Plugin.new("Source.esm", game="fo4", masters=SOURCE_MASTERS)
    form_key = source.upsert_authoring_record({"signature": signature, "eid": f"Source{signature}", "fields": fields})
    assert form_key == "Source.esm:000800"
    return source, (len(SOURCE_MASTERS) << 24) | 0x800


def _built_into_target(record: dict) -> tuple[Plugin, int]:
    target = Plugin.new("Target.esp", game="fo4", masters=TARGET_MASTERS)
    plugin_name, object_id = target.upsert_authoring_record(record).split(":")
    masters = target.header.masters
    index = masters.index(plugin_name) if plugin_name in masters else len(masters)
    return target, (index << 24) | int(object_id, 16)


def _authoring(source: Plugin, form_id: int, signature: str) -> dict:
    record = source.read_authoring_record(form_id)
    assert record is not None
    record.pop("raw_payload_hex", None)
    record["signature"] = signature
    return record


XCRI_SOURCE = _xcri([(SOURCE_OWN, MESH_KEYS[1]), (FALLOUT4_REF, 0), (OTHER_REF, MESH_KEYS[1])])
XCRI_TARGET = _xcri([(0x0100_0801, MESH_KEYS[1]), (0x0200_0ABC, 0), (0x0000_0123, MESH_KEYS[1])])


def test_raw_hex_xcri_still_builds_and_decodes_structured() -> None:
    source, form_id = _built_from_raw(
        "CELL",
        [{"Flags": ["IsInteriorCell"]}, {"CombinedReferenceIndex": {"raw_hex": XCRI_SOURCE.hex().upper()}}],
    )
    assert _subrecords(source, form_id, "XCRI") == [XCRI_SOURCE]

    [xcri] = _field(_authoring(source, form_id, "CELL"), "CombinedReferenceIndex")
    assert "raw_hex" not in xcri
    assert xcri["Meshes"] == MESH_KEYS
    assert [row["Reference"]["reference"] for row in xcri["References"]] == [
        {"plugin": "Source.esm", "object_id": "000801"},
        {"plugin": "Fallout4.esm", "object_id": "000ABC"},
        {"plugin": "Other.esm", "object_id": "000123"},
    ]


def test_structured_xcri_round_trips_byte_exact_with_same_masters() -> None:
    source, form_id = _built_from_raw(
        "CELL",
        [{"Flags": ["IsInteriorCell"]}, {"CombinedReferenceIndex": {"raw_hex": XCRI_SOURCE.hex().upper()}}],
    )
    rebuilt = Plugin.new("Source.esm", game="fo4", masters=SOURCE_MASTERS)
    rebuilt.upsert_authoring_record(_authoring(source, form_id, "CELL"))
    assert _subrecords(rebuilt, form_id, "XCRI") == [XCRI_SOURCE]


def test_structured_xcri_remaps_references_into_a_different_master_order() -> None:
    source, form_id = _built_from_raw(
        "CELL",
        [{"Flags": ["IsInteriorCell"]}, {"CombinedReferenceIndex": {"raw_hex": XCRI_SOURCE.hex().upper()}}],
    )
    target, target_form_id = _built_into_target(_authoring(source, form_id, "CELL"))
    assert _subrecords(target, target_form_id, "XCRI") == [XCRI_TARGET]


def test_structured_ctda_remaps_parameter_form_ids_into_a_different_master_order() -> None:
    get_is_id, has_keyword = 72, 560
    source, form_id = _built_from_raw(
        "COBJ",
        [
            {"CTDA": {"raw_hex": _ctda(get_is_id, SOURCE_OWN).hex().upper()}},
            {"CTDA": {"raw_hex": _ctda(has_keyword, FALLOUT4_REF).hex().upper()}},
            {"CTDA": {"raw_hex": _ctda(get_is_id, OTHER_REF).hex().upper()}},
        ],
    )
    authoring = _authoring(source, form_id, "COBJ")
    assert all("raw_hex" not in ctda for ctda in _field(authoring, "CTDA"))

    target, target_form_id = _built_into_target(authoring)
    assert _subrecords(target, target_form_id, "CTDA") == [
        _ctda(get_is_id, 0x0100_0801),
        _ctda(has_keyword, 0x0200_0ABC),
        _ctda(get_is_id, 0x0000_0123),
    ]


def test_structured_epfd_spell_remaps_into_a_different_master_order() -> None:
    spell_epft = 5
    source, form_id = _built_from_raw(
        "PERK",
        [
            {"PRKE": {"Type": "EntryPoint"}},
            {"EffectData": {"variant": "entry_point", "value": {"EntryPointEntryPoint": 0, "EntryPointFunction": "SetValue"}}},
            {"Type": spell_epft},
            {"EPFD": {"raw_hex": struct.pack("<I", SOURCE_OWN).hex().upper()}},
            {"EndMarker": True},
        ],
    )
    authoring = _authoring(source, form_id, "PERK")
    [epfd] = _field(authoring, "EPFD")
    assert epfd == {"variant": "spell", "value": {"reference": {"plugin": "Source.esm", "object_id": "000801"}}}

    target, target_form_id = _built_into_target(authoring)
    assert _subrecords(target, target_form_id, "EPFD") == [struct.pack("<I", 0x0100_0801)]


def test_set_masters_reorder_remaps_raw_xcri_references() -> None:
    source, form_id = _built_from_raw(
        "CELL",
        [{"Flags": ["IsInteriorCell"]}, {"CombinedReferenceIndex": {"raw_hex": XCRI_SOURCE.hex().upper()}}],
    )
    source.set_masters([("Other.esm", 0), ("Fallout4.esm", 0)])

    assert _subrecords(source, form_id, "XCRI") == [
        _xcri([(SOURCE_OWN, MESH_KEYS[1]), (0x0100_0ABC, 0), (0x0000_0123, MESH_KEYS[1])])
    ]
