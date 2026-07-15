"""Mod scaffold — create, import, and migrate mod structures.

All functions accept explicit Path arguments.
"""
from __future__ import annotations

import concurrent.futures
import logging
import os
import shutil
from pathlib import Path
from typing import Callable

from creation_lib.core.game_profiles import get_profile
from creation_lib.esp.authoring import new_mod_yaml, serialize, get_plugin_ext

_log = logging.getLogger(__name__)

# Asset directories that mirror the game Data/ layout
_ASSET_DIRS = ["Materials", "Meshes", "Scripts", "Sound", "Textures"]

# Directories recognized as game data (case-insensitive comparison)
_DATA_DIR_NAMES = {
    "data", "meshes", "textures", "sound", "materials", "interface",
    "strings", "vis", "scripts",
}
_DEPLOYABLE_DATA_DIR_NAMES = _DATA_DIR_NAMES - {"data"}


# ---------------------------------------------------------------------------
# Parallel copy helpers
# ---------------------------------------------------------------------------

_COPY_WORKERS = min(8, (os.cpu_count() or 4))


def _collect_copy_pairs(src: Path, dst: Path) -> list[tuple[Path, Path]]:
    """Walk src and return (src_file, dst_file) pairs, pre-creating dst directories."""
    pairs: list[tuple[Path, Path]] = []
    for f in src.rglob("*"):
        if f.is_file():
            target = dst / f.relative_to(src)
            target.parent.mkdir(parents=True, exist_ok=True)
            pairs.append((f, target))
    return pairs


