"""Paths settings section (per-game roots, extracted dirs, script sources)."""
from __future__ import annotations

import logging
import os

from imgui_bundle import imgui

from creation_lib.ui.shell.settings_section import SettingsContext, SettingsSection
from creation_lib.ui.widgets.pick_folder import pick_file, pick_folder

_log = logging.getLogger("toolkit.settings.paths")

_GAME_TABS = [
    ("fo4", "Fallout 4"),
    ("skyrimse", "Skyrim SE"),
    ("fo76", "Fallout 76"),
    ("starfield", "Starfield"),
    ("fo3", "Fallout 3"),
    ("fnv", "Fallout: NV"),
]


class _State:
    game_paths: dict = {}   # {game_id: {"root": str, "extracted": str, ...}}
    script_sources: list = []
    fo4_installs: list | None = None  # extra FO4 deploy targets; None = not yet loaded


_state = _State()


def _pick_folder(title: str = "Select Folder") -> str | None:
    try:
        return pick_folder(title)
    except Exception as e:
        _log.warning("Folder picker failed: %s", e)
        return None


def _pick_file(title: str = "Select File", filetypes: list | None = None) -> str | None:
    try:
        return pick_file(title, filetypes)
    except Exception as e:
        _log.warning("File picker failed: %s", e)
        return None


def _draw_game_paths(game_id: str, scale: float | None = None) -> None:
    scale = scale or 1
    gp = _state.game_paths.setdefault(
        game_id,
        {
            "root": "",
            "pts_root": "",
            "extracted": "",
            "additional": [],
            "content_resources_zip": "",
            "scripts_user_dir": "",
            "scripts_base_dir": "",
        },
    )

    imgui.spacing()
    imgui.text("Game Root")
    imgui.set_next_item_width(-100 * scale)
    changed, val = imgui.input_text(f"##root_{game_id}", gp["root"])
    if changed:
        gp["root"] = val
    imgui.same_line()
    if imgui.button(f"Browse##root_{game_id}"):
        path = _pick_folder("Select Game Root Directory")
        if path:
            gp["root"] = path

    if game_id == "fo76":
        imgui.spacing()
        imgui.text("Public Test Server Root (optional)")
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Optional Fallout 76 Playtest install. The retail install above "
                "remains required for ownership verification."
            )
        imgui.set_next_item_width(-100 * scale)
        changed, val = imgui.input_text(
            "##pts_root_fo76", gp.get("pts_root", "")
        )
        if changed:
            gp["pts_root"] = val
        imgui.same_line()
        if imgui.button("Browse##pts_root_fo76"):
            path = _pick_folder("Select Fallout 76 Playtest Directory")
            if path:
                gp["pts_root"] = path

    imgui.spacing()
    imgui.text("Extracted Dir")
    imgui.set_next_item_width(-100 * scale)
    changed, val = imgui.input_text(f"##ext_{game_id}", gp["extracted"])
    if changed:
        gp["extracted"] = val
    imgui.same_line()
    if imgui.button(f"Browse##ext_{game_id}"):
        path = _pick_folder("Select Extracted Directory")
        if path:
            gp["extracted"] = path

    if game_id == "starfield":
        imgui.spacing()
        imgui.text("ContentResources.zip")
        imgui.set_next_item_width(-100 * scale)
        changed, val = imgui.input_text(f"##cr_zip_{game_id}", gp.get("content_resources_zip", ""))
        if changed:
            gp["content_resources_zip"] = val
        imgui.same_line()
        if imgui.button(f"Browse##cr_zip_{game_id}"):
            path = _pick_file("Select ContentResources.zip", [("Zip files", "*.zip")])
            if path:
                gp["content_resources_zip"] = path

    imgui.spacing()
    imgui.text("Additional Paths")
    imgui.begin_child(f"##addl_{game_id}", imgui.ImVec2(0, 120 * scale), True)
    paths_list: list[str] = gp.setdefault("additional", [])
    to_remove = None
    for i, p in enumerate(paths_list):
        imgui.push_id(f"addl_{game_id}_{i}")
        if imgui.small_button("x"):
            to_remove = i
        imgui.same_line()
        display = p if len(p) <= 60 else "..." + p[-57:]
        imgui.text(display)
        if imgui.is_item_hovered() and len(p) > 60:
            imgui.set_tooltip(p)
        imgui.pop_id()
    if to_remove is not None:
        paths_list.pop(to_remove)
    imgui.end_child()

    if imgui.button(f"Add Path##addl_{game_id}"):
        path = _pick_folder("Select Additional Path")
        if path:
            norm = os.path.normpath(path)
            if norm not in paths_list:
                paths_list.append(norm)

    imgui.spacing()
    imgui.text("Scripts User Dir")
    if imgui.is_item_hovered():
        imgui.set_tooltip(
            "Papyrus user script source directory (e.g. Data/Scripts/Source/User).\n"
            "Used by the Papyrus LSP for script resolution."
        )
    imgui.set_next_item_width(-100 * scale)
    changed, val = imgui.input_text(f"##scripts_user_{game_id}", gp.get("scripts_user_dir", ""))
    if changed:
        gp["scripts_user_dir"] = val
    imgui.same_line()
    if imgui.button(f"Browse##scripts_user_{game_id}"):
        path = _pick_folder("Select Scripts User Source Directory")
        if path:
            gp["scripts_user_dir"] = os.path.normpath(path)

    imgui.spacing()
    imgui.text("Scripts Base Dir")
    if imgui.is_item_hovered():
        imgui.set_tooltip(
            "Papyrus base/vanilla script source directory (e.g. Data/Scripts/Source/Base).\n"
            "Used by the Papyrus LSP for script resolution."
        )
    imgui.set_next_item_width(-100 * scale)
    changed, val = imgui.input_text(f"##scripts_base_{game_id}", gp.get("scripts_base_dir", ""))
    if changed:
        gp["scripts_base_dir"] = val
    imgui.same_line()
    if imgui.button(f"Browse##scripts_base_{game_id}"):
        path = _pick_folder("Select Scripts Base Source Directory")
        if path:
            gp["scripts_base_dir"] = os.path.normpath(path)


