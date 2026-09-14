from __future__ import annotations

import errno
import shutil
from pathlib import Path
from unittest.mock import patch

import pytest

from creation_lib.build import deployer
from creation_lib.build.deployer import deploy_mod, undeploy_mod


def test_copy2_fast_uses_windows_native_copy_for_large_files(tmp_path: Path, monkeypatch):
    src = tmp_path / "source.ba2"
    dest = tmp_path / "dest.ba2"
    src.write_bytes(b"archive")
    calls: list[tuple[Path, Path, int]] = []

    def _fake_copyfileex(source: Path, target: Path, flags: int) -> None:
        calls.append((source, target, flags))
        shutil.copyfile(source, target)

    monkeypatch.setattr(deployer, "_use_windows_fast_copy", lambda source, file_size: True)
    monkeypatch.setattr(deployer, "_windows_copyfileex", _fake_copyfileex)

    deployer._copy2_fast(src, dest)

    assert calls == [(src, dest, deployer._COPY_FILE_NO_BUFFERING)]
    assert dest.read_bytes() == b"archive"


def test_copy2_fast_falls_back_to_copy2_when_native_copy_fails(tmp_path: Path, monkeypatch):
    src = tmp_path / "source.ba2"
    dest = tmp_path / "dest.ba2"
    src.write_bytes(b"archive")
    attempts = 0

    def _fail_copyfileex(source: Path, target: Path, flags: int) -> None:
        nonlocal attempts
        attempts += 1
        raise OSError("native copy unavailable")

    monkeypatch.setattr(deployer, "_use_windows_fast_copy", lambda source, file_size: True)
    monkeypatch.setattr(deployer, "_windows_copyfileex", _fail_copyfileex)

    deployer._copy2_fast(src, dest)

    assert attempts == 1
    assert dest.read_bytes() == b"archive"


def test_deploy_archives_use_fast_copy_helper(tmp_path: Path, monkeypatch):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (mod_dir / f"{mod_name} - Main.ba2").write_bytes(b"main")
    copied: list[str] = []

    def _record_copy(source: Path, target: Path) -> None:
        copied.append(target.name)
        shutil.copy2(source, target)

    monkeypatch.setattr(deployer, "_copy2_fast", _record_copy)

    deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert f"{mod_name}.esp" in copied
    assert f"{mod_name} - Main.ba2" in copied


def test_deploy_forwards_archive_workers_to_pack_mod(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    (mod_dir / "data").mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    pack_calls: list[dict] = []

    with patch(
        "creation_lib.build.deployer.pack_mod",
        side_effect=lambda *args, **kwargs: pack_calls.append(kwargs),
    ):
        deploy_mod(
            mod_name,
            game="fo4",
            game_data_dir=game_data_dir,
            skip_build=True,
            ps=True,
            ps_max_res=4096,
            ps_effects_max_res=2048,
            archive_workers=7,
            project_root=tmp_path,
            resource_dir=tmp_path / "resource",
        )

    assert pack_calls[0]["archive_workers"] == 7
    assert pack_calls[0]["ps"] is True
    assert pack_calls[0]["ps_max_res"] == 4096
    assert pack_calls[0]["ps_effects_max_res"] == 2048


def test_deploy_can_pack_archives_directly_to_target(tmp_path: Path, monkeypatch):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    (mod_dir / "data" / "Meshes").mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (mod_dir / f"{mod_name} - Main.ba2").write_bytes(b"old local")
    (game_data_dir / f"{mod_name} - Main.ba2").write_bytes(b"old main")
    (game_data_dir / f"{mod_name} - Textures.ba2").write_bytes(b"old textures")
    manual_archive = game_data_dir / f"{mod_name} - HiRes.ba2"
    manual_archive.write_bytes(b"manual")
    observed_output_dirs: list[Path] = []

    def fake_pack_mod(*args, **kwargs):
        output_dir = Path(kwargs["archive_output_dir"])
        observed_output_dirs.append(output_dir)
        assert not (output_dir / f"{mod_name} - Main.ba2").exists()
        assert not (output_dir / f"{mod_name} - Textures.ba2").exists()
        assert manual_archive.read_bytes() == b"manual"
        (output_dir / f"{mod_name} - Main.ba2").write_bytes(b"new main")
        (output_dir / f"{mod_name} - Textures.ba2").write_bytes(b"new textures")

    monkeypatch.setattr(deployer, "pack_mod", fake_pack_mod)

    def fail_transfer(*args, **kwargs):
        raise AssertionError("direct-packed archives must not be transferred")

    monkeypatch.setattr(deployer, "_transfer_archive", fail_transfer)

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_papyrus_compile=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
        archive_transfer_mode="move",
        pack_archives_to_deploy_target=True,
    )

    assert observed_output_dirs == [game_data_dir]
    assert result.archives_deployed == [
        f"{mod_name} - Main.ba2",
        f"{mod_name} - Textures.ba2",
    ]
    assert not (mod_dir / f"{mod_name} - Main.ba2").exists()
    assert (game_data_dir / f"{mod_name} - Main.ba2").read_bytes() == b"new main"
    assert (game_data_dir / f"{mod_name} - Textures.ba2").read_bytes() == b"new textures"
    assert manual_archive.read_bytes() == b"manual"


