"""Tests for authoring.new_plugin_file — creating an empty plugin binary."""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.esp.authoring import new_plugin_file
from creation_lib.esp.editor import header_flags
from creation_lib.esp.plugin import Plugin


def _flags(path: Path) -> int:
    plugin = Plugin.load(path)
    try:
        return header_flags.get_flags(plugin._rust_handle)
    finally:
        plugin.close()


def _masters(path: Path) -> list[str]:
    plugin = Plugin.load(path)
    try:
        return list(plugin.header.masters)
    finally:
        plugin.close()


def test_creates_empty_esp_with_base_master(tmp_path: Path) -> None:
    out = tmp_path / "B21_New.esp"
    summary = new_plugin_file(out, game="fo4", extension="esp")

    assert out.is_file()
    plugin = Plugin.load(out)
    try:
        assert plugin.record_count == 0
    finally:
        plugin.close()
    flags = _flags(out)
    assert not flags & header_flags.FLAG_MASTER
    assert not flags & header_flags.FLAG_LIGHT
    assert _masters(out) == ["Fallout4.esm"]
    assert summary["extension"] == "esp"
    assert summary["masters"] == ["Fallout4.esm"]


def test_esm_extension_auto_sets_master_bit(tmp_path: Path) -> None:
    out = tmp_path / "B21_New.esm"
    new_plugin_file(out, game="fo4", extension="esm")
    assert _flags(out) & header_flags.FLAG_MASTER


def test_esl_extension_auto_sets_light_bit(tmp_path: Path) -> None:
    out = tmp_path / "B21_New.esl"
    new_plugin_file(out, game="fo4", extension="esl")
    assert _flags(out) & header_flags.FLAG_LIGHT


def test_light_override_flags_esp_as_light(tmp_path: Path) -> None:
    out = tmp_path / "B21_Light.esp"
    new_plugin_file(out, game="fo4", extension="esp", set_light=True)
    assert _flags(out) & header_flags.FLAG_LIGHT


def test_master_override_clears_auto_bit_on_esm(tmp_path: Path) -> None:
    out = tmp_path / "B21_NoMaster.esm"
    new_plugin_file(out, game="fo4", extension="esm", set_master=False)
    assert not _flags(out) & header_flags.FLAG_MASTER


def test_localized_and_medium_flags(tmp_path: Path) -> None:
    out = tmp_path / "B21_Flags.esp"
    new_plugin_file(out, game="fo4", extension="esp", set_localized=True, set_medium=True)
    flags = _flags(out)
    assert flags & header_flags.FLAG_LOCALIZED
    assert flags & header_flags.FLAG_MEDIUM


def test_no_base_master_starts_empty(tmp_path: Path) -> None:
    out = tmp_path / "B21_Bare.esp"
    summary = new_plugin_file(out, game="fo4", extension="esp", include_base_master=False)
    assert _masters(out) == []
    assert summary["masters"] == []


def test_extra_masters_appended_and_deduped(tmp_path: Path) -> None:
    out = tmp_path / "B21_Masters.esp"
    summary = new_plugin_file(
        out,
        game="fo4",
        extension="esp",
        masters=["DLCCoast.esm", "fallout4.esm"],
    )
    # Base master seeded first; the case-insensitive duplicate of it is dropped.
    assert summary["masters"] == ["Fallout4.esm", "DLCCoast.esm"]
    assert _masters(out) == ["Fallout4.esm", "DLCCoast.esm"]


def test_refuses_to_overwrite_without_force(tmp_path: Path) -> None:
    out = tmp_path / "B21_Exists.esp"
    new_plugin_file(out, game="fo4", extension="esp")
    with pytest.raises(FileExistsError):
        new_plugin_file(out, game="fo4", extension="esp")


def test_force_overwrites_existing(tmp_path: Path) -> None:
    out = tmp_path / "B21_Force.esp"
    new_plugin_file(out, game="fo4", extension="esp")
    # Should not raise.
    new_plugin_file(out, game="fo4", extension="esp", force=True)
    assert out.is_file()
