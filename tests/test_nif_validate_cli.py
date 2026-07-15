from creation_lib.nif.validation import validate_nif
from creation_lib.nif.nif_file import NifBlock, NifFile


def test_validate_nif_accepts_non_shape_root():
    nif = NifFile()
    nif.blocks.append(NifBlock(0, "NiNode", fields=[("Name", "FXRoot")]))

    report = validate_nif(nif)

    assert report["valid"] is True
    assert report["error_count"] == 0
    assert "expected BSTriShape" not in str(report)


def test_validate_nif_reports_out_of_range_refs():
    nif = NifFile()
    nif.blocks.append(NifBlock(0, "NiNode", fields=[
        ("Name", "Root"),
        ("Children", [99]),
    ]))

    report = validate_nif(nif)

    assert report["valid"] is False
    assert report["error_count"] == 1
    assert report["errors"][0]["field"] == "Children"
    assert "99" in report["errors"][0]["message"]


def test_validate_nif_warns_for_inline_effect_without_source_texture():
    nif = NifFile()
    nif.blocks.append(NifBlock(0, "BSEffectShaderProperty", fields=[
        ("Name", ""),
        ("Source Texture", ""),
        ("Env Map Texture", ""),
        ("Shader Flags 1", 0),
    ]))

    report = validate_nif(nif)

    assert report["valid"] is True
    assert any(
        warning.get("field") == "Source Texture"
        and "inline BSEffectShaderProperty" in warning["message"]
        for warning in report["warnings"]
    )
