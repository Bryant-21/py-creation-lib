"""Terrain archive family (upgrade-gen Task 1). Cases sourced from the
verified audit (docs/superpowers/specs/appalachia_family_map_verified.md
§2c) — exercised against paths from the deployed Appalachia tree."""
from __future__ import annotations

import pytest

from creation_lib.build.archive_plan import classify_archive_family

CASES = [
    # convert_terrain full-res tiles + materials -> Terrain.
    ("Textures/Terrain/Appalachia/lswamprocks01_g.dds", "Terrain"),
    ("Materials/Terrain/Appalachia/blend.bgsm", "Terrain"),
    ("Terrain/Appalachia.btd4", "Terrain"),
    # lodgen terrain-LOD quad tiles -> LOD, not Terrain.
    ("Textures/Terrain/Appalachia/appalachia.16.-110.-77.dds", "LOD"),
    ("Textures/Terrain/Appalachia/appalachia.16.-110.-77_msn.dds", "LOD"),
    # lodgen object atlas -> LOD.
    ("Textures/Terrain/Appalachia/Objects/hybrid/lod/x_lod_0_d.dds", "LOD"),
    # existing .bto rule unaffected.
    ("Meshes/Terrain/Appalachia/Objects/App.4.0.0.bto", "LOD"),
    # object textures/materials unaffected.
    ("Textures/Weapons/gun_d.dds", "Textures"),
    ("Materials/Weapons/gun.bgsm", "Materials"),
]


@pytest.mark.parametrize("path,expected", CASES)
def test_terrain_family(path: str, expected: str):
    assert classify_archive_family(path) == expected
