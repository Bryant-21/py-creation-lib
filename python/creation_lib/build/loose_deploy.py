"""Deploy mod assets as loose files instead of packing them into BA2 archives.

Keeps a manifest (`.loose_manifest.json` inside the mod folder) that lists every
file the mod pushed into the game's Data directory. The manifest lets us:

  * undeploy exactly what was deployed (even if the mod folder changed since)
  * re-import changes made in the Creation Kit — including brand-new files the
    CK dropped inside dirs the mod owns (e.g. new textures under
    ``Textures/B21_MyMod/``).
"""
from __future__ import annotations

import datetime
import json
import logging
import os
import shutil
import tempfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from creation_lib.build.deployer import compile_papyrus
from creation_lib.esp.validate import validate_authoring
from creation_lib.build.packer import _prepare_texture_root
from creation_lib.esp.authoring import deserialize, get_plugin_ext

_log = logging.getLogger(__name__)

MANIFEST_NAME = ".loose_manifest.json"

# Source sub-folders inside a mod that contribute loose assets, mapped to their
# destination prefix inside the game Data directory.
_SOURCE_ROOTS: list[tuple[str, str]] = [
    ("data", ""),
    ("Meshes", "Meshes"),
    ("MCM", "MCM"),
]


@dataclass
class LooseDeployResult:
    plugin: str = ""
    files_deployed: int = 0
    claimed_dirs: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class _LooseCopyJob:
    source: Path
    dest: Path
    rel: str
    src_root: str
    src_rel: str


# ---------------------------------------------------------------------------
# Manifest I/O
# ---------------------------------------------------------------------------

def _manifest_path(mod_dir: Path) -> Path:
    return mod_dir / MANIFEST_NAME


def _read_manifest(mod_dir: Path) -> dict | None:
    p = _manifest_path(mod_dir)
    if not p.is_file():
        return None
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        _log.warning("Failed to read loose manifest %s: %s", p, e)
        return None


def _write_manifest(mod_dir: Path, data: dict) -> None:
    _manifest_path(mod_dir).write_text(
        json.dumps(data, indent=2, sort_keys=True), encoding="utf-8"
    )


def _file_stat(path: Path) -> dict:
    st = path.stat()
    return {"mtime": st.st_mtime, "size": st.st_size}


def _has_files(directory: Path) -> bool:
    """Return True if a tree contains at least one file."""
    if not directory.is_dir():
        return False
    for path in directory.rglob("*"):
        if path.is_file():
            return True
    return False


