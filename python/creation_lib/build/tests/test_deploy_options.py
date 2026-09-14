from __future__ import annotations

import json

import pytest

from creation_lib.build.deployer import XSE_PLUGIN_DIR, deploy_mod
from creation_lib.build.loose_deploy import deploy_loose_assets, undeploy_loose_assets
from creation_lib.esp import Plugin, export_json, export_yaml


@pytest.mark.parametrize("game,extender", XSE_PLUGIN_DIR.items())
@pytest.mark.parametrize("mode", ["xse", "combined", "loose"])
def test_preserve_xse_inis_copies_new_files_and_updates_dlls(tmp_path, game, extender, mode):
    mod = tmp_path / "mods" / "B21_Options"
    staging = mod / extender / "Plugins"
    staging.mkdir(parents=True)
    (mod / "B21_Options.esp").write_bytes(b"plugin")
    for name in ("B21_Test.INI", "B21_New.ini", "B21_Test.dll"):
        (staging / name).write_text("new", encoding="utf-8")
    target = tmp_path / "MO2" / "B21_Options"
    installed = target / extender / "Plugins"
    installed.mkdir(parents=True)
    for name in ("B21_Test.INI", "B21_Test.dll"):
        (installed / name).write_text("user setting", encoding="utf-8")
    kwargs = dict(game=game, game_data_dir=tmp_path / "Game" / "Data", deploy_data_dir=target,
                  project_root=tmp_path, skip_build=True, skip_papyrus_compile=True)
    deploy = deploy_loose_assets if mode == "loose" else deploy_mod
    if mode != "loose":
        kwargs.update(skip_pack=True, no_esp=mode == "xse")
    result = deploy("B21_Options", preserve_xse_inis=True, **kwargs)
    assert result.preserved_xse_inis == [f"{extender}/Plugins/B21_Test.INI"]
    assert (installed / "B21_Test.INI").read_text() == "user setting"
    assert (installed / "B21_New.ini").read_text() == "new"
    assert (installed / "B21_Test.dll").read_text() == "new"
    assert not kwargs["game_data_dir"].exists()
    if mode == "loose":
        undeploy_loose_assets("B21_Options", game_data_dir=target, project_root=tmp_path)
        assert (installed / "B21_Test.INI").read_text() == "user setting"
    deploy("B21_Options", **kwargs)
    assert (installed / "B21_Test.INI").read_text() == "new"


def test_loose_preserves_xse_inis_from_data_tree_only(tmp_path):
    mod = tmp_path / "mods" / "B21_Options"
    target = tmp_path / "Data"
    for root in (mod / "data", target):
        for rel in ("F4SE/Plugins/B21_Test.ini", "MCM/Config/B21_Test.ini"):
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("old" if root == target else "new", encoding="utf-8")
    (mod / "B21_Options.esp").write_bytes(b"plugin")
    deploy_loose_assets("B21_Options", game="fo4", game_data_dir=target, project_root=tmp_path,
                        skip_build=True, skip_papyrus_compile=True, preserve_xse_inis=True)
    assert (target / "F4SE/Plugins/B21_Test.ini").read_text() == "old"
    assert (target / "MCM/Config/B21_Test.ini").read_text() == "new"


@pytest.mark.parametrize("suffix", ["json", "yaml"])
@pytest.mark.parametrize("loose", [False, True])
@pytest.mark.parametrize("layout", ["root", "yaml"])
def test_deploy_builds_whole_plugin_and_keeps_identity(tmp_path, suffix, loose, layout):
    mod = tmp_path / "mods" / "B21_Whole"
    mod.mkdir(parents=True)
    with Plugin.new("B21_Actual.esm", game="fo4", masters=[]) as plugin:
        record = plugin.new_record("MISC", form_id=0x800)
        record.editor_id = "B21_WholeRecord"
        plugin.add_record(record)
        text = export_json(plugin) if suffix == "json" else export_yaml(plugin)
    source_dir = mod if layout == "root" else mod / "yaml"
    source_dir.mkdir(exist_ok=True)
    (source_dir / f"plugin.{suffix}").write_text(text, encoding="utf-8")
    target = tmp_path / "Data"
    kwargs = dict(game="fo4", game_data_dir=target, project_root=tmp_path, skip_papyrus_compile=True)
    if loose:
        result = deploy_loose_assets("B21_Whole", **kwargs)
        assert result.plugin == "B21_Actual.esm"
        assert json.loads((mod / ".loose_manifest.json").read_text())["files"][0]["rel"] == "B21_Actual.esm"
    else:
        result = deploy_mod("B21_Whole", esp_only=True, **kwargs)
        assert result.plugin_deployed == "B21_Actual.esm"
    with Plugin.load(target / "B21_Actual.esm", game="fo4") as plugin:
        assert plugin.get_record_by_form_id(0x800).editor_id == "B21_WholeRecord"
    assert not (target / "B21_Whole.esp").exists()


@pytest.mark.parametrize("skip_pack", [False, True])
def test_deploy_renamed_binary_with_matching_archives(tmp_path, skip_pack):
    mod = tmp_path / "mods" / "B21_Package"
    mod.mkdir(parents=True)
    (mod / "B21_Actual.esm").write_bytes(b"plugin")
    (mod / "B21_Actual - Main.ba2").write_bytes(b"archive")
    (mod / "data" / "Scripts").mkdir(parents=True)
    (mod / "data" / "Scripts" / "B21_Test.pex").write_bytes(b"script")
    target = tmp_path / "Data"
    result = deploy_mod("B21_Package", game="fo4", game_data_dir=target, project_root=tmp_path,
                        skip_build=True, skip_pack=skip_pack, skip_papyrus_compile=True)
    assert result.plugin_deployed == "B21_Actual.esm"
    assert result.archives_deployed == ["B21_Actual - Main.ba2"]
    assert (target / "B21_Actual - Main.ba2").read_bytes() == (mod / "B21_Actual - Main.ba2").read_bytes()


def test_deploy_source_disambiguates_and_rejects_invalid_whole_plugin(tmp_path):
    mod = tmp_path / "mods" / "B21_Package"
    mod.mkdir(parents=True)
    for name in ("B21_A.esp", "B21_B.esp"):
        (mod / name).write_bytes(name.encode())
    target = tmp_path / "Data"
    kwargs = dict(game="fo4", game_data_dir=target, project_root=tmp_path, esp_only=True)
    with pytest.raises(ValueError, match="Multiple plugin binaries"):
        deploy_mod("B21_Package", **kwargs)
    assert not target.exists()
    deploy_mod("B21_Package", source="B21_B.esp", **kwargs)
    assert (target / "B21_B.esp").read_bytes() == b"B21_B.esp"
    (mod / "broken.json").write_text('{"plugin":"../outside.esp"}', encoding="utf-8")
    with pytest.raises(ValueError, match="plain"):
        deploy_mod("B21_Package", source="broken.json", **kwargs)
    assert (mod / "B21_B.esp").read_bytes() == b"B21_B.esp"
