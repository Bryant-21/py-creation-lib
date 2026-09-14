"""Indexes settings section (db rebuild, YAML extraction, BA2 extraction).

Index building is host-specific (see creation_lib.ui.host.UiHost); apps that
don't register a db_builder_factory (e.g. FallTalk) get this section with
rebuild/extraction actions disabled.
"""
from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor, as_completed
import logging
import os
from pathlib import Path
import threading

from imgui_bundle import imgui

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.ui.shell.settings_section import SettingsContext, SettingsSection

_log = logging.getLogger("toolkit.settings.indexes")

_INDEX_GAMES = [(p.id, p.display_name) for p in GAME_PROFILES.values()]
_MAX_EXTRACT_LOG_LINES = 18
_DEFAULT_ARCHIVE_WORKERS = 8


def _default_archive_workers() -> int:
    return max(1, min(_DEFAULT_ARCHIVE_WORKERS, os.cpu_count() or 1))


def _clamp_archive_workers(value: object) -> int:
    try:
        workers = int(value)
    except (TypeError, ValueError):
        return _default_archive_workers()
    return max(1, workers)


class _State:
    def __init__(self) -> None:
        self.index_game: str = "fo4"
        self.index_fo4_data: bool = True
        self.index_scripts: bool = True
        self.index_wiki: bool = True
        self.index_nifs: bool = True
        self.index_behaviors: bool = True
        self.index_swf: bool = True
        self.index_voice_reference: bool = True
        self.index_builder: object = None
        self.extracting: bool = False
        self.extract_thread: threading.Thread | None = None
        self.extract_status: str = ""
        self.extract_up_to_date: bool = False
        self.extract_last_date: str = ""
        self.extract_archive_workers: int = _default_archive_workers()
        self.extract_lock = threading.Lock()
        self.extract_progress: float = 0.0
        self.extract_total_archives: int = 0
        self.extract_completed_archives: int = 0
        self.extract_total_files: int = 0
        self.extract_error: str = ""
        self.extract_log_lines: list[str] = []
        self.extract_output_dir: str = ""
        self.extract_mode: str = ""


_state = _State()


def _set_extract_state(**kwargs) -> None:
    with _state.extract_lock:
        for key, value in kwargs.items():
            setattr(_state, key, value)


def _reset_extract_run(game: str, output_dir: Path, *, smart: bool, workers: int) -> None:
    mode = "Smart Extract" if smart else "Full Extract"
    with _state.extract_lock:
        _state.extracting = True
        _state.extract_status = f"Starting {mode.lower()} for {game}..."
        _state.extract_progress = 0.0
        _state.extract_total_archives = 0
        _state.extract_completed_archives = 0
        _state.extract_total_files = 0
        _state.extract_error = ""
        _state.extract_log_lines = []
        _state.extract_output_dir = str(output_dir)
        _state.extract_mode = mode
        _state.extract_archive_workers = workers


def _record_extract_log(message: str, *, level: int = logging.INFO) -> None:
    line = str(message).strip()
    if not line:
        return
    _log.log(level, line)
    with _state.extract_lock:
        _state.extract_log_lines.append(line)
        if len(_state.extract_log_lines) > _MAX_EXTRACT_LOG_LINES:
            del _state.extract_log_lines[:-_MAX_EXTRACT_LOG_LINES]


def _extract_snapshot() -> dict:
    with _state.extract_lock:
        return {
            "extracting": _state.extracting,
            "status": _state.extract_status,
            "progress": _state.extract_progress,
            "total_archives": _state.extract_total_archives,
            "completed_archives": _state.extract_completed_archives,
            "total_files": _state.extract_total_files,
            "error": _state.extract_error,
            "log_lines": list(_state.extract_log_lines),
            "output_dir": _state.extract_output_dir,
            "mode": _state.extract_mode,
            "archive_workers": _state.extract_archive_workers,
        }


