from __future__ import annotations

from pathlib import Path
from subprocess import CompletedProcess

import creation_lib.mod.git_ops as git_ops
import creation_lib.mod.scaffold as scaffold


def test_gitea_init_reports_progress(tmp_path: Path, monkeypatch):
    mod_dir = tmp_path / "mods" / "B21_Test"
    mod_dir.mkdir(parents=True)

    messages: list[str] = []

    monkeypatch.setattr(
        git_ops,
        "_run_git",
        lambda *args, **kwargs: CompletedProcess(args=["git"], returncode=0, stdout="", stderr=""),
    )
    monkeypatch.setattr(git_ops, "_current_branch", lambda mod_dir: "main")
    monkeypatch.setattr(
        git_ops,
        "_push",
        lambda *args, **kwargs: CompletedProcess(args=["git"], returncode=0, stdout="", stderr=""),
    )

    git_ops.gitea_init(
        mod_dir,
        "B21_Test",
        game="fo4",
        gitea_url="https://example.invalid",
        gitea_user="user",
        on_progress=messages.append,
    )

    assert "Initializing local git repository..." in messages
    assert "Creating initial commit..." in messages
    assert "Pushing initial commit to Gitea..." in messages
    assert messages[-1] == "Git repo created and pushed (push-to-create)."


def test_migrate_mod_reports_git_stage_before_completion(tmp_path: Path, monkeypatch):
    source_dir = tmp_path / "source_mod"
    source_dir.mkdir()
    messages: list[str] = []

    monkeypatch.setattr(
        scaffold,
        "_find_plugins",
        lambda source_dir, mod_name: (None, [], source_dir),
    )
    monkeypatch.setattr(scaffold, "_find_yaml_dir", lambda source_dir, mod_name: None)
    monkeypatch.setattr(scaffold, "_parallel_copy", lambda pairs, on_progress=None, label="": 0)

    def fake_gitea_init(mod_dir, mod_name, **kwargs):
        on_progress = kwargs.get("on_progress")
        if on_progress:
            on_progress("Git repo created and pushed (push-to-create).")

    monkeypatch.setattr(git_ops, "gitea_init", fake_gitea_init)

    scaffold.migrate_mod(
        source_dir,
        mod_name="B21_Test",
        game="fo4",
        project_root=tmp_path,
        gitea_url="https://example.invalid",
        gitea_user="user",
        on_progress=messages.append,
    )

    git_stage = "[5/5] Initializing git repo and pushing to Gitea..."
    assert git_stage in messages
    assert messages.index(git_stage) < messages.index("=== Migration complete ===")
    assert messages[-1] == "=== Migration complete ==="
