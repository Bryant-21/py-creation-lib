"""creation_lib.ui.shell — workspace base class, view menu, host."""
from .base_workspace import BaseWorkspace, bind_panels, make_window
from .view_menu import ViewMenuHelper
from .settings_section import SettingsContext, SettingsSection
from .settings_window import SettingsWindow
from .workspace_host import WorkspaceHost

__all__ = [
    "BaseWorkspace",
    "bind_panels",
    "make_window",
    "ViewMenuHelper",
    "SettingsContext",
    "SettingsSection",
    "SettingsWindow",
    "WorkspaceHost",
]
