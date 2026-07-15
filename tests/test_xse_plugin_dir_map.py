"""Unit tests for the XSE_PLUGIN_DIR map and per-game path construction."""
from pathlib import Path

from creation_lib.build.deployer import XSE_PLUGIN_DIR, xse_plugin_dir_for


def test_xse_plugin_dir_map_covers_supported_games():
    assert XSE_PLUGIN_DIR == {
        "fo4":       "F4SE",
        "skyrimse":  "SKSE",
        "starfield": "SFSE",
        "fnv":       "NVSE",
        "fo3":       "FOSE",
    }


def test_xse_plugin_dir_for_returns_extender_name():
    assert xse_plugin_dir_for("fo4") == "F4SE"
    assert xse_plugin_dir_for("skyrimse") == "SKSE"
    assert xse_plugin_dir_for("starfield") == "SFSE"
    assert xse_plugin_dir_for("fnv") == "NVSE"
    assert xse_plugin_dir_for("fo3") == "FOSE"


def test_xse_plugin_dir_for_unknown_game_raises():
    import pytest
    with pytest.raises(KeyError):
        xse_plugin_dir_for("oblivion")


def test_staging_path_for_each_game():
    """Sanity check: mods/<name>/<XSE>/Plugins/<name>.dll layout."""
    name = "B21_Demo"
    for game, ext_dir in XSE_PLUGIN_DIR.items():
        staging = Path("mods") / name / ext_dir / "Plugins" / f"{name}.dll"
        assert staging.parts == ("mods", name, ext_dir, "Plugins", f"{name}.dll")
