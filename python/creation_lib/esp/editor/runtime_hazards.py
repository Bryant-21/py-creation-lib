"""Runtime-hazard checks for FO4 loader crash patterns.

These checks are intentionally narrower than xEdit-style validation. They flag
record shapes that can parse cleanly but are known to crash or wedge FO4 while
loading converted plugins.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Mapping

from creation_lib.esp.native_runtime import (
    plugin_handle_close,
    plugin_handle_get,
    plugin_handle_inspection_records,
    plugin_handle_load_index,
)


FO76_TO_FO4_PROFILE = "fo76-to-fo4"
FO4_TARGET_SHAPE_PROFILE = "fo4-target-shape"
SUPPORTED_PROFILES = (FO76_TO_FO4_PROFILE, FO4_TARGET_SHAPE_PROFILE)

_FO76_TO_FO4_RECORD_SIGS = ("IMAD", "LGTM", "MISC", "NPC_", "PROJ", "QUST", "TERM")
_FO4_TARGET_SHAPE_RECORD_SIGS = frozenset({"IMAD", "LGTM", "MISC"})
_FO4_LAYOUT_RECORD_SIGS = frozenset({"EFSH", "NAVI", "NAVM", "REFR", "WTHR"})
_QUST_EVENT_ALIAS_FILL_SIGS = {"ALFE", "ALFD"}
_IMAD_RUNTIME_ARRAY_STRIDES = {
    "TNAM": 20,
    "NAM3": 20,
    "RNAM": 8,
    "SNAM": 8,
    "UNAM": 8,
    "NAM1": 8,
    "NAM2": 8,
    "WNAM": 8,
    "XNAM": 8,
    "YNAM": 8,
    "NAM5": 8,
    "NAM6": 8,
    **{f"{value:c}IAD": 8 for value in range(0x00, 0x15)},
    **{f"{value:c}IAD": 8 for value in range(0x40, 0x55)},
}
_IMAD_DNAM_COUNTED_ARRAY_STRIDES = {
    **_IMAD_RUNTIME_ARRAY_STRIDES,
    "BNAM": 8,
    "VNAM": 8,
    "NAM4": 8,
}
_IMAD_DNAM_COUNT_OFFSETS = {
    **{f"{value:c}IAD": 8 + value * 8 for value in range(0x00, 0x15)},
    **{f"{value:c}IAD": 12 + (value - 0x40) * 8 for value in range(0x40, 0x55)},
    "TNAM": 176,
    "BNAM": 180,
    "VNAM": 184,
    "RNAM": 188,
    "SNAM": 192,
    "UNAM": 196,
    "WNAM": 212,
    "XNAM": 216,
    "YNAM": 220,
    "NAM1": 228,
    "NAM2": 232,
    "NAM3": 236,
    "NAM4": 240,
    "NAM5": 244,
    "NAM6": 248,
}
_FO4_LGTM_DALC_SIZE = 32
_FO4_MISC_DATA_SIZE = 8
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
_FO4_CANONICAL_NAVI_FORM_ID = 0x00000FF1
_FO4_PATHING_CELL_CRC_HASH = 0xA5E9A03C
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
    """Scan one loaded plugin for known FO4 runtime hazards."""
    _validate_profile(profile)
    active = session.active if handle is None else session.get_by_handle(handle)
    if active is None:
        raise RuntimeError(
            "No active plugin to scan; pass handle=loaded.handle when scanning a master"
        )

    plugin_name = str(_field(active, "plugin_name", "") or "")
    game = str(_field(active, "game", "") or "").casefold()
    return _scan_runtime_hazards(active.handle, plugin_name, game, profile)


def scan_runtime_hazards_path(
    plugin_path: str | Path,
    *,
    game: str,
    profile: str = FO76_TO_FO4_PROFILE,
) -> RuntimeHazardReport:
    _validate_profile(profile)
    path = Path(plugin_path)
    if game.casefold() != "fo4":
        return RuntimeHazardReport(plugin_name=path.name, game=game, profile=profile)
    handle = plugin_handle_load_index(str(path), game=game)
    if handle is None:
        raise RuntimeError("Native index loading is required for runtime-hazard inspection")
    try:
        return _scan_runtime_hazards(handle, path.name, game, profile)
    finally:
        plugin_handle_close(handle)


def _scan_runtime_hazards(
    handle: int, plugin_name: str, game: str, profile: str
) -> RuntimeHazardReport:
    game = game.casefold()
    report = RuntimeHazardReport(plugin_name=plugin_name, game=game, profile=profile)
    if game != "fo4":
        return report

    records_by_sig: dict[str, Iterable[object]] = {}
    for record in _record_payloads(handle, profile):
        record_sig = str(_field(record, "signature", "") or "")
        if profile == FO4_TARGET_SHAPE_PROFILE:
            selected = record_sig in _FO4_TARGET_SHAPE_RECORD_SIGS
        else:
            selected = (
                record_sig in _FO76_TO_FO4_RECORD_SIGS
                or record_sig in _FO4_LAYOUT_RECORD_SIGS
                or _has_any_subrecord(record, _FO4_MODEL_INFO_SIGS | {"MNAM"})
            )
        if selected:
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
        _scan_imad_runtime_arrays(result, plugin_name, record)
    for record in _flatten_iter(records_by_sig.get("LGTM", ())):
        _scan_exact_subrecord_size(
            result,
            plugin_name,
            record,
            record_sig="LGTM",
            subrecord_sig="DALC",
            expected_size=_FO4_LGTM_DALC_SIZE,
        )
    for record in _flatten_iter(records_by_sig.get("MISC", ())):
        _scan_exact_subrecord_size(
            result,
            plugin_name,
            record,
            record_sig="MISC",
            subrecord_sig="DATA",
            expected_size=_FO4_MISC_DATA_SIZE,
            required=False,
        )
    if profile == FO4_TARGET_SHAPE_PROFILE:
        return result
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
    _scan_navm_pathing_cell_crc(
        result,
        plugin_name,
        _flatten_iter(records_by_sig.get("NAVM", ())),
    )
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
        "WGDR": 32,
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
    form_id = _record_form_id(record)
    if form_id != _FO4_CANONICAL_NAVI_FORM_ID:
        form_id_text = f"{form_id:08X}" if form_id is not None else "unknown"
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-navi-record-formid",
                plugin_name=plugin_name,
                form_id=form_id,
                record_sig="NAVI",
                path="NAVI",
                message=(
                    f"{_record_label('NAVI', record)} uses raw FormID "
                    f"{form_id_text}; FO4 requires the canonical NavMeshInfoMap "
                    f"override {_FO4_CANONICAL_NAVI_FORM_ID:08X}"
                ),
            )
        )

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

    crc_mismatch_count = 0
    first_crc_mismatch: tuple[int, int] | None = None
    for occurrence, subrecord in enumerate(by_sig.get("NVMI", ())):
        data = bytes(_field(subrecord, "data", b"") or b"")
        error, pathing_tail_offset = _fo4_nvmi_layout(data)
        if error is None and pathing_tail_offset is not None:
            pathing_cell_crc = int.from_bytes(
                data[pathing_tail_offset : pathing_tail_offset + 4], "little"
            )
            if pathing_cell_crc != _FO4_PATHING_CELL_CRC_HASH:
                crc_mismatch_count += 1
                if first_crc_mismatch is None:
                    first_crc_mismatch = (occurrence, pathing_cell_crc)
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

    if first_crc_mismatch is not None:
        occurrence, pathing_cell_crc = first_crc_mismatch
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-navi-pathing-cell-crc",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="NAVI",
                subrecord_sig="NVMI",
                path=f"NAVI.NVMI[{occurrence}].PathingCellCRCHash",
                message=(
                    f"{_record_label('NAVI', record)} has {crc_mismatch_count} NVMI rows "
                    "with a noncanonical PathingCell CRC; first mismatch uses "
                    f"{pathing_cell_crc:08X}, but FO4 requires "
                    f"{_FO4_PATHING_CELL_CRC_HASH:08X}"
                ),
            )
        )


def _scan_navm_pathing_cell_crc(
    report: RuntimeHazardReport,
    plugin_name: str,
    records: Iterable[object],
) -> None:
    mismatch_count = 0
    first_mismatch: tuple[int | None, int, int] | None = None
    for record in records:
        occurrence = 0
        for subrecord in _subrecords(record):
            if str(_field(subrecord, "signature", "") or "") != "NVNM":
                continue
            data = bytes(_field(subrecord, "data", b"") or b"")
            if len(data) >= 8:
                pathing_cell_crc = int.from_bytes(data[4:8], "little")
                if pathing_cell_crc != _FO4_PATHING_CELL_CRC_HASH:
                    mismatch_count += 1
                    if first_mismatch is None:
                        first_mismatch = (
                            _record_form_id(record),
                            occurrence,
                            pathing_cell_crc,
                        )
            occurrence += 1

    if first_mismatch is None:
        return
    form_id, occurrence, pathing_cell_crc = first_mismatch
    report.add(
        RuntimeHazard(
            rule_id="fo4-loader-navm-pathing-cell-crc",
            plugin_name=plugin_name,
            form_id=form_id,
            record_sig="NAVM",
            subrecord_sig="NVNM",
            path=f"NAVM.NVNM[{occurrence}].PathingCellCRCHash",
            message=(
                f"{plugin_name} has {mismatch_count} NAVM NVNM rows with a "
                "noncanonical PathingCell CRC; first mismatch uses "
                f"{pathing_cell_crc:08X}, but FO4 requires "
                f"{_FO4_PATHING_CELL_CRC_HASH:08X}"
            ),
        )
    )


def _fo4_nvmi_layout(data: bytes) -> tuple[str | None, int | None]:
    # Fixed metadata through `preferred` occupies 24 bytes. The remainder is
    # three counted tables, optional island geometry, and a 12-byte pathing tail.
    if len(data) < 49:
        return f"payload is {len(data)} bytes; FO4 NVMI requires at least 49", None
    offset = 24
    for label, row_size in (
        ("edge links", 4),
        ("preferred edge links", 4),
        ("door links", 8),
    ):
        if offset + 4 > len(data):
            return f"missing {label} count at offset {offset}", None
        count = int.from_bytes(data[offset : offset + 4], "little")
        offset += 4
        end = offset + count * row_size
        if end > len(data):
            return (
                f"{label} rows exceed payload: offset={offset} count={count} "
                f"row_size={row_size} len={len(data)}"
            ), None
        offset = end

    if offset >= len(data):
        return "missing island-data selector", None
    has_island_data = data[offset]
    offset += 1
    if has_island_data not in (0, 1):
        return f"island-data selector is {has_island_data}; FO4 requires 0 or 1", None
    if has_island_data:
        if offset + 24 > len(data):
            return "island bounds exceed payload", None
        offset += 24
        for label, row_size in (("island triangles", 6), ("island vertices", 12)):
            if offset + 4 > len(data):
                return f"missing {label} count at offset {offset}", None
            count = int.from_bytes(data[offset : offset + 4], "little")
            offset += 4
            end = offset + count * row_size
            if end > len(data):
                return (
                    f"{label} rows exceed payload: offset={offset} count={count} "
                    f"row_size={row_size} len={len(data)}"
                ), None
            offset = end

    expected_end = offset + 12
    if expected_end != len(data):
        return (
            f"pathing tail ends at {expected_end}, but payload length is {len(data)}",
            None,
        )
    return None, offset


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


def _scan_imad_runtime_arrays(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
) -> None:
    by_sig = _subrecords_by_signature(record)
    for subrecord_sig, row_stride in _IMAD_RUNTIME_ARRAY_STRIDES.items():
        display_sig = _display_subrecord_signature(subrecord_sig)
        rows = by_sig.get(subrecord_sig, ())
        if not rows:
            report.add(
                RuntimeHazard(
                    rule_id="fo4-loader-missing-imad-runtime-data",
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="IMAD",
                    subrecord_sig=subrecord_sig,
                    path=f"IMAD.{display_sig}",
                    message=(
                        f"{_record_label('IMAD', record)} is missing {display_sig}; "
                        "FO4 requires every fixed-stride image-space runtime array"
                    ),
                )
            )
            continue
        for subrecord in rows:
            data = bytes(_field(subrecord, "data", b"") or b"")
            if data and len(data) % row_stride == 0:
                continue
            rule_id = (
                "fo4-loader-empty-imad-runtime-data"
                if not data
                else "fo4-loader-imad-runtime-row-stride"
            )
            report.add(
                RuntimeHazard(
                    rule_id=rule_id,
                    plugin_name=plugin_name,
                    form_id=_record_form_id(record),
                    record_sig="IMAD",
                    subrecord_sig=subrecord_sig,
                    path=f"IMAD.{display_sig}",
                    message=(
                        f"{_record_label('IMAD', record)} has {len(data)}-byte "
                        f"{display_sig}; FO4 requires one or more {row_stride}-byte "
                        "image-space runtime rows"
                    ),
                )
            )

    _scan_imad_dnam_array_counts(report, plugin_name, record, by_sig)


def _scan_imad_dnam_array_counts(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
    by_sig: Mapping[str, Iterable[object]],
) -> None:
    dnam_rows = list(by_sig.get("DNAM", ()))
    if len(dnam_rows) != 1:
        return
    dnam = bytes(_field(dnam_rows[0], "data", b"") or b"")

    for subrecord_sig, row_stride in _IMAD_DNAM_COUNTED_ARRAY_STRIDES.items():
        count_offset = _IMAD_DNAM_COUNT_OFFSETS[subrecord_sig]
        if len(dnam) < count_offset + 4:
            continue

        rows = list(by_sig.get(subrecord_sig, ()))
        if not rows:
            continue
        payloads = [bytes(_field(row, "data", b"") or b"") for row in rows]
        if any(not payload or len(payload) % row_stride for payload in payloads):
            continue

        expected_count = int.from_bytes(dnam[count_offset : count_offset + 4], "little")
        actual_count = sum(len(payload) // row_stride for payload in payloads)
        if expected_count == actual_count:
            continue

        display_sig = _display_subrecord_signature(subrecord_sig)
        report.add(
            RuntimeHazard(
                rule_id="fo4-loader-imad-dnam-array-count-mismatch",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig="IMAD",
                subrecord_sig=subrecord_sig,
                path=f"IMAD.{display_sig}",
                message=(
                    f"{_record_label('IMAD', record)} {display_sig} DNAM count is "
                    f"{expected_count} but the array has {actual_count} rows; FO4 "
                    "allocates from DNAM before reading the fixed-stride payload"
                ),
            )
        )


def _scan_exact_subrecord_size(
    report: RuntimeHazardReport,
    plugin_name: str,
    record,
    *,
    record_sig: str,
    subrecord_sig: str,
    expected_size: int,
    required: bool = True,
) -> None:
    rows = _subrecords_by_signature(record).get(subrecord_sig, ())
    if not rows:
        if not required:
            return
        report.add(
            RuntimeHazard(
                rule_id=f"fo4-loader-{record_sig.casefold()}-missing-{subrecord_sig.casefold()}",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig=record_sig,
                subrecord_sig=subrecord_sig,
                path=f"{record_sig}.{subrecord_sig}",
                message=(
                    f"{_record_label(record_sig, record)} is missing {subrecord_sig}; "
                    f"FO4 requires exactly {expected_size} bytes"
                ),
            )
        )
        return
    for occurrence, subrecord in enumerate(rows):
        data = bytes(_field(subrecord, "data", b"") or b"")
        if len(data) == expected_size:
            continue
        report.add(
            RuntimeHazard(
                rule_id=f"fo4-loader-{record_sig.casefold()}-{subrecord_sig.casefold()}-size",
                plugin_name=plugin_name,
                form_id=_record_form_id(record),
                record_sig=record_sig,
                subrecord_sig=subrecord_sig,
                path=f"{record_sig}.{subrecord_sig}[{occurrence}]",
                message=(
                    f"{_record_label(record_sig, record)} has {len(data)}-byte "
                    f"{subrecord_sig}; FO4 requires exactly {expected_size} bytes"
                ),
            )
        )


def _display_subrecord_signature(signature: str) -> str:
    return "".join(
        character if character.isprintable() else f"\\x{ord(character):02X}"
        for character in signature
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


def _record_payloads(handle: int, profile: str) -> list[dict]:
    if profile == FO4_TARGET_SHAPE_PROFILE:
        signatures = _FO4_TARGET_SHAPE_RECORD_SIGS
        subrecord_signatures = frozenset()
    else:
        signatures = set(_FO76_TO_FO4_RECORD_SIGS) | _FO4_LAYOUT_RECORD_SIGS
        subrecord_signatures = _FO4_MODEL_INFO_SIGS | {"MNAM"}
    records = plugin_handle_inspection_records(
        handle, sorted(signatures), sorted(subrecord_signatures)
    )
    master_count = len(plugin_handle_get(handle, "masters") or [])
    for record in records:
        raw_form_id = record["form_id"]
        record["raw_form_id"] = raw_form_id
        index = raw_form_id >> 24
        # Lossless export used object IDs for resolved owners and FF-local IDs.
        # Retain that diagnostic convention while keeping the raw identity too.
        if index <= master_count or index == 0xFF:
            record["form_id"] = raw_form_id & 0x00FFFFFF
    return records


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
