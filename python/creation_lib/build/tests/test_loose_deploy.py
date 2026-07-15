from __future__ import annotations

from pathlib import Path

from creation_lib.build import loose_deploy
from creation_lib.build.loose_deploy import deploy_loose_assets


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
    scripts_dir.mkdir(parents=True)
    mcm_dir.mkdir(parents=True)
    game_data_dir.mkdir(parents=True)
    (mod_dir / f"{mod_name}.esp").write_bytes(b"esp")
    (scripts_dir / "B21_Test.pex").write_bytes(b"pex")
    (mcm_dir / "settings.ini").write_bytes(b"mcm")

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

    assert max_workers_seen == [2]
    assert result.files_deployed == 3
    assert (game_data_dir / f"{mod_name}.esp").read_bytes() == b"esp"
    assert (game_data_dir / "Scripts" / "B21_Test.pex").read_bytes() == b"pex"
    assert (game_data_dir / "MCM" / "settings.ini").read_bytes() == b"mcm"
