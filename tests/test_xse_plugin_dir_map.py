"""Unit tests for the XSE_PLUGIN_DIR map and per-game path construction."""
import pytest

from creation_lib.build.deployer import xse_plugin_dir_for


def test_xse_plugin_dir_for_returns_extender_name():
    assert xse_plugin_dir_for("fo4") == "F4SE"
    assert xse_plugin_dir_for("skyrimse") == "SKSE"
    assert xse_plugin_dir_for("starfield") == "SFSE"
    assert xse_plugin_dir_for("fnv") == "NVSE"
    assert xse_plugin_dir_for("fo3") == "FOSE"


def test_xse_plugin_dir_for_unknown_game_raises():
    with pytest.raises(KeyError):
        xse_plugin_dir_for("oblivion")
