import json
from pathlib import Path

import pytest

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

    # A DLC .info manifest without playTasks must be skipped in favor of the
    # base game's manifest rather than causing a false rejection.
    dlc_root = _gog_fo4_root(tmp_path / "dlc-case")
    _write_info(dlc_root, product_id="1000000001", play_task_path=None)
    base_info = _write_info(dlc_root, product_id="1998527297")

    dlc_result = validate_gog_install_for_game("fo4", str(dlc_root))

    assert dlc_result.ok is True
    assert dlc_result.info_path == str(base_info)
    assert dlc_result.product_id == "1998527297"


def test_validate_gog_install_accepts_backslash_play_task_path_and_data_dir_input(tmp_path):
    root = _gog_fo4_root(tmp_path)
    (root / "bin").mkdir()
    (root / "bin" / "launcher.exe").write_bytes(b"exe")
    _write_info(root, play_task_path=r"bin\launcher.exe")

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True

    data_dir_root = _gog_fo4_root(tmp_path / "data-dir-case")
    _write_info(data_dir_root)
    data_dir_result = validate_gog_install_for_game("fo4", str(data_dir_root / "Data"))
    assert data_dir_result.ok is True
    assert data_dir_result.root_dir == str(data_dir_root)


@pytest.mark.parametrize("encoding", ["utf-8-sig", "utf-8"])
@pytest.mark.parametrize("game_id_value", ["1998527297", 1998527297])
def test_validate_gog_install_accepts_info_encoding_and_game_id_variants(
    tmp_path, encoding, game_id_value
):
    root = _gog_fo4_root(tmp_path)
    payload = {
        "gameId": game_id_value,
        "playTasks": [{"path": "Fallout4.exe", "type": "FileTask"}],
    }
    (root / "goggame-1998527297.info").write_text(
        json.dumps(payload), encoding=encoding
    )

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.product_id == "1998527297"


def _setup_missing_info_manifest(tmp_path: Path) -> Path:
    return _gog_fo4_root(tmp_path)


def _setup_malformed_info_manifest(tmp_path: Path) -> Path:
    root = _gog_fo4_root(tmp_path)
    (root / "goggame-1998527297.info").write_text("{not json", encoding="utf-8")
    return root


def _setup_info_without_game_id(tmp_path: Path) -> Path:
    root = _gog_fo4_root(tmp_path)
    (root / "goggame-1998527297.info").write_text(
        json.dumps({"name": "Fallout 4"}), encoding="utf-8"
    )
    return root


def _setup_play_task_exe_not_in_folder(tmp_path: Path) -> Path:
    root = _gog_fo4_root(tmp_path)
    _write_info(root, play_task_path="NotHere.exe")
    return root


@pytest.mark.parametrize(
    ("setup_fn", "field", "message_substring", "extra_checks"),
    [
        (
            _setup_missing_info_manifest,
            "info_present",
            "No GOG goggame-*.info manifest was found in the Fallout 4 folder.",
            {"local_install_valid": True},
        ),
        (
            _setup_malformed_info_manifest,
            "info_parsed",
            "could not be read or has no gameId",
            {"info_present": True},
        ),
        (_setup_info_without_game_id, "info_parsed", None, {}),
        (
            _setup_play_task_exe_not_in_folder,
            "play_task_present",
            "does not launch any executable",
            {"info_parsed": True},
        ),
    ],
)
def test_validate_gog_install_rejects_invalid_manifests(
    tmp_path, setup_fn, field, message_substring, extra_checks
):
    root = setup_fn(tmp_path)

    result = validate_gog_install_for_game("fo4", str(root))

    assert result.ok is False
    assert getattr(result, field) is False
    if message_substring is not None:
        assert message_substring in result.message
    for extra_field, expected_value in extra_checks.items():
        assert getattr(result, extra_field) is expected_value


def test_validate_gog_install_rejects_wrong_game_and_missing_local_install(tmp_path):
    root = _gog_fo4_root(tmp_path / "wrong-game-case")
    _write_info(root)

    result = validate_gog_install_for_game("fo76", str(root))

    assert result.ok is False
    assert result.local_install_valid is False
    assert "install is invalid" in result.message

    missing_result = validate_gog_install_for_game("fo4", str(tmp_path / "Fallout 4"))

    assert missing_result.ok is False
    assert missing_result.local_install_valid is False
    assert missing_result.info_present is False
    assert "install is invalid" in missing_result.message
