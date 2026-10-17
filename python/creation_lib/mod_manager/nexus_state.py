"""Bounded file-specific Nexus updates and session account state, independent UI."""
from dataclasses import asdict, dataclass
import json
from functools import cached_property

from .backend import call, invoke
from .paths import check_cancel


@dataclass(frozen=True)
class File:
    id: int
    name: str
    version: str
    category: str


@dataclass(frozen=True)
class State:
    game: str
    mod_id: int
    files: tuple[File, ...]
    updates: tuple[tuple[int, int], ...]
    endorsement: str = "Unknown"
    tracked: bool | None = None

    @cached_property
    def _native(self):
        return call("NexusFileState", json.dumps([asdict(file) for file in self.files]), list(self.updates), self.files)

    @cached_property
    def _updatable_files(self):
        return self._native.updatable

    @property
    def _replacement_cache(self):
        return self._native.cached

    def has_replacement(self, installed_file_id):
        _ = self._updatable_files
        return invoke(self._native.has_replacement, installed_file_id)

    def replacements(self, installed_file_id):
        return invoke(self._native.replacements, installed_file_id)


def identifier(value):
    return call("nexus_file_identifier", json.dumps(value))


def file_state(client, game, mod_id, cancel=None):
    check_cancel(cancel)
    result = json.loads(call("nexus_file_state", json.dumps(client.files(game, mod_id)), cancel))
    return State(game, mod_id, tuple(File(**file) for file in result["files"]),
                 tuple(tuple(edge) for edge in result["updates"]))


def endorsement_snapshot(endorsements, cancel=None):
    return {(game, identifier): status for game, identifier, status in
            json.loads(call("nexus_account_snapshot", json.dumps(endorsements), True, cancel))}


def tracking_snapshot(tracked, cancel=None):
    return {tuple(identity) for identity in json.loads(call("nexus_account_snapshot", json.dumps(tracked), False, cancel))}


def inspect(client, game, mod_id, cancel=None):
    return call("nexus_inspect", client, game, mod_id, cancel)
