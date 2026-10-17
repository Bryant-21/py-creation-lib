from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Drag:
    library_id: str
    profile_id: str
    mod_ids: tuple[str, ...]
    entries: tuple
    expected: str


def begin(workspace, origin):
    return call("manager_drag_begin", workspace, origin)


def drop(workspace, gesture, *, target="", group=None, after=False):
    return call("manager_drag_drop", workspace, gesture, target, group, after)
