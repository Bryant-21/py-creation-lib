"""Thin Python boundary for the native lodgen runtime."""
from __future__ import annotations

import json
from dataclasses import dataclass
from importlib import import_module
from pathlib import Path
from typing import Any, Callable, Sequence

_NATIVE_MODULE: Any | None = None
_IMPORT_ATTEMPTED = False


def _reset_for_tests() -> None:
    global _NATIVE_MODULE, _IMPORT_ATTEMPTED
    _NATIVE_MODULE = None
    _IMPORT_ATTEMPTED = False


def _load_umbrella_submodule() -> Any:
    umbrella = import_module("creation_lib._native")
    mod = getattr(umbrella, "lodgen_native", None)
    if mod is not None:
        return mod
    try:
        return import_module("creation_lib._native.lodgen_native")
    except ImportError as exc:
        raise ImportError("lodgen_native not found in creation_lib._native") from exc


def load_native_module() -> Any:
    global _NATIVE_MODULE, _IMPORT_ATTEMPTED
    if _IMPORT_ATTEMPTED:
        if _NATIVE_MODULE is None:
            raise RuntimeError("lodgen_native is required for creation_lib.lod")
        return _NATIVE_MODULE
    _IMPORT_ATTEMPTED = True
    try:
        _NATIVE_MODULE = _load_umbrella_submodule()
    except ImportError as exc:
        raise RuntimeError("lodgen_native is required for creation_lib.lod") from exc
    return _NATIVE_MODULE


def is_available() -> bool:
    try:
        load_native_module()
        return True
    except RuntimeError:
        return False


@dataclass(frozen=True, slots=True)
class LodGenResult:
    btr: int
    bto: int
    btt: int
    dds: int
    lod_written: bool
    warnings: tuple[str, ...]


def generate_lod(
    world_editor_id: str,
    settings: dict | str,
    *,
    data_dirs: Sequence[str | Path],
    output_dir: str | Path,
    plugin_path: str | Path | None = None,
    source_data_dir: str | Path | None = None,
    object_lod_overlay: str | Path | None = None,
    progress: Callable[[str, float], None] | None = None,
) -> LodGenResult:
    """Generate LOD for one worldspace.

    `data_dirs` are ASSET-ONLY search sources: loose Data roots or BA2 files.
    Loose files win; BA2 members are decompressed in memory on demand. `plugin_path`,
    when given, is the SOLE plugin the worldspace + records are read from (no
    `data_dirs` plugin discovery). When omitted, the native side falls back to
    legacy `data_dirs` plugin discovery (UI / standalone use).
    """
    native = load_native_module()
    settings_json = settings if isinstance(settings, str) else json.dumps(settings)
    paths = native.PyLodPaths(
        [str(p) for p in data_dirs],
        str(output_dir),
        str(plugin_path) if plugin_path is not None else None,
        str(source_data_dir) if source_data_dir is not None else None,
        str(object_lod_overlay) if object_lod_overlay is not None else None,
    )
    stats = native.generate_lod(world_editor_id, settings_json, paths, progress)
    return LodGenResult(
        btr=int(stats.btr),
        bto=int(stats.bto),
        btt=int(stats.btt),
        dds=int(stats.dds),
        lod_written=bool(stats.lod_written),
        warnings=tuple(stats.warnings or ()),
    )


def discover_worldspaces(
    plugin_path: str | Path,
    *,
    game: str = "fo4",
) -> tuple[str, ...]:
    """Return LOD-eligible worldspace EditorIDs from one built plugin."""
    native = load_native_module()
    discover = getattr(native, "discover_worldspaces", None)
    if not callable(discover):
        raise RuntimeError(
            "lodgen_native does not expose discover_worldspaces; "
            "run scripts/ensure_native.py after updating native code"
        )
    return tuple(str(worldspace) for worldspace in discover(str(plugin_path), game))


def count_fo76_bto_tiles(
    source_data_dir: str | Path,
    world_editor_id: str,
) -> int:
    """Count source BTO tiles the native FO76 object-LOD path can consume."""
    native = load_native_module()
    count = getattr(native, "count_fo76_bto_tiles", None)
    if not callable(count):
        raise RuntimeError(
            "lodgen_native does not expose count_fo76_bto_tiles; "
            "run scripts/ensure_native.py after updating native code"
        )
    return int(count(str(source_data_dir), world_editor_id))


def collect_fo76_bto_tree_billboard_species(
    world_editor_id: str,
    settings: dict | str,
    *,
    source_data_dir: str | Path,
) -> tuple[dict[str, Any], ...]:
    native = load_native_module()
    settings_json = settings if isinstance(settings, str) else json.dumps(settings)
    collect = getattr(native, "collect_fo76_bto_tree_billboard_species", None)
    if not callable(collect):
        raise RuntimeError(
            "lodgen_native does not expose collect_fo76_bto_tree_billboard_species; "
            "run scripts/ensure_native.py after updating native code"
        )
    rows = collect(
        world_editor_id,
        settings_json,
        str(source_data_dir),
    )
    return tuple(
        {
            "model": str(model),
            "render_model": str(resolved_path),
            "instance_count": int(instance_count),
        }
        for model, resolved_path, instance_count in rows
    )
