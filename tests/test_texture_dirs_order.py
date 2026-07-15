from pathlib import Path

from creation_lib.textures.texture_dirs import build_texture_dirs


class _Settings:
    def __init__(self, game_paths):
        self._game_paths = game_paths

    def get_game_paths(self, _game_id: str):
        return dict(self._game_paths)


def test_build_texture_dirs_prioritizes_nif_data_root(tmp_path, monkeypatch):
    extra = tmp_path / "extra"
    extracted = tmp_path / "extracted"
    managed = tmp_path / "managed"
    root = tmp_path / "game_root"
    nif_dir = tmp_path / "LooseMod" / "Data" / "Meshes" / "Weapons" / "Demo"
    nif_dir.mkdir(parents=True)
    extra.mkdir()
    extracted.mkdir()
    managed.mkdir()
    (root / "Data").mkdir(parents=True)

    settings = _Settings(
        {
            "additional_paths": [str(extra)],
            "extracted_dir": str(extracted),
            "root_dir": str(root),
        }
    )

    def _add_managed(texture_dirs: list[Path], _game_id: str, _mods_root: Path):
        texture_dirs.append(managed)

    monkeypatch.setattr(
        "creation_lib.textures.texture_dirs._add_managed_mod_dirs",
        _add_managed,
    )

    nif_path = nif_dir / "receiver_int_1.nif"
    texture_dirs, user_archive_dirs, base_archive_dirs = build_texture_dirs(
        settings,
        game_id="fo4",
        nif_path=str(nif_path),
        mods_root=tmp_path / "mods",
    )

    data_root = nif_dir.parents[2]
    assert texture_dirs.index(extra) < texture_dirs.index(nif_dir)
    assert texture_dirs.index(nif_dir) < texture_dirs.index(data_root)
    assert texture_dirs.index(data_root) < texture_dirs.index(extracted)
    assert texture_dirs.index(data_root) < texture_dirs.index(managed)
    assert user_archive_dirs == [extra]
    assert base_archive_dirs == [root, root / "Data"]
