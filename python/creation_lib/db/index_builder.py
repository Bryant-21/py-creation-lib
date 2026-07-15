"""Index builder — orchestrate preprocess scripts to build search indexes."""

from __future__ import annotations

import logging
import os
import shutil
from pathlib import Path
from typing import Callable

from creation_lib.core.game_profiles import get_profile, GAME_PROFILES
from creation_lib.preprocessor.preprocess_runner import run_preprocess
from creation_lib.preprocessor.records import GAME_ESM_YAML_DIR


_SHARED_WIKI_DB_GAME = {
    "fnv": "fo3",
}

_log = logging.getLogger(__name__)


def _required_path(value: Path | str | None, name: str) -> Path:
    if value is None:
        raise ValueError(f"{name} is required")
    return Path(value)


def _run_preprocess(
    script: str,
    game: str,
    *extra_args: str,
    project_root: Path | str,
) -> None:
    """Run a preprocess entrypoint inside the current process."""
    root = Path(project_root)
    result = run_preprocess(script, "--game", game, *extra_args, cwd=root)
    if result != 0:
        raise RuntimeError(f"{script} failed with exit code {result}")


# ---------------------------------------------------------------------------
# Build game index
# ---------------------------------------------------------------------------


def build_game_index(
    game: str,
    *,
    extracted_dir: str | None = None,
    game_dir: str | None = None,
    project_root: Path | str,
    db_dir: Path | str,
    embeddings: bool = False,
    on_progress: Callable[[str], None] | None = None,
) -> dict[str, str]:
    """Run all preprocess scripts for a game.

    *extracted_dir* and *game_dir* are required for domains that need
    them (records/scripts/wiki may not; nifs/havok do). The CLI layer
    is responsible for reading .env and passing these explicitly.

    Returns dict mapping domain name → "built" or "skipped".
    """
    profile = get_profile(game)  # validates game exists
    root = _required_path(project_root, "project_root")
    data_root = _required_path(db_dir, "db_dir")

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    _emit(f"Building indexes for {profile.display_name} ({game})")

    results: dict[str, str] = {}
    embed_args = ["--embeddings"] if embeddings else []

    # 1. Records
    _emit("--- records ---")
    has_records = False
    data_dir = data_root
    if data_dir.is_dir():
        for d in data_dir.iterdir():
            if d.is_dir() and (game in d.name.lower() and "yaml" in d.name.lower()):
                has_records = True
                break
        # Special case for fo4
        if not has_records and game == "fo4" and (data_dir / "fo4_esm_yaml").is_dir():
            has_records = True

    if has_records:
        _emit(f"  Running preprocess_records.py --game {game}")
        _run_preprocess(
            "preprocess_records.py",
            game,
            "--db-path",
            str(data_root / f"{game}_records.db"),
            "--esm-yaml-dir",
            str(data_root / (GAME_ESM_YAML_DIR.get(game) or f"{game}_esm_yaml")),
            *embed_args,
            project_root=root,
        )
        results["records"] = "built"
    else:
        _emit("  Skipped: no ESM YAML data found")
        results["records"] = "skipped"

    # 2. Scripts
    _emit("--- scripts ---")
    has_scripts = False
    if game_dir and os.path.isdir(os.path.join(game_dir, "Data", "Scripts", "Source")):
        has_scripts = True
    if extracted_dir and os.path.isdir(
        os.path.join(extracted_dir, "Scripts", "Source")
    ):
        has_scripts = True

    if has_scripts:
        _emit(f"  Running preprocess_scripts.py --game {game}")
        script_args = ["--db-path", str(data_root / f"{game}_scripts.db")]
        if game_dir:
            script_args.extend(["--game-dir", game_dir])
        _run_preprocess(
            "preprocess_scripts.py",
            game,
            *script_args,
            *embed_args,
            project_root=root,
        )
        results["scripts"] = "built"
    else:
        _emit("  Skipped: no script sources found")
        results["scripts"] = "skipped"

    # 3. Wiki
    _emit("--- wiki ---")
    wiki_dir_name = profile.wiki_dir
    if wiki_dir_name and (root / "Wiki" / wiki_dir_name).is_dir():
        _emit(f"  Running preprocess_wiki.py --game {game}")
        _run_preprocess(
            "preprocess_wiki.py",
            game,
            "--wiki-dir",
            str(root / "Wiki" / wiki_dir_name),
            "--db-path",
            str(data_root / f"{_SHARED_WIKI_DB_GAME.get(game, game)}_wiki.db"),
            *embed_args,
            project_root=root,
        )
        results["wiki"] = "built"
    else:
        _emit(
            f"  Skipped: {'no wiki configured' if not wiki_dir_name else f'Wiki/{wiki_dir_name}/ not found'}"
        )
        results["wiki"] = "skipped"

    # 4. NIFs
    _emit("--- nifs ---")
    if extracted_dir and os.path.isdir(extracted_dir):
        _emit(f"  Running preprocess_nifs.py --game {game}")
        _run_preprocess(
            "preprocess_nifs.py",
            game,
            "--extracted-dir",
            extracted_dir,
            "--db-path",
            str(data_root / f"{game}_nifs.db"),
            "--external-mods-dir",
            str(root / "external_mods"),
            *embed_args,
            project_root=root,
        )
        results["nifs"] = "built"
    else:
        _emit(
            f"  Skipped: no extracted game data ({profile.env_var_name} not set or dir missing)"
        )
        results["nifs"] = "skipped"

    # 5. Behaviors
    _emit("--- behaviors ---")
    if extracted_dir and os.path.isdir(extracted_dir):
        _emit(f"  Running preprocess_havok.py --game {game}")
        _run_preprocess(
            "preprocess_havok.py",
            game,
            "--extracted-dir",
            extracted_dir,
            "--db-path",
            str(data_root / f"{game}_havok.db"),
            "--external-mods-dir",
            str(root / "external_mods"),
            *embed_args,
            project_root=root,
        )
        results["behaviors"] = "built"
    else:
        _emit(
            f"  Skipped: no extracted game data ({profile.env_var_name} not set or dir missing)"
        )
        results["behaviors"] = "skipped"

    built = sum(1 for v in results.values() if v == "built")
    skipped = sum(1 for v in results.values() if v == "skipped")
    _emit(f"Done: {built} index(es) built, {skipped} skipped")

    return results


