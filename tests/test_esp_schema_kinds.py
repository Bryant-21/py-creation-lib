from creation_lib.esp.schema.kinds import FieldKind


def test_field_kind_values():
    assert FieldKind.RAW.value == "raw"
    assert FieldKind.PARSED.value == "parsed"
    assert FieldKind.PARSED_WITH_RAW_FALLBACK.value == "parsed_with_raw_fallback"


def test_field_kind_exhaustive():
    assert {k.value for k in FieldKind} == {
        "raw", "parsed", "parsed_with_raw_fallback",
    }
