"""Texture Upgrader: one mod's FO4 materials, textures and meshes to BGSM v22+, shipped the way the mod ships.

Loose outputs stay loose and archived ones go back into BA2s. With ``replace`` the mod itself is
updated (its BA2s repacked); otherwise everything lands in ``<mod>/upgraded`` (whole repacked
BA2s under their own names, so they swap in, and the changed loose files).
"""

from __future__ import annotations

import os
import shutil
import tempfile
from pathlib import Path

from creation_lib.ba2 import native_runtime as ba2
from creation_lib.material_tools import upgrade_v22

UPGRADED_DIR = "upgraded"
_PLUGIN_SUFFIXES = (".esm", ".esp", ".esl")


def default_workers() -> int:
    return max(1, min(8, (os.cpu_count() or 4) // 2))


def upgrade_mod(
    mod_dir: str | Path,
    game_data_dir: str | Path,
    *,
    replace: bool,
    workers: int | None = None,
    progress: upgrade_v22.ProgressCallback | None = None,
) -> dict:
    """Upgrade ``mod_dir`` with the game's Data as the read-only base; returns the upgrade report
    plus ``placed`` (loose count and repacked archives) and ``output_root``."""
    mod_dir = Path(mod_dir)
    if not mod_dir.is_dir():
        raise FileNotFoundError(f"mod folder not found: {mod_dir}")
    if not Path(game_data_dir).is_dir():
        raise FileNotFoundError(f"game Data folder not found: {game_data_dir}")
    output_root = mod_dir if replace else mod_dir / UPGRADED_DIR
    with tempfile.TemporaryDirectory(prefix=".texture_upgrade_", dir=mod_dir) as staging:
        options = upgrade_v22.UpgradeOptions(
            sources=[str(mod_dir)],
            base_sources=[str(game_data_dir)],
            output=staging,
            # FO4 metal colour lives only in the envmap cube; envmap mode carries it into `_r`.
            metal_mode="envmap",
            include_nifs=True,
            workers=workers or default_workers(),
        )
        report = upgrade_v22.run(options, progress)
        if report["result"].get("cancelled"):
            return report
        written = [key for key in report["result"]["written"] if (Path(staging) / key).is_file()]
        report["placed"] = place_outputs(Path(staging), written, mod_dir, output_root)
    report["output_root"] = str(output_root)
    return report


def place_outputs(staging: Path, keys: list[str], mod_dir: Path, output_root: Path) -> dict:
    """Loose when the mod has no BA2s or ships that file loose (loose overrides archives);
    otherwise into the archive already holding it, else the mod's GNRL/DX10 archive."""
    archives = sorted(p for p in mod_dir.iterdir() if p.is_file() and p.suffix.lower() == ".ba2")
    members = {path: {name.replace("\\", "/").lower(): name for name in ba2.list_archive(str(path)) or []} for path in archives}
    formats = {path: ba2.archive_info(str(path)) or {} for path in archives}

    loose: list[str] = []
    into: dict[Path, list[str]] = {}
    for key in keys:
        if not archives or (mod_dir / key).is_file():
            loose.append(key)
            continue
        owner = next((path for path in archives if key in members[path]), None)
        owner = owner or _archive_for(key, archives, formats, mod_dir)
        into.setdefault(owner, []).append(key)

    for key in loose:
        target = output_root / key
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(staging / key, target)
    repacked = []
    for archive, archive_keys in into.items():
        _repack(archive, archive_keys, staging, output_root / archive.name, formats.get(archive, {}), mod_dir)
        repacked.append(archive.name)
    return {"loose": len(loose), "archives": sorted(repacked)}


def _archive_for(key: str, archives: list[Path], formats: dict[Path, dict], mod_dir: Path) -> Path:
    want = "fo4_dx10" if key.startswith("textures/") else "fo4_gnrl"
    existing = [path for path in archives if formats[path].get("format") == want]
    if existing:
        return existing[0]
    plugin = next((p.stem for p in sorted(mod_dir.iterdir()) if p.suffix.lower() in _PLUGIN_SUFFIXES), mod_dir.name)
    path = mod_dir / f"{plugin} - {'Textures' if want == 'fo4_dx10' else 'Main'}.ba2"
    version = next((formats[p].get("version") for p in archives), 8)
    formats[path] = {"format": want, "version": version}
    return path


def _repack(archive: Path, keys: list[str], staging: Path, target: Path, info: dict, mod_dir: Path) -> None:
    textures = info.get("format") == "fo4_dx10"
    archive_type = ("fo4og" if info.get("version") == 1 else "fo4") + ("dds" if textures else "")
    with tempfile.TemporaryDirectory(prefix=".texture_upgrade_ba2_", dir=mod_dir) as work:
        if archive.is_file():
            ba2.extract_archive(str(archive), work)
        for key in keys:
            destination = Path(work) / key
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(staging / key, destination)
        target.parent.mkdir(parents=True, exist_ok=True)
        packed = Path(work).with_suffix(".ba2")
        ba2.pack_archive(work, str(packed), archive_type)
        os.replace(packed, target)
