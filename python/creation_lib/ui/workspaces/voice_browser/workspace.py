"""Voice Browser workspace."""

from __future__ import annotations

import hashlib
import logging
import queue
import re
import shutil
import subprocess
import tempfile
import threading
import time
from pathlib import Path

from imgui_bundle import imgui

from creation_lib.audio.voice_reference import (
    VoiceLine,
    VoiceReferenceIndex,
    extract_voice_line,
    load_cached_voice_reference,
)
from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.ui.widgets.audio_player import InlineAudioPlayer as _InlineAudioPlayer
from creation_lib.ui.shell import BaseWorkspace, make_window
from creation_lib.ui.widgets import pick_folder

_log = logging.getLogger("toolkit.voice_browser")
_NS = "##voice_browser"


class VoiceBrowserWorkspace(BaseWorkspace):
    name = "Voice Browser"
    icon = "VOC"
    id = "voice_browser"
    user_guide_body = """
Load the cached voice reference for a game, then filter by plugin, voice type, or text.
Preview lines, inspect their archive members, and export audio from the results list.
"""

    def __init__(
        self,
        toolkit_settings=None,
        *,
        on_select=None,
        extra_actions=None,
    ):
        super().__init__(toolkit_settings)
        self._on_select = on_select
        self._extra_actions = extra_actions or []
        self._current_line: VoiceLine | None = None
        self._current_wav_path: str = ""
        self._games = list(GAME_PROFILES.keys())
        self._game_idx = max(0, self._games.index("fo4") if "fo4" in self._games else 0)
        self._language = "English"
        self._query = ""
        self._group_query = ""
        self._selected_group = ""
        self._selected_plugin = ""
        self._available_only = False
        self._index: VoiceReferenceIndex | None = None
        self._results: list[VoiceLine] = []
        self._groups: list[tuple[str, int]] = []
        self._selected_line_idx = -1
        self._dirty_filter = True
        self._busy = False
        self._job_label = ""
        self._last_progress_log = ""
        self._load_attempt_key: tuple[str, str] | None = None
        self._next_cache_load_time = 0.0
        self._progress = 0.0
        self._status_msg = "Loading cached voice reference..."
        self._error_msg = ""
        self._result_msg = ""
        self._jobs: queue.Queue[tuple[str, object]] = queue.Queue()
        self._audio_player = _InlineAudioPlayer()
        self._audio_loaded_key: tuple[str, str] | None = None
        self._preview_root = Path(tempfile.gettempdir()) / "modbox_voice_browser"

    def get_dockable_windows(self):
        return [
            make_window(f"Voices{_NS}", "LeftDock"),
            make_window(f"Voice Lines{_NS}", "MainDockSpace"),
            make_window(f"Preview{_NS}", "RightDock"),
        ]

    def initialize(self) -> None:
        self._initialized = True
        if self._pending_settings:
            self._apply_saved_settings(self._pending_settings)
            self._pending_settings = None
        self._bind_panels(
            {
                f"Voices{_NS}": self._draw_groups_panel,
                f"Voice Lines{_NS}": self._draw_lines_panel,
                f"Preview{_NS}": self._draw_preview_panel,
            }
        )

    def draw_menu(self) -> None:
        if self._view_helper:
            self._view_helper.draw([f"Voices{_NS}", f"Voice Lines{_NS}", f"Preview{_NS}"])

    def draw(self) -> None:
        if not self.active or not self._initialized:
            return
        self._poll_jobs()
        self._maybe_start_cached_index_load()
        self._audio_player.update()

    def cleanup(self) -> None:
        self._audio_player.clear()
        try:
            shutil.rmtree(self._preview_root, ignore_errors=True)
        except Exception:
            pass

    def get_settings_defaults(self) -> dict:
        return {
            "game": "fo4",
            "plugin": "",
            "available_only": False,
        }

    def apply_settings(self, settings: dict) -> None:
        if self._initialized:
            self._apply_saved_settings(settings)
        else:
            self._pending_settings = settings

    def collect_settings(self) -> dict:
        return {
            "game": self._games[self._game_idx] if self._games else "fo4",
            "plugin": self._selected_plugin,
            "available_only": self._available_only,
        }

    def _apply_saved_settings(self, settings: dict) -> None:
        game = str(settings.get("game", "fo4"))
        if game in self._games:
            self._game_idx = self._games.index(game)
        self._language = "English"
        self._selected_plugin = str(settings.get("plugin", "") or "")
        self._available_only = bool(settings.get("available_only", False))

    @property
    def _selected_game(self) -> str:
        if not self._games:
            return "fo4"
        return self._games[max(0, min(self._game_idx, len(self._games) - 1))]

    @property
    def _selected_line(self) -> VoiceLine | None:
        if 0 <= self._selected_line_idx < len(self._results):
            return self._results[self._selected_line_idx]
        return None

    def _draw_groups_panel(self) -> None:
        from .panels import draw_groups_panel
        draw_groups_panel(self)

    def _draw_lines_panel(self) -> None:
        from .panels import draw_lines_panel
        draw_lines_panel(self)

    def _filtered_groups(self) -> list[tuple[str, int]]:
        needle = self._group_query.strip().lower()
        if not needle:
            return self._groups
        return [(group, count) for group, count in self._groups if needle in group.lower()]

    def _draw_group_context_menu(self, group: str, count: int, group_id: str) -> None:
        if not imgui.begin_popup_context_item(f"group_ctx_{group_id}{_NS}"):
            return
        try:
            imgui.text_disabled(f"{group} ({count})")
            imgui.separator()
            if imgui.menu_item("Export Voice FUZ", "", False, True)[0]:
                self._export_group(group, "fuz")
            if imgui.menu_item("Export Voice WAV", "", False, True)[0]:
                self._export_group(group, "wav")
            if imgui.menu_item("Export Voice WAV + LIP", "", False, True)[0]:
                self._export_group(group, "wav_lip")
        finally:
            imgui.end_popup()

    def _draw_preview_panel(self) -> None:
        from .panels import draw_preview_panel
        draw_preview_panel(self)

    def _draw_status_summary(self) -> None:
        if self._busy:
            imgui.progress_bar(self._progress, imgui.ImVec2(-1, 0), self._job_label or "Working...")
        elif self._error_msg:
            imgui.push_style_color(imgui.Col_.text, imgui.ImVec4(1.0, 0.35, 0.35, 1.0))
            imgui.text_wrapped(self._error_msg)
            imgui.pop_style_color()
        elif self._result_msg:
            imgui.push_style_color(imgui.Col_.text, imgui.ImVec4(0.45, 0.9, 0.45, 1.0))
            imgui.text_wrapped(self._result_msg)
            imgui.pop_style_color()
        else:
            imgui.text_disabled(self._status_msg)

    def _end_panel_with_loading_mask(self) -> None:
        if self._busy:
            self._draw_loading_mask()
        imgui.end()

    def _draw_loading_mask(self) -> None:
        pos = imgui.get_window_pos()
        size = imgui.get_window_size()
        draw_list = imgui.get_foreground_draw_list()
        min_pos = imgui.ImVec2(pos.x, pos.y)
        max_pos = imgui.ImVec2(pos.x + size.x, pos.y + size.y)
        draw_list.add_rect_filled(
            min_pos,
            max_pos,
            imgui.color_convert_float4_to_u32(imgui.ImVec4(0.0, 0.0, 0.0, 0.55)),
        )

        spinner = "|/-\\"[int(imgui.get_time() * 8.0) % 4]
        label = self._job_label or self._status_msg or "Working..."
        percent = int(max(0.0, min(1.0, self._progress)) * 100.0)
        text = f"{spinner}  {label} ({percent}%)"
        text_size = imgui.calc_text_size(text)
        pad_x = 18.0
        pad_y = 14.0
        panel_w = max(120.0, min(max(120.0, size.x - 24.0), max(260.0, text_size.x + pad_x * 2.0)))
        panel_h = text_size.y + pad_y * 2.0
        panel_min = imgui.ImVec2(pos.x + (size.x - panel_w) * 0.5, pos.y + (size.y - panel_h) * 0.5)
        panel_max = imgui.ImVec2(panel_min.x + panel_w, panel_min.y + panel_h)
        draw_list.add_rect_filled(
            panel_min,
            panel_max,
            imgui.color_convert_float4_to_u32(imgui.ImVec4(0.10, 0.11, 0.12, 0.95)),
            6.0,
        )
        draw_list.add_rect(
            panel_min,
            panel_max,
            imgui.color_convert_float4_to_u32(imgui.ImVec4(0.35, 0.40, 0.48, 1.0)),
            6.0,
        )
        draw_list.add_text(
            imgui.ImVec2(panel_min.x + pad_x, panel_min.y + pad_y),
            imgui.color_convert_float4_to_u32(imgui.ImVec4(0.92, 0.95, 1.0, 1.0)),
            text,
        )

    def _ensure_filtered(self) -> None:
        if not self._dirty_filter:
            return
        if self._index is None:
            self._results = []
            self._groups = []
        else:
            self._groups = _group_workspace_voice_lines(
                self._index,
                self._query,
                plugin=self._selected_plugin,
                available_only=self._available_only,
            )
            self._results = _filter_workspace_voice_lines(
                self._index,
                self._query,
                group=self._selected_group,
                plugin=self._selected_plugin,
                available_only=self._available_only,
            )
        if self._selected_line_idx >= len(self._results):
            self._selected_line_idx = -1
        self._dirty_filter = False

    def _maybe_start_cached_index_load(self) -> None:
        if self._busy or self._index is not None:
            return
        key = (self._selected_game, self._language)
        now = time.monotonic()
        if self._load_attempt_key == key and now < self._next_cache_load_time:
            return
        self._start_cached_index_load(key)

    def _start_cached_index_load(self, key: tuple[str, str]) -> None:
        if self._busy:
            return
        try:
            data_dir, strings_dir = self._resolve_game_paths()
        except Exception as exc:
            self._error_msg = str(exc)
            self._next_cache_load_time = time.monotonic() + 5.0
            _log.error("Voice reference cache load could not start: %s", exc)
            return

        game = self._selected_game
        language = self._language
        from creation_lib.ui.host import get_host

        db_dir = get_host().get_db_dir()
        _log.info(
            "Loading cached voice reference: game=%s data_dir=%s strings_dir=%s db_dir=%s language=%s",
            game,
            data_dir,
            strings_dir,
            db_dir,
            language,
        )
        self._busy = True
        self._progress = 0.0
        self._job_label = "Loading voice reference..."
        self._last_progress_log = self._job_label
        self._load_attempt_key = key
        self._error_msg = ""
        self._result_msg = ""

        def _worker() -> None:
            try:
                index = load_cached_voice_reference(
                    game=game,
                    data_dir=data_dir,
                    strings_dir=strings_dir,
                    db_dir=db_dir,
                    language=language,
                )
                self._jobs.put(("index", (key, index)) if index is not None else ("no_index", key))
            except Exception as exc:
                _log.exception("Voice reference cache load failed")
                self._jobs.put(("error", str(exc)))

        threading.Thread(target=_worker, daemon=True).start()

    def _start_preview(self, line: VoiceLine) -> None:
        if self._busy:
            return
        self._busy = True
        self._job_label = f"Preparing {line.response_filename}"
        self._progress = 0.0
        self._error_msg = ""
        key = (line.archive_path, line.member_path)

        def _worker() -> None:
            try:
                preview_path = self._prepare_preview_file(line)
                self._jobs.put(("preview", (key, preview_path)))
            except Exception as exc:
                _log.exception("Voice preview failed")
                self._jobs.put(("error", str(exc)))

        threading.Thread(target=_worker, daemon=True).start()

    def _start_group_export(self, group: str, lines: list[VoiceLine], target_dir: Path, mode: str) -> None:
        if self._busy:
            return
        self._busy = True
        self._progress = 0.0
        self._job_label = f"Exporting {group}..."
        self._error_msg = ""
        self._result_msg = ""

        def _worker() -> None:
            exported = 0
            try:
                total = len(lines)
                group_root = target_dir / _safe_path_component(group)
                for idx, line in enumerate(lines, start=1):
                    self._jobs.put(("progress", (idx - 1, total, f"Exporting {group}: {line.response_filename}")))
                    line_dir = (
                        group_root
                        / _safe_path_component(line.plugin)
                        / _safe_path_component(line.voice_type or "Unknown Voice")
                    )
                    line_dir.mkdir(parents=True, exist_ok=True)
                    extracted = extract_voice_line(line, line_dir)
                    if mode in {"wav", "wav_lip"}:
                        self._prepare_playable_audio(extracted, line_dir, keep_lip=mode == "wav_lip")
                    exported += 1
                self._jobs.put(("progress", (total, total, f"Exported {exported:,} line(s)")))
                self._jobs.put(("export_group", (group, exported, str(group_root))))
            except Exception as exc:
                _log.exception("Voice export failed")
                self._jobs.put(("error", str(exc)))

        threading.Thread(target=_worker, daemon=True).start()

    def _poll_jobs(self) -> None:
        while True:
            try:
                kind, payload = self._jobs.get_nowait()
            except queue.Empty:
                return
            if kind == "progress":
                current, total, message = payload  # type: ignore[misc]
                self._progress = float(current) / max(1.0, float(total))
                self._job_label = str(message)
                if self._job_label != self._last_progress_log:
                    _log.info(
                        "Voice reference progress: %s (%d/%d)",
                        self._job_label,
                        int(current),
                        int(total),
                    )
                    self._last_progress_log = self._job_label
            elif kind == "index":
                key, index = payload  # type: ignore[misc]
                if key != (self._selected_game, self._language):
                    self._busy = False
                    continue
                self._index = index
                self._busy = False
                self._progress = 1.0
                self._dirty_filter = True
                self._selected_group = ""
                if self._selected_plugin and self._selected_plugin not in self._plugin_filter_options():
                    self._selected_plugin = ""
                self._selected_line_idx = -1
                count = len(self._index.lines) if self._index else 0
                self._result_msg = f"Loaded {count:,} voice line(s)."
                self._status_msg = self._result_msg
                _log.info("Voice reference cache loaded: %d line(s)", count)
            elif kind == "no_index":
                if payload != (self._selected_game, self._language):
                    self._busy = False
                    continue
                self._busy = False
                self._progress = 0.0
                self._next_cache_load_time = time.monotonic() + 5.0
                self._status_msg = "No voice reference index found. Build it from Settings > Indexes."
            elif kind == "preview":
                key, preview_path = payload  # type: ignore[misc]
                self._busy = False
                self._load_preview(key, Path(str(preview_path)))
            elif kind == "export_group":
                group, exported, output_dir = payload  # type: ignore[misc]
                self._busy = False
                self._progress = 1.0
                self._result_msg = f"Exported {int(exported):,} line(s) for {group} to {output_dir}."
                self._status_msg = self._result_msg
            elif kind == "error":
                self._busy = False
                self._error_msg = str(payload)
                self._next_cache_load_time = time.monotonic() + 5.0
                _log.error("Voice reference job failed: %s", self._error_msg)

    def _resolve_game_paths(self) -> tuple[Path, Path]:
        if self._toolkit_settings is None:
            raise RuntimeError("Toolkit settings are unavailable.")
        paths = self._toolkit_settings.get_game_paths(self._selected_game)
        root_value = str(paths.get("root_dir", "") or "").strip()
        if not root_value:
            raise RuntimeError(f"No root directory configured for {GAME_PROFILES[self._selected_game].display_name}.")
        root_dir = Path(root_value).expanduser()
        data_dir = root_dir / "Data"
        if not data_dir.is_dir() and root_dir.name.lower() == "data":
            data_dir = root_dir
        if not data_dir.is_dir():
            raise RuntimeError(f"Data directory not found: {data_dir}")

        strings_dir = data_dir / "Strings"
        if not strings_dir.is_dir():
            extracted = Path(str(paths.get("extracted_dir", "") or "")).expanduser()
            for candidate in (extracted / "Strings", extracted / "Data" / "Strings"):
                if candidate.is_dir():
                    strings_dir = candidate
                    break
        _log.info(
            "Resolved voice browser paths: game=%s data_dir=%s strings_dir=%s",
            self._selected_game,
            data_dir,
            strings_dir,
        )
        return data_dir.resolve(strict=False), strings_dir.resolve(strict=False)

    def _toggle_preview(self) -> None:
        line = self._selected_line
        if line is None or not line.available:
            return
        key = (line.archive_path, line.member_path)
        if self._audio_loaded_key == key and self._audio_player.has_audio:
            if self._audio_player.is_playing:
                self._audio_player.pause()
            else:
                self._audio_player.play()
            return
        self._start_preview(line)

    def _prepare_preview_file(self, line: VoiceLine) -> Path:
        cache_id = hashlib.sha1(f"{line.archive_path}|{line.member_path}".encode("utf-8")).hexdigest()[:16]
        target_dir = self._preview_root / cache_id
        target_dir.mkdir(parents=True, exist_ok=True)
        extracted = extract_voice_line(line, target_dir)
        return self._prepare_playable_audio(extracted, target_dir, keep_lip=False)

    def _prepare_playable_audio(self, path: Path, output_dir: Path, *, keep_lip: bool) -> Path:
        suffix = path.suffix.lower()
        if suffix == ".wav":
            return path
        if suffix == ".xwm":
            return self._convert_xwm_to_wav(path, output_dir)
        if suffix == ".fuz":
            return self._extract_fuz_to_wav(path, output_dir, keep_lip=keep_lip)
        if suffix in {".ogg", ".wem"}:
            return self._convert_audio_to_wav(path, output_dir)
        raise ValueError(f"Unsupported voice audio format: {path.suffix}")

    def _convert_xwm_to_wav(self, xwm_path: Path, output_dir: Path) -> Path:
        from creation_lib.paths import get_resource_dir as get_creation_lib_resource_dir

        tool = get_creation_lib_resource_dir() / "xWMAEncode.exe"
        if not tool.is_file():
            raise FileNotFoundError(f"xWMAEncode.exe not found: {tool}")
        wav_path = output_dir / f"{xwm_path.stem}.wav"
        result = subprocess.run([str(tool), str(xwm_path), str(wav_path)], capture_output=True, text=True, check=False)
        if result.returncode != 0 or not wav_path.is_file():
            detail = result.stderr.strip() or "XWM decode failed"
            raise RuntimeError(detail)
        return wav_path

    def _convert_audio_to_wav(self, audio_path: Path, output_dir: Path) -> Path:
        ffmpeg_path = shutil.which("ffmpeg")
        if not ffmpeg_path:
            raise FileNotFoundError("ffmpeg not found on PATH")
        wav_path = output_dir / f"{audio_path.stem}.wav"
        result = subprocess.run(
            [ffmpeg_path, "-y", "-i", str(audio_path), "-c:a", "pcm_s16le", str(wav_path)],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0 or not wav_path.is_file():
            detail = result.stderr.strip() or f"Audio decode failed for {audio_path.name}"
            raise RuntimeError(detail)
        return wav_path

    def _extract_fuz_to_wav(self, fuz_path: Path, output_dir: Path, *, keep_lip: bool) -> Path:
        from creation_lib.paths import get_resource_dir
        fuz_decode = Path(get_resource_dir()) / "BmlFuzDecode.exe"
        if not fuz_decode.is_file():
            raise FileNotFoundError(f"BmlFuzDecode.exe not found: {fuz_decode}")
        with tempfile.TemporaryDirectory(prefix="modbox_voice_fuz_") as tmp:
            tmp_dir = Path(tmp)
            temp_fuz = tmp_dir / fuz_path.name
            shutil.copy2(fuz_path, temp_fuz)
            result = subprocess.run([str(fuz_decode), str(temp_fuz)], capture_output=True, text=True, check=False)
            if result.returncode != 0:
                detail = result.stderr.strip() or "FUZ decode failed"
                raise RuntimeError(detail)
            temp_xwm = tmp_dir / f"{fuz_path.stem}.xwm"
            if not temp_xwm.is_file():
                raise RuntimeError(f"FUZ decode did not produce XWM for {fuz_path.name}")
            if keep_lip:
                temp_lip = tmp_dir / f"{fuz_path.stem}.lip"
                if temp_lip.is_file():
                    shutil.copy2(temp_lip, output_dir / temp_lip.name)
            return self._convert_xwm_to_wav(temp_xwm, output_dir)

    def _load_preview(self, key: tuple[str, str], preview_path: Path) -> None:
        try:
            self._audio_player.load_file(str(preview_path), Path(key[1]).name)
            self._audio_loaded_key = key
            self._current_line = self._selected_line
            self._current_wav_path = str(preview_path)
            self._audio_player.play(from_pos=0.0)
            if self._on_select is not None and self._current_wav_path:
                self._on_select(self._current_line, self._current_wav_path)
        except Exception as exc:
            self._audio_loaded_key = None
            self._error_msg = str(exc)

    def _export_selected(self, mode: str) -> None:
        line = self._selected_line
        if line is None or not line.available:
            return
        target_dir = pick_folder("Export voice line to")
        if not target_dir:
            return
        try:
            extracted = extract_voice_line(line, target_dir)
            if mode in {"wav", "wav_lip"}:
                self._prepare_playable_audio(extracted, Path(target_dir), keep_lip=mode == "wav_lip")
            self._result_msg = f"Exported {line.response_filename}."
            self._error_msg = ""
        except Exception as exc:
            self._error_msg = str(exc)

    def _export_group(self, group: str, mode: str) -> None:
        if self._index is None:
            return
        lines = _filter_workspace_voice_lines(
            self._index,
            group=group,
            plugin=self._selected_plugin,
            available_only=True,
        )
        if not lines:
            self._result_msg = ""
            self._error_msg = f"No available voice files found for {group}."
            return
        target_dir = pick_folder(f"Export {group} voice lines to")
        if not target_dir:
            return
        self._start_group_export(group, lines, Path(target_dir), mode)

    def _plugin_filter_options(self) -> list[str]:
        if self._index is None:
            return ["All plugins"]
        plugins = sorted({line.plugin for line in self._index.lines if line.plugin}, key=str.lower)
        return ["All plugins", *plugins]


def _safe_path_component(value: str) -> str:
    cleaned = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', "_", value.strip())
    cleaned = cleaned.strip(" .")
    return cleaned or "Unknown"


def _filter_workspace_voice_lines(
    index: VoiceReferenceIndex,
    query: str = "",
    *,
    group: str = "",
    plugin: str = "",
    available_only: bool = False,
) -> list[VoiceLine]:
    needle = query.strip().lower()
    group_key = group.strip().lower()
    plugin_key = plugin.strip().lower()
    lines: list[VoiceLine] = []
    for line in index.lines:
        if available_only and not line.available:
            continue
        if plugin_key and line.plugin.lower() != plugin_key:
            continue
        if group_key and _voice_group_label(line).lower() != group_key:
            continue
        if needle and needle not in _workspace_line_text(line):
            continue
        lines.append(line)
    return lines


def _group_workspace_voice_lines(
    index: VoiceReferenceIndex,
    query: str = "",
    *,
    plugin: str = "",
    available_only: bool = False,
) -> list[tuple[str, int]]:
    counts: dict[str, int] = {}
    for line in _filter_workspace_voice_lines(index, query, plugin=plugin, available_only=available_only):
        label = _voice_group_label(line)
        counts[label] = counts.get(label, 0) + 1
    return sorted(counts.items(), key=lambda item: item[0].lower())


def _workspace_line_text(line: VoiceLine) -> str:
    return " ".join([line.response_text, line.topic_text]).lower()


def _voice_group_label(line: VoiceLine) -> str:
    return line.voice_type.strip() or "Unknown Voice"
