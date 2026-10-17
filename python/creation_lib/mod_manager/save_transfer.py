from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class File:
    name: str
    source: str
    size: int
    modified: int
    checksum: str


@dataclass(frozen=True)
class Plan:
    source_profile: str
    target_profile: str
    game: str
    files: tuple[File, ...]
    source_backup: str = ""
    backup_token: str = ""
    source_folder: str = ""
    folder_identity: tuple[int, int] | None = None


def _plan(value):
    return Plan(**{**value, "files": tuple(File(**item) for item in value["files"]),
                   "folder_identity": tuple(value["folder_identity"]) if value.get("folder_identity") is not None else None})


def root(library, profile):
    return Path(call("save_transfer_root", library, profile))


def checksum(path, cancel=None):
    return File(**json.loads(call("save_transfer_checksum", Path(path), cancel)))


def _available(destination, files):
    call("save_transfer_available", destination, json.dumps([asdict(item) for item in files]))


def _checkpoint(name):
    pass


def review(library, index, path, target_profile="", cancel=None):
    return _plan(json.loads(call("save_transfer_review", library, index, path, target_profile, cancel)))


def recover(library):
    from .runtime import active_sessions
    return call("save_transfer_recover", library, active_sessions, _checkpoint)


def apply(library, plan, cancel=None, progress=None):
    from .runtime import active_sessions
    return call("save_transfer_apply", library, json.dumps(asdict(plan)), active_sessions, _checkpoint, cancel, progress)
