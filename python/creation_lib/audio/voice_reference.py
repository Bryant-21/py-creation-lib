"""Voice-line reference indexing for Bethesda dialogue archives.

The UI supplies concrete paths. This module does not read project or process
configuration.
"""

from __future__ import annotations

import hashlib
import json
import logging
import re
import unicodedata
from collections import defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

from creation_lib.ba2 import native_runtime
from creation_lib.core.game_profiles import GAME_PROFILES, get_profile
from creation_lib.esp import native_runtime as esp_native_runtime

_log = logging.getLogger("creation_lib.audio.voice_reference")

ProgressCallback = Callable[[int, int, str], None]

_ARCHIVE_EXTS = {".ba2", ".bsa"}
_PLUGIN_EXTS = {".esm"}
_CACHE_VERSION = 2

_VOICE_LANGUAGE_CODES = {
    "English": "en",
    "German": "de",
    "Spanish": "es",
    "French": "fr",
    "Italian": "it",
    "Japanese": "ja",
    "Korean": "ko",
    "Polish": "pl",
    "Russian": "ru",
    "Chinese": "zh",
}
# Matches "... - Voices_de" and "... - Voices_en0"; untagged names like
# "Fallout4 - Voices" or "Starfield - Voices01" deliberately do not match.
_VOICE_LANGUAGE_RE = re.compile(r"voices?_([a-z]{2})\d*$", re.IGNORECASE)


@dataclass(slots=True)
class VoiceLine:
    game: str
    plugin: str
    info_form_id: str
    response_number: int
    response_text: str
    response_filename: str
    voice_type: str = ""
    characters: list[str] = field(default_factory=list)
    archive_path: str = ""
    member_path: str = ""
    topic_form_id: str = ""
    topic_text: str = ""

    @property
    def available(self) -> bool:
        return bool(self.archive_path and self.member_path)

    @property
    def group_label(self) -> str:
        if self.characters:
            return ", ".join(self.characters[:3])
        if self.voice_type:
            return self.voice_type
        return "Unknown Voice"

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        payload["available"] = self.available
        payload["group_label"] = self.group_label
        return payload

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "VoiceLine":
        return cls(
            game=str(data.get("game", "")),
            plugin=str(data.get("plugin", "")),
            info_form_id=str(data.get("info_form_id", "")),
            response_number=int(data.get("response_number", 0) or 0),
            response_text=str(data.get("response_text", "")),
            response_filename=str(data.get("response_filename", "")),
            voice_type=str(data.get("voice_type", "")),
            characters=_clean_character_names([str(item) for item in data.get("characters", []) or []]),
            archive_path=str(data.get("archive_path", "")),
            member_path=str(data.get("member_path", "")),
            topic_form_id=str(data.get("topic_form_id", "")),
            topic_text=str(data.get("topic_text", "")),
        )

    @classmethod
    def from_native_row(cls, row: Sequence[Any]) -> "VoiceLine":
        return cls(
            game=str(row[0]),
            plugin=str(row[1]),
            info_form_id=str(row[2]),
            response_number=int(row[3]),
            response_text=str(row[4]),
            response_filename=str(row[5]),
            voice_type=str(row[6]),
            characters=_clean_character_names([str(item) for item in row[7] or []]),
            archive_path=str(row[8]),
            member_path=str(row[9]),
            topic_form_id=str(row[10]),
            topic_text=str(row[11]),
        )


@dataclass(slots=True)
class VoiceReferenceIndex:
    game: str
    language: str
    data_dir: str
    strings_dir: str
    plugin_paths: list[str]
    archive_paths: list[str]
    lines: list[VoiceLine]
    cache_key: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "game": self.game,
            "language": self.language,
            "data_dir": self.data_dir,
            "strings_dir": self.strings_dir,
            "plugin_paths": self.plugin_paths,
            "archive_paths": self.archive_paths,
            "cache_key": self.cache_key,
            "lines": [line.to_dict() for line in self.lines],
        }

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> "VoiceReferenceIndex":
        return cls(
            game=str(data.get("game", "")),
            language=str(data.get("language", "")),
            data_dir=str(data.get("data_dir", "")),
            strings_dir=str(data.get("strings_dir", "")),
            plugin_paths=[str(item) for item in data.get("plugin_paths", []) or []],
            archive_paths=[str(item) for item in data.get("archive_paths", []) or []],
            cache_key=str(data.get("cache_key", "")),
            lines=[VoiceLine.from_dict(item) for item in data.get("lines", []) or []],
        )


