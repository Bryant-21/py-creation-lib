import json
from pathlib import Path

from creation_lib.core.gog_install import GogInstallResult, validate_gog_install_for_game


def _gog_fo4_root(tmp_path: Path) -> Path:
    root = tmp_path / "GOG Games" / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")
    return root


def _write_info(
    root: Path,
    *,
    product_id: str = "1998527297",
    play_task_path: str | None = "Fallout4.exe",
    game_id_key: str = "gameId",
) -> Path:
    payload: dict = {
        game_id_key: product_id,
        "name": "Fallout 4",
        "version": 1,
    }
    if play_task_path is not None:
        payload["playTasks"] = [
            {
                "category": "game",
                "isPrimary": True,
                "name": "Fallout 4",
                "path": play_task_path,
                "type": "FileTask",
            }
        ]
    info = root / f"goggame-{product_id}.info"
    info.write_text(json.dumps(payload), encoding="utf-8")
    return info


def test_validate_gog_install_accepts_info_manifest_with_matching_play_task(tmp_path):
    root = _gog_fo4_root(tmp_path)
    info = _write_info(root)

    result = validate_gog_install_for_game("fo4", str(root))

    assert result == GogInstallResult(
        ok=True,
        game_id="fo4",
        root_dir=str(root),
        local_install_valid=True,
        info_present=True,
        info_parsed=True,
        play_task_present=True,
        product_id="1998527297",
        info_path=str(info),
        message="Fallout 4 GOG install verified.",
    )


def test_validate_gog_install_accepts_backslash_play_task_path(tmp_path):
    root = _gog_fo4_root(tmp_path)
    (root / "bin").mkdir()
    (root / "bin" / "launcher.exe").write_bytes(b"exe")
    _write_info(root, play_task_path=r"bin\launcher.exe")

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True


def test_validate_gog_install_accepts_bom_encoded_info(tmp_path):
    root = _gog_fo4_root(tmp_path)
    payload = {
        "gameId": "1998527297",
        "playTasks": [{"path": "Fallout4.exe", "type": "FileTask"}],
    }
    (root / "goggame-1998527297.info").write_text(
        json.dumps(payload), encoding="utf-8-sig"
    )

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True


def test_validate_gog_install_skips_dlc_info_without_play_tasks(tmp_path):
    root = _gog_fo4_root(tmp_path)
    _write_info(root, product_id="1000000001", play_task_path=None)
    base_info = _write_info(root, product_id="1998527297")

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.info_path == str(base_info)
    assert result.product_id == "1998527297"


def test_validate_gog_install_accepts_data_dir_input(tmp_path):
    root = _gog_fo4_root(tmp_path)
    _write_info(root)

    result = validate_gog_install_for_game("fo4", str(root / "Data"))

    assert result.ok is True
    assert result.root_dir == str(root)


def test_validate_gog_install_accepts_integer_game_id(tmp_path):
    root = _gog_fo4_root(tmp_path)
    payload = {
        "gameId": 1998527297,
        "playTasks": [{"path": "Fallout4.exe", "type": "FileTask"}],
    }
    (root / "goggame-1998527297.info").write_text(
        json.dumps(payload), encoding="utf-8"
    )

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.product_id == "1998527297"


def test_validate_gog_install_rejects_missing_info_manifest(tmp_path):
    root = _gog_fo4_root(tmp_path)

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is False
    assert result.local_install_valid is True
    assert result.info_present is False
    assert result.message == (
        "No GOG goggame-*.info manifest was found in the Fallout 4 folder."
    )


def test_validate_gog_install_rejects_malformed_info_manifest(tmp_path):
    root = _gog_fo4_root(tmp_path)
    (root / "goggame-1998527297.info").write_text("{not json", encoding="utf-8")

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is False
    assert result.info_present is True
    assert result.info_parsed is False
    assert "could not be read or has no gameId" in result.message


def test_validate_gog_install_rejects_info_without_game_id(tmp_path):
    root = _gog_fo4_root(tmp_path)
    (root / "goggame-1998527297.info").write_text(
        json.dumps({"name": "Fallout 4"}), encoding="utf-8"
    )

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is False
    assert result.info_parsed is False


def test_validate_gog_install_rejects_play_task_exe_not_in_folder(tmp_path):
    root = _gog_fo4_root(tmp_path)
    _write_info(root, play_task_path="NotHere.exe")

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is False
    assert result.info_parsed is True
    assert result.play_task_present is False
    assert "does not launch any executable" in result.message


def test_validate_gog_install_rejects_gog_artifacts_for_wrong_game(tmp_path):
    root = _gog_fo4_root(tmp_path)
    _write_info(root)

    result = validate_gog_install_for_game("fo76", str(root))

    assert result.ok is False
    assert result.local_install_valid is False
    assert "install is invalid" in result.message


def test_validate_gog_install_fails_before_gog_checks_when_local_install_invalid(tmp_path):
    result = validate_gog_install_for_game("fo4", str(tmp_path / "Fallout 4"))

    assert result.ok is False
    assert result.local_install_valid is False
    assert result.info_present is False
    assert "install is invalid" in result.message
