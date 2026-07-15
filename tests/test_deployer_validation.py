from __future__ import annotations

from pathlib import Path
from unittest.mock import patch

from creation_lib.build.deployer import deploy_mod


def test_deploy_mod_skips_validation_when_requested(tmp_path: Path):
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    yaml_dir = mod_dir / "yaml"
    yaml_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_text("plugin", encoding="utf-8")

    def _validate_authoring(*args, **kwargs):
        raise AssertionError("validate_authoring should not run when skip_validation is enabled")

    with patch(
        "creation_lib.build.deployer.validate_authoring", side_effect=_validate_authoring
    ), patch("creation_lib.build.deployer.deserialize"):
        result = deploy_mod(
            mod_name,
            game="fo4",
            game_data_dir=game_data_dir,
            skip_build=False,
            skip_validation=True,
            esp_only=True,
            project_root=tmp_path,
            resource_dir=tmp_path / "resource",
        )

    assert result.plugin_deployed == f"{mod_name}.esp"
    assert (game_data_dir / f"{mod_name}.esp").read_text(encoding="utf-8") == "plugin"
