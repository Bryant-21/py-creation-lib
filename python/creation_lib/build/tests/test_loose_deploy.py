from __future__ import annotations

import json
from pathlib import Path

import pytest

from creation_lib.build import loose_deploy
from creation_lib.build.loose_deploy import (
    deploy_loose_assets,
    deploy_loose_file,
    undeploy_loose_assets,
    undeploy_loose_file,
)


def test_deploy_loose_assets_uses_worker_pool_for_bulk_copy(tmp_path: Path, monkeypatch):
    monkeypatch.setattr(loose_deploy.os, "cpu_count", lambda: 16)

    assert loose_deploy._resolve_copy_workers(0, 20) == 8
    assert loose_deploy._resolve_copy_workers(3, 20) == 3
    assert loose_deploy._resolve_copy_workers(12, 5) == 5

    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    scripts_dir = mod_dir / "data" / "Scripts"
    mcm_dir = mod_dir / "MCM"
    terrain_dir = mod_dir / "Terrain"
    scripts_dir.mkdir(parents=True)
    mcm_dir.mkdir(parents=True)
    terrain_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (scripts_dir / "B21_Test.pex").write_bytes(b"pex")
    (mcm_dir / "settings.ini").write_bytes(b"mcm")
    (terrain_dir / "Appalachia.btd4").write_bytes(b"btd4")

    max_workers_seen: list[int] = []

    class ImmediateFuture:
        def __init__(self, value):
            self.value = value

        def result(self):
            return self.value

    class RecordingExecutor:
        def __init__(self, max_workers: int):
            max_workers_seen.append(max_workers)

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def submit(self, fn, job):
            return ImmediateFuture(fn(job))

    monkeypatch.setattr(loose_deploy, "ThreadPoolExecutor", RecordingExecutor)

    result = deploy_loose_assets(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_papyrus_compile=True,
        workers=3,
        project_root=tmp_path,
    )

    assert max_workers_seen == [3]
    assert result.files_deployed == 4
    assert (game_data_dir / f"{mod_name}.esp").read_bytes() == b"esp"
    assert (game_data_dir / "Scripts" / "B21_Test.pex").read_bytes() == b"pex"
    assert (game_data_dir / "MCM" / "settings.ini").read_bytes() == b"mcm"
    assert (game_data_dir / "Terrain" / "Appalachia.btd4").read_bytes() == b"btd4"


def test_deploy_loose_file_copies_only_requested_asset_and_tracks_it(tmp_path: Path):
    mod_dir = tmp_path / "mods" / "B21_Test"
    source = mod_dir / "data" / "Meshes" / "Props" / "test.nif"
    unrelated = mod_dir / "data" / "Meshes" / "Props" / "unrelated.nif"
    game_data = tmp_path / "Game" / "Data"
    source.parent.mkdir(parents=True)
    game_data.mkdir(parents=True)
    source.write_bytes(b"first")
    unrelated.write_bytes(b"unrelated")

    deployed = deploy_loose_file(
        "B21_Test",
        source,
        game="fo4",
        game_data_dir=game_data,
        project_root=tmp_path,
    )

    assert deployed.read_bytes() == b"first"
    assert deployed == game_data / "Meshes" / "Props" / "test.nif"
    assert not (game_data / "Meshes" / "Props" / "unrelated.nif").exists()
    manifest = json.loads((mod_dir / ".loose_manifest.json").read_text(encoding="utf-8"))
    assert [entry["rel"] for entry in manifest["files"]] == ["Meshes/Props/test.nif"]

    source.write_bytes(b"second")
    deploy_loose_file(
        "B21_Test",
        Path("data/Meshes/Props/test.nif"),
        game="fo4",
        game_data_dir=game_data,
        project_root=tmp_path,
    )
    manifest = json.loads((mod_dir / ".loose_manifest.json").read_text(encoding="utf-8"))
    assert deployed.read_bytes() == b"second"
    assert len(manifest["files"]) == 1

    outside = tmp_path / "outside.nif"
    outside.write_bytes(b"outside")
    with pytest.raises(ValueError, match="must be under"):
        deploy_loose_file(
            "B21_Test",
            outside,
            game="fo4",
            game_data_dir=game_data,
            project_root=tmp_path,
        )


