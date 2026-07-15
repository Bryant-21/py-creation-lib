"""Creation Kit automation — DLL safety dance and CK command-line operations.

ENB/F4SE DLLs crash the Creation Kit. This module provides a context manager
that temporarily renames interfering DLLs and restores them on exit, plus
wrappers for CK command-line operations (previs, dialogue export, anim data).

All functions accept explicit Path arguments. Currently FO4-only.
"""
from __future__ import annotations

import glob
import logging
import os
import shutil
import subprocess
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Generator, Iterable

_log = logging.getLogger(__name__)

# DLLs that interfere with Creation Kit
_DLL_BLOCKLIST = [
    "d3d11.dll", "d3d10.dll", "d3d9.dll", "dxgi.dll", "enbimgui.dll",
    "d3dcompiler_46e.dll", "IpHlpAPI.dll", "f4se_loader.exe",
]

_CK_TEMP_SUFFIX = "_CK_TEMP"


@dataclass(frozen=True)
class _DeploymentRecord:
    path: Path
    backup_path: Path | None = None


# ---------------------------------------------------------------------------
# DLL safety context manager
# ---------------------------------------------------------------------------

@contextmanager
def ck_safe_env(game_dir: Path) -> Generator[Path, None, None]:
    """Context manager: rename interfering DLLs, yield CK exe path, restore on exit.

    Usage::

        with ck_safe_env(game_dir) as ck_exe:
            subprocess.run([str(ck_exe), "-SomeCommand:arg"])
    """
    ck_exe = game_dir / "CreationKit.exe"
    if not ck_exe.is_file():
        raise FileNotFoundError(f"CreationKit.exe not found at {ck_exe}")

    renamed: list[str] = []

    # Build full DLL list including versioned f4se DLLs
    all_dlls = list(_DLL_BLOCKLIST)
    for f in game_dir.glob("f4se*.dll"):
        all_dlls.append(f.name)

    try:
        for dll in all_dlls:
            src = game_dir / dll
            if src.is_file():
                tmp = game_dir / f"{dll}{_CK_TEMP_SUFFIX}"
                try:
                    src.rename(tmp)
                    renamed.append(dll)
                    _log.debug("Renamed: %s → %s%s", dll, dll, _CK_TEMP_SUFFIX)
                except OSError as e:
                    _log.error("Cannot rename %s (file locked?): %s", dll, e)
                    # Restore already renamed before raising
                    for r in renamed:
                        t = game_dir / f"{r}{_CK_TEMP_SUFFIX}"
                        if t.is_file():
                            t.rename(game_dir / r)
                    raise RuntimeError(f"Cannot rename {dll}: {e}")

        _log.info("Renamed %d DLL(s) for CK safety", len(renamed))
        yield ck_exe

    finally:
        for dll in renamed:
            tmp = game_dir / f"{dll}{_CK_TEMP_SUFFIX}"
            if tmp.is_file():
                try:
                    tmp.rename(game_dir / dll)
                except OSError:
                    _log.warning("Failed to restore %s", dll)
        _log.info("Restored %d DLL(s)", len(renamed))


# ---------------------------------------------------------------------------
# Plugin extension helper (shared with other modules)
# ---------------------------------------------------------------------------

def _get_plugin_ext(mod_dir: Path) -> str:
    """Read plugin extension from the mod's ``plugin.yaml``."""
    from creation_lib.esp.authoring import get_plugin_ext
    return get_plugin_ext(mod_dir)


def _get_plugin_path(mod_dir: Path, mod_name: str) -> tuple[str, Path]:
    """Return (plugin_ext, esp_path) for a mod."""
    ext = _get_plugin_ext(mod_dir)
    esp = mod_dir / f"{mod_name}.{ext}"
    return ext, esp


def _validate_fo4_only(game: str, operation: str) -> None:
    """Raise if game is not fo4."""
    if game != "fo4":
        raise ValueError(f"{operation} is only supported for Fallout 4 (got: {game})")


# ---------------------------------------------------------------------------
# Export Dialogue
# ---------------------------------------------------------------------------

