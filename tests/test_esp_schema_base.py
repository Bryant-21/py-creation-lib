import pytest

from creation_lib.esp.schema.base import (
    ArraySpec,
    ConditionSpec,
    EnumDef,
    FieldSpec,
    GameSchema,
    RecordSpec,
    SubrecordSpec,
    TargetMapEntry,
    UnionVariantSpec,
)
from creation_lib.esp.schema.kinds import FieldKind


def _edid_spec() -> SubrecordSpec:
    return SubrecordSpec(sig="EDID", kind=FieldKind.PARSED, codec="zstring")


def test_subrecord_spec_defaults():
    s = _edid_spec()
    assert s.sig == "EDID"
    assert s.kind is FieldKind.PARSED
    assert s.display_label is None
    assert s.codec == "zstring"
    assert s.repeatable is False
    assert s.required is False
    assert s.localized is False
    assert s.fields is None
    assert s.enum_ref is None
    assert s.formlink_target is None
    assert s.enum is None
    assert s.flags is None
    assert s.struct_layout is None
    assert s.presence_conditions == ()
    assert s.union_selector is None
    assert s.union_variants == ()
    assert s.array is None
    assert s.row_label is None
    assert s.authoring_layout is None
    assert s.authoring_key is None
    assert s.notes == ""


def test_subrecord_spec_frozen():
    s = _edid_spec()
    with pytest.raises(Exception):
        s.sig = "BOGUS"  # type: ignore[misc]


def test_record_spec_construction():
    rec = RecordSpec(
        sig="KYWD",
        subrecords=(_edid_spec(),),
        display_label="Keyword",
        order_hint=("EDID",),
    )
    assert rec.sig == "KYWD"
    assert rec.subrecords[0].sig == "EDID"
    assert rec.display_label == "Keyword"
    assert rec.order_hint == ("EDID",)


def test_game_schema_construction():
    rec = RecordSpec(sig="KYWD", subrecords=(_edid_spec(),))
    gs = GameSchema(
        game="fo4",
        records={"KYWD": rec},
        header_version=0.95,
        localized_support=True,
    )
    assert gs.game == "fo4"
    assert gs.records["KYWD"] is rec
    assert gs.header_version == 0.95
    assert gs.localized_support is True
    assert gs.enums == {}


def test_field_and_enum_construction():
    field = FieldSpec(
        name="Ammo",
        kind="formid",
        formlink_target="AMMO",
    )
    enum_def = EnumDef(name="Weapon.HitBehavior", byte_width=4)
    assert field.formlink_target == "AMMO"
    assert enum_def.byte_width == 4


def test_enum_def_stability_and_alias_resolution():
    enum_def = EnumDef(
        name="Weapon.HitBehavior",
        values=((0, "default"), (1, "explode")),
        labels=((1, "Explode"),),
        aliases=(("legacy_default", "default"),),
        scope="record",
        storage_kind="enum",
        byte_width=2,
        notes="synthetic enum",
    )

    assert enum_def.token_for_value(0) == "default"
    assert enum_def.token_for_value(99) is None
    assert enum_def.label_for_value(1) == "Explode"
    assert enum_def.label_for_value(0) == "default"
    assert enum_def.resolve_token(" legacy_default ") == "default"
    assert enum_def.resolve_token("explode") == "explode"
    assert enum_def.scope == "record"
    assert enum_def.storage_kind == "enum"
    assert enum_def.byte_width == 2


def test_richer_schema_primitives_capture_nested_layouts_and_targets():
    array = ArraySpec(
        layout="array_struct",
        element_kind="struct",
        element_codec="I,f",
        count_field="count",
        notes="row array",
    )
    condition = ConditionSpec(field="mode", operator="in", values=("A", "B"))
    target_entry = TargetMapEntry(selector="form_type", value="weapon", target="WEAP")
    nested_field = FieldSpec(name="damage", kind="uint16")
    union_variant = UnionVariantSpec(
        name="Primary",
        codec="struct:I",
        fields=(FieldSpec(name="value", kind="uint32"),),
        conditions=(ConditionSpec(field="enabled", operator="eq", value=True),),
        notes="primary branch",
    )
    field = FieldSpec(
        name="payload",
        kind="struct",
        target_map=(target_entry,),
        presence_conditions=(condition,),
        nested_fields=(nested_field,),
        array=array,
        authoring_label="Payload",
        notes="rich field",
    )
    subrecord = SubrecordSpec(
        sig="DNAM",
        kind=FieldKind.PARSED_WITH_RAW_FALLBACK,
        display_label="Data",
        codec="struct:I,f",
        fields=(field,),
        presence_conditions=(condition,),
        union_selector="kind",
        union_variants=(union_variant,),
        array=array,
        row_label="row",
        authoring_layout="table",
        authoring_key="id",
        notes="rich subrecord",
    )

    assert field.target_map == (target_entry,)
    assert field.presence_conditions == (condition,)
    assert field.nested_fields == (nested_field,)
    assert field.array == array
    assert field.authoring_label == "Payload"
    assert subrecord.fields == (field,)
    assert subrecord.display_label == "Data"
    assert subrecord.presence_conditions == (condition,)
    assert subrecord.union_selector == "kind"
    assert subrecord.union_variants == (union_variant,)
    assert subrecord.array == array
    assert subrecord.row_label == "row"
    assert subrecord.authoring_layout == "table"
    assert subrecord.authoring_key == "id"


def test_game_schema_can_store_rich_enum_metadata():
    enum_def = EnumDef(
        name="Weapon.HitBehavior",
        values=((0, "default"), (1, "explode")),
        labels=((1, "Explode"),),
        byte_width=4,
    )
    gs = GameSchema(
        game="fo4",
        records={"KYWD": RecordSpec(sig="KYWD", subrecords=(_edid_spec(),))},
        header_version=0.95,
        localized_support=True,
        enums={"Weapon.HitBehavior": enum_def},
    )

    assert gs.enums["Weapon.HitBehavior"] is enum_def
    assert gs.enums["Weapon.HitBehavior"].token_for_value(1) == "explode"
