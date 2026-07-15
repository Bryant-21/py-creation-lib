"""Path helpers for packaged creation_lib resources.

Workspace and game-path configuration belongs at the app/CLI boundary.  Runtime
library code should receive explicit paths or a GameContext instead of reading
environment variables or project ``.env`` files.
"""

from __future__ import annotations

import sys
from importlib import resources
from pathlib import Path


def is_frozen() -> bool:
    return getattr(sys, "frozen", False)


def package_root() -> Path:
    return Path(__file__).resolve().parent


def find_project_root(start: Path | None = None) -> Path | None:
    return None


def load_dotenv_into_environ(env_path: Path | None = None, *, override: bool = False) -> Path | None:
    return None


def get_app_root() -> Path:
    return package_root()


def get_code_root() -> Path:
    return package_root()


def get_resource_dir() -> Path:
    resource = resources.files("creation_lib").joinpath("resources")
    return Path(str(resource))


def get_db_dir() -> Path:
    return get_app_root() / "data"


def get_settings_config_dir() -> Path:
    if is_frozen():
        return get_app_root() / "settings"
    return get_app_root() / "ui" / "toolkit" / "settings_data"


def get_shared_settings_path() -> Path:
    return get_settings_config_dir() / "shared_settings.json"


def get_variant_settings_path(variant_id: str = "full") -> Path:
    return get_settings_config_dir() / "variants" / f"{variant_id}.json"


def get_settings_path() -> Path:
    return get_variant_settings_path("full")


def get_logs_dir() -> Path:
    return get_app_root() / "logs"


def get_ini_dir() -> Path:
    path = get_app_root() / "settings"
    path.mkdir(exist_ok=True)
    return path
