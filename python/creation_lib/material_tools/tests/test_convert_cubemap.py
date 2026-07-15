"""End-to-end integration of cubemap_heuristics into BGSM/BGEM downgrade.

The unit-level coverage of select_cubemap is in test_cubemap_heuristics.py.
This file verifies that downgrade_bgsm and downgrade_bgem actually call the
heuristic and propagate the result onto the right output fields.
"""
from __future__ import annotations

import copy
import io
from pathlib import Path

import pytest

from creation_lib.material_tools.bgsm_bin import read_bgsm
from creation_lib.material_tools.bgem_bin import read_bgem
from creation_lib.material_tools.convert import (
    BGSM_VERSION_FO4,
    BGEM_VERSION_FO4,
    downgrade_bgsm,
    downgrade_bgem,
)

FIXTURE_DIR = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "materials"
)
BGSM_FIXTURE = FIXTURE_DIR / "sample_v22.bgsm"
BGEM_FIXTURE = FIXTURE_DIR / "sample_v22.bgem"


pytestmark = pytest.mark.skipif(
    not BGSM_FIXTURE.exists() or not BGEM_FIXTURE.exists(),
    reason="FO76 BGSM/BGEM fixtures missing",
)


def _load_bgsm_v20():
    data = read_bgsm(io.BytesIO(BGSM_FIXTURE.read_bytes()))
    clone = copy.deepcopy(data)
    clone.header.version = 20
    return clone


# ----------------------------------------------------------------- BGSM
def test_downgrade_bgsm_weapons_path_gets_outside_cubemap():
    data = _load_bgsm_v20()
    fo4 = downgrade_bgsm(
        data, BGSM_VERSION_FO4, source_path="materials/weapons/meltdown/MBody.bgsm"
    )
    assert fo4.EnvmapTexture == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
    assert fo4.header.env_mapping is True
    assert fo4.header.env_mapping_mask_scale == 1.0


def test_downgrade_bgsm_chrome_keyword_overrides_path():
    data = _load_bgsm_v20()
    data.DiffuseTexture = "textures/weapons/foo/Chrome_d.dds"
    fo4 = downgrade_bgsm(
        data, BGSM_VERSION_FO4, source_path="materials/weapons/foo/x.bgsm"
    )
    assert fo4.EnvmapTexture == "Shared/Cubemaps/MetalChrome01Cube_e.dds"


def test_downgrade_bgsm_effects_path_leaves_envmap_empty():
    data = _load_bgsm_v20()
    fo4 = downgrade_bgsm(
        data, BGSM_VERSION_FO4, source_path="materials/effects/blood01.bgsm"
    )
    assert fo4.EnvmapTexture == ""
    assert fo4.header.env_mapping is False


def test_downgrade_bgsm_no_source_path_falls_back_to_default():
    """When source_path is empty/None, the heuristic still runs and the
    fallback returns mipblur_DefaultOutside1 — never leaves the slot empty
    (current convert.py contract: BGSM downgrade always populates Envmap
    unless the path is in an excluded category)."""
    data = _load_bgsm_v20()
    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4)
    assert fo4.EnvmapTexture == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
    assert fo4.header.env_mapping is True


# ----------------------------------------------------------------- BGEM
def test_downgrade_bgem_weapons_path_sets_cubemap_and_mapping():
    data = read_bgem(io.BytesIO(BGEM_FIXTURE.read_bytes()))
    # Force EnvmapTexture empty + EnvironmentMapping default-off so we can see
    # the heuristic populate them.
    data.EnvmapTexture = ""
    data.EnvironmentMapping = False
    data.EnvironmentMappingMaskScale = 0.0

    fo4 = downgrade_bgem(
        data, BGEM_VERSION_FO4, source_path="materials/weapons/foo/x.bgem"
    )

    assert fo4.EnvmapTexture == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
    assert fo4.EnvironmentMapping is True
    assert fo4.EnvironmentMappingMaskScale == 1.0


def test_downgrade_bgem_effects_path_leaves_envmap_alone():
    data = read_bgem(io.BytesIO(BGEM_FIXTURE.read_bytes()))
    data.EnvmapTexture = ""
    data.EnvironmentMapping = False

    fo4 = downgrade_bgem(
        data, BGEM_VERSION_FO4, source_path="materials/effects/blood01.bgem"
    )

    assert fo4.EnvmapTexture == ""
    # EnvironmentMapping unchanged because heuristic returned (None, None).
    assert fo4.EnvironmentMapping is False


def test_downgrade_bgem_preserves_existing_envmap_value():
    """If the source BGEM already named a cubemap, the heuristic must NOT
    overwrite it — only populates when the slot is empty."""
    data = read_bgem(io.BytesIO(BGEM_FIXTURE.read_bytes()))
    data.EnvmapTexture = "Shared/Cubemaps/MyCustomCube.dds"

    fo4 = downgrade_bgem(
        data, BGEM_VERSION_FO4, source_path="materials/weapons/foo/x.bgem"
    )

    assert fo4.EnvmapTexture == "Shared/Cubemaps/MyCustomCube.dds"
