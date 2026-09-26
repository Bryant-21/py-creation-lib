from pathlib import Path
from unittest.mock import patch

import pytest

from creation_lib.build.deploy_plan import plan_deploy
from creation_lib.build.deployer import deploy_mod, undeploy_mod


@pytest.fixture
def runtime_mod(tmp_path):
    mod = tmp_path / "mods" / "B21_FullScreenMap"
    files = {
        "F4SE/Plugins/B21_FullScreenMap.dll": b"dll",
        "Scripts/B21_FullScreenMap.pex": b"old interface",
        "Scripts/B21/Helper.PEX": b"helper",
        "Scripts/Source/User/B21_FullScreenMap.psc": b"source",
        "Scripts/Source/User/Import.pex": b"import only",
        "data/Scripts/B21_FullScreenMap.pex": b"current interface",
        "data/Scripts/B21/Launcher.pex": b"launcher",
        "data/Scripts/Source/Import.pex": b"import only",
        "data/Scripts/Readme.txt": b"readme",
    }
    for relative, content in files.items():
        path = mod / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
    return mod


def test_no_esp_installs_only_runtime_scripts_and_matches_dry_run(runtime_mod, tmp_path):
    target = tmp_path / "MO2" / "B21_FullScreenMap"
    plan = plan_deploy(runtime_mod, target, game="fo4", no_esp=True, skip_papyrus_compile=True)
    assert not target.exists()
    result = deploy_mod(
        runtime_mod.name, game="fo4", game_data_dir=tmp_path / "Game" / "Data",
        deploy_data_dir=target, project_root=tmp_path, no_esp=True, skip_papyrus_compile=True,
    )
    installed = {p.relative_to(target).as_posix(): p.read_bytes() for p in target.rglob("*") if p.is_file()}
    assert installed == {
        "F4SE/Plugins/B21_FullScreenMap.dll": b"dll",
        "Scripts/B21_FullScreenMap.pex": b"current interface",
        "Scripts/B21/Helper.PEX": b"helper",
        "Scripts/B21/Launcher.pex": b"launcher",
    }
    assert result.loose_files_deployed == len(installed)
    assert plan["file_inventory_complete"] is True
    assert {Path(op["destination"]).relative_to(target).as_posix() for op in plan["operations"]} == set(installed)


def test_no_esp_compiles_before_copy_using_game_imports(runtime_mod, tmp_path):
    game_data = tmp_path / "Game" / "Data"
    target = tmp_path / "MO2" / "B21_FullScreenMap"

    def compile_scripts(mod, game, imports, **kwargs):
        assert (mod, game, imports) == (runtime_mod, "fo4", game_data)
        assert not target.exists()
        (mod / "data/Scripts/B21_FullScreenMap.pex").write_bytes(b"rebuilt")
        return 1

    plan = plan_deploy(runtime_mod, target, game="fo4", no_esp=True)
    assert plan["pending_build_steps"] == [{
        "action": "compile_papyrus", "sources": 1,
        "output_directory": str(runtime_mod / "data/Scripts"),
    }]
    assert plan["file_inventory_complete"] is False
    with patch("creation_lib.build.deployer.compile_papyrus", side_effect=compile_scripts) as compiler:
        deploy_mod(runtime_mod.name, game="fo4", game_data_dir=game_data,
                   deploy_data_dir=target, project_root=tmp_path, no_esp=True)
    compiler.assert_called_once()
    assert (target / "Scripts/B21_FullScreenMap.pex").read_bytes() == b"rebuilt"
    assert not game_data.exists()


def test_no_esp_compile_failure_and_skip_compile(runtime_mod, tmp_path):
    target = tmp_path / "Game" / "Data"
    with patch("creation_lib.build.deployer.compile_papyrus", side_effect=RuntimeError("compile failed")):
        with pytest.raises(RuntimeError, match="compile failed"):
            deploy_mod(runtime_mod.name, game="fo4", game_data_dir=target,
                       project_root=tmp_path, no_esp=True)
    assert not target.exists()

    # skip_papyrus_compile bypasses compilation entirely and ships the pex as-is.
    with patch("creation_lib.build.deployer.compile_papyrus") as compiler:
        deploy_mod(runtime_mod.name, game="fo4", game_data_dir=target,
                   project_root=tmp_path, no_esp=True, skip_papyrus_compile=True)
    compiler.assert_not_called()
    assert (target / "Scripts/B21_FullScreenMap.pex").read_bytes() == b"current interface"


def test_no_esp_undeploy_removes_only_shipped_scripts(runtime_mod, tmp_path):
    target = tmp_path / "Game" / "Data"
    for relative in ("Scripts/F4SE.pex", "Scripts/B21/OtherMod.pex", "Scripts/Source/User/Import.pex"):
        path = target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"keep")
    deploy_mod(runtime_mod.name, game="fo4", game_data_dir=target,
               project_root=tmp_path, no_esp=True, skip_papyrus_compile=True)
    before = {p for p in target.rglob("*") if p.is_file()}
    preview = undeploy_mod(runtime_mod.name, game="fo4", game_data_dir=target,
                          project_root=tmp_path, no_esp=True, dry_run=True)
    assert {p for p in target.rglob("*") if p.is_file()} == before
    removed = undeploy_mod(runtime_mod.name, game="fo4", game_data_dir=target,
                          project_root=tmp_path, no_esp=True)
    assert set(removed) == set(preview)
    assert {Path(p).as_posix() for p in removed} == {
        "F4SE/Plugins/B21_FullScreenMap.dll", "Scripts/B21_FullScreenMap.pex",
        "Scripts/B21/Helper.PEX", "Scripts/B21/Launcher.pex",
    }
    assert {p.relative_to(target).as_posix() for p in target.rglob("*") if p.is_file()} == {
        "Scripts/F4SE.pex", "Scripts/B21/OtherMod.pex", "Scripts/Source/User/Import.pex",
    }
