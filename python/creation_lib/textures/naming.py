"""Texture filename suffix conversion between game naming conventions.

Uses GameProfile.texture_suffixes to map semantic roles (diffuse, normal,
specular, etc.) to game-specific filename suffixes (_d, _n, _s, _color, etc.).
"""
from __future__ import annotations

from pathlib import PurePosixPath

from creation_lib.core.game_profiles import GameProfile

# Fallback mappings for roles that don't exist in the target game.
# e.g. FO76 "lighting" (_l emissive rolloff) -> FO4 "glow" (_g) when the
# BGSM downgrade promotes LightingTexture into GlowTexture. Not "specular",
# which would collide with the SmoothSpecTexture slot (two BGSM slots
# pointing at the same _s.dds).
_ROLE_FALLBACKS: dict[str, str] = {
    "lighting": "glow",
    "reflectivity": "specular",
    "roughness": "specular",
    "metallic": "specular",
    "specular": "reflectivity",
}


def detect_texture_role(filename: str, profile: GameProfile) -> str | None:
    """Detect the semantic role of a texture from its filename suffix.

    Returns the role name (e.g. "diffuse", "normal", "specular") or None
    if no recognized suffix matches.
    """
    stem = PurePosixPath(filename).stem.lower()
    # Sort by suffix length descending so "_normal" matches before "_n"
    for role, suffix in sorted(profile.texture_suffixes.items(),
                                key=lambda x: len(x[1]), reverse=True):
        if stem.endswith(suffix.lower()):
            return role
    return None


def convert_texture_name(
    filename: str,
    source_profile: GameProfile,
    target_profile: GameProfile,
) -> str:
    """Convert a texture filename from source game naming to target game naming.

    Detects the semantic role via the source profile's suffixes, then replaces
    the suffix with the target profile's equivalent. Returns the filename
    unchanged if no recognized suffix is found or the target has no mapping
    for that role.
    """
    role = detect_texture_role(filename, source_profile)
    if role is None:
        return filename

    source_suffix = source_profile.texture_suffixes[role]
    target_suffix = target_profile.texture_suffixes.get(role)
    if target_suffix is None:
        # Try fallback mapping
        alt_role = _ROLE_FALLBACKS.get(role)
        if alt_role:
            target_suffix = target_profile.texture_suffixes.get(alt_role)
        if target_suffix is None:
            return filename

    # Replace suffix in filename
    stem = PurePosixPath(filename).stem
    ext = PurePosixPath(filename).suffix  # .dds

    stem_lower = stem.lower()
    suffix_lower = source_suffix.lower()
    idx = stem_lower.rfind(suffix_lower)
    if idx < 0:
        return filename

    new_stem = stem[:idx] + target_suffix
    return new_stem + ext