def _draw_fo4_installs(settings, scale: float | None = None) -> None:
    """Editor for extra FO4 installs (deploy targets), persisted to the canonical store.

    Writes through ``set_fo4_extra_installs`` rather than the section's save() dict,
    because the section_data path the rest of this tab uses does not reach
    ``get_game_paths`` (the store the Mod Builder reads).
    """
    scale = scale or 1
    if not hasattr(settings, "get_fo4_extra_installs"):
        return
    if _state.fo4_installs is None:
        _state.fo4_installs = settings.get_fo4_extra_installs()
    installs = _state.fo4_installs

    imgui.spacing()
    imgui.separator()
    imgui.text("Additional Installs")
    if imgui.is_item_hovered():
        imgui.set_tooltip(
            "Extra Fallout 4 installs (e.g. different game versions).\n"
            "Pick one as the deploy target on the Mod Builder's Deploy tab.\n"
            "Game Root above stays the primary install used everywhere else."
        )
    imgui.spacing()

    imgui.begin_child("##fo4_installs", imgui.ImVec2(0, 120 * scale), True)
    to_remove = None
    for i, inst in enumerate(installs):
        imgui.push_id(f"fo4_install_{i}")
        if imgui.small_button("x"):
            to_remove = i
        imgui.same_line()
        imgui.set_next_item_width(220 * scale)
        changed, new_label = imgui.input_text("##label", inst.get("label", ""))
        if changed:
            inst["label"] = new_label
        if imgui.is_item_deactivated_after_edit():
            settings.set_fo4_extra_installs(installs)
        imgui.same_line()
        root = inst.get("root_dir", "")
        display = root if len(root) <= 48 else "..." + root[-45:]
        imgui.text_disabled(display)
        if imgui.is_item_hovered() and len(root) > 48:
            imgui.set_tooltip(root)
        imgui.pop_id()
    if to_remove is not None:
        installs.pop(to_remove)
        settings.set_fo4_extra_installs(installs)
    imgui.end_child()

    if imgui.button("Add Install##fo4_installs"):
        path = _pick_folder("Select Fallout 4 Install Directory")
        if path:
            norm = os.path.normpath(path)
            if not any(
                os.path.normpath(x.get("root_dir", "")) == norm for x in installs
            ):
                from creation_lib.core.fo4_version import detect_fo4_version

                version = detect_fo4_version(norm)
                label = f"Fallout 4 {version}" if version else os.path.basename(norm)
                installs.append({"label": label, "root_dir": norm})
                settings.set_fo4_extra_installs(installs)


def _draw_script_sources(scale: float | None = None) -> None:
    scale = scale or 1
    imgui.spacing()
    imgui.text_disabled(
        "Additional Papyrus .psc source directories (optional).\n"
        "The active game's Data/Scripts/Source/User is loaded automatically."
    )
    imgui.spacing()
    imgui.text("Script Source Paths")
    imgui.begin_child("##script_sources", imgui.ImVec2(0, 120 * scale), True)
    to_remove = None
    for i, p in enumerate(_state.script_sources):
        imgui.push_id(f"ss_{i}")
        if imgui.small_button("x"):
            to_remove = i
        imgui.same_line()
        display = p if len(p) <= 60 else "..." + p[-57:]
        imgui.text(display)
        if imgui.is_item_hovered() and len(p) > 60:
            imgui.set_tooltip(p)
        imgui.pop_id()
    if to_remove is not None:
        _state.script_sources.pop(to_remove)
    imgui.end_child()

    if imgui.button("Add Path##script_sources"):
        path = _pick_folder("Select Script Source Directory")
        if path:
            norm = os.path.normpath(path)
            if norm not in _state.script_sources:
                _state.script_sources.append(norm)


