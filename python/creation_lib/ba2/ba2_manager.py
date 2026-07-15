"""Bethesda archive manager — unified BA2 + BSA file lookup.

Scans directories for BA2 (FO4/FO76/Starfield) and BSA (Skyrim LE/SE)
archives, provides a single `find()` method that searches all loaded
archives regardless of format.

Archives are opened lazily: scan_directories() collects archive paths but
does not open them.  The first call to find() that reaches the archive
layer triggers a one-time batch open of all collected archives.  This
means archives are never touched when loose-file lookup (done by callers
via texture_dirs) already satisfies every request.

Unified routing cache (archive_cache.py) persists the combined
``{file_key: archive_path}`` index to SQLite, keyed by a fingerprint of
all archive mtimes + sizes.  On warm starts the index is loaded in one
shot and archives are opened lazily only when extract() is actually
needed — skipping the ~700 ms per-archive open+parse loop.
"""
from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import TYPE_CHECKING

from .archive_cache import ArchiveCache
from . import native_runtime

if TYPE_CHECKING:
    pass

_log = logging.getLogger("ba2.manager")

# Extensions to scan — BA2 (FO4/FO76/Starfield) + BSA (Skyrim LE/SE)
_ARCHIVE_EXTENSIONS = (".ba2", ".bsa")


class _NativeArchive:
    """Thin native-backed archive handle used by BA2Manager.

    This avoids parsing BA2/BSA headers in Python when the Rust backend is
    available, but preserves the small reader API surface BA2Manager expects.
    """

    def __init__(self, path: str | Path, *, _cached: dict | None = None):
        self._path = Path(path)
        self._files: dict[str, None] = {}
        self._format = ""
        self._version = 0
        self._archive_type = ""
        if _cached is not None:
            self._restore_from_cache(_cached)
        else:
            self._load_from_native()

    @staticmethod
    def _normalize(path: str) -> str:
        return path.lower().replace("\\", "/")

    @property
    def path(self) -> Path:
        return self._path

    @property
    def file_count(self) -> int:
        return len(self._files)

    @property
    def archive_type(self) -> str:
        return self._archive_type

    def _apply_info(self, info: dict) -> None:
        self._format = str(info.get("format") or "")
        try:
            self._version = int(info.get("version") or 0)
        except (TypeError, ValueError):
            self._version = 0
        if self._format == "tes4":
            self._archive_type = f"BSA_v{self._version}" if self._version else "BSA"
        elif self._format.endswith("_dx10"):
            self._archive_type = "DX10"
        elif self._format.endswith("_gnrl"):
            self._archive_type = "GNRL"
        elif self._format.endswith("_gnmf"):
            self._archive_type = "GNMF"
        elif self._format:
            self._archive_type = self._format
        else:
            self._archive_type = "BSA" if self._path.suffix.lower() == ".bsa" else "BA2"

    def _load_from_native(self) -> None:
        info = native_runtime.archive_info(str(self._path))
        files = native_runtime.list_archive(str(self._path))
        self._apply_info(info)
        self._files = {
            self._normalize(file_key): None
            for file_key in files
        }

    def to_cache(self) -> dict:
        return {
            "backend": "native",
            "format": self._format,
            "version": self._version,
            "archive_type": self._archive_type,
            "files": list(self._files),
        }

    def _restore_from_cache(self, data: dict) -> None:
        if data.get("backend") != "native":
            raise ValueError("not a native archive cache entry")
        info = {
            "format": data.get("format"),
            "version": data.get("version"),
        }
        self._apply_info(info)
        raw_files = data.get("files")
        if not isinstance(raw_files, list):
            raise ValueError("native archive cache is missing file list")
        self._files = {
            self._normalize(file_key): None
            for file_key in raw_files
            if isinstance(file_key, str)
        }

    def extract(self, path: str) -> bytes | None:
        return native_runtime.extract_one(str(self._path), self._normalize(path))

    def list_files(self, prefix: str = "", suffix: str = "") -> list[str]:
        prefix_lower = self._normalize(prefix)
        suffix_lower = suffix.lower()
        result = []
        for key in self._files:
            if prefix_lower and not key.startswith(prefix_lower):
                continue
            if suffix_lower and not key.endswith(suffix_lower):
                continue
            result.append(key)
        return result

    def contains(self, path: str) -> bool:
        return self._normalize(path) in self._files

    def close(self) -> None:
        return None


