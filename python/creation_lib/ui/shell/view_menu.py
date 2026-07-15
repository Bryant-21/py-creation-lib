"""ViewMenuHelper — per-workspace View submenu for the toolkit."""
from __future__ import annotations

from imgui_bundle import hello_imgui, imgui


class ViewMenuHelper:
    """Renders a workspace-specific 'View' submenu inside the menu bar.

    Instantiated once by ToolkitApp and injected into all workspaces via
    set_view_helper().  Each workspace calls draw() from inside draw_menu().
    """

    def __init__(self, shared_labels: list[str]):
        """
        Args:
            shared_labels: Labels for panels always shown at the bottom of
                every View submenu (e.g. ["AI Chat", "Log"]).  Labels not
                found in docking_params are silently skipped.
        """
        self._shared_labels = shared_labels

    @staticmethod
    def _display_name(label: str) -> str:
        """Strip ##suffix — 'Files##papyrus' -> 'Files'."""
        idx = label.find("##")
        return label[:idx] if idx >= 0 else label

    @staticmethod
    def _find_window(label: str) -> hello_imgui.DockableWindow | None:
        """Look up a DockableWindow by its full label (including ##suffix).

        Returns None if not found — never raises.
        """
        dp = hello_imgui.get_runner_params().docking_params
        for w in dp.dockable_windows:
            if w.label == label:
                return w
        return None

    def draw(self, ws_panel_labels: list[str], menu_label: str = "View") -> None:
        """Render begin_menu(menu_label) with visibility checkboxes.

        Call this from inside a workspace's draw_menu(), at the top-level
        menu bar context (not nested inside another begin_menu call).

        Args:
            ws_panel_labels: Full labels (with ##suffix) for this workspace's
                panels, in the order they should appear in the menu.
            menu_label: The menu label (default "View").
        """
        if not imgui.begin_menu(menu_label):
            return

        for lbl in ws_panel_labels:
            win = self._find_window(lbl)
            if win is None:
                continue
            changed, new_val = imgui.menu_item(self._display_name(lbl), "", win.is_visible)
            if changed:
                win.is_visible = new_val

        if self._shared_labels:
            imgui.separator()
            for lbl in self._shared_labels:
                win = self._find_window(lbl)
                if win is None:
                    continue
                changed, new_val = imgui.menu_item(self._display_name(lbl), "", win.is_visible)
                if changed:
                    win.is_visible = new_val

        imgui.end_menu()
