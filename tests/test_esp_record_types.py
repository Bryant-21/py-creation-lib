from __future__ import annotations

from creation_lib.esp.record_types import (
    record_type_display_label,
    record_type_signature,
)


def test_record_type_signature_accepts_legacy_aliases() -> None:
    assert record_type_signature("Weapons") == "WEAP"
    assert record_type_signature("Ammo") == "AMMO"
    assert record_type_signature("LeveledNpcs") == "LVLN"


def test_record_type_signature_preserves_signature_codes() -> None:
    assert record_type_signature("WEAP") == "WEAP"
    assert record_type_signature("npc_") == "npc_"


def test_record_type_signature_falls_back_to_raw_name_for_unknown_four_char_name() -> None:
    assert record_type_signature("Race") == "Race"


def test_record_type_display_label_uses_schema_label_for_alias() -> None:
    assert record_type_display_label("Weapons", "fo4") == "Weapon"
    assert record_type_display_label("Ammo", "fo4") == "Ammunition"


def test_record_type_display_label_falls_back_to_raw_name() -> None:
    assert record_type_display_label("MadeUpRecords", "fo4") == "MadeUpRecords"