def export_dialogue(
    mod_name: str,
    *,
    game: str,
    game_dir: Path,
    game_data_dir: Path,
    mod_dir: Path,
    on_progress: Callable[[str], None] | None = None,
) -> Path | None:
    """Export dialogue lines via CK -ExportDialogue.

    Returns path to dialogue_export.txt or None if no dialogue produced.
    """
    _validate_fo4_only(game, "Dialogue export")

    plugin_ext, esp = _get_plugin_path(mod_dir, mod_name)
    if not esp.is_file():
        raise FileNotFoundError(f"{esp} not found. Build the mod first.")

    plugin_name = f"{mod_name}.{plugin_ext}"
    output_file = mod_dir / "dialogue_export.txt"

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    _emit(f"[1/3] Deploying {plugin_name} to {game_data_dir}...")
    shutil.copy2(esp, game_data_dir / plugin_name)

    try:
        _emit("[2/3] Renaming interfering DLLs...")
        with ck_safe_env(game_dir) as ck_exe:
            _emit("[3/3] Exporting dialogue...")
            result = subprocess.run(
                [str(ck_exe), f"-ExportDialogue:{output_file}"],
                cwd=str(game_dir),
                capture_output=True, text=True,
            )
            if result.returncode != 0:
                _log.warning("CK exited with code %d", result.returncode)
    finally:
        # Clean up deployed plugin
        deployed = game_data_dir / plugin_name
        if deployed.is_file():
            deployed.unlink()

    if output_file.is_file() and output_file.stat().st_size > 0:
        _emit(f"Dialogue export complete: {output_file}")
        return output_file
    elif output_file.is_file():
        _emit("Dialogue export file is empty — mod may have no dialogue records")
        return output_file
    else:
        _emit("No dialogue produced — mod may have no dialogue records")
        return None


# ---------------------------------------------------------------------------
# Generate Anim Data
# ---------------------------------------------------------------------------

def _file_bytes_equal(left: Path, right: Path) -> bool:
    if left.stat().st_size != right.stat().st_size:
        return False
    return left.read_bytes() == right.read_bytes()


def _deploy_file(src: Path, dst: Path) -> _DeploymentRecord | None:
    if dst.is_file() and _file_bytes_equal(src, dst):
        return None

    backup_path: Path | None = None
    if dst.is_file():
        with tempfile.NamedTemporaryFile(delete=False) as backup:
            backup_path = Path(backup.name)
        shutil.copy2(dst, backup_path)

    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    return _DeploymentRecord(dst, backup_path)


def _normalize_loose_root(root: str) -> str:
    return root.replace("\\", "/").strip("/").lower()


def _deploy_loose_data(
    mod_dir: Path,
    game_data_dir: Path,
    roots: Iterable[str] | None = None,
) -> list[_DeploymentRecord]:
    """Copy mod data/ assets into game Data/ as loose files.

    Replaces stale destination files temporarily so CK sees the current build.
    Cleanup restores pre-existing files and removes files that were missing.
    """
    deployed: list[_DeploymentRecord] = []
    data_dir = mod_dir / "data"
    if not data_dir.is_dir():
        return deployed
    normalized_roots = None
    if roots is not None:
        normalized_roots = tuple(
            root for root in (_normalize_loose_root(r) for r in roots) if root
        )
    for src in data_dir.rglob("*"):
        if not src.is_file():
            continue
        rel = src.relative_to(data_dir)
        rel_norm = rel.as_posix().lower()
        if normalized_roots is not None and not any(
            rel_norm == root or rel_norm.startswith(f"{root}/")
            for root in normalized_roots
        ):
            continue
        dst = game_data_dir / rel
        record = _deploy_file(src, dst)
        if record is not None:
            deployed.append(record)
    return deployed


def _cleanup_loose_data(deployed: list[_DeploymentRecord]) -> None:
    """Remove previously deployed loose files."""
    for record in reversed(deployed):
        f = record.path
        try:
            if record.backup_path is not None:
                if f.is_file():
                    f.unlink()
                shutil.move(str(record.backup_path), str(f))
            elif f.is_file():
                f.unlink()
        except OSError:
            _log.warning("Failed to remove deployed file: %s", f)


@contextmanager
def _temporarily_clear_directory(path: Path) -> Generator[None, None, None]:
    backup_path: Path | None = None
    if path.exists():
        backup_path = Path(tempfile.mkdtemp(prefix="modkit_ck_backup_")) / path.name
        shutil.move(str(path), str(backup_path))
    try:
        yield
    finally:
        if path.exists():
            shutil.rmtree(path)
        if backup_path is not None and backup_path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(backup_path), str(path))


