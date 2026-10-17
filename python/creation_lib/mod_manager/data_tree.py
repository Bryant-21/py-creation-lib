"""Cached virtual loose-file hierarchy; inventories are built by workers."""
from dataclasses import dataclass

from .backend import call, invoke


@dataclass(frozen=True)
class Row:
    path: str
    depth: int
    entry: object = None


class Tree:
    def __init__(self, entries, cancel=None):
        self.entries = entries
        self._native = call("DataTree", entries, cancel)

    def visible(self, expanded, query=""):
        return invoke(self._native.visible, list(expanded), query)
