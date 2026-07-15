"""Rust-native auto-merge contracts for patch authoring."""

from __future__ import annotations

import json
import struct
from collections.abc import Iterable

import creation_lib.esp.native_runtime as native_runtime


def _sub(signature: str, data: bytes) -> dict[str, str]:
    return {"signature": signature, "data_hex": data.hex().upper()}


def _record_payload(signature: str, form_id: int, subrecords: Iterable[dict[str, str]]) -> dict[str, object]:
    return {
        "signature": signature,
        "form_id": f"{form_id:06X}",
        "subrecords": list(subrecords),
    }


def _plugin_handle(
    plugin_name: str,
    masters: list[str],
    signature: str,
    form_id: int,
    subrecords: Iterable[dict[str, str]],
) -> int:
    payload = {
        "plugin": plugin_name,
        "game": "fo4",
        "header": {
            "version": 1.0,
            "masters": masters,
            "master_sizes": [0 for _ in masters],
            "next_object_id": "000800",
        },
        "items": [
            {
                "type": "group",
                "label_text": signature,
                "group_type": 0,
                "children": [_record_payload(signature, form_id, subrecords)],
            }
        ],
    }
    return int(native_runtime.plugin_handle_import_text(json.dumps(payload), "json", "fo4"))


def _empty_patch_handle(masters: list[str]) -> int:
    payload = {
        "plugin": "Patch.esp",
        "game": "fo4",
        "header": {
            "version": 1.0,
            "masters": masters,
            "master_sizes": [0 for _ in masters],
            "next_object_id": "000800",
        },
        "items": [],
    }
    return int(native_runtime.plugin_handle_import_text(json.dumps(payload), "json", "fo4"))


def _merge(
    patch: int,
    signature: str,
    chain: list[tuple[int, str, int, int]],
) -> dict[str, object]:
    assert native_runtime.plugin_handle_merge_conflict_to_patch(patch, signature, chain)
    exported = native_runtime.plugin_handle_call(patch, "export_plugin_text", "lossless", "json")
    payload = json.loads(exported)
    group = payload["items"][0]
    return group["children"][0]


def _subrecords(record: dict[str, object], signature: str) -> list[bytes]:
    out: list[bytes] = []
    for subrecord in record["subrecords"]:
        if subrecord["signature"] == signature:
            out.append(bytes.fromhex(subrecord.get("data_hex", "")))
    return out


def _lvlo(level: int, ref_form_id: int, count: int) -> bytes:
    return (
        struct.pack("<H", level)
        + b"\x00\x00"
        + struct.pack("<I", ref_form_id)
        + struct.pack("<H", count)
        + b"\x00\x00"
    )


def _edid(value: str) -> dict[str, str]:
    return _sub("EDID", value.encode("ascii") + b"\x00")


def test_native_merge_lvli_unions_disjoint_entries() -> None:
    base_entry = _lvlo(1, 0x00ABCDEF, 1)
    a_entry = _lvlo(2, 0x010000AA, 1)
    b_entry = _lvlo(3, 0x010000BB, 2)

    base = _plugin_handle(
        "Base.esm",
        [],
        "LVLI",
        0x0000A1,
        [_edid("TestList"), _sub("LVLD", b"\x00"), _sub("LVLF", b"\x00"), _sub("LLCT", b"\x01"), _sub("LVLO", base_entry)],
    )
    mod_a = _plugin_handle(
        "ModA.esp",
        ["Base.esm"],
        "LVLI",
        0x0000A1,
        [_edid("TestList"), _sub("LVLD", b"\x00"), _sub("LVLF", b"\x00"), _sub("LLCT", b"\x02"), _sub("LVLO", base_entry), _sub("LVLO", a_entry)],
    )
    mod_b = _plugin_handle(
        "ModB.esp",
        ["Base.esm"],
        "LVLI",
        0x0000A1,
        [_edid("TestList"), _sub("LVLD", b"\x00"), _sub("LVLF", b"\x00"), _sub("LLCT", b"\x02"), _sub("LVLO", base_entry), _sub("LVLO", b_entry)],
    )
    patch = _empty_patch_handle(["Base.esm", "ModA.esp", "ModB.esp"])

    merged = _merge(
        patch,
        "LVLI",
        [(base, "Base.esm", 0, 0x0000A1), (mod_a, "ModA.esp", 1, 0x0000A1), (mod_b, "ModB.esp", 2, 0x0000A1)],
    )

    lvlos = _subrecords(merged, "LVLO")
    refs = sorted(struct.unpack_from("<I", entry, 4)[0] for entry in lvlos)
    assert refs == [0x00ABCDEF, 0x010000AA, 0x020000BB]
    assert _subrecords(merged, "LLCT")[0] == b"\x03"