def _load_extract_status(settings) -> None:
    from creation_lib.preprocessor.extraction import load_manifest, manifest_matches, find_archives
    from creation_lib.core.game_profiles import get_profile

    _state.extract_status = "No extracted data — first run does a full extract."
    _state.extract_up_to_date = False
    _state.extract_last_date = ""

    game = _state.index_game
    gp = settings.get_game_paths(game)
    extracted_dir = gp.get("extracted_dir", "")
    game_root = gp.get("root_dir", "")

    if not extracted_dir or not os.path.isdir(extracted_dir):
        return

    manifest = load_manifest(Path(extracted_dir))
    if manifest is None:
        return

    if game_root:
        data_dir = Path(game_root) / "Data"
        if data_dir.is_dir():
            try:
                profile = get_profile(game)
                archives = find_archives(data_dir, profile.archive_format)
                if manifest_matches(manifest, data_dir, archives):
                    _state.extract_up_to_date = True
                    date_str = manifest.get("extracted_at", "")[:10]
                    _state.extract_last_date = date_str
                    _state.extract_status = f"Up to date ({date_str})" if date_str else "Up to date"
                else:
                    _state.extract_status = "Updates available"
                return
            except Exception:
                pass

    date_str = manifest.get("extracted_at", "")[:10]
    _state.extract_last_date = date_str
    _state.extract_status = f"Extracted ({date_str})" if date_str else "Extracted"


def _voice_reference_index_status(game: str, settings) -> tuple[bool, str]:
    from creation_lib.ui.host import get_host
    from creation_lib.audio.voice_reference import voice_reference_sqlite_cache_path

    gp = settings.get_game_paths(game)
    root_value = str(gp.get("root_dir", "") or "").strip()
    if not root_value:
        return False, "No game path"
    root_dir = Path(root_value).expanduser()
    data_dir = root_dir / "Data"
    if not data_dir.is_dir() and root_dir.name.lower() == "data":
        data_dir = root_dir
    if not data_dir.is_dir():
        return False, "No Data folder"
    strings_dir = data_dir / "Strings"
    extracted_value = str(gp.get("extracted_dir", "") or "").strip()
    if not strings_dir.is_dir() and extracted_value:
        extracted = Path(extracted_value).expanduser()
        for candidate in (extracted / "Strings", extracted / "Data" / "Strings"):
            if candidate.is_dir():
                strings_dir = candidate
                break
    cache_path = voice_reference_sqlite_cache_path(
        game=game,
        data_dir=data_dir,
        strings_dir=strings_dir,
        db_dir=get_host().get_db_dir(),
    )
    if cache_path is None or not cache_path.is_file():
        return False, "Not built"
    size_mb = cache_path.stat().st_size // (1024 * 1024)
    return True, f"{size_mb} MB"


def _start_rebuild(settings, only: str | None) -> None:
    from creation_lib.ui.host import get_host

    factory = get_host().db_builder_factory
    if factory is None:
        return  # index building requires a host app service

    game = _state.index_game
    gp = settings.get_game_paths(game)
    game_root = gp.get("root_dir", "")
    extracted = gp.get("extracted_dir", "")

    if only is None:
        _state.index_builder = factory(
            game_root=game_root,
            extracted_dir=extracted,
            build_fo4_data=_state.index_fo4_data,
            build_scripts=_state.index_scripts,
            build_wiki=_state.index_wiki,
            build_nifs=_state.index_nifs,
            build_behaviors=_state.index_behaviors,
            build_swf=_state.index_swf,
            build_voice_reference_index=_state.index_voice_reference,
            force_rebuild=True,
            game=game,
        )
    else:
        _state.index_builder = factory(
            game_root=game_root,
            extracted_dir=extracted,
            build_fo4_data=(only == "fo4_data"),
            build_scripts=(only == "scripts"),
            build_wiki=(only == "wiki"),
            build_nifs=(only == "nifs"),
            build_behaviors=(only == "behaviors"),
            build_swf=(only == "swf"),
            build_voice_reference_index=(only == "voice_reference"),
            force_rebuild=True,
            game=game,
        )
    _state.index_builder.start()


