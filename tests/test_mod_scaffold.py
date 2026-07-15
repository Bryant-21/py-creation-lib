from __future__ import annotations

from pathlib import Path

from creation_lib.mod import scaffold


def test_migrate_mod_keeps_project_dirs_out_of_data(tmp_path, monkeypatch):
    project_root = tmp_path / "workspace"
    (project_root / "mods").mkdir(parents=True)

    source_dir = tmp_path / "Fallout4Mods" / "B21_PlasmaCaster"
    (source_dir / "B21_PlasmaCaster.esp").parent.mkdir(parents=True, exist_ok=True)
    (source_dir / "B21_PlasmaCaster.esp").write_text("", encoding="utf-8")
    (source_dir / "data" / "Meshes").mkdir(parents=True)
    (source_dir / "data" / "Meshes" / "weapon.nif").write_text("mesh", encoding="utf-8")
    (source_dir / "Scripts" / "Source" / "User").mkdir(parents=True)
    (source_dir / "Scripts" / "Source" / "User" / "PlasmaCaster.psc").write_text(
        "Scriptname PlasmaCaster",
        encoding="utf-8",
    )
    (source_dir / "docs").mkdir()
    (source_dir / "docs" / "notes.txt").write_text("notes", encoding="utf-8")
    (source_dir / "patches" / "Example" / "yaml").mkdir(parents=True)
    (source_dir / "patches" / "Example" / "yaml" / "Patch.yaml").write_text(
        "plugin: Example.esp",
        encoding="utf-8",
    )

    monkeypatch.setattr(
        scaffold,
        "serialize",
        lambda *args, **kwargs: (kwargs.get("on_progress") or (lambda _msg: None))("serialized"),
    )

    mod_dir = scaffold.migrate_mod(
        source_dir,
        mod_name="B21_PlasmaCaster",
        game="fo4",
        init_git=False,
        project_root=project_root,
    )

    assert mod_dir == project_root / "mods" / "B21_PlasmaCaster"
    assert (mod_dir / "data" / "Meshes" / "weapon.nif").is_file()
    assert not (mod_dir / "data" / "docs").exists()
    assert (mod_dir / "docs" / "notes.txt").is_file()
    assert not (mod_dir / "data" / "patches").exists()
    assert (mod_dir / "patches" / "Example" / "yaml" / "Patch.yaml").is_file()
    assert (mod_dir / "Scripts" / "Source" / "User" / "PlasmaCaster.psc").is_file()
    assert not (mod_dir / "Scripts" / "Source" / "User" / "User" / "PlasmaCaster.psc").exists()
    assert not (mod_dir / "data" / "Scripts" / "Source").exists()
