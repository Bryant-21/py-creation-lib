"""Mod inspection: extract BA2s, decompile PEX scripts, serialize .esp to YAML, catalog assets.

Public API:
    inspect_mod(mod_path, game, *, force, extract_textures, skip_authoring_yaml,
                all_plugins, data_folder, on_progress) -> dict
"""

import json
import logging
import os
from datetime import datetime, timezone
from pathlib import Path

_log = logging.getLogger(__name__)

EXTENSION_CATEGORIES = {
    "meshes": {".nif"},
    "textures": {".dds"},
    "materials": {".bgsm", ".bgem"},
    "animations": {".hkx"},
    "sounds": {".wav", ".xwm"},
    "scripts_pex": {".pex"},
    "scripts_psc": {".psc"},
    "plugins": {".esp", ".esm", ".esl"},
    "archives": {".ba2", ".bsa"},
}

DECOMPILED_SCRIPTS_DIR = Path("SCRIPTS") / "SOURCE"
EXCLUDE_FILES = {"inspection_report.json"}


# ---------------------------------------------------------------------------
# detect_plugins
# ---------------------------------------------------------------------------

def detect_plugins(mod_path: Path) -> list[dict]:
    """Find .esp/.esm/.esl files at mod root."""
    plugins = []
    for ext in (".esp", ".esm", ".esl"):
        for f in sorted(mod_path.glob(f"*{ext}")):
            plugins.append({
                "name": f.name,
                "size_bytes": f.stat().st_size,
                "type": ext.lstrip("."),
            })
    return plugins


# ---------------------------------------------------------------------------
# extract_archives
# ---------------------------------------------------------------------------

def extract_archives(
    mod_path: Path,
    force: bool = False,
    extract_textures: bool = False,
    on_progress=None,
) -> dict:
    """Extract BA2/BSA archives using the native archive backend."""
    from creation_lib.preprocessor.extraction import extract_with_native_archive

    def _emit(msg: str):
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    result: dict = {"extracted": [], "skipped_textures": [], "errors": []}

    archives = sorted(
        [*mod_path.glob("*.ba2"), *mod_path.glob("*.bsa")],
        key=lambda p: p.name.lower(),
    )
    for ba2 in archives:
        if " - Textures" in ba2.stem and not extract_textures:
            result["skipped_textures"].append(ba2.name)
            _emit(f"  Skipping texture archive: {ba2.name}")
            continue

        output_dir = mod_path / ba2.stem

        if output_dir.exists() and any(output_dir.iterdir()) and not force:
            file_count = sum(1 for _ in output_dir.rglob("*") if _.is_file())
            result["extracted"].append({
                "name": ba2.name,
                "extracted_to": ba2.stem,
                "file_count": file_count,
                "skipped": True,
            })
            _emit(f"  Already extracted: {ba2.name} ({file_count} files)")
            continue

        output_dir.mkdir(exist_ok=True)
        _emit(f"  Extracting: {ba2.name} -> {ba2.stem}/")

        try:
            archive_format = "bsa" if ba2.suffix.lower() == ".bsa" else "ba2"
            file_count = extract_with_native_archive(ba2, output_dir, archive_format)
            result["extracted"].append({
                "name": ba2.name,
                "extracted_to": ba2.stem,
                "file_count": file_count,
                "skipped": False,
            })
            _emit(f"  Extracted {file_count} files")
        except (FileNotFoundError, RuntimeError) as exc:
            result["errors"].append({"name": ba2.name, "error": str(exc)})
            _emit(f"  ERROR extracting {ba2.name}: {exc}")

    return result


# ---------------------------------------------------------------------------
# decompile_scripts
# ---------------------------------------------------------------------------

def decompile_scripts(
    mod_path: Path,
    force: bool = False,
    on_progress=None,
) -> dict:
    """Decompile all .pex files in mod_path to .psc source."""
    from creation_lib.pex import decompile_pex

    def _emit(msg: str):
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    output_dir = mod_path / DECOMPILED_SCRIPTS_DIR
    result: dict = {
        "pex_files": [],
        "decompiled_to": str(DECOMPILED_SCRIPTS_DIR),
        "results": [],
    }

    pex_files = []
    for pex in mod_path.rglob("*.pex"):
        try:
            pex.relative_to(output_dir)
            continue  # skip already-decompiled output
        except ValueError:
            pass
        pex_files.append(pex)

    pex_files.sort()
    result["pex_files"] = [str(p.relative_to(mod_path)) for p in pex_files]

    if not pex_files:
        return result

    output_dir.mkdir(parents=True, exist_ok=True)

    for pex in pex_files:
        psc_name = pex.stem + ".psc"
        expected = output_dir / psc_name

        if expected.exists() and not force:
            result["results"].append({
                "pex": str(pex.relative_to(mod_path)),
                "psc": psc_name,
                "success": True,
                "skipped": True,
            })
            _emit(f"  Already decompiled: {pex.name}")
            continue

        _emit(f"  Decompiling: {pex.name}")
        try:
            source = decompile_pex(pex)
            expected.write_text(source, encoding="utf-8")
            result["results"].append({
                "pex": str(pex.relative_to(mod_path)),
                "psc": psc_name,
                "success": True,
                "skipped": False,
            })
            _emit(f"  OK: {psc_name}")
        except Exception as exc:
            result["results"].append({
                "pex": str(pex.relative_to(mod_path)),
                "psc": None,
                "success": False,
                "skipped": False,
                "error": str(exc),
            })
            _emit(f"  FAILED: {pex.name} — {str(exc)[:100]}")

    return result