class BA2Manager:
    """Manages multiple archive files (BA2 + BSA) for on-demand extraction.

    Two-phase startup:

    1. ``scan_directories()`` — collect archive *paths* only. Fast (directory
       listing). No archives opened.

    2. First ``find()`` call triggers ``_ensure_loaded()``:
       - **Warm start (unified routing cache hit):** load ``{file_key →
         archive_path}`` from a single SQLite blob (~few ms for fingerprint
         check + decompress). Archives are then opened lazily per-extraction.
       - **Cold start (cache miss):** open every archive, build unified
         routing, persist it for next time.

    Usage::

        mgr = BA2Manager()
        mgr.scan_directories([Path("C:/Games/Fallout4/Data")])
        data = mgr.find("textures/weapons/gun/diffuse.dds")
        mgr.close_all()
    """

    def __init__(self, *, cache_dir: Path | None = None, use_cache: bool = True):
        self._pending: list[Path] = []          # archive paths collected, not yet processed
        self._known_paths: set[str] = set()     # normalized paths of all known archives

        # Unified routing index: file_key → archive_path_str (populated by _ensure_loaded)
        self._file_index: dict[str, str] = {}

        # Lazily-opened archive handles: archive_path_str → archive object
        self._open_archives: dict[str, _NativeArchive] = {}

        # Priority order for progressive fallback and list_files
        self._archive_order: list[str] = []     # archive_path_str in scan order

        self._routing_loaded = False            # True once _file_index is populated
        self._cache: ArchiveCache | None = None
        self._use_cache = use_cache
        if use_cache:
            try:
                self._cache = ArchiveCache(cache_dir)
            except Exception as e:
                _log.warning("Failed to initialize archive cache: %s", e)
                self._cache = None

    @property
    def archive_count(self) -> int:
        """Total number of known archives (pending + loaded)."""
        return len(self._archive_order) + len(self._pending)

    @property
    def total_file_count(self) -> int:
        """Total number of uniquely-indexed files across all archives."""
        self._ensure_loaded()
        return len(self._file_index)

    @property
    def archives(self) -> list[dict]:
        """Return info about known archives for UI display.

        Does not force-open archives that haven't been needed for extraction.
        Uses the routing index to compute per-archive file counts.
        """
        self._ensure_loaded()
        # Count files per archive from the routing index
        counts: dict[str, int] = {}
        for archive_path_str in self._file_index.values():
            counts[archive_path_str] = counts.get(archive_path_str, 0) + 1

        result = []
        for path_str in self._archive_order:
            p = Path(path_str)
            archive = self._open_archives.get(path_str)
            arc_type = archive.archive_type if archive is not None else (
                "DX10" if "textures" in p.name.lower() else "GNRL"
            )
            result.append({
                "name": p.name,
                "path": path_str,
                "type": arc_type,
                "files": counts.get(path_str, 0),
            })
        return result

    # ------------------------------------------------------------------
    # Directory scanning
    # ------------------------------------------------------------------

    def scan_directories(self, dirs: list[Path]):
        """Collect archive paths from dirs without opening them.

        Archives are opened lazily the first time find() needs them.
        For each directory, also checks for a ``Data/`` subdirectory.
        Skips archives that are already known.
        """
        for d in dirs:
            self._collect_dir(d)
            data_sub = d / "Data"
            if data_sub.is_dir() and data_sub != d:
                self._collect_dir(data_sub)

        _log.info(
            "BA2 scan: %d archives queued (lazy — opened on first miss)",
            len(self._pending),
        )

    def _collect_dir(self, d: Path):
        """Collect archive file paths from a directory without opening them."""
        if not d.is_dir():
            return
        try:
            for archive_path in sorted(d.iterdir()):
                if not archive_path.is_file():
                    continue
                if archive_path.suffix.lower() not in _ARCHIVE_EXTENSIONS:
                    continue
                norm = str(archive_path.resolve()).lower()
                if norm in self._known_paths:
                    continue
                self._known_paths.add(norm)
                self._pending.append(archive_path)
        except PermissionError:
            _log.debug("Permission denied scanning %s", d)

    # ------------------------------------------------------------------
    # Unified routing — warm start
    # ------------------------------------------------------------------

    def _ensure_loaded(self):
        """Populate the unified file index (called lazily on first find() miss).

        Warm start: load routing from cache in one shot (~few ms).
        Cold start: open all archives, build index, persist cache (~700 ms).
        """
        if self._routing_loaded or not self._pending:
            self._routing_loaded = True
            return

        # Try unified routing cache first
        if self._cache is not None:
            fingerprint = self._cache.compute_fingerprint(self._pending)
            cached = self._cache.get_routing(fingerprint)
            if cached is not None:
                self._file_index = cached
                # Record archive order from what's referenced in the routing
                seen: set[str] = set()
                for p in self._pending:
                    ps = str(p.resolve()).lower()
                    if ps not in seen:
                        seen.add(ps)
                        self._archive_order.append(ps)
                self._pending.clear()
                self._routing_loaded = True
                _log.info(
                    "BA2 init: warm start — %d files indexed from routing cache "
                    "(fingerprint=%s…)",
                    len(self._file_index), fingerprint[:8],
                )
                return
        else:
            fingerprint = None

        # Cold start: open all archives and build routing from scratch
        self._cold_load(fingerprint)

    def _cold_load(self, fingerprint: str | None):
        """Open all pending archives, build routing index, persist cache.

        The native archive parse (``archive_info`` / ``list_archive``) releases
        the GIL (``py.detach``), so cache-miss archives are parsed across a
        thread pool — a serial open of 100+ base-game archives otherwise pins
        the GIL long enough to freeze an interactive UI. SQLite cache reads and
        writes stay on this thread (single-threaded connection).
        """
        t0 = time.perf_counter()

        pending = list(self._pending)

        # Phase 1 (serial): SQLite cache reads — restore hits, queue misses.
        restored: dict[Path, _NativeArchive] = {}
        to_parse: list[Path] = []
        for archive_path in pending:
            arc = None
            if self._cache is not None:
                cached = self._cache.get(archive_path)
                if cached is not None and cached.get("backend") == "native":
                    try:
                        arc = _NativeArchive(archive_path, _cached=cached)
                    except Exception:
                        arc = None
            if arc is not None:
                restored[archive_path] = arc
            else:
                to_parse.append(archive_path)

        # Phase 2 (parallel): native parse of cache-miss archives.
        parsed: dict[Path, _NativeArchive] = {}
        if to_parse:
            import os
            from concurrent.futures import ThreadPoolExecutor

            def _parse(p: Path):
                try:
                    return p, _NativeArchive(p)
                except Exception as e:
                    _log.warning("Failed to open archive %s: %s", p.name, e)
                    return p, None

            workers = min(len(to_parse), max(4, (os.cpu_count() or 8)), 16)
            with ThreadPoolExecutor(max_workers=workers) as ex:
                for p, arc in ex.map(_parse, to_parse):
                    if arc is not None:
                        parsed[p] = arc

        # Phase 3 (serial): persist newly-parsed archives to the SQLite cache.
        if self._cache is not None:
            for p, arc in parsed.items():
                try:
                    self._cache.put(p, arc.to_cache())
                except Exception as e:
                    _log.debug("Failed to cache %s: %s", p.name, e)

        cache_hits = len(restored)
        cache_misses = len(parsed)

        # Phase 4: assemble in scan order; collect (path_str, archive) pairs.
        ordered: list[tuple[str, _NativeArchive]] = []
        for archive_path in pending:
            archive = restored.get(archive_path) or parsed.get(archive_path)
            if archive is None:
                continue
            path_str = str(archive_path.resolve()).lower()
            ordered.append((path_str, archive))
            self._open_archives[path_str] = archive
            self._archive_order.append(path_str)

        self._pending.clear()
        self._routing_loaded = True

        # Build unified routing: first archive wins (reverse so earlier entries overwrite)
        self._file_index.clear()
        for path_str, archive in reversed(ordered):
            if hasattr(archive, '_files'):
                for file_key in archive._files:
                    self._file_index[file_key] = path_str

        # Prune stale per-archive cache entries
        loaded_norms = set(self._open_archives.keys())
        if self._cache is not None:
            self._cache.cleanup(loaded_norms)
            if fingerprint is not None and self._file_index:
                try:
                    self._cache.put_routing(fingerprint, self._file_index)
                except Exception as e:
                    _log.debug("Failed to persist unified routing: %s", e)

        elapsed = (time.perf_counter() - t0) * 1000
        _log.info(
            "BA2 init complete (cold): %d archives, %d files indexed "
            "(%.0fms, %d cached / %d parsed)",
            len(self._open_archives),
            len(self._file_index),
            elapsed, cache_hits, cache_misses,
        )

    # ------------------------------------------------------------------
    # Archive open helpers
    # ------------------------------------------------------------------

    def _open_archive(self, archive_path: Path) -> tuple[_NativeArchive, bool]:
        """Open an archive, using the per-archive cache if available.

        Returns (archive_instance, was_cache_hit).
        """
        if self._cache is not None:
            cached = self._cache.get(archive_path)
            if cached is not None and cached.get("backend") == "native":
                try:
                    return _NativeArchive(archive_path, _cached=cached), True
                except Exception:
                    _log.debug("Native cache restore failed for %s, re-parsing", archive_path.name)

        archive = _NativeArchive(archive_path)

        if self._cache is not None:
            try:
                self._cache.put(archive_path, archive.to_cache())
            except Exception as e:
                _log.debug("Failed to cache %s: %s", archive_path.name, e)

        return archive, False

    def _get_archive(self, path_str: str) -> _NativeArchive | None:
        """Return an open archive for the given path, opening it if needed."""
        archive = self._open_archives.get(path_str)
        if archive is not None:
            return archive
        try:
            archive, _ = self._open_archive(Path(path_str))
            self._open_archives[path_str] = archive
            return archive
        except Exception as e:
            _log.warning("Failed to lazy-open archive %s: %s", Path(path_str).name, e)
            return None

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def find(self, path: str) -> bytes | None:
        """Find and extract a file from any loaded archive.

        Callers are responsible for checking loose files (via texture_dirs)
        before calling this.  Uses the unified routing index for O(1) lookup,
        opening individual archives lazily only when needed for extraction.

        Normalizes the path (lowercase, forward slashes, strips leading
        'data/' prefix). Returns extracted bytes or None.
        """
        key = path.lower().replace("\\", "/").strip()
        for prefix in ("data/", "./"):
            if key.startswith(prefix):
                key = key[len(prefix):]

        # Fast path: routing loaded, O(1) lookup + lazy archive open
        if self._routing_loaded and not self._pending:
            archive_path_str = self._file_index.get(key)
            if archive_path_str is None:
                return None
            archive = self._get_archive(archive_path_str)
            if archive is None:
                return None
            data = archive.extract(key)
            if data is not None:
                _log.debug("Found %s in %s (indexed)", key, Path(archive_path_str).name)
            return data

        # Ensure routing is loaded, then retry fast path
        if not self._routing_loaded:
            self._ensure_loaded()
            return self.find(path)

        # Slow path: pending archives remain (shouldn't happen after _ensure_loaded)
        for archive in self._open_archives.values():
            data = archive.extract(key)
            if data is not None:
                return data

        while self._pending:
            archive_path = self._pending.pop(0)
            try:
                archive, _ = self._open_archive(archive_path)
                path_str = str(archive_path.resolve()).lower()
                self._open_archives[path_str] = archive
                data = archive.extract(key)
                if data is not None:
                    _log.debug("Found %s in %s (lazy-opened)", key, archive_path.name)
                    return data
            except Exception as e:
                _log.warning("Failed to open archive %s: %s", archive_path.name, e)

        return None

    def has_file(self, path: str) -> bool:
        """Check if a file exists in any archive without extracting it."""
        self._ensure_loaded()
        key = path.lower().replace("\\", "/").strip()
        for prefix in ("data/", "./"):
            if key.startswith(prefix):
                key = key[len(prefix):]
        return key in self._file_index

    def list_files(self, prefix: str = "", suffix: str = "") -> list[str]:
        """List all file paths across all loaded archives, optionally filtered.

        Args:
            prefix: Only include paths starting with this (case-insensitive).
            suffix: Only include paths ending with this (case-insensitive).

        Returns:
            Deduplicated list of normalized file paths.
        """
        self._ensure_loaded()
        prefix = prefix.lower().replace("\\", "/")
        suffix = suffix.lower()
        result = []
        for file_key in self._file_index:
            if prefix and not file_key.startswith(prefix):
                continue
            if suffix and not file_key.endswith(suffix):
                continue
            result.append(file_key)
        return result

    def close_all(self):
        """Close all open archive file handles."""
        for archive in self._open_archives.values():
            archive.close()
        self._open_archives.clear()
        self._pending.clear()
        self._known_paths.clear()
        self._archive_order.clear()
        self._file_index.clear()
        self._routing_loaded = False
        if self._cache is not None:
            self._cache.close()
            self._cache = None
