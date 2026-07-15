"""BGSM/BGEM material file readers.

"""
from __future__ import annotations
import io
import logging
from pathlib import Path

from .base import MaterialData

_log = logging.getLogger("nif_editor.material_readers.bgsm")


def read_bgsm(filepath_or_bytes: Path | bytes) -> MaterialData:
    """Parse a .bgsm material file and return MaterialData.

    Accepts a file Path or raw bytes (from BA2 extraction).
    Raises on invalid data.
    """
    from creation_lib.material_tools.bgsm_bin import read_bgsm as _read_bgsm_bin

    if isinstance(filepath_or_bytes, bytes):
        data = _read_bgsm_bin(io.BytesIO(filepath_or_bytes))
    else:
        with open(filepath_or_bytes, "rb") as f:
            data = _read_bgsm_bin(f)

    def _s(v):
        return (v or "").rstrip("\x00")

    texture_paths = {}
    for key, attr in [
        ("diffuse", "DiffuseTexture"),
        ("normal", "NormalTexture"),
        ("smooth_spec", "SmoothSpecTexture"),
        ("greyscale", "GreyscaleTexture"),
        ("envmap", "EnvmapTexture"),
        ("glow", "GlowTexture"),
        ("specular", "SpecularTexture"),
    ]:
        val = _s(getattr(data, attr, ""))
        if val:
            texture_paths[key] = val

    params = {
        "type": "bgsm",
        "grayscale_to_palette_color": data.header.grayscale_to_palette_color,
        "palette_scale": data.GrayscaleToPaletteScale,
        "spec_color": (data.SpecularColor[0], data.SpecularColor[1], data.SpecularColor[2]),
        "spec_strength": data.SpecularMult,
        "glossiness": data.Smoothness,
        "fresnel_power": data.FresnelPower,
    }

    return MaterialData(
        texture_paths=texture_paths,
        params=params,
        material_model="spec-gloss",
    )


def read_bgem(filepath_or_bytes: Path | bytes) -> MaterialData:
    """Parse a .bgem effect material file and return MaterialData.

    Accepts a file Path or raw bytes (from BA2 extraction).
    Raises on invalid data.
    """
    from creation_lib.material_tools.bgem_bin import read_bgem as _read_bgem_bin

    if isinstance(filepath_or_bytes, bytes):
        data = _read_bgem_bin(io.BytesIO(filepath_or_bytes))
    else:
        with open(filepath_or_bytes, "rb") as f:
            data = _read_bgem_bin(f)

    def _s(v):
        return (v or "").rstrip("\x00")

    texture_paths = {}
    for key, attr in [
        ("diffuse", "BaseTexture"),
        ("normal", "NormalTexture"),
        ("greyscale", "GrayscaleTexture"),
        ("cubemap", "EnvmapTexture"),
        ("envmask", "EnvmapMaskTexture"),
    ]:
        val = _s(getattr(data, attr, ""))
        if val:
            texture_paths[key] = val
    # SpecularTexture may not exist on all BGEM versions
    spec = _s(getattr(data, "SpecularTexture", ""))
    if spec:
        texture_paths["specular"] = spec

    params = {
        "type": "bgem",
        "base_color": data.BaseColor,
        "base_color_scale": data.BaseColorScale,
        "env_mapping": data.EnvironmentMapping,
        "env_mapping_mask_scale": data.EnvironmentMappingMaskScale,
        "falloff_enabled": data.FalloffEnabled,
        "falloff_start_angle": data.FalloffStartAngle,
        "falloff_stop_angle": data.FalloffStopAngle,
        "falloff_start_opacity": data.FalloffStartOpacity,
        "falloff_stop_opacity": data.FalloffStopOpacity,
        "lighting_influence": data.LightingInfluence,
        "grayscale_to_palette_alpha": data.GrayscaleToPaletteAlpha,
        "grayscale_to_palette_color": data.header.grayscale_to_palette_color,
        "falloff_color_enabled": data.FalloffColorEnabled,
        "effect_lighting_enabled": data.EffectLightingEnabled,
        "header_alpha": data.header.alpha,
        "header_blend_mode": data.header.alpha_blend_mode0,
        "header_blend_src": data.header.alpha_blend_mode1,
        "header_blend_dst": data.header.alpha_blend_mode2,
        "header_alpha_test": data.header.alpha_test,
        "header_alpha_test_ref": data.header.alpha_test_ref,
        "header_zbuffer_write": data.header.zbuffer_write,
        "alpha_blend": data.header.alpha_blend_mode0 != 0,
    }

    return MaterialData(
        texture_paths=texture_paths,
        params=params,
        material_model="spec-gloss",
    )
