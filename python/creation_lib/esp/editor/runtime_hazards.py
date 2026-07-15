"""Runtime-hazard checks for FO4 loader crash patterns.

These checks are intentionally narrower than xEdit-style validation. They flag
record shapes that can parse cleanly but are known to crash or wedge FO4 while
loading converted plugins.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Iterable, Mapping

from creation_lib.esp.native_runtime import plugin_handle_call


FO76_TO_FO4_PROFILE = "fo76-to-fo4"
SUPPORTED_PROFILES = (FO76_TO_FO4_PROFILE,)

_FO76_TO_FO4_RECORD_SIGS = ("IMAD", "NPC_", "PROJ", "QUST", "TERM")
_FO4_LAYOUT_RECORD_SIGS = frozenset({"EFSH", "NAVI", "REFR", "WTHR"})
_QUST_EVENT_ALIAS_FILL_SIGS = {"ALFE", "ALFD"}
_IMAD_EMPTY_UNSAFE_SIGS = {"NAM5", "NAM6"}
_FO4_TERM_MARKER_ROW_VERSION = 125
_FO4_TERM_MARKER_ROW_SIZE = 24
_FO4_MODEL_INFO_COUNTER4_VERSION = 131
_FO4_MODEL_INFO_HEADER_SIZE = 20
_FO4_MODEL_INFO_ENTRY_SIZE = 12
_FO4_MODEL_INFO_SIGS = frozenset({"MODT", "MO2T", "MO3T", "MO4T", "MO5T", "DMDT"})
_FO4_MNAM_RECORD_SIGS = frozenset(
    {
        "ACHR",
        "CAMS",
        "CMPO",
        "COLL",
        "EXPL",
        "FACT",
        "FURN",
        "HAZD",
        "LCTN",
        "LTEX",
        "MATT",
        "MOVT",
        "NAVM",
        "OMOD",
        "PARW",
        "PBAR",
        "PBEA",
        "PCON",
        "PFLA",
        "PGRE",
        "PHZD",
        "PMIS",
        "RACE",
        "REFR",
        "SMQN",
        "SNCT",
        "SOPM",
        "SPGD",
        "STAT",
        "TERM",
        "TXST",
        "WRLD",
        "WTHR",
    }
)
_FO4_EFSH_CURRENT_FORM_VERSION = 106
_FO4_EFSH_DNAM_SIZE = 157
_FO4_EFSH_LEGACY_DNAM_SIZE = 395
_FO4_WTHR_ROW_SIZE = 32
_FO4_WTHR_MAX_CLOUD_ROWS = 32
_FO4_WTHR_DALC_ROWS = 8
_FO4_NAVI_VERSION = 15
_FO4_NAVI_LEGACY_FALLOUT_VERSION = 11
_FO4_DISTANT_LOD_MNAM_SIZE = 1040


@dataclass(frozen=True)
class RuntimeHazard:
    rule_id: str
    message: str
    plugin_name: str
    form_id: int | None = None
    record_sig: str | None = None
    subrecord_sig: str | None = None
    path: str | None = None
    severity: str = "error"
    category: str = "runtime_hazard"

    def to_dict(self) -> dict[str, object]:
        return {
            "severity": self.severity,
            "category": self.category,
            "rule_id": self.rule_id,
            "plugin": self.plugin_name,
            "form_id": self.form_id,
            "form_id_hex": f"{self.form_id:08X}" if self.form_id is not None else None,
            "record_sig": self.record_sig,
            "subrecord_sig": self.subrecord_sig,
            "path": self.path,
            "message": self.message,
        }


@dataclass
class RuntimeHazardReport:
    plugin_name: str
    game: str
    profile: str
    hazards: list[RuntimeHazard] = field(default_factory=list)

    def add(self, hazard: RuntimeHazard) -> None:
        self.hazards.append(hazard)

    def to_dict(self, *, max_hazards: int | None = None) -> dict[str, object]:
        hazards = self.hazards
        omitted = 0
        if max_hazards is not None and len(hazards) > max_hazards:
            omitted = len(hazards) - max_hazards
            hazards = hazards[:max_hazards]
        return {
            "plugin": self.plugin_name,
            "game": self.game,
            "profile": self.profile,
            "hazard_count": len(self.hazards),
            "displayed_hazard_count": len(hazards),
            "omitted_hazard_count": omitted,
            "hazards": [hazard.to_dict() for hazard in hazards],
        }

    def __len__(self) -> int:
        return len(self.hazards)


def scan_runtime_hazards(
    session,
    *,
    handle: int | None = None,
    profile: str = FO76_TO_FO4_PROFILE,
) -> RuntimeHazardReport:
    """Scan the active plugin for known FO4 runtime hazards."""
    _validate_profile(profile)
    active = session.active if handle is None else session.get_by_handle(handle)
    if active is None:
        return RuntimeHazardReport(plugin_name="", game="", profile=profile)

    plugin_name = str(_field(active, "plugin_name", "") or "")
    game = str(_field(active, "game", "") or "").casefold()
    report = RuntimeHazardReport(plugin_name=plugin_name, game=game, profile=profile)
    if game != "fo4":
        return report

    records_by_sig: dict[str, Iterable[object]] = {}
    for record in _record_payloads(active.handle):
        record_sig = str(_field(record, "signature", "") or "")
        if (
            record_sig in _FO76_TO_FO4_RECORD_SIGS
            or record_sig in _FO4_LAYOUT_RECORD_SIGS
            or _has_any_subrecord(record, _FO4_MODEL_INFO_SIGS | {"MNAM"})
        ):
            records_by_sig.setdefault(record_sig, []).append(record)

    scan_runtime_hazard_records(
        records_by_sig,
        plugin_name=plugin_name,
        game=game,
        profile=profile,
        report=report,
    )
    return report


def scan_runtime_hazard_records(
    records_by_sig: Mapping[str, Iterable[object]],
    *,
    plugin_name: str,
    game: str,
    profile: str = FO76_TO_FO4_PROFILE,
    report: RuntimeHazardReport | None = None,
) -> RuntimeHazardReport:
    """Scan supplied records. This is used by tests and by EditorSession scans."""
    _validate_profile(profile)
    result = (
        report
        if report is not None
        else RuntimeHazardReport(plugin_name=plugin_name, game=game, profile=profile)
    )
    if game.casefold() != "fo4":
        return result

    for record in _flatten_iter(records_by_sig.get("IMAD", ())):
        _scan_imad_empty_runtime_data(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("NPC_", ())):
        _scan_npc_template_self_slots(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("PROJ", ())):
        _scan_proj_target_shape(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("QUST", ())):
        _scan_qust_event_alias_fill(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("TERM", ())):
        _scan_term_marker_parameters(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("REFR", ())):
        _scan_refr_xloc(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("EFSH", ())):
        _scan_efsh_target_shape(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("WTHR", ())):
        _scan_wthr_target_shape(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("NAVI", ())):
        _scan_navi_target_shape(result, plugin_name, record)
    for records in records_by_sig.values():
        for record in _flatten_iter(records):
            _scan_model_info_payloads(result, plugin_name, record)
            _scan_mnam_payloads(result, plugin_name, record)

    return result


def _scan_refr_xloc(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    occurrence = 0
    for subrecord in _subrecords(record):
        if str(_field(subrecord, "signature", "") or "") != "XLOC":
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) != 16:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-refr-xloc-size",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="REFR",
                    subrecord_sig="XLOC",
                    path=f"REFR.XLOC[{occurrence}]",
                    message=(
                        f"{_record_label('REFR', record)} has {len(data)}-byte XLOC; "
                        "FO4 requires the exact 16-byte lock-data layout"
                    ),
                )
            )
        occurrence += 1


def _scan_efsh_target_shape(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    by_sig = _subrecords_by_signature(record)
    required = ("ICON", "NAM7", "NAM8", "DATA", "DNAM")
    for required_sig in required:
        if by_sig.get(required_sig):
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-efsh-missing-target-subrecord",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="EFSH",
                subrecord_sig=required_sig,
                path=f"EFSH.{required_sig}",
                message=(
                    f"{_record_label('EFSH', record)} is missing required {required_sig}; "
                    "FO4 requires the rebuilt target EFSH contract"
                ),
            )
        )

    for occurrence, subrecord in enumerate(by_sig.get("ICON", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if _is_valid_zstring(data):
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-efsh-invalid-icon",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="EFSH",
                subrecord_sig="ICON",
                path=f"EFSH.ICON[{occurrence}]",
                message=(
                    f"{_record_label('EFSH', record)} has invalid ICON data; "
                    "FO4 requires one null-terminated fill-texture path"
                ),
            )
        )

    for occurrence, subrecord in enumerate(by_sig.get("DATA", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if data:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-efsh-nonempty-data",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="EFSH",
                    subrecord_sig="DATA",
                    path=f"EFSH.DATA[{occurrence}]",
                    message=(
                        f"{_record_label('EFSH', record)} has nonempty {len(data)}-byte DATA; "
                        "FO4's emitted EFSH DATA is an empty marker"
                    ),
                )
            )

    form_version = _record_form_version(record)
    for occurrence, subrecord in enumerate(by_sig.get("DNAM", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        valid_legacy_variant = (
            form_version is not None
            and form_version < _FO4_EFSH_CURRENT_FORM_VERSION
            and len(data) == _FO4_EFSH_LEGACY_DNAM_SIZE
        )
        if len(data) == _FO4_EFSH_DNAM_SIZE or valid_legacy_variant:
            continue
        variant = (
            f"v{form_version} requires {_FO4_EFSH_DNAM_SIZE}"
            if form_version is not None
            and form_version >= _FO4_EFSH_CURRENT_FORM_VERSION
            else (
                f"only a proven pre-v{_FO4_EFSH_CURRENT_FORM_VERSION} record may use "
                f"{_FO4_EFSH_LEGACY_DNAM_SIZE}"
            )
        )
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-efsh-dnam-size",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="EFSH",
                subrecord_sig="DNAM",
                path=f"EFSH.DNAM[{occurrence}]",
                message=(
                    f"{_record_label('EFSH', record)} has {len(data)}-byte DNAM; "
                    f"{variant} ({_FO4_EFSH_DNAM_SIZE} bytes for the emitted target layout)"
                ),
            )
        )

    _scan_singleton_subrecords(
        report,
        plugin_name,
        record,
        "EFSH",
        by_sig,
        required,
    )


def _scan_wthr_target_shape(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    by_sig = _subrecords_by_signature(record)
    required = (
        "LNAM",
        "MNAM",
        "NNAM",
        "RNAM",
        "QNAM",
        "PNAM",
        "JNAM",
        "NAM0",
        "NAM4",
        "FNAM",
        "DATA",
        "NAM1",
        "IMSP",
        "UNAM",
        "VNAM",
        "WNAM",
    )
    for required_sig in required:
        if by_sig.get(required_sig):
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-missing-target-subrecord",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig=required_sig,
                path=f"WTHR.{required_sig}",
                message=(
                    f"{_record_label('WTHR', record)} is missing required {required_sig}; "
                    "FO4 requires the expanded target weather layout"
                ),
            )
        )

    expected_sizes = {
        "LNAM": 4,
        "MNAM": 4,
        "NNAM": 4,
        "NAM0": 608,
        "FNAM": 72,
        "DATA": 20,
        "NAM1": 4,
        "IMSP": 32,
        "UNAM": 24,
        "VNAM": 4,
        "WNAM": 4,
    }
    for subrecord_sig, expected_size in expected_sizes.items():
        for occurrence, subrecord in enumerate(by_sig.get(subrecord_sig, ())):
            data = bytes(_field(subrecord, "data", b"") or b"")
            if len(data) == expected_size:
                continue
            report.add(
                RuntimeHazard(
                    rule_id=f"fo4-loader-wthr-{subrecord_sig.casefold()}-size",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="WTHR",
                    subrecord_sig=subrecord_sig,
                    path=f"WTHR.{subrecord_sig}[{occurrence}]",
                    message=(
                        f"{_record_label('WTHR', record)} has {len(data)}-byte "
                        f"{subrecord_sig}; FO4 requires exactly {expected_size} bytes"
                    ),
                )
            )

    for subrecord_sig in ("RNAM", "QNAM"):
        for occurrence, subrecord in enumerate(by_sig.get(subrecord_sig, ())):
            data = bytes(_field(subrecord, "data", b"") or b"")
            if data:
                continue
            report.add(
                RuntimeHazard(
                    rule_id=f"fo4-loader-wthr-{subrecord_sig.casefold()}-empty-rows",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="WTHR",
                    subrecord_sig=subrecord_sig,
                    path=f"WTHR.{subrecord_sig}[{occurrence}]",
                    message=(
                        f"{_record_label('WTHR', record)} has empty {subrecord_sig}; "
                        "FO4 requires one or more byte rows"
                    ),
                )
            )

    table_rows: dict[str, int] = {}
    for subrecord_sig in ("PNAM", "JNAM"):
        for occurrence, subrecord in enumerate(by_sig.get(subrecord_sig, ())):
            data = bytes(_field(subrecord, "data", b"") or b"")
            if data and len(data) % _FO4_WTHR_ROW_SIZE == 0:
                table_rows[subrecord_sig] = len(data) // _FO4_WTHR_ROW_SIZE
                continue
            report.add(
                RuntimeHazard(
                    rule_id=f"fo4-loader-wthr-{subrecord_sig.casefold()}-row-stride",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="WTHR",
                    subrecord_sig=subrecord_sig,
                    path=f"WTHR.{subrecord_sig}[{occurrence}]",
                    message=(
                        f"{_record_label('WTHR', record)} has {len(data)}-byte "
                        f"{subrecord_sig}; FO4 requires one or more 32-byte rows"
                    ),
                )
            )

    for occurrence, subrecord in enumerate(by_sig.get("NAM4", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if data and len(data) % 4 == 0:
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-nam4-row-stride",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig="NAM4",
                path=f"WTHR.NAM4[{occurrence}]",
                message=(
                    f"{_record_label('WTHR', record)} has {len(data)}-byte NAM4; "
                    "FO4 requires one or more four-byte cloud-layer rows"
                ),
            )
        )

    pnam_rows = table_rows.get("PNAM")
    if pnam_rows is not None and not (1 <= pnam_rows <= _FO4_WTHR_MAX_CLOUD_ROWS):
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-pnam-row-count",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig="PNAM",
                path="WTHR.PNAM[0]",
                message=(
                    f"{_record_label('WTHR', record)} has {pnam_rows} PNAM cloud rows; "
                    f"FO4 exposes at most {_FO4_WTHR_MAX_CLOUD_ROWS} cloud layers"
                ),
            )
        )
    jnam_rows = table_rows.get("JNAM")
    if pnam_rows is not None and jnam_rows is not None and pnam_rows != jnam_rows:
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-cloud-table-row-count",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig="JNAM",
                path="WTHR.JNAM[0]",
                message=(
                    f"{_record_label('WTHR', record)} has {pnam_rows} PNAM rows but "
                    f"{jnam_rows} JNAM rows; FO4 cloud color/alpha tables must align"
                ),
            )
        )

    dalc = by_sig.get("DALC", ())
    if len(dalc) != _FO4_WTHR_DALC_ROWS:
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-dalc-row-count",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig="DALC",
                path="WTHR.DALC",
                message=(
                    f"{_record_label('WTHR', record)} has {len(dalc)} DALC rows; "
                    f"FO4 requires exactly {_FO4_WTHR_DALC_ROWS}"
                ),
            )
        )
    for occurrence, subrecord in enumerate(dalc):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) == _FO4_WTHR_ROW_SIZE:
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-wthr-dalc-row-size",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="WTHR",
                subrecord_sig="DALC",
                path=f"WTHR.DALC[{occurrence}]",
                message=(
                    f"{_record_label('WTHR', record)} has {len(data)}-byte DALC row "
                    f"{occurrence}; FO4 requires exactly {_FO4_WTHR_ROW_SIZE} bytes"
                ),
            )
        )

    singleton_required = tuple(sig for sig in required if sig not in {"RNAM", "QNAM"})
    _scan_singleton_subrecords(
        report, plugin_name, record, "WTHR", by_sig, singleton_required
    )


def _scan_navi_target_shape(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    by_sig = _subrecords_by_signature(record)
    nver = by_sig.get("NVER", ())
    if len(nver) != 1:
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-navi-nver-count",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="NAVI",
                subrecord_sig="NVER",
                path="NAVI.NVER",
                message=(
                    f"{_record_label('NAVI', record)} has {len(nver)} NVER subrecords; "
                    "FO4 requires exactly one target NVER=15"
                ),
            )
        )
    for occurrence, subrecord in enumerate(nver):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) != 4:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-navi-nver-size",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="NAVI",
                    subrecord_sig="NVER",
                    path=f"NAVI.NVER[{occurrence}]",
                    message=(
                        f"{_record_label('NAVI', record)} has {len(data)}-byte NVER; "
                        "FO4 requires a four-byte NVER=15"
                    ),
                )
            )
            continue
        version = int.from_bytes(data, "little")
        if version == _FO4_NAVI_VERSION:
            continue
        legacy = version == _FO4_NAVI_LEGACY_FALLOUT_VERSION
        report.add(
            RuntimeHazard(
                rule_id=(
                    "fo4-loader-navi-legacy-nver11"
                    if legacy
                    else "fo4-loader-navi-target-version"
                ),
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="NAVI",
                subrecord_sig="NVER",
                path=f"NAVI.NVER[{occurrence}]",
                message=(
                    f"{_record_label('NAVI', record)} has "
                    f"{'legacy Fallout ' if legacy else ''}NVER={version}; "
                    f"FO4 requires rebuilt target NVER={_FO4_NAVI_VERSION}"
                ),
            )
        )

    for occurrence, subrecord in enumerate(by_sig.get("NVMI", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        error = _fo4_nvmi_shape_error(data)
        if error is None:
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-navi-nvmi-shape",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="NAVI",
                subrecord_sig="NVMI",
                path=f"NAVI.NVMI[{occurrence}]",
                message=(
                    f"{_record_label('NAVI', record)} has an invalid target NVMI: {error}"
                ),
            )
        )


def _fo4_nvmi_shape_error(data: bytes) -> str | None:
    # Fixed metadata through `preferred` occupies 24 bytes. The remainder is
    # three counted tables, optional island geometry, and a 12-byte pathing tail.
    if len(data) < 49:
        return f"payload is {len(data)} bytes; FO4 NVMI requires at least 49"
    offset = 24
    for label, row_size in (
        ("edge links", 4),
        ("preferred edge links", 4),
        ("door links", 8),
    ):
        if offset + 4 > len(data):
            return f"missing {label} count at offset {offset}"
        count = int.from_bytes(data[offset : offset + 4], "little")
        offset += 4
        end = offset + count * row_size
        if end > len(data):
            return (
                f"{label} rows exceed payload: offset={offset} count={count} "
                f"row_size={row_size} len={len(data)}"
            )
        offset = end

    if offset >= len(data):
        return "missing island-data selector"
    has_island_data = data[offset]
    offset += 1
    if has_island_data not in (0, 1):
        return f"island-data selector is {has_island_data}; FO4 requires 0 or 1"
    if has_island_data:
        if offset + 24 > len(data):
            return "island bounds exceed payload"
        offset += 24
        for label, row_size in (("island triangles", 6), ("island vertices", 12)):
            if offset + 4 > len(data):
                return f"missing {label} count at offset {offset}"
            count = int.from_bytes(data[offset : offset + 4], "little")
            offset += 4
            end = offset + count * row_size
            if end > len(data):
                return (
                    f"{label} rows exceed payload: offset={offset} count={count} "
                    f"row_size={row_size} len={len(data)}"
                )
            offset = end

    expected_end = offset + 12
    if expected_end != len(data):
        return f"pathing tail ends at {expected_end}, but payload length is {len(data)}"
    return None


def _scan_mnam_payloads(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    record_sig = str(_field(record, "signature", "") or "")
    if not record_sig:
        return
    occurrence = 0
    for subrecord in _subrecords(record):
        if str(_field(subrecord, "signature", "") or "") != "MNAM":
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        schema_disallows = record_sig not in _FO4_MNAM_RECORD_SIGS
        illegal_distant_lod = (
            len(data) == _FO4_DISTANT_LOD_MNAM_SIZE and record_sig != "STAT"
        )
        if schema_disallows or illegal_distant_lod:
            reason = (
                "FO4's record schema does not allow MNAM"
                if schema_disallows
                else "FO4's 1040-byte distant-LOD MNAM belongs only on STAT"
            )
            report.add(
                RuntimeHazard(
                    rule_id=(
                        "fo4-loader-mnam-disallowed-record"
                        if schema_disallows
                        else "fo4-loader-mnam-illegal-distant-lod"
                    ),
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig=record_sig,
                    subrecord_sig="MNAM",
                    path=f"{record_sig}.MNAM[{occurrence}]",
                    message=(
                        f"{_record_label(record_sig, record)} has {len(data)}-byte MNAM; "
                        f"{reason}"
                    ),
                )
            )
        occurrence += 1


def _subrecords_by_signature(record) -> dict[str, list[object]]:
    result: dict[str, list[object]] = {}
    for subrecord in _subrecords(record):
        signature = str(_field(subrecord, "signature", "") or "")
        result.setdefault(signature, []).append(subrecord)
    return result


def _is_valid_zstring(data: bytes) -> bool:
    return bool(data) and data.endswith(b"\0") and b"\0" not in data[:-1]


def _scan_singleton_subrecords(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
    record_sig: str,
    by_sig: Mapping[str, list[object]],
    signatures: Iterable[str],
) -> None:
    for subrecord_sig in signatures:
        count = len(by_sig.get(subrecord_sig, ()))
        if count <= 1:
            continue
        report.add(
            RuntimeHazard(
                rule_id=f"fo4-loader-{record_sig.casefold()}-duplicate-{subrecord_sig.casefold()}",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig=record_sig,
                subrecord_sig=subrecord_sig,
                path=f"{record_sig}.{subrecord_sig}",
                message=(
                    f"{_record_label(record_sig, record)} has {count} {subrecord_sig} "
                    "subrecords; FO4's target layout allows exactly one"
                ),
            )
        )


def _scan_proj_target_shape(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    occurrences: dict[str, int] = {}
    found_dnam = False
    form_version = _record_form_version(record)

    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        if subrecord_sig not in {"DATA", "DNAM", "NAM2"}:
            continue
        occurrence = occurrences.get(subrecord_sig, 0)
        occurrences[subrecord_sig] = occurrence + 1
        data = bytes(_field(subrecord, "data", b"") or b"")

        if subrecord_sig == "DNAM":
            found_dnam = True
            if len(data) == 93:
                continue
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-proj-dnam-size",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="PROJ",
                    subrecord_sig="DNAM",
                    path=f"PROJ.DNAM[{occurrence}]",
                    message=(
                        f"{_record_label('PROJ', record)} has {len(data)}-byte DNAM; "
                        "FO4 requires the exact 93-byte projectile data layout"
                    ),
                )
            )
            continue

        if subrecord_sig == "DATA" and data:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-proj-nonempty-data",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="PROJ",
                    subrecord_sig="DATA",
                    path=f"PROJ.DATA[{occurrence}]",
                    message=(
                        f"{_record_label('PROJ', record)} has nonempty {len(data)}-byte DATA; "
                        "FO4's projectile DATA subrecord is an empty marker"
                    ),
                )
            )
            continue

        # At form version 131, NAM2 model info must use FO4's four-counter
        # layout. This distinguishes source-game NAM2 from valid target NAM2
        # without flagging arbitrary valid FO4 model-info payloads.
        if (
            subrecord_sig == "NAM2"
            and form_version is not None
            and form_version >= _FO4_MODEL_INFO_COUNTER4_VERSION
        ):
            error = _fo4_model_info_error(data)
            if error is not None:
                report.add(
                    RuntimeHazard(
                        rule_id="fo4-loader-proj-legacy-nam2-model-info",
                        plugin_name=plugin_name,
                        form_id=_record_form_id(record),
                        record_sig="PROJ",
                        subrecord_sig="NAM2",
                        path=f"PROJ.NAM2[{occurrence}]",
                        message=(
                            f"{_record_label('PROJ', record)} has source-layout NAM2 "
                            f"model info incompatible with FO4 v{form_version}: {error}"
                        ),
                    )
                )

    if not found_dnam:
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-proj-missing-dnam",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="PROJ",
                subrecord_sig="DNAM",
                path="PROJ.DNAM",
                message=(
                    f"{_record_label('PROJ', record)} is missing required DNAM; "
                    "FO4 requires one 93-byte projectile data payload"
                ),
            )
        )


def _scan_imad_empty_runtime_data(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        if subrecord_sig not in _IMAD_EMPTY_UNSAFE_SIGS:
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) != 0:
            continue
        form_id = _record_form_id(record)
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-empty-imad-runtime-data",
                plugin_name=plugin_name,
                form_id=form_id,
                record_sig="IMAD",
                subrecord_sig=subrecord_sig,
                path=f"IMAD.{subrecord_sig}",
                message=(
                    f"{_record_label('IMAD', record)} has empty {subrecord_sig}; "
                    "FO4 can fault while reading image-space runtime data"
                ),
            )
        )


def _scan_npc_template_self_slots(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    form_id = _record_form_id(record)
    if not form_id:
        return

    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        if subrecord_sig != "TPTA":
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        for slot_index, raw_form_id in _iter_u32_slots(data):
            if raw_form_id != form_id:
                continue
            report.add(
                RuntimeHazard(
                    rule_id="fo76-to-fo4-npc-template-self-slot",
                    plugin_name=plugin_name,
                    form_id=form_id,
                    record_sig="NPC_",
                    subrecord_sig="TPTA",
                    path=f"NPC_.TPTA[{slot_index}]",
                    message=(
                        f"{_record_label('NPC_', record)} has TPTA slot {slot_index} "
                        "pointing back to itself; FO4 can loop while resolving actor templates"
                    ),
                )
            )


def _scan_qust_event_alias_fill(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    after_alias_table_anchor = False
    current_alias: int | None = None
    current_alias_ordinal = -1

    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        data = bytes(_field(subrecord, "data", b"") or b"")
        if subrecord_sig == "ANAM":
            after_alias_table_anchor = True
            current_alias = None
            current_alias_ordinal = -1
            continue
        if not after_alias_table_anchor:
            continue
        if subrecord_sig == "ALST":
            current_alias_ordinal += 1
            current_alias = (
                int.from_bytes(data[:4], "little") if len(data) >= 4 else None
            )
            continue
        if subrecord_sig not in _QUST_EVENT_ALIAS_FILL_SIGS:
            continue

        form_id = _record_form_id(record)
        alias_label = _alias_label(current_alias, current_alias_ordinal)
        report.add(
            RuntimeHazard(
                rule_id="fo76-to-fo4-qust-event-alias-fill",
                plugin_name=plugin_name,
                form_id=form_id,
                record_sig="QUST",
                subrecord_sig=subrecord_sig,
                path=f"QUST.{alias_label}.{subrecord_sig}",
                message=(
                    f"{_record_label('QUST', record)} retains {subrecord_sig} on "
                    f"{alias_label}; FO76 event alias fills are unsafe after conversion "
                    "because FO4 resolves them through an incompatible event table"
                ),
            )
        )


def _scan_term_marker_parameters(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    form_version = _record_form_version(record)
    if form_version is None or form_version < _FO4_TERM_MARKER_ROW_VERSION:
        return

    for occurrence, subrecord in enumerate(
        subrecord
        for subrecord in _subrecords(record)
        if str(_field(subrecord, "signature", "") or "") == "SNAM"
    ):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) % _FO4_TERM_MARKER_ROW_SIZE == 0:
            continue
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-term-snam-row-stride",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="TERM",
                subrecord_sig="SNAM",
                path=f"TERM.SNAM[{occurrence}]",
                message=(
                    f"{_record_label('TERM', record)} has {len(data)}-byte SNAM at "
                    f"form version {form_version}; FO4 expects 24-byte marker-parameter rows"
                ),
            )
        )


def _scan_model_info_payloads(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    form_version = _record_form_version(record)
    if form_version is None or form_version < _FO4_MODEL_INFO_COUNTER4_VERSION:
        return

    record_sig = str(_field(record, "signature", "") or "")
    occurrences: dict[str, int] = {}
    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        if subrecord_sig not in _FO4_MODEL_INFO_SIGS:
            continue
        occurrence = occurrences.get(subrecord_sig, 0)
        occurrences[subrecord_sig] = occurrence + 1
        data = bytes(_field(subrecord, "data", b"") or b"")
        error = _fo4_model_info_error(data)
        if error is not None:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-invalid-model-info",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig=record_sig,
                    subrecord_sig=subrecord_sig,
                    path=f"{record_sig}.{subrecord_sig}[{occurrence}]",
                    message=(
                        f"{_record_label(record_sig, record)} has invalid "
                        f"{subrecord_sig} model info: {error}"
                    ),
                )
            )


def _fo4_model_info_error(data: bytes) -> str | None:
    if len(data) < _FO4_MODEL_INFO_HEADER_SIZE:
        return f"payload is {len(data)} bytes; FO4 model-info header requires 20"

    counter_count = int.from_bytes(data[0:4], "little")
    if counter_count != 4:
        return f"counter_count is {counter_count}; FO4 requires 4"

    num_textures = int.from_bytes(data[4:8], "little")
    num_addon_nodes = int.from_bytes(data[8:12], "little")
    num_materials = int.from_bytes(data[16:20], "little")
    remaining = len(data) - _FO4_MODEL_INFO_HEADER_SIZE

    if num_textures > remaining // _FO4_MODEL_INFO_ENTRY_SIZE:
        return f"texture count {num_textures} exceeds the {len(data)}-byte payload"
    remaining -= num_textures * _FO4_MODEL_INFO_ENTRY_SIZE
    if num_addon_nodes > remaining // 4:
        return (
            f"addon-node count {num_addon_nodes} exceeds the {len(data)}-byte payload"
        )
    remaining -= num_addon_nodes * 4
    if num_materials > remaining // _FO4_MODEL_INFO_ENTRY_SIZE:
        return f"material count {num_materials} exceeds the {len(data)}-byte payload"
    remaining -= num_materials * _FO4_MODEL_INFO_ENTRY_SIZE
    if remaining != 0:
        expected = len(data) - remaining
        return f"payload is {len(data)} bytes; counters require exactly {expected}"
    return None


def _alias_label(alias_id: int | None, ordinal: int) -> str:
    if alias_id is not None:
        return f"Alias[{alias_id}]"
    if ordinal >= 0:
        return f"AliasOrdinal[{ordinal}]"
    return "Alias[unknown]"


def _record_label(record_sig: str, record) -> str:
    form_id = _record_form_id(record)
    form = f"0x{form_id:08X}" if form_id is not None else "unknown form"
    editor_id = _record_editor_id(record)
    if editor_id:
        return f"{record_sig} {form} {editor_id}"
    return f"{record_sig} {form}"


def _record_form_id(record) -> int | None:
    form_id = _field(record, "form_id", None)
    if isinstance(form_id, int):
        return form_id
    if isinstance(form_id, str):
        text = form_id.split(":", 1)[0]
        try:
            return int(text, 16)
        except ValueError:
            return None
    return None


def _record_form_version(record) -> int | None:
    form_version = _field(record, "form_version", None)
    if isinstance(form_version, int):
        return form_version
    if isinstance(form_version, str):
        try:
            return int(form_version, 0)
        except ValueError:
            return None
    return None


def _record_editor_id(record) -> str | None:
    for subrecord in _subrecords(record):
        if str(_field(subrecord, "signature", "") or "") != "EDID":
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        if not data:
            return None
        return data.split(b"\0", 1)[0].decode("utf-8", errors="replace") or None
    return None


def _subrecords(record):
    return _field(record, "subrecords", []) or []


def _has_any_subrecord(record, signatures: frozenset[str]) -> bool:
    return any(
        str(_field(subrecord, "signature", "") or "") in signatures
        for subrecord in _subrecords(record)
    )


def _field(item, name: str, default=None):
    if isinstance(item, dict):
        if name == "data" and "data_hex" in item:
            return bytes.fromhex(str(item.get("data_hex", "")))
        return item.get(name, default)
    return getattr(item, name, default)


def _record_payloads(handle: int) -> list[dict]:
    try:
        payload = json.loads(
            plugin_handle_call(handle, "export_plugin_text", "lossless", "json")
        )
    except Exception:
        return []
    return list(_iter_record_payloads(payload))


def _iter_record_payloads(payload):
    if isinstance(payload, dict):
        if "signature" in payload and "subrecords" in payload:
            yield payload
        for value in payload.values():
            yield from _iter_record_payloads(value)
    elif isinstance(payload, list):
        for item in payload:
            yield from _iter_record_payloads(item)


def _iter_u32_slots(data: bytes):
    for offset in range(0, len(data) - 3, 4):
        yield offset // 4, int.from_bytes(data[offset : offset + 4], "little")


def _flatten_iter(items):
    for item in items:
        children = _field(item, "children", None)
        if children is not None:
            yield from _flatten_iter(children)
        else:
            yield item


def _validate_profile(profile: str) -> None:
    if profile not in SUPPORTED_PROFILES:
        choices = ", ".join(SUPPORTED_PROFILES)
        raise ValueError(
            f"Unsupported runtime hazard profile {profile!r}; expected one of: {choices}"
        )
