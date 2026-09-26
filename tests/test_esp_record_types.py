from __future__ import annotations

import pytest

from creation_lib.esp.record_types import (
    record_type_display_label,
    record_type_signature,
)


@pytest.mark.parametrize(
    ("name", "expected"),
    [("Weapons", "WEAP"), ("WEAP", "WEAP"), ("npc_", "npc_"), ("Race", "Race")],
    ids=["legacy-alias", "signature-passthrough", "already-four-chars", "unknown-falls-back-to-name"],
)
def test_record_type_signature(name: str, expected: str) -> None:
    assert record_type_signature(name) == expected


@pytest.mark.parametrize(
    ("name", "expected"),
    [("Weapons", "Weapon"), ("MadeUpRecords", "MadeUpRecords")],
    ids=["alias-uses-schema-label", "unknown-falls-back-to-name"],
)
def test_record_type_display_label(name: str, expected: str) -> None:
    assert record_type_display_label(name, "fo4") == expected