def generate_anim_data(
    mod_name: str,
    *,
    game: str,
    game_dir: Path,
    game_data_dir: Path,
    mod_dir: Path,
    plugin_name: str | None = None,
    deploy_loose_data: bool = True,
    loose_data_roots: Iterable[str] | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> Path | None:
    """Generate AnimTextData via CK -GenerateAnimInfo.

    Deploys the mod's .esp and, by default, loose data/ assets so CK can
    resolve meshes and animations. Callers that already ran the standard
    pack-and-deploy pipeline can disable loose deployment so CK uses the
    deployed archives instead.

    Returns the path to the generated AnimTextData directory, or None if
    nothing was produced.
    """
    _validate_fo4_only(game, "Anim data generation")

    if plugin_name is not None:
        plugin_name = Path(plugin_name).name
        plugin_ext = Path(plugin_name).suffix.lower().lstrip(".")
        if plugin_ext not in {"esp", "esm", "esl"}:
            raise ValueError(
                f"Invalid plugin_name for AnimTextData generation: {plugin_name!r}"
            )
        esp = mod_dir / plugin_name
    else:
        plugin_ext, esp = _get_plugin_path(mod_dir, mod_name)
        plugin_name = f"{mod_name}.{plugin_ext}"
    if not esp.is_file():
        raise FileNotFoundError(f"{esp} not found. Build the mod first.")

    data_out = mod_dir / "data"
    data_out.mkdir(exist_ok=True)
    animtext_dest = data_out / "meshes" / "AnimTextData"
    game_animtext_dir = game_data_dir / "meshes" / "AnimTextData"

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    if animtext_dest.exists():
        shutil.rmtree(animtext_dest)
        _emit(f"Cleared stale AnimTextData output: {animtext_dest}")

    # Deploy plugin (skip if already in game Data from a prior deploy)
    deployed_plugin = game_data_dir / plugin_name
    plugin_deployment = _deploy_file(esp, deployed_plugin)
    if plugin_deployment is None:
        _emit(f"[1/4] {plugin_name} already in game Data — skipping plugin deploy")
    elif plugin_deployment.backup_path is not None:
        _emit(f"[1/4] Temporarily replacing stale {plugin_name} in {game_data_dir}...")
    else:
        _emit(f"[1/4] Deploying {plugin_name} to {game_data_dir}...")

    # Deploy loose data assets so CK can find meshes/animations when this
    # helper is called directly. Standard CLI animdata deploys archives first.
    deployed_files: list[_DeploymentRecord] = []
    if deploy_loose_data:
        _emit("[2/4] Deploying loose data assets...")
        deployed_files = _deploy_loose_data(
            mod_dir,
            game_data_dir,
            roots=loose_data_roots,
        )
        _emit(f"  Deployed {len(deployed_files)} file(s)")
    else:
        _emit("[2/4] Using already deployed archives — skipping loose assets")

    try:
        with _temporarily_clear_directory(game_animtext_dir):
            _emit("[3/4] Renaming interfering DLLs...")
            with ck_safe_env(game_dir) as ck_exe:
                _emit("[4/4] Generating anim data (CK -GenerateAnimInfo)...")
                result = subprocess.run(
                    [str(ck_exe), f"-GenerateAnimInfo:{plugin_name}",
                     str(game_data_dir), str(data_out), "--speed", "--stance"],
                    cwd=str(game_dir),
                    capture_output=True, text=True,
                )
                if result.returncode != 0:
                    _log.warning("CK exited with code %d", result.returncode)
                    if result.stderr:
                        _log.warning("CK stderr: %s", result.stderr[:2000])

        # CK writes AnimTextData directly into data_out via the output arg
        if animtext_dest.is_dir():
            count = sum(1 for f in animtext_dest.rglob("*") if f.is_file())
            _emit(f"AnimTextData generated: {count} file(s) in {animtext_dest}")
            return animtext_dest
        else:
            _emit("No AnimTextData was generated by CK")
            return None

    finally:
        if plugin_deployment is not None:
            _cleanup_loose_data([plugin_deployment])
        _cleanup_loose_data(deployed_files)


# ---------------------------------------------------------------------------
# PreVis Generation
# ---------------------------------------------------------------------------

def run_previs(
    mod_name: str,
    *,
    game: str,
    game_dir: Path,
    game_data_dir: Path,
    mod_dir: Path,
    clean_output: bool = True,
    on_progress: Callable[[str], None] | None = None,
) -> None:
    """Run CK's precombine/previs pipeline and merge generated ESP output."""
    _validate_fo4_only(game, "PreVis generation")

    plugin_ext, esp = _get_plugin_path(mod_dir, mod_name)
    if not esp.is_file():
        raise FileNotFoundError(f"{esp} not found. Build the mod first.")

    plugin_name = f"{mod_name}.{plugin_ext}"
    deployed_plugin = game_data_dir / plugin_name
    combined_esp = game_data_dir / "CombinedObjects.esp"
    previs_esp = game_data_dir / "PreVis.esp"
    cdx_file = game_data_dir / f"{mod_name}.cdx"
    geometry_psg = game_data_dir / f"{mod_name} - Geometry.psg"
    geometry_csg = game_data_dir / f"{mod_name} - Geometry.csg"
    previs_tmp = mod_dir / "previs_tmp"

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    def _run_ck(ck_exe: Path, args: list[str]) -> None:
        result = subprocess.run(
            [str(ck_exe), *args],
            cwd=str(game_dir),
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            _log.warning("CK command exited with code %d: %s", result.returncode, " ".join(args))
            if result.stderr:
                _log.warning("CK stderr: %s", result.stderr[:2000])

    def _snapshot_tree(root: Path) -> dict[str, tuple[int, int]]:
        if not root.is_dir():
            return {}
        snapshot: dict[str, tuple[int, int]] = {}
        for f in root.rglob("*"):
            if f.is_file():
                stat = f.stat()
                snapshot[str(f)] = (stat.st_size, stat.st_mtime_ns)
        return snapshot

    def _changed_since_snapshot(path: Path, snapshot: dict[str, tuple[int, int]]) -> bool:
        try:
            stat = path.stat()
        except FileNotFoundError:
            return False
        return snapshot.get(str(path)) != (stat.st_size, stat.st_mtime_ns)

    def _clean_mod_previs_outputs() -> None:
        for path in [
            mod_dir / "data" / "Meshes" / "PreCombined",
            mod_dir / "data" / "Vis",
        ]:
            if path.is_dir():
                shutil.rmtree(path)
        for path in [
            mod_dir / f"{mod_name}.cdx",
            mod_dir / f"{mod_name} - Geometry.psg",
            mod_dir / f"{mod_name} - Geometry.csg",
        ]:
            if path.is_file():
                path.unlink(missing_ok=True)

    # Step 1: Deploy
    _emit(f"[1/10] Deploying {plugin_name} to {game_data_dir}...")
    shutil.copy2(esp, deployed_plugin)

    # Step 2: Snapshot existing files
    _emit("[2/10] Snapshotting existing PreCombined/Vis files...")
    precomb_dir = game_data_dir / "Meshes" / "PreCombined"
    vis_dir = game_data_dir / "Vis"
    precomb_snapshot = _snapshot_tree(precomb_dir)
    vis_snapshot = _snapshot_tree(vis_dir)

    try:
        _emit("[3/10] Clearing stale CK previs outputs...")
        for generated in [combined_esp, previs_esp, cdx_file, geometry_psg, geometry_csg]:
            if generated.is_file():
                generated.unlink(missing_ok=True)
        if previs_tmp.is_dir():
            shutil.rmtree(previs_tmp)
        previs_tmp.mkdir(exist_ok=True)

        # Step 4: DLL safety
        _emit("[4/10] Renaming interfering DLLs...")
        with ck_safe_env(game_dir) as ck_exe:
            # Step 5: Generate and merge PreCombines before PSG/CDX/PreVis.
            _emit("[5/10] Generating PreCombines (this may take a while)...")
            _run_ck(ck_exe, [f"-GeneratePrecombined:{plugin_name}", "clean", "all"])
            if not combined_esp.is_file():
                raise RuntimeError("CK did not produce CombinedObjects.esp")
            if not geometry_psg.is_file():
                raise RuntimeError(f"CK did not produce {geometry_psg.name}")
            _emit("  CombinedObjects.esp generated")
            _emit(f"  {geometry_psg.name} generated")

            shutil.copy2(combined_esp, previs_tmp / "CombinedObjects.esp")
            _emit("[6/10] Merging precombine data into plugin...")
            from creation_lib.nif.previs_merge import merge_precombined
            merge_precombined(
                mod_name,
                game=game,
                game_dir=str(game_dir),
                mods_dir=mod_dir.parent,
            )
            shutil.copy2(esp, deployed_plugin)
            _emit("  Updated deployed plugin after precombine merge")

            _emit("[7/10] Compressing PSG...")
            _run_ck(ck_exe, [f"-CompressPSG:{plugin_name}"])
            if not geometry_csg.is_file():
                raise RuntimeError(f"CK did not produce {geometry_csg.name}")
            if geometry_psg.is_file():
                geometry_psg.unlink(missing_ok=True)
            _emit(f"  {geometry_csg.name} generated")

            _emit("[8/10] Building CDX...")
            _run_ck(ck_exe, [f"-BuildCDX:{plugin_name}"])
            if not cdx_file.is_file():
                raise RuntimeError(f"CK did not produce {cdx_file.name}")
            _emit(f"  {cdx_file.name} generated")

            _emit("[9/10] Generating PreVis data (this may take a while)...")
            _run_ck(ck_exe, [f"-GeneratePreVisData:{plugin_name}", "clean", "all"])
            if not previs_esp.is_file():
                raise RuntimeError("CK did not produce PreVis.esp")
            _emit("  PreVis.esp generated")

        # Step 10: Collect generated files
        _emit("[10/10] Collecting generated files...")
        if clean_output:
            _clean_mod_previs_outputs()
            _emit("  Cleaned existing mod previs outputs")

        # Precombined meshes
        precomb_dest = mod_dir / "data" / "Meshes" / "PreCombined"
        precomb_dest.mkdir(parents=True, exist_ok=True)
        precomb_count = 0
        if precomb_dir.is_dir():
            for f in precomb_dir.rglob("*"):
                if f.is_file() and _changed_since_snapshot(f, precomb_snapshot):
                    rel = f.relative_to(precomb_dir)
                    dest = precomb_dest / rel
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(f, dest)
                    precomb_count += 1
        _emit(f"  Collected {precomb_count} precombined mesh(es)")

        # Vis data
        vis_dest = mod_dir / "data" / "Vis"
        vis_dest.mkdir(parents=True, exist_ok=True)
        vis_count = 0
        if vis_dir.is_dir():
            for f in vis_dir.rglob("*"):
                if f.is_file() and _changed_since_snapshot(f, vis_snapshot):
                    rel = f.relative_to(vis_dir)
                    dest = vis_dest / rel
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(f, dest)
                    vis_count += 1
        _emit(f"  Collected {vis_count} vis file(s)")

        # CDX
        if cdx_file.is_file():
            shutil.copy2(cdx_file, mod_dir / f"{mod_name}.cdx")
            _emit(f"  Collected {mod_name}.cdx")

        # CSG
        if geometry_csg.is_file():
            shutil.copy2(geometry_csg, mod_dir / geometry_csg.name)
            _emit(f"  Collected {geometry_csg.name}")

        # CK-generated PreVis ESP for final merge
        shutil.copy2(previs_esp, previs_tmp / "PreVis.esp")
        _emit("  Collected PreVis.esp to previs_tmp/")

        # Final merge into the built plugin
        _emit("Merging previs data into plugin...")
        from creation_lib.nif.previs_merge import merge_previs
        merge_previs(
            mod_name,
            game=game,
            game_dir=str(game_dir),
            mods_dir=mod_dir.parent,
            include_combined=False,
            include_previs=True,
        )

        # Clean previs_tmp on success
        if previs_tmp.is_dir():
            shutil.rmtree(previs_tmp)
            _emit("Cleaned previs_tmp/")

    finally:
        # Cleanup game Data directory
        _log.info("Cleaning up game Data directory...")
        for name in [
            "CombinedObjects.esp",
            "PreVis.esp",
            plugin_name,
            f"{mod_name}.cdx",
            f"{mod_name} - Geometry.psg",
            f"{mod_name} - Geometry.csg",
        ]:
            f = game_data_dir / name
            if f.is_file():
                f.unlink(missing_ok=True)

        # Remove only our generated files from PreCombined/ and Vis/
        if precomb_dir.is_dir():
            for f in precomb_dir.rglob("*"):
                if f.is_file() and _changed_since_snapshot(f, precomb_snapshot):
                    f.unlink(missing_ok=True)
        if vis_dir.is_dir():
            for f in vis_dir.rglob("*"):
                if f.is_file() and _changed_since_snapshot(f, vis_snapshot):
                    f.unlink(missing_ok=True)

    _emit("PreVis generation complete")
