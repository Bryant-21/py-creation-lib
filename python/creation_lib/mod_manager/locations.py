"""Manager data locations follow the toolkit's writable app-root convention."""
from pathlib import Path

from app import paths as app_paths


def default_library_path() -> Path:
    # Resolution only: preview/startup must never create a library.
    return app_paths.get_db_dir() / "mod_manager" / "library"


def library_path(destination="", current=None) -> Path:
    value = str(destination or "").strip()
    return Path(value or current or default_library_path()).absolute()