def discover_official_plugins(data_dir: str | Path, game: str) -> list[Path]:
    """Return official master plugins from a game Data directory.

    The initial voice browser scope is official installed data, so only ESMs are
    considered. The primary master is ordered first when present.
    """
    root = Path(data_dir)
    if not root.is_dir():
        return []
    profile = get_profile(game)
    plugins = sorted(
        [path for path in root.iterdir() if path.is_file() and path.suffix.lower() in _PLUGIN_EXTS],
        key=lambda path: path.name.lower(),
    )
    if profile.master_esm:
        master_name = profile.master_esm.lower()
        plugins.sort(key=lambda path: (0 if path.name.lower() == master_name else 1, path.name.lower()))
    return plugins


def discover_archives(data_dir: str | Path, *, language: str = "English") -> list[Path]:
    """Archives in a game Data directory, excluding other languages' voices.

    Per-language voice archives share member paths, so indexing more than one
    language would make the audio behind a voice line non-deterministic.
    """
    root = Path(data_dir)
    if not root.is_dir():
        return []
    wanted = _VOICE_LANGUAGE_CODES.get(language, "en")
    archives = []
    for path in root.iterdir():
        if not path.is_file() or path.suffix.lower() not in _ARCHIVE_EXTS:
            continue
        match = _VOICE_LANGUAGE_RE.search(path.stem)
        if match and match.group(1).lower() != wanted:
            continue
        archives.append(path)
    return sorted(archives, key=lambda path: path.name.lower())


def build_voice_reference(
    *,
    game: str,
    data_dir: str | Path,
    strings_dir: str | Path | None = None,
    db_dir: str | Path | None = None,
    language: str = "English",
    force: bool = False,
    progress: ProgressCallback | None = None,
    plugin_paths: Iterable[str | Path] | None = None,
    archive_paths: Iterable[str | Path] | None = None,
) -> VoiceReferenceIndex:
    """Build or load a voice reference index for one game."""
    if game not in GAME_PROFILES:
        raise ValueError(f"Unsupported game: {game}")

    root = Path(data_dir).expanduser().resolve(strict=False)
    if not root.is_dir():
        raise FileNotFoundError(f"Data directory not found: {root}")

    resolved_strings = (
        Path(strings_dir).expanduser().resolve(strict=False)
        if strings_dir
        else root / "Strings"
    )
    _log.info(
        "Building voice reference: game=%s data_dir=%s strings_dir=%s language=%s force=%s",
        game,
        root,
        resolved_strings,
        language,
        force,
    )
    plugins = [Path(path).expanduser().resolve(strict=False) for path in (plugin_paths or discover_official_plugins(root, game))]
    archives = [Path(path).expanduser().resolve(strict=False) for path in (archive_paths or discover_archives(root, language=language))]
    plugins = [path for path in plugins if path.is_file()]
    archives = [path for path in archives if path.is_file()]
    _log.info("Voice reference inputs: %d plugin(s), %d archive(s)", len(plugins), len(archives))

    cache_key = _build_cache_key(
        game=game,
        data_dir=root,
        strings_dir=resolved_strings,
        language=language,
        plugin_paths=plugins,
        archive_paths=archives,
    )
    cache_path = _json_fallback_path(db_dir, game, cache_key)
    sqlite_path = _sqlite_db_path(db_dir, game)
    native_error: Exception | None = None
    if sqlite_path is not None:
        try:
            if progress:
                progress(0, 1, "Building native SQLite voice reference")
            _log.info("Using native SQLite voice reference index: %s", sqlite_path)
            summary = esp_native_runtime.voice_reference_build_index(
                str(sqlite_path),
                game,
                str(root),
                str(resolved_strings),
                language,
                cache_key,
                [str(path) for path in plugins],
                [str(path) for path in archives],
                force=force,
            )
            rows = esp_native_runtime.voice_reference_read_index(str(sqlite_path))
            lines = [VoiceLine.from_native_row(row) for row in rows]
            if progress:
                progress(1, 1, f"Indexed {len(lines):,} voice lines")
            _log.info(
                "Loaded native SQLite voice reference: %s (%d line(s), reused=%s)",
                sqlite_path,
                len(lines),
                bool(summary[2]),
            )
            return VoiceReferenceIndex(
                game=game,
                language=language,
                data_dir=str(root),
                strings_dir=str(resolved_strings),
                plugin_paths=[str(path) for path in plugins],
                archive_paths=[str(path) for path in archives],
                lines=lines,
                cache_key=cache_key,
            )
        except Exception as exc:
            native_error = exc
            _log.warning("Native SQLite voice reference build failed", exc_info=True)
    else:
        native_error = RuntimeError("native voice reference build requires db_dir for the SQLite cache")

    if cache_path is not None and not force and cache_path.is_file():
        try:
            cached = VoiceReferenceIndex.from_dict(json.loads(cache_path.read_text(encoding="utf-8")))
            if cached.cache_key == cache_key:
                _log.info("Loaded voice reference cache: %s (%d line(s))", cache_path, len(cached.lines))
                if progress:
                    progress(1, 1, "Loaded cached voice reference")
                return cached
        except Exception:
            _log.warning("Ignoring stale voice reference cache: %s", cache_path, exc_info=True)

    if native_error is not None:
        raise native_error
    raise RuntimeError("native voice reference build is unavailable")


