from __future__ import annotations

import os
import shutil
from pathlib import Path
from unittest.mock import patch

import pytest

from creation_lib.ba2 import native_runtime
from creation_lib.build import packer


DEFAULT_FO4_XBOX_MOD = Path(
    r"N:\ModOrganizer\Fallout 4 - VC\mods\B21_PlasmaCaster"
)


def _require_native() -> None:
    try:
        native_runtime.load_native_module()
    except RuntimeError:
        pytest.skip("bsarchive_native extension not built — run maturin develop")


def _require_xbox_mod() -> Path:
    raw = os.environ.get("MODKIT_TEST_FO4_XBOX_MOD")
    path = Path(raw) if raw else DEFAULT_FO4_XBOX_MOD
    if not path.is_dir():
        pytest.skip(
            f"FO4 Xbox mod directory not available: {path} "
            "(set MODKIT_TEST_FO4_XBOX_MOD to override)"
        )
    return path


def _require_texture_tools() -> None:
    try:
        packer._find_xtexconv(Path(__file__).resolve().parents[2] / "resource")
    except FileNotFoundError as exc:
        pytest.skip(str(exc))


def _copy_source_mod(src_mod: Path, app_root: Path) -> str:
    mod_name = src_mod.name
    data_dir = app_root / "mods" / mod_name / "data"
    data_dir.mkdir(parents=True, exist_ok=True)

    for name in ["Materials", "Meshes", "SCRIPTS", "Sound", "textures"]:
        src = src_mod / name
        if src.is_dir():
            shutil.copytree(src, data_dir / name)

    achlist = src_mod / "archive.achlist"
    if achlist.is_file():
        shutil.copy2(achlist, app_root / "mods" / mod_name / achlist.name)

    for archive_name in [
        "B21_PlasmaCaster - Main_xbox.ba2",
        "B21_PlasmaCaster - Textures_xbox.ba2",
    ]:
        archive_path = src_mod / archive_name
        if archive_path.is_file():
            shutil.copy2(archive_path, app_root / "mods" / mod_name / archive_name)

    return mod_name


def test_pack_mod_native_matches_reference_xbox_archives(tmp_path: Path):
    _require_native()
    _require_texture_tools()
    source_mod = _require_xbox_mod()
    mod_name = _copy_source_mod(source_mod, tmp_path)

    with patch("creation_lib.build.packer._find_xtexconv", return_value=str(Path(__file__).resolve().parents[2] / "resource" / "xtexconv.exe")):
        packer.pack_mod(
            mod_name,
            pc=False,
            xbox=True,
            xbox_max_res=0,
            xbox_effects_max_res=0,
            game="fo4",
            use_archive2=False,
            project_root=tmp_path,
            resource_dir=Path(__file__).resolve().parents[2] / "resource",
        )

    for archive_name in [
        "B21_PlasmaCaster - Main_xbox.ba2",
        "B21_PlasmaCaster - Textures_xbox.ba2",
    ]:
        expected = (source_mod / archive_name).read_bytes()
        actual = (tmp_path / "mods" / mod_name / archive_name).read_bytes()
        assert actual == expected, (
            f"{archive_name} differs: expected {len(expected)} bytes, "
            f"got {len(actual)} bytes"
        )