# ---------------------------------------------------------------------------
# Native YAML cache regeneration
# ---------------------------------------------------------------------------


def regenerate_esm_yaml_cache(
    game: str,
    *,
    game_data_dir: Path | str,
    project_root: Path | str,
    db_dir: Path | str,
    plugins: list[str] | None = None,
    fresh: bool = False,
    on_progress: Callable[[str], None] | None = None,
) -> dict[str, str]:
    """Re-export the game's master plugins into ``data/<game>_esm_yaml/``.

    Walks ``game_data_dir`` (or the explicit ``plugins`` list) and runs the
    native authoring exporter for each master plugin. The output drives
    ``preprocess_records.py`` (records search index).

    Args:
        game: Game ID — must be in ``GAME_ESM_YAML_DIR``.
        game_data_dir: Path to the game's installed ``Data/`` folder.
        plugins: Optional explicit list of plugin file names (e.g.
            ``["Fallout4.esm", "DLCRobot.esm"]``). When omitted, every
            ``*.esm`` in ``game_data_dir`` is re-exported.
        fresh: If True, clear the per-plugin cache directory before re-export.
        on_progress: Optional progress callback.

    Returns:
        Dict mapping plugin file name → ``"exported"`` or ``"skipped (...)"``.
    """
    root = _required_path(project_root, "project_root")
    data_root = _required_path(db_dir, "db_dir")
    cache_subdir = GAME_ESM_YAML_DIR.get(game) or f"{game}_esm_yaml"
    cache_root = data_root / cache_subdir
    cache_root.mkdir(parents=True, exist_ok=True)

    data_dir = Path(game_data_dir)
    if not data_dir.is_dir():
        raise FileNotFoundError(f"Game Data directory not found: {data_dir}")

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    if plugins is None:
        plugin_paths = sorted(data_dir.glob("*.esm"))
    else:
        plugin_paths = [data_dir / name for name in plugins]

    if not plugin_paths:
        _emit(f"No master plugins found in {data_dir}")
        return {}

    from creation_lib.esp.native_runtime import export_authoring_dir_native

    results: dict[str, str] = {}
    _emit(f"Regenerating native YAML cache for {game} ({len(plugin_paths)} plugin(s))")
    _emit(f"  Source: {data_dir}")
    _emit(f"  Cache: {cache_root}")

    for plugin_path in plugin_paths:
        if not plugin_path.is_file():
            _emit(f"  - {plugin_path.name}: skipped (file missing)")
            results[plugin_path.name] = "skipped (file missing)"
            continue

        target = cache_root / plugin_path.stem
        if fresh and target.is_dir():
            shutil.rmtree(target)
        target.mkdir(parents=True, exist_ok=True)

        try:
            display_target = target.relative_to(root)
        except ValueError:
            display_target = target
        _emit(f"  - {plugin_path.name} → {display_target}")
        try:
            export_authoring_dir_native(
                str(plugin_path),
                str(target),
                game=game,
                format="yaml",
            )
            results[plugin_path.name] = "exported"
        except Exception as exc:  # surface but continue with other plugins
            _log.exception("Native export failed for %s", plugin_path)
            _emit(f"      ERROR: {exc}")
            results[plugin_path.name] = f"error: {exc}"

    exported = sum(1 for v in results.values() if v == "exported")
    _emit(f"Done: {exported}/{len(plugin_paths)} plugin(s) exported")
    return results


