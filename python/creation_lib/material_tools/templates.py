"""FO4 BGSM root-material-template lookup.

FO76 stripped the root-template inheritance system — 99% of FO76 BGSMs ship
with ``RootMaterialPath`` empty, with the per-material file storing all shader
fields inline. FO4 on the other hand still uses the template chain: almost
every vanilla weapon/armor/clothes BGSM sets ``RootMaterialPath`` to something
under ``template/`` and inherits base shader params from there.

When downgrading a FO76 BGSM to FO4, leaving ``RootMaterialPath`` empty is
legal but produces subtly wrong visuals — the engine falls back to default
shader params instead of the per-category template's spec/metallic tuning.
This module provides a heuristic lookup that picks a sensible FO4 root
template from the source file path + the BGSM's own shader flags, so the
downgrade pass can synthesize a non-empty ``RootMaterialPath`` that points at
a known-real FO4 vanilla template.

Public API:
  resolve_root_material_path(source_path, bgsm) -> str | None
  KNOWN_FO4_TEMPLATES  (frozenset of canonical template names for validation)

Heuristic ordering (first match wins):
  1. Source BGSM already has a non-empty RootMaterialPath that names an
     existing FO4 template -> pass it through (case-normalized).
  2. Shader flags: Hair, Tree, SkinTint -> specific templates.
  3. Source path contains ``/weapons/`` -> WeaponMetalTemplate (default) or
     WeaponWoodTemplate / WeaponPlasticTemplate when the filename hints wood
     or plastic (``stock``, ``grip``, ``handle``, etc.).
  4. Source path contains ``/armor/`` -> ArmorTemplate.
  5. Source path contains ``/clothes/`` or ``/clothing/`` -> OutfitTemplate.
  6. Source path contains ``/actors/`` -> CreatureTemplate.
  7. Architecture / setdressing / landscape / effects / interface / misc
     paths get category-appropriate templates (metal, wood, default, etc).
  8. Catch-all fallback -> ``template/defaultTemplate_wet.bgsm``.

All synthesized paths use the canonical casing that appears in the FO4
``Data/Materials/template/`` directory on disk. A missing or invalid
RootMaterialPath causes the engine to load the material with default shader
params (the previous behavior); a BAD RootMaterialPath causes the material
to fail to load entirely, so the lookup MUST only return template names
that exist in vanilla FO4.
"""
from __future__ import annotations

from typing import Optional

from .bgsm_bin import BGSMData

# Canonical casing for every FO4 vanilla template under
# ``Data/Materials/template/`` (as it appears on disk in the extracted fo4
# data). Verified against ``extracted/fo4/Materials/template/`` listing.
# The engine is case-insensitive on Windows but we preserve the on-disk
# casing so xEdit / manual inspection shows something consistent with
# vanilla files.
KNOWN_FO4_TEMPLATES: frozenset[str] = frozenset({
    "template/ArmorTemplate_Wet.bgsm",
    "template/AsphaltTemplate.bgsm",
    "template/AsphaltTemplate_Wet.bgsm",
    "template/BrickTemplate.bgsm",
    "template/BrickTemplate_Wet.bgsm",
    "template/CapMetalTemplate_Wet.bgsm",
    "template/ClothTemplate_Cotton.bgsm",
    "template/ClothTemplate_Cotton_Wet.bgsm",
    "template/ClothTemplate_Felt.bgsm",
    "template/ClothTemplate_Felt_Wet.bgsm",
    "template/ClothTemplate_Wool.bgsm",
    "template/ClothTemplate_Wool_Wet.bgsm",
    "template/ConcreteTemplate_Wet.bgsm",
    "template/CrackedMudTemplate_Wet.bgsm",
    "template/CreatureTemplate_Wet.bgsm",
    "template/DefaultTemplateDim_Wet.bgsm",
    "template/DefaultTemplate_Wet.bgsm",
    "template/DirtTemplate_Wet.bgsm",
    "template/FurTemplate_Wet.bgsm",
    "template/GrassTemplate_Wet.BGSM",
    "template/LeafTemplate_Wet.bgsm",
    "template/MetalTemplate.bgsm",
    "template/MetalTemplate_Brushed.bgsm",
    "template/MetalTemplate_Chrome.bgsm",
    "template/MetalTemplate_Wet.bgsm",
    "template/OutfitTemplate2_Wet.bgsm",
    "template/OutfitTemplate3_Wet.bgsm",
    "template/OutfitTemplate4_Wet.bgsm",
    "template/OutfitTemplate5_Wet.bgsm",
    "template/OutfitTemplate_Wet.bgsm",
    "template/RockSlabTemplate_Wet.bgsm",
    "template/RockTemplate.bgsm",
    "template/RockTemplate_Wet.bgsm",
    "template/RubberTemplate_Wet.bgsm",
    "template/SkinTemplate_Wet.bgsm",
    "template/VehicleBusTemplate_Wet.bgsm",
    "template/VehicleTemplateDim_Wet.bgsm",
    "template/VehicleTemplate_Wet.bgsm",
    "template/WeaponMetalTemplate_Wet.bgsm",
    "template/WeaponPlasticTemplate_Wet.bgsm",
    "template/WeaponWoodTemplate_Wet.bgsm",
    "template/WoodTemplate.bgsm",
    "template/WoodTemplate_Wet.bgsm",
    "template/WoodTemplate_rough.bgsm",
    "template/WroughtIronMetalTemplate_Wet.bgsm",
    "template/basicrough.bgsm",
    "template/basicsmooth.bgsm",
})

