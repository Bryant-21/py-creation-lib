from pathlib import Path


def _reset_host():
    import creation_lib.ui.host as host_mod

    host_mod._host = None


def test_default_host_is_functional(tmp_path, monkeypatch):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    _reset_host()
    from creation_lib.ui.host import get_host

    host = get_host()
    root = host.get_app_root()
    assert root == tmp_path / "creation_lib"
    assert root.is_dir()
    assert host.get_ini_dir().is_dir()
    assert host.get_db_dir() == root / "data"
    assert host.resolve_extracted_output_dir("fo4") == root / "extracted" / "fo4"
    assert host.db_builder_factory is None
    _reset_host()


def test_set_host_round_trip():
    _reset_host()
    from creation_lib.ui.host import UiHost, get_host, set_host

    marker = Path("X:/host-marker")
    set_host(
        UiHost(
            get_app_root=lambda: marker,
            get_ini_dir=lambda: marker,
            get_db_dir=lambda: marker,
            resolve_extracted_output_dir=lambda game: marker / game,
            db_builder_factory=lambda **kw: kw,
        )
    )
    try:
        host = get_host()
        assert host.get_app_root() == marker
        assert host.db_builder_factory(a=1) == {"a": 1}
    finally:
        _reset_host()


def test_game_esm_yaml_dir_covers_all_games():
    from creation_lib.ui.host import GAME_ESM_YAML_DIR

    assert GAME_ESM_YAML_DIR == {
        "fo4": "fo4_esm_yaml",
        "skyrimse": "skyrimse_esm_yaml",
        "starfield": "starfield_esm_yaml",
        "fo76": "fo76_esm_yaml",
        "fo3": "fo3_esm_yaml",
        "fnv": "fnv_esm_yaml",
    }
