"""The voice fields every in-scope game must declare, and their invariants."""
import pytest

from creation_lib.core.game_profiles import GAME_PROFILES

SUPPORTED = ("fo4", "skyrimse", "fnv", "fo3", "starfield")

EXPECTED_CONTAINERS = {
    "fo4": "fuz",
    "skyrimse": "fuz",
    "fnv": "ogg",
    "fo3": "ogg",
    "starfield": "wav",
}

EXPECTED_LIP = {
    "fo4": "embedded",
    "skyrimse": "embedded",
    "fnv": "sidecar",
    "fo3": "sidecar",
    "starfield": None,
}

EXPECTED_FACEFX = {
    "fo4": "Fallout4",
    "skyrimse": "Skyrim",
    "fnv": "Skyrim",
    "fo3": "Skyrim",
    "starfield": None,
}


@pytest.mark.parametrize("game", SUPPORTED)
def test_container_and_lip_and_facefx(game):
    profile = GAME_PROFILES[game]
    assert profile.voice_container == EXPECTED_CONTAINERS[game]
    assert profile.voice_lip == EXPECTED_LIP[game]
    assert profile.facefx_game == EXPECTED_FACEFX[game]


@pytest.mark.parametrize("game", SUPPORTED)
def test_masters_are_non_empty_esm_names(game):
    masters = GAME_PROFILES[game].voice_official_masters
    assert isinstance(masters, tuple)
    assert masters, f"{game} must pin at least one master"
    # _PLUGIN_EXTS accepts only .esm, so a typo'd .esl would silently index nothing.
    assert all(name.lower().endswith(".esm") for name in masters)
    assert len(set(masters)) == len(masters), "duplicate master"


@pytest.mark.parametrize("game", SUPPORTED)
def test_lip_and_container_are_consistent(game):
    profile = GAME_PROFILES[game]
    if profile.voice_container == "wav":
        assert profile.voice_lip is None
        assert profile.facefx_game is None
    if profile.voice_lip == "embedded":
        assert profile.voice_container == "fuz"
    if profile.voice_lip is not None:
        assert profile.facefx_game, "a game that carries lip needs a FaceFX type"


def test_fo4_masters_match_the_list_falltalk_used():
    assert GAME_PROFILES["fo4"].voice_official_masters == (
        "Fallout4.esm",
        "DLCRobot.esm",
        "DLCworkshop01.esm",
        "DLCCoast.esm",
        "DLCworkshop02.esm",
        "DLCworkshop03.esm",
        "DLCNukaWorld.esm",
    )


def test_starfield_includes_its_bundled_creations():
    masters = GAME_PROFILES["starfield"].voice_official_masters
    assert masters[0] == "Starfield.esm"
    for creation in ("SFBGS003.esm", "SFBGS004.esm", "SFBGS00D.esm"):
        assert creation in masters


def test_out_of_scope_profiles_keep_the_defaults():
    for game in ("oblivion", "fo76"):
        profile = GAME_PROFILES[game]
        assert profile.voice_official_masters == ()
        assert profile.voice_container == "wav"
        assert profile.voice_lip is None
        assert profile.facefx_game is None
