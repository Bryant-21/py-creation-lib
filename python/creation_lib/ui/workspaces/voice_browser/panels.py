"""Voice Browser panels."""
from __future__ import annotations

import hashlib

from imgui_bundle import imgui

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.ui.widgets.audio_player import _format_time

_NS = "##voice_browser"


def draw_groups_panel(ws) -> None:
    if not imgui.begin(f"Voices{_NS}"):
        imgui.end()
        return

    controls_disabled = ws._busy
    if controls_disabled:
        imgui.begin_disabled()
    game_labels = [GAME_PROFILES[game].display_name for game in ws._games]
    imgui.text_disabled("Game")
    imgui.set_next_item_width(-1)
    changed, ws._game_idx = imgui.combo(f"##game{_NS}", ws._game_idx, game_labels)
    if changed:
        ws._index = None
        ws._selected_group = ""
        ws._selected_plugin = ""
        ws._selected_line_idx = -1
        ws._dirty_filter = True
        ws._load_attempt_key = None
        ws._next_cache_load_time = 0.0
        ws._status_msg = "Loading cached voice reference..."
        ws._error_msg = ""
        ws._result_msg = ""

    plugin_options = ws._plugin_filter_options()
    if ws._selected_plugin and ws._selected_plugin not in plugin_options:
        ws._selected_plugin = ""
        ws._dirty_filter = True
    plugin_idx = max(
        0,
        plugin_options.index(ws._selected_plugin) if ws._selected_plugin in plugin_options else 0,
    )
    imgui.text_disabled("Plugin")
    imgui.set_next_item_width(-1)
    changed_plugin, plugin_idx = imgui.combo(f"##plugin{_NS}", plugin_idx, plugin_options)
    if changed_plugin:
        ws._selected_plugin = "" if plugin_idx <= 0 else plugin_options[plugin_idx]
        ws._selected_group = ""
        ws._selected_line_idx = -1
        ws._dirty_filter = True

    changed_group_query, ws._group_query = imgui.input_text_with_hint(
        f"##filter_voices{_NS}",
        "Filter voices",
        ws._group_query,
    )
    if changed_group_query:
        ws._selected_line_idx = -1

    changed_available, ws._available_only = imgui.checkbox(f"Available only{_NS}", ws._available_only)
    if changed_available:
        ws._dirty_filter = True
    if controls_disabled:
        imgui.end_disabled()

    imgui.separator()
    ws._draw_status_summary()
    imgui.separator()
    if ws._index is None:
        imgui.text_wrapped("No cached voice reference is loaded.")
        ws._end_panel_with_loading_mask()
        return

    ws._ensure_filtered()
    from .workspace import _filter_workspace_voice_lines
    all_line_count = len(
        _filter_workspace_voice_lines(
            ws._index,
            ws._query,
            plugin=ws._selected_plugin,
            available_only=ws._available_only,
        )
    )
    label = f"All voice lines ({all_line_count})"
    if imgui.selectable(label, ws._selected_group == "")[0]:
        ws._selected_group = ""
        ws._selected_line_idx = -1
        ws._dirty_filter = True

    for group, count in ws._filtered_groups():
        group_id = hashlib.sha1(group.encode("utf-8", errors="ignore")).hexdigest()[:12]
        if imgui.selectable(f"{group} ({count})##group_{group_id}", ws._selected_group == group)[0]:
            ws._selected_group = group
            ws._selected_line_idx = -1
            ws._dirty_filter = True
        ws._draw_group_context_menu(group, count, group_id)

    ws._end_panel_with_loading_mask()


