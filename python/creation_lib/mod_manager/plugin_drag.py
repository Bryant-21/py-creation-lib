"""Freeze a selected plugin block for one native drag gesture."""
from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Drag:
    profile_id: str
    catalog: object
    order: tuple[str, ...]
    active: frozenset[str]
    locked: tuple
    selected: tuple[str, ...]


def begin(panes, origin):
    return call("plugin_drag_begin", panes, origin)


def drop(panes, drag, target):
    return call("plugin_drag_drop", panes, drag, target)