def voice_reference_sqlite_cache_path(
    *,
    game: str,
    data_dir: str | Path,
    strings_dir: str | Path | None = None,
    db_dir: str | Path | None = None,
    language: str = "English",
    plugin_paths: Iterable[str | Path] | None = None,
    archive_paths: Iterable[str | Path] | None = None,
) -> Path | None:
    """Return the native SQLite cache path for the current voice-reference inputs."""
    if game not in GAME_PROFILES:
        raise ValueError(f"Unsupported game: {game}")
    root = Path(data_dir).expanduser().resolve(strict=False)
    if not root.is_dir():
        return None
    resolved_strings = (
        Path(strings_dir).expanduser().resolve(strict=False)
        if strings_dir
        else root / "Strings"
    )
    plugins = [Path(path).expanduser().resolve(strict=False) for path in (plugin_paths or discover_official_plugins(root, game))]
    archives = [Path(path).expanduser().resolve(strict=False) for path in (archive_paths or discover_archives(root, language=language))]
    plugins = [path for path in plugins if path.is_file()]
    archives = [path for path in archives if path.is_file()]
    cache_key = _build_cache_key(
        game=game,
        data_dir=root,
        strings_dir=resolved_strings,
        language=language,
        plugin_paths=plugins,
        archive_paths=archives,
    )
    return _sqlite_db_path(db_dir, game)