# ---------------------------------------------------------------------------
# Build single domain
# ---------------------------------------------------------------------------

_DOMAIN_SCRIPTS: dict[str, str] = {
    "records": "preprocess_records.py",
    "scripts": "preprocess_scripts.py",
    "wiki": "preprocess_wiki.py",
    "ck": "preprocess_wiki.py",
    "nifs": "preprocess_nifs.py",
    "behaviors": "preprocess_havok.py",
    "havok": "preprocess_havok.py",
    "external": "preprocess_external.py",
    "swf": "preprocess_swf.py",
}


def build_domain_index(
    game: str,
    domain: str,
    *,
    extracted_dir: str | None = None,
    game_dir: str | None = None,
    project_root: Path | str,
    db_dir: Path | str,
    embeddings: bool = False,
    on_progress: Callable[[str], None] | None = None,
) -> None:
    """Run the preprocess script for a single domain.

    Args:
        game: Game ID (fo4, skyrimse, starfield, etc.)
        domain: Index domain name (records, scripts, wiki, ck, nifs, behaviors, havok, external)
    """
    get_profile(game)  # validates game exists

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    script = _DOMAIN_SCRIPTS.get(domain.lower())
    if not script:
        raise ValueError(
            f"Unknown domain: {domain!r}. Valid: {', '.join(_DOMAIN_SCRIPTS)}"
        )

    embed_args = ["--embeddings"] if embeddings else []
    root = _required_path(project_root, "project_root")
    data_root = _required_path(db_dir, "db_dir")
    extra_args: list[str] = []
    lower_domain = domain.lower()
    if lower_domain == "records":
        extra_args.extend([
            "--db-path",
            str(data_root / f"{game}_records.db"),
            "--esm-yaml-dir",
            str(data_root / (GAME_ESM_YAML_DIR.get(game) or f"{game}_esm_yaml")),
        ])
    elif lower_domain == "scripts":
        extra_args.extend(["--db-path", str(data_root / f"{game}_scripts.db")])
        if game_dir:
            extra_args.extend(["--game-dir", game_dir])
    elif lower_domain == "nifs":
        if extracted_dir:
            extra_args.extend(["--extracted-dir", extracted_dir])
        extra_args.extend([
            "--db-path",
            str(data_root / f"{game}_nifs.db"),
            "--external-mods-dir",
            str(root / "external_mods"),
        ])
    elif lower_domain in {"behaviors", "havok"}:
        if extracted_dir:
            extra_args.extend(["--extracted-dir", extracted_dir])
        extra_args.extend([
            "--db-path",
            str(data_root / f"{game}_havok.db"),
            "--external-mods-dir",
            str(root / "external_mods"),
        ])
    elif lower_domain == "swf":
        if extracted_dir:
            extra_args.extend(["--extracted-dir", extracted_dir])
        extra_args.extend(["--db-path", str(data_root / f"{game}_swf_shapes.db")])
    elif lower_domain in {"wiki", "ck"}:
        profile = get_profile(game)
        if not profile.wiki_dir:
            raise ValueError(f"No wiki configured for {game}")
        extra_args.extend([
            "--wiki-dir",
            str(root / "Wiki" / profile.wiki_dir),
            "--db-path",
            str(data_root / f"{_SHARED_WIKI_DB_GAME.get(game, game)}_wiki.db"),
        ])
    elif lower_domain == "external":
        extra_args.extend([
            "--external-mods-dir",
            str(root / "external_mods"),
            "--db-path",
            str(data_root / f"{game}_external_mods.db"),
        ])
    _emit(f"Building {domain} index for {game}...")
    _run_preprocess(script, game, *extra_args, *embed_args, project_root=root)
    _emit(f"Done.")