def _start_yaml_extraction(settings, game: str, game_root: str, force: bool = False, smart: bool = False) -> None:
    from creation_lib.ui.host import get_host

    factory = get_host().db_builder_factory
    if factory is None:
        return  # index building requires a host app service

    gp = settings.get_game_paths(game)
    extracted = gp.get("extracted_dir", "")
    _state.index_builder = factory(
        game_root=game_root,
        extracted_dir=extracted,
        build_fo4_data=True,
        build_scripts=False,
        build_wiki=False,
        build_nifs=False,
        build_behaviors=False,
        force_rebuild=force,
        smart=smart,
        game=game,
    )
    _state.index_builder.start()


def _start_extraction(settings, game: str, game_root: str, smart: bool = False) -> None:
    from creation_lib.ui.host import get_host

    output_dir = get_host().resolve_extracted_output_dir(game)
    output_dir.mkdir(parents=True, exist_ok=True)
    workers = _clamp_archive_workers(_state.extract_archive_workers)
    _reset_extract_run(game, output_dir, smart=smart, workers=workers)

    def _run():
        try:
            _run_direct_extraction(
                settings,
                game,
                Path(game_root),
                output_dir,
                smart=smart,
                archive_workers=workers,
            )
            _load_extract_status(settings)
        except Exception as e:
            message = f"Extraction failed for {game}: {e}"
            _record_extract_log(message, level=logging.ERROR)
            _set_extract_state(extract_error=str(e), extract_status=message)
        finally:
            _set_extract_state(extracting=False, extract_thread=None)

    thread = threading.Thread(target=_run, daemon=True, name=f"extract-{game}")
    _set_extract_state(extract_thread=thread)
    thread.start()


