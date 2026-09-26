"""Tests for GameProfile multi-game extensions."""
from creation_lib.core.game_profiles import get_profile, GAME_PROFILES


def test_all_profiles_registered_and_moddable_flag():
    assert set(GAME_PROFILES.keys()) == {
        "fo4", "skyrimse", "fo76", "starfield", "fo3", "fnv", "oblivion",
    }
    assert get_profile("fo76").is_moddable is False
    for gid in ("fo4", "skyrimse", "starfield"):
        assert get_profile(gid).is_moddable is True


def test_skyrimse_env_var_name_fixed():
    """Regression: env_var_name was 'SKYRIM_EXTRACTED_DIR', should be 'SKYRIMSE_EXTRACTED_DIR'."""
    assert get_profile("skyrimse").env_var_name == "SKYRIMSE_EXTRACTED_DIR"