def load_cached_voice_reference(
    *,
    game: str,
    data_dir: str | Path,
    strings_dir: str | Path | None = None,
    db_dir: str | Path | None = None,
    language: str = "English",
    plugin_paths: Iterable[str | Path] | None = None,
    archive_paths: Iterable[str | Path] | None = None,
) -> VoiceReferenceIndex | None:
    """Load an existing voice-reference cache without building a new index."""
    if game not in GAME_PROFILES:
        raise ValueError(f"Unsupported game: {game}")
    root = Path(data_dir).expanduser().resolve(strict=False)
    if not root.is_dir():
        return None
    resolved_strings = (
        Path(strings_dir).expanduser().resolve(strict=False)
        if strings_dir
        else root / "Strings"
    )
    plugins = [Path(path).expanduser().resolve(strict=False) for path in (plugin_paths or discover_official_plugins(root, game))]
    archives = [Path(path).expanduser().resolve(strict=False) for path in (archive_paths or discover_archives(root, language=language))]
    plugins = [path for path in plugins if path.is_file()]
    archives = [path for path in archives if path.is_file()]
    cache_key = _build_cache_key(
        game=game,
        data_dir=root,
        strings_dir=resolved_strings,
        language=language,
        plugin_paths=plugins,
        archive_paths=archives,
    )

    sqlite_path = _sqlite_db_path(db_dir, game)
    if sqlite_path is not None and sqlite_path.is_file():
        rows = esp_native_runtime.voice_reference_read_index(str(sqlite_path))
        lines = [VoiceLine.from_native_row(row) for row in rows]
        _rebase_archive_paths(lines, root, archives)
        _log.info("Loaded cached SQLite voice reference: %s (%d line(s))", sqlite_path, len(lines))
        return VoiceReferenceIndex(
            game=game,
            language=language,
            data_dir=str(root),
            strings_dir=str(resolved_strings),
            plugin_paths=[str(path) for path in plugins],
            archive_paths=[str(path) for path in archives],
            lines=lines,
            cache_key=cache_key,
        )

    legacy_sqlite_path = _legacy_sqlite_fallback_path(db_dir, game)
    if legacy_sqlite_path is not None:
        rows = esp_native_runtime.voice_reference_read_index(str(legacy_sqlite_path))
        lines = [VoiceLine.from_native_row(row) for row in rows]
        _rebase_archive_paths(lines, root, archives)
        _log.info("Loaded legacy SQLite voice reference: %s (%d line(s))", legacy_sqlite_path, len(lines))
        return VoiceReferenceIndex(
            game=game,
            language=language,
            data_dir=str(root),
            strings_dir=str(resolved_strings),
            plugin_paths=[str(path) for path in plugins],
            archive_paths=[str(path) for path in archives],
            lines=lines,
            cache_key=cache_key,
        )

    json_path = _json_fallback_path(db_dir, game, cache_key)
    if json_path is not None and json_path.is_file():
        cached = VoiceReferenceIndex.from_dict(json.loads(json_path.read_text(encoding="utf-8")))
        if cached.cache_key == cache_key:
            _rebase_archive_paths(cached.lines, root, archives)
            _log.info("Loaded cached JSON voice reference: %s (%d line(s))", json_path, len(cached.lines))
            return cached
    return None


def read_voice_lines(db_path: str | Path) -> list[VoiceLine]:
    """Read every line from a prebuilt voice-reference database.

    Unlike load_cached_voice_reference this needs no game install: it is for
    shipped or copied index files whose archives may not be mounted locally.
    """
    path = Path(db_path)
    if not path.is_file():
        raise FileNotFoundError(f"Voice reference database not found: {path}")
    rows = esp_native_runtime.voice_reference_read_index(str(path))
    _log.info("Read voice reference database: %s (%d line(s))", path, len(rows))
    return [VoiceLine.from_native_row(row) for row in rows]


def search_voice_lines(
    index: VoiceReferenceIndex,
    query: str = "",
    *,
    group: str = "",
    available_only: bool = False,
) -> list[VoiceLine]:
    needle = query.strip().lower()
    group_key = group.strip().lower()
    result: list[VoiceLine] = []
    for line in index.lines:
        if available_only and not line.available:
            continue
        if group_key and line.group_label.lower() != group_key:
            continue
        if needle and needle not in _line_search_text(line):
            continue
        result.append(line)
    return result


def group_voice_lines(
    index: VoiceReferenceIndex,
    query: str = "",
    *,
    available_only: bool = False,
) -> list[tuple[str, int]]:
    counts: dict[str, int] = defaultdict(int)
    for line in search_voice_lines(index, query, available_only=available_only):
        counts[line.group_label] += 1
    return sorted(counts.items(), key=lambda item: (item[0].lower()))


def extract_voice_line(line: VoiceLine, output_dir: str | Path, *, preserve_tree: bool = False) -> Path:
    if not line.available:
        raise ValueError("Voice line has no matching archive member")
    out_dir = Path(output_dir).expanduser().resolve(strict=False)
    out_dir.mkdir(parents=True, exist_ok=True)
    target = out_dir.joinpath(*line.member_path.split("/")) if preserve_tree else out_dir / Path(line.member_path).name
    target.parent.mkdir(parents=True, exist_ok=True)
    if not Path(line.archive_path).is_file():
        raise FileNotFoundError(f"Archive not found: {line.archive_path}")
    data = native_runtime.extract_one(line.archive_path, line.member_path)
    if data is None:
        raise FileNotFoundError(f"Archive member not found: {line.member_path}")
    target.write_bytes(bytes(data))
    return target


