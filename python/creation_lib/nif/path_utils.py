from __future__ import annotations

_DATA_RELATIVE_ROOTS = (
    "materials",
    "textures",
    "meshes",
    "sound",
    "music",
    "interface",
)


def normalize_material_path(raw: str) -> str:
    if not raw or not isinstance(raw, str):
        return raw
    s = raw.rstrip("\x00").strip()
    if not s:
        return raw
    uses_backslash = "\\" in s and "/" not in s
    scan = s.replace("\\", "/")
    lower = scan.lower()
    best_idx = -1
    for root in _DATA_RELATIVE_ROOTS:
        idx = lower.rfind(f"/{root}/")
        if idx != -1:
            best_idx = max(best_idx, idx + 1)
    if best_idx <= 0:
        return s if s != raw else raw
    stripped = scan[best_idx:]
    return stripped.replace("/", "\\") if uses_backslash else stripped
