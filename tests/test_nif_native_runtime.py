from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace

import pytest

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


def test_convert_nif_file_raw_handles_fnv_geometry_shader_collision_fixture(
    tmp_path: Path,
) -> None:
    from creation_lib.nif.nif_file import NifFile

    src = Path(
        "bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/nif/fnv/weapons/gaussrifle.nif"
    )
    dst = tmp_path / "converted.nif"

    report = native_runtime.convert_nif_file_raw(
        str(src),
        str(dst),
        "fnv",
        "fo4",
        None,
        {"asset_prefix": "fnv"},
    )

    assert report["supported"] is True
    assert any("NiTriStrips -> BSTriShape" in change for change in report["changes"])
    assert any("Legacy shader properties" in change for change in report["changes"])
    assert any("Legacy collision" in change for change in report["changes"])
    converted = NifFile.load(str(dst))
    block_types = {block.type_name for block in converted.blocks}
    assert "BSTriShape" in block_types
    assert "BSLightingShaderProperty" in block_types
    assert "NiTriStrips" not in block_types
    assert "BSShaderPPLightingProperty" not in block_types


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
    assert textures[2] == "textures\\weapons\\rifle_l.dds"
    assert textures[7] == "textures\\weapons\\rifle_r.dds"


def test_convert_nif_file_raw_preserves_fo76_stair_compressed_collision(
    tmp_path: Path,
) -> None:
    from creation_lib.havok.native_runtime import collision_preview_native
    from creation_lib.nif.nif_file import NifFile

    src = Path(
        "extracted/fo76/meshes/hardscape/unique/hard_unique_vault76_stairslg01.nif"
    )
    if not src.exists():
        pytest.skip("FO76 Vault 76 stair fixture is not available")

    def root_collision_preview(path: Path) -> list[dict]:
        nif = NifFile.load(str(path))
        collision = next(
            block
            for block in nif.blocks
            if block.type_name == "bhkNPCollisionObject"
            and block.get_field("Target") == 0
        )
        physics = nif.get_block(collision.get_field("Data"))
        blob = bytes(physics.get_field("Binary Data")["Data"])
        return collision_preview_native(blob, 69.99125, int(collision.get_field("Body ID") or 0))[
            "meshes"
        ]

    def triangle_indices(triangle: dict | list) -> list[int]:
        if isinstance(triangle, dict):
            return [int(triangle["v1"]), int(triangle["v2"]), int(triangle["v3"])]
        return [int(index) for index in triangle]

    def valid_triangle_count(preview: list[dict]) -> int:
        total = 0
        for mesh in preview:
            vertices = mesh["mesh"]["vertices"]
            for triangle in mesh["mesh"]["triangles"]:
                indices = triangle_indices(triangle)
                if all(0 <= index < len(vertices) for index in indices):
                    total += 1
        return total

    def vertex_tuple(vertex: dict | list) -> tuple[int, int, int]:
        if isinstance(vertex, dict):
            xyz = (vertex["x"], vertex["y"], vertex["z"])
        else:
            xyz = vertex
        return tuple(round(float(value) * 1000) for value in xyz)

    def triangle_signature(preview: list[dict]) -> list[tuple[tuple[int, int, int], ...]]:
        signatures = []
        for mesh in preview:
            vertices = mesh["mesh"]["vertices"]
            for triangle in mesh["mesh"]["triangles"]:
                indices = triangle_indices(triangle)
                if not all(0 <= index < len(vertices) for index in indices):
                    continue
                signatures.append(tuple(sorted(vertex_tuple(vertices[index]) for index in indices)))
        return sorted(signatures)

    source_preview = root_collision_preview(src)
    source_raw_triangles = sum(len(mesh["mesh"]["triangles"]) for mesh in source_preview)
    source_triangles = valid_triangle_count(source_preview)
    assert source_triangles > 128
    assert source_triangles == source_raw_triangles
    source_signature = triangle_signature(source_preview)

    dst = tmp_path / "converted.nif"
    report = native_runtime.convert_nif_file_raw(
        str(src),
        str(dst),
        "fo76",
        "fo4",
        None,
        {"asset_prefix": "fo76"},
    )

    assert report["supported"] is True
    converted_preview = root_collision_preview(dst)
    converted_triangles = valid_triangle_count(converted_preview)
    assert converted_triangles == source_triangles
    assert {mesh["shape_type"] for mesh in converted_preview} == {"compressed_mesh"}
    assert triangle_signature(converted_preview) == source_signature
