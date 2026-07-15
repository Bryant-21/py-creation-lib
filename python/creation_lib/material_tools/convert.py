"""Cross-game BGSM/BGEM version conversion.

BGSM versions:
  FO4 = 2   — uses EnvmapTexture/InnerLayerTexture/DisplacementTexture; RimLighting block
  FO76 = 20 — uses SpecularTexture/LightingTexture/FlowTexture; Translucency block

BGEM versions:
  FO4 = 2   — minimal effect shader; no PBR/Glass fields
  FO76 ≈ 22 — adds GlassRoughnessScratch, GlassDirtOverlay, Glass* floats

Public API:
  BGSM_VERSION_FO4, BGEM_VERSION_FO4
  downgrade_bgsm(bgsm, target_version) -> BGSMData
  downgrade_bgem(bgem, target_version) -> BGEMData
"""
from __future__ import annotations

from .bgsm_bin import BGSMData
from .bgem_bin import BGEMData
from .fo76_downgrade_policy import (
    clear_fo76_only_bgem_fields,
    clear_fo76_only_bgsm_fields,
)
from .templates import resolve_root_material_path

# Target versions for FO4 materials
BGSM_VERSION_FO4 = 2
BGEM_VERSION_FO4 = 2


def _apply_vegetation_material_defaults(bgsm: BGSMData) -> None:
    root = (bgsm.RootMaterialPath or "").replace("\x00", "").strip().lower()
    if "leaftemplate_wet" in root:
        bgsm.BackLighting = True
        bgsm.BackLightPower = 0.25
        bgsm.SubsurfaceLighting = True
        bgsm.SubsurfaceLightingRolloff = 2.0
        bgsm.EnvmapTexture = ""
    elif "grasstemplate_wet" in root:
        bgsm.SubsurfaceLighting = True
        bgsm.EnvmapTexture = ""


def _rewrite_fo76_reflectivity_suffix(path: str) -> str:
    normalized = path.replace("\\", "/")
    directory, _, basename = normalized.rpartition("/")
    stem, dot, ext = basename.rpartition(".")
    if not dot:
        stem, ext = basename, ""
    lower = stem.lower()
    if lower.endswith("_r"):
        basename = f"{stem[:-2]}_s{dot}{ext}"
    return f"{directory}/{basename}" if directory else basename


def _suppress_fo76_bgsm_emittance(source_path: str | None, source_version: int) -> bool:
    if source_version <= BGSM_VERSION_FO4:
        return False
    path = (source_path or "").replace("\\", "/").strip("/")
    relative = path.removeprefix("Materials/").removeprefix("materials/")
    lower = relative.lower()
    return not (lower.startswith("effects/") or lower.startswith("decals/"))


