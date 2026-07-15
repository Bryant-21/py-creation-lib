"""Persistent case-insensitive directory index — thin wrapper over Rust backend."""

from __future__ import annotations

from pathlib import Path
from typing import Optional

from .native_runtime import DirectoryIndex as _NativeDirectoryIndex


class DirectoryIndex:
    """Persistent case-insensitive directory index for fast file lookups."""

    def __init__(self, root: Path, *, cache_dir: Path | None = None):
        self._root = Path(root).resolve()
        cache = str(cache_dir) if cache_dir is not None else None
        self._impl = _NativeDirectoryIndex(str(self._root), cache_dir=cache)

    def resolve(self, rel_path: str) -> Optional[Path]:
        abs_str = self._impl.resolve(rel_path)
        return Path(abs_str) if abs_str is not None else None

    def contains(self, rel_path: str) -> bool:
        return self._impl.contains(rel_path)

    @property
    def file_count(self) -> int:
        return self._impl.file_count

    @property
    def _lookup(self) -> dict[str, str]:
        return self._impl._lookup

    def close(self) -> None:
        self._impl.close()