def _parallel_copy(
    pairs: list[tuple[Path, Path]],
    *,
    on_progress: Callable[[str], None] | None = None,
    label: str = "",
) -> int:
    """Copy (src, dst) file pairs using a thread pool. Returns count copied."""
    if not pairs:
        return 0
    total = len(pairs)
    done = 0
    report_every = max(1, total // 10)   # emit ~10 progress updates

    def _copy_one(pair: tuple[Path, Path]) -> None:
        shutil.copy2(pair[0], pair[1])

    with concurrent.futures.ThreadPoolExecutor(max_workers=_COPY_WORKERS) as pool:
        futures = {pool.submit(_copy_one, p): p for p in pairs}
        for future in concurrent.futures.as_completed(futures):
            try:
                future.result()
            except Exception as e:
                _log.warning("Failed to copy %s: %s", futures[future][0].name, e)
            done += 1
            if on_progress and label and done % report_every == 0:
                on_progress(f"      {label}: {done}/{total} files...")

    return done


def _collect_deployable_asset_pairs(src: Path, dst: Path) -> list[tuple[Path, Path]]:
    """Collect file copy pairs for a deployable game-data directory.

    `Scripts/Source` is source-only and must stay outside `data/`.
    Likewise, `.psc` sources should not be copied into the deployed
    `data/Scripts` tree.
    """
    pairs: list[tuple[Path, Path]] = []
    root_name = src.name.lower()
    for f in src.rglob("*"):
        if not f.is_file():
            continue
        rel = f.relative_to(src)
        rel_parts = {part.lower() for part in rel.parts}
        if root_name == "scripts":
            if "source" in rel_parts or f.suffix.lower() == ".psc":
                continue
        target = dst / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        pairs.append((f, target))
    return pairs


# ---------------------------------------------------------------------------
# Create mod
# ---------------------------------------------------------------------------

def create_mod(
    mod_name: str,
    *,
    game: str,
    mod_prefix: str = "B21",
    plugin_ext: str = "esl",
    init_git: bool = True,
    gitea_url: str = "",
    gitea_user: str = "",
    gitea_org: str = "",
    gitea_token: str = "",
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> Path:
    """Create a new mod with full directory structure + YAML scaffold.

    Args:
        mod_name: Mod name (e.g. "B21_MyMod").
        game: Game ID.
        mod_prefix: Required prefix for mod names.
        plugin_ext: "esl", "esp", or "esm".
        init_git: Whether to initialize a git repo.
        gitea_url/gitea_user/gitea_org/gitea_token: Gitea config for auto-repo.
        on_progress: Progress callback.

    Returns:
        Path to the created mod directory.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    project_root = Path(project_root)
    mod_dir = project_root / "mods" / mod_name

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    # Validate prefix
    if mod_prefix and not mod_name.startswith(f"{mod_prefix}_"):
        raise ValueError(f"Mod name must start with {mod_prefix}_ (got: {mod_name})")

    if mod_dir.is_dir():
        raise FileExistsError(f"{mod_dir} already exists")

    # Create YAML scaffold
    new_mod_yaml(
        mod_name, mod_dir, game=game,
        plugin_ext=plugin_ext, mod_prefix=mod_prefix,
    )

    # Write .game file
    (mod_dir / ".game").write_text(game, encoding="utf-8")

    # Create asset directories
    for dirname in _ASSET_DIRS:
        (mod_dir / "data" / dirname).mkdir(parents=True, exist_ok=True)

    # Papyrus source directory
    (mod_dir / "Scripts" / "Source" / "User").mkdir(parents=True, exist_ok=True)

    _emit(f"=== Mod structure created ({game}) ===")
    _emit(f"{mod_dir}/")

    # Git + Gitea
    if init_git and gitea_url and gitea_user:
        from creation_lib.mod.git_ops import gitea_init
        try:
            gitea_init(
                mod_dir, mod_name, game=game,
                gitea_url=gitea_url, gitea_user=gitea_user,
                gitea_org=gitea_org, gitea_token=gitea_token,
                on_progress=on_progress,
            )
        except Exception as e:
            _emit(f"WARNING: Git/Gitea setup failed — {e}")
    elif init_git:
        _emit("NOTE: Skipping Gitea repo creation — GITEA_URL or GITEA_USER not set.")

    return mod_dir


# ---------------------------------------------------------------------------
# Migrate (import external mod)
# ---------------------------------------------------------------------------

def migrate_mod(
    source_dir: Path,
    *,
    mod_name: str | None = None,
    game: str,
    game_dir: str = "",
    mod_prefix: str = "B21",
    init_git: bool = True,
    gitea_url: str = "",
    gitea_user: str = "",
    gitea_org: str = "",
    gitea_token: str = "",
    project_root: Path | str | None = None,
    on_progress: Callable[[str], None] | None = None,
) -> Path:
    """Import an external mod into the project's mods/ folder.

    Handles flat and Data/ subdirectory layouts.

    Args:
        source_dir: Path to the external mod folder.
        mod_name: Override mod name (defaults to source folder basename).
        game: Game ID.
        mod_prefix: Required prefix.
        init_git: Initialize git repo.
        on_progress: Progress callback.

    Returns:
        Path to the created mod directory.
    """
    if project_root is None:
        raise ValueError("project_root is required")
    project_root = Path(project_root)
    source_dir = Path(source_dir)

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    if not source_dir.is_dir():
        raise FileNotFoundError(f"Source directory not found: {source_dir}")

    # Derive mod name
    if not mod_name:
        mod_name = source_dir.name

    # Validate prefix
    if mod_prefix and not mod_name.startswith(f"{mod_prefix}_"):
        raise ValueError(
            f"Mod name must start with {mod_prefix}_ (got: {mod_name}). "
            f"Pass an explicit name."
        )

    mod_dir = project_root / "mods" / mod_name
    if mod_dir.is_dir():
        raise FileExistsError(f"{mod_dir} already exists. Remove it first to re-migrate.")

    profile = get_profile(game)

    _emit(f"=== Migrating: {mod_name} ===")
    _emit(f"  Source: {source_dir}")
    _emit(f"  Target: {mod_dir}")

    # Step 1: Find plugin(s) or YAML source
    main_esp, patch_esps, asset_root = _find_plugins(source_dir, mod_name)
    esp_path = main_esp
    yaml_src = _find_yaml_dir(source_dir, mod_name)

    asset_only = not esp_path and not yaml_src

    if not asset_only:
        from creation_lib.esp.native_runtime import supported_games_native
        if game not in supported_games_native():
            raise ValueError(f"Game '{game}' is not supported by the native ESP pipeline")

    has_patches = len(patch_esps) > 0
    do_git = init_git and gitea_url and gitea_user
    total_steps = (5 if has_patches else 4) + (1 if do_git else 0)

    if esp_path:
        _emit(f"[1/{total_steps}] Found plugin: {esp_path}")
        if has_patches:
            _emit(f"      Found {len(patch_esps)} patch plugin(s):")
            for pp in patch_esps:
                _emit(f"        - {pp.name}")
    elif yaml_src:
        _emit(f"[1/{total_steps}] No plugin found — YAML-only mod: {yaml_src}")
    else:
        _emit(f"[1/{total_steps}] No plugin or YAML found — asset-only mod")

    # Step 2: Create mod structure
    _emit(f"[2/{total_steps}] Creating mod structure...")
    mod_dir.mkdir(parents=True)
    (mod_dir / "yaml").mkdir()
    (mod_dir / ".game").write_text(game, encoding="utf-8")
    (mod_dir / "data").mkdir()
    (mod_dir / "Scripts" / "Source" / "User").mkdir(parents=True)
    # The native YAML scaffold is written by serialize() when esp_path is set,
    # or by copying the source yaml/ directly. Asset-only mods leave yaml/ empty.

    # Step 3: Serialize .esp → YAML, or copy existing YAML, or skip for asset-only
    if esp_path:
        _emit(f"[3/{total_steps}] Serializing .esp → YAML...")
        data_folder = Path(game_dir) / "Data" if game_dir else None

        serialize(
            esp_path, mod_dir,
            game=game,
            data_folder=data_folder if data_folder and data_folder.is_dir() else None,
            on_progress=on_progress,
        )
        _emit(f"      Serialized to: {mod_dir / 'yaml'}")
    elif yaml_src:
        _emit(f"[3/{total_steps}] Copying existing YAML...")
        yaml_dst = mod_dir / "yaml"
        pairs = _collect_copy_pairs(yaml_src, yaml_dst)
        _parallel_copy(pairs)
        _emit(f"      Copied from: {yaml_src}")
    else:
        _emit(f"[3/{total_steps}] Skipping YAML — asset-only mod")

    # Step 3b: Serialize patch plugins
    if has_patches and esp_path:
        _emit(f"[4/{total_steps}] Serializing {len(patch_esps)} patch plugin(s)...")
        patches_dir = mod_dir / "patches"
        patches_dir.mkdir(exist_ok=True)

        for patch_esp in patch_esps:
            patch_stem = patch_esp.stem
            patch_parent = patches_dir / patch_stem
            patch_parent.mkdir(parents=True, exist_ok=True)

            _emit(f"      Serializing patch: {patch_esp.name} → patches/{patch_stem}/yaml/")

            try:
                # serialize() creates output_dir/yaml/ automatically
                # Patches often require third-party masters that may not be
                # installed, so disable ErrorOnUnknown to avoid failures.
                serialize(
                    patch_esp, patch_parent,
                    game=game,
                    data_folder=data_folder if data_folder and data_folder.is_dir() else None,
                    error_on_unknown=False,
                    on_progress=on_progress,
                )
                _emit(f"      Done: patches/{patch_stem}/yaml/")
            except Exception as exc:
                _emit(f"      WARNING: Failed to serialize {patch_esp.name}: {exc}")

    # Step 4/5: Copy assets
    asset_step = 5 if has_patches else 4
    _emit(f"[{asset_step}/{total_steps}] Copying assets (parallel, {_COPY_WORKERS} threads)...")
    copied_files = 0
    psc_copied = 0

    # Collect all asset copy jobs up-front so we can run them in one pool pass
    all_pairs: list[tuple[Path, Path]] = []
    dir_labels: dict[str, int] = {}   # dir name → file count (for summary)

    asset_roots = [asset_root]
    if asset_root == source_dir and (source_dir / "Data").is_dir():
        asset_roots.append(source_dir / "Data")

    asset_roots_set = set(asset_roots)
    for ar in asset_roots:
        for item in ar.iterdir():
            if not item.is_dir() or item in asset_roots_set:
                continue
            if item.name.lower() not in _DEPLOYABLE_DATA_DIR_NAMES:
                continue
            dest = mod_dir / "data" / item.name
            if dest.exists():
                shutil.rmtree(dest)
            dest.mkdir(parents=True, exist_ok=True)
            pairs = _collect_deployable_asset_pairs(item, dest)
            if pairs:
                dir_labels[item.name] = len(pairs)
                all_pairs.extend(pairs)

    # Extra dirs (non-standard)
    extra_pairs: list[tuple[Path, Path]] = []
    for d in source_dir.iterdir():
        if not d.is_dir():
            continue
        if d.name.lower() in _DATA_DIR_NAMES:
            continue
        dest = mod_dir / d.name
        if not dest.exists():
            pairs = _collect_copy_pairs(d, dest)
            if pairs:
                dir_labels[f"{d.name} (extra)"] = len(pairs)
                extra_pairs.extend(pairs)

    all_pairs.extend(extra_pairs)

    # Papyrus sources (.psc) — small, include in the same pass
    psc_roots: list[Path] = []
    for root in [source_dir, asset_root]:
        if root not in psc_roots:
            psc_roots.append(root)
    for root in psc_roots:
        user_src = root / "Scripts" / "Source" / "User"
        source_src = root / "Scripts" / "Source"
        psc_src = user_src if user_src.is_dir() else source_src
        if psc_src.is_dir():
            psc_files = list(psc_src.rglob("*.psc"))
            if psc_files:
                dest_psc = mod_dir / "Scripts" / "Source" / "User"
                for psc in psc_files:
                    rel = psc.relative_to(psc_src)
                    dst_f = dest_psc / rel
                    dst_f.parent.mkdir(parents=True, exist_ok=True)
                    all_pairs.append((psc, dst_f))
                psc_copied += len(psc_files)

    if all_pairs:
        total_files = len(all_pairs)
        _emit(f"      {total_files} file(s) across {len(dir_labels)} director(ies)...")
        for dname, cnt in dir_labels.items():
            _emit(f"        {dname}: {cnt} file(s)")
        if psc_copied:
            _emit(f"        Papyrus sources (.psc): {psc_copied} file(s)")
        copied_files = _parallel_copy(all_pairs, on_progress=on_progress, label="copying")
        _emit(f"      Done — copied {copied_files} file(s)")
    else:
        _emit("      (no assets found — yaml-only mod)")

    # Git + Gitea
    if do_git:
        _emit(f"[{total_steps}/{total_steps}] Initializing git repo and pushing to Gitea...")
        from creation_lib.mod.git_ops import gitea_init
        try:
            gitea_init(
                mod_dir, mod_name, game=game,
                gitea_url=gitea_url, gitea_user=gitea_user,
                gitea_org=gitea_org, gitea_token=gitea_token,
                on_progress=on_progress,
            )
        except Exception as e:
            _emit(f"WARNING: Git/Gitea setup failed — {e}")
    elif init_git:
        _emit("NOTE: Skipping Gitea repo creation — GITEA_URL or GITEA_USER not set.")

    _emit(f"=== Migration complete ===")

    return mod_dir


def _find_yaml_dir(source_dir: Path, mod_name: str) -> Path | None:
    """Return a YAML folder in the source if it contains an authoring manifest."""
    for candidate in [source_dir / "yaml", source_dir]:
        if candidate.is_dir():
            if (candidate / "plugin.yaml").is_file() or (candidate / "plugin.json").is_file():
                return candidate
    return None


def _find_plugins(source_dir: Path, mod_name: str) -> tuple[Path | None, list[Path], Path]:
    """Locate all plugin files in common layouts.

    Returns (main_esp, patch_esps, asset_root).
    main_esp: the plugin whose stem best matches mod_name (or shortest stem).
    patch_esps: all other plugins found.
    """
    # Collect all plugins from root and Data/
    all_plugins: list[Path] = []
    asset_root = source_dir

    for ext in ["esp", "esl", "esm"]:
        all_plugins.extend(source_dir.glob(f"*.{ext}"))

    data_sub = source_dir / "Data"
    if data_sub.is_dir():
        for ext in ["esp", "esl", "esm"]:
            for p in data_sub.glob(f"*.{ext}"):
                all_plugins.append(p)
                asset_root = data_sub  # prefer Data/ as asset root if plugins found there

    if not all_plugins:
        return None, [], source_dir

    # Deduplicate by resolved path
    seen: set[Path] = set()
    unique: list[Path] = []
    for p in all_plugins:
        rp = p.resolve()
        if rp not in seen:
            seen.add(rp)
            unique.append(p)
    all_plugins = unique

    # Strip prefix for matching (e.g. mod_name="B21_M50_Madsen" → base="M50_Madsen")
    base_name = mod_name
    if "_" in mod_name:
        # Try without prefix (e.g. B21_)
        _, _, after_prefix = mod_name.partition("_")
        if after_prefix:
            base_name = after_prefix

    # Find main plugin: exact mod_name match > base_name match > shortest stem
    main: Path | None = None
    for p in all_plugins:
        if p.stem == mod_name:
            main = p
            break
    if not main:
        for p in all_plugins:
            if p.stem == base_name:
                main = p
                break
    if not main:
        # Pick the one with shortest stem (likely the main, not a patch)
        main = min(all_plugins, key=lambda p: len(p.stem))

    patches = [p for p in all_plugins if p != main]
    return main, patches, asset_root


def _find_plugin(source_dir: Path, mod_name: str) -> tuple[Path | None, Path]:
    """Locate a plugin file in common layouts. Returns (esp_path, asset_root).

    Backward-compatible wrapper around _find_plugins().
    """
    main, _, asset_root = _find_plugins(source_dir, mod_name)
    return main, asset_root
