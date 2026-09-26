import pytest

from creation_lib.esp.schema.base import EnumDef, SubrecordSpec
from creation_lib.esp.schema.kinds import FieldKind


def test_subrecord_spec_is_frozen():
    s = SubrecordSpec(sig="EDID", kind=FieldKind.PARSED, codec="zstring")
    with pytest.raises(Exception):
        s.sig = "BOGUS"  # type: ignore[misc]


def test_enum_def_resolves_tokens_labels_and_aliases():
    enum_def = EnumDef(
        name="Weapon.HitBehavior",
        values=((0, "default"), (1, "explode")),
        labels=((1, "Explode"),),
        aliases=(("legacy_default", "default"),),
        scope="record",
        storage_kind="enum",
        byte_width=2,
    )

    assert enum_def.token_for_value(0) == "default"
    assert enum_def.token_for_value(99) is None
    assert enum_def.label_for_value(1) == "Explode"
    assert enum_def.label_for_value(0) == "default"
    assert enum_def.resolve_token(" legacy_default ") == "default"
    assert enum_def.resolve_token("explode") == "explode"
