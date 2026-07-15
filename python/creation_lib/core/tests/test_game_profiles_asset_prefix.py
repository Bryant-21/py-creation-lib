from creation_lib.core.game_profiles import GAME_PROFILES


def test_asset_prefix_is_empty_for_each_known_game() -> None:
    for profile_id in ("fo4", "fo76", "fnv", "fo3", "skyrimse", "starfield"):
        profile = GAME_PROFILES[profile_id]
        assert profile.asset_prefix == ""


def test_asset_prefix_is_string_for_each_known_game() -> None:
    for profile_id in ("fo4", "fo76", "fnv", "fo3", "skyrimse", "starfield"):
        assert isinstance(GAME_PROFILES[profile_id].asset_prefix, str)
