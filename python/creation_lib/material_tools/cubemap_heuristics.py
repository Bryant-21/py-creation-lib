"""FO4 environment cubemap heuristic selection.

FO76→FO4 BGSM/BGEM downgrade strips ``EnvmapTexture`` because FO76 PBR has no
spec-gloss cubemap concept. FO4 vanilla materials almost always set one — the
audit at ``cubemap_audit.txt`` shows weapons are 60%+ ``mipblur_DefaultOutside1``
plus the dielectric / bronze / copper variants for grips and accents. Without a
cubemap, FO4 metal surfaces render flat-grey — visually obvious on weapons.

This module centralizes cubemap selection for all three downgrade call sites
(BGSM, BGEM, inline NIF ``BSLightingShaderProperty``) so behavior stays
consistent.

Public API:
  select_cubemap(source_path, mat) -> tuple[cubemap_path | None, scale | None]

Heuristic ordering (first match wins; cubemap names verified against
``extracted/fo4/Textures/Shared/Cubemaps/``):

  1. Path exclusions: effects/, interface/, menu/, sky/, decals/ -> (None, None)
  2. Texture-name keywords on Diffuse/Normal/Specular (case-insensitive substring)
  3. Path-driven category defaults (weapons, actors skin, armor metal, ...)
  4. Fallback: mipblur_DefaultOutside1.dds, scale 1.0

Returning ``(None, None)`` means leave EnvmapTexture empty — appropriate for
effect shaders, UI, sky, decals where cubemap reflection is wrong.
"""

from __future__ import annotations

import logging
import os
from typing import Any

_log = logging.getLogger(__name__)


# Universally-safe FO4 cubemaps (verified present in
# ``extracted/fo4/Textures/Shared/Cubemaps/``). Scene-specific cubemaps
# (CGPlayerHouseCube, MemoryDenCube, Vault111CryoCube, MuseumIntCube01_e,
# ConcordeStreetCube_e, OutsideMetalOldTownCube_e) are NOT used as defaults
# even when they appear in vanilla data — they bake scene lighting and look
# wrong outside their authored location.
_DEFAULT_OUTSIDE = "Shared/Cubemaps/mipblur_DefaultOutside1.dds"
_DEFAULT_DIELECTRIC = "Shared/Cubemaps/mipblur_DefaultOutside1_dielectric.dds"
_DEFAULT_COPPER = "Shared/Cubemaps/mipblur_DefaultOutside1_Copper.dds"
_DEFAULT_BRONZE = "Shared/Cubemaps/mipblur_DefaultOutside1_bronze.dds"


# Texture-keyword overrides. Case-insensitive substring on the resolved
# Diffuse/Normal/Specular texture filenames.
_TEXTURE_KEYWORD_RULES: tuple[tuple[str, str, float], ...] = (
    ("chrome", "Shared/Cubemaps/MetalChrome01Cube_e.dds", 1.0),
    ("copper", "Shared/Cubemaps/MetalCopperShine01Cube_e.dds", 1.0),
    ("bronze", "Shared/Cubemaps/MetalBronzeCube_e.dds", 1.0),
    ("gold", "Shared/Cubemaps/MetalBrushedGold_e.dds", 1.0),
    ("brushed", "Shared/Cubemaps/MetalBrushed01Cube_e.dds", 1.0),
    ("glass", "Shared/Cubemaps/mipblur_DefaultOutside1.dds", 0.5),
    ("eye", "Shared/Cubemaps/EyeCubeMap.dds", 1.0),
    ("oil", "Shared/Cubemaps/Oil_e.dds", 1.0),
)


# Path segments that mean "no cubemap" — effect shaders, sky, UI use a
# different reflection path or none at all.
_EXCLUDED_PATH_SEGMENTS = (
    "effects/",
    "interface/",
    "menu/",
    "sky/",
    "decals/",
)


