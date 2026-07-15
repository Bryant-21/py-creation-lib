"""Translate a CE2Material (FO76/Starfield) into a flat BGSMData.

This module takes the layered,
PBR-metal-rough ``CE2Material`` view produced by ``materials_cdb`` and
emits an equivalent flat ``BGSMData`` instance **at the source game's
native BGSM version** (typically FO76 v22). It does NOT perform the
FO76 -> FO4 downgrade; that's ``creation_lib.material_tools.convert.downgrade_bgsm``'s
job, and the orchestrator chains the two calls.

Per-texel math for the PBR -> spec-gloss conversion routes through
``creation_lib.material_tools.pbr_convert.pbr_to_specgloss`` so the scalar
conversion here stays consistent with the channel-remix path in
BACUP texture-remix workflows.

Layer collapsing is lossy by design:

* Multi-layer materials keep only the top (highest-index) layer.
* Blenders are dropped entirely.
* LOD materials are dropped entirely.

All drops are logged at INFO level. Downstream the caller can inspect
the log to decide whether a given material warrants manual attention.
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

    Produces an FO76-shaped BGSM (even at v22) that the caller should
    then pass through ``creation_lib.material_tools.convert.downgrade_bgsm`` if
    emitting for FO4. This separation is deliberate: the downgrade path
    has battle-tested texture-slot remapping, Translucency -> RimLighting
    conversion, and RootMaterialPath synthesis that would be lossy to
    duplicate here.

    Layer collapsing (multi-layer -> top-only, blenders/LODs dropped) is
    logged at INFO level on ``creation_lib.material_tools.cdb_to_bgsm``.
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

    # PBR -> spec-gloss scalar conversion. We run a 1x1 unit-albedo pass
    # through the shared ``pbr_to_specgloss`` helper so scalar conversion
    # here matches the per-texel pass texture_remix does on real DDS data.
    # The albedo color is taken from the texture (which we don't have
    # here at the scalar level), so we use unit albedo as a stand-in --
    # the downstream orchestrator overwrites DiffuseTexture with the
    # real remixed _d.dds anyway.
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

    # Texture slots. For FO76 BGSMs (v>2) we populate the color/normal
    # slots directly; the PBR roughness/metal maps live on the Specular /
    # Lighting texture slots when present. ``cdb_to_bgsm`` works from a
    # CE2TextureSet that doesn't distinguish roughness vs specular at the
    # slot level, so we pass through the top-layer's diffuse/normal and
    # leave the FO76 PBR slots empty -- texture_remix supplies
    # the remixed DDS data alongside this material, and the downstream
    # downgrade copes with missing slots gracefully.
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
