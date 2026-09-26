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
def test_voice_fields_and_invariants(game):
    profile = GAME_PROFILES[game]
    assert profile.voice_container == EXPECTED_CONTAINERS[game]
    assert profile.voice_lip == EXPECTED_LIP[game]
    assert profile.facefx_game == EXPECTED_FACEFX[game]

    masters = profile.voice_official_masters
    assert isinstance(masters, tuple)
    assert masters, f"{game} must pin at least one master"
    # _PLUGIN_EXTS accepts only .esm, so a typo'd .esl would silently index nothing.
    assert all(name.lower().endswith(".esm") for name in masters)
    assert len(set(masters)) == len(masters), "duplicate master"

    if profile.voice_container == "wav":
        assert profile.voice_lip is None
        assert profile.facefx_game is None
    if profile.voice_lip == "embedded":
        assert profile.voice_container == "fuz"
    if profile.voice_lip is not None:
        assert profile.facefx_game, "a game that carries lip needs a FaceFX type"


def test_out_of_scope_profiles_keep_the_defaults():
    for game in ("oblivion", "fo76"):
        profile = GAME_PROFILES[game]
        assert profile.voice_official_masters == ()
        assert profile.voice_container == "wav"
        assert profile.voice_lip is None
        assert profile.facefx_game is None