# Filename hints suggesting a non-metal surface (rubber grips, plastic
# accents, cloth wraps) — picks dielectric variant + lower mask scale.
_DIELECTRIC_HINTS = (
    "rubber",
    "plastic",
    "polymer",
    "synthetic",
    "leather",
    "cloth",
    "fabric",
    "wood",
    "stock",
    "grip",
    "handle",
    "handguard",
    "foregrip",
    "buttstock",
)


# Filename hints suggesting metal — used for architecture/setdressing/armor
# splits where the path alone is ambiguous.
_METAL_HINTS = (
    "metal",
    "steel",
    "iron",
    "alum",
    "barrel",
    "receiver",
    "frame",
    "bolt",
)


# Cached cubemap inventory — populated lazily on first call so we can warn
# when a heuristic asks for a file that the FO4 install doesn't ship.
_CUBEMAP_INVENTORY: set[str] | None = None
_INVENTORY_LOGGED_MISSING: set[str] = set()


def _load_cubemap_inventory() -> set[str]:
    """Return the lowercase set of cubemap filenames present in the local
    FO4 extraction. Empty set if the directory isn't present (development
    environment without extracted data — heuristic still runs but we can't
    validate).
    """
    global _CUBEMAP_INVENTORY
    if _CUBEMAP_INVENTORY is not None:
        return _CUBEMAP_INVENTORY

    inventory: set[str] = set()
    cubemap_dir = os.path.join(
        "extracted", "fo4", "Textures", "Shared", "Cubemaps"
    )
    if os.path.isdir(cubemap_dir):
        for entry in os.listdir(cubemap_dir):
            if entry.lower().endswith(".dds"):
                inventory.add(entry.lower())
    _CUBEMAP_INVENTORY = inventory
    return inventory


def _verify_cubemap(name: str) -> str:
    """Return ``name`` if its filename exists in the FO4 cubemap dir,
    otherwise fall back to ``mipblur_DefaultOutside1.dds`` and warn once.
    Bypasses verification when the local FO4 inventory is empty (CI / dev
    env without extracted data) — assume the cubemap exists in real install.
    """
    inventory = _load_cubemap_inventory()
    if not inventory:
        return name
    filename = name.replace("\\", "/").rsplit("/", 1)[-1].lower()
    if filename in inventory:
        return name
    if name not in _INVENTORY_LOGGED_MISSING:
        _INVENTORY_LOGGED_MISSING.add(name)
        _log.warning(
            "Cubemap %r missing from extracted/fo4/Textures/Shared/Cubemaps; "
            "falling back to mipblur_DefaultOutside1.dds",
            name,
        )
    return _DEFAULT_OUTSIDE


def _gather_texture_strings(mat: Any) -> str:
    """Concatenate all texture-slot strings from a BGSMData/BGEMData-like
    object into one lowercase blob for substring scanning.
    """
    parts: list[str] = []
    for attr in (
        # BGSM slots
        "DiffuseTexture",
        "NormalTexture",
        "SmoothSpecTexture",
        "SpecularTexture",
        "GlowTexture",
        # BGEM slots
        "BaseTexture",
        "EnvmapMaskTexture",
    ):
        v = getattr(mat, attr, None)
        if isinstance(v, str) and v:
            parts.append(v)
    return " ".join(parts).lower()


def _normalize_path(p: str) -> str:
    """Lowercase + forward-slash + strip ``materials/`` prefix."""
    p = (p or "").replace("\\", "/").lower().lstrip("/")
    if p.startswith("materials/"):
        p = p[len("materials/") :]
    return p


