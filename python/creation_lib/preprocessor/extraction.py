"""Unified native BSA/BA2 extraction for multi-game support.

Public API:
    resolve_install_dir(game_id, explicit_dir) -> Path | None
    find_archives(data_dir, archive_format) -> list[Path]
    group_archives_by_update_phase(archives) -> list[list[Path]]
    plan_archive_extraction_batches(archives, total_workers) -> list[list[ArchiveExtractionTask]]
    load_manifest(output_dir) -> dict | None
    build_manifest(game, source_dir, archives) -> dict
    manifest_matches(manifest, source_dir, archives) -> bool
    save_manifest(output_dir, manifest) -> None
    extract_one(archive, output_dir, archive_format, file_workers) -> tuple
    extract_with_native_archive(archive_path, output_dir, archive_format, file_workers) -> int
"""
from __future__ import annotations

import datetime
import json
import logging
import math
import os
import re
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from creation_lib.ba2 import native_runtime
from creation_lib.core.game_profiles import get_profile

_log = logging.getLogger("modkit.extraction")
_UPDATE_ARCHIVE_RE = re.compile(r"(?:^|[\s_-])(\d{2,})update", re.IGNORECASE)


@dataclass(frozen=True)
class ArchiveExtractionTask:
    archive: Path
    file_workers: int
    file_count: int


def resolve_install_dir(
    game_id: str,
    explicit_dir: str | None,
    game_dir: str = "",
    steam_dir: str = "",
) -> Path | None:
    """Resolve game install directory from flag, caller-supplied env value, or Steam auto-detect.

    Args:
        game_id: Game profile ID (e.g. 'fo4', 'skyrimse').
        explicit_dir: Explicit path from --install-dir flag (highest priority).
        game_dir: Value of {GAME}_DIR from the caller's environment boundary.
        steam_dir: Value of STEAM_DIR from the caller's environment boundary.
    """
    # 1. Explicit --install-dir flag
    if explicit_dir:
        p = Path(explicit_dir)
        if p.is_dir():
            return p
        _log.error("--install-dir does not exist: %s", explicit_dir)
        return None

    # 2. {GAME}_DIR passed by caller (read from .env at CLI boundary)
    if game_dir:
        p = Path(game_dir)
        if p.is_dir():
            return p
        env_var = f"{game_id.upper()}_DIR"
        _log.warning("%s=%s does not exist, trying auto-detect...", env_var, game_dir)

    # 3. Auto-detect via Steam common paths
    profile = get_profile(game_id)
    if profile.steam_app_id:
        steam_paths = _find_steam_library_paths(steam_dir=steam_dir)
        for lib_path in steam_paths:
            app_dir = lib_path / "steamapps" / "common"
            # Try matching by executable name
            if profile.executable_name:
                game_name = profile.executable_name.replace(".exe", "")
                candidate = app_dir / game_name
                if candidate.is_dir():
                    return candidate
            # Also try display name
            candidate = app_dir / profile.display_name
            if candidate.is_dir():
                return candidate

    env_var = f"{game_id.upper()}_DIR"
    _log.error(
        "Could not find install directory for %s. Set %s in .env, or pass --install-dir PATH",
        profile.display_name, env_var,
    )
    return None


def _find_steam_library_paths(steam_dir: str = "") -> list[Path]:
    """Find Steam library folders on this system."""
    candidates = []

    # Common Steam install locations on Windows
    for drive in "CDEFGHIJKLMNOPQRSTUVWXYZ":
        candidates.append(Path(f"{drive}:/Program Files (x86)/Steam"))
        candidates.append(Path(f"{drive}:/Program Files/Steam"))
        candidates.append(Path(f"{drive}:/Steam"))
        candidates.append(Path(f"{drive}:/Steam Games"))

    # Caller-supplied STEAM_DIR takes priority
    if steam_dir:
        candidates.insert(0, Path(steam_dir))

    # Filter to existing directories
    found = []
    for c in candidates:
        if c.is_dir():
            found.append(c)
            # Also check libraryfolders.vdf for additional library paths
            vdf = c / "steamapps" / "libraryfolders.vdf"
            if vdf.is_file():
                found.extend(_parse_library_folders(vdf))

    return found


