from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

from creation_lib.mod.git_ops import git_lfs_prune

pytestmark = pytest.mark.skipif(shutil.which("git-lfs") is None, reason="git-lfs is not installed")

MESH = b"mesh" * 1000


def _git(root: Path, *args: str) -> str:
    return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout.strip()


def _local_lfs_objects(repo: Path) -> set[str]:
    return {p.name for p in (repo / ".git" / "lfs" / "objects").rglob("*") if p.is_file()}


def _oid(repo: Path, path: str) -> str:
    pointer = _git(repo, "show", f"HEAD:{path}")
    return next(line.split(":", 1)[1] for line in pointer.splitlines() if line.startswith("oid sha256:"))


@pytest.fixture
def repo(tmp_path: Path) -> tuple[Path, str]:
    remote = tmp_path / "remote.git"
    remote.mkdir()
    _git(remote, "init", "--bare")
    work = tmp_path / "B21_Test"
    work.mkdir()
    _git(work, "init", "-b", "main")
    _git(work, "config", "user.name", "Test")
    _git(work, "config", "user.email", "test@example.invalid")
    _git(work, "lfs", "install", "--local")
    _git(work, "lfs", "track", "*.nif")
    (work / "mesh.nif").write_bytes(MESH)
    _git(work, "add", ".")
    _git(work, "commit", "-m", "Initial")
    _git(work, "remote", "add", "origin", remote.as_uri())
    return work, remote.as_uri()


def test_prunes_objects_pushed_to_a_url_without_tracking_refs(repo):
    work, remote_url = repo
    _git(work, "push", remote_url, "main")
    assert _git(work, "for-each-ref", "refs/remotes") == ""

    git_lfs_prune(work)

    assert _local_lfs_objects(work) == set()
    assert (work / "mesh.nif").read_bytes() == MESH


def test_keeps_objects_of_unpushed_commits(repo):
    work, remote_url = repo
    _git(work, "push", remote_url, "main")
    (work / "unpushed.nif").write_bytes(b"unpushed" * 1000)
    _git(work, "add", "unpushed.nif")
    _git(work, "commit", "-m", "Local only")

    git_lfs_prune(work)

    assert _local_lfs_objects(work) == {_oid(work, "unpushed.nif")}
