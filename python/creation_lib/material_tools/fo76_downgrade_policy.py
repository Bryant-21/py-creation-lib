"""Declarative field-clear rules for FO76 -> FO4 material downgrades.

Only mechanical clears live here. Texture-slot remapping, static/object BGSM
``EmitEnabled`` suppression, ``Translucency`` -> ``SubsurfaceLighting`` value
preservation, and ``RootMaterialPath`` synthesis stay in
:mod:`creation_lib.material_tools.convert`.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .bgsm_bin import BGSMData
    from .bgem_bin import BGEMData


# ---------------------------------------------------------------------------
# BGSM
# ---------------------------------------------------------------------------

# FO76 PBR texture slots that do NOT exist in the FO4 v2 BGSM layout. The
# serializer in :class:`bgsm_bin.BGSMData.write` already skips them at
# ``header.version <= 2`` but we null them on the downgraded instance so
# nothing stale leaks through when inspecting the result in code.
_FO76_BGSM_PBR_TEXTURE_FIELDS: tuple[str, ...] = (
    "SpecularTexture",
    "LightingTexture",
    "FlowTexture",
    "DistanceFieldAlphaTexture",
)

# FO76 Translucency block (v>=8). FO4 v2 uses the older RimLighting block
# instead; convert.downgrade_bgsm handles the value-preserving parts
# (Translucency -> SubsurfaceLighting) explicitly before the clear pass.
_FO76_BGSM_TRANSLUCENCY_FIELDS: tuple[str, ...] = (
    "Translucency",
    "TranslucencyThickObject",
    "TranslucencyMixAlbedoWithSubsurfaceColor",
    "TranslucencySubsurfaceColor",
    "TranslucencyTransmissiveScale",
    "TranslucencyTurbulence",
)

#: All BGSM fields that the policy module may clear when downgrading to an
#: older version. This is the union of every rule-expressible drop — callers
#: can inspect it for audit purposes but should prefer
#: :func:`clear_fo76_only_bgsm_fields` which applies the version gating.
FO76_ONLY_BGSM_FIELDS: tuple[str, ...] = (
    _FO76_BGSM_PBR_TEXTURE_FIELDS + _FO76_BGSM_TRANSLUCENCY_FIELDS
)


def clear_fo76_only_bgsm_fields(data: "BGSMData", target_version: int) -> None:
    """Null out rule-expressible FO76-only BGSM fields in place; callers deep-copy first.

    PBR texture slots are cleared at ``target_version <= 2`` (absent from the FO4
    layout); the Translucency* block at ``target_version < 8`` (FO4 uses the older
    RimLighting block).
    """
    if target_version <= 2:
        for name in _FO76_BGSM_PBR_TEXTURE_FIELDS:
            setattr(data, name, None)
    if target_version < 8:
        for name in _FO76_BGSM_TRANSLUCENCY_FIELDS:
            setattr(data, name, None)


# ---------------------------------------------------------------------------
# BGEM
# ---------------------------------------------------------------------------

# FO76 v21+ glass refraction block. FO4 v20 BGEM has none of these.
# ``EffectPbrSpecular`` is not listed: it appears at BGEM v20, is valid on an
# FO4 v20 BGEM, and the writer emits it at ``version >= 20``.
_FO76_BGEM_GLASS_FIELDS: tuple[str, ...] = (
    "GlassRoughnessScratch",
    "GlassDirtOverlay",
    "GlassEnabled",
    "GlassFresnelColor",
    "GlassBlurScaleBase",
    "GlassBlurScaleFactor",
    "GlassRefractionScaleBase",
)

#: All BGEM fields the policy module may clear.
FO76_ONLY_BGEM_FIELDS: tuple[str, ...] = _FO76_BGEM_GLASS_FIELDS


def clear_fo76_only_bgem_fields(data: "BGEMData", target_version: int) -> None:
    """Null out rule-expressible FO76-only BGEM fields in place.

    Version gating:
      * Glass refraction block is cleared when ``target_version < 21``.
    """
    if target_version < 21:
        for name in _FO76_BGEM_GLASS_FIELDS:
            setattr(data, name, None)
