from __future__ import annotations

import json
from pathlib import Path
from unittest.mock import patch

from creation_lib.build.loose_deploy import deploy_loose_assets


def _build_loose_mod_tree(root: Path, mod_name: str) -> Path:
    mod_dir = root / "mods" / mod_name
    (mod_dir / "data" / "Textures" / "Effects").mkdir(parents=True, exist_ok=True)
    (mod_dir / "data").mkdir(parents=True, exist_ok=True)
    (mod_dir / "data" / "Textures" / "base.dds").write_text("source-base", encoding="utf-8")
    (mod_dir / "data" / "Textures" / "Effects" / "effect.dds").write_text(
        "source-effect",
        encoding="utf-8",
    )
    (mod_dir / "data" / "payload.txt").write_text("payload", encoding="utf-8")
    (mod_dir / f"{mod_name}.esp").write_text("plugin", encoding="utf-8")
    return mod_dir


def test_deploy_loose_assets_resizes_pc_textures_and_effects(tmp_path: Path):
    mod_name = "B21_TestLoose"
    mod_dir = _build_loose_mod_tree(tmp_path, mod_name)
    game_data_dir = tmp_path / "Game" / "Data"
    game_data_dir.mkdir(parents=True)

    prepare_calls: list[dict] = []

    def _fake_prepare(texture_src_dir: str, dest_root: str, max_res: int, effects_max_res: int):
        prepare_calls.append(
            {
                "texture_src_dir": Path(texture_src_dir).as_posix(),
                "dest_root": Path(dest_root).as_posix(),
                "max_res": max_res,
                "effects_max_res": effects_max_res,
            }
        )
        staged_textures = Path(dest_root) / "Textures"
        (staged_textures / "Effects").mkdir(parents=True, exist_ok=True)
        staged_textures.mkdir(parents=True, exist_ok=True)
        (staged_textures / "base.dds").write_text(
            f"base:{max_res}:{effects_max_res}",
            encoding="utf-8",
        )
        (staged_textures / "Effects" / "effect.dds").write_text(
            f"effect:{max_res}:{effects_max_res}",
            encoding="utf-8",
        )

    with patch("creation_lib.build.loose_deploy._prepare_texture_root", side_effect=_fake_prepare):
        result = deploy_loose_assets(
            mod_name,
            game="fo4",
            game_data_dir=game_data_dir,
            skip_build=True,
            pc_max_res=1024,
            pc_effects_max_res=512,
            project_root=tmp_path,
        )

    assert len(prepare_calls) == 1
    assert prepare_calls[0]["texture_src_dir"] == (mod_dir / "data" / "Textures").as_posix()
    assert prepare_calls[0]["max_res"] == 1024
    assert prepare_calls[0]["effects_max_res"] == 512
    assert result.files_deployed == 4

    assert (game_data_dir / f"{mod_name}.esp").read_text(encoding="utf-8") == "plugin"
    assert (game_data_dir / "payload.txt").read_text(encoding="utf-8") == "payload"
    assert (game_data_dir / "Textures" / "base.dds").read_text(encoding="utf-8") == "base:1024:512"
    assert (
        game_data_dir / "Textures" / "Effects" / "effect.dds"
    ).read_text(encoding="utf-8") == "effect:1024:512"

    manifest = json.loads((mod_dir / ".loose_manifest.json").read_text(encoding="utf-8"))
    rels = {entry["rel"] for entry in manifest["files"]}
    assert rels == {
        f"{mod_name}.esp",
        "payload.txt",
        "Textures/base.dds",
        "Textures/Effects/effect.dds",
    }


def test_deploy_loose_assets_can_copy_to_separate_virtual_data_dir(tmp_path: Path):
    mod_name = "B21_TestLoose"
    mod_dir = _build_loose_mod_tree(tmp_path, mod_name)
    game_data_dir = tmp_path / "Game" / "Data"
    deploy_data_dir = tmp_path / "ModOrganizer" / "mods" / mod_name
    game_data_dir.mkdir(parents=True)
    deploy_data_dir.mkdir(parents=True)

    result = deploy_loose_assets(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        deploy_data_dir=deploy_data_dir,
        skip_build=True,
        skip_papyrus_compile=True,
        project_root=tmp_path,
    )

    assert result.plugin == f"{mod_name}.esp"
    assert (deploy_data_dir / f"{mod_name}.esp").read_text(encoding="utf-8") == "plugin"
    assert (deploy_data_dir / "payload.txt").read_text(encoding="utf-8") == "payload"
    assert not (game_data_dir / f"{mod_name}.esp").exists()
    manifest = json.loads((mod_dir / ".loose_manifest.json").read_text(encoding="utf-8"))
    assert manifest["game_data_dir"] == str(deploy_data_dir)


def test_deploy_loose_assets_skips_validation_when_requested(tmp_path: Path):
    mod_name = "B21_TestLoose"
    mod_dir = _build_loose_mod_tree(tmp_path, mod_name)
    (mod_dir / "yaml").mkdir(parents=True)
    game_data_dir = tmp_path / "Game" / "Data"
    game_data_dir.mkdir(parents=True)

    def _validate_authoring(*args, **kwargs):
        raise AssertionError("validate_authoring should not run when skip_validation is enabled")

    with patch(
        "creation_lib.build.loose_deploy.validate_authoring", side_effect=_validate_authoring
    ), patch("creation_lib.build.loose_deploy.deserialize"):
        result = deploy_loose_assets(
            mod_name,
            game="fo4",
            game_data_dir=game_data_dir,
            skip_build=False,
            skip_validation=True,
            project_root=tmp_path,
        )

    assert result.plugin == f"{mod_name}.esp"
    assert (game_data_dir / f"{mod_name}.esp").read_text(encoding="utf-8") == "plugin"
