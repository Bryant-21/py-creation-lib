"""End-to-end integration of cubemap_heuristics into BGSM/BGEM downgrade.

The unit-level coverage of select_cubemap is in test_cubemap_heuristics.py.
This file verifies that downgrade_bgsm and downgrade_bgem actually call the
heuristic and propagate the result onto the right output fields.
"""
from __future__ import annotations

import pytest

from creation_lib.material_tools.convert import (
    BGSM_VERSION_FO4,
    BGEM_VERSION_FO4,
    downgrade_bgsm,
    downgrade_bgem,
)
from .test_convert_py_regression import _default_bgsm_v20, _default_bgem_v22


def _load_bgsm_v20():
    return _default_bgsm_v20()


# ----------------------------------------------------------------- BGSM
@pytest.mark.parametrize(
    ("source_path", "diffuse_override", "expected_envmap", "expected_env_mapping"),
    [
        (
            "materials/weapons/meltdown/MBody.bgsm",
            None,
            "Shared/Cubemaps/mipblur_DefaultOutside1.dds",
            True,
        ),
        (
            # Chrome keyword in the diffuse texture overrides the path heuristic.
            "materials/weapons/foo/x.bgsm",
            "textures/weapons/foo/Chrome_d.dds",
            "Shared/Cubemaps/MetalChrome01Cube_e.dds",
            None,
        ),
        ("materials/effects/blood01.bgsm", None, "", False),
        (
            # When source_path is empty/None, the heuristic still runs and the
            # fallback returns mipblur_DefaultOutside1 — never leaves the slot
            # empty (current convert.py contract: BGSM downgrade always
            # populates Envmap unless the path is in an excluded category).
            None,
            None,
            "Shared/Cubemaps/mipblur_DefaultOutside1.dds",
            True,
        ),
    ],
)
def test_downgrade_bgsm_envmap_heuristic(source_path, diffuse_override, expected_envmap, expected_env_mapping):
    data = _load_bgsm_v20()
    if diffuse_override is not None:
        data.DiffuseTexture = diffuse_override
    kwargs = {"source_path": source_path} if source_path is not None else {}
    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, **kwargs)
    assert fo4.EnvmapTexture == expected_envmap
    if expected_env_mapping is not None:
        assert fo4.header.env_mapping is expected_env_mapping
    if expected_env_mapping is True:
        assert fo4.header.env_mapping_mask_scale == 1.0


# ----------------------------------------------------------------- BGEM
@pytest.mark.parametrize(
    ("initial_envmap", "source_path", "expected_envmap", "expected_mapping"),
    [
        # Empty EnvmapTexture + EnvironmentMapping default-off on a weapons
        # path: the heuristic populates both.
        ("", "materials/weapons/foo/x.bgem", "Shared/Cubemaps/mipblur_DefaultOutside1.dds", True),
        # Empty EnvmapTexture on an effects path: heuristic returns (None,
        # None), so EnvironmentMapping is left unchanged (still off).
        ("", "materials/effects/blood01.bgem", "", False),
        # A source BGEM that already names a cubemap must NOT be overwritten
        # — the heuristic only populates when the slot is empty. (Whatever
        # EnvironmentMapping the heuristic derives from the weapons path is
        # not this case's concern, so it's left unchecked.)
        ("Shared/Cubemaps/MyCustomCube.dds", "materials/weapons/foo/x.bgem", "Shared/Cubemaps/MyCustomCube.dds", None),
    ],
)
def test_downgrade_bgem_envmap_heuristic(initial_envmap, source_path, expected_envmap, expected_mapping):
    data = _default_bgem_v22()
    data.EnvmapTexture = initial_envmap
    data.EnvironmentMapping = False
    data.EnvironmentMappingMaskScale = 0.0

    fo4 = downgrade_bgem(data, BGEM_VERSION_FO4, source_path=source_path)

    assert fo4.EnvmapTexture == expected_envmap
    if expected_mapping is not None:
        assert fo4.EnvironmentMapping is expected_mapping
    if expected_mapping is True:
        assert fo4.EnvironmentMappingMaskScale == 1.0
