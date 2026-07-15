from __future__ import annotations

from types import SimpleNamespace

from creation_lib.esp.editor import runtime_hazards
from creation_lib.esp.editor.runtime_hazards import (
    RuntimeHazardReport,
    scan_runtime_hazard_records,
    scan_runtime_hazards,
)


def _subrecord(signature: str, data: bytes = b""):
    return SimpleNamespace(signature=signature, data=data)


def _record(
    form_id: int, *subrecords, form_version: int = 131, signature: str | None = None
):
    return SimpleNamespace(
        form_id=form_id,
        form_version=form_version,
        signature=signature,
        subrecords=list(subrecords),
    )


_SKYRIM_DEBR_MODT = bytes.fromhex(
    "600FF3856464730038973CEA71D535C26464730038973CEAC10BFC6564647300"
    "38973CEA2E7CB2C964647300262C333BE5FC6B216464730038973CEA06E758A1"
    "64647300582C5533"
)
_FNV_DEBR_MODT = bytes.fromhex(
    "F0E10C68FBF4023770610C6800F5023765720D74387A3871EEDF0E684AD2051D"
    "6E5F0E684FD2051D65720D74387A3871F3DF0E684AD2051D735F0E684FD2051D"
    "65720D74387A3871"
)
_FO4_STAT_MODT = bytes.fromhex(
    "0400000013000000000000000F0000000100000075DB5AEF6464730038973CEA"
    "B3AEEF3B646473007A7C3A5ADAE0E40B646473000BD80002038CE26864647300"
    "0BD800020717F56F6464730038973CEA2D0D94F664647300582C55331D653788"
    "646473000BD80002B92277AE646473001CDB88C54D1A1046646473001CDB88C5"
    "528FF9866464730038973CEA29F70F0064647300582C5533AD473ADB64647300"
    "7A7C3A5A791608CF646473000786F88DF6E39FC764647300582C5533F4441C85"
    "64647300582C553332A999016464730038973CEA7F44DDF864647300582C5533"
    "62F091FC6464730038973CEAE8C1B2DE6464730038973CEA7FFC0CAC6267736D"
    "C23D6406"
)
_FNV_PROJ_NAM2 = bytes.fromhex(
    "B1B0106696E60762313010669BE6076273741074B3E1C96DB2B011662D9C07A13"
    "2301166329C07A173651E74E527EFD8"
)


def _valid_fo4_nvmi(*, island: bool = False) -> bytes:
    data = bytearray(24)
    data.extend((0).to_bytes(4, "little"))  # edge links
    data.extend((0).to_bytes(4, "little"))  # preferred edge links
    data.extend((0).to_bytes(4, "little"))  # door links
    data.append(int(island))
    if island:
        data.extend(b"\0" * 24)  # bounds
        data.extend((0).to_bytes(4, "little"))  # triangles
        data.extend((0).to_bytes(4, "little"))  # vertices
    data.extend(b"\0" * 12)  # CRC, parent world, parent cell/coordinates
    return bytes(data)


def _valid_fo4_wthr_subrecords(
    *,
    cloud_rows: int = 2,
    omit: frozenset[str] = frozenset(),
    size_overrides: dict[str, int] | None = None,
):
    sizes = {
        "LNAM": 4,
        "MNAM": 4,
        "NNAM": 4,
        "RNAM": 32,
        "QNAM": 32,
        "PNAM": 32 * cloud_rows,
        "JNAM": 32 * cloud_rows,
        "NAM0": 608,
        "NAM4": 4 * cloud_rows,
        "FNAM": 72,
        "DATA": 20,
        "NAM1": 4,
        "IMSP": 32,
        "UNAM": 24,
        "VNAM": 4,
        "WNAM": 4,
    }
    sizes.update(size_overrides or {})
    subrecords = [
        _subrecord(signature, b"\0" * size)
        for signature, size in sizes.items()
        if signature not in omit
    ]
    if "DALC" not in omit:
        subrecords.extend(_subrecord("DALC", b"\0" * 32) for _ in range(8))
    return tuple(subrecords)


