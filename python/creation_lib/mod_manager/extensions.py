"""Versioned, explicitly registered trusted Python metadata adapters.

No discovery, dynamic imports, scripts from packages or MO2 binary loading.
Callbacks execute in the existing worker and must be trusted application code.
"""
from dataclasses import dataclass
from typing import Callable, Protocol

from .backend import call


API_VERSION = 1


class SaveReader(Protocol):
    def __call__(self, save, cancel=None): ...


class ArchiveResolver(Protocol):
    def __call__(self, path, loose, archives, settings, *, game="fo4"): ...


@dataclass(frozen=True)
class Adapter:
    id: str
    name: str
    kind: str
    game: str
    callback: Callable
    summary: str
    api_version: int = API_VERSION


class Registry:
    def __init__(self):
        self._adapters = {}

    @property
    def adapters(self):
        return tuple(self._adapters.values())

    def register(self, adapter):
        return call("extension_register", self, adapter)

    def adapter(self, kind, game):
        return call("extension_adapter", self, kind, game)

    def inspect_save(self, game, save, cancel=None):
        return call("extension_inspect_save", self, game, save, cancel)

    def resolve_archive(self, path, loose, archives, settings, *, game="fo4"):
        return call("extension_resolve_archive", self, path, loose, archives, settings, game)


def builtin_registry():
    from .combined_data import resolve
    from .save_inspection import inspect
    registry = Registry()
    registry.register(Adapter("fo4.save-plugins", "Fallout 4 save plugin tables", "save", "fo4", inspect,
                              "Read-only header versions 11/12/13/15; version 15 form 68+ regular/light tables. No save-body decoding."))
    registry.register(Adapter("fo4.archive-provenance", "Fallout 4 combined Data", "archive", "fo4", resolve,
                              "INI/plugin load-plan predictions with explicit unresolved attachment and loose override cases."))
    return registry
