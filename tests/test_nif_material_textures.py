"""Tests for material->texture extraction and NIF database integration."""

import io
import os
import struct
from pathlib import Path

import pytest

from creation_lib.material_tools.extract_textures import parse_material_textures


def _write_bgsm(path: Path, diffuse: str, normal: str, version: int = 2):
    """Write a valid BGSM binary file using the actual writer."""
    from creation_lib.material_tools.bgsm_bin import BGSMData
    from creation_lib.material_tools.base import BaseHeader

    header = BaseHeader(
        signature=0x4D534742,
        version=version,
        tile_u=False,
        tile_v=False,
        u_offset=0.0,
        v_offset=0.0,
        u_scale=1.0,
        v_scale=1.0,
        alpha=1.0,
        alpha_blend_mode0=0,
        alpha_blend_mode1=0,
        alpha_blend_mode2=0,
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
        env_mapping=False,
        env_mapping_mask_scale=0.0,
        depth_bias=None,
        grayscale_to_palette_color=False,
        mask_writes=None,
    )

    mat = BGSMData(
        header=header,
        DiffuseTexture=diffuse,
        NormalTexture=normal,
        SmoothSpecTexture="",
        GreyscaleTexture="",
        EnvmapTexture="",
        GlowTexture="",
        InnerLayerTexture="",
        WrinklesTexture="",
        DisplacementTexture="",
        SpecularTexture=None,
        LightingTexture=None,
        FlowTexture=None,
        DistanceFieldAlphaTexture=None,
        EnableEditorAlphaRef=False,
        RimLighting=False,
        RimPower=0.0,
        BackLightPower=0.0,
        SubsurfaceLighting=False,
        SubsurfaceLightingRolloff=0.0,
        Translucency=None,
        TranslucencyThickObject=None,
        TranslucencyMixAlbedoWithSubsurfaceColor=None,
        TranslucencySubsurfaceColor=None,
        TranslucencyTransmissiveScale=None,
        TranslucencyTurbulence=None,
        SpecularEnabled=False,
        SpecularColor=(1.0, 1.0, 1.0),
        SpecularMult=1.0,
        Smoothness=0.0,
        FresnelPower=5.0,
        WetnessControlSpecScale=0.0,
        WetnessControlSpecPowerScale=0.0,
        WetnessControlSpecMinvar=0.0,
        WetnessControlEnvMapScale=0.0,
        WetnessControlFresnelPower=0.0,
        WetnessControlMetalness=0.0,
        PBR=None,
        CustomPorosity=None,
        PorosityValue=None,
        RootMaterialPath="",
        AnisoLighting=False,
        EmitEnabled=False,
        EmittanceColor=None,
        EmittanceMult=0.0,
        ModelSpaceNormals=False,
        ExternalEmittance=False,
        LumEmittance=None,
        UseAdaptativeEmissive=None,
        AdaptativeEmissive_ExposureOffset=None,
        AdaptativeEmissive_FinalExposureMin=None,
        AdaptativeEmissive_FinalExposureMax=None,
        BackLighting=False,
        ReceiveShadows=False,
        HideSecret=False,
        CastShadows=False,
        DissolveFade=False,
        AssumeShadowmask=False,
        Glowmap=False,
        EnvironmentMappingWindow=False,
        EnvironmentMappingEye=False,
        Hair=False,
        HairTintColor=(0.0, 0.0, 0.0),
        Tree=False,
        Facegen=False,
        SkinTint=False,
        Tessellate=False,
        DisplacementTextureBias=0.0,
        DisplacementTextureScale=0.0,
        TessellationPnScale=0.0,
        TessellationBaseFactor=0.0,
        TessellationFadeDistance=0.0,
        GrayscaleToPaletteScale=1.0,
        SkewSpecularAlpha=False,
        Terrain=None,
        UnkInt1=None,
        TerrainThresholdFalloff=None,
        TerrainTilingDistance=None,
        TerrainRotationAngle=None,
    )

    with open(str(path), "wb") as f:
        mat.write(f)