def test_flags_qust_event_alias_fill_after_alias_table_anchor():
    record = _record(
        0x0710E201,
        _subrecord("EDID", b"TW002\0"),
        _subrecord("ANAM", (7).to_bytes(4, "little")),
        _subrecord("ALST", (3).to_bytes(4, "little")),
        _subrecord("ALID", b"GuardAlias\0"),
        _subrecord("ALFD", (0x00003152).to_bytes(4, "little")),
    )

    report = scan_runtime_hazard_records(
        {"QUST": [record]},
        plugin_name="SeventySix.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo76-to-fo4-qust-event-alias-fill"
    assert hazard.form_id == 0x0710E201
    assert hazard.path == "QUST.Alias[3].ALFD"
    assert "TW002" in hazard.message


def test_ignores_qust_event_alias_fill_before_alias_table_anchor():
    record = _record(
        0x0710E201,
        _subrecord("EDID", b"TW002\0"),
        _subrecord("ALFD", (0x00003152).to_bytes(4, "little")),
        _subrecord("ANAM", (7).to_bytes(4, "little")),
        _subrecord("ALST", (3).to_bytes(4, "little")),
    )

    report = scan_runtime_hazard_records(
        {"QUST": [record]},
        plugin_name="SeventySix.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_empty_imad_runtime_data():
    record = _record(
        0x076E908E,
        _subrecord("EDID", b"Storm_MQ08_HallucGasImod\0"),
        _subrecord("NAM5", b""),
        _subrecord("NAM6", b"\x01\x02\x03\x04"),
    )

    report = scan_runtime_hazard_records(
        {"IMAD": [record]},
        plugin_name="SeventySix.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-empty-imad-runtime-data"
    assert hazard.form_id == 0x076E908E
    assert hazard.path == "IMAD.NAM5"
    assert "Storm_MQ08_HallucGasImod" in hazard.message


def test_flags_npc_template_self_slot():
    tpta = b"".join(
        raw.to_bytes(4, "little")
        for raw in (0x0703D628, 0x00000000, 0x0003D628, 0x0703D628)
    )
    record = _record(
        0x0703D628,
        _subrecord("EDID", b"RE_TravelJM01_LvlViciousDogNonHostile\0"),
        _subrecord("TPTA", tpta),
    )

    report = scan_runtime_hazard_records(
        {"NPC_": [record]},
        plugin_name="SeventySix.esm",
        game="fo4",
    )

    assert len(report.hazards) == 2
    assert [hazard.path for hazard in report.hazards] == [
        "NPC_.TPTA[0]",
        "NPC_.TPTA[3]",
    ]
    assert {hazard.rule_id for hazard in report.hazards} == {
        "fo76-to-fo4-npc-template-self-slot"
    }


def test_scans_native_dict_records_and_groups():
    tpta = b"".join(
        raw.to_bytes(4, "little") for raw in (0x0703D628, 0x00000000, 0x0703D628)
    )
    record = {
        "kind": "record",
        "form_id": 0x0703D628,
        "subrecords": [
            {"signature": "EDID", "data": b"RE_TravelJM01_LvlViciousDogNonHostile\0"},
            {"signature": "TPTA", "data": tpta},
        ],
    }
    group = {"kind": "group", "children": [record]}

    report = scan_runtime_hazard_records(
        {"NPC_": [group]},
        plugin_name="SeventySix.esm",
        game="fo4",
    )

    assert len(report.hazards) == 2
    assert [hazard.path for hazard in report.hazards] == [
        "NPC_.TPTA[0]",
        "NPC_.TPTA[2]",
    ]


def test_populates_supplied_empty_report():
    record = _record(
        0x076E908E,
        _subrecord("EDID", b"Storm_MQ08_HallucGasImod\0"),
        _subrecord("NAM5", b""),
    )
    report = RuntimeHazardReport(
        plugin_name="SeventySix.esm",
        game="fo4",
        profile="fo76-to-fo4",
    )

    returned = scan_runtime_hazard_records(
        {"IMAD": [record]},
        plugin_name="SeventySix.esm",
        game="fo4",
        report=report,
    )

    assert returned is report
    assert len(report.hazards) == 1


def test_runtime_hazard_profile_is_fo4_only():
    record = _record(
        0x0710E201,
        _subrecord("ANAM", (7).to_bytes(4, "little")),
        _subrecord("ALST", (3).to_bytes(4, "little")),
        _subrecord("ALFE", b""),
    )

    report = scan_runtime_hazard_records(
        {"QUST": [record]},
        plugin_name="SeventySix.esm",
        game="fo76",
    )

    assert report.hazards == []


def test_flags_fo4_proj_missing_required_dnam():
    record = _record(
        0x070BEDF6,
        _subrecord("EDID", b"FlameProjectileANT\0"),
        _subrecord("DATA", b""),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-proj-missing-dnam"
    assert hazard.path == "PROJ.DNAM"
    assert "FlameProjectileANT" in hazard.message


def test_flags_every_wrong_sized_fo4_proj_dnam_with_occurrences():
    record = _record(
        0x070BEDF6,
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 68),
        _subrecord("DNAM", b"\0" * 84),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-proj-dnam-size",
        "fo4-loader-proj-dnam-size",
    ]
    assert [hazard.path for hazard in report.hazards] == [
        "PROJ.DNAM[0]",
        "PROJ.DNAM[1]",
    ]


def test_flags_every_nonempty_fo4_proj_data_but_allows_empty_marker():
    record = _record(
        0x070BEDF6,
        _subrecord("DATA", b""),
        _subrecord("DATA", b"source-layout"),
        _subrecord("DNAM", b"\0" * 93),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-proj-nonempty-data"
    assert hazard.path == "PROJ.DATA[1]"


def test_flags_actual_fnv_proj_source_layout_nam2_at_fo4_v131():
    assert len(_FNV_PROJ_NAM2) == 48
    record = _record(
        0x070BEDF6,
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 93),
        _subrecord("NAM2", _FNV_PROJ_NAM2),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-proj-legacy-nam2-model-info"
    assert hazard.path == "PROJ.NAM2[0]"
    assert "counter_count" in hazard.message


def test_accepts_valid_fo4_proj_shape_and_model_info():
    valid_model_info = (4).to_bytes(4, "little") + b"\0" * 16
    record = _record(
        0x01001004,
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 93),
        _subrecord("NAM2", valid_model_info),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_proj_shape_gate_is_fo4_only():
    record = _record(
        0x000BEDF6,
        _subrecord("DATA", b"source"),
        signature="PROJ",
    )

    report = scan_runtime_hazard_records(
        {"PROJ": [record]},
        plugin_name="FalloutNV.esm",
        game="fnv",
    )

    assert report.hazards == []


def test_session_scan_includes_proj_records(monkeypatch):
    active = SimpleNamespace(
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
        handle=7,
    )
    session = SimpleNamespace(active=active)
    monkeypatch.setattr(
        runtime_hazards,
        "_record_payloads",
        lambda handle: [
            {
                "signature": "PROJ",
                "form_id": 0x070BEDF6,
                "form_version": 131,
                "subrecords": [{"signature": "DATA", "data": b""}],
            }
        ],
    )

    report = scan_runtime_hazards(session)

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-proj-missing-dnam"
    ]


def test_flags_fnv_term_snam_at_fo4_v131_row_stride():
    record = _record(
        0x0717B7A0,
        _subrecord("EDID", b"P04CompanionFireTerminal\0"),
        _subrecord("SNAM", bytes.fromhex("34120000")),
        signature="TERM",
    )

    report = scan_runtime_hazard_records(
        {"TERM": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-term-snam-row-stride"
    assert hazard.path == "TERM.SNAM[0]"
    assert "4-byte SNAM" in hazard.message


def test_accepts_fo4_term_snam_rows_and_ignores_pre_v125_layouts():
    valid = _record(
        0x01001000,
        _subrecord("SNAM", b"\0" * 48),
        signature="TERM",
    )
    legacy_version = _record(
        0x01001001,
        _subrecord("SNAM", bytes.fromhex("34120000")),
        form_version=124,
        signature="TERM",
    )

    report = scan_runtime_hazard_records(
        {"TERM": [valid, legacy_version]},
        plugin_name="LayoutFixtures.esp",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_actual_skyrim_debr_legacy_modt():
    assert len(_SKYRIM_DEBR_MODT) == 72
    record = _record(
        0x070DEDC9,
        _subrecord("EDID", b"IceFormDebris14\0"),
        _subrecord("MODT", _SKYRIM_DEBR_MODT),
        signature="DEBR",
    )

    report = scan_runtime_hazard_records(
        {"DEBR": [record]},
        plugin_name="Skyrim_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    hazard = report.hazards[0]
    assert hazard.rule_id == "fo4-loader-invalid-model-info"
    assert hazard.path == "DEBR.MODT[0]"
    assert "counter_count" in hazard.message
    assert "IceFormDebris14" in hazard.message


def test_flags_actual_fnv_debr_legacy_modt():
    assert len(_FNV_DEBR_MODT) == 72
    record = _record(
        0x070B8FF4,
        _subrecord("EDID", b"RobotGoreBits01\0"),
        _subrecord("MODT", _FNV_DEBR_MODT),
        signature="DEBR",
    )

    report = scan_runtime_hazard_records(
        {"DEBR": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert len(report.hazards) == 1
    assert report.hazards[0].rule_id == "fo4-loader-invalid-model-info"


def test_accepts_actual_fo4_encoded_modt_in_any_record_context():
    record = _record(
        0x00048280,
        _subrecord("MODT", _FO4_STAT_MODT),
        signature="STAT",
    )

    report = scan_runtime_hazard_records(
        {"STAT": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_ignores_pre_v131_fo4_three_counter_model_info_layout():
    record = _record(
        0x001E48E0,
        _subrecord("MODT", bytes.fromhex("03000000000000000000000000000000")),
        form_version=126,
        signature="ACTI",
    )

    report = scan_runtime_hazard_records(
        {"ACTI": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_every_fo4_model_info_subrecord_family():
    model_info_sigs = ("MODT", "MO2T", "MO3T", "MO4T", "MO5T", "DMDT")
    record = _record(
        0x01001003,
        *(_subrecord(signature, _FNV_DEBR_MODT) for signature in model_info_sigs),
        signature="ARMO",
    )

    report = scan_runtime_hazard_records(
        {"ARMO": [record]},
        plugin_name="LayoutFixtures.esp",
        game="fo4",
    )

    assert [hazard.subrecord_sig for hazard in report.hazards] == list(model_info_sigs)
    assert [hazard.path for hazard in report.hazards] == [
        f"ARMO.{signature}[0]" for signature in model_info_sigs
    ]


def test_flags_model_info_counts_that_exceed_payload_without_allocating():
    malformed = (
        (4).to_bytes(4, "little") + (0xFFFFFFFF).to_bytes(4, "little") + b"\0" * 12
    )
    record = _record(
        0x01001002,
        _subrecord("MODT", malformed),
        signature="STAT",
    )

    report = scan_runtime_hazard_records(
        {"STAT": [record]},
        plugin_name="LayoutFixtures.esp",
        game="fo4",
    )

    assert len(report.hazards) == 1
    assert "texture count 4294967295 exceeds" in report.hazards[0].message


def test_flags_legacy_refr_xloc_but_not_unrelated_cell_xcll():
    refr = _record(
        0x07001000,
        _subrecord("XLOC", b"\0" * 12),
        signature="REFR",
    )
    cell = _record(
        0x07001001,
        _subrecord("XCLL", b"\0" * 136),
        signature="CELL",
    )

    report = scan_runtime_hazard_records(
        {"REFR": [refr], "CELL": [cell]},
        plugin_name="LayoutFixtures.esp",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-refr-xloc-size"
    ]
    assert report.hazards[0].path == "REFR.XLOC[0]"
    assert "12-byte XLOC" in report.hazards[0].message


def test_accepts_exact_fo4_refr_xloc():
    record = _record(
        0x01001000,
        _subrecord("XLOC", b"\0" * 16),
        signature="REFR",
    )

    report = scan_runtime_hazard_records(
        {"REFR": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_source_layout_efsh_contract():
    record = _record(
        0x07002000,
        _subrecord("DATA", b"\0" * 400),
        _subrecord("DNAM", b"\0" * 395),
        signature="EFSH",
    )

    report = scan_runtime_hazard_records(
        {"EFSH": [record]},
        plugin_name="Skyrim_Merged.esm",
        game="fo4",
    )

    assert {hazard.rule_id for hazard in report.hazards} == {
        "fo4-loader-efsh-missing-target-subrecord",
        "fo4-loader-efsh-nonempty-data",
        "fo4-loader-efsh-dnam-size",
    }
    assert {hazard.subrecord_sig for hazard in report.hazards} >= {
        "ICON",
        "NAM7",
        "NAM8",
        "DNAM",
    }
    assert all(hazard.severity == "error" for hazard in report.hazards)


def test_accepts_current_and_proven_legacy_fo4_efsh_variants():
    current = _record(
        0x01002000,
        _subrecord("ICON", b"\0"),
        _subrecord("NAM7", b"\0"),
        _subrecord("NAM8", b"\0"),
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 157),
        signature="EFSH",
    )
    legacy = _record(
        0x01002001,
        _subrecord("ICON", b"Effects\\Legacy.dds\0"),
        _subrecord("NAM7", b"\0"),
        _subrecord("NAM8", b"\0"),
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 395),
        form_version=105,
        signature="EFSH",
    )

    report = scan_runtime_hazard_records(
        {"EFSH": [current, legacy]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_invalid_efsh_icon_zstring():
    record = _record(
        0x07002002,
        _subrecord("ICON", b"Effects\\Fill.dds"),
        _subrecord("NAM7", b"\0"),
        _subrecord("NAM8", b"\0"),
        _subrecord("DATA", b""),
        _subrecord("DNAM", b"\0" * 157),
        signature="EFSH",
    )

    report = scan_runtime_hazard_records(
        {"EFSH": [record]},
        plugin_name="Converted.esm",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-efsh-invalid-icon"
    ]


def test_flags_source_layout_wthr_fields_and_dalc_rows():
    record = _record(
        0x07003000,
        _subrecord("DATA", b"\0" * 15),
        _subrecord("PNAM", b"\0" * 16),
        _subrecord("JNAM", b"\0" * 16),
        _subrecord("IMSP", b"\0" * 16),
        _subrecord("FNAM", b"\0" * 32),
        *(_subrecord("DALC", b"\0" * 24) for _ in range(8)),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [record]},
        plugin_name="Skyrim_Merged.esm",
        game="fo4",
    )

    rule_ids = {hazard.rule_id for hazard in report.hazards}
    assert {
        "fo4-loader-wthr-data-size",
        "fo4-loader-wthr-pnam-row-stride",
        "fo4-loader-wthr-jnam-row-stride",
        "fo4-loader-wthr-imsp-size",
        "fo4-loader-wthr-fnam-size",
        "fo4-loader-wthr-dalc-row-size",
    } <= rule_ids
    assert (
        sum(h.rule_id == "fo4-loader-wthr-dalc-row-size" for h in report.hazards) == 8
    )


def test_flags_wthr_cloud_table_mismatch_and_bad_dalc_count():
    record = _record(
        0x07003001,
        *_valid_fo4_wthr_subrecords(
            cloud_rows=1,
            omit=frozenset({"DALC"}),
            size_overrides={"JNAM": 64},
        ),
        *(_subrecord("DALC", b"\0" * 32) for _ in range(7)),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [record]},
        plugin_name="LayoutFixtures.esp",
        game="fo4",
    )

    assert {hazard.rule_id for hazard in report.hazards} == {
        "fo4-loader-wthr-cloud-table-row-count",
        "fo4-loader-wthr-dalc-row-count",
    }


def test_accepts_valid_fo4_wthr_target_shape():
    record = _record(
        0x01003000,
        *_valid_fo4_wthr_subrecords(),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_missing_and_wrong_sized_wthr_nam0():
    missing = _record(
        0x07003002,
        *_valid_fo4_wthr_subrecords(omit=frozenset({"NAM0"})),
        signature="WTHR",
    )
    wrong = _record(
        0x07003003,
        *_valid_fo4_wthr_subrecords(size_overrides={"NAM0": 272}),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [missing, wrong]},
        plugin_name="Converted.esm",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-wthr-missing-target-subrecord",
        "fo4-loader-wthr-nam0-size",
    ]
    assert [hazard.form_id for hazard in report.hazards] == [0x07003002, 0x07003003]


def test_flags_missing_wthr_required_companions():
    missing = frozenset(
        {
            "LNAM",
            "MNAM",
            "NNAM",
            "RNAM",
            "QNAM",
            "NAM4",
            "NAM1",
            "UNAM",
            "VNAM",
            "WNAM",
        }
    )
    record = _record(
        0x07003004,
        *_valid_fo4_wthr_subrecords(omit=missing),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [record]},
        plugin_name="Converted.esm",
        game="fo4",
    )

    assert {hazard.subrecord_sig for hazard in report.hazards} == missing
    assert {hazard.rule_id for hazard in report.hazards} == {
        "fo4-loader-wthr-missing-target-subrecord"
    }


def test_flags_wrong_sized_wthr_required_companions():
    record = _record(
        0x07003005,
        *_valid_fo4_wthr_subrecords(
            size_overrides={
                "LNAM": 3,
                "MNAM": 3,
                "NNAM": 3,
                "RNAM": 0,
                "QNAM": 0,
                "NAM0": 607,
                "NAM4": 3,
                "NAM1": 3,
                "IMSP": 64,
                "UNAM": 23,
                "VNAM": 3,
                "WNAM": 3,
            }
        ),
        signature="WTHR",
    )

    report = scan_runtime_hazard_records(
        {"WTHR": [record]},
        plugin_name="Converted.esm",
        game="fo4",
    )

    assert {hazard.rule_id for hazard in report.hazards} == {
        "fo4-loader-wthr-lnam-size",
        "fo4-loader-wthr-mnam-size",
        "fo4-loader-wthr-nnam-size",
        "fo4-loader-wthr-rnam-empty-rows",
        "fo4-loader-wthr-qnam-empty-rows",
        "fo4-loader-wthr-nam0-size",
        "fo4-loader-wthr-nam4-row-stride",
        "fo4-loader-wthr-nam1-size",
        "fo4-loader-wthr-imsp-size",
        "fo4-loader-wthr-unam-size",
        "fo4-loader-wthr-vnam-size",
        "fo4-loader-wthr-wnam-size",
    }


def test_flags_legacy_navi_nver11_and_source_nvmi_shape():
    malformed_nvmi = bytearray(_valid_fo4_nvmi())
    malformed_nvmi[24:28] = (0xFFFFFFFF).to_bytes(4, "little")
    record = _record(
        0x07004000,
        _subrecord("NVER", (11).to_bytes(4, "little")),
        _subrecord("NVMI", bytes(malformed_nvmi)),
        signature="NAVI",
    )

    report = scan_runtime_hazard_records(
        {"NAVI": [record]},
        plugin_name="FNV_FO3_Merged.esm",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-navi-legacy-nver11",
        "fo4-loader-navi-nvmi-shape",
    ]
    assert "edge links rows exceed payload" in report.hazards[1].message


def test_accepts_fo4_navi_v15_nvmi_shapes_with_and_without_island_data():
    record = _record(
        0x01004000,
        _subrecord("NVER", (15).to_bytes(4, "little")),
        _subrecord("NVMI", _valid_fo4_nvmi()),
        _subrecord("NVMI", _valid_fo4_nvmi(island=True)),
        signature="NAVI",
    )

    report = scan_runtime_hazard_records(
        {"NAVI": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_flags_disallowed_and_misplaced_distant_lod_mnam():
    acti = _record(
        0x07005000,
        _subrecord("MNAM", b"\0" * 1040),
        signature="ACTI",
    )
    mstt = _record(
        0x07005001,
        _subrecord("MNAM", b"\0" * 4),
        signature="MSTT",
    )
    achr = _record(
        0x07005002,
        _subrecord("MNAM", b"\0" * 1040),
        signature="ACHR",
    )

    report = scan_runtime_hazard_records(
        {"ACTI": [acti], "MSTT": [mstt], "ACHR": [achr]},
        plugin_name="Converted.esm",
        game="fo4",
    )

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-mnam-disallowed-record",
        "fo4-loader-mnam-disallowed-record",
        "fo4-loader-mnam-illegal-distant-lod",
    ]
    assert [hazard.path for hazard in report.hazards] == [
        "ACTI.MNAM[0]",
        "MSTT.MNAM[0]",
        "ACHR.MNAM[0]",
    ]


def test_accepts_stat_distant_lod_mnam():
    record = _record(
        0x01005000,
        _subrecord("MNAM", b"\0" * 1040),
        signature="STAT",
    )

    report = scan_runtime_hazard_records(
        {"STAT": [record]},
        plugin_name="Fallout4.esm",
        game="fo4",
    )

    assert report.hazards == []


def test_session_scan_includes_records_with_disallowed_mnam(monkeypatch):
    active = SimpleNamespace(plugin_name="Converted.esm", game="fo4", handle=9)
    session = SimpleNamespace(active=active)
    monkeypatch.setattr(
        runtime_hazards,
        "_record_payloads",
        lambda handle: [
            {
                "signature": "ACTI",
                "form_id": 0x07005000,
                "form_version": 131,
                "subrecords": [{"signature": "MNAM", "data": b"\0" * 1040}],
            }
        ],
    )

    report = scan_runtime_hazards(session)

    assert [hazard.rule_id for hazard in report.hazards] == [
        "fo4-loader-mnam-disallowed-record"
    ]
