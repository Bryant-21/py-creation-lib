from __future__ import annotations

import json

import pytest

from creation_lib.lod.default_settings import DEFAULT_SETTINGS_JSON, fo4_default_settings


def test_fo4_defaults_shape_and_json_roundtrip():
    s = fo4_default_settings()
    assert set(s) == {"global", "terrain", "objects", "trees", "grass"}
    assert s["global"]["lod_min"] == 4
    assert s["global"]["lod_max"] == 32
    assert len(s["terrain"]["levels"]) == 4
    # All four terrain levels use 256x256 tiles; only L4 (index 0) mipmaps
    # diffuse; normal tiles never mipmap.
    for i, lvl in enumerate(s["terrain"]["levels"]):
        assert lvl["diffuse_size"] == 256
        assert lvl["normal_size"] == 256
        assert lvl["diffuse_mipmap"] == (i == 0)
        assert not lvl["normal_mipmap"]
    assert s["terrain"]["default_diffuse_size"] == 128
    assert s["terrain"]["default_normal_size"] == 128
    assert s["objects"]["atlas_size"] == 4096
    assert s["trees"]["trees_3d"] is True

    assert json.loads(DEFAULT_SETTINGS_JSON) == s


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