def _draw(ctx: SettingsContext) -> None:
    scale = ctx.scale
    imgui.spacing()
    imgui.separator()
    imgui.spacing()

    if imgui.begin_tab_bar("##paths_tabs"):
        for game_id, label in _GAME_TABS:
            selected, _ = imgui.begin_tab_item(label)
            if selected:
                _draw_game_paths(game_id, scale)
                if game_id == "fo4":
                    _draw_fo4_installs(ctx.settings, scale)
                imgui.end_tab_item()
        selected, _ = imgui.begin_tab_item("Script Sources")
        if selected:
            _draw_script_sources(scale)
            imgui.end_tab_item()
        imgui.end_tab_bar()


def _load(saved: dict) -> None:
    _state.game_paths = {}
    for game_id, _ in _GAME_TABS:
        gp = saved.get(game_id, {})
        _state.game_paths[game_id] = {
            "root": gp.get("root_dir", ""),
            "pts_root": gp.get("pts_root_dir", ""),
            "extracted": gp.get("extracted_dir", ""),
            "additional": list(gp.get("additional_paths", [])),
            "content_resources_zip": gp.get("content_resources_zip", ""),
            "scripts_user_dir": gp.get("scripts_user_dir", ""),
            "scripts_base_dir": gp.get("scripts_base_dir", ""),
        }
    _state.script_sources = list(saved.get("script_sources", []))
    _state.fo4_installs = None  # re-read from canonical settings on next FO4-tab draw


def _save() -> dict:
    result: dict = {}
    for game_id, _ in _GAME_TABS:
        gp = _state.game_paths.get(game_id, {})
        result[game_id] = {
            "root_dir": gp.get("root", ""),
            "pts_root_dir": gp.get("pts_root", ""),
            "extracted_dir": gp.get("extracted", ""),
            "additional_paths": list(gp.get("additional", [])),
            "content_resources_zip": gp.get("content_resources_zip", ""),
            "scripts_user_dir": gp.get("scripts_user_dir", ""),
            "scripts_base_dir": gp.get("scripts_base_dir", ""),
        }
    result["script_sources"] = list(_state.script_sources)
    return result


def _canonical_saved(settings, fallback: dict) -> dict:
    """Build section-shaped data from the app's canonical path store."""
    if settings is None or not hasattr(settings, "get_game_paths"):
        return fallback

    saved: dict = {}
    for game_id, _ in _GAME_TABS:
        canonical = settings.get_game_paths(game_id) or {}
        merged = dict(fallback.get(game_id, {}))
        for field, value in canonical.items():
            if value or field not in merged:
                merged[field] = value
        saved[game_id] = merged
    if hasattr(settings, "get_script_source_paths"):
        canonical_sources = settings.get_script_source_paths()
        saved["script_sources"] = canonical_sources or fallback.get(
            "script_sources", []
        )
    else:
        saved["script_sources"] = fallback.get("script_sources", [])
    return saved


def _save_canonical(settings, saved: dict) -> bool:
    """Write edited paths to canonical settings when that API is available."""
    if settings is None or not hasattr(settings, "get_game_paths"):
        return False

    persisted = False
    for game_id, _ in _GAME_TABS:
        game_paths = saved.get(game_id, {})
        if hasattr(settings, "set_game_paths"):
            settings.set_game_paths(game_id, game_paths)
            persisted = True
            continue

        # ToolkitSettings exposes its canonical mapping plus focused setters.
        # Updating the mapping avoids a disk write for every individual field.
        paths_store = getattr(settings, "_paths", None)
        if isinstance(paths_store, dict):
            paths_store.setdefault(game_id, {}).update(game_paths)
            persisted = True
            continue

        if hasattr(settings, "set_game_root_dir"):
            settings.set_game_root_dir(game_id, game_paths.get("root_dir", ""))
            persisted = True
        if hasattr(settings, "set_game_extracted_dir"):
            settings.set_game_extracted_dir(
                game_id, game_paths.get("extracted_dir", "")
            )
            persisted = True

    if hasattr(settings, "set_script_source_paths"):
        settings.set_script_source_paths(saved.get("script_sources", []))
        persisted = True
    return persisted


def make_section(settings=None) -> SettingsSection:
    def load(saved: dict) -> None:
        _load(_canonical_saved(settings, saved))

    def save() -> dict:
        saved = _save()
        return {} if _save_canonical(settings, saved) else saved

    return SettingsSection(id="paths", label="Paths", draw=_draw, load=load, save=save)
