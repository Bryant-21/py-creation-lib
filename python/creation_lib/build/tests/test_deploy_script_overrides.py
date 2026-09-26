from pathlib import Path

import pytest

from creation_lib.build.deploy_plan import plan_deploy
from creation_lib.build.deployer import deploy_mod


@pytest.mark.parametrize("esp_only", [False, True])
def test_packed_deploy_refreshes_only_existing_owned_script_overrides(tmp_path, esp_only):
    mod = tmp_path / "mods" / "B21_Test"
    target = tmp_path / "MO2" / "B21_Test"
    for root, files in [
        (mod, {
            "B21_Test.esp": b"plugin",
            "B21_Test - Main.ba2": b"archive",
            "Scripts/B21/Controller.pex": b"old staging",
            "data/Scripts/B21/Controller.pex": b"new compiled script",
            "data/Scripts/B21/ArchiveOnly.pex": b"archive only",
            "data/Scripts/Source/Import.pex": b"import only",
        }),
        (target, {
            "Scripts/B21/Controller.pex": b"stale override",
            "Scripts/OtherMod.pex": b"other mod",
            "Scripts/Source/Import.pex": b"keep import",
        }),
    ]:
        for relative, content in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
    plan = plan_deploy(mod, target, game="fo4", skip_build=True, skip_pack=True,
                       skip_papyrus_compile=True, esp_only=esp_only)
    assert (target / "Scripts/B21/Controller.pex").read_bytes() == b"stale override"
    result = deploy_mod(mod.name, game="fo4", game_data_dir=tmp_path / "Game" / "Data",
                        deploy_data_dir=target, project_root=tmp_path, skip_build=True,
                        skip_pack=True, skip_papyrus_compile=True, esp_only=esp_only)
    assert (target / "Scripts/B21/Controller.pex").read_bytes() == (
        b"stale override" if esp_only else b"new compiled script")
    assert (target / "Scripts/OtherMod.pex").read_bytes() == b"other mod"
    assert (target / "Scripts/Source/Import.pex").read_bytes() == b"keep import"
    assert not (target / "Scripts/B21/ArchiveOnly.pex").exists()
    assert not (tmp_path / "Game").exists()
    assert result.loose_files_deployed == (0 if esp_only else 1)
    scripts = {Path(op["destination"]).relative_to(target).as_posix()
               for op in plan["operations"] if Path(op["destination"]).suffix == ".pex"}
    assert scripts == (set() if esp_only else {"Scripts/B21/Controller.pex"})