# ---------------------------------------------------------------------------
# Add to library
# ---------------------------------------------------------------------------


def add_to_library(
    mod_name: str,
    *,
    project_root: Path | str,
    db_dir: Path | str,
    on_progress: Callable[[str], None] | None = None,
) -> None:
    """Migrate inspected mod from mod_inspector/ to external_mods/ and rebuild indexes."""
    root = _required_path(project_root, "project_root")
    data_root = _required_path(db_dir, "db_dir")
    src = root / "mod_inspector" / mod_name
    dest = root / "external_mods" / mod_name

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    dest.mkdir(parents=True, exist_ok=True)

    # Copy YAML records
    if (src / "yaml").is_dir():
        import shutil

        if (dest / "yaml").exists():
            shutil.rmtree(dest / "yaml")
        shutil.copytree(src / "yaml", dest / "yaml")

    # Copy scripts (case-insensitive search)
    for scripts_dir_name in ["SCRIPTS/SOURCE", "scripts/source", "Scripts/Source"]:
        scripts_src = src / scripts_dir_name
        if scripts_src.is_dir():
            import shutil

            scripts_dest = dest / "scripts"
            if scripts_dest.exists():
                shutil.rmtree(scripts_dest)
            shutil.copytree(scripts_src, scripts_dest)
            break

    # Copy README
    readme = src / "README.md"
    if readme.is_file():
        import shutil

        shutil.copy2(readme, dest / "README.md")

    # Copy meshes
    for meshdir_name in ["MESHES", "Meshes", "meshes"]:
        meshdir = src / meshdir_name
        if meshdir.is_dir():
            import shutil

            _emit(f"Copying meshes from {meshdir}...")
            if (dest / "Meshes").exists():
                shutil.rmtree(dest / "Meshes")
            shutil.copytree(meshdir, dest / "Meshes")
            break

    # Copy behaviors
    for behavdir_name in ["behaviors", "Behaviors", "BEHAVIORS"]:
        behavdir = src / behavdir_name
        if behavdir.is_dir():
            import shutil

            _emit(f"Copying behaviors from {behavdir}...")
            if (dest / "behaviors").exists():
                shutil.rmtree(dest / "behaviors")
            shutil.copytree(behavdir, dest / "behaviors")
            break

    # Rebuild indexes
    _emit("Rebuilding records/scripts index...")
    _run_preprocess(
        "preprocess_external.py",
        "fo4",
        "--mod",
        mod_name,
        "--external-mods-dir",
        str(root / "external_mods"),
        "--db-path",
        str(data_root / "fo4_external_mods.db"),
        project_root=root,
    )

    if (dest / "Meshes").is_dir():
        _emit("Indexing NIF meshes...")
        _run_preprocess(
            "preprocess_nifs.py",
            "fo4",
            "--mod",
            mod_name,
            "--external-mods-dir",
            str(root / "external_mods"),
            "--db-path",
            str(data_root / "fo4_nifs.db"),
            project_root=root,
        )

    if (dest / "behaviors").is_dir() or (dest / "Meshes").is_dir():
        _emit("Indexing behavior files...")
        _run_preprocess(
            "preprocess_havok.py",
            "fo4",
            "--mod",
            mod_name,
            "--external-mods-dir",
            str(root / "external_mods"),
            "--db-path",
            str(data_root / "fo4_havok.db"),
            project_root=root,
        )

    _emit(f'Done! "{mod_name}" added to reference library.')


# ---------------------------------------------------------------------------
# Add game scaffold
# ---------------------------------------------------------------------------


