from dataclasses import dataclass, field
import json
from .backend import call
from .cli_reader import run_json
from .conflicts import Provider


@dataclass(frozen=True)
class Archive:
    name: str
    provider: Provider
    selected: bool
    activation: str
    reason: str
    rank: int
    members: tuple[str, ...] = ()
    format: str = ""

    @property
    def identity(self):
        return self.provider.mod_id, self.provider.path, self.provider.priority


@dataclass(frozen=True)
class Member:
    path: str
    archives: tuple[Archive, ...]

    @property
    def candidates(self):
        return call("archive_member_candidates", self)


@dataclass(frozen=True)
class ArchiveIndex:
    profile_id: str
    archives: tuple[Archive, ...]
    members: tuple[Member, ...]
    issues: tuple[str, ...]
    signature: tuple
    effective_ini: dict = field(default_factory=dict)
    plugin_signature: tuple = ()



def plugin_signature(catalog):
    return call("archive_plugin_signature", catalog)


def source_stamp(path):
    return call("archive_source_stamp", path)


def preview(archive, member, cancel=None):
    return call("archive_preview", archive, member, cancel)


class CachedArchives:
    def __init__(self, library, reader=None):
        self.library, self.reader = library, reader or run_json

    def __call__(self, path, cancel=None):
        return json.loads(call("archive_cached", self.library, self.reader, path, cancel))

    @staticmethod
    def validate(data):
        return json.loads(call("archive_validate", json.dumps(data)))


def ini_settings(files):
    return call("archive_ini_settings", json.dumps(files))


def ini_archives(files):
    return tuple(call("archive_ini_archives", json.dumps(files)))


def build(library, profile_id, cancel=None, progress=None, reader=None, *, loose=None):
    return call("archive_build", library, profile_id, cancel, progress, reader, loose)