def _run_direct_extraction(
    settings,
    game: str,
    install_dir: Path,
    output_dir: Path,
    *,
    smart: bool,
    archive_workers: int,
) -> None:
    from creation_lib.core.game_profiles import get_profile
    from creation_lib.preprocessor.extraction import (
        build_manifest,
        extract_one,
        find_archives,
        group_archives_by_update_phase,
        load_manifest,
        manifest_matches,
        plan_archive_extraction_batches,
        resolve_papyrus_source_dir,
        save_manifest,
        sync_papyrus_sources,
    )

    profile = get_profile(game)
    data_dir = install_dir / "Data"
    if not data_dir.is_dir():
        raise RuntimeError(f"Data directory not found: {data_dir}")

    _record_extract_log(f"{_state.extract_mode}: {profile.display_name} ({game})")
    _record_extract_log(f"Install dir: {install_dir}")
    _record_extract_log(f"Data dir: {data_dir}")
    _record_extract_log(f"Output dir: {output_dir}")

    papyrus_source_dir = resolve_papyrus_source_dir(install_dir, game)
    if papyrus_source_dir is not None:
        _record_extract_log(f"Papyrus source: {papyrus_source_dir}")

    archives = find_archives(data_dir, profile.archive_format)
    if not archives:
        raise RuntimeError(f"No {profile.archive_format.upper()} archives found in {data_dir}")

    archive_count = len(archives)
    workers = _clamp_archive_workers(archive_workers)
    _set_extract_state(
        extract_total_archives=archive_count,
        extract_status=f"Found {archive_count} {profile.archive_format.upper()} archive(s).",
    )
    _record_extract_log(f"Found {archive_count} {profile.archive_format.upper()} archive(s).")

    if smart:
        existing = load_manifest(output_dir)
        if manifest_matches(existing, data_dir, archives, papyrus_source_dir):
            settings.set_game_extracted_dir(game, str(output_dir))
            _set_extract_state(
                extract_completed_archives=archive_count,
                extract_progress=1.0,
                extract_status=f"Smart extract skipped; {archive_count} archive(s) unchanged.",
            )
            _record_extract_log("Smart extract skipped; archives are unchanged.")
            return

    archive_groups = group_archives_by_update_phase(archives)
    _record_extract_log(f"Extracting with {workers} total worker(s).")
    if len(archive_groups) > 1:
        _record_extract_log(f"Using {len(archive_groups)} ordered overwrite phase(s).")
    errors: list[str] = []

    progress_marks: dict[str, int] = {}

    def _progress_callback(archive: Path):
        def _progress(event: dict) -> bool:
            completed_files = int(event.get("completed", 0) or 0)
            total_archive_files = int(event.get("total", 0) or 0)
            _set_extract_state(
                extract_status=(
                    f"Extracting {archive.name}: "
                    f"{completed_files:,}/{total_archive_files:,} file(s)."
                )
            )
            previous = progress_marks.get(archive.name, 0)
            if completed_files == total_archive_files or completed_files - previous >= 5_000:
                progress_marks[archive.name] = completed_files
                _record_extract_log(
                    f"{archive.name}: {completed_files:,}/{total_archive_files:,} file(s)..."
                )
            return True

        return _progress

    def _extract_archive(task):
        archive = task.archive
        details = f", {task.file_count:,} file(s)" if task.file_count else ""
        _record_extract_log(
            f"Extracting {archive.name} with {task.file_workers} file worker(s){details}..."
        )
        return extract_one(
            archive,
            output_dir,
            profile.archive_format,
            file_workers=task.file_workers,
            progress=_progress_callback(archive),
        )

    completed = 0
    total_files = 0
    for phase_idx, phase_archives in enumerate(archive_groups, start=1):
        batches = plan_archive_extraction_batches(phase_archives, workers)
        if len(archive_groups) > 1:
            _record_extract_log(
                f"Phase {phase_idx}/{len(archive_groups)}: "
                f"{len(phase_archives)} archive(s), {workers} total worker(s)."
            )
        for batch in batches:
            batch_workers = sum(task.file_workers for task in batch)
            if batch_workers > len(batch):
                names = ", ".join(f"{task.archive.name}={task.file_workers}" for task in batch)
                _record_extract_log(f"Worker batch: {names}")
            with ThreadPoolExecutor(max_workers=len(batch)) as pool:
                futures = {pool.submit(_extract_archive, task): task.archive for task in batch}
                for future in as_completed(futures):
                    archive = futures[future]
                    completed += 1
                    try:
                        _archive, count, error = future.result()
                    except Exception as exc:
                        count = 0
                        error = str(exc)

                    if error:
                        errors.append(f"{archive.name}: {error}")
                        _record_extract_log(f"[{completed}/{archive_count}] ERROR {archive.name}: {error}", level=logging.ERROR)
                    else:
                        total_files += int(count)
                        _record_extract_log(f"[{completed}/{archive_count}] {archive.name}: {int(count):,} files")

                    progress = completed / archive_count if archive_count else 1.0
                    _set_extract_state(
                        extract_completed_archives=completed,
                        extract_total_files=total_files,
                        extract_progress=progress,
                        extract_status=f"Extracted {completed}/{archive_count} archive(s), {total_files:,} file(s).",
                    )

    if errors:
        raise RuntimeError(f"{len(errors)} archive(s) failed. See the extraction log.")

    if papyrus_source_dir is not None:
        papyrus_files = sync_papyrus_sources(papyrus_source_dir, output_dir)
        _record_extract_log(f"Papyrus sources mirrored: {papyrus_files:,} file(s).")

    manifest = build_manifest(game, data_dir, archives, papyrus_source_dir)
    save_manifest(output_dir, manifest)
    settings.set_game_extracted_dir(game, str(output_dir))
    _set_extract_state(
        extract_progress=1.0,
        extract_completed_archives=archive_count,
        extract_total_files=total_files,
        extract_status=f"Extraction complete: {total_files:,} file(s) from {archive_count} archive(s).",
    )
    _record_extract_log(f"Extraction complete: {total_files:,} file(s) from {archive_count} archive(s).")
    _record_extract_log(f"Manifest saved: {output_dir / '.ba2_manifest.json'}")


def _count_yaml_plugins(yaml_dir) -> int:
    d = Path(yaml_dir)
    if not d.is_dir():
        return 0
    return sum(1 for p in d.iterdir() if p.is_dir())