def _write_bgem(path: Path, base_tex: str, normal_tex: str, version: int = 2):
    """Write a valid BGEM binary file using the actual writer."""
    from creation_lib.material_tools.bgem_bin import BGEMData
    from creation_lib.material_tools.base import BaseHeader

    header = BaseHeader(
        signature=0x4D454742,
        version=version,
        tile_u=False,
        tile_v=False,
        u_offset=0.0,
        v_offset=0.0,
        u_scale=1.0,
        v_scale=1.0,
        alpha=1.0,
        alpha_blend_mode0=0,
        alpha_blend_mode1=0,
        alpha_blend_mode2=0,
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
        env_mapping=False,
        env_mapping_mask_scale=0.0,
        depth_bias=None,
        grayscale_to_palette_color=False,
        mask_writes=None,
    )

    mat = BGEMData(
        header=header,
        BaseTexture=base_tex,
        GrayscaleTexture="",
        EnvmapTexture="",
        NormalTexture=normal_tex,
        EnvmapMaskTexture="",
        SpecularTexture=None,
        LightingTexture=None,
        GlowTexture=None,
        GlassRoughnessScratch=None,
        GlassDirtOverlay=None,
        GlassEnabled=None,
        GlassFresnelColor=None,
        GlassBlurScaleBase=None,
        GlassBlurScaleFactor=None,
        GlassRefractionScaleBase=None,
        EnvironmentMapping=None,
        EnvironmentMappingMaskScale=None,
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
        AdaptativeEmissive_ExposureOffset=None,
        AdaptativeEmissive_FinalExposureMin=None,
        AdaptativeEmissive_FinalExposureMax=None,
        Glowmap=None,
        EffectPbrSpecular=None,
    )

    with open(str(path), "wb") as f:
        mat.write(f)


class TestParseMaterialTextures:
    def test_parses_bgsm_textures(self, tmp_path):
        bgsm_path = tmp_path / "test.bgsm"
        _write_bgsm(bgsm_path, "textures/sword_d.dds", "textures/sword_n.dds")
        result = parse_material_textures(str(bgsm_path))
        assert result is not None
        assert result["type"] == "bgsm"
        assert result["textures"]["DiffuseTexture"] == "textures/sword_d.dds"
        assert result["textures"]["NormalTexture"] == "textures/sword_n.dds"

    def test_parses_bgem_textures(self, tmp_path):
        bgem_path = tmp_path / "test.bgem"
        _write_bgem(bgem_path, "textures/effect_base.dds", "textures/effect_n.dds")
        result = parse_material_textures(str(bgem_path))
        assert result is not None
        assert result["type"] == "bgem"
        assert result["textures"]["BaseTexture"] == "textures/effect_base.dds"
        assert result["textures"]["NormalTexture"] == "textures/effect_n.dds"

    def test_skips_empty_texture_slots(self, tmp_path):
        bgsm_path = tmp_path / "test.bgsm"
        _write_bgsm(bgsm_path, "textures/armor_d.dds", "")
        result = parse_material_textures(str(bgsm_path))
        assert "NormalTexture" not in result["textures"]
        assert "DiffuseTexture" in result["textures"]

    def test_returns_none_for_missing_file(self):
        result = parse_material_textures("/nonexistent/path.bgsm")
        assert result is None

    def test_returns_none_for_invalid_file(self, tmp_path):
        bad = tmp_path / "bad.bgsm"
        bad.write_bytes(b"not a valid bgsm")
        result = parse_material_textures(str(bad))
        assert result is None

    def test_normalizes_paths(self, tmp_path):
        bgsm_path = tmp_path / "test.bgsm"
        _write_bgsm(bgsm_path, "Textures\\Weapons\\Sword_d.DDS", "")
        result = parse_material_textures(str(bgsm_path))
        diffuse = result["textures"]["DiffuseTexture"]
        assert "\\" not in diffuse  # forward slashes
        assert diffuse == diffuse.lower()  # lowercase


import sqlite3


class TestPreprocessNifsMaterialTextures:
    def test_creates_material_textures_table(self, tmp_path):
        """Verify the new table exists in freshly created DB."""
        from creation_lib.preprocessor.nifs import CREATE_TABLES_DDL

        db_path = tmp_path / "test_nifs.db"
        conn = sqlite3.connect(db_path)
        conn.executescript(CREATE_TABLES_DDL)
        tables = {
            r[0]
            for r in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
        }
        assert "nif_material_textures" in tables
        conn.close()

    def test_material_textures_schema(self, tmp_path):
        """Verify table columns and PK."""
        from creation_lib.preprocessor.nifs import CREATE_TABLES_DDL

        db_path = tmp_path / "test_nifs.db"
        conn = sqlite3.connect(db_path)
        conn.executescript(CREATE_TABLES_DDL)
        info = conn.execute("PRAGMA table_info(nif_material_textures)").fetchall()
        col_names = {r[1] for r in info}
        assert "material_path" in col_names
        assert "material_type" in col_names
        assert "texture_slot" in col_names
        assert "texture_path" in col_names
        conn.close()
