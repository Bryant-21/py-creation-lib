"""SettingsSection — pluggable settings panels."""
from __future__ import annotations
from collections.abc import Callable
from dataclasses import dataclass


@dataclass
class SettingsContext:
    """Per-frame state passed to section draw functions."""
    active_game: str
    settings: object  # the app's settings handle (ToolkitSettings, FallTalkConfig, etc.)
    mark_dirty: Callable[[], None]
    active_workspace: object | None = None

    @property
    def scale(self) -> float:
        from creation_lib.ui.widgets.modern import scaled

        return scaled(1)


@dataclass
class SettingsSection:
    """One left-rail entry in the SettingsWindow."""
    id: str
    label: str
    draw: Callable[[SettingsContext], None]
    load: Callable[[dict], None] | None = None
    save: Callable[[], dict] | None = None
