"""Integration test for xEdit-parity error checking.

Loads B21_AppalachiaCell00Compare.esp, runs the deep validator, and asserts:
1. The walker reaches CELLs nested under WRLD (paths include `World Children of …`).
2. Subrecord ordering errors fire with xEdit-format text.
3. Unused-data warnings fire with xEdit-format text.

This is a regression guard for the mechanism, not byte-equal parity with
xEdit — our FO76 schema is broader than xEdit's so the specific signatures
flagged will differ.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.esp.editor.session import EditorSession
from creation_lib.esp.editor.validate import validate, IssueCategory, Severity


PLUGIN_REL = Path("mods/B21_AppalachiaCell00Compare/B21_AppalachiaCell00Compare.esp")


@pytest.fixture
def plugin_path(repo_root: Path) -> Path:
    p = repo_root / PLUGIN_REL
    if not p.exists():
        pytest.skip(f"fixture plugin not present: {p}")
    return p


@pytest.fixture
def report(plugin_path: Path):
    session = EditorSession()
    session.load(plugin_path, game="fo76")
    return validate(session)


def test_walker_reaches_records_nested_in_wrld(report):
    """CELLs/REFRs under WRLD must show up in issue paths."""
    nested_paths = [
        i.path for i in report.issues
        if i.path and "GRUP World Children of" in i.path
    ]
    assert nested_paths, "no issues with paths inside WRLD groups — walker isn't reaching nested records"


def test_subrecord_ordering_errors_use_xedit_format(report):
    """xEdit text parity: 'Error: record SIG contains unexpected (or out of order) subrecord XXXX HHHHHHHH'."""
    ordering = [
        i for i in report.issues
        if "unexpected (or out of order)" in i.message
    ]
    assert ordering, "no ordering errors fired — schema-driven check not running"
    # Verify the format: 'Error: record SIG contains unexpected (or out of order) subrecord SUB HEX'
    sample = ordering[0].message
    assert sample.startswith("Error: record "), f"bad prefix: {sample!r}"
    assert " contains unexpected (or out of order) subrecord " in sample
    # Tail should be 8 uppercase hex chars
    last_token = sample.split()[-1]
    assert len(last_token) == 8 and all(c in "0123456789ABCDEF" for c in last_token), f"bad hex tail: {sample!r}"


def test_unused_data_warnings_use_xedit_format(report):
    """xEdit text parity: '<Warning: Unused data in: PATH>'."""
    warnings = [
        i for i in report.issues
        if i.message.startswith("<Warning: Unused data in:")
    ]
    assert warnings, "no unused-data warnings fired — codec size check not running"
    sample = warnings[0]
    assert sample.message.endswith(">"), f"bad suffix: {sample.message!r}"
    assert sample.severity == Severity.WARNING, f"wrong severity: {sample.severity}"


def test_issues_carry_path_and_signature(report):
    """The new mechanism populates the path and signature fields on every walker-emitted issue."""
    walker_issues = [
        i for i in report.issues
        if "unexpected (or out of order)" in i.message
        or i.message.startswith("<Warning: Unused data in:")
    ]
    assert walker_issues
    sample = walker_issues[0]
    assert sample.path is not None, "path is missing on walker issue"
    assert sample.signature is not None, "signature is missing on walker issue"
    assert len(sample.signature) == 4, f"signature should be 4 chars: {sample.signature!r}"


def test_missing_master_still_fires(report):
    """The existing structural checks (missing_master, broken_reference, parse_error, itm, udr) must still run."""
    missing_masters = [
        i for i in report.issues if i.category == IssueCategory.MISSING_MASTER
    ]
    # Appalachia plugin references fallout4.esm which isn't loaded — should fire 1 missing_master
    assert missing_masters, "missing_master check not running — structural validation regressed"
