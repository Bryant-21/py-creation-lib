"""Error checker.

The walker, per-element ordering check, and unused-data detection all live
in Rust (`py_creation_lib/native/esp/src/validate_walker.rs`). This module
exposes the result as `ValidationReport` and adds Python-side checks that need
plugin-level context.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import Enum
from typing import Optional

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_validation_records,
    validate_plugin_deep,
)


class Severity(Enum):
    INFO = "info"
    WARNING = "warning"
    ERROR = "error"


class IssueCategory(Enum):
    MISSING_MASTER = "missing_master"
    BROKEN_REFERENCE = "broken_reference"
    PARSE_ERROR = "parse_error"
    ITM = "itm"
    UDR = "udr"
    CIRCULAR_LIST = "circular_list"
    CK_COMPATIBILITY = "ck_compatibility"
    FIELD_ERROR = "field_error"


@dataclass
class Issue:
    severity: Severity
    category: IssueCategory
    plugin_handle: int
    plugin_name: str
    message: str
    form_id: Optional[int] = None
    path: Optional[str] = None
    signature: Optional[str] = None


@dataclass
class ValidationReport:
    issues: list[Issue] = field(default_factory=list)

    def add(self, issue: Issue) -> None:
        self.issues.append(issue)

    def by_category(self, category: IssueCategory) -> list[Issue]:
        return [i for i in self.issues if i.category == category]

    def __iter__(self):
        return iter(self.issues)

    def __len__(self) -> int:
        return len(self.issues)


def validate(session: EditorSession, *, handle: int | None = None) -> ValidationReport:
    """Validate a plugin using the native deep walker + circular-list check."""
    report = ValidationReport()
    target = session.active if handle is None else session.get_by_handle(handle)
    if target is None:
        return report
    load_order = [(p.handle, p.plugin_name) for p in session.plugins]
    for entry in validate_plugin_deep(target.handle, load_order):
        try:
            category = IssueCategory(entry["category"])
        except ValueError:
            category = IssueCategory.FIELD_ERROR
        report.add(
            Issue(
                severity=Severity(entry["severity"]),
                category=category,
                plugin_handle=int(entry["plugin_handle"]),
                plugin_name=str(entry["plugin_name"]),
                message=str(entry["message"]),
                form_id=entry.get("form_id"),
                path=entry.get("path"),
                signature=entry.get("signature"),
            )
        )
    records = _record_payloads(target.handle)
    _check_ck_compatibility(session, target, report, records)
    _check_circular_leveled_lists(session, target, report, records)
    return report


_CK_MAX_SUBRECORD_SIZES = {
    ("ARMO", "DAMA"): 8,
    ("FURN", "WBDT"): 2,
    ("MOVT", "SPED"): 112,
    ("TERM", "WBDT"): 2,
    ("WEAP", "DAMA"): 8,
}
_CK_EXACT_SUBRECORD_SIZES = {
    ("MGEF", "DATA"): 152,
}
_FO4_MGEF_ARCHETYPE_OFFSET = 64
_FO4_MGEF_ARCHETYPE_SIZE = 4
_FO4_MGEF_MAX_ARCHETYPE = 49
_FO4_MAX_CONDITION_FUNCTION_ID = 817


def _check_ck_compatibility_subrecord_sizes(session, active, report: ValidationReport) -> None:
    _check_ck_compatibility(session, active, report)


def _check_ck_compatibility(
    session,
    active,
    report: ValidationReport,
    records=None,
) -> None:
    if _active_game(active) != "fo4":
        return

    for record in records if records is not None else _record_payloads(active.handle):
        record_sig = str(_field(record, "signature", "") or "")
        _check_ck_record_compatibility(active, record_sig, record, report)


def _check_ck_record_compatibility(active, record_sig: str, record, report: ValidationReport) -> None:
    for subrecord in _subrecords(record):
        subrecord_sig = str(_field(subrecord, "signature", "") or "")
        data = bytes(_field(subrecord, "data", b"") or b"")

        max_size = _CK_MAX_SUBRECORD_SIZES.get((record_sig, subrecord_sig))
        if max_size is not None and len(data) > max_size:
            _add_ck_issue(
                active,
                record,
                report,
                subrecord_sig=subrecord_sig,
                path=f"{record_sig}.{subrecord_sig}",
                message=(
                    f"CK compatibility: {_record_label(record_sig, record)} "
                    f"{subrecord_sig} is {len(data)} bytes; FO4 CK max is {max_size} bytes"
                ),
            )

        exact_size = _CK_EXACT_SUBRECORD_SIZES.get((record_sig, subrecord_sig))
        if exact_size is not None and len(data) != exact_size:
            _add_ck_issue(
                active,
                record,
                report,
                subrecord_sig=subrecord_sig,
                path=f"{record_sig}.{subrecord_sig}",
                message=(
                    f"CK compatibility: {_record_label(record_sig, record)} "
                    f"{subrecord_sig} is {len(data)} bytes; FO4 CK expects {exact_size} bytes"
                ),
            )

        if record_sig == "MGEF" and subrecord_sig == "DATA":
            _check_mgef_archetype(active, record_sig, record, data, report)
        if subrecord_sig in {"CTDA", "CTDT"}:
            _check_condition_function(active, record_sig, record, subrecord_sig, data, report)


def _check_mgef_archetype(
    active,
    record_sig: str,
    record,
    data: bytes,
    report: ValidationReport,
) -> None:
    end = _FO4_MGEF_ARCHETYPE_OFFSET + _FO4_MGEF_ARCHETYPE_SIZE
    if len(data) < end:
        _add_ck_issue(
            active,
            record,
            report,
            subrecord_sig="DATA",
            path="MGEF.DATA+64",
            message=(
                f"CK compatibility: {_record_label(record_sig, record)} DATA is {len(data)} "
                f"bytes; FO4 CK needs at least {end} bytes to read Archetype"
            ),
        )
        return

    archetype = int.from_bytes(data[_FO4_MGEF_ARCHETYPE_OFFSET:end], "little")
    if archetype <= _FO4_MGEF_MAX_ARCHETYPE:
        return

    _add_ck_issue(
        active,
        record,
        report,
        subrecord_sig="DATA",
        path="MGEF.DATA+64",
        message=(
            f"CK compatibility: {_record_label(record_sig, record)} DATA+64 Archetype is "
            f"0x{archetype:08X} ({archetype}); FO4 CK max is {_FO4_MGEF_MAX_ARCHETYPE}"
        ),
    )


def _check_condition_function(
    active,
    record_sig: str,
    record,
    subrecord_sig: str,
    data: bytes,
    report: ValidationReport,
) -> None:
    if len(data) < 10:
        return
    function_id = int.from_bytes(data[8:10], "little")
    if function_id <= _FO4_MAX_CONDITION_FUNCTION_ID:
        return

    _add_ck_issue(
        active,
        record,
        report,
        subrecord_sig=subrecord_sig,
        path=f"{record_sig}.{subrecord_sig}+8",
        message=(
            f"CK compatibility: {_record_label(record_sig, record)} {subrecord_sig}+8 "
            f"condition function id is {function_id}; FO4 CK max is "
            f"{_FO4_MAX_CONDITION_FUNCTION_ID}"
        ),
    )


def _add_ck_issue(
    active,
    record,
    report: ValidationReport,
    *,
    subrecord_sig: str,
    path: str,
    message: str,
) -> None:
    form_id = _record_form_id(record)
    report.add(
        Issue(
            severity=Severity.ERROR,
            category=IssueCategory.CK_COMPATIBILITY,
            plugin_handle=active.handle,
            plugin_name=active.plugin_name,
            message=message,
            form_id=form_id if isinstance(form_id, int) else None,
            path=path,
            signature=subrecord_sig,
        )
    )


def _active_game(active) -> str:
    return str(getattr(active, "game", "") or "").casefold()


def _record_label(record_sig: str, record) -> str:
    form_id = _record_form_id(record)
    form = f"0x{form_id:08X}" if isinstance(form_id, int) else "unknown form"
    editor_id = _record_editor_id(record)
    if editor_id:
        return f"{record_sig} {form} {editor_id}"
    return f"{record_sig} {form}"


def _record_editor_id(record) -> str | None:
    for subrecord in _subrecords(record):
        if str(_field(subrecord, "signature", "") or "") != "EDID":
            continue
        data = bytes(_field(subrecord, "data", b"") or b"")
        if not data:
            return None
        return data.split(b"\0", 1)[0].decode("utf-8", errors="replace") or None
    return None


# -- Circular leveled list detection (graph-level — stays in Python) ----------

_LVL_SIGS = ("LVLI", "LVLN", "LVLC", "LVSP")
_LVLO_STRIDE = 12
_LVLO_FORMID_OFFSET = 4


def _check_circular_leveled_lists(
    session,
    active,
    report: ValidationReport,
    records=None,
) -> None:
    """Detect cycles in LVLI/LVLN/LVLC/LVSP chains."""
    handle = active.handle

    lvl_records: dict[int, object] = {}
    for rec in records if records is not None else _record_payloads(handle):
        if _field(rec, "signature", None) in _LVL_SIGS:
            form_id = _record_form_id(rec)
            if form_id is not None:
                lvl_records[form_id] = rec

    visited: set[int] = set()
    for fid, record in lvl_records.items():
        if fid in visited:
            continue
        cycle = _walk_lvl(fid, record, lvl_records, [], visited)
        if cycle:
            path = " → ".join(f"0x{f:08X}" for f in cycle)
            report.add(
                Issue(
                    severity=Severity.ERROR,
                    category=IssueCategory.CIRCULAR_LIST,
                    plugin_handle=handle,
                    plugin_name=active.plugin_name,
                    message=f"Circular leveled list: {path}",
                    form_id=fid,
                )
            )


def _walk_lvl(fid, record, lvl_records, stack, visited):
    if fid in stack:
        return stack[stack.index(fid):] + [fid]
    if fid in visited:
        return None
    stack.append(fid)
    try:
        for child_fid in _lvlo_form_ids(record):
            child = lvl_records.get(child_fid)
            if child is None:
                continue
            cycle = _walk_lvl(child_fid, child, lvl_records, stack, visited)
            if cycle:
                return cycle
    finally:
        stack.pop()
    visited.add(fid)
    return None


def _lvlo_form_ids(record) -> list[int]:
    out = []
    for sub in _subrecords(record):
        if _field(sub, "signature", None) != "LVLO":
            continue
        data = bytes(_field(sub, "data", b"") or b"")
        if len(data) >= _LVLO_STRIDE:
            for start in range(0, len(data) - _LVLO_STRIDE + 1, _LVLO_STRIDE):
                fid = int.from_bytes(
                    data[start + _LVLO_FORMID_OFFSET:start + _LVLO_FORMID_OFFSET + 4],
                    "little",
                )
                out.append(fid)
    return out


def _record_payloads(handle: int) -> list[dict]:
    try:
        return plugin_handle_validation_records(handle)
    except Exception:
        try:
            payload = json.loads(plugin_handle_call(handle, "export_plugin_text", "lossless", "json"))
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


def _subrecords(record) -> list:
    return list(_field(record, "subrecords", []) or [])


def _field(item, name: str, default=None):
    if isinstance(item, dict):
        if name == "data" and "data_hex" in item:
            return bytes.fromhex(str(item.get("data_hex", "")))
        return item.get(name, default)
    return getattr(item, name, default)


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