# ---------------------------------------------------------------------------
# catalog_assets
# ---------------------------------------------------------------------------

def catalog_assets(mod_path: Path) -> tuple[dict, int]:
    """Walk mod folder and categorize all files by extension."""
    counts: dict[str, dict] = {cat: {"count": 0, "top_dirs": set()} for cat in EXTENSION_CATEGORIES}
    voice_types: set[str] = set()
    anim_data_count = 0
    anim_data_dirs: set[str] = set()
    total_files = 0

    for root_str, dirs, files in os.walk(mod_path):
        root = Path(root_str)
        rel_root = root.relative_to(mod_path)
        # Never descend into the decompiled scripts output
        dirs[:] = [d for d in dirs if Path(rel_root, d) != DECOMPILED_SCRIPTS_DIR]

        for fname in files:
            if fname in EXCLUDE_FILES:
                continue

            total_files += 1
            fpath = root / fname
            ext = fpath.suffix.lower()
            rel_path = fpath.relative_to(mod_path)
            rel_parts = rel_path.parts

            # Animation text data
            if ext == ".txt" and any(p.lower() == "animtextdata" for p in rel_parts):
                anim_data_count += 1
                if len(rel_parts) >= 2:
                    anim_data_dirs.add(str(Path(*rel_parts[:2])))
                continue

            # Voice files
            if ext == ".fuz":
                lower_parts = [p.lower() for p in rel_parts]
                try:
                    voice_idx = lower_parts.index("voice")
                    if voice_idx + 2 < len(rel_parts):
                        voice_types.add(rel_parts[voice_idx + 2])
                except ValueError:
                    pass
                counts["sounds"]["count"] += 1
                continue

            for cat, exts in EXTENSION_CATEGORIES.items():
                if ext in exts:
                    counts[cat]["count"] += 1
                    if len(rel_parts) >= 2:
                        counts[cat]["top_dirs"].add(str(Path(*rel_parts[:2])))
                    break

    assets: dict = {}
    for cat, data in counts.items():
        entry: dict = {"count": data["count"]}
        top_dirs = sorted(data["top_dirs"])
        if top_dirs:
            entry["top_dirs"] = top_dirs
        assets[cat] = entry

    if voice_types:
        assets["voice"] = {
            "count": sum(1 for _ in mod_path.rglob("*.fuz")),
            "voice_types": sorted(voice_types),
        }

    if anim_data_count:
        assets["animation_data"] = {
            "count": anim_data_count,
            "top_dirs": sorted(anim_data_dirs),
        }

    return assets, total_files


# ---------------------------------------------------------------------------
# serialize_plugins
# ---------------------------------------------------------------------------