# Lowercase lookup for pass-through normalization (the source field may use
# ``Template/`` or ``template/`` and any filename casing).
_TEMPLATE_BY_LOWER: dict[str, str] = {
    t.lower(): t for t in KNOWN_FO4_TEMPLATES
}

# Fallback when no heuristic matches. Chosen because it's the most-used
# "catch-all" template in FO4 vanilla non-weapon/non-armor BGSMs.
_FALLBACK_TEMPLATE = "template/defaultTemplate_wet.bgsm"

# Filename substrings that suggest a weapon part is wood rather than metal.
_WEAPON_WOOD_HINTS = (
    "stock", "grip", "handle", "handguard", "forestock", "forestk",
    "buttstock", "foregrip", "furniture", "wood",
)
# Filename substrings that suggest a weapon part is plastic / synthetic.
_WEAPON_PLASTIC_HINTS = (
    "plastic", "polymer", "synthetic", "rubber",
)


def _has_any(path: str, hints: tuple[str, ...]) -> bool:
    return any(hint in path for hint in hints)


def _structural_template_for(path: str) -> str:
    if _has_any(path, ("basicrough", "roughbasic")):
        return "template/basicrough.bgsm"
    if _has_any(path, ("basicsmooth", "smoothbasic")):
        return "template/basicsmooth.bgsm"
    if _has_any(path, ("defaultdim", " dim", "_dim", "dark")):
        return "template/DefaultTemplateDim_Wet.bgsm"
    if _has_any(path, ("vehiclebus", "/bus/", "bus_", "bus.")):
        return "template/VehicleBusTemplate_Wet.bgsm"
    if _has_any(path, ("vehicle", "/vehicles/", "/cars/", "/truck", "/car/")):
        return "template/VehicleTemplate_Wet.bgsm"
    if _has_any(path, ("capmetal", "capdetail", "/capsules/", "capsule")):
        return "template/CapMetalTemplate_Wet.bgsm"
    if _has_any(path, ("wrought", "railing", "standpipe", "streetlamp", "ironfence")):
        return "template/WroughtIronMetalTemplate_Wet.bgsm"
    if _has_any(path, ("chrome", "polishedmetal")):
        return "template/MetalTemplate_Chrome.bgsm"
    if _has_any(path, ("brushed", "baremetal")):
        return "template/MetalTemplate_Brushed.bgsm"
    if _has_any(path, ("asphalt", "ashphalt", "parking", "road", "pavement")):
        return "template/AsphaltTemplate_Wet.bgsm"
    if _has_any(path, ("crackedmud", "cracked_mud")):
        return "template/CrackedMudTemplate_Wet.bgsm"
    if _has_any(path, ("rockslab", "rock_slab", "fakerockslab")):
        return "template/RockSlabTemplate_Wet.bgsm"
    if _has_any(path, ("rock", "rocks/", "stone", "cliff", "boulder")):
        return "template/RockTemplate_Wet.bgsm"
    if _has_any(path, ("dirt", "mud", "soil", "gravel", "forestfloor")):
        return "template/DirtTemplate_Wet.bgsm"
    if _has_any(path, ("rubber", "tire", "tyre")):
        return "template/RubberTemplate_Wet.bgsm"
    if _has_any(path, ("felt", "hide", "leatherhide")):
        return "template/ClothTemplate_Felt_Wet.bgsm"
    if _has_any(path, ("wool",)):
        return "template/ClothTemplate_Wool_Wet.bgsm"
    if _has_any(path, ("cloth", "fabric", "awning", "canvas", "cotton", "tarp")):
        return "template/ClothTemplate_Cotton_Wet.bgsm"
    if _has_any(path, ("wood", "timber", "log", "bark", "stump")):
        return "template/WoodTemplate_Wet.bgsm"
    if _has_any(path, ("metal", "steel", "iron", "aluminum", "aluminium")):
        return "template/MetalTemplate_Wet.bgsm"
    if _has_any(path, ("brick",)):
        return "template/BrickTemplate_Wet.bgsm"
    if _has_any(path, ("concrete", "cement", "plaster", "tile", "linoleum")):
        return "template/ConcreteTemplate_Wet.bgsm"
    return _FALLBACK_TEMPLATE


