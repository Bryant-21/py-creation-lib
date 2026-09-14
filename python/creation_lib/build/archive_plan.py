"""Archive planning helpers for mod BA2/BSA outputs."""
from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

DEFAULT_ARCHIVE_MAX_BYTES = 16 * 1024**3
_ARCHIVE_HEADER_OVERHEAD = 4096
_ENTRY_OVERHEAD = 512
_MAIN_FAMILY_ORDER = (
    "LOD",
    "Terrain",
    "Meshes",
    "Sounds",
    "Animations",
    "Scripts",
    "Strings",
    "Materials",
    "Interface",
    "Main",
)
_FO4_MESH_LABELS = ("Meshes", "MeshesExtra")
_FO4_FAMILY_LABELS = {
    "Meshes": _FO4_MESH_LABELS,
    "Scripts": ("Misc",),
}
_GENERATED_LABEL_BASES = frozenset(
    (
        "Main",
        "Textures",
        "LODTextures",
        "TerrainTextures",
        "MeshesExtra",
        "Misc",
        *_MAIN_FAMILY_ORDER,
    )
)


@dataclass(frozen=True)
class ArchiveEntry:
    relative_path: str
    source_path: Path
    size: int


@dataclass(frozen=True)
class PlannedArchive:
    family: str
    label: str
    output_name: str
    entries: tuple[ArchiveEntry, ...]
    texture_archive: bool


def gib_to_bytes(value: float) -> int:
    if value <= 0:
        raise ValueError("archive max size must be greater than 0 GiB")
    return int(value * 1024**3)


def _normalize_relative_path(relative_path: str) -> str:
    return relative_path.replace("\\", "/").lstrip("/")


# Matches lodgen's terrain-LOD tile naming: `<world>.<level>.<x>.<y>` (signed
# ints), then anything (an "_msn" normal-map suffix, a ".season" segment,
# etc.), then ".dds". Mirror of mod_pack.rs::is_lodgen_quad_tile. It can't
# match convert_terrain output: convert_terrain runs its texture-set name
# through safe_name, which replaces "." and "-" with "_", so its filenames
# never contain this dot-separated signed-integer triple.
_LODGEN_QUAD_TILE_RE = re.compile(r"^[^.]+\.-?\d+\.-?\d+\.-?\d+.*\.dds$")


def _is_lodgen_quad_tile(basename: str) -> bool:
    return bool(_LODGEN_QUAD_TILE_RE.match(basename.lower()))


def classify_archive_family(relative_path: str) -> str:
    path = _normalize_relative_path(relative_path)
    lower = path.lower()
    parts = lower.split("/")
    if parts and parts[0] == "data":
        lower = "/".join(parts[1:])
        parts = lower.split("/") if lower else []
    suffix = Path(lower).suffix

    # LOD shares Textures/Terrain with full-resolution land textures. Keep only
    # lodgen products in the LOD family; the remaining textures and materials
    # belong in the ordinary Textures/Materials archives.
    if len(parts) > 1 and parts[0] == "textures" and parts[1] == "terrain":
        basename = parts[-1]
        if "objects" in parts:
            return "LOD"
        if "lodgen" in parts:
            return "LOD"
        if _is_lodgen_quad_tile(basename):
            return "LOD"
        return "Textures"
    if len(parts) > 1 and parts[0] == "materials" and parts[1] == "terrain":
        return "Materials"

    if parts and parts[0] == "textures":
        return "Textures"
    if parts and parts[0] == "interface":
        return "Interface"
    if (parts and parts[0] == "materials") or suffix in {".bgsm", ".bgem"}:
        return "Materials"
    if (parts and parts[0] == "strings") or suffix in {".strings", ".dlstrings", ".ilstrings"}:
        return "Strings"
    if (parts and parts[0] in {"sound", "music"}) or suffix in {".xwm", ".wav"}:
        return "Sounds"
    if suffix == ".hkx" or "animations" in parts:
        return "Animations"
    if (parts and parts[0] == "scripts") or suffix in {".pex", ".psc"}:
        return "Scripts"
    if suffix in {".bto", ".btr"}:
        return "LOD"
    if (parts and parts[0] == "meshes") or suffix == ".nif":
        return "Meshes"
    return "Main"


def _estimated_entry_size(entry: ArchiveEntry) -> int:
    return max(0, int(entry.size)) + _ENTRY_OVERHEAD + len(
        _normalize_relative_path(entry.relative_path).encode("utf-8")
    )


def estimate_archive_size(entries: Iterable[ArchiveEntry]) -> int:
    total = _ARCHIVE_HEADER_OVERHEAD
    count = 0
    for entry in entries:
        count += 1
        total += _estimated_entry_size(entry)
    return total if count else 0


def _output_name(mod_name: str, label: str, archive_ext: str, platform_suffix: str) -> str:
    ext = archive_ext[1:] if archive_ext.startswith(".") else archive_ext
    return f"{mod_name} - {label}{platform_suffix}.{ext}"


def plan_archive_outputs(
    mod_name: str,
    entries: Iterable[ArchiveEntry],
    archive_ext: str,
    platform_suffix: str = "",
    max_bytes: int | None = None,
    *,
    game: str | None = None,
    expanded_archives: bool = False,
) -> list[PlannedArchive]:
    from creation_lib.ba2 import native_runtime

    if expanded_archives:
        cap = DEFAULT_ARCHIVE_MAX_BYTES if max_bytes is None else max_bytes
        if cap <= 0:
            raise ValueError("archive max size must be greater than 0 bytes")
    else:
        cap = DEFAULT_ARCHIVE_MAX_BYTES

    native_plans = native_runtime.plan_archives(
        mod_name,
        [(e.relative_path, str(e.source_path), int(e.size)) for e in entries],
        archive_ext,
        platform_suffix,
        cap,
        game,
        expanded_archives,
    )
    return [
        PlannedArchive(
            family=plan["family"],
            label=plan["label"],
            output_name=plan["output_name"],
            entries=tuple(
                ArchiveEntry(rel, Path(src), int(size))
                for (rel, src, size) in plan["entries"]
            ),
            texture_archive=plan["texture_archive"],
        )
        for plan in native_plans
    ]


def discover_mod_archives(
    directory: Path | str,
    mod_name: str,
    extensions: tuple[str, ...] = (".ba2", ".bsa"),
) -> list[Path]:
    root = Path(directory)
    if not root.is_dir():
        return []
    normalized_extensions = tuple(ext.lower() if ext.startswith(".") else f".{ext.lower()}" for ext in extensions)
    prefix = f"{mod_name} - "
    return sorted(
        (
            path
            for path in root.iterdir()
            if path.is_file()
            and path.name.startswith(prefix)
            and path.suffix.lower() in normalized_extensions
            and _is_generated_archive_name(path, prefix)
        ),
        key=lambda path: path.name.lower(),
    )


def _is_generated_archive_name(path: Path, prefix: str) -> bool:
    label = path.stem[len(prefix):]
    for suffix in ("_xbox", "_ps"):
        if label.endswith(suffix):
            label = label[: -len(suffix)]
            break
    label_base = label.rstrip("0123456789")
    return bool(label_base) and label_base in _GENERATED_LABEL_BASES
