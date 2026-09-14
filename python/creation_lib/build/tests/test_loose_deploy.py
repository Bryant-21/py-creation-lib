from __future__ import annotations

import json
from pathlib import Path

import pytest

from creation_lib.build import loose_deploy
from creation_lib.build.loose_deploy import deploy_loose_assets, deploy_loose_file


def test_resolve_loose_copy_workers_uses_auto_cpu_count(monkeypatch):
    monkeypatch.setattr(loose_deploy.os, "cpu_count", lambda: 16)

    assert loose_deploy._resolve_copy_workers(0, 20) == 8
    assert loose_deploy._resolve_copy_workers(3, 20) == 3
    assert loose_deploy._resolve_copy_workers(12, 5) == 5


def test_deploy_loose_assets_uses_worker_pool_for_bulk_copy(tmp_path: Path, monkeypatch):
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


def test_deploy_loose_file_rejects_asset_outside_mod_roots(tmp_path: Path):
    mod_dir = tmp_path / "mods" / "B21_Test"
    outside = tmp_path / "outside.nif"
    game_data = tmp_path / "Game" / "Data"
    mod_dir.mkdir(parents=True)
    game_data.mkdir(parents=True)
    outside.write_bytes(b"outside")

    with pytest.raises(ValueError, match="must be under"):
        deploy_loose_file(
            "B21_Test",
            outside,
            game="fo4",
            game_data_dir=game_data,
            project_root=tmp_path,
        )


def test_deploy_asset_directory_preserves_other_manifest_entries(tmp_path: Path):
    mod_dir = tmp_path / "mods/B21_Test"
    icons = mod_dir / "data/Meshes/Icons"
    (icons / "nested").mkdir(parents=True)
    (icons / "first.nif").write_bytes(b"first")
    (icons / "nested/second.nif").write_bytes(b"second")
    script = mod_dir / "data/Scripts/pending.pex"
    script.parent.mkdir()
    script.write_bytes(b"keep staged")
    data = tmp_path / "Game/Data"
    manifest = mod_dir / ".loose_manifest.json"
    manifest.write_text(json.dumps({"game_data_dir": str(data),
        "files": [{"rel": "Textures/existing.dds"}], "claimed_dirs": []}), encoding="utf-8")
    for _ in range(2):
        deployed = deploy_loose_file("B21_Test", "data/Meshes/Icons", game="fo4",
                                    game_data_dir=data, project_root=tmp_path)
    assert deployed == data / "Meshes/Icons"
    assert (deployed / "first.nif").read_bytes() == b"first"
    assert (deployed / "nested/second.nif").read_bytes() == b"second"
    assert not (data / "Scripts/pending.pex").exists()
    entries = json.loads(manifest.read_text(encoding="utf-8"))["files"]
    assert {entry["rel"] for entry in entries} == {
        "Textures/existing.dds", "Meshes/Icons/first.nif", "Meshes/Icons/nested/second.nif"}
    assert len(entries) == 3
