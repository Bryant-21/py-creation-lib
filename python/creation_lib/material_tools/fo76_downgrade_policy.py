"""Rule-expressible drop/keep policy for FO76 -> FO4 material downgrades.

This module holds ONLY the mechanical field-clear rules that are safe to
express declaratively. Non-rule-expressible logic (texture slot remapping,
static/object BGSM ``EmitEnabled`` suppression,
``Translucency`` -> ``SubsurfaceLighting`` value preservation, and
``RootMaterialPath`` synthesis) stays in :mod:`creation_lib.material_tools.convert`
where the surrounding bug-fix comments provide institutional memory for real
incidents (mirror-shiny weapons, whole-object emittance, template inheritance).

Public API:
    FO76_ONLY_BGSM_FIELDS
    FO76_ONLY_BGEM_FIELDS
    clear_fo76_only_bgsm_fields(data, target_version)
    clear_fo76_only_bgem_fields(data, target_version)
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
    """Null out rule-expressible FO76-only BGSM fields in place.

    This operates on ``data`` directly — callers are expected to have already
    made a deep copy (see :func:`creation_lib.material_tools.convert.downgrade_bgsm`).

    Version gating:
      * PBR texture slots (SpecularTexture / LightingTexture / FlowTexture /
        DistanceFieldAlphaTexture) are cleared when ``target_version <= 2``
        because the FO4 BGSM layout doesn't have them.
      * Translucency block (Translucency* fields) is cleared when
        ``target_version < 8`` because FO4 uses the older RimLighting block.
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
#
# Note: ``EffectPbrSpecular`` is intentionally NOT in this list. It was
# added at BGEM v20 and is a valid field on an FO4 v20 BGEM — the writer
# emits it at ``version >= 20``. The old ``BGEMData._FO76_ONLY_FIELDS``
# class var (now deleted) incorrectly included it; the ground-truth
# ``convert.downgrade_bgem`` did not clear it, which is what we preserve.
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
