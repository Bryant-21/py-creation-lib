from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Edit:
    library: object
    profile_id: str
    catalog: object
    details: str
    name: str
    priority: int
    order: tuple[str, ...]
    active: frozenset[str]
    locked: tuple



def begin(panes, name):
    return call("plugin_priority_begin", panes, name)


def _checkpoint():
    pass


def apply(panes, plan, value, cancel=None):
    return call("plugin_priority_apply", panes, plan, value, cancel, _checkpoint)
