"""Extract texture paths from BGSM/BGEM material files.

Wraps the existing bgsm_bin/bgem_bin parsers to provide a simple
dict of {slot_name: texture_path} for database indexing.
"""
from __future__ import annotations

import io
import os

# BGSM texture field names in order (matching bgsm_bin.py field order)
BGSM_TEXTURE_SLOTS = [
    "DiffuseTexture", "NormalTexture", "SmoothSpecTexture",
    "GreyscaleTexture", "EnvmapTexture", "GlowTexture",
    "InnerLayerTexture", "WrinklesTexture", "DisplacementTexture",
    "SpecularTexture", "LightingTexture", "FlowTexture",
    "DistanceFieldAlphaTexture",
]

# BGEM texture field names in order (matching bgem_bin.py field order)
BGEM_TEXTURE_SLOTS = [
    "BaseTexture", "GrayscaleTexture", "EnvmapTexture",
    "NormalTexture", "EnvmapMaskTexture", "SpecularTexture",
    "LightingTexture", "GlowTexture",
    "GlassRoughnessScratch", "GlassDirtOverlay",
]


def _normalize_path(p: str) -> str:
    """Normalize a texture path to lowercase forward-slash relative."""
    return p.replace("\\", "/").strip().lower()


def parse_material_textures(filepath: str) -> dict | None:
    """Parse a BGSM or BGEM file and return texture slot mappings.

    Returns:
        {"type": "bgsm"|"bgem", "textures": {"SlotName": "path", ...}}
        or None if file is missing/invalid.
    """
    if not os.path.isfile(filepath):
        return None

    ext = os.path.splitext(filepath)[1].lower()
    try:
        if ext == ".bgsm":
            return _parse_bgsm(filepath)
        elif ext == ".bgem":
            return _parse_bgem(filepath)
    except Exception:
        return None
    return None


def _parse_bgsm(filepath: str) -> dict | None:
    """Parse BGSM and extract texture paths."""
    from creation_lib.material_tools.bgsm_bin import read_bgsm
    try:
        with open(filepath, "rb") as f:
            mat = read_bgsm(f)
    except Exception:
        return None

    textures = {}
    for slot in BGSM_TEXTURE_SLOTS:
        val = getattr(mat, slot, None)
        if val and isinstance(val, str) and val.strip():
            textures[slot] = _normalize_path(val)

    return {"type": "bgsm", "textures": textures}


def _parse_bgem(filepath: str) -> dict | None:
    """Parse BGEM and extract texture paths."""
    from creation_lib.material_tools.bgem_bin import read_bgem
    try:
        with open(filepath, "rb") as f:
            mat = read_bgem(f)
    except Exception:
        return None

    textures = {}
    for slot in BGEM_TEXTURE_SLOTS:
        val = getattr(mat, slot, None)
        if val and isinstance(val, str) and val.strip():
            textures[slot] = _normalize_path(val)

    return {"type": "bgem", "textures": textures}