def test_undeploy_loose_file_removes_named_asset_and_empty_manifest(tmp_path: Path):
    mod_dir = tmp_path / "mods" / "B21_Test"
    target = mod_dir / "data" / "Meshes" / "Props" / "test.nif"
    keep = mod_dir / "data" / "Scripts" / "Keep.pex"
    game_data = tmp_path / "Game" / "Data"
    target.parent.mkdir(parents=True)
    keep.parent.mkdir(parents=True)
    game_data.mkdir(parents=True)
    target.write_bytes(b"nif")
    keep.write_bytes(b"pex")
    for asset in (target, keep):
        deploy_loose_file(
            "B21_Test", asset, game="fo4", game_data_dir=game_data, project_root=tmp_path
        )

    # The mod copy may already have been regenerated away; undeploy resolves the
    # destination from the path alone.
    target.unlink()
    dry = undeploy_loose_file(
        "B21_Test",
        "data/Meshes/Props/test.nif",
        project_root=tmp_path,
        dry_run=True,
    )
    assert dry == ["Meshes/Props/test.nif"]
    assert (game_data / "Meshes" / "Props" / "test.nif").is_file()

    removed = undeploy_loose_file(
        "B21_Test", "data/Meshes/Props/test.nif", project_root=tmp_path
    )

    assert removed == ["Meshes/Props/test.nif"]
    assert not (game_data / "Meshes" / "Props" / "test.nif").exists()
    assert (game_data / "Scripts" / "Keep.pex").read_bytes() == b"pex"
    manifest = json.loads((mod_dir / ".loose_manifest.json").read_text(encoding="utf-8"))
    assert [entry["rel"] for entry in manifest["files"]] == ["Scripts/Keep.pex"]

    assert undeploy_loose_file(
        "B21_Test", "data/Meshes/Props/test.nif", project_root=tmp_path
    ) == []

    # Undeploying a whole directory of assets drops the manifest entirely once
    # it has nothing left in it.
    empty_root = tmp_path / "empty"
    empty_mod_dir = empty_root / "mods" / "B21_Test"
    empty_tree = empty_mod_dir / "data" / "Meshes" / "Props"
    empty_game_data = empty_root / "Game" / "Data"
    empty_tree.mkdir(parents=True)
    empty_game_data.mkdir(parents=True)
    (empty_tree / "a.nif").write_bytes(b"a")
    (empty_tree / "b.nif").write_bytes(b"b")
    deploy_loose_file(
        "B21_Test", "data/Meshes/Props", game="fo4", game_data_dir=empty_game_data,
        project_root=empty_root,
    )

    empty_removed = undeploy_loose_file(
        "B21_Test", "data/Meshes/Props", project_root=empty_root
    )

    assert sorted(empty_removed) == ["Meshes/Props/a.nif", "Meshes/Props/b.nif"]
    assert not (empty_game_data / "Meshes" / "Props").exists()
    assert not (empty_mod_dir / ".loose_manifest.json").exists()


