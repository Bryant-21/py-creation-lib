"""Host-provider seam for creation_lib.ui.

The shared ImGui shell needs a few paths/services only the host application
(modkit toolkit, BACUP) can supply. Hosts call set_host() once at startup;
without one, defaults keep the shell functional with index building disabled
and user-local data dirs.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

GAME_ESM_YAML_DIR = {
    "fo4": "fo4_esm_yaml",
    "skyrimse": "skyrimse_esm_yaml",
    "starfield": "starfield_esm_yaml",
    "fo76": "fo76_esm_yaml",
    "fo3": "fo3_esm_yaml",
    "fnv": "fnv_esm_yaml",
}


@dataclass
class UiHost:
    get_app_root: Callable[[], Path]
    get_ini_dir: Callable[[], Path]
    get_db_dir: Callable[[], Path]
    resolve_extracted_output_dir: Callable[[str], Path]
    # Returns a DbBuilder-compatible object (kwargs match
    # ui.toolkit.db_builder.DbBuilder, must expose .start()).
    # None -> host has no index-build service; UI disables it.
    db_builder_factory: Callable[..., Any] | None = None


def _default_data_root() -> Path:
    base = os.environ.get("LOCALAPPDATA")
    root = Path(base) if base else Path.home() / ".local" / "share"
    return root / "creation_lib"


def _default_host() -> UiHost:
    def app_root() -> Path:
        root = _default_data_root()
        root.mkdir(parents=True, exist_ok=True)
        return root

    def ini_dir() -> Path:
        path = app_root() / "settings"
        path.mkdir(parents=True, exist_ok=True)
        return path

    return UiHost(
        get_app_root=app_root,
        get_ini_dir=ini_dir,
        get_db_dir=lambda: app_root() / "data",
        resolve_extracted_output_dir=lambda game: app_root() / "extracted" / game,
        db_builder_factory=None,
    )


_host: UiHost | None = None


def set_host(host: UiHost) -> None:
    global _host
    _host = host


def get_host() -> UiHost:
    global _host
    if _host is None:
        _host = _default_host()
    return _host
