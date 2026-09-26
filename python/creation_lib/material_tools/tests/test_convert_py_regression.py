"""Regression tests for the FO76 -> FO4 downgrade in ``creation_lib.material_tools.convert``.

Each test pins an incident documented in ``convert.py``'s inline comments.
Tests build a minimal synthetic v20 BGSM/BGEM (no checked-in fixtures) and
mutate only the fields relevant to the bug being regression-tested.
"""
from __future__ import annotations

import io

import pytest

from creation_lib.material_tools.bgsm_bin import BGSMData, read_bgsm
from creation_lib.material_tools.bgem_bin import BGEMData, read_bgem
from creation_lib.material_tools.base import BaseHeader
from creation_lib.material_tools.bgsm_bin import BGSM_SIGNATURE
from creation_lib.material_tools.bgem_bin import BGEM_SIGNATURE
from creation_lib.material_tools.convert import (
    BGSM_VERSION_FO4,
    BGEM_VERSION_FO4,
    downgrade_bgsm,
    downgrade_bgem,
)


def _base_header(signature: int, version: int) -> BaseHeader:
    return BaseHeader(
        signature=signature,
        version=version,
        tile_u=True,
        tile_v=True,
        u_offset=0.0,
        v_offset=0.0,
        u_scale=1.0,
        v_scale=1.0,
        alpha=1.0,
        alpha_blend_mode0=0,
        alpha_blend_mode1=0,
        alpha_blend_mode2=0,
        alpha_test_ref=0,
        alpha_test=False,
        zbuffer_write=True,
        zbuffer_test=True,
        ssr=False,
        wet_ssr=False,
        decal=False,
        two_sided=False,
        decal_nofade=False,
        non_occluder=False,
        refraction=False,
        refraction_falloff=False,
        refraction_power=0.0,
        env_mapping=False,
        env_mapping_mask_scale=0.0,
        depth_bias=None,
        grayscale_to_palette_color=False,
        mask_writes=None,
    )


def _default_bgsm_v20() -> BGSMData:
    """A v20 BGSM with all fields at their "empty FO76 default" value,
    matching what `downgrade_bgsm` sees for the vast majority of real
    assets. Individual tests mutate only the fields relevant to the bug
    under test."""
    return BGSMData(
        header=_base_header(BGSM_SIGNATURE, 20),
        DiffuseTexture="weapons/gaussrifle/foo_d.dds",
        NormalTexture="weapons/gaussrifle/foo_n.dds",
        SmoothSpecTexture="",
        GreyscaleTexture="",
        EnvmapTexture="",
        GlowTexture="",
        InnerLayerTexture="",
        WrinklesTexture="",
        DisplacementTexture="",
        SpecularTexture="",
        LightingTexture="",
        FlowTexture="",
        DistanceFieldAlphaTexture="",
        EnableEditorAlphaRef=False,
        RimLighting=False,
        RimPower=0.0,
        BackLightPower=0.0,
        SubsurfaceLighting=False,
        SubsurfaceLightingRolloff=0.0,
        Translucency=False,
        TranslucencyThickObject=False,
        TranslucencyMixAlbedoWithSubsurfaceColor=False,
        TranslucencySubsurfaceColor=(0.0, 0.0, 0.0),
        TranslucencyTransmissiveScale=0.0,
        TranslucencyTurbulence=0.0,
        SpecularEnabled=True,
        SpecularColor=(1.0, 1.0, 1.0),
        SpecularMult=1.0,
        Smoothness=0.5,
        FresnelPower=1.0,
        WetnessControlSpecScale=1.0,
        WetnessControlSpecPowerScale=1.0,
        WetnessControlSpecMinvar=0.0,
        WetnessControlEnvMapScale=1.0,
        WetnessControlFresnelPower=1.0,
        WetnessControlMetalness=0.0,
        PBR=False,
        CustomPorosity=False,
        PorosityValue=0.0,
        RootMaterialPath="",
        AnisoLighting=False,
        EmitEnabled=False,
        EmittanceColor=None,
        EmittanceMult=1.0,
        ModelSpaceNormals=False,
        ExternalEmittance=False,
        LumEmittance=None,
        UseAdaptativeEmissive=False,
        AdaptativeEmissive_ExposureOffset=0.0,
        AdaptativeEmissive_FinalExposureMin=0.0,
        AdaptativeEmissive_FinalExposureMax=0.0,
        BackLighting=False,
        ReceiveShadows=True,
        HideSecret=False,
        CastShadows=True,
        DissolveFade=False,
        AssumeShadowmask=True,
        Glowmap=False,
        EnvironmentMappingWindow=False,
        EnvironmentMappingEye=False,
        Hair=False,
        HairTintColor=(1.0, 1.0, 1.0),
        Tree=False,
        Facegen=False,
        SkinTint=False,
        Tessellate=False,
        DisplacementTextureBias=0.0,
        DisplacementTextureScale=1.0,
        TessellationPnScale=1.0,
        TessellationBaseFactor=1.0,
        TessellationFadeDistance=1.0,
        GrayscaleToPaletteScale=1.0,
        SkewSpecularAlpha=False,
        Terrain=False,
        UnkInt1=0,
        TerrainThresholdFalloff=0.0,
        TerrainTilingDistance=0.0,
        TerrainRotationAngle=0.0,
    )


