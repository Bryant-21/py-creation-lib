"""WorkspaceHost — generic dockable-workspace host shell.

Apps compose this with their own menus, settings bindings, and shell chrome.
"""
from __future__ import annotations

import logging
from collections.abc import Iterable

from imgui_bundle import hello_imgui

from .base_workspace import BaseWorkspace
from .view_menu import ViewMenuHelper

_log = logging.getLogger("creation_lib.ui.shell.workspace_host")


class WorkspaceHost:
    """Owns workspace registry, runner params skeleton, and active-workspace switching.

    Apps (e.g. ToolkitApp) compose this and fill in their own docking layout,
    menu callbacks, and addons before calling run().
    """

    def __init__(
        self,
        *,
        app_window_params: hello_imgui.AppWindowParams | None = None,
        ini_filename: str = "imgui.ini",
        fps_idle: float = 10.0,
        shared_view_labels: list[str] | None = None,
    ):
        self._workspaces: dict[str, BaseWorkspace] = {}
        self._active_id: str | None = None
        self._view_helper = ViewMenuHelper(shared_view_labels or [])
        self._params = hello_imgui.RunnerParams()
        if app_window_params is not None:
            self._params.app_window_params = app_window_params
        self._params.fps_idling.fps_idle = fps_idle
        self._params.docking_params = hello_imgui.DockingParams()
        self._ini_filename = ini_filename
        self._on_post_init: list = []

    def register(self, ws: BaseWorkspace) -> None:
        """Register a workspace with the host and inject the shared view helper."""
        if ws.id in self._workspaces:
            raise ValueError(f"workspace already registered: {ws.id}")
        self._workspaces[ws.id] = ws
        setter = getattr(ws, "set_view_helper", None)
        if setter is not None:
            setter(self._view_helper)

    def activate(self, workspace_id: str) -> None:
        """Switch to workspace_id, initializing it if needed."""
        if workspace_id not in self._workspaces:
            raise KeyError(workspace_id)
        if self._active_id == workspace_id:
            return
        if self._active_id:
            self._workspaces[self._active_id].on_deactivate()
        self._active_id = workspace_id
        ws = self._workspaces[workspace_id]
        if not getattr(ws, "_initialized", True):
            ws.initialize()
        ws.on_activate()

    def active(self) -> BaseWorkspace | None:
        """Return the currently active workspace, or None."""
        return self._workspaces.get(self._active_id) if self._active_id else None

    def runner_params(self) -> hello_imgui.RunnerParams:
        return self._params

    def workspaces(self) -> Iterable[BaseWorkspace]:
        return self._workspaces.values()

    def workspace(self, workspace_id: str) -> BaseWorkspace | None:
        return self._workspaces.get(workspace_id)

    def add_post_init(self, fn) -> None:
        self._on_post_init.append(fn)

    def run(self) -> None:
        """Run the host with a basic immapp loop (no addons).

        Apps that need addons (e.g. node_editor, implot) should call
        immapp.run directly with the params from runner_params().
        """
        from imgui_bundle import immapp

        params = self._params
        params.callbacks.post_init = self._post_init
        params.callbacks.before_exit = self._before_exit
        immapp.run(runner_params=params)

    def _post_init(self) -> None:
        for fn in self._on_post_init:
            fn()

    def _before_exit(self) -> None:
        for ws in self._workspaces.values():
            try:
                ws.cleanup()
            except Exception:
                _log.error("WorkspaceHost: cleanup error for %s", ws.id, exc_info=True)
