"""Shared texture directory resolution for NIF rendering.

All UI apps that load NIFs need to resolve texture search paths from
ToolkitSettings.  This module provides a single implementation so that
editor, bone_editor, aligner (and future apps) stay consistent.
"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from creation_lib.ba2 import BA2Manager

_log = logging.getLogger(__name__)


def build_texture_dirs(
    toolkit_settings,
    game_id: str = "fo4",
    nif_path: str | None = None,
    mods_root: Path | str | None = None,
) -> tuple[list[Path], list[Path], list[Path]]:
    """Build ordered texture search directories from toolkit settings.

    Lookup priority (loose files first, then archives):
      1. additional_paths  — loose files (user mod dirs)
      2. NIF's own dir     — loose files (+ walk up to Data root)
      3. extracted_dir     — loose files (extracted game assets)
      4. managed mods      — loose files (mods/ matching game)
      5. additional_paths  — BA2s (user mod archives)
      6. root_dir/Data     — BA2s (base game — last resort)

    Returns:
        (texture_dirs, user_archive_dirs, base_archive_dirs)
    """
    texture_dirs: list[Path] = []
    user_archive_dirs: list[Path] = []
    base_archive_dirs: list[Path] = []

    def _append_unique(target: list[Path], p: Path):
        if p not in target:
            target.append(p)

    def _add_with_data(target: list, p: Path):
        _append_unique(target, p)
        data_sub = p / "Data"
        if data_sub.is_dir():
            _append_unique(target, data_sub)

    if toolkit_settings is not None:
        game_paths = toolkit_settings.get_game_paths(game_id)

        # 1. additional_paths — loose + user BA2s
        for p in game_paths.get("additional_paths", []):
            _add_with_data(texture_dirs, Path(p))
            _add_with_data(user_archive_dirs, Path(p))

        # 6. root_dir — base game BA2s (last resort)
        root = game_paths.get("root_dir", "")
        if root:
            _add_with_data(base_archive_dirs, Path(root))

    # 2. NIF's own dir + walk up to Data root. Keep mod-local assets ahead of
    # extracted/base trees so custom materials/textures resolve without
    # hydrating large unrelated directory indexes first.
    if nif_path:
        nif_dir = Path(nif_path).parent
        _append_unique(texture_dirs, nif_dir)
        p = nif_dir
        for _ in range(8):
            p = p.parent
            if p == p.parent:
                break
            if p.name.lower() == "data" and (p / "Meshes").is_dir():
                _append_unique(texture_dirs, p)
                break

    if toolkit_settings is not None:
        game_paths = toolkit_settings.get_game_paths(game_id)

        # 3. extracted_dir — loose only
        extracted = game_paths.get("extracted_dir", "")
        if extracted:
            _add_with_data(texture_dirs, Path(extracted))

        # 4. managed mods — loose only
        if mods_root is not None:
            _add_managed_mod_dirs(texture_dirs, game_id, Path(mods_root))

    return texture_dirs, user_archive_dirs, base_archive_dirs


def _add_managed_mod_dirs(texture_dirs: list[Path], game_id: str, mods_root: Path):
    """Auto-discover managed mods (mods/*) matching game_id."""
    if not mods_root.is_dir():
        return
    try:
        for mod_dir in sorted(mods_root.iterdir()):
            if not mod_dir.is_dir():
                continue
            game_file = mod_dir / ".game"
            if not game_file.is_file():
                continue
            try:
                mod_game = game_file.read_text().strip()
            except Exception:
                continue
            if mod_game != game_id:
                continue
            data_dir = mod_dir / "data"
            if data_dir.is_dir() and data_dir not in texture_dirs:
                texture_dirs.append(data_dir)
    except PermissionError:
        pass


def create_ba2_manager(
    user_archive_dirs: list[Path],
    base_archive_dirs: list[Path],
    existing: BA2Manager | None = None,
) -> BA2Manager | None:
    """Create a BA2Manager scanning the given directories.

    If *existing* is provided it is closed first.  Returns None if no
    directories contain archives.

    Args:
        user_archive_dirs: Directories with user mod BA2s (checked first).
        base_archive_dirs: Directories with base game BA2s (checked last).
        existing: Optional existing manager to close and replace.
    """
    if not user_archive_dirs and not base_archive_dirs:
        return existing

    from creation_lib.ba2 import BA2Manager

    if existing is not None:
        existing.close_all()

    mgr = BA2Manager()
    mgr.scan_directories(user_archive_dirs)
    mgr.scan_directories(base_archive_dirs)
    _log.info("BA2 manager: %d archives queued", mgr.archive_count)
    return mgr