def add_game_scaffold(
    game_id: str,
    display_name: str,
    *,
    project_root: Path | str,
    on_progress: Callable[[str], None] | None = None,
) -> dict[str, str]:
    """Scaffold directories and skill stubs for a new game.

    Returns a checklist dict of manual steps still needed.
    """
    root = _required_path(project_root, "project_root")

    def _emit(msg: str) -> None:
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    # Validate game_id
    if not game_id.isalnum() or game_id != game_id.lower():
        raise ValueError(f"game_id must be lowercase alphanumeric (got: {game_id})")

    if game_id in GAME_PROFILES:
        raise ValueError(
            f"Game profile '{game_id}' already exists in py_creation_lib/python/creation_lib/game_profiles.py"
        )

    game_upper = game_id.upper()

    _emit(f"Scaffolding new game: {display_name} ({game_id})")

    # 1. Create directories
    wiki_dir = root / "Wiki" / f"{game_id}_wiki"
    extracted_dir = root / "extracted" / game_id
    wiki_dir.mkdir(parents=True, exist_ok=True)
    extracted_dir.mkdir(parents=True, exist_ok=True)
    _emit(f"  Created: Wiki/{game_id}_wiki/")
    _emit(f"  Created: extracted/{game_id}/")

    # 2. Create skill overlay stubs
    skills_dir = root / ".claude" / "skills"

    # creation-data domains reference
    domains_stub = skills_dir / "creation-data" / "references" / f"{game_id}-domains.md"
    if not domains_stub.is_file():
        domains_stub.parent.mkdir(parents=True, exist_ok=True)
        domains_stub.write_text(
            f"# {display_name} — Domain Reference\n\n"
            f"## Available Domains\n\n"
            f"> **TODO:** Populate this file after building indexes with:\n"
            f"> ```\n"
            f"> modkit index --game {game_id} build\n"
            f"> ```\n\n"
            f"### records\n"
            f"- Database: `data/{game_id}_records.db`\n"
            f"- Status: Not yet populated\n\n"
            f"### scripts\n"
            f"- Database: `data/{game_id}_scripts.db`\n"
            f"- Status: Not yet populated\n\n"
            f"### wiki\n"
            f"- Database: `data/{game_id}_wiki.db`\n"
            f"- Source: `Wiki/{game_id}_wiki/`\n"
            f"- Status: Not yet populated\n",
            encoding="utf-8",
        )
        _emit(
            f"  Created: .claude/skills/creation-data/references/{game_id}-domains.md"
        )

    # papyrus-language compiler reference
    compiler_stub = (
        skills_dir / "papyrus-language" / "references" / f"{game_id}-compiler.md"
    )
    if not compiler_stub.is_file():
        compiler_stub.parent.mkdir(parents=True, exist_ok=True)
        compiler_stub.write_text(
            f"# {display_name} — Papyrus Compiler Reference\n\n"
            f"> **TODO:** Populate this file with game-specific compiler settings.\n\n"
            f"## Compiler Location\n\n"
            f"Set in GameProfile:\n"
            f"- `papyrus_compiler_dir`: (configure in py_creation_lib/python/creation_lib/game_profiles.py)\n"
            f"- `papyrus_flags`: (configure in py_creation_lib/python/creation_lib/game_profiles.py)\n\n"
            f"## Notes\n\n"
            f"Not yet documented.\n",
            encoding="utf-8",
        )
        _emit(
            f"  Created: .claude/skills/papyrus-language/references/{game_id}-compiler.md"
        )

    # 3. Manual steps checklist
    checklist = {
        "add_profile": (
            f"Add GameProfile to py_creation_lib/python/creation_lib/game_profiles.py:\n"
            f'  {game_upper}_PROFILE = GameProfile(id="{game_id}", display_name="{display_name}", ...)\n'
            f"  register_game({game_upper}_PROFILE)"
        ),
        "add_env": (
            f"Add to .env:\n"
            f'  {game_upper}_DIR="/path/to/{display_name}"\n'
            f'  {game_upper}_EXTRACTED_DIR=""'
        ),
        "add_wiki": f"(Optional) Add wiki content to Wiki/{game_id}_wiki/",
        "populate_stubs": "Populate skill reference stubs with game-specific content",
    }

    _emit("\nManual steps required:")
    for key, desc in checklist.items():
        _emit(f"  - {desc}")

    _emit(f"\nAfter adding the GameProfile and .env vars:")
    _emit(f"  modkit --game {game_id} build extract")
    _emit(f"  modkit index --game {game_id} build")

    return checklist
