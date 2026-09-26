from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace

from creation_lib.nif import native_runtime


def test_convert_nif_file_raw_calls_native_module(monkeypatch) -> None:
    calls: dict[str, object] = {}

    def fake_convert_nif_file(
        src: str,
        dst: str,
        source_game: str,
        target_game: str,
        bgsm_output_dir: str | None,
        options: dict[str, object],
    ) -> dict[str, object]:
        calls["args"] = (
            src,
            dst,
            source_game,
            target_game,
            bgsm_output_dir,
            options,
        )
        return {"supported": True, "changes": ["ok"]}

    module = SimpleNamespace(
        load_nif=lambda _path: {},
        convert_nif_file=fake_convert_nif_file,
    )
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", module)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", True)

    result = native_runtime.convert_nif_file_raw(
        "in.nif",
        "out.nif",
        "fnv",
        "fo4",
        "Materials/fnv",
        {"source_path": "Meshes/test.nif"},
    )

    assert result == {"supported": True, "changes": ["ok"]}
    assert calls["args"] == (
        "in.nif",
        "out.nif",
        "fnv",
        "fo4",
        "Materials/fnv",
        {"source_path": "Meshes/test.nif"},
    )


def test_convert_nif_file_raw_exports_real_native_file_converter(tmp_path: Path) -> None:
    from creation_lib.nif.nif_file import NifFile

    src = tmp_path / "source.nif"
    dst = tmp_path / "out" / "converted.nif"
    NifFile.new("fnv").save(str(src))

    report = native_runtime.convert_nif_file_raw(
        str(src),
        str(dst),
        "fnv",
        "fo4",
        None,
        {"source_path": "Meshes/test.nif"},
    )

    assert report["supported"] is True
    assert dst.exists()
    converted = NifFile.load(str(dst))
    assert converted.header.user_version == 12
    assert converted.header.bs_version == 130


def test_convert_nif_file_raw_handles_fo76_inline_shader_slots(tmp_path: Path) -> None:
    from creation_lib.nif.nif_file import NifFile

    src = tmp_path / "source.nif"
    dst = tmp_path / "converted.nif"
    nif = NifFile.new("fo76")
    texset = nif.add_block(
        "BSShaderTextureSet",
        {
            "Num Textures": 11,
            "Textures": [
                "weapons/rifle_d.dds",
                "weapons/rifle_n.dds",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "weapons/rifle_r.dds",
                "weapons/rifle_l.dds",
            ],
        },
    )
    shader = nif.add_block(
        "BSLightingShaderProperty",
        {
            "Texture Set": texset.block_id,
            "Shader Property Data": {
                "Shader Type": 0,
                "Texture Set": texset.block_id,
                "Num SF1": 2,
                "SF1": [2893749418, 2262553490],
                "Num SF2": 0,
                "SF2": [],
                "Smoothness": 0.45,
            },
        },
    )
    nif.blocks[0].set_field("Num Children", 1)
    nif.blocks[0].set_field("Children", [shader.block_id])
    nif.save(str(src))

    report = native_runtime.convert_nif_file_raw(
        str(src),
        str(dst),
        "fo76",
        "fo4",
        None,
        {"asset_prefix": "fo76"},
    )

    assert report["supported"] is True
    assert any("FO76 texture slot remap" in change for change in report["changes"])
    converted = NifFile.load(str(dst))
    converted_texset = next(
        block for block in converted.blocks if block.type_name == "BSShaderTextureSet"
    )
    textures = converted_texset.get_field("Textures")
    assert len(textures) == 10
    # slot 10 (emissive) -> slot 2 (glow); slot 9 (reflectivity) -> slot 7 (specular);
    # texture paths are renormalized to the target slot's suffix and asset_prefix.
    assert textures[2] == "textures\\fo76\\weapons\\rifle_g.dds"
    assert textures[7] == "textures\\fo76\\weapons\\rifle_s.dds"