def _default_bgem_v22() -> BGEMData:
    return BGEMData(
        header=_base_header(BGEM_SIGNATURE, 22),
        BaseTexture="",
        GrayscaleTexture="",
        EnvmapTexture="",
        NormalTexture="",
        EnvmapMaskTexture="",
        SpecularTexture="",
        LightingTexture="",
        GlowTexture="",
        GlassRoughnessScratch=None,
        GlassDirtOverlay=None,
        GlassEnabled=None,
        GlassFresnelColor=None,
        GlassBlurScaleBase=None,
        GlassBlurScaleFactor=None,
        GlassRefractionScaleBase=None,
        EnvironmentMapping=False,
        EnvironmentMappingMaskScale=0.0,
        BloodEnabled=False,
        EffectLightingEnabled=False,
        FalloffEnabled=False,
        FalloffColorEnabled=False,
        GrayscaleToPaletteAlpha=False,
        SoftEnabled=False,
        BaseColor=(1.0, 1.0, 1.0),
        BaseColorScale=1.0,
        FalloffStartAngle=0.0,
        FalloffStopAngle=0.0,
        FalloffStartOpacity=0.0,
        FalloffStopOpacity=0.0,
        LightingInfluence=0.0,
        EnvmapMinLOD=0,
        SoftDepth=0.0,
        EmittanceColor=None,
        AdaptativeEmissive_ExposureOffset=0.0,
        AdaptativeEmissive_FinalExposureMin=0.0,
        AdaptativeEmissive_FinalExposureMax=0.0,
        Glowmap=False,
        EffectPbrSpecular=False,
    )


# ---------------------------------------------------------------------------
# 1. Mirror-shiny weapon bug
# ---------------------------------------------------------------------------


def test_mirror_shiny_weapon_bug_specular_promoted_to_smoothspec():
    """FO76 v20 BGSM with a populated SpecularTexture and an empty
    SmoothSpecTexture must NOT write the _s.dds roughness map into
    EnvmapTexture / InnerLayerTexture / DisplacementTexture — the FO4
    render path samples those as a cubemap and produces mirror-shiny
    weapons. The _s.dds must instead be promoted into SmoothSpecTexture.

    EnvmapTexture gets a heuristic FO4 cubemap instead; select_cubemap is
    covered by test_cubemap_heuristics.
    """
    data = _default_bgsm_v20()
    data.SpecularTexture = "weapons/gaussrifle/foo_s.dds"
    data.SmoothSpecTexture = ""
    data.LightingTexture = ""
    data.FlowTexture = ""

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    # The _s.dds roughness map MUST NOT leak into any FO4 cubemap slot.
    assert "_s.dds" not in (fo4.EnvmapTexture or "")
    assert fo4.InnerLayerTexture == ""
    assert fo4.DisplacementTexture == ""
    assert fo4.SmoothSpecTexture == "weapons/gaussrifle/foo_s.dds"
    # EnvmapTexture should be a real FO4 cubemap (heuristic-injected).
    assert fo4.EnvmapTexture and "Cubemaps" in fo4.EnvmapTexture
    assert fo4.header.env_mapping is True


