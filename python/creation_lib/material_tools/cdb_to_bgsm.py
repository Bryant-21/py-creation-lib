"""Translate a CE2Material (FO76/Starfield) into a flat BGSMData.

Flattens the layered PBR metal-rough ``CE2Material`` from ``materials_cdb`` into a
``BGSMData`` at the source game's native BGSM version (typically FO76 v22). The
FO76 -> FO4 downgrade is ``convert.downgrade_bgsm``; the orchestrator chains both.
PBR -> spec-gloss scalars go through ``pbr_convert.pbr_to_specgloss`` to match
BACUP's per-texel texture-remix path.

Layer collapsing is lossy: only the top (highest-index) layer is kept, and
blenders and LOD materials are dropped. Each drop is logged at INFO.
"""
from __future__ import annotations

import logging

import numpy as np

from creation_lib.core.game_profiles import RemixProfile
from creation_lib.material_tools.base import BaseHeader
from creation_lib.material_tools.bgsm_bin import BGSM_SIGNATURE, BGSMData
from creation_lib.material_tools.materials_cdb import CE2Layer, CE2Material
from creation_lib.material_tools.pbr_convert import PBRToSpecGlossParams, pbr_to_specgloss

log = logging.getLogger(__name__)


def _pick_top_layer(material: CE2Material) -> CE2Layer:
    if not material.layers:
        raise ValueError(f"CE2Material {material.name!r} has no layers")
    # highest index == topmost == last painted
    return material.layers[-1]


def _make_default_header(version: int) -> BaseHeader:
    """Return a fresh BaseHeader with neutral defaults at ``version``.

    Matches the field set required by ``BaseHeader.write`` for FO76 v22
    (env_mapping=None / depth_bias=False because version >= 10).
    """
    return BaseHeader(
        signature=BGSM_SIGNATURE,
        version=version,
        tile_u=True,
        tile_v=True,
        u_offset=0.0,
        v_offset=0.0,
        u_scale=1.0,
        v_scale=1.0,
        alpha=1.0,
        alpha_blend_mode0=0,
        alpha_blend_mode1=6,
        alpha_blend_mode2=7,
        alpha_test_ref=128,
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
        # FO76 is v22 >= 10, so env_mapping/env_mapping_mask_scale are None
        # and depth_bias is the live field.
        env_mapping=None,
        env_mapping_mask_scale=None,
        depth_bias=False,
        grayscale_to_palette_color=False,
        mask_writes=63,  # default: all channels enabled
    )


def _make_default_bgsm(header: BaseHeader) -> BGSMData:
    """Return a BGSMData with every required field populated to a neutral
    default suitable for an FO76-era PBR material.

    The version-conditional fields follow the read_bgsm invariants: for
    ``version >= 8`` the Translucency block is live and RimLighting block
    is None; for ``version >= 10`` WetnessControlEnvMapScale is None; for
    ``version > 2`` the FO76 texture slots are live and the FO4 ones are
    None. Downgrade to FO4 is the caller's job downstream.
    """
    v = header.version
    is_fo76 = v > 2
    return BGSMData(
        header=header,
        DiffuseTexture="",
        NormalTexture="",
        SmoothSpecTexture="",
        GreyscaleTexture="",
        # FO76-era texture slots
        EnvmapTexture=None if is_fo76 else "",
        GlowTexture="" if is_fo76 else "",
        InnerLayerTexture=None if is_fo76 else "",
        WrinklesTexture="" if is_fo76 else "",
        DisplacementTexture=None if is_fo76 else "",
        SpecularTexture="" if is_fo76 else None,
        LightingTexture="" if is_fo76 else None,
        FlowTexture="" if is_fo76 else None,
        DistanceFieldAlphaTexture="" if v >= 17 else None,
        EnableEditorAlphaRef=False,
        # Translucency block (v>=8) vs RimLighting block (v<8)
        RimLighting=None if v >= 8 else False,
        RimPower=None if v >= 8 else 2.0,
        BackLightPower=None if v >= 8 else 0.0,
        SubsurfaceLighting=None if v >= 8 else False,
        SubsurfaceLightingRolloff=None if v >= 8 else 0.3,
        Translucency=False if v >= 8 else None,
        TranslucencyThickObject=False if v >= 8 else None,
        TranslucencyMixAlbedoWithSubsurfaceColor=False if v >= 8 else None,
        TranslucencySubsurfaceColor=(1.0, 1.0, 1.0) if v >= 8 else None,
        TranslucencyTransmissiveScale=0.0 if v >= 8 else None,
        TranslucencyTurbulence=0.0 if v >= 8 else None,
        # Spec / smoothness defaults -- overwritten by caller
        SpecularEnabled=True,
        SpecularColor=(1.0, 1.0, 1.0),
        SpecularMult=1.0,
        Smoothness=0.5,
        FresnelPower=5.0,
        WetnessControlSpecScale=-0.95,
        WetnessControlSpecPowerScale=0.5,
        WetnessControlSpecMinvar=0.2,
        WetnessControlEnvMapScale=None if v >= 10 else 1.0,
        WetnessControlFresnelPower=1.6,
        WetnessControlMetalness=0.0,
        # PBR / porosity (v>2 only)
        PBR=True if is_fo76 else None,
        CustomPorosity=False if v >= 9 else None,
        PorosityValue=0.0 if v >= 9 else None,
        RootMaterialPath="",
        AnisoLighting=False,
        EmitEnabled=False,
        EmittanceColor=None,
        EmittanceMult=1.0,
        ModelSpaceNormals=False,
        ExternalEmittance=False,
        LumEmittance=0.0 if v >= 12 else None,
        UseAdaptativeEmissive=False if v >= 13 else None,
        AdaptativeEmissive_ExposureOffset=0.0 if v >= 13 else None,
        AdaptativeEmissive_FinalExposureMin=0.0 if v >= 13 else None,
        AdaptativeEmissive_FinalExposureMax=0.0 if v >= 13 else None,
        BackLighting=None if v >= 8 else False,
        ReceiveShadows=True,
        HideSecret=False,
        CastShadows=True,
        DissolveFade=False,
        AssumeShadowmask=False,
        Glowmap=False,
        EnvironmentMappingWindow=False if v < 7 else None,
        EnvironmentMappingEye=False if v < 7 else None,
        Hair=False,
        HairTintColor=(0.0, 0.0, 0.0),
        Tree=False,
        Facegen=False,
        SkinTint=False,
        Tessellate=False,
        DisplacementTextureBias=0.0 if v < 3 else None,
        DisplacementTextureScale=0.0 if v < 3 else None,
        TessellationPnScale=0.0 if v < 3 else None,
        TessellationBaseFactor=0.0 if v < 3 else None,
        TessellationFadeDistance=0.0 if v < 3 else None,
        GrayscaleToPaletteScale=1.0,
        SkewSpecularAlpha=False if v >= 1 else None,
        Terrain=False if v >= 3 else None,
        UnkInt1=None,
        TerrainThresholdFalloff=None,
        TerrainTilingDistance=None,
        TerrainRotationAngle=None,
    )


