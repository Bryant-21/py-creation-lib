"""Persistent SQLite cache for BA2/BSA archive file tables.

Keyed by (path, mtime_ns, size); a changed mtime or size re-parses the archive.
Also caches the unified routing table (file_key → archive_path) as one
gzip-compressed pickle blob, so BA2Manager can skip opening archives on warm
starts and open them lazily for extraction. Directories are still scanned
every time to pick up new archives.
"""
from __future__ import annotations

import gzip
import hashlib
import json
import logging
import os
import pickle
from pathlib import Path

from creation_lib.db.native_runtime import Database

_log = logging.getLogger("ba2.cache")

_SCHEMA_VERSION = 3


def _default_cache_dir() -> Path:
    """Platform-appropriate cache directory."""
    local = os.environ.get("LOCALAPPDATA")
    if local:
        return Path(local) / "modkit21"
    return Path.home() / ".cache" / "modkit21"


class ArchiveCache:
    """SQLite-backed persistent cache for archive file tables and unified routing.

    Usage::

        cache = ArchiveCache()
        hit = cache.get(archive_path)   # None on miss
        cache.put(archive_path, data)   # store after parsing

        fp = cache.compute_fingerprint(pending_paths)
        routing = cache.get_routing(fp)  # None on miss / fingerprint mismatch
        cache.put_routing(fp, routing)   # store unified {file_key: archive_path}

        cache.cleanup(valid_paths)       # prune stale per-archive entries
        cache.close()
    """

    def __init__(self, cache_dir: Path | None = None):
        if cache_dir is None:
            cache_dir = _default_cache_dir()
        self._db_path = cache_dir / "archive_cache.sqlite"
        self._conn: Database | None = None

    def _connect(self) -> Database:
        if self._conn is not None:
            return self._conn
        self._db_path.parent.mkdir(parents=True, exist_ok=True)
        conn = Database.open(str(self._db_path), "rw")
        conn.execute(
            """CREATE TABLE IF NOT EXISTS archives (
                   path     TEXT PRIMARY KEY,
                   mtime_ns INTEGER NOT NULL,
                   size     INTEGER NOT NULL,
                   data     TEXT NOT NULL
               );
               CREATE TABLE IF NOT EXISTS unified_routing (
                   id          INTEGER PRIMARY KEY CHECK (id = 1),
                   fingerprint TEXT NOT NULL,
                   data        BLOB NOT NULL
               );
               CREATE TABLE IF NOT EXISTS meta (
                   key   TEXT PRIMARY KEY,
                   value TEXT NOT NULL
               );"""
        )
        # Schema migration — wipe on version bump
        row = conn.query_one("SELECT value FROM meta WHERE key = 'schema_version'")
        if row is None or int(row["value"]) != _SCHEMA_VERSION:
            conn.execute("DELETE FROM archives; DELETE FROM unified_routing;")
            conn.execute_one(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?)",
                [str(_SCHEMA_VERSION)],
            )
            _log.debug("Cache schema v%d — cleared stale entries", _SCHEMA_VERSION)
        self._conn = conn
        return conn

    # ------------------------------------------------------------------
    # Per-archive cache (file table keyed by path + mtime + size)
    # ------------------------------------------------------------------

    def get(self, archive_path: Path) -> dict | None:
        """Return cached file-table data if the archive hasn't changed.

        Returns ``None`` on cache miss (unknown path, changed mtime/size,
        or missing file).
        """
        try:
            stat = archive_path.stat()
        except OSError:
            return None
        row = self._connect().query_one(
            "SELECT mtime_ns, size, data FROM archives WHERE path = ?",
            [str(archive_path.resolve()).lower()],
        )
        if row is None:
            return None
        if row["mtime_ns"] != stat.st_mtime_ns or row["size"] != stat.st_size:
            _log.debug("Cache stale for %s (mtime/size changed)", archive_path.name)
            return None
        try:
            return json.loads(row["data"])
        except (json.JSONDecodeError, KeyError):
            _log.debug("Cache corrupt for %s — will re-parse", archive_path.name)
            return None

    def put(self, archive_path: Path, data: dict) -> None:
        """Store parsed file-table data for an archive."""
        try:
            stat = archive_path.stat()
        except OSError:
            return
        self._connect().execute_one(
            "INSERT OR REPLACE INTO archives (path, mtime_ns, size, data) VALUES (?, ?, ?, ?)",
            [
                str(archive_path.resolve()).lower(),
                stat.st_mtime_ns,
                stat.st_size,
                json.dumps(data),
            ],
        )

    def cleanup(self, valid_paths: set[str]) -> None:
        """Remove cache entries whose archive files no longer exist."""
        conn = self._connect()
        rows = conn.query_all("SELECT path FROM archives")
        all_paths = {r["path"] for r in rows if r.get("path")}
        stale = all_paths - valid_paths
        if stale:
            for p in stale:
                conn.execute_one("DELETE FROM archives WHERE path = ?", [p])
            _log.debug("Cleaned %d stale cache entries", len(stale))

    # ------------------------------------------------------------------
    # Unified routing cache (file_key → archive_path, all archives combined)
    # ------------------------------------------------------------------

    def compute_fingerprint(self, archive_paths: list[Path]) -> str:
        """Compute a fingerprint from the full set of archives and their mtime/size.

        Any change in the archive set (added, removed, or modified) produces
        a different fingerprint, invalidating the unified routing cache.
        """
        parts: list[str] = []
        for p in sorted(archive_paths, key=lambda x: str(x).lower()):
            try:
                st = p.stat()
                parts.append(f"{p.resolve()}:{st.st_mtime_ns}:{st.st_size}")
            except OSError:
                parts.append(f"{p.resolve()}:missing")
        digest = hashlib.md5("\n".join(parts).encode()).hexdigest()
        return digest

    def get_routing(self, fingerprint: str) -> dict[str, str] | None:
        """Load the unified routing table if the fingerprint matches.

        Returns ``{normalized_file_key: archive_path_str}`` or ``None`` on
        miss or fingerprint mismatch.
        """
        row = self._connect().query_one(
            "SELECT fingerprint, data FROM unified_routing WHERE id = 1"
        )
        if row is None or row["fingerprint"] != fingerprint:
            return None
        try:
            routing: dict[str, str] = pickle.loads(gzip.decompress(row["data"]))
            _log.debug(
                "Unified routing cache hit: %d files (fingerprint=%s…)",
                len(routing), fingerprint[:8],
            )
            return routing
        except Exception as e:
            _log.debug("Unified routing cache corrupt — will rebuild: %s", e)
            return None

    def put_routing(self, fingerprint: str, routing: dict[str, str]) -> None:
        """Persist the unified routing table for future warm starts.

        ``routing`` is ``{normalized_file_key: archive_path_str}``.
        Replaces any existing entry (only one row is ever kept).
        """
        try:
            blob = gzip.compress(pickle.dumps(routing, protocol=5), compresslevel=1)
        except Exception as e:
            _log.debug("Failed to serialize unified routing: %s", e)
            return
        self._connect().execute_one(
            "INSERT OR REPLACE INTO unified_routing (id, fingerprint, data) VALUES (1, ?, ?)",
            [fingerprint, blob],
        )
        _log.debug(
            "Unified routing cache saved: %d files, %.1f KB (fingerprint=%s…)",
            len(routing), len(blob) / 1024, fingerprint[:8],
        )

    def close(self) -> None:
        if self._conn is not None:
            self._conn.close()
            self._conn = None