def draw_lines_panel(ws) -> None:
    flags = imgui.WindowFlags_.no_scrollbar | imgui.WindowFlags_.no_scroll_with_mouse
    if not imgui.begin(f"Voice Lines{_NS}", flags=flags):
        imgui.end()
        return

    changed, ws._query = imgui.input_text_with_hint(f"Search{_NS}", "Search voice lines", ws._query)
    if changed:
        ws._dirty_filter = True
        ws._selected_line_idx = -1

    imgui.same_line()
    imgui.text_disabled(f"{len(ws._results):,} lines" if ws._index else "No index")

    ws._ensure_filtered()
    table_height = max(120.0, imgui.get_content_region_avail().y - 48.0)
    if imgui.begin_table(
        f"voice_lines_table{_NS}",
        4,
        imgui.TableFlags_.row_bg | imgui.TableFlags_.resizable | imgui.TableFlags_.scroll_y,
        imgui.ImVec2(0, table_height),
    ):
        imgui.table_setup_column("Text", imgui.TableColumnFlags_.width_stretch, 0.62)
        imgui.table_setup_column("Voice", imgui.TableColumnFlags_.width_stretch, 0.18)
        imgui.table_setup_column("Plugin", imgui.TableColumnFlags_.width_fixed, 110)
        imgui.table_setup_column("File", imgui.TableColumnFlags_.width_fixed, 120)
        imgui.table_headers_row()

        clipper = imgui.ListClipper()
        clipper.begin(len(ws._results))
        while clipper.step():
            for row_idx in range(clipper.display_start, clipper.display_end):
                line = ws._results[row_idx]
                imgui.table_next_row()
                imgui.table_next_column()
                selected = row_idx == ws._selected_line_idx
                text = line.response_text or "(blank response)"
                if not line.available:
                    text = f"[missing] {text}"
                clicked = imgui.selectable(
                    f"{text}##line_{row_idx}",
                    selected,
                    imgui.SelectableFlags_.span_all_columns,
                )[0]
                if clicked:
                    ws._selected_line_idx = row_idx
                if clicked and imgui.is_mouse_double_clicked(0):
                    ws._toggle_preview()
                imgui.table_next_column()
                imgui.text_unformatted(line.voice_type or "-")
                imgui.table_next_column()
                imgui.text_unformatted(line.plugin)
                imgui.table_next_column()
                imgui.text_unformatted(line.response_filename)
        clipper.end()
        imgui.end_table()

    imgui.separator()
    ws._draw_status_summary()

    ws._end_panel_with_loading_mask()


def draw_preview_panel(ws) -> None:
    if not imgui.begin(f"Preview{_NS}"):
        imgui.end()
        return

    line = ws._selected_line
    if line is None:
        imgui.text_wrapped("Select a voice line.")
        ws._end_panel_with_loading_mask()
        return

    imgui.text_wrapped(line.response_text or "(blank response)")
    imgui.separator()
    imgui.text(f"Voice: {line.voice_type or '-'}")
    imgui.text(f"Plugin: {line.plugin}")
    imgui.text(f"INFO: {line.info_form_id}")
    imgui.text(f"File: {line.response_filename}")
    if line.member_path:
        imgui.text_wrapped(line.member_path)

    if not line.available:
        imgui.spacing()
        imgui.text_wrapped("No matching archive member was found for this response.")
        ws._end_panel_with_loading_mask()
        return

    imgui.spacing()
    if imgui.button(f"Play / Pause{_NS}", imgui.ImVec2(-1, 0)):
        ws._toggle_preview()
    if imgui.button(f"Stop{_NS}", imgui.ImVec2(-1, 0)):
        ws._audio_player.stop()

    if ws._audio_player.has_audio:
        pos = ws._audio_player._play_pos
        changed, pos = imgui.slider_float(
            f"{_format_time(pos * ws._audio_player._duration)} / {_format_time(ws._audio_player._duration)}{_NS}",
            pos,
            0.0,
            1.0,
        )
        if changed:
            ws._audio_player.seek(pos)

    imgui.separator()
    if imgui.button(f"Export FUZ{_NS}", imgui.ImVec2(-1, 0)):
        ws._export_selected("fuz")
    if imgui.button(f"Export WAV{_NS}", imgui.ImVec2(-1, 0)):
        ws._export_selected("wav")
    if imgui.button(f"Export WAV + LIP{_NS}", imgui.ImVec2(-1, 0)):
        ws._export_selected("wav_lip")

    for action in ws._extra_actions:
        if imgui.button(action.label):
            action.handler(ws._current_line, ws._current_wav_path)
        if action.tooltip and imgui.is_item_hovered():
            imgui.set_tooltip(action.tooltip)

    ws._end_panel_with_loading_mask()