def _draw_extract_run_status() -> None:
    from creation_lib.ui.widgets.modern import scaled, semantic_color

    scale = scaled(1)
    snapshot = _extract_snapshot()
    status = snapshot["status"] or "Extracting archives..."
    imgui.text_wrapped(status)

    if snapshot["total_archives"]:
        imgui.progress_bar(float(snapshot["progress"]), imgui.ImVec2(-1, 0))
        imgui.text_disabled(
            f"{snapshot['completed_archives']}/{snapshot['total_archives']} archives"
            f" | {snapshot['total_files']:,} files"
            f" | {snapshot['archive_workers']} extraction worker(s)"
        )
    else:
        imgui.progress_bar(-1.0 * imgui.get_time(), imgui.ImVec2(-1, 0))
        imgui.text_disabled(f"{snapshot['archive_workers']} extraction worker(s)")

    if snapshot["output_dir"]:
        imgui.text_disabled(f"Output: {snapshot['output_dir']}")

    if snapshot["error"]:
        imgui.push_style_color(imgui.Col_.text, semantic_color("error"))
        imgui.text_wrapped(snapshot["error"])
        imgui.pop_style_color()

    if snapshot["log_lines"]:
        imgui.spacing()
        imgui.text("Extraction log")
        imgui.begin_child("##archive_extract_log", imgui.ImVec2(-1, 120 * scale), True)
        for line in snapshot["log_lines"]:
            imgui.text_wrapped(line)
        imgui.end_child()


def _draw_game_selector(settings, scale: float = 1) -> str:
    game_labels = [lbl for _, lbl in _INDEX_GAMES]
    game_ids = [gid for gid, _ in _INDEX_GAMES]

    imgui.text("Game:")
    imgui.same_line()
    current_idx = game_ids.index(_state.index_game) if _state.index_game in game_ids else 0
    imgui.set_next_item_width(140 * scale)
    changed, new_idx = imgui.combo("##index_game", current_idx, game_labels)
    if changed:
        _state.index_game = game_ids[new_idx]
        _load_extract_status(settings)
    imgui.spacing()
    return _state.index_game


def _draw_archive_extraction(settings, game: str, *, show_heading: bool) -> None:
    from creation_lib.ui.widgets.modern import scaled, semantic_color

    scale = scaled(1)
    gp = settings.get_game_paths(game)
    extracted = gp.get("extracted_dir", "")
    game_root = gp.get("root_dir", "")
    has_extracted = bool(extracted and os.path.isdir(extracted))

    if show_heading:
        imgui.separator()
        imgui.spacing()
        imgui.text("Source Data")
        imgui.spacing()

    if not game_root:
        imgui.text_disabled(
            "Set the game install path in Paths tab to enable extraction."
        )
        return

    if not has_extracted:
        imgui.text_colored(
            semantic_color("warning"), "⚠ Loose files: not extracted"
        )
        imgui.text_disabled("  Extract the installed game archives to continue.")
        imgui.spacing()
    else:
        if _state.extract_up_to_date:
            imgui.text_colored(
                semantic_color("success"),
                f"✓ Loose files: {_state.extract_status}",
            )
        elif _state.extract_status == "Updates available":
            imgui.text_colored(
                semantic_color("warning"),
                f"⚠ Loose files: {_state.extract_status}",
            )
        else:
            imgui.text_disabled(f"Loose files: {_state.extract_status}")
        imgui.spacing()

    if _state.extracting:
        _draw_extract_run_status()
        return

    imgui.set_next_item_width(120 * scale)
    changed, workers = imgui.input_int(
        f"Extraction workers##archive_extract_workers_{game}",
        _state.extract_archive_workers,
    )
    if changed:
        _state.extract_archive_workers = _clamp_archive_workers(workers)
    if imgui.is_item_hovered():
        imgui.set_tooltip(
            "Total extraction worker budget, shared across archives and large archive internals."
        )

    if imgui.button(f"Smart Extract##{game}", imgui.ImVec2(140 * scale, 0)):
        _start_extraction(settings, game, game_root, smart=True)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Re-extract only if archives have changed since last run.")
    imgui.same_line()
    if imgui.button(f"Full Extract##{game}", imgui.ImVec2(120 * scale, 0)):
        _start_extraction(settings, game, game_root, smart=False)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Always re-extract all archives.")

    if _extract_snapshot()["log_lines"]:
        imgui.spacing()
        _draw_extract_run_status()


