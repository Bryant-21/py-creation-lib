"""Merge CK-generated previs/precombine data into a built plugin.

Creation Kit still generates ``CombinedObjects.esp``, ``PreVis.esp``, precombined
meshes, and ``.uvd`` files. This module delegates the plugin merge to
``esp_authoring_core`` so the merge preserves binary plugin topology instead of
round-tripping through authoring YAML.
"""
from __future__ import annotations

import logging
import sys
from pathlib import Path

_log = logging.getLogger("previs_merge")


def merge_previs(
    mod_name: str,
    game: str = "fo4",
    *,
    game_dir: str = "",
    mods_dir: str | Path,
    include_combined: bool = True,
    include_previs: bool = True,
) -> None:
    """Merge CK-generated CombinedObjects/PreVis ESP output into the mod plugin."""
    _merge_native(
        mod_name,
        game,
        game_dir=game_dir,
        mods_dir=mods_dir,
        include_combined=include_combined,
        include_previs=include_previs,
    )


def merge_precombined(
    mod_name: str,
    game: str = "fo4",
    *,
    game_dir: str = "",
    mods_dir: str | Path,
) -> None:
    """Merge CK-generated CombinedObjects ESP output into the mod plugin."""
    _merge_native(
        mod_name,
        game,
        game_dir=game_dir,
        mods_dir=mods_dir,
        include_combined=True,
        include_previs=False,
    )


def _merge_native(
    mod_name: str,
    game: str,
    *,
    game_dir: str,
    mods_dir: str | Path,
    include_combined: bool,
    include_previs: bool,
) -> None:
    if not include_combined and not include_previs:
        raise ValueError("at least one merge phase must be enabled")

    mod_dir = Path(mods_dir) / mod_name
    previs_tmp = mod_dir / "previs_tmp"
    combined_esp = previs_tmp / "CombinedObjects.esp"
    previs_esp = previs_tmp / "PreVis.esp"

    if include_combined and not combined_esp.is_file():
        raise FileNotFoundError(f"CombinedObjects.esp not found in {previs_tmp}")
    if include_previs and not previs_esp.is_file():
        raise FileNotFoundError(f"PreVis.esp not found in {previs_tmp}")

    from creation_lib.esp.authoring import get_plugin_ext
    from creation_lib.esp.native_runtime import _require_native_function

    plugin_ext = get_plugin_ext(mod_dir)
    plugin_path = mod_dir / f"{mod_name}.{plugin_ext}"
    if not plugin_path.is_file():
        raise FileNotFoundError(f"{plugin_path} not found. Build the mod first.")

    data_dir = mod_dir / "data"
    native_merge = _require_native_function("merge_previs_native")
    combined_count, previs_count, missing_uvds, removed_refrs, warnings = native_merge(
        str(plugin_path),
        str(combined_esp) if include_combined else None,
        str(previs_esp) if include_previs else None,
        game,
        str(data_dir) if data_dir.is_dir() else None,
    )

    _log.info(
        "Native previs merge complete for %s: %d precombine cell(s), %d previs cell(s), "
        "%d helper ref(s) removed",
        plugin_path.name,
        combined_count,
        previs_count,
        removed_refrs,
    )
    if missing_uvds:
        _log.warning("Previs merge skipped VISI for %d cell(s) with missing .uvd files", missing_uvds)
    for warning in warnings:
        _log.warning(warning)


if __name__ == "__main__":
    import argparse

    logging.basicConfig(level=logging.INFO, format="%(message)s")

    parser = argparse.ArgumentParser(description="Merge CK-generated previs data into a plugin")
    parser.add_argument("mod_name", help="Mod name, e.g. B21_MyMod")
    parser.add_argument("--game", default="fo4", help="Game ID, default: fo4")
    parser.add_argument("--game-dir", default="", help="Game install root, kept for API compatibility")
    parser.add_argument("--mods-dir", required=True, help="Directory containing managed mods")
    args = parser.parse_args()

    try:
        merge_previs(args.mod_name, args.game, game_dir=args.game_dir, mods_dir=args.mods_dir)
    except Exception as e:
        _log.error("Merge failed: %s", e)
        sys.exit(1)