def test_deploy_removes_stale_generated_archives_but_keeps_manual_archives(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (mod_dir / f"{mod_name} - Main.ba2").write_bytes(b"main")
    (game_data_dir / f"{mod_name} - Meshes.ba2").write_bytes(b"stale")
    (game_data_dir / f"{mod_name} - HiRes.ba2").write_bytes(b"manual")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert result.archives_deployed == [f"{mod_name} - Main.ba2"]
    assert (game_data_dir / f"{mod_name} - Main.ba2").read_bytes() == b"main"
    assert not (game_data_dir / f"{mod_name} - Meshes.ba2").exists()
    assert (game_data_dir / f"{mod_name} - HiRes.ba2").read_bytes() == b"manual"


def test_deploy_can_copy_to_separate_virtual_data_dir(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    deploy_data_dir = tmp_path / "ModOrganizer" / "mods" / mod_name
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    deploy_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (mod_dir / f"{mod_name} - Main.ba2").write_bytes(b"main")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        deploy_data_dir=deploy_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert result.plugin_deployed == f"{mod_name}.esp"
    assert (deploy_data_dir / f"{mod_name}.esp").read_bytes() == b"esp"
    assert (deploy_data_dir / f"{mod_name} - Main.ba2").read_bytes() == b"main"
    assert not (game_data_dir / f"{mod_name}.esp").exists()
    assert not (game_data_dir / f"{mod_name} - Main.ba2").exists()


def test_deploy_can_move_archives_to_target(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    source_archive = mod_dir / f"{mod_name} - Main.ba2"
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    source_archive.write_bytes(b"main")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
        archive_transfer_mode="move",
    )

    assert result.archives_deployed == [f"{mod_name} - Main.ba2"]
    assert not source_archive.exists()
    assert (game_data_dir / f"{mod_name} - Main.ba2").read_bytes() == b"main"
    assert (mod_dir / f"{mod_name}.esp").read_bytes() == b"esp"


def test_cross_volume_archive_move_uses_fast_copy(tmp_path: Path, monkeypatch):
    src = tmp_path / "source.ba2"
    dest = tmp_path / "dest.ba2"
    src.write_bytes(b"new archive")
    dest.write_bytes(b"old archive")
    copied: list[tuple[Path, Path]] = []
    original_replace = Path.replace

    def _cross_volume_replace(path: Path, target: Path) -> Path:
        if path == src:
            raise OSError(errno.EXDEV, "cross-device link")
        return original_replace(path, target)

    def _record_copy(source: Path, target: Path) -> None:
        copied.append((source, target))
        shutil.copy2(source, target)

    monkeypatch.setattr(Path, "replace", _cross_volume_replace)
    monkeypatch.setattr(deployer, "_copy2_fast", _record_copy)

    deployer._move_archive(src, dest)

    assert copied == [(src, dest)]
    assert not src.exists()
    assert dest.read_bytes() == b"new archive"
    assert not dest.with_name("dest.ba2.old").exists()


def test_cross_volume_archive_move_restores_previous_destination_on_copy_failure(
    tmp_path: Path,
    monkeypatch,
):
    src = tmp_path / "source.ba2"
    dest = tmp_path / "dest.ba2"
    src.write_bytes(b"new archive")
    dest.write_bytes(b"old archive")
    original_replace = Path.replace

    def _cross_volume_replace(path: Path, target: Path) -> Path:
        if path == src:
            raise OSError(errno.EXDEV, "cross-device link")
        return original_replace(path, target)

    def _fail_copy(source: Path, target: Path) -> None:
        target.write_bytes(b"partial")
        raise OSError("copy failed")

    monkeypatch.setattr(Path, "replace", _cross_volume_replace)
    monkeypatch.setattr(deployer, "_copy2_fast", _fail_copy)

    with pytest.raises(OSError, match="copy failed"):
        deployer._move_archive(src, dest)

    assert src.read_bytes() == b"new archive"
    assert dest.read_bytes() == b"old archive"
    assert not dest.with_name("dest.ba2.old").exists()


def test_deploy_can_skip_archive_loop_for_direct_deployed_archives(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    direct_archive = game_data_dir / f"{mod_name} - Main.ba2"
    direct_archive.write_bytes(b"direct")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
        deploy_archives=False,
    )

    assert result.archives_deployed == []
    assert direct_archive.read_bytes() == b"direct"


def test_deploy_can_skip_papyrus_compile(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    source_dir = mod_dir / "Scripts" / "Source" / "User"
    source_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (source_dir / "Broken.psc").write_text("Scriptname Broken\n", encoding="utf-8")

    def _compile_papyrus(*args, **kwargs):
        raise AssertionError("compile_papyrus should not run")

    with patch(
        "creation_lib.build.deployer.compile_papyrus",
        side_effect=_compile_papyrus,
    ):
        result = deploy_mod(
            mod_name,
            game="fo4",
            game_data_dir=game_data_dir,
            skip_build=True,
            skip_pack=True,
            skip_papyrus_compile=True,
            project_root=tmp_path,
            resource_dir=tmp_path / "resource",
        )

    assert result.plugin_deployed == f"{mod_name}.esp"
    assert (game_data_dir / f"{mod_name}.esp").read_bytes() == b"esp"


def test_compile_papyrus_uses_native_compiler(tmp_path: Path, monkeypatch):
    from creation_lib.build.deployer import compile_papyrus

    mod_dir = tmp_path / "mods" / "B21_Test"
    game_data_dir = tmp_path / "Game" / "Data"
    source_dir = mod_dir / "Scripts" / "Source" / "User"
    scripts_base = game_data_dir / "Scripts" / "Source" / "Base"
    (source_dir / "B21").mkdir(parents=True)
    scripts_base.mkdir(parents=True)
    (scripts_base / "Institute_Papyrus_Flags.flg").write_text("", encoding="utf-8")
    (scripts_base / "ScriptObject.psc").write_text(
        "Scriptname ScriptObject\n", encoding="utf-8"
    )
    (source_dir / "B21" / "Foo.psc").write_text("Scriptname B21:Foo\n", encoding="utf-8")
    (source_dir / "B21" / "Bar.psc").write_text("Scriptname B21:Bar\n", encoding="utf-8")
    (source_dir / "Baz.psc").write_text("Scriptname Baz\n", encoding="utf-8")

    calls: list[dict] = []

    def _fake_compile_psc(source, *, imports, game, flags, source_path=None):
        from creation_lib.pex.native_runtime import CompileResult

        calls.append(
            {
                "source": source,
                "imports": imports,
                "game": game,
                "flags": flags,
                "source_path": source_path,
            }
        )
        return CompileResult(ok=True, pex_bytes=b"pex")

    monkeypatch.setattr("creation_lib.pex.native_runtime.compile_psc", _fake_compile_psc)

    count = compile_papyrus(mod_dir, "fo4", game_data_dir)

    assert count == 3
    assert len(calls) == 3
    assert calls[0]["game"] == "fo4"
    assert calls[0]["imports"] == [str(source_dir), str(scripts_base)]
    assert calls[0]["flags"] == str(scripts_base / "Institute_Papyrus_Flags.flg")
    assert (mod_dir / "data" / "Scripts" / "B21" / "Foo.pex").read_bytes() == b"pex"
    assert (mod_dir / "data" / "Scripts" / "B21" / "Bar.pex").read_bytes() == b"pex"
    assert (mod_dir / "data" / "Scripts" / "Baz.pex").read_bytes() == b"pex"


def test_no_esp_deploy_copies_xse_tree_and_fo4cs_data_root(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    plugin_dir = mod_dir / "F4SE" / "Plugins"
    lut_dir = mod_dir / "FO4CS" / "LUTs"
    mcm_dir = mod_dir / "MCM" / "Config" / mod_name
    materials_dir = mod_dir / "Materials" / "Weapons" / "M2"
    plugin_dir.mkdir(parents=True)
    lut_dir.mkdir(parents=True)
    mcm_dir.mkdir(parents=True)
    materials_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (plugin_dir / f"{mod_name}.dll").write_bytes(b"dll")
    (lut_dir / "neutral_32.dds").write_bytes(b"lut")
    (mcm_dir / "config.json").write_text("{}", encoding="utf-8")
    (materials_dir / "M2Barrel.bgsm").write_bytes(b"bgsm")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        no_esp=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert result.loose_files_deployed == 4
    assert (game_data_dir / "F4SE" / "Plugins" / f"{mod_name}.dll").read_bytes() == b"dll"
    assert (game_data_dir / "FO4CS" / "LUTs" / "neutral_32.dds").read_bytes() == b"lut"
    assert (game_data_dir / "MCM" / "Config" / mod_name / "config.json").read_text(encoding="utf-8") == "{}"
    # A renderer mod ships its .bgsm/.bgem beside the DLL; without this the only
    # route was writing to the game Data dir by hand.
    assert (
        game_data_dir / "Materials" / "Weapons" / "M2" / "M2Barrel.bgsm"
    ).read_bytes() == b"bgsm"


def test_no_esp_undeploy_removes_xse_tree_and_fo4cs_data_root(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    plugin_dir = mod_dir / "F4SE" / "Plugins"
    lut_dir = mod_dir / "FO4CS" / "LUTs"
    deployed_plugin_dir = game_data_dir / "F4SE" / "Plugins"
    deployed_lut_dir = game_data_dir / "FO4CS" / "LUTs"
    mcm_dir = mod_dir / "MCM" / "Config" / mod_name
    deployed_mcm_dir = game_data_dir / "MCM" / "Config" / mod_name
    plugin_dir.mkdir(parents=True)
    lut_dir.mkdir(parents=True)
    deployed_plugin_dir.mkdir(parents=True)
    deployed_lut_dir.mkdir(parents=True)
    mcm_dir.mkdir(parents=True)
    deployed_mcm_dir.mkdir(parents=True)
    (plugin_dir / f"{mod_name}.dll").write_bytes(b"dll")
    (lut_dir / "neutral_32.dds").write_bytes(b"lut")
    (deployed_plugin_dir / f"{mod_name}.dll").write_bytes(b"dll")
    (deployed_lut_dir / "neutral_32.dds").write_bytes(b"lut")
    (mcm_dir / "config.json").write_text("{}", encoding="utf-8")
    (deployed_mcm_dir / "config.json").write_text("{}", encoding="utf-8")
    (game_data_dir / "FO4CS" / "other_mod_file.txt").write_bytes(b"keep")

    removed = undeploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        no_esp=True,
        project_root=tmp_path,
    )
    removed = [path.replace("\\", "/") for path in removed]

    assert "F4SE/Plugins/B21_Test.dll" in removed
    assert "FO4CS/LUTs/neutral_32.dds" in removed
    assert f"MCM/Config/{mod_name}/config.json" in removed
    assert not (deployed_plugin_dir / f"{mod_name}.dll").exists()
    assert not (deployed_lut_dir / "neutral_32.dds").exists()
    assert not (deployed_mcm_dir / "config.json").exists()
    assert (game_data_dir / "FO4CS" / "other_mod_file.txt").read_bytes() == b"keep"


def test_deploy_does_not_copy_root_strings_for_archive_deploy(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    strings_dir = mod_dir / "Strings"
    deployed_strings_dir = game_data_dir / "Strings"
    strings_dir.mkdir(parents=True)
    deployed_strings_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (strings_dir / f"{mod_name}_en.STRINGS").write_bytes(b"strings")
    (strings_dir / f".{mod_name}.ckfix.tmp_en.STRINGS").write_bytes(b"temp")
    (deployed_strings_dir / f"{mod_name}_en.STRINGS").write_bytes(b"stale")
    (deployed_strings_dir / f"{mod_name}.esm_en.dlstrings").write_bytes(b"stale")
    (deployed_strings_dir / f"{mod_name}Other_en.STRINGS").write_bytes(b"other")

    result = deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert result.strings_deployed == 0
    assert not (game_data_dir / "Strings" / f"{mod_name}_en.STRINGS").exists()
    assert not (game_data_dir / "Strings" / f"{mod_name}.esm_en.dlstrings").exists()
    assert not (game_data_dir / "Strings" / f".{mod_name}.ckfix.tmp_en.STRINGS").exists()
    assert (game_data_dir / "Strings" / f"{mod_name}Other_en.STRINGS").read_bytes() == b"other"


def test_deploy_copies_terrain_btd4_sidecar(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    terrain_dir = mod_dir / "Terrain"
    terrain_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (terrain_dir / "Appalachia.btd4").write_bytes(b"btd4")

    deploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_pack=True,
        project_root=tmp_path,
        resource_dir=tmp_path / "resource",
    )

    assert (game_data_dir / "Terrain" / "Appalachia.btd4").read_bytes() == b"btd4"


def test_undeploy_removes_terrain_btd4_sidecar(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    terrain_dir = mod_dir / "Terrain"
    terrain_dir.mkdir(parents=True)
    deployed_terrain_dir = game_data_dir / "Terrain"
    deployed_terrain_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (terrain_dir / "Appalachia.btd4").write_bytes(b"btd4")
    (deployed_terrain_dir / "Appalachia.btd4").write_bytes(b"btd4")
    (deployed_terrain_dir / "Vanilla.btd").write_bytes(b"keep")

    removed = undeploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        project_root=tmp_path,
    )

    assert "Terrain/Appalachia.btd4" in removed
    assert not (deployed_terrain_dir / "Appalachia.btd4").exists()
    # A shared Data/Terrain/ holding other files survives the prune.
    assert (deployed_terrain_dir / "Vanilla.btd").read_bytes() == b"keep"


def test_undeploy_removes_generated_archives_but_keeps_manual_archives(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    strings_dir = game_data_dir / "Strings"
    strings_dir.mkdir()
    (game_data_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (game_data_dir / f"{mod_name} - Main.ba2").write_bytes(b"main")
    (game_data_dir / f"{mod_name} - HiRes.ba2").write_bytes(b"manual")
    (strings_dir / f"{mod_name}_en.STRINGS").write_bytes(b"strings")
    (strings_dir / f"{mod_name}.esm_en.ILSTRINGS").write_bytes(b"strings")
    (strings_dir / f"{mod_name}Other_en.STRINGS").write_bytes(b"other")

    removed = undeploy_mod(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        project_root=tmp_path,
    )

    assert f"{mod_name}.esp" in removed
    assert f"{mod_name} - Main.ba2" in removed
    assert f"Strings/{mod_name}_en.STRINGS" in removed
    assert f"Strings/{mod_name}.esm_en.ILSTRINGS" in removed
    assert not (game_data_dir / f"{mod_name}.esp").exists()
    assert not (game_data_dir / f"{mod_name} - Main.ba2").exists()
    assert (game_data_dir / f"{mod_name} - HiRes.ba2").read_bytes() == b"manual"
    assert (strings_dir / f"{mod_name}Other_en.STRINGS").read_bytes() == b"other"


def _mod_calling_into_the_base_game(tmp_path: Path) -> tuple[Path, Path]:
    mod_dir = tmp_path / "mod"
    source_dir = mod_dir / "Scripts" / "Source" / "User"
    source_dir.mkdir(parents=True)
    (source_dir / "S.psc").write_text(
        "Scriptname S extends Quest\nFunction F()\n  Int x = GetStage()\nEndFunction\n",
        encoding="utf-8",
    )
    game_data = tmp_path / "Data"
    (game_data / "Scripts" / "Source" / "Base").mkdir(parents=True)
    return mod_dir, game_data


def test_compile_papyrus_falls_back_to_the_bundled_corpus(tmp_path: Path):
    """No game install is not an error — the shipped type universe covers it.

    An empty Source/Base satisfies `is_dir()` and resolves nothing, so without a
    fallback every base-game call types as None and the diagnostics land on the
    mod's own lines as "cannot assign None to Int".
    """
    from creation_lib.build.deployer import compile_papyrus

    mod_dir, game_data = _mod_calling_into_the_base_game(tmp_path)
    messages: list[str] = []
    assert compile_papyrus(mod_dir, "fo4", game_data, on_progress=messages.append) == 1
    assert any("bundled fo4 type universe" in m for m in messages)
    assert (mod_dir / "data" / "Scripts" / "S.pex").is_file()


def test_compile_papyrus_names_the_missing_vanilla_sources(tmp_path: Path, monkeypatch):
    """With no install and no corpus, name what is missing.

    Asserts the diagnostic rather than the compile failure on purpose: the
    resolver memoizes parsed ASTs process-globally, keyed by script name with no
    import root in the key, so once another test in this process has compiled
    against a type universe the callees stay resolvable here no matter what this
    test imports. The message is what this test is about, and it is unaffected.
    """
    from creation_lib.build import deployer

    monkeypatch.setattr("creation_lib.pex.corpus.bundled_corpus_root", lambda game: None)
    mod_dir, game_data = _mod_calling_into_the_base_game(tmp_path)

    messages: list[str] = []
    try:
        deployer.compile_papyrus(mod_dir, "fo4", game_data, on_progress=messages.append)
    except RuntimeError as error:
        assert "no Creation Kit sources" in str(error)

    assert any("no bundled fo4 corpus" in m for m in messages)


def test_compile_papyrus_finds_skyrims_reversed_source_layout(tmp_path: Path, monkeypatch):
    """Skyrim keeps vanilla sources at Data/Source/Scripts, not Scripts/Source/Base.

    Looking only where Fallout 4 puts them meant a Skyrim install with its
    sources present was treated as having none.
    """
    from creation_lib.build import deployer

    mod_dir, game_data = _mod_calling_into_the_base_game(tmp_path)
    skyrim_base = game_data / "Source" / "Scripts"
    skyrim_base.mkdir(parents=True)
    (skyrim_base / "Quest.psc").write_text(
        "Scriptname Quest\nInt Function GetStage() native\n", encoding="utf-8"
    )

    captured: list[list[str]] = []
    real = deployer.compile_papyrus

    def _spy(source, *, imports, game, flags, source_path=None):
        captured.append(list(imports))
        from creation_lib.pex.native_runtime import CompileResult

        return CompileResult(ok=True, pex_bytes=b"pex")

    monkeypatch.setattr("creation_lib.pex.native_runtime.compile_psc", _spy)
    real(mod_dir, "skyrimse", game_data)

    assert any(str(skyrim_base) in parts for parts in captured)