def _draw_extraction_only(ctx: SettingsContext) -> None:
    settings = ctx.settings
    game = _draw_game_selector(settings, ctx.scale)
    _draw_archive_extraction(settings, game, show_heading=True)


def _draw(ctx: SettingsContext) -> None:
    from creation_lib.ui.host import GAME_ESM_YAML_DIR as _GAME_ESM_YAML_DIR
    from creation_lib.ui.host import get_host

    settings = ctx.settings
    db_dir = get_host().get_db_dir()
    game = _draw_game_selector(settings)

    _INDEXES = [
        ("fo4_data", "Records", f"{game}_records.db", "Record data (weapons, NPCs, keywords…). Needed for Search."),
        ("scripts", "Papyrus Scripts", f"{game}_scripts.db", "Papyrus source index. Needed for script search and API browsing."),
        ("wiki", "Wiki", f"{game}_wiki.db", "Local wiki index. Needed for Papyrus, CK, and GECK wiki search."),
        ("nifs", "NIF Mesh Index", f"{game}_nifs.db", "Mesh file index. Requires extracted game files."),
        ("behaviors", "Havok Index", f"{game}_havok.db", "Havok asset index (behaviors, skeletons, animations). Requires extracted game files."),
        ("swf", "SWF Shape Library", f"{game}_swf_shapes.db", "Pipboy icon shape library. Requires extracted game files."),
        ("voice_reference", "Voice Reference", "", "Dialogue voice-line index for the Voice Browser. Uses installed game Data archives."),
    ]

    if _state.index_builder is not None:
        phase = _state.index_builder.phase
        progress = _state.index_builder.progress
        status = _state.index_builder.status
        imgui.text(f"Rebuilding... ({phase})")
        imgui.progress_bar(progress, imgui.ImVec2(-1, 0))
        imgui.text_wrapped(status)
        if _state.index_builder.done:
            if _state.index_builder.error:
                imgui.text_colored(imgui.ImVec4(1, 0.3, 0.3, 1), f"Error: {_state.index_builder.error}")
            else:
                imgui.text_colored(imgui.ImVec4(0.3, 0.9, 0.3, 1), "Rebuild complete.")
            _state.index_builder = None
        imgui.spacing()
        return

    local_vals = {
        "fo4_data": _state.index_fo4_data,
        "scripts": _state.index_scripts,
        "wiki": _state.index_wiki,
        "nifs": _state.index_nifs,
        "behaviors": _state.index_behaviors,
        "swf": _state.index_swf,
        "voice_reference": _state.index_voice_reference,
    }
    for key, label, db_file, desc in _INDEXES:
        if key == "voice_reference":
            exists, size_str = _voice_reference_index_status(game, settings)
        elif key == "wiki":
            from creation_lib.creation_data._db_resolver import db_available, get_db_path
            exists = db_available("wiki", game, str(db_dir))
            if exists:
                db_path = Path(get_db_path("wiki", game, str(db_dir)))
                size_str = f"{db_path.stat().st_size // (1024 * 1024)} MB"
                if db_path.name != db_file:
                    size_str = f"{size_str} ({db_path.name})"
            else:
                size_str = "Not built"
        else:
            db_path = db_dir / db_file
            exists = db_path.is_file()
            size_str = f"{db_path.stat().st_size // (1024 * 1024)} MB" if exists else "Not built"
        status_color = imgui.ImVec4(0.4, 0.8, 0.4, 1) if exists else imgui.ImVec4(0.6, 0.6, 0.6, 1)

        changed, new_val = imgui.checkbox(f"##{key}_enabled", local_vals[key])
        if changed:
            setattr(_state, f"index_{key}", new_val)
        imgui.same_line()
        imgui.text(label)
        imgui.same_line(250)
        imgui.text_colored(status_color, size_str)
        imgui.same_line(350)
        if imgui.button(f"Rebuild##{key}"):
            _start_rebuild(settings, only=key)
        imgui.text_disabled(f"  {desc}")
        imgui.spacing()

    imgui.separator()
    imgui.spacing()
    if imgui.button("Rebuild All", imgui.ImVec2(120, 0)):
        _start_rebuild(settings, only=None)

    game_root = settings.get_game_paths(game).get("root_dir", "")

    imgui.spacing()
    imgui.separator()
    imgui.spacing()
    imgui.text("Source Data")
    imgui.spacing()

    if not game_root:
        imgui.text_disabled("Set the game install path in Paths tab to enable extraction.")
        return

    yaml_subdir = _GAME_ESM_YAML_DIR.get(game, f"{game}_esm_yaml")
    yaml_dir = db_dir / yaml_subdir
    yaml_count = _count_yaml_plugins(yaml_dir)

    if yaml_count > 0:
        imgui.text_colored(imgui.ImVec4(0.3, 0.9, 0.3, 1), f"✓ YAML records: {yaml_count} plugin(s) extracted")
        second_label = f"Re-extract YAML##{game}"
        second_tooltip = "Delete existing YAML and re-extract all ESM/ESL plugins"
        second_width = 120
    else:
        imgui.text_colored(imgui.ImVec4(1.0, 0.8, 0.3, 1), "⚠ YAML records: not extracted")
        imgui.text_disabled("  Records index requires Spriggit YAML extraction from game ESM files.")
        second_label = f"Extract YAML##{game}"
        second_tooltip = "Serialize game ESM/ESL plugins to YAML (needed for Records index)."
        second_width = 120
    imgui.same_line(350)
    if imgui.button(f"Smart YAML##{game}", imgui.ImVec2(140, 0)):
        _start_yaml_extraction(settings, game, game_root, smart=True)
    if imgui.is_item_hovered():
        imgui.set_tooltip("Extract only new or updated ESM plugins and add their records to the existing index.")
    imgui.same_line()
    if imgui.button(second_label, imgui.ImVec2(second_width, 0)):
        _start_yaml_extraction(settings, game, game_root, force=True)
    if imgui.is_item_hovered():
        imgui.set_tooltip(second_tooltip)
    imgui.spacing()

    _draw_archive_extraction(settings, game, show_heading=False)


