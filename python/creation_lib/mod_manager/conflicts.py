from dataclasses import dataclass, field
from functools import cached_property

from .backend import call


@dataclass(frozen=True)
class Provider:
    mod_id: str
    name: str
    priority: int
    enabled: bool
    hidden: bool
    path: str
    size: int
    modified: int


@dataclass(frozen=True)
class FileEntry:
    path: str
    providers: tuple[Provider, ...]

    @cached_property
    def winner(self):
        return call("conflict_providers", self, False)

    @cached_property
    def active(self):
        return call("conflict_providers", self, True)


@dataclass
class FileIndex:
    profile_id: str
    profile_name: str
    files: tuple[FileEntry, ...]
    issues: tuple[str, ...]
    signature: tuple
    unavailable_mod_ids: tuple[str, ...] = ()
    _key: tuple | None = field(default=None, repr=False)
    _filtered: list = field(default_factory=list, repr=False)
    _visible: set = field(default_factory=set, repr=False)
    tree: object = field(default=None, repr=False)

    @cached_property
    def _by_path(self):
        return {entry.path: entry for entry in self.files}

    def selected(self, path):
        return call("conflict_selected", self, path)

    def filtered(self, query="", mode="all", mod_id=""):
        return call("conflict_filter", self, query, mode, mod_id)



@dataclass(frozen=True)
class DataSource:
    id: str
    name: str
    source: str
    kind: str
    priority: int
    enabled: bool


def rows_signature(rows):
    return call("conflict_rows_signature", rows)


def build_index(library, profile_id, cancel=None, progress=None, *, include_disabled=True, include_context=True, build_tree=True):
    return call("conflict_build_index", library, profile_id, cancel, progress, include_disabled, include_context, build_tree)


def reveal(provider):
    return call("conflict_reveal", provider)