def _serialize_plugins(
    mod_path: Path,
    plugins: list[dict],
    game: str,
    data_folder: Path | None,
    force: bool,
    all_plugins: bool,
    on_progress,
) -> dict:
    """Serialize plugins to YAML via creation_lib.esp.authoring.serialize (native pipeline)."""
    from creation_lib.esp.authoring import serialize

    def _emit(msg: str):
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    result: dict = {"serialized": [], "skipped": [], "errors": []}

    if not plugins:
        return result

    plugins_to_serialize = plugins if all_plugins else plugins[:1]

    for i, plugin in enumerate(plugins_to_serialize):
        plugin_path = mod_path / plugin["name"]
        stem = plugin_path.stem

        # Determine target yaml dir:
        #   main plugin → mod_path/yaml/
        #   patches     → mod_path/patches/<stem>/yaml/
        if i == 0:
            yaml_dir = mod_path / "yaml"
            yaml_dir_label = "yaml"
        else:
            # Check for legacy yaml_<stem>/ convention first
            legacy_dir = mod_path / f"yaml_{stem}"
            if legacy_dir.is_dir() and not force:
                yaml_dir = legacy_dir
                yaml_dir_label = f"yaml_{stem}"
            else:
                yaml_dir = mod_path / "patches" / stem / "yaml"
                yaml_dir_label = f"patches/{stem}/yaml"

        if yaml_dir.exists() and any(yaml_dir.iterdir()) and not force:
            file_count = sum(1 for _ in yaml_dir.rglob("*") if _.is_file())
            result["skipped"].append({
                "plugin": plugin["name"],
                "yaml_dir": yaml_dir_label,
                "file_count": file_count,
                "reason": "already serialized (use --force to redo)",
            })
            _emit(f"  Already serialized: {plugin['name']} ({file_count} YAML files)")
            continue

        _emit(f"  Serializing: {plugin['name']} -> {yaml_dir_label}/")

        # creation_lib.esp.authoring.serialize always writes to output_dir/yaml/
        # For the main plugin output_dir=mod_path gives mod_path/yaml/ ✓
        # For patches we use a temp parent then rename.
        if i == 0:
            output_dir = mod_path
        else:
            output_dir = mod_path / f"_tmp_authoring_yaml_{stem}"
            output_dir.mkdir(exist_ok=True)

        try:
            created = serialize(
                plugin_path,
                output_dir,
                game=game,
                data_folder=data_folder,
                error_on_unknown=(i == 0),  # lenient for patches
                on_progress=on_progress,
            )
            # Rename temp yaml/ to patches/<stem>/yaml/ for patches
            if i > 0:
                yaml_dir.parent.mkdir(parents=True, exist_ok=True)
                if yaml_dir.exists():
                    import shutil
                    shutil.rmtree(yaml_dir)
                created.rename(yaml_dir)
                output_dir.rmdir()  # remove empty temp dir

            file_count = sum(1 for _ in yaml_dir.rglob("*") if _.is_file())
            result["serialized"].append({
                "plugin": plugin["name"],
                "yaml_dir": yaml_dir_label,
                "file_count": file_count,
            })
            _emit(f"  Serialized {file_count} YAML files")
        except Exception as exc:
            result["errors"].append({"plugin": plugin["name"], "error": str(exc)})
            _emit(f"  ERROR: {exc}")

    # Note unprocessed patches
    if not all_plugins:
        for extra in plugins[1:]:
            result["skipped"].append({
                "plugin": extra["name"],
                "reason": "optional patch — re-run with --all-plugins to serialize",
            })
            _emit(f"  NOTE: {extra['name']} is an optional patch (re-run with --all-plugins to include)")

    return result


# ---------------------------------------------------------------------------
# inspect_mod  (main entry point)
# ---------------------------------------------------------------------------

def inspect_mod(
    mod_path: Path,
    game: str = "fo4",
    *,
    force: bool = False,
    extract_textures: bool = False,
    skip_authoring_yaml: bool = False,
    all_plugins: bool = False,
    data_folder: Path | None = None,
    on_progress=None,
) -> dict:
    """Run full inspection of a mod folder.

    Returns the inspection report dict (also written to mod_path/inspection_report.json).
    """
    def _emit(msg: str):
        _log.info(msg)
        if on_progress:
            on_progress(msg)

    mod_name = mod_path.name
    _emit(f"=== Inspecting: {mod_name} ===")
    _emit(f"Path: {mod_path}\n")

    _emit("--- Plugins ---")
    plugins = detect_plugins(mod_path)
    for p in plugins:
        _emit(f"  {p['name']} ({p['size_bytes']:,} bytes, {p['type']})")
    if not plugins:
        _emit("  (none found)")

    yaml_result: dict = {"serialized": [], "skipped": [], "errors": []}
    if not skip_authoring_yaml and plugins:
        _emit("\n--- ESP YAML Serialization ---")
        yaml_result = _serialize_plugins(
            mod_path, plugins, game, data_folder, force, all_plugins, on_progress,
        )
    elif skip_authoring_yaml:
        _emit("\n--- ESP YAML Serialization --- (skipped)")

    _emit("\n--- Archives ---")
    archives = extract_archives(mod_path, force=force, extract_textures=extract_textures, on_progress=on_progress)
    if not archives["extracted"] and not archives["skipped_textures"]:
        _emit("  (no BA2/BSA archives found)")

    _emit("\n--- Scripts ---")
    scripts = decompile_scripts(mod_path, force=force, on_progress=on_progress)
    if not scripts["pex_files"]:
        _emit("  (no PEX scripts found)")

    _emit("\n--- Assets ---")
    assets, total_files = catalog_assets(mod_path)
    for cat, data in sorted(assets.items()):
        if data["count"] > 0:
            _emit(f"  {cat}: {data['count']}")

    report = {
        "mod_name": mod_name,
        "mod_path": str(mod_path),
        "game": game,
        "plugins": plugins,
        "yaml_serialization": yaml_result,
        "archives": archives,
        "scripts": scripts,
        "assets": assets,
        "total_files": total_files,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    output_path = mod_path / "inspection_report.json"
    output_path.write_text(json.dumps(report, indent=2))
    _emit(f"\n=== Report written to: {output_path} ===")

    return report
