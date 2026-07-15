"""Settings window — floating singleton for toolkit-wide configuration.

Opened via ToolkitApp menu: ModBox21 > Settings > General... / Paths...
Close button saves. X button discards.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path

from imgui_bundle import imgui

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.ui.widgets.pick_folder import pick_file as _shared_pick_file, pick_folder as _shared_pick_folder
from .settings_section import SettingsContext, SettingsSection

_log = logging.getLogger("toolkit.settings_window")


class SettingsWindow:
    """Non-modal floating ImGui window for toolkit-wide settings.

    Owned by ToolkitApp. Call draw() once per frame.
    open("general") / open("paths") to show and switch sections.
    register_section(SettingsSection(...)) to add pluggable sections.
    """

    def __init__(self, settings, *, include_indexes: bool = True, env_path: Path | None = None):
        self._settings = settings
        self._include_indexes = include_indexes
        self._env_path = env_path
        self._is_open: bool = False
        self._pending_focus: bool = False
        self._active_section: str = "general"
        self._active_workspace = None  # set by ToolkitApp each frame
        self._sections: list[SettingsSection] = []
        self._SECTION_LABELS: dict[str, str] = {}

        # Local edit state (populated by load_settings)
        self._active_game: str = "fo4"
        # Per-game path edit state: {game_id: {"root": str, "extracted": str, "additional": list}}
        self._game_paths: dict[str, dict] = {}
        self._script_sources: list[str] = []
        self._gitea_url: str = ""
        self._gitea_username: str = ""
        self._gitea_orgs: dict[str, str] = {}
        self._gitea_token: str = ""

        self.saved_and_closed: bool = False
        self.rerun_setup: bool = False
        self._env_sync_status: str = ""
        self._index_fo4_data: bool = True
        self._index_scripts: bool = True
        self._index_wiki: bool = True
        self._index_nifs: bool = True
        self._index_behaviors: bool = True
        self._index_swf: bool = True
        self._index_voice_reference: bool = True
        self._index_builder: object = None  # DbBuilder when a rebuild is running
        self._index_game: str = "fo4"  # selected game for index rebuild
        self._extracting: bool = False
        self._extract_thread: object = None
        self._extract_status: str = ""
        self._extract_up_to_date: bool = False
        self._extract_last_date: str = ""

    def open(self, section: str = "general"):
        """Open the window (or switch section if already open)."""
        if not self._is_open:
            self.load_settings()
            self._is_open = True
        self._pending_focus = True
        if section == "indexes" and not self._include_indexes:
            section = "general"
        self._active_section = section

    def register_section(self, section: SettingsSection) -> None:
        """Register a settings section. Order of registration = display order."""
        if any(s.id == section.id for s in self._sections):
            raise ValueError(f"Section already registered: {section.id}")
        self._sections.append(section)
        self._SECTION_LABELS[section.id] = section.label
        if section.load is not None:
            saved = (
                self._settings.get_settings_section(section.id)
                if hasattr(self._settings, "get_settings_section")
                else {}
            )
            section.load(saved)

    def _reload_sections(self) -> None:
        """Refresh section edit state whenever the settings window opens."""
        for section in self._sections:
            if section.load is None:
                continue
            saved = (
                self._settings.get_settings_section(section.id)
                if hasattr(self._settings, "get_settings_section")
                else {}
            )
            section.load(saved)

    def _reload_section(self, section_id: str) -> None:
        for section in self._sections:
            if section.id != section_id or section.load is None:
                continue
            saved = (
                self._settings.get_settings_section(section.id)
                if hasattr(self._settings, "get_settings_section")
                else {}
            )
            section.load(saved)
            return

    def _commit_section(self, section_id: str) -> None:
        for section in self._sections:
            if section.id == section_id and section.save is not None:
                section.save()
                return

    def _mark_dirty(self) -> None:
        """No-op placeholder; section save/load is driven by the close button."""

    _GAME_TABS = [
        ("fo4", "Fallout 4"),
        ("skyrimse", "Skyrim SE"),
        ("fo76", "Fallout 76"),
        ("starfield", "Starfield"),
        ("fo3", "Fallout 3"),
        ("fnv", "Fallout: NV"),
    ]

    def load_settings(self):
        """Copy values from self._settings into local edit state."""
        self._active_game = self._settings.get_active_game()

        self._game_paths = {}
        for game_id, _ in self._GAME_TABS:
            gp = self._settings.get_game_paths(game_id)
            self._game_paths[game_id] = {
                "root": gp.get("root_dir", ""),
                "extracted": gp.get("extracted_dir", ""),
                "additional": list(gp.get("additional_paths", [])),
                "content_resources_zip": gp.get("content_resources_zip", ""),
                "scripts_user_dir": gp.get("scripts_user_dir", ""),
                "scripts_base_dir": gp.get("scripts_base_dir", ""),
            }

        self._script_sources = list(self._settings.get_script_source_paths())

        self._gitea_url = self._settings.gitea.get("url", "")
        self._gitea_username = self._settings.gitea.get("username", "")
        self._gitea_orgs = dict(self._settings.gitea.get("orgs", {}))
        self._gitea_token = self._settings.gitea.get("token", "")

        idx = self._settings.indexes
        self._index_fo4_data = idx.get("fo4_data", True)
        self._index_scripts = idx.get("scripts", True)
        self._index_wiki = idx.get("wiki", True)
        self._index_nifs = idx.get("nifs", True)
        self._index_behaviors = idx.get("behaviors", True)
        self._index_swf = idx.get("swf", True)
        self._index_voice_reference = idx.get("voice_reference", True)

        # Load manifest status for current extraction game
        self._load_extract_status()
        self._reload_sections()

    def _save_settings(self):
        """Write local state back to self._settings and persist to disk."""
        self._settings.set_active_game(self._active_game)

        # A registered Paths section owns canonical path persistence. Keep the
        # legacy local editor fallback for embedders that do not register it.
        if not any(section.id == "paths" for section in self._sections):
            for game_id, _ in self._GAME_TABS:
                gp = self._game_paths.get(game_id, {})
                self._settings._paths[game_id]["root_dir"] = gp.get("root", "")
                self._settings._paths[game_id]["extracted_dir"] = gp.get(
                    "extracted", ""
                )
                self._settings._paths[game_id]["additional_paths"] = list(
                    gp.get("additional", [])
                )
                self._settings._paths[game_id]["scripts_user_dir"] = gp.get(
                    "scripts_user_dir", ""
                )
                self._settings._paths[game_id]["scripts_base_dir"] = gp.get(
                    "scripts_base_dir", ""
                )
                if game_id == "starfield":
                    self._settings._paths[game_id]["content_resources_zip"] = gp.get(
                        "content_resources_zip", ""
                    )

            self._settings.set_script_source_paths(list(self._script_sources))

        self._settings.gitea["url"] = self._gitea_url
        self._settings.gitea["username"] = self._gitea_username
        self._settings.gitea["orgs"] = dict(self._gitea_orgs)
        self._settings.gitea["token"] = self._gitea_token

        self._settings.indexes = {
            "fo4_data": self._index_fo4_data,
            "scripts": self._index_scripts,
            "wiki": self._index_wiki,
            "nifs": self._index_nifs,
            "behaviors": self._index_behaviors,
            "swf": self._index_swf,
            "voice_reference": self._index_voice_reference,
        }

        for s in self._sections:
            if s.save is not None and hasattr(self._settings, "set_settings_section"):
                self._settings.set_settings_section(s.id, s.save())

        self._settings.save()
        self._export_to_env()

    def draw(self):
        """Draw the settings window. No-op when closed. Call every frame."""
        if not self._is_open:
            return

        if self._pending_focus:
            imgui.set_next_window_focus()
            self._pending_focus = False

        imgui.set_next_window_size(imgui.ImVec2(800, 600), imgui.Cond_.first_use_ever)
        flags = imgui.WindowFlags_.no_docking
        expanded, is_open = imgui.begin("Settings##toolkit_settings", True, flags)

        if not is_open:
            # X button clicked — save and close (same as Close button)
            self._save_settings()
            self._is_open = False
            self.saved_and_closed = True
            imgui.end()
            return

        if not expanded:
            imgui.end()
            return

        avail = imgui.get_content_region_avail()
        sidebar_w = 180.0
        btn_h = 28.0  # height reserved for Close button row
        content_h = avail.y - btn_h

        # Left sidebar
        imgui.begin_child("##settings_sidebar", imgui.ImVec2(sidebar_w, content_h))
        for sec in self._sections:
            is_active = self._active_section == sec.id
            if is_active:
                imgui.push_style_color(
                    imgui.Col_.button, imgui.ImVec4(0.26, 0.59, 0.98, 1.0)
                )
            if imgui.button(sec.label, imgui.ImVec2(-1, 0)):
                self._active_section = sec.id
            if is_active:
                imgui.pop_style_color()
        if self._active_workspace and hasattr(self._active_workspace, "draw_settings"):
            ws_key = f"ws:{self._active_workspace.id}"
            ws_label = self._SECTION_LABELS.get(ws_key, self._active_workspace.name)
            is_active = self._active_section == ws_key
            if is_active:
                imgui.push_style_color(
                    imgui.Col_.button, imgui.ImVec4(0.26, 0.59, 0.98, 1.0)
                )
            if imgui.button(ws_label, imgui.ImVec2(-1, 0)):
                self._active_section = ws_key
            if is_active:
                imgui.pop_style_color()
        imgui.end_child()

        imgui.same_line()

        # Right content area
        imgui.begin_child(
            "##settings_content",
            imgui.ImVec2(avail.x - sidebar_w - 8, content_h),
        )
        ctx = SettingsContext(
            active_game=self._active_game,
            settings=self._settings,
            mark_dirty=self._mark_dirty,
            active_workspace=self._active_workspace,
        )
        for s in self._sections:
            if s.id == self._active_section:
                s.draw(ctx)
                break
        else:
            if self._active_section.startswith("ws:"):
                self._draw_workspace_settings()
        imgui.end_child()

        # Close button — bottom right
        close_w = 80.0
        imgui.set_cursor_pos_x(
            imgui.get_cursor_pos_x() + imgui.get_content_region_avail().x - close_w
        )
        if imgui.button("Close", imgui.ImVec2(close_w, 0)):
            self._save_settings()
            self._is_open = False
            self.saved_and_closed = True

        imgui.end()

    def _fixed_and_workspace_sections(self) -> list[str]:
        """Return section IDs from the registry, plus workspace pseudo-section."""
        sections = [s.id for s in self._sections]
        if self._active_workspace and hasattr(self._active_workspace, "draw_settings"):
            ws_key = f"ws:{self._active_workspace.id}"
            self._SECTION_LABELS[ws_key] = self._active_workspace.name
            sections.append(ws_key)
        return sections

    def _draw_workspace_settings(self):
        """Delegate to the active workspace's draw_settings()."""
        if self._active_workspace and hasattr(self._active_workspace, "draw_settings"):
            self._active_workspace.draw_settings()
        else:
            imgui.text("No workspace settings available.")

    # ------------------------------------------------------------------ #
    #  Section: Indexes                                                    #
    # ------------------------------------------------------------------ #

    _INDEX_GAMES = [(p.id, p.display_name) for p in GAME_PROFILES.values()]
    _INDEX_GAME_LABELS = {game_id: label for game_id, label in _INDEX_GAMES}
    # Maps db-game-id → settings game key (now unified — identity mapping)

    def _draw_indexes(self):
        """Draw the Indexes settings section."""
        from creation_lib.ui.host import get_host

        db_dir = get_host().get_db_dir()

        # Game selector
        imgui.text("Game:")
        imgui.same_line()
        game_labels = [label for _, label in self._INDEX_GAMES]
        game_ids = [g for g, _ in self._INDEX_GAMES]
        current_idx = (
            game_ids.index(self._index_game) if self._index_game in game_ids else 0
        )
        imgui.set_next_item_width(140)
        changed, new_idx = imgui.combo("##index_game", current_idx, game_labels)
        if changed:
            self._index_game = game_ids[new_idx]
            self._load_extract_status()
        imgui.spacing()

        game = self._index_game
        _INDEXES = [
            (
                "fo4_data",
                "Records",
                f"{game}_records.db",
                "Record data (weapons, NPCs, keywords…). Needed for Search.",
            ),
            (
                "scripts",
                "Papyrus Scripts",
                f"{game}_scripts.db",
                "Papyrus source index. Needed for script search and API browsing.",
            ),
            (
                "wiki",
                "Wiki",
                f"{game}_wiki.db",
                "Local wiki index. Needed for Papyrus, CK, and GECK wiki search.",
            ),
            (
                "nifs",
                "NIF Mesh Index",
                f"{game}_nifs.db",
                "Mesh file index. Requires extracted game files.",
            ),
            (
                "behaviors",
                "Havok Index",
                f"{game}_havok.db",
                "Havok asset index (behaviors, skeletons, animations). Requires extracted game files.",
            ),
            (
                "swf",
                "SWF Shape Library",
                f"{game}_swf_shapes.db",
                "Pipboy icon shape library. Requires extracted game files.",
            ),
            (
                "voice_reference",
                "Voice Reference",
                "",
                "Dialogue voice-line index for the Voice Browser. Uses installed game Data archives.",
            ),
        ]

        # Poll active rebuild
        if self._index_builder is not None:
            phase = self._index_builder.phase
            progress = self._index_builder.progress
            status = self._index_builder.status
            imgui.text(f"Rebuilding... ({phase})")
            imgui.progress_bar(progress, imgui.ImVec2(-1, 0))
            imgui.text_wrapped(status)
            if self._index_builder.done:
                if self._index_builder.error:
                    imgui.text_colored(
                        imgui.ImVec4(1, 0.3, 0.3, 1),
                        f"Error: {self._index_builder.error}",
                    )
                else:
                    imgui.text_colored(
                        imgui.ImVec4(0.3, 0.9, 0.3, 1), "Rebuild complete."
                    )
                self._index_builder = None
            imgui.spacing()
            return

        # Index table
        local_vals = {
            "fo4_data": self._index_fo4_data,
            "scripts": self._index_scripts,
            "wiki": self._index_wiki,
            "nifs": self._index_nifs,
            "behaviors": self._index_behaviors,
            "swf": self._index_swf,
            "voice_reference": self._index_voice_reference,
        }
        for key, label, db_file, desc in _INDEXES:
            if key == "voice_reference":
                exists, size_str = self._voice_reference_index_status(game)
            elif key == "wiki":
                from creation_lib.creation_data._db_resolver import (
                    db_available,
                    get_db_path,
                )

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
                size_str = (
                    f"{db_path.stat().st_size // (1024 * 1024)} MB"
                    if exists
                    else "Not built"
                )
            status_color = (
                imgui.ImVec4(0.4, 0.8, 0.4, 1)
                if exists
                else imgui.ImVec4(0.6, 0.6, 0.6, 1)
            )

            changed, new_val = imgui.checkbox(f"##{key}_enabled", local_vals[key])
            if changed:
                setattr(self, f"_index_{key}", new_val)
            imgui.same_line()
            imgui.text(label)
            imgui.same_line(250)
            imgui.text_colored(status_color, size_str)
            imgui.same_line(350)
            if imgui.button(f"Rebuild##{key}"):
                self._start_rebuild(only=key)
            imgui.text_disabled(f"  {desc}")
            imgui.spacing()

        imgui.separator()
        imgui.spacing()
        if imgui.button("Rebuild All", imgui.ImVec2(120, 0)):
            self._start_rebuild(only=None)

        # ---- Source Data section ----
        gp = self._game_paths.get(game, self._settings.get_game_paths(game))
        extracted = gp.get("extracted", "") or gp.get("extracted_dir", "")
        game_root = gp.get("root", "") or gp.get("root_dir", "")
        has_extracted = bool(extracted and os.path.isdir(extracted))

        imgui.spacing()
        imgui.separator()
        imgui.spacing()
        imgui.text("Source Data")
        imgui.spacing()

        if not game_root:
            imgui.text_disabled(
                "Set the game install path in Paths tab to enable extraction."
            )
            return

        # -- YAML records status --
        from creation_lib.ui.host import GAME_ESM_YAML_DIR as _GAME_ESM_YAML_DIR

        yaml_subdir = _GAME_ESM_YAML_DIR.get(game, f"{game}_esm_yaml")
        yaml_dir = db_dir / yaml_subdir
        yaml_count = self._count_yaml_plugins(yaml_dir)

        if yaml_count > 0:
            imgui.text_colored(
                imgui.ImVec4(0.3, 0.9, 0.3, 1),
                f"\u2713 YAML records: {yaml_count} plugin(s) extracted",
            )
            second_label = f"Re-extract YAML##{game}"
            second_tooltip = (
                "Delete existing YAML and re-extract all ESM/ESL plugins."
            )
            second_width = 120
        else:
            imgui.text_colored(
                imgui.ImVec4(1.0, 0.8, 0.3, 1), "\u26a0 YAML records: not extracted"
            )
            imgui.text_disabled(
                "  Records index requires YAML extraction from game ESM files."
            )
            second_label = f"Extract YAML##{game}"
            second_tooltip = "Serialize game ESM/ESL plugins to YAML (needed for Records index)."
            second_width = 120
        imgui.same_line(350)
        if imgui.button(f"Smart YAML##{game}", imgui.ImVec2(140, 0)):
            self._start_yaml_extraction(game, game_root, smart=True)
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Extract only new or updated ESM plugins and add their records to the existing index."
            )
        imgui.same_line()
        if imgui.button(second_label, imgui.ImVec2(second_width, 0)):
            self._start_yaml_extraction(game, game_root, force=True)
        if imgui.is_item_hovered():
            imgui.set_tooltip(second_tooltip)
        imgui.spacing()

        # -- BA2/BSA archive extraction status --
        if not has_extracted:
            imgui.text_colored(
                imgui.ImVec4(1.0, 0.8, 0.3, 1), "\u26a0 Loose files: not extracted"
            )
            imgui.text_disabled(
                "  NIFs and behaviors require extracted BSA/BA2 archives."
            )
            imgui.spacing()
        else:
            # Show BA2 manifest status
            if self._extract_up_to_date:
                imgui.text_colored(
                    imgui.ImVec4(0.3, 0.9, 0.3, 1),
                    f"\u2713 Loose files: {self._extract_status}",
                )
            elif self._extract_status == "Updates available":
                imgui.text_colored(
                    imgui.ImVec4(1.0, 0.8, 0.2, 1),
                    f"\u26a0 Loose files: {self._extract_status}",
                )
            else:
                imgui.text_disabled(f"Loose files: {self._extract_status}")
            imgui.spacing()

        if self._extracting:
            imgui.text("Extracting archives...")
            imgui.progress_bar(-1.0 * imgui.get_time(), imgui.ImVec2(-1, 0))
            if self._extract_thread and not self._extract_thread.is_alive():
                self._extracting = False
                self._extract_thread = None
                self.load_settings()
        else:
            if imgui.button(f"Smart Extract##{game}", imgui.ImVec2(140, 0)):
                self._start_extraction(game, game_root, smart=True)
            if imgui.is_item_hovered():
                imgui.set_tooltip(
                    "Re-extract only if archives have changed since last run."
                )
            imgui.same_line()
            if imgui.button(f"Full Extract##{game}", imgui.ImVec2(120, 0)):
                self._start_extraction(game, game_root, smart=False)
            if imgui.is_item_hovered():
                imgui.set_tooltip("Always re-extract all archives.")

    def _load_extract_status(self) -> None:
        """Read .ba2_manifest.json and populate _extract_status/_extract_up_to_date/_extract_last_date."""
        from pathlib import Path
        from creation_lib.preprocessor.extraction import load_manifest, manifest_matches, find_archives
        from creation_lib.core.game_profiles import get_profile

        self._extract_status = "No extracted data — first run does a full extract."
        self._extract_up_to_date = False
        self._extract_last_date = ""

        game = self._index_game
        gp = self._settings.get_game_paths(game)
        extracted_dir = gp.get("extracted_dir", "")
        game_root = gp.get("root_dir", "")

        if not extracted_dir or not os.path.isdir(extracted_dir):
            return  # default status already set

        manifest = load_manifest(Path(extracted_dir))
        if manifest is None:
            return  # no manifest — default status

        # Compare against current archives if we have a game root
        if game_root:
            data_dir = Path(game_root) / "Data"
            if data_dir.is_dir():
                try:
                    profile = get_profile(game)
                    archives = find_archives(data_dir, profile.archive_format)
                    if manifest_matches(manifest, data_dir, archives):
                        self._extract_up_to_date = True
                        date_str = manifest.get("extracted_at", "")[:10]  # YYYY-MM-DD
                        self._extract_last_date = date_str
                        self._extract_status = (
                            f"Up to date ({date_str})" if date_str else "Up to date"
                        )
                    else:
                        self._extract_status = "Updates available"
                    return
                except Exception:
                    pass

        # Has manifest but can't check archives (no game_root or Data/ missing)
        date_str = manifest.get("extracted_at", "")[:10]
        self._extract_last_date = date_str
        self._extract_status = f"Extracted ({date_str})" if date_str else "Extracted"

    def _start_rebuild(self, only: str | None):
        """Start a DbBuilder for rebuilding. only=None means all enabled; only='nifs' means just nifs."""
        from creation_lib.ui.host import get_host

        factory = get_host().db_builder_factory
        if factory is None:
            return  # index building requires a host app service

        game = self._index_game
        gp = self._settings.get_game_paths(game)
        game_root = gp.get("root_dir", "")
        extracted = gp.get("extracted_dir", "")

        if only is None:
            # Rebuild all enabled
            self._index_builder = factory(
                game_root=game_root,
                extracted_dir=extracted,
                build_fo4_data=self._index_fo4_data,
                build_scripts=self._index_scripts,
                build_wiki=self._index_wiki,
                build_nifs=self._index_nifs,
                build_behaviors=self._index_behaviors,
                build_swf=self._index_swf,
                build_voice_reference_index=self._index_voice_reference,
                force_rebuild=True,
                game=game,
            )
        else:
            self._index_builder = factory(
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
        self._index_builder.start()

    def _voice_reference_index_status(self, game: str) -> tuple[bool, str]:
        from creation_lib.ui.host import get_host
        from creation_lib.audio.voice_reference import voice_reference_sqlite_cache_path

        gp = self._game_paths.get(game, self._settings.get_game_paths(game))
        root_value = str(gp.get("root", "") or gp.get("root_dir", "") or "").strip()
        if not root_value:
            return False, "No game path"
        root_dir = Path(root_value).expanduser()
        data_dir = root_dir / "Data"
        if not data_dir.is_dir() and root_dir.name.lower() == "data":
            data_dir = root_dir
        if not data_dir.is_dir():
            return False, "No Data folder"
        strings_dir = data_dir / "Strings"
        extracted_value = str(gp.get("extracted", "") or gp.get("extracted_dir", "") or "").strip()
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
            cache_dir=get_host().get_db_dir() / "cache",
        )
        if cache_path is None or not cache_path.is_file():
            return False, "Not built"
        size_mb = cache_path.stat().st_size // (1024 * 1024)
        return True, f"{size_mb} MB"

    def _start_extraction(self, game: str, game_root: str, smart: bool = False):
        """Run archive extraction in a background thread."""
        import threading
        from concurrent.futures import ThreadPoolExecutor, as_completed
        from creation_lib.ui.host import get_host

        output_dir = get_host().resolve_extracted_output_dir(game)
        output_dir.mkdir(parents=True, exist_ok=True)

        def _run():
            try:
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
                install_dir = Path(game_root)
                data_dir = install_dir / "Data"
                if not data_dir.is_dir():
                    raise RuntimeError(f"Data directory not found: {data_dir}")

                archives = find_archives(data_dir, profile.archive_format)
                if not archives:
                    raise RuntimeError(f"No {profile.archive_format.upper()} archives found in {data_dir}")

                papyrus_source_dir = resolve_papyrus_source_dir(install_dir, game)
                if smart and manifest_matches(load_manifest(output_dir), data_dir, archives, papyrus_source_dir):
                    self._settings.set_game_extracted_dir(game, str(output_dir))
                    _log.info("Smart extraction skipped for %s; archives unchanged", game)
                    return

                workers = min(max(1, min(4, os.cpu_count() or 1)), len(archives))
                total_files = 0
                errors: list[str] = []
                self._extract_status = f"Extracting {len(archives)} archive(s) with {workers} worker(s)..."
                _log.info("%s", self._extract_status)

                completed = 0
                def _progress_callback(archive_name: str):
                    def _progress(event: dict) -> bool:
                        completed_files = int(event.get("completed", 0) or 0)
                        total_archive_files = int(event.get("total", 0) or 0)
                        self._extract_status = (
                            f"Extracting {archive_name}: "
                            f"{completed_files:,}/{total_archive_files:,} file(s)"
                        )
                        return True

                    return _progress

                for archive_group in group_archives_by_update_phase(archives):
                    for batch in plan_archive_extraction_batches(archive_group, workers):
                        with ThreadPoolExecutor(max_workers=len(batch)) as pool:
                            futures = {
                                pool.submit(
                                    extract_one,
                                    task.archive,
                                    output_dir,
                                    profile.archive_format,
                                    task.file_workers,
                                    _progress_callback(task.archive.name),
                                ): task.archive
                                for task in batch
                            }
                            for future in as_completed(futures):
                                archive = futures[future]
                                _archive, count, error = future.result()
                                completed += 1
                                if error:
                                    errors.append(f"{archive.name}: {error}")
                                    _log.error("[%d/%d] %s", completed, len(archives), errors[-1])
                                else:
                                    total_files += int(count)
                                    _log.info(
                                        "[%d/%d] %s: %d files",
                                        completed,
                                        len(archives),
                                        archive.name,
                                        int(count),
                                    )

                if errors:
                    raise RuntimeError(f"{len(errors)} archive(s) failed")

                if papyrus_source_dir is not None:
                    papyrus_files = sync_papyrus_sources(papyrus_source_dir, output_dir)
                    _log.info("Papyrus sources mirrored: %d file(s)", papyrus_files)

                save_manifest(output_dir, build_manifest(game, data_dir, archives, papyrus_source_dir))
                self._settings.set_game_extracted_dir(game, str(output_dir))
                _log.info("Extraction complete for %s -> %s", game, output_dir)
            except Exception as e:
                _log.error("Extraction failed for %s: %s", game, e, exc_info=True)
            finally:
                self._extracting = False
                self._extract_thread = None
                self.load_settings()  # Refresh manifest status

        self._extracting = True
        self._extract_thread = threading.Thread(target=_run, daemon=True)
        self._extract_thread.start()

    @staticmethod
    def _count_yaml_plugins(yaml_dir) -> int:
        """Count extracted plugin subdirectories in a YAML dir (e.g. data/fo4_esm_yaml/)."""
        from pathlib import Path

        d = Path(yaml_dir)
        if not d.is_dir():
            return 0
        return sum(1 for p in d.iterdir() if p.is_dir())

    def _start_yaml_extraction(
        self, game: str, game_root: str, force: bool = False, smart: bool = False
    ):
        """Run Spriggit YAML extraction for a game's ESM files via DbBuilder (records only)."""
        from creation_lib.ui.host import get_host

        factory = get_host().db_builder_factory
        if factory is None:
            return  # index building requires a host app service

        gp = self._settings.get_game_paths(game)
        extracted = gp.get("extracted_dir", "")
        self._index_builder = factory(
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
        self._index_builder.start()

    # ------------------------------------------------------------------ #
    #  Section: General                                                    #
    # ------------------------------------------------------------------ #

    def _draw_general(self):
        # Default Game selector
        imgui.text("Default Game")
        imgui.separator()
        imgui.spacing()

        game_ids = list(GAME_PROFILES.keys())
        game_labels = [GAME_PROFILES[g].display_name for g in game_ids]
        current_idx = (
            game_ids.index(self._active_game) if self._active_game in game_ids else 0
        )
        imgui.set_next_item_width(200)
        changed, new_idx = imgui.combo("##default_game", current_idx, game_labels)
        if changed:
            self._active_game = game_ids[new_idx]

        imgui.spacing()
        imgui.separator()
        imgui.spacing()

        imgui.text("AddonNode Index Range")
        imgui.separator()
        imgui.spacing()
        imgui.set_next_item_width(120)
        changed, val = imgui.input_int(
            "Start Index##addon_start",
            self._settings.addon_node_index_start,
        )
        if changed:
            self._settings.addon_node_index_start = max(1, val)
        imgui.set_item_tooltip(
            "Starting index for new AddonNode allocations.\n"
            "Reserve a unique range to avoid collisions with other modders.\n"
            "Your current range starts at this value."
        )

        imgui.spacing()
        imgui.set_next_item_width(120)
        changed, val = imgui.input_int(
            "Conversion Start Index##addon_conv_start",
            self._settings.conversion_addon_node_index_start,
        )
        if changed:
            self._settings.conversion_addon_node_index_start = max(1, val)
        imgui.set_item_tooltip(
            "Starting index for AddonNode allocations created by the conversion workspace.\n"
            "This range is separate from regular mods."
        )

        imgui.spacing()
        imgui.separator()
        imgui.spacing()

        # Gitea Integration
        imgui.text("Gitea Server")
        imgui.separator()
        imgui.spacing()
        imgui.text_disabled(
            "Auto-create a git repo on your Gitea server when setting up a new mod."
        )
        imgui.spacing()

        _gitea_label_w = imgui.calc_text_size("Username").x + 12
        _gitea_input_w = imgui.get_content_region_avail().x - _gitea_label_w

        imgui.text("URL")
        imgui.same_line(_gitea_label_w)
        imgui.set_next_item_width(_gitea_input_w)
        _, self._gitea_url = imgui.input_text("##gitea_url", self._gitea_url)
        if imgui.is_item_hovered():
            imgui.set_tooltip("Gitea server URL, e.g. https://192.168.1.252:3100")

        imgui.text("Username")
        imgui.same_line(_gitea_label_w)
        imgui.set_next_item_width(_gitea_input_w)
        _, self._gitea_username = imgui.input_text(
            "##gitea_username", self._gitea_username
        )
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Your Gitea username — used for authentication and fallback repo owner"
            )

        imgui.spacing()
        imgui.text_disabled(
            "Orgs (optional) — repos are created under the org for that game, or your username if blank."
        )
        imgui.spacing()
        _org_label_w = imgui.calc_text_size("Starfield").x + 12
        _org_input_w = imgui.get_content_region_avail().x - _org_label_w
        for game_id, profile in GAME_PROFILES.items():
            imgui.text(profile.display_name)
            imgui.same_line(_org_label_w)
            imgui.set_next_item_width(_org_input_w)
            current = self._gitea_orgs.get(game_id, "")
            changed, new_val = imgui.input_text(f"##gitea_org_{game_id}", current)
            if changed:
                self._gitea_orgs[game_id] = new_val
        imgui.spacing()

        imgui.text("Token")
        imgui.same_line(_gitea_label_w)
        imgui.set_next_item_width(_gitea_input_w)
        _, self._gitea_token = imgui.input_text(
            "##gitea_token",
            self._gitea_token,
            flags=imgui.InputTextFlags_.password.value,
        )
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Optional API token — fallback if push-to-create is not enabled"
            )

        imgui.spacing()
        imgui.separator()
        imgui.spacing()

        # Rerun setup wizard
        imgui.text("Setup")
        imgui.separator()
        imgui.spacing()
        if imgui.button("Rerun Setup Wizard", imgui.ImVec2(200, 0)):
            self.rerun_setup = True
            self._is_open = False
        if imgui.is_item_hovered():
            imgui.set_tooltip("Close settings and rerun the first-time setup wizard")
        imgui.spacing()
        imgui.separator()
        imgui.spacing()

        # .env sync
        imgui.text(".env Sync")
        imgui.separator()
        imgui.spacing()
        imgui.text_disabled(
            "Sync settings with the .env file used by CLI scripts and MCP servers."
        )
        imgui.spacing()
        if imgui.button("Import from .env", imgui.ImVec2(180, 0)):
            self._import_from_env()
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Load game paths, Gitea settings, and MOD_PREFIX from .env into the UI"
            )
        imgui.same_line()
        if imgui.button("Export to .env", imgui.ImVec2(160, 0)):
            self._export_to_env()
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Write current UI settings back to .env (also happens automatically on Save)"
            )
        if self._env_sync_status:
            imgui.same_line()
            imgui.text_disabled(self._env_sync_status)

    # ------------------------------------------------------------------ #
    #  ENV sync helpers                                                    #
    # ------------------------------------------------------------------ #

    # Maps settings field names → .env key names, per game
    _ENV_KEY_MAP: dict[str, dict[str, str]] = {
        "fo4": {"root": "FO4_DIR", "extracted": "FO4_EXTRACTED_DIR"},
        "skyrimse": {"root": "SKYRIMSE_DIR", "extracted": "SKYRIMSE_EXTRACTED_DIR"},
        "starfield": {"root": "STARFIELD_DIR", "extracted": "STARFIELD_EXTRACTED_DIR"},
        "fo76": {"root": "FO76_DIR", "extracted": "FO76_EXTRACTED_DIR"},
        "fo3": {"root": "FO3_DIR", "extracted": "FO3_EXTRACTED_DIR"},
        "fnv": {"root": "FONV_DIR", "extracted": "FONV_EXTRACTED_DIR"},
    }

    def _get_env_path(self):
        return self._env_path

    def _parse_env_file(self) -> dict[str, str]:
        """Read .env and return a dict of unquoted key→value pairs."""
        env_path = self._env_path
        if env_path is None or not env_path.exists():
            return {}
        result = {}
        for line in env_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, _, v = line.partition("=")
                result[k.strip()] = v.strip().strip('"').strip("'")
        return result

    def _update_env_file(self, updates: dict[str, str]) -> bool:
        """Update specific key=value pairs in .env in place. Returns True on success."""
        env_path = self._env_path
        if env_path is None or not env_path.exists():
            return False
        lines = env_path.read_text(encoding="utf-8").splitlines()
        written = set()
        result = []
        for line in lines:
            stripped = line.strip()
            if stripped and not stripped.startswith("#") and "=" in stripped:
                key = stripped.split("=", 1)[0].strip()
                if key in updates:
                    result.append(f'{key}="{updates[key]}"')
                    written.add(key)
                    continue
            result.append(line)
        # Append any new keys not already present
        for key, val in updates.items():
            if key not in written:
                result.append(f'{key}="{val}"')
        env_path.write_text("\n".join(result) + "\n", encoding="utf-8")
        return True

    def _import_from_env(self):
        """Read .env values into local edit state (paths + active game + mod prefix + gitea)."""
        env = self._parse_env_file()
        if not env:
            self._env_sync_status = "No .env file found"
            return
        count = 0
        paths_section_registered = any(
            section.id == "paths" for section in self._sections
        )
        for game_id, field_map in self._ENV_KEY_MAP.items():
            imported: dict[str, str] = {}
            for field, env_key in field_map.items():
                if env.get(env_key):
                    imported[field] = env[env_key]
                    count += 1
            if not imported:
                continue
            if paths_section_registered and hasattr(
                self._settings, "get_game_paths"
            ):
                paths_store = getattr(self._settings, "_paths", None)
                if isinstance(paths_store, dict):
                    canonical = paths_store.setdefault(game_id, {})
                    if "root" in imported:
                        canonical["root_dir"] = imported["root"]
                    if "extracted" in imported:
                        canonical["extracted_dir"] = imported["extracted"]
                else:
                    if "root" in imported and hasattr(
                        self._settings, "set_game_root_dir"
                    ):
                        self._settings.set_game_root_dir(game_id, imported["root"])
                    if "extracted" in imported and hasattr(
                        self._settings, "set_game_extracted_dir"
                    ):
                        self._settings.set_game_extracted_dir(
                            game_id, imported["extracted"]
                        )
            else:
                gp = self._game_paths.setdefault(
                    game_id,
                    {
                        "root": "",
                        "extracted": "",
                        "additional": [],
                        "content_resources_zip": "",
                        "scripts_user_dir": "",
                        "scripts_base_dir": "",
                    },
                )
                gp.update(imported)
        if env.get("DEFAULT_GAME"):
            self._active_game = env["DEFAULT_GAME"]
            count += 1
        if env.get("MOD_PREFIX"):
            self._settings.mod_prefix = env["MOD_PREFIX"]
            count += 1
        if env.get("ADDON_NODE_INDEX_START"):
            try:
                self._settings.addon_node_index_start = int(
                    env["ADDON_NODE_INDEX_START"]
                )
                count += 1
            except ValueError:
                pass
        if env.get("CONVERSION_ADDON_NODE_INDEX_START"):
            try:
                self._settings.conversion_addon_node_index_start = int(
                    env["CONVERSION_ADDON_NODE_INDEX_START"]
                )
                count += 1
            except ValueError:
                pass
        # Gitea
        for key, attr in (
            ("GITEA_URL", "_gitea_url"),
            ("GITEA_USER", "_gitea_username"),
            ("GITEA_TOKEN", "_gitea_token"),
        ):
            if env.get(key) is not None:
                setattr(self, attr, env[key])
                count += 1
        for game_id in GAME_PROFILES:
            org_key = f"GITEA_ORG_{game_id.upper()}"
            if env.get(org_key) is not None:
                self._gitea_orgs[game_id] = env[org_key]
                count += 1
        if paths_section_registered:
            self._reload_section("paths")
        self._env_sync_status = f"Imported {count} value(s) from .env"
        _log.info("ENV→UI: %s", self._env_sync_status)

    def _export_to_env(self):
        """Write local edit state back to .env in place (paths + gitea)."""
        updates: dict[str, str] = {}
        paths_section_registered = any(
            section.id == "paths" for section in self._sections
        )
        for game_id, field_map in self._ENV_KEY_MAP.items():
            if paths_section_registered and hasattr(
                self._settings, "get_game_paths"
            ):
                canonical = self._settings.get_game_paths(game_id)
                gp = {
                    "root": canonical.get("root_dir", ""),
                    "extracted": canonical.get("extracted_dir", ""),
                }
            else:
                gp = self._game_paths.get(game_id, {})
            for field, env_key in field_map.items():
                updates[env_key] = gp.get(field, "")
        updates["DEFAULT_GAME"] = self._active_game
        if self._settings.mod_prefix:
            updates["MOD_PREFIX"] = self._settings.mod_prefix
        updates["ADDON_NODE_INDEX_START"] = str(self._settings.addon_node_index_start)
        updates["CONVERSION_ADDON_NODE_INDEX_START"] = str(
            self._settings.conversion_addon_node_index_start
        )
        # Gitea
        updates["GITEA_URL"] = self._gitea_url
        updates["GITEA_USER"] = self._gitea_username
        updates["GITEA_TOKEN"] = self._gitea_token
        for game_id in GAME_PROFILES:
            updates[f"GITEA_ORG_{game_id.upper()}"] = self._gitea_orgs.get(game_id, "")
        if self._update_env_file(updates):
            self._env_sync_status = f"Exported {len(updates)} value(s) to .env"
            _log.info("UI→ENV: %s", self._env_sync_status)
        else:
            self._env_sync_status = "Export failed: .env not found"

    # ------------------------------------------------------------------ #
    #  Section: Paths                                                      #
    # ------------------------------------------------------------------ #

    def _draw_paths(self):
        imgui.spacing()
        imgui.separator()
        imgui.spacing()

        if imgui.begin_tab_bar("##paths_tabs"):
            for game_id, label in self._GAME_TABS:
                selected, _ = imgui.begin_tab_item(label)
                if selected:
                    self._draw_game_paths(game_id)
                    imgui.end_tab_item()
            selected, _ = imgui.begin_tab_item("Script Sources")
            if selected:
                self._draw_script_sources()
                imgui.end_tab_item()
            imgui.end_tab_bar()

    def _draw_game_paths(self, game_id: str):
        """Draw paths UI for one game tab."""
        gp = self._game_paths.setdefault(
            game_id,
            {
                "root": "",
                "extracted": "",
                "additional": [],
                "content_resources_zip": "",
                "scripts_user_dir": "",
                "scripts_base_dir": "",
            },
        )

        # Game Root
        imgui.spacing()
        imgui.text("Game Root")
        imgui.set_next_item_width(-85)
        changed, val = imgui.input_text(f"##root_{game_id}", gp["root"])
        if changed:
            gp["root"] = val
        imgui.same_line()
        if imgui.button(f"Browse##root_{game_id}"):
            path = self._pick_folder("Select Game Root Directory")
            if path:
                gp["root"] = path

        # Extracted Dir
        imgui.spacing()
        imgui.text("Extracted Dir")
        imgui.set_next_item_width(-85)
        changed, val = imgui.input_text(f"##ext_{game_id}", gp["extracted"])
        if changed:
            gp["extracted"] = val
        imgui.same_line()
        if imgui.button(f"Browse##ext_{game_id}"):
            path = self._pick_folder("Select Extracted Directory")
            if path:
                gp["extracted"] = path

        # Starfield: ContentResources.zip file picker
        if game_id == "starfield":
            imgui.spacing()
            imgui.text("ContentResources.zip")
            imgui.set_next_item_width(-85)
            changed, val = imgui.input_text(
                f"##cr_zip_{game_id}", gp.get("content_resources_zip", "")
            )
            if changed:
                gp["content_resources_zip"] = val
            imgui.same_line()
            if imgui.button(f"Browse##cr_zip_{game_id}"):
                path = self._pick_file(
                    "Select ContentResources.zip", [("Zip files", "*.zip")]
                )
                if path:
                    gp["content_resources_zip"] = path

        # Additional Paths
        imgui.spacing()
        imgui.text("Additional Paths")
        imgui.begin_child(f"##addl_{game_id}", imgui.ImVec2(0, 120), True)
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
            path = self._pick_folder("Select Additional Path")
            if path:
                norm = os.path.normpath(path)
                if norm not in paths_list:
                    paths_list.append(norm)

        # Scripts User Dir
        imgui.spacing()
        imgui.text("Scripts User Dir")
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Papyrus user script source directory (e.g. Data/Scripts/Source/User).\n"
                "Used by the Papyrus LSP for script resolution."
            )
        imgui.set_next_item_width(-85)
        changed, val = imgui.input_text(
            f"##scripts_user_{game_id}", gp.get("scripts_user_dir", "")
        )
        if changed:
            gp["scripts_user_dir"] = val
        imgui.same_line()
        if imgui.button(f"Browse##scripts_user_{game_id}"):
            path = self._pick_folder("Select Scripts User Source Directory")
            if path:
                gp["scripts_user_dir"] = os.path.normpath(path)

        # Scripts Base Dir
        imgui.spacing()
        imgui.text("Scripts Base Dir")
        if imgui.is_item_hovered():
            imgui.set_tooltip(
                "Papyrus base/vanilla script source directory (e.g. Data/Scripts/Source/Base).\n"
                "Used by the Papyrus LSP for script resolution."
            )
        imgui.set_next_item_width(-85)
        changed, val = imgui.input_text(
            f"##scripts_base_{game_id}", gp.get("scripts_base_dir", "")
        )
        if changed:
            gp["scripts_base_dir"] = val
        imgui.same_line()
        if imgui.button(f"Browse##scripts_base_{game_id}"):
            path = self._pick_folder("Select Scripts Base Source Directory")
            if path:
                gp["scripts_base_dir"] = os.path.normpath(path)

    def _draw_script_sources(self):
        """Draw the Script Sources path list (add/remove)."""
        imgui.spacing()
        imgui.text_disabled(
            "Additional Papyrus .psc source directories (optional).\n"
            "The active game's Data/Scripts/Source/User is loaded automatically."
        )
        imgui.spacing()
        imgui.text("Script Source Paths")
        imgui.begin_child("##script_sources", imgui.ImVec2(0, 120), True)
        to_remove = None
        for i, p in enumerate(self._script_sources):
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
            self._script_sources.pop(to_remove)
        imgui.end_child()

        if imgui.button("Add Path##script_sources"):
            path = self._pick_folder("Select Script Source Directory")
            if path:
                norm = os.path.normpath(path)
                if norm not in self._script_sources:
                    self._script_sources.append(norm)

    @staticmethod
    def _pick_folder(title: str = "Select Folder") -> str | None:
        """Open an OS folder picker. Returns path string or None."""
        try:
            return _shared_pick_folder(title)
        except Exception as e:
            _log.warning("Folder picker failed: %s", e)
            return None

    @staticmethod
    def _pick_file(
        title: str = "Select File",
        filetypes: list[tuple[str, str]] | None = None,
    ) -> str | None:
        """Open an OS file picker. Returns path string or None."""
        try:
            return _shared_pick_file(title, filetypes)
        except Exception as e:
            _log.warning("File picker failed: %s", e)
            return None
