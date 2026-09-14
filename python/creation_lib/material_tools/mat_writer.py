"""BGSM / BGEM → Starfield version-2 .mat JSON (layered materials, used by CK and engine).

Starfield .mat structure:
  {
    "version": 2,
    "data": {
      "BSMaterial::MaterialOverrideColorTypeComponent": {...},
      "BSMaterial::LayeredMaterialProperties": {
        "BSMaterial::Layer": [{ texture set, emittance, UV stream, ... }]
      },
      "BSMaterial::TranslucencySettings": {...},
      "BSMaterial::ShaderModelComponent": {...},
      "BSMaterial::MaterialFlags": {...},
      "BSMaterial::EmittanceComponent": {...},   # top-level emittance
    }
  }
"""
from __future__ import annotations

import json
from pathlib import PurePosixPath
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from creation_lib.core.game_profiles import GameProfile
    from .bgsm_bin import BGSMData
    from .bgem_bin import BGEMData


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _tex(path: str | None, source_profile: "GameProfile", target_profile: "GameProfile") -> str:
    """Normalise a BGSM texture path and rename its suffix for Starfield.

    Returns empty string if path is absent/empty.
    """
    if not path or not path.strip():
        return ""
    from creation_lib.textures.naming import convert_texture_name
    normalized = path.replace("\\", "/").strip()
    filename = normalized.split("/")[-1]
    dir_part = normalized[: normalized.rfind(filename)]
    new_name = convert_texture_name(filename, source_profile, target_profile)
    return dir_part + new_name


def _color3(rgb: tuple[float, float, float] | None, default=(1.0, 1.0, 1.0)) -> dict:
    r, g, b = rgb if rgb else default
    return {"R": round(float(r), 6), "G": round(float(g), 6), "B": round(float(b), 6)}


def _f(v, default: float = 0.0) -> float:
    return round(float(v if v is not None else default), 6)


def _b(v, default: bool = False) -> bool:
    return bool(v) if v is not None else default


# ---------------------------------------------------------------------------
# BGSM → .mat
# ---------------------------------------------------------------------------