def cdb_to_bgsm(
    material: CE2Material,
    target_version: int,
    remix_profile: RemixProfile,
) -> BGSMData:
    """Flatten a ``CE2Material`` into a ``BGSMData`` at ``target_version``.

    The result is FO76-shaped even at v22. FO4 output must then go through
    ``convert.downgrade_bgsm``, which owns texture-slot remapping, Translucency ->
    RimLighting conversion, and RootMaterialPath synthesis.
    """
    if len(material.layers) > 1:
        log.info(
            "cdb_to_bgsm: flattening %d layers on %s (keeping top layer, "
            "dropping %d)",
            len(material.layers), material.name, len(material.layers) - 1,
        )
    if material.blenders:
        log.info(
            "cdb_to_bgsm: dropping %d blenders on %s",
            len(material.blenders), material.name,
        )
    if material.lod_materials:
        log.info(
            "cdb_to_bgsm: dropping %d LOD materials on %s",
            len(material.lod_materials), material.name,
        )

    layer = _pick_top_layer(material)

    # Scalar PBR -> spec-gloss as a 1x1 pass through ``pbr_to_specgloss``, so it
    # matches texture_remix's per-texel pass. Unit albedo stands in for the
    # texture color; the orchestrator replaces DiffuseTexture with the remixed
    # _d.dds.
    params = PBRToSpecGlossParams(
        ao_multiplier=remix_profile.ao_multiplier,
        specular_multiplier=remix_profile.specular_multiplier,
        gloss_multiplier=remix_profile.gloss_multiplier,
        spec_offset=remix_profile.spec_offset,
    )
    albedo = np.ones((1, 1, 3), dtype=np.float32)
    metallic = np.array([[layer.material.metalness]], dtype=np.float32)
    roughness = np.array([[1.0 - layer.material.smoothness]], dtype=np.float32)
    _diffuse, spec_arr, gloss_arr = pbr_to_specgloss(
        albedo, metallic, roughness, None, params
    )
    spec_color: tuple[float, float, float] = (
        float(spec_arr[0, 0, 0]),
        float(spec_arr[0, 0, 1]),
        float(spec_arr[0, 0, 2]),
    )
    smoothness = float(gloss_arr[0, 0])

    # Build the BGSM. Header first; then populate over the defaults.
    header = _make_default_header(target_version)
    bgsm = _make_default_bgsm(header)

    # CE2TextureSet doesn't separate roughness from specular per slot, so only
    # diffuse/normal pass through and the FO76 PBR slots (Specular/Lighting)
    # stay empty. texture_remix supplies the remixed DDS alongside, and the
    # downgrade tolerates the missing slots.
    bgsm.DiffuseTexture = layer.texture_set.diffuse or ""
    bgsm.NormalTexture = layer.texture_set.normal or ""

    # Shader params
    bgsm.SpecularColor = spec_color
    bgsm.Smoothness = smoothness

    # Emissive pass-through if the layer has one.
    if (
        layer.material.emissive_multiplier > 0.0
        and layer.material.emissive_color != (0.0, 0.0, 0.0)
    ):
        bgsm.EmitEnabled = True
        bgsm.EmittanceColor = layer.material.emissive_color
        bgsm.EmittanceMult = layer.material.emissive_multiplier

    # Alpha pass-through.
    bgsm.header.alpha = float(layer.material.alpha)

    return bgsm