def _parse_library_folders(vdf_path: Path) -> list[Path]:
    """Parse Steam libraryfolders.vdf for additional library paths."""
    extra = []
    try:
        text = vdf_path.read_text(encoding="utf-8", errors="replace")
        # Simple parse: look for "path" keys with directory values
        import re
        for m in re.finditer(r'"path"\s+"([^"]+)"', text):
            p = Path(m.group(1).replace("\\\\", "/"))
            if p.is_dir():
                extra.append(p)
    except Exception:
        pass
    return extra


def _natural_sort_key(path: Path) -> tuple:
    parts = re.split(r"(\d+)", path.name.casefold())
    return tuple(int(part) if part.isdigit() else part for part in parts)


def _update_archive_phase(path: Path) -> int:
    match = _UPDATE_ARCHIVE_RE.search(path.stem)
    if match is None:
        return 0
    return int(match.group(1)) + 1


def group_archives_by_update_phase(archives: list[Path]) -> list[list[Path]]:
    """Group archives so numbered update BA2s extract after base archives."""
    groups: dict[int, list[Path]] = {}
    for archive in archives:
        groups.setdefault(_update_archive_phase(archive), []).append(archive)
    return [groups[phase] for phase in sorted(groups)]


def archive_entry_count(archive: Path) -> int:
    try:
        info = native_runtime.archive_info(str(archive))
        if not isinstance(info, dict):
            return 0
        return max(0, int(info.get("file_count", 0) or 0))
    except Exception:
        return 0


def _suggest_file_workers(file_count: int, total_workers: int) -> int:
    if file_count <= 0:
        return 1
    return max(1, min(total_workers, math.ceil(file_count / 10_000)))


def plan_archive_extraction_batches(
    archives: list[Path],
    total_workers: int,
) -> list[list[ArchiveExtractionTask]]:
    budget = max(1, int(total_workers or 1))
    batches: list[list[ArchiveExtractionTask]] = []
    batch: list[ArchiveExtractionTask] = []
    used_workers = 0
    for archive in archives:
        file_count = archive_entry_count(archive)
        file_workers = _suggest_file_workers(file_count, budget)
        task = ArchiveExtractionTask(
            archive=archive,
            file_workers=file_workers,
            file_count=file_count,
        )
        if batch and used_workers + file_workers > budget:
            batches.append(batch)
            batch = []
            used_workers = 0
        batch.append(task)
        used_workers += file_workers
    if batch:
        batches.append(batch)
    return batches


def find_archives(data_dir: Path, archive_format: str) -> list[Path]:
    """Find all BSA/BA2 archives in a Data/ directory."""
    if archive_format == "ba2":
        extensions = [".ba2"]
    elif archive_format == "bsa":
        extensions = [".bsa"]
    else:
        _log.error("Unknown archive format: %s", archive_format)
        return []

    archives = []
    for f in sorted(data_dir.iterdir(), key=_natural_sort_key):
        if f.is_file() and f.suffix.lower() in extensions:
            archives.append(f)
    return [archive for group in group_archives_by_update_phase(archives) for archive in group]


def resolve_papyrus_source_dir(install_dir: Path, game_id: str) -> Path | None:
    """Return the game's loose Papyrus source directory, if the profile defines one."""
    profile = get_profile(game_id)
    if not profile.papyrus_source_subpath:
        return None
    source_dir = install_dir / Path(profile.papyrus_source_subpath)
    if source_dir.is_dir():
        return source_dir
    return None


def sync_papyrus_sources(source_dir: Path, output_dir: Path) -> int:
    """Mirror loose Papyrus sources into extracted/Scripts/Source and return file count."""
    dest_dir = output_dir / "Scripts" / "Source"
    if dest_dir.exists():
        shutil.rmtree(dest_dir)
    dest_dir.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source_dir, dest_dir)
    return sum(1 for path in dest_dir.rglob("*") if path.is_file())


def _snapshot_dir(path: Path | None) -> dict:
    """Build a lightweight directory snapshot for smart-extract change detection."""
    if path is None or not path.is_dir():
        return {"exists": False}

    file_count = 0
    latest_mtime = 0.0
    total_size = 0
    for child in path.rglob("*"):
        if not child.is_file():
            continue
        try:
            stat = child.stat()
        except OSError:
            return {"exists": True, "path": str(path), "file_count": -1, "latest_mtime": -1.0, "total_size": -1}
        file_count += 1
        total_size += stat.st_size
        latest_mtime = max(latest_mtime, stat.st_mtime)

    return {
        "exists": True,
        "path": str(path),
        "file_count": file_count,
        "latest_mtime": latest_mtime,
        "total_size": total_size,
    }