def bgsm_to_mat(
    bgsm: "BGSMData",
    source_profile: "GameProfile",
    target_profile: "GameProfile",
) -> dict:
    """Convert a BGSMData (any version) to a Starfield .mat dict ready for JSON."""
    def t(path: str | None) -> str:
        return _tex(path, source_profile, target_profile)

    # --- Texture slots ---
    # FO4 v2: spec-gloss layout (Diffuse, Normal, SmoothSpec, Greyscale,
    #         Envmap, Glow, InnerLayer, Wrinkles, Displacement)
    # FO76 v>2: PBR layout (Diffuse, Normal, SmoothSpec, Greyscale,
    #           Glow, Wrinkles, Specular, Lighting, Flow, [DistanceFieldAlpha])
    albedo    = t(bgsm.DiffuseTexture)
    normal    = t(bgsm.NormalTexture)
    emissive  = t(bgsm.GlowTexture)
    opacity   = t(bgsm.GreyscaleTexture) or t(getattr(bgsm, "DistanceFieldAlphaTexture", None))

    # Roughness / Metalness
    # FO76 (PBR): SpecularTexture = metallic/reflectivity, LightingTexture = smoothness/AO
    # FO4 (spec-gloss): SmoothSpecTexture encodes both R=roughness(inv) G=metallic B=AO
    # We map the best available candidate into Roughness; Metalness gets its own slot if separate.
    roughness = ""
    metalness = ""
    if getattr(bgsm, "SpecularTexture", None):          # FO76 PBR layout
        roughness = t(bgsm.SpecularTexture)
        metalness = t(getattr(bgsm, "LightingTexture", None))
    elif bgsm.SmoothSpecTexture:                         # FO4 spec-gloss: same map for both
        roughness = t(bgsm.SmoothSpecTexture)
        metalness = roughness

    # Height: Wrinkles map is the closest semantic match
    height = t(bgsm.WrinklesTexture) or t(getattr(bgsm, "DisplacementTexture", None))

    # AO: separate slot not present in BGSM — leave empty
    ao = ""

    textures = {
        "Albedo":          albedo,
        "Normal":          normal,
        "Roughness":       roughness,
        "Metalness":       metalness,
        "AmbientOcclusion": ao,
        "Height":          height,
        "Opacity":         opacity,
        "Emissive":        emissive,
    }

    # --- Layer-level emittance ---
    emit_color  = bgsm.EmittanceColor or (1.0, 1.0, 1.0)
    emit_mult   = _f(bgsm.EmittanceMult, 0.0) if bgsm.EmitEnabled else 0.0
    use_adaptive = _b(getattr(bgsm, "UseAdaptativeEmissive", None))

    layer_emittance = {
        "EmittanceMult":   emit_mult,
        "EmittanceColor":  _color3(emit_color),
        "AdaptativeEmissive_ExposureOffset":    _f(getattr(bgsm, "AdaptativeEmissive_ExposureOffset", None)),
        "AdaptativeEmissive_FinalExposureMin":  _f(getattr(bgsm, "AdaptativeEmissive_FinalExposureMin", None)),
        "AdaptativeEmissive_FinalExposureMax":  _f(getattr(bgsm, "AdaptativeEmissive_FinalExposureMax", None)),
        "UseAdaptiveEmissive": use_adaptive,
    }

    # --- Translucency ---
    # FO76+ has explicit Translucency flags; FO4 has SubsurfaceLighting.
    translucency = {
        "Thin": _b(getattr(bgsm, "Translucency", None)),
        "FlipBackFaceNormalsInViewSpace": _b(getattr(bgsm, "TranslucencyThickObject", None)),
        "UseSSS": _b(getattr(bgsm, "SubsurfaceLighting", None)),
        "SSSStrength": _f(getattr(bgsm, "SubsurfaceLightingRolloff", None) or
                         getattr(bgsm, "TranslucencyTransmissiveScale", None)),
    }
    if getattr(bgsm, "TranslucencySubsurfaceColor", None):
        translucency["SSSColor"] = _color3(bgsm.TranslucencySubsurfaceColor)

    # --- Shader PBR properties (layer-level) ---
    shader_properties: dict = {}
    if _b(getattr(bgsm, "SpecularEnabled", True)):
        shader_properties["SpecularColor"] = _color3(getattr(bgsm, "SpecularColor", None))
        shader_properties["SpecularMult"]  = _f(getattr(bgsm, "SpecularMult", 1.0), 1.0)
        shader_properties["Smoothness"]    = _f(getattr(bgsm, "Smoothness", 1.0), 1.0)
    if getattr(bgsm, "FresnelPower", None) is not None:
        shader_properties["FresnelPower"]  = _f(bgsm.FresnelPower)

    # --- Material flags ---
    flags = {
        "CastShadows":    _b(getattr(bgsm, "CastShadows", True), True),
        "ReceiveShadows": _b(getattr(bgsm, "ReceiveShadows", True), True),
        "IsHair":         _b(getattr(bgsm, "Hair", None)),
        "IsTree":         _b(getattr(bgsm, "Tree", None)),
        "IsFacegen":      _b(getattr(bgsm, "Facegen", None)),
        "IsSkinTint":     _b(getattr(bgsm, "SkinTint", None)),
        "HasOpacity":     bool(opacity),
    }

    # --- Shader model ---
    shader_model = "BaseMaterial"
    if _b(getattr(bgsm, "Hair", None)):
        shader_model = "Hair"
    elif _b(getattr(bgsm, "Facegen", None)) or _b(getattr(bgsm, "SkinTint", None)):
        shader_model = "Face"
    elif _b(getattr(bgsm, "Terrain", None)):
        shader_model = "Terrain"

    # --- Assemble layer ---
    layer: dict = {
        "BSMaterial::TextureSetComponent": {
            "BSMaterial::ResolutionHint": "High",
            "Textures": textures,
        },
        "BSMaterial::LayeredEmittanceComponent": layer_emittance,
    }
    if shader_properties:
        layer["BSMaterial::ShaderPropertyComponent"] = shader_properties

    # --- Assemble top-level data ---
    data: dict = {
        "BSMaterial::MaterialOverrideColorTypeComponent": {"Value": "None"},
        "BSMaterial::LayeredMaterialProperties": {
            "BSMaterial::Layer": [layer],
        },
        "BSMaterial::TranslucencySettings":   translucency,
        "BSMaterial::ShaderModelComponent":   {"BSMaterial::ShaderModel": shader_model},
        "BSMaterial::MaterialFlags":          flags,
    }

    # Top-level emittance summary (mirrors layer-level for engine compatibility)
    if bgsm.EmitEnabled:
        data["BSMaterial::EmittanceComponent"] = {
            "EmittanceMult":  emit_mult,
            "EmittanceColor": _color3(emit_color),
            "LumEmittance":   _f(getattr(bgsm, "LumEmittance", None)),
        }

    return {"version": 2, "data": data}