def test_native_merge_flst_unions_lnam_form_ids() -> None:
    base = _plugin_handle(
        "Base.esm",
        [],
        "FLST",
        0x000200,
        [_edid("List"), _sub("LNAM", struct.pack("<I", 0x00111111))],
    )
    mod_a = _plugin_handle(
        "ModA.esp",
        ["Base.esm"],
        "FLST",
        0x000200,
        [_edid("List"), _sub("LNAM", struct.pack("<I", 0x00111111)), _sub("LNAM", struct.pack("<I", 0x01000022))],
    )
    mod_b = _plugin_handle(
        "ModB.esp",
        ["Base.esm"],
        "FLST",
        0x000200,
        [_edid("List"), _sub("LNAM", struct.pack("<I", 0x00111111)), _sub("LNAM", struct.pack("<I", 0x01000033))],
    )
    patch = _empty_patch_handle(["Base.esm", "ModA.esp", "ModB.esp"])

    merged = _merge(
        patch,
        "FLST",
        [(base, "Base.esm", 0, 0x000200), (mod_a, "ModA.esp", 1, 0x000200), (mod_b, "ModB.esp", 2, 0x000200)],
    )

    fids = sorted(struct.unpack_from("<I", data)[0] for data in _subrecords(merged, "LNAM"))
    assert fids == [0x00111111, 0x01000022, 0x02000033]


def test_native_merge_musc_unions_track_form_ids() -> None:
    def musc(tracks: list[int]) -> list[dict[str, str]]:
        return [
            _edid("Music"),
            _sub("FNAM", b"\x00\x00\x00\x00"),
            _sub("PNAM", b"\x00\x00\x00\x00"),
            _sub("WNAM", b"\x00\x00\x80\x3f"),
            _sub("TNAM", b"".join(struct.pack("<I", track) for track in tracks)),
        ]

    base = _plugin_handle("Base.esm", [], "MUSC", 0x000300, musc([0x00AAAAAA]))
    mod_a = _plugin_handle("ModA.esp", ["Base.esm"], "MUSC", 0x000300, musc([0x00AAAAAA, 0x010000FF]))
    mod_b = _plugin_handle("ModB.esp", ["Base.esm"], "MUSC", 0x000300, musc([0x00AAAAAA, 0x010000EE]))
    patch = _empty_patch_handle(["Base.esm", "ModA.esp", "ModB.esp"])

    merged = _merge(
        patch,
        "MUSC",
        [(base, "Base.esm", 0, 0x000300), (mod_a, "ModA.esp", 1, 0x000300), (mod_b, "ModB.esp", 2, 0x000300)],
    )

    tnam = _subrecords(merged, "TNAM")[0]
    fids = sorted(struct.unpack_from("<I", tnam, offset)[0] for offset in range(0, len(tnam), 4))
    assert fids == [0x00AAAAAA, 0x010000FF, 0x020000EE]


def test_native_merge_kwda_keeps_winner_subrecords_and_unions_keywords() -> None:
    def weap(keywords: list[int], damage: int) -> list[dict[str, str]]:
        return [
            _edid("Gun"),
            _sub("DAMG", struct.pack("<I", damage)),
            _sub("KSIZ", struct.pack("<I", len(keywords))),
            _sub("KWDA", b"".join(struct.pack("<I", keyword) for keyword in keywords)),
        ]

    base = _plugin_handle("Base.esm", [], "WEAP", 0x000400, weap([0x00CAFE01], 10))
    mod_a = _plugin_handle("ModA.esp", ["Base.esm"], "WEAP", 0x000400, weap([0x00CAFE01, 0x010000A0], 20))
    mod_b = _plugin_handle("ModB.esp", ["Base.esm"], "WEAP", 0x000400, weap([0x00CAFE01, 0x010000B0], 20))
    patch = _empty_patch_handle(["Base.esm", "ModA.esp", "ModB.esp"])

    merged = _merge(
        patch,
        "WEAP",
        [(base, "Base.esm", 0, 0x000400), (mod_a, "ModA.esp", 1, 0x000400), (mod_b, "ModB.esp", 2, 0x000400)],
    )

    assert struct.unpack_from("<I", _subrecords(merged, "DAMG")[0])[0] == 20
    assert struct.unpack_from("<I", _subrecords(merged, "KSIZ")[0])[0] == 3
    kwda = _subrecords(merged, "KWDA")[0]
    fids = sorted(struct.unpack_from("<I", kwda, offset)[0] for offset in range(0, len(kwda), 4))
    assert fids == [0x00CAFE01, 0x010000A0, 0x020000B0]


def test_native_merge_unknown_master_preserves_raw_form_id() -> None:
    base = _plugin_handle(
        "ModX.esp",
        ["Base.esm"],
        "FLST",
        0x000200,
        [_edid("List"), _sub("LNAM", struct.pack("<I", 0x00ABCDEF))],
    )
    patch = _empty_patch_handle(["OnlyOther.esm"])

    merged = _merge(patch, "FLST", [(base, "ModX.esp", 0, 0x000200)])

    assert struct.unpack_from("<I", _subrecords(merged, "LNAM")[0])[0] == 0x00ABCDEF
