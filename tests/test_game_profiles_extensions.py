"""Tests for GameProfile multi-game extensions."""
from creation_lib.core.game_profiles import (
    get_profile, GAME_PROFILES, FO4_PROFILE, SKYRIMSE_PROFILE,
    FO76_PROFILE, STARFIELD_PROFILE,
)


def test_master_esm_for_each_game():
    assert get_profile("fo4").master_esm == "Fallout4.esm"
    assert get_profile("skyrimse").master_esm == "Skyrim.esm"
    assert get_profile("starfield").master_esm == "Starfield.esm"
    assert get_profile("fo76").master_esm == "SeventySix.esm"
    assert get_profile("fo3").master_esm == "Fallout3.esm"
    assert get_profile("fnv").master_esm == "FalloutNV.esm"


def test_fo76_not_moddable():
    p = get_profile("fo76")
    assert p.is_moddable is False


def test_moddable_games():
    for gid in ("fo4", "skyrimse", "starfield"):
        assert get_profile(gid).is_moddable is True


def test_engine_field():
    assert get_profile("fo4").engine == "creation1"
    assert get_profile("skyrimse").engine == "creation1"
    assert get_profile("starfield").engine == "creation2"
    assert get_profile("fo76").engine == "creation1"


def test_steam_app_ids():
    assert get_profile("fo4").steam_app_id == 377160
    assert get_profile("skyrimse").steam_app_id == 489830
    assert get_profile("starfield").steam_app_id == 1716740
    assert get_profile("fo76").steam_app_id == 1151340


def test_executable_names():
    assert get_profile("fo4").executable_name == "Fallout4.exe"
    assert get_profile("skyrimse").executable_name == "SkyrimSE.exe"
    assert get_profile("starfield").executable_name == "Starfield.exe"


def test_wiki_dirs():
    assert get_profile("fo4").wiki_dir == "fo4_wiki"
    assert get_profile("skyrimse").wiki_dir == "skyrim_wiki"
    assert get_profile("fo3").wiki_dir == "fo3_nv_wiki"
    assert get_profile("fnv").wiki_dir == "fo3_nv_wiki"
    assert get_profile("starfield").wiki_dir is None
    assert get_profile("fo76").wiki_dir is None


def test_skyrimse_env_var_name_fixed():
    """Regression: env_var_name was 'SKYRIM_EXTRACTED_DIR', should be 'SKYRIMSE_EXTRACTED_DIR'."""
    p = get_profile("skyrimse")
    assert p.env_var_name == "SKYRIMSE_EXTRACTED_DIR"


def test_all_profiles_registered():
    assert set(GAME_PROFILES.keys()) == {
        "fo4", "skyrimse", "fo76", "starfield", "fo3", "fnv",
    }
