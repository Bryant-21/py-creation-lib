"""Local GOG-install checks for release workflows."""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.core.path_detector import validate_game_path


_INFO_GLOB = "goggame-*.info"


@dataclass(frozen=True)
class GogInstallResult:
    ok: bool
    game_id: str
    root_dir: str
    local_install_valid: bool
    info_present: bool
    info_parsed: bool
    play_task_present: bool
    product_id: str
    info_path: str
    message: str


def validate_gog_install_for_game(
    game_id: str,
    root_or_data_dir: str,
) -> GogInstallResult:
    profile = GAME_PROFILES.get(game_id)
    root_dir = _install_root_from_path(root_or_data_dir)
    root_text = str(root_dir) if root_dir is not None else str(root_or_data_dir or "")

    if profile is None:
        return _result(False, game_id, root_text, "Unknown game profile.")

    local_valid = root_dir is not None and validate_game_path(game_id, str(root_dir))
    if not local_valid:
        return _result(
            False,
            game_id,
            root_text,
            f"{profile.display_name} install is invalid: executable or Data archives not found.",
        )

    assert root_dir is not None
    info_files = sorted(root_dir.glob(_INFO_GLOB))
    if not info_files:
        return _result(
            False,
            game_id,
            root_text,
            f"No GOG {_INFO_GLOB} manifest was found in the {profile.display_name} folder.",
            local_install_valid=True,
        )

    parsed: list[tuple[Path, str]] = []
    for path in info_files:
        product_id = _product_id(path)
        if product_id:
            parsed.append((path, product_id))

    if not parsed:
        return _result(
            False,
            game_id,
            root_text,
            f"GOG manifest {info_files[0].name} could not be read or has no gameId.",
            local_install_valid=True,
            info_present=True,
            info_path=str(info_files[0]),
        )

    launching = next(
        ((path, pid) for path, pid in parsed if _has_resolvable_play_task(path, root_dir)),
        None,
    )
    if launching is None:
        info_path, product_id = parsed[0]
        return _result(
            False,
            game_id,
            root_text,
            (
                f"GOG manifest {info_path.name} does not launch any executable "
                f"present in the {profile.display_name} folder."
            ),
            local_install_valid=True,
            info_present=True,
            info_parsed=True,
            product_id=product_id,
            info_path=str(info_path),
        )

    info_path, product_id = launching
    return GogInstallResult(
        ok=True,
        game_id=game_id,
        root_dir=root_text,
        local_install_valid=True,
        info_present=True,
        info_parsed=True,
        play_task_present=True,
        product_id=product_id,
        info_path=str(info_path),
        message=f"{profile.display_name} GOG install verified.",
    )


def _result(
    ok: bool,
    game_id: str,
    root_dir: str,
    message: str,
    *,
    local_install_valid: bool = False,
    info_present: bool = False,
    info_parsed: bool = False,
    product_id: str = "",
    info_path: str = "",
) -> GogInstallResult:
    return GogInstallResult(
        ok=ok,
        game_id=game_id,
        root_dir=root_dir,
        local_install_valid=local_install_valid,
        info_present=info_present,
        info_parsed=info_parsed,
        play_task_present=False,
        product_id=product_id,
        info_path=info_path,
        message=message,
    )


def _install_root_from_path(value: str) -> Path | None:
    text = str(value or "").strip()
    if not text:
        return None
    path = Path(text).expanduser()
    if path.name.lower() == "data":
        path = path.parent
    return path


def _load_info(path: Path) -> dict | None:
    try:
        # GOG writes these as UTF-8, occasionally with a BOM.
        text = path.read_text(encoding="utf-8-sig", errors="replace")
    except OSError:
        return None
    try:
        payload = json.loads(text)
    except ValueError:
        return None
    return payload if isinstance(payload, dict) else None


def _product_id(path: Path) -> str:
    payload = _load_info(path)
    if payload is None:
        return ""
    game_id = payload.get("gameId")
    if isinstance(game_id, (str, int)) and str(game_id).strip():
        return str(game_id).strip()
    return ""


def _has_resolvable_play_task(path: Path, root_dir: Path) -> bool:
    payload = _load_info(path)
    if payload is None:
        return False
    tasks = payload.get("playTasks")
    if not isinstance(tasks, list):
        return False
    for task in tasks:
        if not isinstance(task, dict):
            continue
        rel = task.get("path")
        if not isinstance(rel, str) or not rel.strip():
            continue
        candidate = root_dir / rel.replace("\\", "/").lstrip("/")
        try:
            if candidate.is_file():
                return True
        except OSError:
            continue
    return False
