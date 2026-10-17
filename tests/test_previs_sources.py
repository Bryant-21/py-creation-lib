from __future__ import annotations

import struct
from pathlib import Path

import pytest

from creation_lib.previs.sources import from_mo2_profile, from_plugin, read_masters


def _plugin(path: Path, masters=()) -> Path:
    fields = b"HEDR" + struct.pack("<H", 12) + bytes(12)
    for master in masters:
        name = master.encode() + b"\0"
        fields += b"MAST" + struct.pack("<H", len(name)) + name + b"DATA" + struct.pack("<H", 8) + bytes(8)
    header = b"TES4" + struct.pack("<IIIII", len(fields), 0, 0, 0, 131)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + fields)
    return path


def _archive(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"")
    return path


@pytest.fixture
def game(tmp_path):
    root = tmp_path / "Fallout 4"
    data = root / "Data"
    _plugin(data / "Fallout4.esm")
    _plugin(data / "DLCRobot.esm", ["Fallout4.esm"])
    _archive(data / "Fallout4 - Meshes.ba2")
    _archive(data / "Fallout4 - Textures1.ba2")
    (root / "Fallout4.ccc").write_text("ccTest.esl\nccMissing.esl\n")
    _plugin(data / "ccTest.esl", ["Fallout4.esm"])
    return root


def test_reads_master_list(game):
    assert read_masters(game / "Data" / "DLCRobot.esm") == ["Fallout4.esm"]


def test_mo2_profile_orders_officials_then_plugins_txt_and_mods_by_priority(game, tmp_path):
    instance = tmp_path / "MO2"
    profile = instance / "profiles" / "Default"
    profile.mkdir(parents=True)
    (profile / "modlist.txt").write_text("# generated\n+High\n-Disabled\n+Low\n*Unmanaged: DLCRobot\n")
    (profile / "plugins.txt").write_text("# generated\n*Low.esp\nOff.esp\n*High.esp\n")
    _plugin(instance / "mods" / "Low" / "Low.esp", ["Fallout4.esm"])
    _plugin(instance / "mods" / "High" / "High.esp", ["Fallout4.esm"])
    _plugin(instance / "mods" / "Disabled" / "Low.esp", ["Fallout4.esm"])
    _archive(instance / "mods" / "Low" / "Low - Main.ba2")
    _archive(instance / "mods" / "High" / "Low - Main.ba2")

    spec = from_mo2_profile(profile, game)

    assert [p.name for p in spec.plugins] == ["Fallout4.esm", "DLCRobot.esm", "ccTest.esl", "Low.esp", "High.esp"]
    assert spec.plugins[3].parent.name == "Low"
    assert spec.base_plugins == ["Fallout4.esm", "DLCRobot.esm", "ccTest.esl"]
    archives = [Path(s["path"]) for s in spec.data_sources if s["kind"] == "archive"]
    assert [a.name for a in archives] == ["Fallout4 - Meshes.ba2", "Low - Main.ba2"]
    assert archives[1].parent.name == "High", "the higher-priority mod's copy of an archive wins"
    loose = [Path(s["path"]).name for s in spec.data_sources if s["kind"] == "loose"]
    assert loose == ["Data", "Low", "High"] + (["overwrite"] if (instance / "overwrite").is_dir() else [])


def test_single_plugin_loads_only_its_masters(game, tmp_path):
    mod = _plugin(tmp_path / "work" / "Mod.esp", ["DLCRobot.esm"])
    spec = from_plugin(mod, game)
    assert [p.name for p in spec.plugins] == ["Fallout4.esm", "DLCRobot.esm", "Mod.esp"]
    assert spec.base_plugins == ["Fallout4.esm", "DLCRobot.esm"]


def test_missing_master_is_reported(game, tmp_path):
    mod = _plugin(tmp_path / "work" / "Mod.esp", ["Absent.esm"])
    with pytest.raises(FileNotFoundError, match="Absent.esm"):
        from_plugin(mod, game)


def test_a_previs_framework_counts_as_the_games_own(game, tmp_path):
    instance = tmp_path / "MO2"
    profile = instance / "profiles" / "Default"
    profile.mkdir(parents=True)
    (profile / "modlist.txt").write_text("+Framework\n+Mod\n")
    (profile / "plugins.txt").write_text("*PPF.esm\n*Mod.esp\n")
    _plugin(instance / "mods" / "Framework" / "PPF.esm", ["Fallout4.esm"])
    _plugin(instance / "mods" / "Mod" / "Mod.esp", ["Fallout4.esm"])

    spec = from_mo2_profile(profile, game)

    assert "PPF.esm" in spec.base_plugins
    assert "Mod.esp" not in spec.base_plugins


def test_a_plugin_is_loaded_against_a_framework_it_does_not_master(game, tmp_path):
    framework = _plugin(tmp_path / "Framework" / "PPF.esm", ["Fallout4.esm", "DLCRobot.esm"])
    _archive(tmp_path / "Framework" / "PPF - Main.ba2")
    mod = _plugin(tmp_path / "Mod" / "Mod.esp", ["Fallout4.esm"])

    spec = from_plugin(mod, game, against=[framework])

    assert [p.name for p in spec.plugins] == ["Fallout4.esm", "DLCRobot.esm", "PPF.esm", "Mod.esp"]
    assert spec.plugins[2] == framework
    assert "PPF - Main.ba2" in [Path(s["path"]).name for s in spec.data_sources if s["kind"] == "archive"]
    assert spec.base_plugins == ["Fallout4.esm", "DLCRobot.esm", "PPF.esm"]
    assert spec.label == "Mod.esp against PPF.esm"
