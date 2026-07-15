"""Pure env mapping helpers for ESP boundary callers."""

from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path

_ENV_KEYS: dict[str, dict[str, tuple[str, ...]]] = {
    "fo4": {
        "install": ("FO4_DIR",),
        "extracted": ("FO4_EXTRACTED_DIR",),
    },
    "fo3": {
        "install": ("FO3_DIR",),
        "extracted": ("FO3_EXTRACTED_DIR",),
    },
    "fnv": {
        "install": ("FNV_DIR", "FONV_DIR"),
        "extracted": ("FNV_EXTRACTED_DIR", "FONV_EXTRACTED_DIR"),
    },
    "fo76": {
        "install": ("FO76_DIR",),
        "extracted": ("FO76_EXTRACTED_DIR",),
    },
    "starfield": {
        "install": ("STARFIELD_DIR",),
        "extracted": ("STARFIELD_EXTRACTED_DIR",),
    },
    "skyrimse": {
        "install": ("SKYRIMSE_DIR",),
        "extracted": ("SKYRIMSE_EXTRACTED_DIR",),
    },
}

_GAME_ALIASES = {
    "oblivion": "oblivion",
    "the elder scrolls iv oblivion": "oblivion",
    "tes4": "oblivion",
    "fallout4": "fo4",
    "fallout 4": "fo4",
    "fallout3": "fo3",
    "fallout 3": "fo3",
    "falloutnv": "fnv",
    "fallout new vegas": "fnv",
    "fonv": "fnv",
    "skyrim": "skyrimse",
    "skyrim special edition": "skyrimse",
    "sse": "skyrimse",
    "tes5": "skyrimse",
    "fallout76": "fo76",
    "fallout 76": "fo76",
    "sf1": "starfield",
}


def _canonical_game_id(game: str) -> str:
    value = (game or "").strip().lower()
    return _GAME_ALIASES.get(value, value)


def resolve_env_keys(game: str, kind: str) -> tuple[str, ...]:
    canonical = _canonical_game_id(game)
    keys = _ENV_KEYS.get(canonical, {}).get(kind)
    if keys:
        return keys
    stem = canonical.upper()
    if kind == "install":
        return (f"{stem}_DIR",)
    if kind == "extracted":
        return (f"{stem}_EXTRACTED_DIR",)
    raise KeyError(f"Unsupported env kind: {kind}")


def resolve_game_path(
    game: str,
    kind: str,
    *,
    env: Mapping[str, str],
) -> Path | None:
    for key in resolve_env_keys(game, kind):
        value = env.get(key, "").strip()
        if value:
            return Path(value)
    return None


def resolve_game_install_dir(
    game: str,
    *,
    env: Mapping[str, str],
) -> Path | None:
    return resolve_game_path(game, "install", env=env)


def resolve_game_data_dir(
    game: str,
    *,
    env: Mapping[str, str],
) -> Path | None:
    install = resolve_game_install_dir(game, env=env)
    if install is None:
        return None
    return install / "Data"


def resolve_game_strings_dir(
    game: str,
    *,
    env: Mapping[str, str],
) -> Path | None:
    for candidate in resolve_game_strings_dirs(game, env=env):
        if candidate.exists():
            return candidate
    data_dir = resolve_game_data_dir(game, env=env)
    if data_dir is not None:
        return data_dir / "Strings"
    extracted_dir = resolve_game_path(game, "extracted", env=env)
    if extracted_dir is not None:
        return extracted_dir / "Strings"
    return None


def resolve_game_strings_dirs(
    game: str,
    *,
    env: Mapping[str, str],
) -> list[Path]:
    candidates: list[Path] = []
    data_dir = resolve_game_data_dir(game, env=env)
    if data_dir is not None:
        candidates.append(data_dir / "Strings")
    extracted_dir = resolve_game_path(game, "extracted", env=env)
    if extracted_dir is not None:
        candidates.append(extracted_dir / "Strings")
        candidates.append(extracted_dir / "Data" / "Strings")
    unique: list[Path] = []
    seen: set[Path] = set()
    for candidate in candidates:
        if candidate in seen:
            continue
        seen.add(candidate)
        unique.append(candidate)
    return unique