def test_deploy_prisma_icons_and_asset_directory_preserve_other_entries(tmp_path: Path):
    mod = tmp_path / "mods/B21_Test"
    relative = Path("PrismaUI_F4/views/B21_FullScreenMap/challenges")
    icons = mod / relative
    icons.mkdir(parents=True)
    (icons / "caps.png").write_bytes(b"caps")
    data = tmp_path / "Game/Data"
    other = data / "PrismaUI_F4/views/Other/index.html"
    other.parent.mkdir(parents=True)
    other.write_text("keep", encoding="utf-8")
    deployed = deploy_loose_file("B21_Test", relative, game="fo4",
                                game_data_dir=data, project_root=tmp_path)
    assert (deployed / "caps.png").read_bytes() == b"caps"
    manifest = json.loads((mod / loose_deploy.MANIFEST_NAME).read_text(encoding="utf-8"))
    assert [entry["rel"] for entry in manifest["files"]] == [(relative / "caps.png").as_posix()]
    undeploy_loose_file("B21_Test", relative, project_root=tmp_path)
    assert not (deployed / "caps.png").exists()
    assert other.read_text(encoding="utf-8") == "keep"

    # Deploying a whole asset directory (twice, to cover the redeploy path) keeps
    # unrelated pre-existing manifest entries and doesn't sweep up staged files
    # outside the deployed subtree.
    dir_root = tmp_path / "dir"
    mod_dir = dir_root / "mods/B21_Test"
    icons = mod_dir / "data/Meshes/Icons"
    (icons / "nested").mkdir(parents=True)
    (icons / "first.nif").write_bytes(b"first")
    (icons / "nested/second.nif").write_bytes(b"second")
    script = mod_dir / "data/Scripts/pending.pex"
    script.parent.mkdir()
    script.write_bytes(b"keep staged")
    dir_data = dir_root / "Game/Data"
    dir_manifest = mod_dir / ".loose_manifest.json"
    dir_manifest.write_text(json.dumps({"game_data_dir": str(dir_data),
        "files": [{"rel": "Textures/existing.dds"}], "claimed_dirs": []}), encoding="utf-8")
    for _ in range(2):
        dir_deployed = deploy_loose_file("B21_Test", "data/Meshes/Icons", game="fo4",
                                    game_data_dir=dir_data, project_root=dir_root)
    assert dir_deployed == dir_data / "Meshes/Icons"
    assert (dir_deployed / "first.nif").read_bytes() == b"first"
    assert (dir_deployed / "nested/second.nif").read_bytes() == b"second"
    assert not (dir_data / "Scripts/pending.pex").exists()
    dir_entries = json.loads(dir_manifest.read_text(encoding="utf-8"))["files"]
    assert {entry["rel"] for entry in dir_entries} == {
        "Textures/existing.dds", "Meshes/Icons/first.nif", "Meshes/Icons/nested/second.nif"}
    assert len(dir_entries) == 3


def test_deploy_and_undeploy_round_trip_precombine_sidecars(tmp_path: Path):
    """Loose deploy ships the .csg/.cdx precombine sidecars beside the plugin
    like any other loose asset, and the manifest-driven undeploy removes them.
    """
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    game_data_dir = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (mod_dir / f"{mod_name} - Geometry.csg").write_bytes(b"csg")
    (mod_dir / f"{mod_name}.cdx").write_bytes(b"cdx")
    (mod_dir / f"{mod_name} - Exterior.cdx").write_bytes(b"exterior")

    result = deploy_loose_assets(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_papyrus_compile=True,
        project_root=tmp_path,
    )

    assert result.files_deployed == 4  # esp + csg + cdx + exterior cdx
    assert (game_data_dir / f"{mod_name} - Geometry.csg").read_bytes() == b"csg"
    assert (game_data_dir / f"{mod_name}.cdx").read_bytes() == b"cdx"
    assert (game_data_dir / f"{mod_name} - Exterior.cdx").read_bytes() == b"exterior"

    removed = undeploy_loose_assets(
        mod_name,
        game_data_dir=game_data_dir,
        project_root=tmp_path,
    )

    assert f"{mod_name} - Geometry.csg" in removed
    assert f"{mod_name}.cdx" in removed
    assert f"{mod_name} - Exterior.cdx" in removed
    assert not (game_data_dir / f"{mod_name} - Geometry.csg").exists()
    assert not (game_data_dir / f"{mod_name}.cdx").exists()
    assert not (game_data_dir / f"{mod_name} - Exterior.cdx").exists()


def test_deploy_loose_assets_finds_sidecar_under_data_dir_without_duplicating(tmp_path: Path):
    """A sidecar shipped under mods/<Mod>/data/ is picked up exactly once —
    not once by the generic data/ walk and again by the explicit sidecar step.
    """
    mod_name = "B21_Test"
    mod_dir = tmp_path / "mods" / mod_name
    data_dir = mod_dir / "data"
    game_data_dir = tmp_path / "Game" / "Data"
    data_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (data_dir / f"{mod_name} - Geometry.csg").write_bytes(b"csg-in-data")

    result = deploy_loose_assets(
        mod_name,
        game="fo4",
        game_data_dir=game_data_dir,
        skip_build=True,
        skip_papyrus_compile=True,
        project_root=tmp_path,
    )

    assert result.files_deployed == 2  # esp + csg, not double-counted
    assert (game_data_dir / f"{mod_name} - Geometry.csg").read_bytes() == b"csg-in-data"
    manifest = json.loads((mod_dir / loose_deploy.MANIFEST_NAME).read_text(encoding="utf-8"))
    rels = [entry["rel"] for entry in manifest["files"]]
    assert rels.count(f"{mod_name} - Geometry.csg") == 1
