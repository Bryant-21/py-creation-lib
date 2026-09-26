import importlib.util
from pathlib import Path

from creation_lib.havok.collision_validation import summarize_violations
from creation_lib.havok.native_runtime import validate_collision_blob_native

_SCRIPT = Path(__file__).resolve().parents[5] / "scripts" / "mine_collision_invariants.py"
_spec = importlib.util.spec_from_file_location("mine_collision_invariants", _SCRIPT)
_mine = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mine)
aggregate_invariants = _mine.aggregate_invariants


def test_validate_aggregate_and_summarize():
    # Garbage bytes are not a parseable Havok packfile: the native wrapper surfaces
    # an exception rather than crashing the interpreter. The Python pass catches this
    # and turns it into a finding (report-only), so the raise here is the contract.
    import pytest

    with pytest.raises(Exception):
        validate_collision_blob_native(b"NOT_A_HAVOK_BLOB", "{}")

    rows = [
        {"aggregate": {"collision_layers": [1, 2], "body_masses": [10.0]}},
        {"aggregate": {"collision_layers": [2, 3], "quality_ids": [4]}},
    ]
    inv = aggregate_invariants(rows)
    assert inv["layers"] == [1, 2, 3]
    assert inv["quality_ids"] == [4]
    # Geometry thresholds are emitted so a re-mine doesn't revert them to code defaults.
    assert inv["thin_hull_ratio"] == 1e-5
    assert inv["degenerate_extent_eps"] == 1e-4

    findings = [
        {"rule_id": "degenerate_hull_coplanar", "severity": "error"},
        {"rule_id": "layer_outside_vanilla_domain", "severity": "warning"},
        {"rule_id": "degenerate_hull_coplanar", "severity": "error"},
    ]
    summary = summarize_violations(findings)
    assert summary["errors"] == 2
    assert summary["warnings"] == 1
    assert summary["by_rule"]["degenerate_hull_coplanar"] == 2