def _outfit_template_for(path: str) -> str:
    if _has_any(path, ("courser", "silvershroud")):
        return "template/OutfitTemplate2_Wet.bgsm"
    if _has_any(path, ("childrenofatom", "hancock", "chef")):
        return "template/OutfitTemplate3_Wet.bgsm"
    if _has_any(path, ("bathrobe", "desdemona", "father", "labcoat", "prewar")):
        return "template/OutfitTemplate4_Wet.bgsm"
    if _has_any(path, ("housedress", "mobster", "piper", "derby")):
        return "template/OutfitTemplate5_Wet.bgsm"
    return "template/OutfitTemplate_Wet.bgsm"


def _actor_template_for(path: str) -> str:
    if _has_any(path, ("fur", "dogmeat", "/cat/", "seagull", "crow", "feather")):
        return "template/FurTemplate_Wet.bgsm"
    if _has_any(path, ("robot", "protectron", "sentrybot", "assaultron", "eyebot", "handy")):
        return "template/ArmorTemplate_Wet.bgsm"
    return "template/CreatureTemplate_Wet.bgsm"


def _normalize_template(raw: str) -> Optional[str]:
    """If ``raw`` refers to a known FO4 template, return canonical casing.

    ``raw`` may be ``Template/Foo.bgsm``, ``template/foo.bgsm``, or
    ``materials/template/foo.bgsm``. Returns None if it doesn't match any
    known vanilla FO4 template.
    """
    if not raw:
        return None
    p = raw.replace("\\", "/").strip().lstrip("/")
    if p.lower().startswith("materials/"):
        p = p[len("materials/"):]
    return _TEMPLATE_BY_LOWER.get(p.lower())


def _weapon_template_for(lower_path: str) -> str:
    """Pick a weapon template (metal / wood / plastic) from the path hints."""
    for hint in _WEAPON_WOOD_HINTS:
        if hint in lower_path:
            return "template/WeaponWoodTemplate_Wet.bgsm"
    for hint in _WEAPON_PLASTIC_HINTS:
        if hint in lower_path:
            return "template/WeaponPlasticTemplate_Wet.bgsm"
    return "template/WeaponMetalTemplate_Wet.bgsm"


