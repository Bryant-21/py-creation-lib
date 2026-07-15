"""Table-driven coverage of every cubemap_heuristics branch.

The heuristic is the single source of truth for FO4 EnvmapTexture selection
during FO76->FO4 BGSM/BGEM downgrade. Each row exercises one branch in the
order the function checks them so a regression in priority shows up clearly.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

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


# ---------------------------------------------------------------- exclusions
class TestPathExclusions:
    @pytest.mark.parametrize("path", [
        "materials/effects/blood/blood01.bgsm",
        "effects/blood01.bgem",
        "materials/interface/lockpicking/lock.bgsm",
        "materials/menu/main.bgsm",
        "materials/sky/clouds01.bgsm",
        "materials/decals/blood/decal01.bgsm",
    ])
    def test_excluded_paths_return_none(self, path: str) -> None:
        cubemap, scale = select_cubemap(path, FakeMat())
        assert cubemap is None
        assert scale is None


# ----------------------------------------------------- texture-keyword branch
class TestTextureKeywords:
    def test_chrome_keyword_overrides_path_default(self) -> None:
        # Even on a weapon path (which would default to mipblur), the chrome
        # keyword in the diffuse name picks the chrome cube.
        mat = FakeMat(DiffuseTexture="textures/weapons/foo/Chrome_d.dds")
        cubemap, scale = select_cubemap("materials/weapons/foo/foo.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalChrome01Cube_e.dds"
        assert scale == 1.0

    def test_copper_keyword(self) -> None:
        mat = FakeMat(DiffuseTexture="weapons/gauss/copperReceiver_d.dds")
        cubemap, scale = select_cubemap("materials/weapons/gauss/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalCopperShine01Cube_e.dds"
        assert scale == 1.0

    def test_bronze_keyword(self) -> None:
        mat = FakeMat(NormalTexture="actors/bug/bronzeArmor_n.dds")
        cubemap, scale = select_cubemap("materials/actors/bug/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalBronzeCube_e.dds"

    def test_gold_keyword(self) -> None:
        mat = FakeMat(DiffuseTexture="props/Gold_d.dds")
        cubemap, scale = select_cubemap("materials/props/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalBrushedGold_e.dds"

    def test_brushed_keyword(self) -> None:
        mat = FakeMat(DiffuseTexture="props/brushedSteel_d.dds")
        cubemap, scale = select_cubemap("materials/props/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalBrushed01Cube_e.dds"

    def test_glass_keyword_uses_lower_scale(self) -> None:
        mat = FakeMat(DiffuseTexture="setdressing/glassBottle_d.dds")
        cubemap, scale = select_cubemap("materials/setdressing/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 0.5

    def test_eye_keyword(self) -> None:
        mat = FakeMat(DiffuseTexture="actors/cat/eye_d.dds")
        cubemap, scale = select_cubemap("materials/actors/cat/eye.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/EyeCubeMap.dds"

    def test_oil_keyword(self) -> None:
        mat = FakeMat(DiffuseTexture="setdressing/oilSpill_d.dds")
        cubemap, scale = select_cubemap("materials/setdressing/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/Oil_e.dds"

    def test_keyword_in_specular_slot_also_matches(self) -> None:
        mat = FakeMat(SpecularTexture="weapons/gauss/copperish_s.dds")
        cubemap, scale = select_cubemap("materials/weapons/gauss/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalCopperShine01Cube_e.dds"


# ----------------------------------------------------- path-driven defaults
class TestPathDefaults:
    def test_weapons_path_uses_outside_cubemap(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/weapons/meltdown/MBody.bgsm",
            FakeMat(DiffuseTexture="textures/weapons/meltdown/body_d.dds"),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 1.0

    def test_weapons_with_grip_hint_uses_dielectric(self) -> None:
        # Rubber grips, wooden stocks, etc. should not look mirror-shiny.
        cubemap, scale = select_cubemap(
            "materials/weapons/10mmpistol/10mmRubberGrips.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"
        assert scale == 0.3

    def test_atx_weapons_path_treated_as_weapon(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/atx/weapons/paint01/foo.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 1.0

    def test_actors_skintint_uses_dielectric(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/actors/character/Body.bgsm",
            FakeMat(SkinTint=True),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"
        assert scale == 0.3

    def test_actors_creature_template_uses_dielectric(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/actors/dog/dog.bgsm",
            FakeMat(RootMaterialPath="template/CreatureTemplate_Wet.bgsm"),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"
        assert scale == 0.3

    def test_architecture_metal_hint_uses_outside(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/architecture/MetalRoof01.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 1.0

    def test_architecture_no_metal_hint_uses_dielectric(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/architecture/Brickwall01.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"
        assert scale == 0.3

    def test_clothes_path_uses_dielectric(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/clothes/Bathrobe/bathrobe.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"

    def test_armor_metal_hint_uses_outside(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/armor/Metal/MetalArmor.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"

    def test_armor_leather_hint_uses_dielectric(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/armor/LeatherCoat/leather.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"

    def test_vehicles_uses_outside(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/vehicles/Car01.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"

    def test_ammo_uses_outside(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/ammo/10mm/cartridge.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"


# ---------------------------------------------------------------- fallback
class TestFallback:
    def test_unknown_category_falls_back_to_outside(self) -> None:
        cubemap, scale = select_cubemap(
            "materials/unknownThing/foo.bgsm",
            FakeMat(),
        )
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 1.0

    def test_empty_source_path_returns_default(self) -> None:
        cubemap, scale = select_cubemap("", FakeMat())
        assert cubemap == "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
        assert scale == 1.0


# ----------------------------------------------- precedence: exclude > kw > path
class TestPrecedence:
    def test_exclusion_beats_keyword(self) -> None:
        # Even with a chrome diffuse, an effects path returns None.
        mat = FakeMat(DiffuseTexture="effects/Chrome_d.dds")
        cubemap, scale = select_cubemap("materials/effects/x.bgsm", mat)
        assert cubemap is None
        assert scale is None

    def test_keyword_beats_path(self) -> None:
        # Weapons path normally returns mipblur_DefaultOutside1, but a chrome
        # diffuse overrides it.
        mat = FakeMat(DiffuseTexture="weapons/foo/Chrome_d.dds")
        cubemap, scale = select_cubemap("materials/weapons/foo/x.bgsm", mat)
        assert cubemap == "Shared/Cubemaps/MetalChrome01Cube_e.dds"