def extract_with_native_archive(
    archive_path: Path,
    output_dir: Path,
    archive_format: str,
    file_workers: int = 8,
    progress: Callable[[dict], bool | None] | None = None,
) -> int:
    """Extract a BA2/BSA archive using the native Rust backend."""
    output_dir.mkdir(parents=True, exist_ok=True)
    count = native_runtime.extract_archive(
        str(archive_path),
        str(output_dir),
        format=archive_format,
        workers=max(0, file_workers),
        progress=progress,
    )
    return int(count)


# ---------------------------------------------------------------------------
# Manifest helpers
# ---------------------------------------------------------------------------

def load_manifest(output_dir: Path) -> dict | None:
    """Load .ba2_manifest.json from output_dir. Returns None if missing or corrupt."""
    path = output_dir / ".ba2_manifest.json"
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError, OSError):
        return None


def build_manifest(game: str, source_dir: Path, archives: list, papyrus_source_dir: Path | None = None) -> dict:
    """Build a manifest dict from the current archive list."""
    entries = {}
    for a in archives:
        try:
            st = a.stat()
            entries[a.name] = {"size": st.st_size, "mtime": st.st_mtime}
        except OSError:
            # If stat fails, record sentinel values so mismatch is detected next time
            entries[a.name] = {"size": -1, "mtime": -1.0}
    return {
        "game": game,
        "source_dir": str(source_dir),
        "extracted_at": datetime.datetime.now().isoformat(timespec="seconds"),
        "archives": entries,
        "papyrus_source": _snapshot_dir(papyrus_source_dir),
    }


def manifest_matches(
    manifest: dict | None,
    source_dir: Path,
    archives: list,
    papyrus_source_dir: Path | None = None,
) -> bool:
    """Return True if every archive recorded in the manifest still has the same size/mtime.

    Only checks archives that were in the manifest -- new archives (e.g. from
    installed mods) are ignored so they don't trigger a false "Updates available".

    Returns False if:
    - manifest is None
    - source_dir differs
    - any manifest archive is missing from disk, or has different size/mtime

    Note: on FAT32 volumes mtime precision is 2 seconds; Steam updates
    always write a new mtime so this is still reliable in practice.
    """
    if manifest is None:
        return False
    if manifest.get("source_dir") != str(source_dir):
        return False
    stored = manifest.get("archives", {})
    if not stored:
        return False
    current_by_name = {a.name: a for a in archives}
    for name, entry in stored.items():
        a = current_by_name.get(name)
        if a is None:
            return False  # manifest archive removed from disk
        try:
            st = a.stat()
        except OSError:
            return False  # stat failure -> treat as changed
        if entry.get("size") != st.st_size or entry.get("mtime") != st.st_mtime:
            return False
    if manifest.get("papyrus_source") != _snapshot_dir(papyrus_source_dir):
        return False
    return True


def save_manifest(output_dir: Path, manifest: dict) -> None:
    """Write manifest to {output_dir}/.ba2_manifest.json (atomic: write temp then rename)."""
    path = output_dir / ".ba2_manifest.json"
    try:
        fd, tmp = tempfile.mkstemp(dir=str(output_dir), suffix=".tmp")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(manifest, f, indent=2)
            Path(tmp).replace(path)
        except Exception:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise
    except Exception as e:
        _log.warning("Could not write manifest: %s", e)


def extract_one(
    archive: Path, output_dir: Path, archive_format: str,
    file_workers: int = 8,
    progress: Callable[[dict], bool | None] | None = None,
) -> tuple[Path, int, str | None]:
    """Extract a single archive. Returns (archive, file_count, error_or_None).

    Uses the native Rust backend for all archive extraction.
    """
    if archive_format not in {"ba2", "bsa"}:
        return archive, 0, f"unknown format: {archive_format}"

    try:
        count = extract_with_native_archive(
            archive,
            output_dir,
            archive_format,
            file_workers=file_workers,
            progress=progress,
        )
        return archive, count, None
    except Exception as e:
        return archive, 0, str(e)
