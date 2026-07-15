from __future__ import annotations

import json

import pytest

from creation_lib.lod.default_settings import DEFAULT_SETTINGS_JSON, fo4_default_settings


def test_fo4_defaults_shape():
    s = fo4_default_settings()
    assert set(s) == {"global", "terrain", "objects", "trees"}
    assert s["global"]["lod_min"] == 4
    assert s["global"]["lod_max"] == 32
    assert s["global"]["southwest_cell"] is None
    assert s["global"]["bounds"] is None
    assert len(s["terrain"]["levels"]) == 4
    assert [lvl["quality"] for lvl in s["terrain"]["levels"]] == [10.0, 15.0, 20.0, 25.0]
    assert s["objects"]["atlas_size"] == 4096
    assert s["objects"]["atlas_mip_flooding"] is False
    assert s["objects"]["uv_range"] == 1.5
    assert s["objects"]["alpha_threshold"] == 128
    assert s["trees"]["trees_3d"] is True


def test_terrain_diffuse_size_256_at_every_level():
    # fo4_default(): all four terrain levels use 256x256 BC1 tiles.
    s = fo4_default_settings()
    sizes = [lvl["diffuse_size"] for lvl in s["terrain"]["levels"]]
    assert sizes == [256, 256, 256, 256], f"Expected [256,256,256,256], got {sizes}"


def test_terrain_normal_size_256_at_every_level():
    s = fo4_default_settings()
    sizes = [lvl["normal_size"] for lvl in s["terrain"]["levels"]]
    assert sizes == [256, 256, 256, 256], f"Expected [256,256,256,256], got {sizes}"


def test_diffuse_mipmap_only_on_l4():
    # L4 (index 0) has diffuse_mipmap=True; L8/L16/L32 do not.
    s = fo4_default_settings()
    mips = [lvl["diffuse_mipmap"] for lvl in s["terrain"]["levels"]]
    assert mips == [True, False, False, False], f"Expected [True,False,False,False], got {mips}"


def test_normal_mipmap_never():
    # _msn tiles are always single-mip across all levels.
    s = fo4_default_settings()
    for i, lvl in enumerate(s["terrain"]["levels"]):
        assert not lvl["normal_mipmap"], f"level[{i}] normal_mipmap should be False"


def test_default_diffuse_size_128():
    s = fo4_default_settings()
    assert s["terrain"]["default_diffuse_size"] == 128, (
        f"Expected 128, got {s['terrain']['default_diffuse_size']}"
    )


def test_default_normal_size_128():
    s = fo4_default_settings()
    assert s["terrain"]["default_normal_size"] == 128, (
        f"Expected 128, got {s['terrain']['default_normal_size']}"
    )


def test_default_settings_json_roundtrips():
    assert json.loads(DEFAULT_SETTINGS_JSON) == fo4_default_settings()


def test_native_matches_python_when_available():
    """When lodgen_native.default_settings_json is importable, the Python dict must equal it."""
    try:
        from creation_lib._native import lodgen_native  # type: ignore[import]
        native_fn = lodgen_native.default_settings_json
    except (ImportError, AttributeError):
        pytest.skip("lodgen_native.default_settings_json not available (pre-rebuild)")

    native_dict = json.loads(native_fn())
    python_dict = fo4_default_settings()
    assert native_dict == python_dict, (
        "Python fo4_default_settings() diverged from native default_settings_json()"
    )
