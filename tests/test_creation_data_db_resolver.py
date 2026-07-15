import pytest


def test_get_db_path_requires_explicit_db_dir_when_database_exists_in_cwd(tmp_path, monkeypatch):
    from creation_lib.creation_data._db_resolver import get_db_path

    data_dir = tmp_path / "data"
    data_dir.mkdir()
    db_path = data_dir / "fo4_wiki.db"
    db_path.write_bytes(b"")

    monkeypatch.chdir(tmp_path)

    with pytest.raises(TypeError):
        get_db_path("wiki", "fo4")


def test_get_db_path_uses_explicit_db_dir(tmp_path):
    from creation_lib.creation_data._db_resolver import get_db_path

    db_path = tmp_path / "fo4_wiki.db"
    db_path.write_bytes(b"")

    resolved = get_db_path("wiki", "fo4", db_dir=str(tmp_path))

    assert resolved == str(db_path)


def test_fnv_wiki_uses_shared_fo3_database(tmp_path):
    from creation_lib.creation_data._db_resolver import db_available, get_db_path

    db_path = tmp_path / "fo3_wiki.db"
    db_path.write_bytes(b"")

    assert db_available("wiki", "fnv", db_dir=str(tmp_path))
    assert get_db_path("wiki", "fnv", db_dir=str(tmp_path)) == str(db_path)