def _rebase_archive_paths(
    lines: Iterable[VoiceLine],
    data_dir: Path,
    archives: Sequence[Path],
) -> None:
    """Re-point cached archive paths at this machine's archives, in place.

    An index records the absolute path of every archive it read, so a shipped
    index — or one built before the game moved — names archives that do not
    exist here. Archive filenames are stable, so each stored path is matched by
    name against the archives the caller resolved for this run, which is the
    authoritative set (discovered from the data directory, or passed in
    explicitly). A name the caller does not have is reported under the local
    data directory, so failures name a path on this machine rather than the
    build machine's.
    """
    by_name = {path.name.lower(): str(path) for path in archives}
    resolved: dict[str, str] = {}
    for line in lines:
        stored = line.archive_path
        if not stored:
            continue
        replacement = resolved.get(stored)
        if replacement is None:
            name = stored.replace("\\", "/").rsplit("/", 1)[-1]
            replacement = by_name.get(name.lower()) or str(data_dir / name)
            resolved[stored] = replacement
        line.archive_path = replacement


def _line_search_text(line: VoiceLine) -> str:
    return " ".join(
        [
            line.plugin,
            line.info_form_id,
            line.response_filename,
            line.response_text,
            line.voice_type,
            line.topic_form_id,
            line.topic_text,
            *line.characters,
        ]
    ).lower()


def _build_cache_key(
    *,
    game: str,
    data_dir: Path,
    strings_dir: Path,
    language: str,
    plugin_paths: Iterable[Path],
    archive_paths: Iterable[Path],
) -> str:
    payload: dict[str, Any] = {
        "version": _CACHE_VERSION,
        "game": game,
        "data_dir": str(data_dir),
        "strings_dir": str(strings_dir),
        "language": language,
        "plugins": [_file_fingerprint(path) for path in plugin_paths],
        "archives": [_file_fingerprint(path) for path in archive_paths],
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha1(raw).hexdigest()


def _file_fingerprint(path: Path) -> dict[str, Any]:
    stat = path.stat()
    return {
        "path": str(path),
        "size": stat.st_size,
        "mtime_ns": stat.st_mtime_ns,
    }


def _json_fallback_path(db_dir: str | Path | None, game: str, cache_key: str) -> Path | None:
    if db_dir is None:
        return None
    safe_game = re.sub(r"[^a-zA-Z0-9_.-]+", "_", game)
    return Path(db_dir) / "voice_reference" / f"{safe_game}_{cache_key}.json"


def _sqlite_db_path(db_dir: str | Path | None, game: str) -> Path | None:
    if db_dir is None:
        return None
    safe_game = re.sub(r"[^a-zA-Z0-9_.-]+", "_", game)
    return Path(db_dir) / f"{safe_game}_voice_reference.db"


def _legacy_sqlite_fallback_path(db_dir: str | Path | None, game: str) -> Path | None:
    if db_dir is None:
        return None
    safe_game = re.sub(r"[^a-zA-Z0-9_.-]+", "_", game)
    cache_dir = Path(db_dir) / "voice_reference"
    if not cache_dir.is_dir():
        return None
    candidates = [path for path in cache_dir.glob(f"{safe_game}_*.sqlite") if path.is_file()]
    if not candidates:
        return None
    return max(candidates, key=lambda path: path.stat().st_mtime_ns)


def _clean_character_names(names: Iterable[str]) -> list[str]:
    cleaned: list[str] = []
    for raw in names:
        name = str(raw).strip()
        if not name or "\ufffd" in name:
            continue
        if any(unicodedata.category(char).startswith("C") for char in name):
            continue
        question_count = name.count("?")
        if question_count >= 3 or (question_count and question_count / max(1, len(name)) > 0.25):
            continue
        alpha_count = sum(1 for char in name if char.isalpha())
        if len(name) <= 6 and alpha_count == 0:
            continue
        cleaned.append(name)
    return cleaned
