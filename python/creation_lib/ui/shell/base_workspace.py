"""BaseWorkspace — shared boilerplate for toolkit workspaces.

Provides common init, panel binding, lifecycle hooks, and settings
plumbing that every workspace duplicates. Subclasses override only
the parts that differ.
"""

from __future__ import annotations

import textwrap

from imgui_bundle import hello_imgui

from creation_lib.ui.widgets.user_guide import UserGuide


def make_window(label: str, dock: str, **kwargs) -> hello_imgui.DockableWindow:
    """Create a DockableWindow with call_begin_end=False and a no-op gui_function.

    Extra kwargs (e.g. is_visible=False) are set as attributes on the window.
    """
    w = hello_imgui.DockableWindow(label_=label, dock_space_name_=dock)
    w.call_begin_end = False
    w.gui_function = lambda: None
    for k, v in kwargs.items():
        setattr(w, k, v)
    return w


def bind_panels(workspace, panel_map: dict) -> None:
    """Bind draw functions to DockableWindows, guarded by workspace.active.

    Works with any object that has an ``active`` attribute — does not require
    BaseWorkspace inheritance.
    """
    dp = hello_imgui.get_runner_params().docking_params
    for w in dp.dockable_windows:
        if w.label in panel_map:
            fn = panel_map[w.label]
            w.gui_function = lambda f=fn: f() if workspace.active else None


class BaseWorkspace:
    """Optional base class implementing the Workspace protocol boilerplate.

    Subclasses must define class-level ``name``, ``icon``, and ``id``.

    Common patterns handled here:
    - ``__init__`` with ``active``, ``_view_helper``, ``_initialized``,
      ``_toolkit_settings``, ``_pending_settings``, ``_app``
    - ``_bind_panels(panel_map)`` — wires draw fns into DockableWindows
    - ``set_view_helper``
    - ``on_activate`` / ``on_deactivate`` with fps_idle and app.active
    - No-op defaults for ``draw``, ``draw_menu``, ``cleanup``,
      ``get_required_addons``, settings methods
    """

    name: str = ""
    icon: str = ""
    id: str = ""
    user_guide_body: str = ""

    def __init__(self, toolkit_settings=None):
        self.active = False
        self._view_helper = None
        self._initialized = False
        self._show_user_guide = False
        self._toolkit_settings = toolkit_settings
        self._pending_settings: dict | None = None
        self._app = None

    # -- Panel helpers --

    def _bind_panels(self, panel_map: dict) -> None:
        """Bind draw functions to their DockableWindows, guarded by self.active."""
        bind_panels(self, panel_map)

    # -- Workspace protocol defaults --

    def set_view_helper(self, helper) -> None:
        self._view_helper = helper

    def on_activate(self) -> None:
        hello_imgui.get_runner_params().fps_idling.fps_idle = 60.0
        self.active = True
        if self._app:
            self._app.active = True

    def on_deactivate(self) -> None:
        self.active = False
        if self._app:
            self._app.active = False

    def draw(self) -> None:
        pass

    def draw_menu(self) -> None:
        pass

    def cleanup(self) -> None:
        pass

    def get_required_addons(self) -> dict:
        return {}

    def get_user_guide(self) -> UserGuide | None:
        body = textwrap.dedent(str(getattr(self, "user_guide_body", "") or "")).strip()
        if not body:
            return None
        return UserGuide(
            title=f"{self.name} User Guide",
            body=body,
            window_id=f"user_guide_{self.id}",
        )

    def toggle_user_guide(self) -> None:
        if self._toggle_docked_user_guide_window():
            self._show_user_guide = False
            return
        self._show_user_guide = not self._show_user_guide

    def _user_guide_window_label(self) -> str:
        return f"Help##{self.id}"

    def _toggle_docked_user_guide_window(self) -> bool:
        runner_params = hello_imgui.get_runner_params()
        docking_params = getattr(runner_params, "docking_params", None)
        if docking_params is None:
            return False
        for window in docking_params.dockable_windows:
            if window.label == self._user_guide_window_label():
                window.is_visible = not window.is_visible
                return True
        return False

    def _has_docked_user_guide_window(self) -> bool:
        runner_params = hello_imgui.get_runner_params()
        docking_params = getattr(runner_params, "docking_params", None)
        if docking_params is None:
            return False
        return any(window.label == self._user_guide_window_label() for window in docking_params.dockable_windows)

    def draw_user_guide_window(self) -> None:
        guide = self.get_user_guide()
        if guide is None:
            self._show_user_guide = False
            return
        if self._has_docked_user_guide_window():
            self._show_user_guide = False
            return
        from creation_lib.ui.widgets.user_guide import draw_generic_user_guide_window

        self._show_user_guide = draw_generic_user_guide_window(
            self._show_user_guide,
            guide,
        )

    def get_settings_defaults(self) -> dict:
        return {}

    def apply_settings(self, settings: dict) -> None:
        pass

    def collect_settings(self) -> dict:
        return {}

    def draw_settings(self) -> None:
        pass
