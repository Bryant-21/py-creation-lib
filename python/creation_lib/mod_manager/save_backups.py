from dataclasses import dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class Backup:
    id: str
    profile: str
    game: str
    names: tuple[str, ...]
    sizes: tuple[int, ...]
    checksums: tuple[str, ...]
    modified: int
    token: str


def _backup(value):
    return Backup(**{**value, "names": tuple(value["names"]), "sizes": tuple(value["sizes"]),
                     "checksums": tuple(value["checksums"])})


def root(library, profile, identifier):
    return Path(call("save_backup_root", library, profile, identifier))


def _read(library, profile, identifier):
    return _backup(json.loads(call("save_backup_read", library, profile, identifier)))


def inventory(library, profile, cancel=None, progress=None):
    return tuple(_backup(value) for value in json.loads(call("save_backup_inventory", library, profile, cancel, progress)))


def review(library, profile, identifier, target_profile, cancel=None):
    return call("save_backup_review", library, profile, identifier, target_profile, cancel)


def validate(library, plan, cancel=None):
    call("save_backup_validate", library, plan, cancel)
