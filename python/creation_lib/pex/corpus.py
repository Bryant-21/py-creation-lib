"""Bundled per-game Papyrus type universes.

The compiler resolves every type from source, and the shipping games carry only
compiled `.pex` — the `.psc` are a Creation Kit deliverable. Synthesizing
headers from the player's own install covers the machines that have one; this
covers the rest, so the compiler runs with no game installed at all.

One archive per game, built by `tools/gen_papyrus_corpus.py` and expanded on
first use. The archive is the shipped unit because a corpus is ~10000 files: as
loose files it would dominate the repository and every diff. `.tar.gz` rather
than `.zip` — compressing the corpus as one stream instead of 10000 separate
members takes it from 3.3 MiB to under 1 MiB.
"""
from __future__ import annotations

import os
import shutil
import sys
import tarfile
from hashlib import sha256
from pathlib import Path

DATA_DIR = Path(__file__).with_name("data") / "corpus"

FLAGS_SUFFIX = ".flg"


def bundled_corpus_archive(game: str) -> Path | None:
    """The shipped archive for `game`, if one was built for it."""
    candidate = DATA_DIR / f"{game.lower()}.tar.gz"
    return candidate if candidate.is_file() else None


def _cache_root() -> Path:
    if bool(getattr(sys, "frozen", False)):
        return Path(sys.executable).resolve().parent / "cache" / "papyrus_corpus"
    local_app_data = os.environ.get("LOCALAPPDATA")
    root = Path(local_app_data) if local_app_data else Path.home() / "AppData" / "Local"
    return root / "modkit21" / "papyrus_corpus"


def _archive_key(archive: Path) -> str:
    digest = sha256()
    with archive.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()[:12]


def bundled_corpus_root(game: str) -> Path | None:
    """Expand the shipped corpus for `game` and return its root.

    Keyed by archive content, so shipping a rebuilt corpus expands afresh rather
    than mixing generations. Returns None when no corpus ships for the game.
    """
    archive = bundled_corpus_archive(game)
    if archive is None:
        return None

    target = _cache_root() / f"{game.lower()}-{_archive_key(archive)}"
    if (target / ".complete").is_file():
        return target

    # Extract beside the target and move it into place, so a killed run never
    # leaves a half-corpus that looks finished. A partial type universe is worse
    # than none: it resolves some calls and types the rest as None, and those
    # errors surface on the caller's lines.
    staging = target.with_name(f"{target.name}.{os.getpid()}.part")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir(parents=True, exist_ok=True)
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            bundle.extractall(staging, filter="data")
        (staging / ".complete").write_text("", encoding="utf-8")
        try:
            staging.replace(target)
        except OSError:
            # Another process finished first; its copy is equivalent.
            shutil.rmtree(staging, ignore_errors=True)
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    return target if (target / ".complete").is_file() else None


def bundled_flags_file(root: Path) -> Path | None:
    """The user-flag definitions shipped alongside a corpus."""
    return next(Path(root).glob(f"*{FLAGS_SUFFIX}"), None)
