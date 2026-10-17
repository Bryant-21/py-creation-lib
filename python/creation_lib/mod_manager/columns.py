"""Persisted column preferences and deterministic table-free tree geometry."""
from .backend import call
from dataclasses import dataclass



ORDER = ("enabled", "priority", "name", "status", "version", "category", "nexus", "notes")
LABELS = dict(zip(ORDER, ("On", "Priority", "Mod name", "Status", "Version", "Category", "ID", "Notes")))
DEFAULT_WIDTHS = dict(zip(ORDER, (38, 76, 280, 78, 96, 110, 62, 130)))
MINIMUMS = dict(zip(ORDER, (28, 54, 120, 28, 48, 48, 42, 48)))
MAXIMUMS = dict(zip(ORDER, (100, 220, 1200, 360, 600, 800, 220, 1200)))
REQUIRED = frozenset(("enabled", "priority", "name"))


@dataclass(frozen=True)
class Column:
    key: str
    label: str
    offset: float
    width: float


def normalize(value=None):
    return call("columns_normalize", value)


def resized(value, key, width):
    return call("columns_resized", value, key, width)


def moved(value, key, before):
    return call("columns_moved", value, key, before)


def layout(available, visible, value=None):
    return call("columns_layout", available, visible, value)
