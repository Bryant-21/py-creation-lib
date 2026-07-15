import json

from creation_lib.havok.native_runtime import validate_collision_blob_native

from scripts.mine_collision_invariants import aggregate_invariants

from pathlib import Path

from creation_lib.havok.collision_validation import summarize_violations


def test_validate_raises_on_unparseable_blob():
    # Garbage bytes are not a parseable Havok packfile: the native wrapper surfaces
    # an exception rather than crashing the interpreter. The Python pass catches this
    # and turns it into a finding (report-only), so the raise here is the contract.
    import pytest

    with pytest.raises(Exception):
        validate_collision_blob_native(b"NOT_A_HAVOK_BLOB", "{}")


def test_aggregate_invariants_unions_domains():
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


def test_summarize_counts_by_severity_and_rule():
    findings = [
        {"rule_id": "degenerate_hull_coplanar", "severity": "error"},
        {"rule_id": "layer_outside_vanilla_domain", "severity": "warning"},
        {"rule_id": "degenerate_hull_coplanar", "severity": "error"},
    ]
    summary = summarize_violations(findings)
    assert summary["errors"] == 2
    assert summary["warnings"] == 1
    assert summary["by_rule"]["degenerate_hull_coplanar"] == 2