def _load(saved: dict) -> None:
    _state.index_fo4_data = bool(saved.get("fo4_data", True))
    _state.index_scripts = bool(saved.get("scripts", True))
    _state.index_wiki = bool(saved.get("wiki", True))
    _state.index_nifs = bool(saved.get("nifs", True))
    _state.index_behaviors = bool(saved.get("behaviors", True))
    _state.index_swf = bool(saved.get("swf", True))
    _state.index_voice_reference = bool(saved.get("voice_reference", True))
    _state.extract_archive_workers = _clamp_archive_workers(
        saved.get("extract_archive_workers", _default_archive_workers())
    )


def _save() -> dict:
    return {
        "fo4_data": _state.index_fo4_data,
        "scripts": _state.index_scripts,
        "wiki": _state.index_wiki,
        "nifs": _state.index_nifs,
        "behaviors": _state.index_behaviors,
        "swf": _state.index_swf,
        "voice_reference": _state.index_voice_reference,
        "extract_archive_workers": _state.extract_archive_workers,
    }


def _load_extraction_only(saved: dict) -> None:
    _state.extract_archive_workers = _clamp_archive_workers(
        saved.get("extract_archive_workers", _default_archive_workers())
    )


def _save_extraction_only() -> dict:
    return {"extract_archive_workers": _state.extract_archive_workers}


def make_section(*, extraction_only: bool = False) -> SettingsSection:
    if extraction_only:
        return SettingsSection(
            id="indexes",
            label="Extraction",
            draw=_draw_extraction_only,
            load=_load_extraction_only,
            save=_save_extraction_only,
        )
    return SettingsSection(id="indexes", label="Indexes", draw=_draw, load=_load, save=_save)