# ---------------------------------------------------------------------------
# BGEM → .mat
# ---------------------------------------------------------------------------

def bgem_to_mat(
    bgem: "BGEMData",
    source_profile: "GameProfile",
    target_profile: "GameProfile",
) -> dict:
    """Convert a BGEMData (effect shader) to a Starfield .mat dict.

    BGEM maps to a Starfield effect/decal material with opacity and falloff.
    """
    def t(path: str | None) -> str:
        return _tex(path, source_profile, target_profile)

    textures = {
        "Albedo":          t(bgem.BaseTexture),
        "Normal":          t(bgem.NormalTexture),
        "Roughness":       t(getattr(bgem, "SpecularTexture", None)),
        "Metalness":       t(getattr(bgem, "LightingTexture", None)),
        "AmbientOcclusion": "",
        "Height":          "",
        "Opacity":         t(bgem.GrayscaleTexture),
        "Emissive":        t(getattr(bgem, "GlowTexture", None)),
    }

    emit_color = getattr(bgem, "EmittanceColor", None) or (1.0, 1.0, 1.0)
    layer_emittance = {
        "EmittanceMult":   _f(bgem.BaseColorScale, 1.0),
        "EmittanceColor":  _color3(emit_color),
        "AdaptativeEmissive_ExposureOffset":   _f(getattr(bgem, "AdaptativeEmissive_ExposureOffset", None)),
        "AdaptativeEmissive_FinalExposureMin": _f(getattr(bgem, "AdaptativeEmissive_FinalExposureMin", None)),
        "AdaptativeEmissive_FinalExposureMax": _f(getattr(bgem, "AdaptativeEmissive_FinalExposureMax", None)),
        "UseAdaptiveEmissive": False,
    }

    # Falloff → opacity settings
    opacity_settings: dict = {}
    if bgem.FalloffEnabled:
        opacity_settings = {
            "FalloffEnabled":      True,
            "FalloffStartAngle":   _f(bgem.FalloffStartAngle),
            "FalloffStopAngle":    _f(bgem.FalloffStopAngle),
            "FalloffStartOpacity": _f(bgem.FalloffStartOpacity),
            "FalloffStopOpacity":  _f(bgem.FalloffStopOpacity),
        }

    translucency = {
        "Thin": True,   # BGEM effect shaders are typically thin/transparent
        "FlipBackFaceNormalsInViewSpace": False,
        "UseSSS": False,
        "SSSStrength": 0.0,
    }

    flags = {
        "CastShadows":    False,  # effect shaders typically don't cast shadows
        "ReceiveShadows": False,
        "IsDecal":        True,
        "HasOpacity":     True,
        "TwoSided":       True,
    }

    layer: dict = {
        "BSMaterial::TextureSetComponent": {
            "BSMaterial::ResolutionHint": "High",
            "Textures": textures,
        },
        "BSMaterial::LayeredEmittanceComponent": layer_emittance,
    }
    if opacity_settings:
        layer["BSMaterial::OpacityComponent"] = opacity_settings

    # Tint colour from BaseColor
    base_r, base_g, base_b = bgem.BaseColor
    if (base_r, base_g, base_b) != (1.0, 1.0, 1.0):
        layer["BSMaterial::ColorRemapComponent"] = {
            "TintColor": _color3(bgem.BaseColor),
        }

    data: dict = {
        "BSMaterial::MaterialOverrideColorTypeComponent": {"Value": "None"},
        "BSMaterial::LayeredMaterialProperties": {
            "BSMaterial::Layer": [layer],
        },
        "BSMaterial::TranslucencySettings":  translucency,
        "BSMaterial::ShaderModelComponent":  {"BSMaterial::ShaderModel": "Effect"},
        "BSMaterial::MaterialFlags":         flags,
    }

    return {"version": 2, "data": data}


# ---------------------------------------------------------------------------
# Writer
# ---------------------------------------------------------------------------

def write_mat(obj: dict, path: str) -> None:
    """Serialise a .mat dict to a JSON file."""
    import os
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, indent=2, ensure_ascii=False)