# ---------------------------------------------------------------------------
# 2/3. Whole-object BGSM emittance suppression
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("source_path", "emit_enabled", "expect_glow", "expect_emit"),
    [
        ("weapons/gaussrifle/foo.bgsm", True, False, False),
        ("materials/effects/foo.bgsm", True, True, True),
        ("weapons/gaussrifle/foo.bgsm", False, False, False),
    ],
)
def test_lighting_texture_emittance_scoped_to_effects(source_path, emit_enabled, expect_glow, expect_emit):
    """FO4 applies converted BGSM emittance across the whole object
    surface, so LightingTexture->Glowmap promotion is only safe for
    effects-path materials with EmitEnabled; everywhere else, or without
    EmitEnabled, it must be suppressed."""
    data = _default_bgsm_v20()
    data.LightingTexture = "foo_l.dds"
    data.GlowTexture = ""
    data.EmitEnabled = emit_enabled
    if emit_enabled:
        data.EmittanceMult = 10.0
        data.EmittanceColor = (1.0, 1.0, 0.0)

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path=source_path)

    assert bool(fo4.GlowTexture) is expect_glow
    assert fo4.Glowmap is expect_glow
    assert fo4.EmitEnabled is expect_emit
    assert fo4.EmittanceMult == 1.0
    assert fo4.LightingTexture is None


# ---------------------------------------------------------------------------
# 4. Translucency -> SubsurfaceLighting value preservation
# ---------------------------------------------------------------------------


def test_translucency_converted_to_subsurface_lighting():
    """FO76 v20 BGSM with Translucency=True and TranslucencyTransmissiveScale
    must downgrade to SubsurfaceLighting=True with matching rolloff; the
    entire Translucency* block must be cleared."""
    data = _default_bgsm_v20()
    data.Translucency = True
    data.TranslucencyTransmissiveScale = 0.5
    data.TranslucencyThickObject = True
    data.TranslucencyMixAlbedoWithSubsurfaceColor = True
    data.TranslucencySubsurfaceColor = (0.8, 0.4, 0.2)
    data.TranslucencyTurbulence = 0.25

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    assert fo4.SubsurfaceLighting is True
    assert fo4.SubsurfaceLightingRolloff == pytest.approx(0.5)
    assert fo4.Translucency is None
    assert fo4.TranslucencyThickObject is None
    assert fo4.TranslucencyMixAlbedoWithSubsurfaceColor is None
    assert fo4.TranslucencySubsurfaceColor is None
    assert fo4.TranslucencyTransmissiveScale is None
    assert fo4.TranslucencyTurbulence is None


# ---------------------------------------------------------------------------
# 5. RootMaterialPath synthesis (family rules + generic fallback)
# ---------------------------------------------------------------------------


def test_vegetation_material_defaults_for_leaf_template():
    data = _default_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = True
    data.Translucency = True
    data.TranslucencyTransmissiveScale = 1.0
    data.SpecularTexture = "Landscape/Plants/Bramble01_r.dds"
    data.LightingTexture = "Landscape/Plants/Bramble01_l.dds"

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="materials/landscape/plants/bramble.bgsm",
    )

    assert fo4.RootMaterialPath == "Template/LeafTemplate_Wet.bgsm"
    assert fo4.BackLighting is True
    assert fo4.BackLightPower == pytest.approx(0.25)
    assert fo4.SubsurfaceLighting is True
    assert fo4.SubsurfaceLightingRolloff == pytest.approx(2.0)
    assert fo4.EnvmapTexture == ""
    assert not fo4.GlowTexture
    assert fo4.SmoothSpecTexture == "Landscape/Plants/Bramble01_s.dds"


def test_grass_material_uses_grass_template_before_tree_flag():
    # Grass paths take priority over the Tree flag (which is also set on
    # some grass assets and would otherwise pick the leaf template).
    data = _default_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = True
    data.Translucency = True

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="materials/landscape/grass/mtntop_grass01.bgsm",
    )

    assert fo4.RootMaterialPath == "Template/GrassTemplate_Wet.BGSM"
    assert fo4.SubsurfaceLighting is True
    assert fo4.EnvmapTexture == ""