def resolve_root_material_path(
    source_path: str,
    bgsm: BGSMData,
) -> Optional[str]:
    """Return a canonical FO4 root template path for the given BGSM.

    Args:
        source_path: Game-relative source path of the BGSM, e.g.
            ``"materials/weapons/gausspistol/gausspistol.bgsm"``. Used for
            category detection via path segments. May be empty.
        bgsm: The BGSMData being downgraded. Shader flags (Hair, Tree,
            SkinTint) are consulted ahead of the path heuristic.

    Returns:
        A template path under ``template/`` that exists in vanilla FO4, or
        ``None`` if no plausible category was found (in which case the
        caller should leave ``RootMaterialPath`` empty — the old behavior).
    """
    # 1. If the source BGSM already names a valid FO4 template, keep it.
    existing = getattr(bgsm, "RootMaterialPath", None)
    if existing:
        cleaned = existing.replace("\x00", "").strip()
        normalized = _normalize_template(cleaned)
        if normalized:
            return normalized

    # 2. Shader-flag-driven matches take priority over path heuristics.
    if getattr(bgsm, "SkinTint", False):
        return "template/SkinTemplate_Wet.bgsm"
    if getattr(bgsm, "Hair", False):
        # No dedicated hair template in vanilla FO4 — skin template is the
        # closest match and is what Bethesda uses for hair BGSMs in-house.
        return "template/SkinTemplate_Wet.bgsm"

    # 3. Path-driven matches. Normalize to lowercase with forward slashes.
    p = (source_path or "").replace("\\", "/").lower().lstrip("/")
    if p.startswith("materials/"):
        p = p[len("materials/"):]

    if not p:
        if getattr(bgsm, "Tree", False):
            return "Template/LeafTemplate_Wet.bgsm"
        return None

    if "landscape/grass/" in p:
        return "Template/GrassTemplate_Wet.BGSM"
    if "landscape/plants/" in p or "landscape/trees/" in p:
        return "Template/LeafTemplate_Wet.bgsm"
    if getattr(bgsm, "Tree", False):
        return "Template/LeafTemplate_Wet.bgsm"

    # ATX weapon skins live at ``atx/weapons/...`` in FO76 — treat them as
    # weapons for template purposes (they're paint variants of weapon parts).
    if "atx/weapons/" in p or p.startswith("atx/weapons/"):
        return _weapon_template_for(p)

    if "/weapons/" in p or p.startswith("weapons/"):
        return _weapon_template_for(p)

    if "/armor/" in p or p.startswith("armor/") \
       or "/armors/" in p or p.startswith("armors/"):
        return "template/ArmorTemplate_Wet.bgsm"

    if "/clothes/" in p or p.startswith("clothes/") \
       or "/clothing/" in p or p.startswith("clothing/"):
        return _outfit_template_for(p)

    if "/actors/" in p or p.startswith("actors/") \
       or "/creatures/" in p or p.startswith("creatures/") \
       or "/creature/" in p or p.startswith("creature/"):
        return _actor_template_for(p)

    if "/gore/" in p or p.startswith("gore/"):
        if _has_any(p, ("organ", "intestine", "guts")):
            return "template/basicsmooth.bgsm"
        return "template/SkinTemplate_Wet.bgsm"

    # Architecture and setdressing use material-type keywords in the path
    # (wood/metal/brick/concrete) more reliably than a single category
    # template — fall through to a keyword scan before the catch-all.
    structural_cats = (
        "/architecture/", "/setdressing/", "/landscape/",
        "/construction/", "/furniture/", "/props/", "/vehicles/",
        "architecture/", "setdressing/", "landscape/",
        "construction/", "furniture/", "props/", "vehicles/",
    )
    if any(seg in p for seg in structural_cats) or p.startswith(
        (
            "architecture/",
            "setdressing/",
            "landscape/",
            "construction/",
            "furniture/",
            "props/",
            "vehicles/",
        )
    ):
        return _structural_template_for(p)

    # Effects, interface, sky, etc. — leave empty so the engine uses its own
    # defaults (effect shaders don't honor RootMaterialPath anyway).
    if "/effects/" in p or p.startswith("effects/"):
        return None
    if "/interface/" in p or "/menu/" in p:
        return None
    if "/sky/" in p or p.startswith("sky/"):
        return None

    # Catch-all: default template is safe for unknown categories.
    return _FALLBACK_TEMPLATE
