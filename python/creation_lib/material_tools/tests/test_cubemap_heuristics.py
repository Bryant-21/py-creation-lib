"""Table-driven coverage of every cubemap_heuristics branch.

The heuristic is the single source of truth for FO4 EnvmapTexture selection
during FO76->FO4 BGSM/BGEM downgrade. Each row exercises one branch in the
order the function checks them so a regression in priority shows up clearly.
"""
from __future__ import annotations

from dataclasses import dataclass

import pytest

from creation_lib.material_tools.cubemap_heuristics import select_cubemap


@dataclass
class FakeMat:
    DiffuseTexture: str = ""
    NormalTexture: str = ""
    SmoothSpecTexture: str = ""
    SpecularTexture: str = ""
    GlowTexture: str = ""
    BaseTexture: str = ""
    EnvmapMaskTexture: str = ""
    SkinTint: bool = False
    Hair: bool = False
    RootMaterialPath: str = ""


@pytest.mark.parametrize("path", [
    "materials/effects/blood/blood01.bgsm",
    "effects/blood01.bgem",
    "materials/interface/lockpicking/lock.bgsm",
    "materials/menu/main.bgsm",
    "materials/sky/clouds01.bgsm",
    "materials/decals/blood/decal01.bgsm",
])
def test_excluded_paths_return_none(path: str) -> None:
    cubemap, scale = select_cubemap(path, FakeMat())
    assert cubemap is None
    assert scale is None


@pytest.mark.parametrize(("mat_kwargs", "src_path", "expected_cubemap", "expected_scale"), [
    ({"DiffuseTexture": "textures/weapons/foo/Chrome_d.dds"}, "materials/weapons/foo/foo.bgsm",
     "Shared/Cubemaps/MetalChrome01Cube_e.dds", 1.0),
    ({"DiffuseTexture": "weapons/gauss/copperReceiver_d.dds"}, "materials/weapons/gauss/x.bgsm",
     "Shared/Cubemaps/MetalCopperShine01Cube_e.dds", 1.0),
    ({"NormalTexture": "actors/bug/bronzeArmor_n.dds"}, "materials/actors/bug/x.bgsm",
     "Shared/Cubemaps/MetalBronzeCube_e.dds", None),
    ({"DiffuseTexture": "props/Gold_d.dds"}, "materials/props/x.bgsm",
     "Shared/Cubemaps/MetalBrushedGold_e.dds", None),
    ({"DiffuseTexture": "props/brushedSteel_d.dds"}, "materials/props/x.bgsm",
     "Shared/Cubemaps/MetalBrushed01Cube_e.dds", None),
    ({"DiffuseTexture": "setdressing/glassBottle_d.dds"}, "materials/setdressing/x.bgsm",
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 0.5),
    ({"DiffuseTexture": "actors/cat/eye_d.dds"}, "materials/actors/cat/eye.bgsm",
     "Shared/Cubemaps/EyeCubeMap.dds", None),
    ({"DiffuseTexture": "setdressing/oilSpill_d.dds"}, "materials/setdressing/x.bgsm",
     "Shared/Cubemaps/Oil_e.dds", None),
    ({"SpecularTexture": "weapons/gauss/copperish_s.dds"}, "materials/weapons/gauss/x.bgsm",
     "Shared/Cubemaps/MetalCopperShine01Cube_e.dds", None),
    # Priority: exclusion beats keyword — even with a chrome diffuse, an
    # effects path returns None.
    ({"DiffuseTexture": "effects/Chrome_d.dds"}, "materials/effects/x.bgsm", None, None),
    # Priority: keyword beats path — a weapons path normally returns
    # mipblur_DefaultOutside1, but a chrome diffuse overrides it.
    ({"DiffuseTexture": "weapons/foo/Chrome_d.dds"}, "materials/weapons/foo/x.bgsm",
     "Shared/Cubemaps/MetalChrome01Cube_e.dds", None),
])
def test_texture_keyword_branch(mat_kwargs, src_path, expected_cubemap, expected_scale) -> None:
    mat = FakeMat(**mat_kwargs)
    cubemap, scale = select_cubemap(src_path, mat)
    assert cubemap == expected_cubemap
    if expected_cubemap is None:
        assert scale is None
    elif expected_scale is not None:
        assert scale == expected_scale


@pytest.mark.parametrize(("src_path", "mat_kwargs", "expected_cubemap", "expected_scale"), [
    ("materials/weapons/meltdown/MBody.bgsm", {"DiffuseTexture": "textures/weapons/meltdown/body_d.dds"},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 1.0),
    ("materials/weapons/10mmpistol/10mmRubberGrips.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", 0.3),
    ("materials/atx/weapons/paint01/foo.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 1.0),
    ("materials/actors/character/Body.bgsm", {"SkinTint": True},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", 0.3),
    ("materials/actors/dog/dog.bgsm", {"RootMaterialPath": "template/CreatureTemplate_Wet.bgsm"},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", 0.3),
    ("materials/architecture/MetalRoof01.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 1.0),
    ("materials/architecture/Brickwall01.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", 0.3),
    ("materials/clothes/Bathrobe/bathrobe.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", None),
    ("materials/armor/Metal/MetalArmor.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", None),
    ("materials/armor/LeatherCoat/leather.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds", None),
    ("materials/vehicles/Car01.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", None),
    ("materials/ammo/10mm/cartridge.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", None),
    ("materials/unknownThing/foo.bgsm", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 1.0),
    ("", {},
     "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 1.0),
])
def test_path_driven_default(src_path, mat_kwargs, expected_cubemap, expected_scale) -> None:
    cubemap, scale = select_cubemap(src_path, FakeMat(**mat_kwargs))
    assert cubemap == expected_cubemap
    if expected_scale is not None:
        assert scale == expected_scale


