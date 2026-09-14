"""Store-agnostic install verification (Steam or GOG) for release workflows."""
from __future__ import annotations

from dataclasses import dataclass

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.core.gog_install import GogInstallResult, validate_gog_install_for_game
from creation_lib.core.steam_install import SteamInstallResult, validate_steam_install_for_game


@dataclass(frozen=True)
class StoreInstallResult:
    ok: bool
    game_id: str
    store: str
    root_dir: str
    local_install_valid: bool
    message: str
    steam: SteamInstallResult
    gog: GogInstallResult


def validate_store_install_for_game(
    game_id: str,
    root_or_data_dir: str,
) -> StoreInstallResult:
    """Accept the install if either Steam or GOG ownership can be verified."""
    steam = validate_steam_install_for_game(game_id, root_or_data_dir)
    if steam.ok:
        return _result(game_id, "steam", steam.message, steam, None)

    gog = validate_gog_install_for_game(game_id, root_or_data_dir)
    if gog.ok:
        return _result(game_id, "gog", gog.message, steam, gog)

    # Neither store verified. A folder that isn't the game at all is the more
    # useful thing to say, and both validators agree on that message.
    if not steam.local_install_valid:
        return _result(game_id, "", steam.message, steam, gog)

    profile = GAME_PROFILES.get(game_id)
    display_name = profile.display_name if profile is not None else game_id
    message = (
        f"{display_name} was not recognized as a Steam or GOG install.\n"
        f"Steam: {steam.message}\n"
        f"GOG: {gog.message}"
    )
    return _result(game_id, "", message, steam, gog)


def _result(
    game_id: str,
    store: str,
    message: str,
    steam: SteamInstallResult,
    gog: GogInstallResult | None,
) -> StoreInstallResult:
    if gog is None:
        gog = _empty_gog(game_id, steam.root_dir)
    return StoreInstallResult(
        ok=bool(store),
        game_id=game_id,
        store=store,
        root_dir=steam.root_dir,
        local_install_valid=steam.local_install_valid,
        message=message,
        steam=steam,
        gog=gog,
    )


def _empty_gog(game_id: str, root_dir: str) -> GogInstallResult:
    return GogInstallResult(
        ok=False,
        game_id=game_id,
        root_dir=root_dir,
        local_install_valid=False,
        info_present=False,
        info_parsed=False,
        play_task_present=False,
        product_id="",
        info_path="",
        message="GOG verification not attempted.",
    )
