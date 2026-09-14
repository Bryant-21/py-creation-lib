import json
from pathlib import Path

from creation_lib.core.store_install import validate_store_install_for_game


def _steam_fo4_root(tmp_path: Path) -> Path:
    root = tmp_path / "SteamLibrary" / "steamapps" / "common" / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (root / "steam_api64.dll").write_bytes(b"dll")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")
    manifest = root.parents[1] / "appmanifest_377160.acf"
    manifest.write_text(
        '"AppState"\n{\n    "appid"        "377160"\n'
        '    "installdir"   "Fallout 4"\n}',
        encoding="utf-8",
    )
    return root


def _gog_fo4_root(tmp_path: Path) -> Path:
    root = tmp_path / "GOG Games" / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")
    (root / "goggame-1998527297.info").write_text(
        json.dumps(
            {
                "gameId": "1998527297",
                "playTasks": [{"path": "Fallout4.exe", "type": "FileTask"}],
            }
        ),
        encoding="utf-8",
    )
    return root


def test_validate_store_install_accepts_steam_install(tmp_path):
    root = _steam_fo4_root(tmp_path)

    result = validate_store_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.store == "steam"
    assert result.message == "Fallout 4 Steam install verified."


def test_validate_store_install_accepts_gog_install(tmp_path):
    root = _gog_fo4_root(tmp_path)

    result = validate_store_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.store == "gog"
    assert result.message == "Fallout 4 GOG install verified."
    assert result.gog.product_id == "1998527297"


def test_validate_store_install_rejects_install_from_neither_store(tmp_path):
    root = tmp_path / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")

    result = validate_store_install_for_game("fo4", str(root))

    assert result.ok is False
    assert result.store == ""
    assert result.local_install_valid is True
    assert "was not recognized as a Steam or GOG install" in result.message
    assert "Steam: " in result.message
    assert "GOG: " in result.message


def test_validate_store_install_reports_invalid_folder_without_store_noise(tmp_path):
    result = validate_store_install_for_game("fo4", str(tmp_path / "Fallout 4"))

    assert result.ok is False
    assert result.store == ""
    assert result.local_install_valid is False
    assert result.message == (
        "Fallout 4 install is invalid: executable or Data archives not found."
    )
    assert "GOG: " not in result.message


def test_validate_store_install_fo76_has_no_gog_release_and_needs_steam(tmp_path):
    root = tmp_path / "Fallout76"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout76.exe").write_bytes(b"exe")
    (data / "SeventySix - Startup.ba2").write_bytes(b"ba2")

    result = validate_store_install_for_game("fo76", str(root))

    assert result.ok is False
    assert result.store == ""
    assert result.gog.info_present is False