def downgrade_bgsm(
    bgsm: BGSMData,
    target_version: int = BGSM_VERSION_FO4,
    source_path: str | None = None,
) -> BGSMData:
    """Convert a BGSMData to a lower version format.

    FO76 (v>2) → FO4 (v2) changes:
    - Texture layout: FO76 drops EnvmapTexture/InnerLayerTexture/DisplacementTexture
      and adds SpecularTexture/LightingTexture/FlowTexture.  We map them back.
    - Lighting block: FO76 v>=8 replaces RimLighting/Subsurface with Translucency.
      We restore the older fields with neutral defaults.
    - WetnessControlEnvMapScale: removed in FO76 (v>=10). Restore with 0.0.
    - Version-gated fields (PBR, LumEmittance, AdaptativeEmissive, etc.) are
      stripped automatically because BGSMData.write() is version-conditional.
    """
    if bgsm.header.version <= target_version:
        return bgsm  # nothing to do

    import copy
    result = copy.deepcopy(bgsm)
    src_v = bgsm.header.version

    # ---- Texture slot remapping ----
    # v>2 texture order: Diffuse, Normal, SmoothSpec, Greyscale,
    #                    Glow, Wrinkles, Specular, Lighting, Flow, [DistField]
    # v<=2 texture order: Diffuse, Normal, SmoothSpec, Greyscale,
    #                     Envmap, Glow, InnerLayer, Wrinkles, Displacement
    #
    # FO76 PBR slots (Specular = roughness/reflectivity, Lighting = emissive
    # rolloff, Flow = anisotropic flow map) have NO semantic equivalent in
    # FO4's spec-gloss model. The previous attempt to "preserve" them by
    # writing into Envmap/InnerLayer/Displacement caused the FO76 _s.dds
    # roughness texture to be sampled as a cubemap reflection on FO4's
    # render path — visibly wrong (mirror-shiny weapons everywhere).
    #
    # Correct approach: only promote SpecularTexture into SmoothSpecTexture
    # when the source has no SmoothSpec (FO76 PBR materials sometimes leave
    # the gloss map slot empty and put the data in Specular). Leave Envmap,
    # InnerLayer, Displacement EMPTY — the engine falls back to the cubemap
    # referenced by the BGSM's RootMaterialPath template (or to the default
    # gray cubemap if there is no template).
    if src_v > 2:
        # Helper: BGSM string fields can hold a literal NUL byte (``'\x00'``)
        # for "empty" instead of an empty Python string. Strip nulls and
        # whitespace before any truthiness check or copy-across.
        def _clean(s: str | None) -> str:
            if not s:
                return ""
            return s.replace("\x00", "").strip()

        # Preserve semantically-matching slots
        result.GlowTexture = _clean(bgsm.GlowTexture) or None
        result.WrinklesTexture = _clean(bgsm.WrinklesTexture) or None
        smoothspec_clean = _clean(bgsm.SmoothSpecTexture)
        specular_clean = _clean(bgsm.SpecularTexture)
        # Promote FO76 SpecularTexture (PBR roughness map) into FO4
        # SmoothSpecTexture only if the source SmoothSpec slot is empty
        # — which is the common case in FO76 PBR materials.
        if not smoothspec_clean and specular_clean:
            result.SmoothSpecTexture = _rewrite_fo76_reflectivity_suffix(specular_clean)
        else:
            result.SmoothSpecTexture = _rewrite_fo76_reflectivity_suffix(smoothspec_clean)
        # FO4-only texture slots — InnerLayer/Displacement are vestigial
        # (FO4 vanilla never populates them) so always clear. EnvmapTexture
        # gets injected from a heuristic because FO4 vanilla weapon BGSMs
        # almost always set a Shared/Cubemaps/* reflection cubemap and the
        # downgrade path strips it (FO76 PBR has no equivalent slot).
        # Without the cubemap, FO4 metal renders flat-grey.
        from .cubemap_heuristics import select_cubemap
        cubemap, scale = select_cubemap(source_path or "", bgsm)
        result.EnvmapTexture = cubemap or ""
        result.header.env_mapping = bool(cubemap)
        result.header.env_mapping_mask_scale = float(scale or 1.0)
        result.InnerLayerTexture = ""
        result.DisplacementTexture = ""
        lighting_clean = _clean(bgsm.LightingTexture)
        glow_clean = _clean(result.GlowTexture) if result.GlowTexture else ""
        if _suppress_fo76_bgsm_emittance(source_path, src_v) and bgsm.EmitEnabled:
            result.EmitEnabled = False
            result.Glowmap = False
            result.GlowTexture = None
            result.EmittanceColor = None
            result.EmittanceMult = 1.0
            result.ExternalEmittance = False
        elif bgsm.EmitEnabled and lighting_clean and not glow_clean:
            result.GlowTexture = lighting_clean
            result.Glowmap = True
            if result.EmittanceMult > 1.0:
                result.EmittanceMult = 1.0

    # ---- Lighting block: Translucency (v>=8) → RimLighting (v<8) ----
    if src_v >= 8:
        # Preserve translucency intent as subsurface lighting if it was on
        had_translucency = bool(bgsm.Translucency)
        result.RimLighting = False
        result.RimPower = 2.0
        result.BackLightPower = 0.0
        result.SubsurfaceLighting = had_translucency
        result.SubsurfaceLightingRolloff = float(bgsm.TranslucencyTransmissiveScale or 0.3)

    # Clear rule-expressible FO76-only fields (PBR texture slots at v<=2,
    # Translucency block at v<8). Non-rule-expressible value preservation
    # (SmoothSpec promotion, static-emittance suppression, the
    # SubsurfaceLighting values above) has already been applied above.
    clear_fo76_only_bgsm_fields(result, target_version)

    # ---- BackLighting: present in v<8 only ----
    if result.BackLighting is None:
        result.BackLighting = False

    # ---- WetnessControlEnvMapScale: removed in v>=10, needed in v<10 ----
    if src_v >= 10 and result.WetnessControlEnvMapScale is None:
        result.WetnessControlEnvMapScale = 0.0

    # ---- RootMaterialPath: FO76 ships this empty on 99% of BGSMs; FO4
    # vanilla relies on it for per-category shader param inheritance.
    # Synthesize a plausible FO4 template path from the source path and
    # shader flags. Leaves it empty only if no category matches.
    synthesized = resolve_root_material_path(source_path or "", bgsm)
    if synthesized is not None:
        result.RootMaterialPath = synthesized
    else:
        # Normalize any residual NULs from the source field so we don't
        # emit an invalid reference downstream.
        current = (result.RootMaterialPath or "").replace("\x00", "").strip()
        result.RootMaterialPath = current
    _apply_vegetation_material_defaults(result)

    # Set the target version on the header
    result.header.version = target_version

    return result


def downgrade_bgem(
    bgem: BGEMData,
    target_version: int = BGEM_VERSION_FO4,
    source_path: str | None = None,
) -> BGEMData:
    """Convert a BGEMData to a lower version format.

    FO76 (v>=21) adds Glass* fields not present in FO4.  Setting the version
    to target_version causes BGEMData.write() to skip those fields automatically.

    Heuristically populates ``EnvmapTexture`` + ``EnvironmentMapping`` +
    ``EnvironmentMappingMaskScale`` from ``source_path`` when the source did
    not set them — symmetric to the BGSM downgrade. Effect-shader paths
    (effects/, decals/, sky/, UI) are correctly skipped by the heuristic.
    """
    if bgem.header.version <= target_version:
        return bgem  # nothing to do

    import copy
    result = copy.deepcopy(bgem)
    result.header.version = target_version

    # Clear rule-expressible FO76-only fields (glass block at v<21,
    # EffectPbrSpecular at v<20).
    clear_fo76_only_bgem_fields(result, target_version)

    # Inject env cubemap when the heuristic agrees this material category
    # should reflect the environment. Only populate when slot is empty —
    # respect any value already authored on the source BGEM.
    from .cubemap_heuristics import select_cubemap
    cubemap, scale = select_cubemap(source_path or "", bgem)
    if cubemap:
        existing = (result.EnvmapTexture or "").replace("\x00", "").strip()
        if not existing:
            result.EnvmapTexture = cubemap
        if result.EnvironmentMapping is None or result.EnvironmentMapping is False:
            result.EnvironmentMapping = True
        # Preserve any non-zero authored scale; only fill defaults.
        if (
            result.EnvironmentMappingMaskScale is None
            or result.EnvironmentMappingMaskScale == 0.0
        ):
            result.EnvironmentMappingMaskScale = float(scale or 1.0)

    return result