@pytest.mark.parametrize(
    ("source_path", "expected_template"),
    [
        # No family match: RootMaterialPath synthesis still falls back to a
        # generic template rather than staying empty (checked via `in`,
        # since the exact fallback value isn't part of the contract here).
        ("weapons/gaussrifle/foo.bgsm", None),
        ("materials/landscape/rocks/mtntopcliff01.bgsm", "template/RockTemplate_Wet.bgsm"),
        ("materials/landscape/rocks/rockslab01.bgsm", "template/RockSlabTemplate_Wet.bgsm"),
        ("materials/landscape/ground/crackedmud01.bgsm", "template/CrackedMudTemplate_Wet.bgsm"),
        ("materials/landscape/roads/asphaltroad01.bgsm", "template/AsphaltTemplate_Wet.bgsm"),
        ("materials/setdressing/rubbertire01.bgsm", "template/RubberTemplate_Wet.bgsm"),
        ("materials/setdressing/hides/radstaghides.bgsm", "template/ClothTemplate_Felt_Wet.bgsm"),
        ("materials/architecture/buildings/metalrailing01.bgsm", "template/WroughtIronMetalTemplate_Wet.bgsm"),
        ("materials/architecture/capsules/capdetailsheet01.bgsm", "template/CapMetalTemplate_Wet.bgsm"),
        ("materials/vehicles/bus/bus01.bgsm", "template/VehicleBusTemplate_Wet.bgsm"),
        ("materials/vehicles/car/car01.bgsm", "template/VehicleTemplate_Wet.bgsm"),
        ("materials/actors/dogmeat/dogmeat_body.bgsm", "template/FurTemplate_Wet.bgsm"),
        ("materials/gore/goreorgans.bgsm", "template/basicsmooth.bgsm"),
        ("materials/gore/goresupermutanthead.bgsm", "template/SkinTemplate_Wet.bgsm"),
    ],
)
def test_root_material_path_uses_fo4_template_family_rules(source_path, expected_template):
    data = _default_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = False

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path=source_path)

    if expected_template is None:
        assert fo4.RootMaterialPath
        assert "template/" in fo4.RootMaterialPath.lower()
    else:
        assert fo4.RootMaterialPath == expected_template


# ---------------------------------------------------------------------------
# 6. Round-trip validity of downgraded BGSM
# ---------------------------------------------------------------------------


def test_downgraded_bgsm_round_trips_through_reader():
    """A downgraded BGSM must serialize + re-parse cleanly as a valid
    FO4 v2 BGSM."""
    data = _default_bgsm_v20()
    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgsm(buf)

    assert reloaded.header.version == BGSM_VERSION_FO4
    # The native writer null-terminates strings in place, so compare the
    # meaningful (stripped) content rather than exact byte identity.
    assert reloaded.DiffuseTexture.rstrip("\x00") == fo4.DiffuseTexture.rstrip("\x00")
    assert reloaded.NormalTexture.rstrip("\x00") == fo4.NormalTexture.rstrip("\x00")


# ---------------------------------------------------------------------------
# 7. BGEM downgrade: Glass field clearing + no-op at target version
# ---------------------------------------------------------------------------


def test_bgem_downgrade_clears_glass_fields_and_is_noop_at_target():
    """FO76 v22 BGEM with Glass* fields populated must downgrade with all
    Glass* fields cleared so the FO4 v20 writer doesn't see stray glass
    state; a BGEM whose header is already at the target version must
    instead be returned unchanged (the same instance, per current
    contract)."""
    already_fo4 = _default_bgem_v22()
    already_fo4.header.version = BGEM_VERSION_FO4  # simulate "already FO4"

    result = downgrade_bgem(already_fo4, BGEM_VERSION_FO4)

    assert result is already_fo4
    assert result.header.version == BGEM_VERSION_FO4

    data = _default_bgem_v22()
    data.GlassEnabled = True
    data.GlassFresnelColor = (0.5, 0.5, 0.5)
    data.GlassBlurScaleBase = 1.0
    data.GlassBlurScaleFactor = 1.0
    data.GlassRefractionScaleBase = 1.0

    fo4 = downgrade_bgem(data, BGEM_VERSION_FO4)

    assert fo4.GlassRoughnessScratch is None
    assert fo4.GlassDirtOverlay is None
    assert fo4.GlassEnabled is None
    assert fo4.GlassFresnelColor is None
    assert fo4.GlassBlurScaleBase is None
    assert fo4.GlassBlurScaleFactor is None
    assert fo4.GlassRefractionScaleBase is None

    # Round-trip as a valid v20 BGEM.
    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgem(buf)
    assert reloaded.header.version == BGEM_VERSION_FO4