def _resolve_copy_workers(workers: int, file_count: int) -> int:
    if file_count <= 0:
        return 0
    requested = int(workers or 0)
    if requested <= 0:
        requested = max(1, (os.cpu_count() or 2) // 2)
    return min(file_count, requested)


def _copy_loose_job(job: _LooseCopyJob) -> dict:
    job.dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(job.source, job.dest)
    return {
        "rel": job.rel,
        "src_root": job.src_root,
        "src_rel": job.src_rel,
        **_file_stat(job.dest),
    }


def _copy_loose_jobs(jobs: list[_LooseCopyJob], workers: int) -> list[dict]:
    worker_count = _resolve_copy_workers(workers, len(jobs))
    if worker_count <= 1:
        return [_copy_loose_job(job) for job in jobs]
    with ThreadPoolExecutor(max_workers=worker_count) as executor:
        futures = [executor.submit(_copy_loose_job, job) for job in jobs]
        return [future.result() for future in futures]


# ---------------------------------------------------------------------------
# Source walking + claim logic
# ---------------------------------------------------------------------------

def _iter_source_files(mod_dir: Path):
    """Yield (src_root_name, src_rel, abs_path, dest_rel_posix) for every loose asset.

    ``src_rel`` is the file's path relative to its source root inside the mod.
    ``dest_rel_posix`` is where that file lands under the game Data directory.
    """
    for src_root_name, dest_prefix in _SOURCE_ROOTS:
        src_root = mod_dir / src_root_name
        if not src_root.is_dir():
            continue
        for abs_path in src_root.rglob("*"):
            if not abs_path.is_file():
                continue
            # Behavior XML sources are packed into .hkx in place; don't deploy the .xml.
            if src_root_name == "Meshes" and abs_path.suffix.lower() == ".xml":
                continue
            rel = abs_path.relative_to(src_root)
            # Texture files are handled separately so PC resize limits can be applied.
            if src_root_name == "data" and rel.parts and rel.parts[0].lower() == "textures":
                continue
            dest_rel = Path(dest_prefix) / rel if dest_prefix else rel
            yield src_root_name, rel, abs_path, dest_rel.as_posix()


def _collect_claimed_dirs(deployed_rel_paths: list[str], mod_name: str) -> list[str]:
    """Return the set of game-Data-relative dirs this mod owns exclusively.

    A directory is "claimed" when one of its path segments contains the mod
    name (e.g. ``Textures/B21_MyMod``). Claimed dirs are scanned on import for
    files the Creation Kit added that aren't in the manifest yet. Plain shared
    top-level dirs (``Textures``, ``Meshes``, ``Scripts``) are never claimed on
    their own.
    """
    claimed: set[str] = set()
    for rel in deployed_rel_paths:
        parts = rel.replace("\\", "/").split("/")
        # Walk segments; as soon as we hit one that includes the mod name and
        # we've accumulated at least one parent (avoids claiming bare top level),
        # record that path as a claim root and stop.
        accum: list[str] = []
        for seg in parts[:-1]:  # exclude filename
            accum.append(seg)
            if mod_name in seg and len(accum) >= 2:
                claimed.add("/".join(accum))
                break
    return sorted(claimed)


def _map_entry_to_source(mod_dir: Path, entry: dict) -> Path:
    """Given a manifest entry, return the source path inside the mod folder."""
    src_root = entry.get("src_root") or ""
    src_rel = entry.get("src_rel") or entry["rel"]
    if not src_root:
        # File lives at the mod root (e.g. the .esp/.esl itself).
        return mod_dir / src_rel
    return mod_dir / src_root / src_rel


def _infer_source_for_new(mod_dir: Path, rel: str) -> tuple[str, str]:
    """Decide which source folder a newly-discovered game file should go back to.

    Prefers ``Meshes/`` when the file lives under ``Meshes/`` in the game AND
    the mod already uses a top-level ``Meshes/`` source folder. Everything else
    mirrors into ``data/``.
    """
    parts = rel.replace("\\", "/").split("/")
    if parts[0].lower() == "meshes" and (mod_dir / "Meshes").is_dir():
        return "Meshes", "/".join(parts[1:])
    return "data", rel


def _copy_tree_to_game(
    source_root: Path,
    dest_root: Path,
    *,
    src_root_name: str,
    src_rel_prefix: str = "",
    workers: int = 0,
) -> list[dict]:
    """Copy a tree into game Data and return manifest entries for the files."""
    if not source_root.is_dir():
        return []

    jobs: list[_LooseCopyJob] = []
    for abs_path in source_root.rglob("*"):
        if not abs_path.is_file():
            continue
        rel = abs_path.relative_to(source_root).as_posix()
        dest = dest_root / rel
        src_rel = f"{src_rel_prefix}/{rel}" if src_rel_prefix else rel
        jobs.append(
            _LooseCopyJob(
                source=abs_path,
                dest=dest,
                rel=src_rel,
                src_root=src_root_name,
                src_rel=src_rel,
            )
        )
    return _copy_loose_jobs(jobs, workers)


# ---------------------------------------------------------------------------
# Deploy
# ---------------------------------------------------------------------------

def deploy_loose_assets(
    mod_name: str,
    *,
    game: str,
    game_data_dir: Path,
    deploy_data_dir: Path | None = None,
    skip_build: bool = False,
    skip_validation: bool = False,
    skip_papyrus_compile: bool = False,
    pc_max_res: int = 0,
    pc_effects_max_res: int | None = None,
    workers: int = 0,
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> LooseDeployResult:
    """Build the ESP and copy all mod assets to the game Data dir as loose files.

    Writes ``<mod_dir>/.loose_manifest.json`` so a later undeploy/import knows
    exactly what was placed in the game folder.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    project_root = Path(project_root)
    game_data_dir = Path(game_data_dir)
    target_data_dir = Path(deploy_data_dir) if deploy_data_dir is not None else game_data_dir
    mod_dir = project_root / "mods" / mod_name
    if not mod_dir.is_dir():
        raise FileNotFoundError(f"Mod directory not found: {mod_dir}")

    result = LooseDeployResult()
    if pc_effects_max_res is None:
        pc_effects_max_res = pc_max_res

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    plugin_ext = get_plugin_ext(mod_dir)
    esp = mod_dir / f"{mod_name}.{plugin_ext}"

    # ── Step 1: build .esp ──
    if not skip_build and (mod_dir / "yaml").is_dir():
        if not skip_validation:
            _emit("[1/4] Validating authoring dir...")
            errors, _ = validate_authoring(mod_dir / "yaml")
            if errors:
                _emit(f"WARNING: Validation found {len(errors)} error(s)")
                for err in errors:
                    _emit(
                        f"VALIDATION_ERROR: {err['file']}:{err['line']} "
                        f"[{err['formkey']}] — {err['reason']}"
                    )
        _emit("[1/4] Building .esp...")
        deserialize(
            mod_dir / "yaml", esp,
            game=game, data_folder=game_data_dir,
            on_progress=on_progress,
        )
    else:
        _emit("[1/4] Skipping .esp build")

    if not esp.is_file():
        raise FileNotFoundError(f"{esp} not found. Build the mod first.")

    # ── Step 2: pack behavior XMLs → HKX (in the mod folder) ──
    meshes_dir = mod_dir / "Meshes"
    if meshes_dir.is_dir():
        xml_files = list(meshes_dir.rglob("*.xml"))
        if xml_files:
            _emit(f"[2/4] Packing {len(xml_files)} behavior XML(s) -> HKX...")
            from creation_lib._native.havok_native import pack_xml_to_hkx
            pack_failed = False
            for xmlfile in xml_files:
                hkxfile = xmlfile.with_suffix(".hkx")
                relpath = xmlfile.relative_to(meshes_dir)
                try:
                    pack_xml_to_hkx(str(xmlfile), str(hkxfile))
                    _emit(f"  Packed {relpath} OK")
                except Exception as e:
                    _emit(f"  FAILED {relpath}: {e}")
                    pack_failed = True
            if pack_failed:
                raise RuntimeError(
                    "One or more behavior XMLs failed to pack. "
                    "Fix the errors, then re-run deploy."
                )

    # ── Step 3: compile Papyrus (.psc → .pex into mod/data/Scripts) ──
    if skip_papyrus_compile:
        _emit("[3/4] Skipping Papyrus compile")
    else:
        _emit("[3/4] Compiling Papyrus scripts...")
        compiled = compile_papyrus(mod_dir, game, game_data_dir, on_progress=on_progress)
        if compiled == 0:
            _emit("  (no .psc sources)")

    # ── Step 4: copy plugin + all loose assets ──
    _emit(f"[4/4] Copying loose assets -> {target_data_dir}")
    target_data_dir.mkdir(parents=True, exist_ok=True)
    deployed: list[dict] = []

    # Plugin
    dest_esp = target_data_dir / f"{mod_name}.{plugin_ext}"
    shutil.copy2(esp, dest_esp)
    deployed.append({
        "rel": dest_esp.name,
        "src_root": "",
        "src_rel": esp.name,
        **_file_stat(dest_esp),
    })
    result.plugin = dest_esp.name
    _emit(f"  Copied plugin: {dest_esp.name}")

    # Strings/ + data/ + Meshes/ trees, excluding data/Textures so we can resize those first.
    copy_jobs: list[_LooseCopyJob] = []

    # Strings/
    strings_dir = mod_dir / "Strings"
    if strings_dir.is_dir():
        for srcfile in strings_dir.rglob("*"):
            if not srcfile.is_file():
                continue
            copy_jobs.append(
                _LooseCopyJob(
                    source=srcfile,
                    dest=target_data_dir / "Strings" / srcfile.name,
                    rel=f"Strings/{srcfile.name}",
                    src_root="Strings",
                    src_rel=srcfile.name,
                )
            )

    for src_root_name, src_rel, abs_path, dest_rel in _iter_source_files(mod_dir):
        copy_jobs.append(
            _LooseCopyJob(
                source=abs_path,
                dest=target_data_dir / dest_rel,
                rel=dest_rel,
                src_root=src_root_name,
                src_rel=src_rel.as_posix(),
            )
        )
    if copy_jobs:
        worker_count = _resolve_copy_workers(workers, len(copy_jobs))
        _emit(f"  Copying {len(copy_jobs)} loose file(s) with {worker_count} worker(s)...")
        deployed.extend(_copy_loose_jobs(copy_jobs, workers))

    # data/Textures/ tree with optional PC resize limits applied.
    texture_src_dir = mod_dir / "data" / "Textures"
    if texture_src_dir.is_dir() and _has_files(texture_src_dir):
        if pc_max_res > 0 or (pc_effects_max_res or 0) > 0:
            _emit(
                "  Resizing loose textures for PC deploy "
                f"(max {pc_max_res}px, effects {pc_effects_max_res}px)..."
            )
            with tempfile.TemporaryDirectory(prefix="modkit_loose_") as tmp:
                stage_root = Path(tmp) / "data"
                _prepare_texture_root(
                    str(texture_src_dir),
                    str(stage_root),
                    pc_max_res,
                    pc_effects_max_res or 0,
                )
                deployed.extend(
                    _copy_tree_to_game(
                        stage_root / "Textures",
                        target_data_dir / "Textures",
                        src_root_name="data",
                        src_rel_prefix="Textures",
                        workers=workers,
                    )
                )
        else:
            deployed.extend(
                _copy_tree_to_game(
                    texture_src_dir,
                    target_data_dir / "Textures",
                    src_root_name="data",
                    src_rel_prefix="Textures",
                    workers=workers,
                )
            )

    result.files_deployed = len(deployed)
    claimed = _collect_claimed_dirs([d["rel"] for d in deployed], mod_name)
    result.claimed_dirs = claimed

    manifest = {
        "mod_name": mod_name,
        "game": game,
        "game_data_dir": str(target_data_dir),
        "plugin": result.plugin,
        "deployed_at": datetime.datetime.now().isoformat(timespec="seconds"),
        "files": sorted(deployed, key=lambda e: e["rel"]),
        "claimed_dirs": claimed,
    }
    _write_manifest(mod_dir, manifest)
    _emit(f"  Wrote manifest: {MANIFEST_NAME}")
    _emit(
        f"=== Loose deploy complete: "
        f"{len(deployed)} file(s), {len(claimed)} claimed dir(s) ==="
    )
    return result


# ---------------------------------------------------------------------------
# Undeploy
# ---------------------------------------------------------------------------

def undeploy_loose_assets(
    mod_name: str,
    *,
    game_data_dir: Path | None = None,
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> list[str]:
    """Remove every file recorded in the mod's loose manifest from the game.

    ``game_data_dir`` overrides the path stored in the manifest if supplied.
    The manifest file itself is deleted after a successful undeploy.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    project_root = Path(project_root)
    mod_dir = project_root / "mods" / mod_name

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    manifest = _read_manifest(mod_dir)
    if not manifest:
        _emit(f"No loose manifest for {mod_name} — nothing to undeploy")
        return []

    target = game_data_dir or Path(manifest.get("game_data_dir", ""))
    if not target.is_dir():
        raise FileNotFoundError(f"game_data_dir not found: {target}")

    removed: list[str] = []
    dirs_touched: set[Path] = set()
    for entry in manifest.get("files", []):
        rel = entry["rel"]
        f = target / rel
        if f.is_file():
            try:
                f.unlink()
                removed.append(rel)
                dirs_touched.add(f.parent)
                _emit(f"  Removed: {rel}")
            except OSError as e:
                _emit(f"  FAILED: {rel}: {e}")

    # Prune now-empty dirs bottom-up, but stop at the game Data root.
    for d in sorted(dirs_touched, key=lambda p: len(p.parts), reverse=True):
        cur = d
        try:
            while cur != target and cur.is_dir() and not any(cur.iterdir()):
                cur.rmdir()
                cur = cur.parent
        except OSError:
            pass

    _manifest_path(mod_dir).unlink(missing_ok=True)
    _emit(f"=== Loose undeploy complete === ({len(removed)} file(s) removed)")
    return removed


# ---------------------------------------------------------------------------
# Smart re-import
# ---------------------------------------------------------------------------

def import_loose_assets(
    mod_name: str,
    *,
    game_data_dir: Path | None = None,
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> dict:
    """Pull loose asset changes from the game Data folder back into the mod.

    Three passes:
      1. For each file tracked in the manifest, if its mtime/size changed in
         the game, copy it back to the correct mod source location.
      2. For every "claimed" directory (e.g. ``Textures/B21_MyMod``), scan for
         files not yet in the manifest — these are things the CK created —
         and copy them back into the mod, adding them to the manifest.
      3. Rewrite the manifest so tracked state matches the game Data folder.

    Returns a summary dict with counts for changed/new/missing files.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    project_root = Path(project_root)
    mod_dir = project_root / "mods" / mod_name

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    manifest = _read_manifest(mod_dir)
    if not manifest:
        raise FileNotFoundError(
            f"No loose manifest for {mod_name} — deploy loose assets first"
        )

    target = game_data_dir or Path(manifest.get("game_data_dir", ""))
    if not target.is_dir():
        raise FileNotFoundError(f"game_data_dir not found: {target}")

    entries_by_rel: dict[str, dict] = {
        e["rel"].replace("\\", "/"): dict(e) for e in manifest.get("files", [])
    }

    changed = 0
    new = 0
    missing = 0

    # ── 1. Update tracked files ──
    for rel, entry in list(entries_by_rel.items()):
        game_file = target / rel
        if not game_file.is_file():
            missing += 1
            _emit(f"  MISSING in game: {rel}")
            continue
        st = game_file.stat()
        if st.st_mtime != entry.get("mtime") or st.st_size != entry.get("size"):
            dest = _map_entry_to_source(mod_dir, entry)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(game_file, dest)
            entry["mtime"] = st.st_mtime
            entry["size"] = st.st_size
            changed += 1
            _emit(f"  Updated: {rel}")

    # ── 2. Scan claimed dirs for brand-new files ──
    for claim in manifest.get("claimed_dirs", []):
        claim_dir = target / claim
        if not claim_dir.is_dir():
            continue
        for game_file in claim_dir.rglob("*"):
            if not game_file.is_file():
                continue
            rel = game_file.relative_to(target).as_posix()
            if rel in entries_by_rel:
                continue
            src_root, src_rel = _infer_source_for_new(mod_dir, rel)
            dest = mod_dir / src_root / src_rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(game_file, dest)
            st = game_file.stat()
            entries_by_rel[rel] = {
                "rel": rel,
                "src_root": src_root,
                "src_rel": src_rel,
                "mtime": st.st_mtime,
                "size": st.st_size,
            }
            new += 1
            _emit(f"  New: {rel}")

    # ── 3. Persist updated manifest ──
    manifest["files"] = sorted(entries_by_rel.values(), key=lambda e: e["rel"])
    manifest["claimed_dirs"] = _collect_claimed_dirs(
        [e["rel"] for e in manifest["files"]], mod_name
    )
    manifest["imported_at"] = datetime.datetime.now().isoformat(timespec="seconds")
    _write_manifest(mod_dir, manifest)

    _emit(
        f"=== Loose import complete: "
        f"{changed} changed, {new} new, {missing} missing ==="
    )
    return {"changed": changed, "new": new, "missing": missing}