def select_cubemap(
    source_path: str,
    mat: Any,
) -> tuple[str | None, float | None]:
    """Select an FO4 environment cubemap for a downgraded BGSM/BGEM material.

    Args:
        source_path: Game-relative source path of the material (e.g.
            ``"materials/weapons/gausspistol/foo.bgsm"``). May be empty;
            heuristic still runs against the texture-name keywords.
        mat: A BGSMData/BGEMData-like object with texture-slot string
            attributes (DiffuseTexture, NormalTexture, etc.). Only the
            attribute access is used — duck-typed for synthetic test fakes.

    Returns:
        ``(cubemap_path, env_mapping_mask_scale)`` where both are ``None``
        when the material category should leave EnvmapTexture empty
        (effects/UI/sky/decals). Otherwise ``cubemap_path`` is a Data-relative
        path under ``Shared/Cubemaps/`` and ``scale`` is the recommended
        ``EnvironmentMappingMaskScale`` value (FO4 BGEMs only — BGSM callers
        ignore the scale).
    """
    norm_path = _normalize_path(source_path)

    # 1. Exclusions — effect shaders, UI, sky, decals don't use cubemap.
    for seg in _EXCLUDED_PATH_SEGMENTS:
        if seg in norm_path or norm_path.startswith(seg):
            return (None, None)

    tex_blob = _gather_texture_strings(mat)

    # 2. Texture-name keyword overrides take priority over path defaults.
    for keyword, cubemap, scale in _TEXTURE_KEYWORD_RULES:
        if keyword in tex_blob:
            return (_verify_cubemap(cubemap), scale)

    # 3. Path-driven category defaults.
    if not norm_path:
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    is_skin = bool(getattr(mat, "SkinTint", False) or getattr(mat, "Hair", False))
    rmp = (getattr(mat, "RootMaterialPath", "") or "").lower()
    is_creature_template = "creaturetemplate" in rmp or "skintemplate" in rmp

    # Weapons. Rubber/wood/leather grips get dielectric.
    if "/weapons/" in norm_path or norm_path.startswith("weapons/"):
        if any(h in norm_path for h in _DIELECTRIC_HINTS):
            return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    # ATX weapons (FO76 paint variants under atx/weapons/).
    if "atx/weapons/" in norm_path or norm_path.startswith("atx/weapons/"):
        if any(h in norm_path for h in _DIELECTRIC_HINTS):
            return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    # Actors — skin, hair, creature templates get dielectric (low mask).
    if "/actors/" in norm_path or norm_path.startswith("actors/") \
       or "/creatures/" in norm_path or norm_path.startswith("creatures/") \
       or "/critters/" in norm_path or norm_path.startswith("critters/"):
        if is_skin or is_creature_template:
            return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)
        # Fallback for actor accessories without skin flag.
        return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)

    # Architecture / setdressing / landscape — metal hint vs non-metal.
    if any(seg in norm_path for seg in (
        "/architecture/", "architecture/",
        "/setdressing/", "setdressing/",
        "/interiors/", "interiors/",
        "/landscape/", "landscape/",
        "/props/", "props/",
        "/furniture/", "furniture/",
    )):
        if any(h in norm_path for h in _METAL_HINTS):
            return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)
        return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)

    # Armor — metal hint vs cloth/leather hint.
    if "/armor/" in norm_path or norm_path.startswith("armor/") \
       or "/armors/" in norm_path or norm_path.startswith("armors/"):
        if any(h in norm_path for h in _DIELECTRIC_HINTS):
            return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)
        if any(h in norm_path for h in _METAL_HINTS):
            return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)
        # Default armor to outside (metal is more common than cloth in FO4).
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    # Clothes — almost always cloth/leather, dielectric.
    if "/clothes/" in norm_path or norm_path.startswith("clothes/") \
       or "/clothing/" in norm_path or norm_path.startswith("clothing/"):
        return (_verify_cubemap(_DEFAULT_DIELECTRIC), 0.3)

    # Vehicles — outside cubemap.
    if "/vehicles/" in norm_path or norm_path.startswith("vehicles/"):
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    # Ammo — small metal cartridges, outside.
    if "/ammo/" in norm_path or norm_path.startswith("ammo/"):
        return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)

    # 4. Fallback — outside cubemap is the safest default.
    return (_verify_cubemap(_DEFAULT_OUTSIDE), 1.0)
