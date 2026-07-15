from creation_lib.nif.schema import FieldDef, NiObjectType, NifSchema, StructType


def test_schema_memoizes_inherited_field_and_type_lookups() -> None:
    schema = NifSchema()
    schema.structs["SimpleStruct"] = StructType(
        name="SimpleStruct",
        fields=[FieldDef(name="StructField", type="uint")],
    )
    schema.niobjects["Base"] = NiObjectType(
        name="Base",
        inherit=None,
        fields=[FieldDef(name="BaseField", type="uint")],
    )
    schema.niobjects["Mid"] = NiObjectType(
        name="Mid",
        inherit="Base",
        fields=[FieldDef(name="MidField", type="uint")],
    )
    schema.niobjects["Leaf"] = NiObjectType(
        name="Leaf",
        inherit="Mid",
        fields=[FieldDef(name="LeafField", type="uint")],
    )

    first_fields = schema.get_all_fields("Leaf")
    second_fields = schema.get_all_fields("Leaf")
    assert first_fields is second_fields
    assert [field.name for field in first_fields] == [
        "BaseField",
        "MidField",
        "LeafField",
    ]

    first_hierarchy = schema.get_type_hierarchy("Leaf")
    second_hierarchy = schema.get_type_hierarchy("Leaf")
    assert first_hierarchy is second_hierarchy
    assert first_hierarchy == ("Leaf", "Mid", "Base")

    assert schema.is_subtype_of("Leaf", "Base") is True
    assert schema.is_subtype_of("Leaf", "Base") is True
    assert schema.is_subtype_of("Leaf", "SimpleStruct") is False

    assert schema._all_fields_cache["Leaf"] is first_fields
    assert schema._type_hierarchy_cache["Leaf"] is first_hierarchy
    assert schema._is_subtype_cache[("Leaf", "Base")] is True
    assert schema._is_subtype_cache[("Leaf", "SimpleStruct")] is False


def test_schema_returns_cached_empty_results_for_unknown_types() -> None:
    schema = NifSchema()

    fields = schema.get_all_fields("Missing")
    hierarchy = schema.get_type_hierarchy("Missing")

    assert fields == ()
    assert hierarchy == ("Missing",)
    assert schema.is_subtype_of("Missing", "Base") is False
    assert schema._all_fields_cache["Missing"] == ()
    assert schema._type_hierarchy_cache["Missing"] == ("Missing",)
