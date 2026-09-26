from pathlib import Path

import pytest

from creation_lib.core.steam_install import SteamInstallResult, validate_steam_install_for_game


def _steam_fo4_root(tmp_path: Path, *, with_steam_api: bool = True) -> Path:
    root = tmp_path / "SteamLibrary" / "steamapps" / "common" / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")
    if with_steam_api:
        (root / "steam_api64.dll").write_bytes(b"dll")
    return root


def _steam_fo76_root(tmp_path: Path, *, with_steam_api: bool = True) -> Path:
    root = tmp_path / "SteamLibrary" / "steamapps" / "common" / "Fallout76"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout76.exe").write_bytes(b"exe")
    (data / "SeventySix - Startup.ba2").write_bytes(b"ba2")
    if with_steam_api:
        (root / "steam_api64.dll").write_bytes(b"dll")
    return root


def _steam_fnv_root(tmp_path: Path, *, with_steam_api: bool = True) -> Path:
    root = tmp_path / "SteamLibrary" / "steamapps" / "common" / "Fallout New Vegas"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "FalloutNV.exe").write_bytes(b"exe")
    (data / "Fallout - Meshes.bsa").write_bytes(b"bsa")
    if with_steam_api:
        (root / "steam_api.dll").write_bytes(b"dll")
    return root


def _steam_fo3_root(tmp_path: Path) -> Path:
    root = tmp_path / "SteamLibrary" / "steamapps" / "common" / "Fallout 3 GOTY"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout3.exe").write_bytes(b"exe")
    (root / "steam_api.dll").write_bytes(b"dll")
    (data / "Fallout - Meshes.bsa").write_bytes(b"bsa")
    return root


def _write_manifest(
    root: Path,
    *,
    appid: int = 377160,
    installdir: str = "Fallout 4",
    in_common: bool = False,
) -> Path:
    manifest_dir = root.parent if in_common else root.parents[1]
    manifest = manifest_dir / f"appmanifest_{appid}.acf"
    manifest.write_text(
        "\n".join(
            (
                '"AppState"',
                "{",
                f'    "appid"        "{appid}"',
                f'    "installdir"   "{installdir}"',
                "}",
            )
        ),
        encoding="utf-8",
    )
    return manifest


def test_validate_steam_install_accepts_matching_steam_library_install(tmp_path):
    root = _steam_fo4_root(tmp_path)
    manifest = _write_manifest(root)

    result = validate_steam_install_for_game("fo4", str(root))

    assert result == SteamInstallResult(
        ok=True,
        game_id="fo4",
        app_id=377160,
        root_dir=str(root),
        local_install_valid=True,
        steam_layout_valid=True,
        steam_api_present=True,
        appmanifest_present=True,
        appmanifest_matches=True,
        steam_library_dir=str(tmp_path / "SteamLibrary"),
        appmanifest_path=str(manifest),
        message="Fallout 4 Steam install verified.",
    )


def test_validate_steam_install_accepts_manifest_variants(tmp_path):
    root = _steam_fo4_root(tmp_path)
    manifest = _write_manifest(root, in_common=True)

    result = validate_steam_install_for_game("fo4", str(root))

    assert result.ok is True
    assert result.appmanifest_present is True
    assert result.appmanifest_matches is True
    assert result.appmanifest_path == str(manifest)

    # A game_data_dir input resolves back to the game root.
    data_dir_root = _steam_fo4_root(tmp_path / "data-dir-case")
    _write_manifest(data_dir_root)
    data_dir_result = validate_steam_install_for_game("fo4", str(data_dir_root / "Data"))
    assert data_dir_result.ok is True
    assert data_dir_result.root_dir == str(data_dir_root)

    # A folder name with different spacing than the display name still matches
    # via the appmanifest's installdir.
    spacing_root = _steam_fo4_root(tmp_path / "spacing-case")
    spacing_root = spacing_root.rename(spacing_root.with_name("Fallout4"))
    spacing_manifest = _write_manifest(spacing_root, installdir="Fallout 4")
    (spacing_manifest.parent / "common" / "Fallout 4").mkdir()
    spacing_result = validate_steam_install_for_game("fo4", str(spacing_root))
    assert spacing_result.ok is True
    assert spacing_result.appmanifest_matches is True


@pytest.mark.parametrize(
    ("game_id", "root_factory", "appid", "installdir"),
    [
        ("fo76", _steam_fo76_root, 1151340, "Fallout76"),
        ("fnv", _steam_fnv_root, 22380, "Fallout New Vegas"),
        ("fo3", _steam_fo3_root, 22370, "Fallout 3 GOTY"),
    ],
)
def test_validate_steam_install_accepts_other_games(
    tmp_path, game_id, root_factory, appid, installdir
):
    root = root_factory(tmp_path)
    manifest = _write_manifest(root, appid=appid, installdir=installdir)

    result = validate_steam_install_for_game(game_id, str(root))

    assert result.ok is True
    assert result.steam_api_present is True
    assert result.appmanifest_path == str(manifest)


def _setup_non_steam_layout(tmp_path: Path) -> Path:
    root = tmp_path / "Fallout 4"
    data = root / "Data"
    data.mkdir(parents=True)
    (root / "Fallout4.exe").write_bytes(b"exe")
    (root / "steam_api64.dll").write_bytes(b"dll")
    (data / "Fallout4 - Main.ba2").write_bytes(b"ba2")
    return root


def _setup_missing_steam_api_dll(tmp_path: Path) -> Path:
    root = _steam_fo4_root(tmp_path, with_steam_api=False)
    _write_manifest(root)
    return root


def _setup_missing_appmanifest(tmp_path: Path) -> Path:
    return _steam_fo4_root(tmp_path)


def _setup_manifest_for_other_folder(tmp_path: Path) -> Path:
    root = _steam_fo4_root(tmp_path)
    _write_manifest(root, installdir="Other Folder")
    return root


def _message_is_non_steam_layout(result, root: Path) -> bool:
    return "steamapps\\common" in result.message


def _message_is_missing_steam_api_dll(result, root: Path) -> bool:
    return result.message == "Fallout 4 install is missing steam_api64.dll."


def _message_is_missing_appmanifest(result, root: Path) -> bool:
    return result.message == "Steam app manifest appmanifest_377160.acf was not found."


def _message_is_manifest_for_other_folder(result, root: Path) -> bool:
    expected_root = root.parent / "Other Folder"
    return result.message == (
        f"Steam app manifest expects Fallout 4 at:\n"
        f"{expected_root}\n"
        f"Selected folder:\n"
        f"{root}"
    )


def _setup_missing_local_install(tmp_path: Path) -> Path:
    return tmp_path / "Fallout 4"


def _message_is_missing_local_install(result, root: Path) -> bool:
    return "install is invalid" in result.message


@pytest.mark.parametrize(
    ("setup_fn", "field", "message_check"),
    [
        (_setup_non_steam_layout, "steam_layout_valid", _message_is_non_steam_layout),
        (_setup_missing_steam_api_dll, "steam_api_present", _message_is_missing_steam_api_dll),
        (_setup_missing_appmanifest, "appmanifest_present", _message_is_missing_appmanifest),
        (_setup_manifest_for_other_folder, "appmanifest_matches", _message_is_manifest_for_other_folder),
        (_setup_missing_local_install, "local_install_valid", _message_is_missing_local_install),
    ],
)
def test_validate_steam_install_rejects_invalid_installs(tmp_path, setup_fn, field, message_check):
    root = setup_fn(tmp_path)

    result = validate_steam_install_for_game("fo4", str(root))

    assert result.ok is False
    assert getattr(result, field) is False
    assert message_check(result, root)
