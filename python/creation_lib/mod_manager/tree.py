"""A flattened visible group tree allows native clipping without visiting hidden rows."""
from dataclasses import dataclass, field
from .backend import call


@dataclass
class Node:
    key: str
    name: str
    depth: int = 0
    row: object = None
    children: list = field(default_factory=list)
    priority: int = 0
    count: int = 0


class ModTree:
    def __init__(self, all_rows, matching_rows):
        self.roots, self.groups = call("mod_tree_build", all_rows, matching_rows)

    def visible(self, closed=(), *, reveal=False):
        return call("mod_tree_visible", self.roots, set(closed), reveal)
