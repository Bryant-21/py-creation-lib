"""Reviewed reference relinking and guarded metadata Undo; no source payload writes."""
from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class Review:
    library_id: str
    profile_id: str
    mod_id: str
    name: str
    old_source: str
    source: str
    files: int
    bytes: int
    profiles: int
    expected: str
    fingerprint: str
    directory_identity: tuple


def _directory(library, source):
    root, identity = json.loads(call("reference_directory", library, source))
    return Path(root), tuple(identity)


def review(library, profile, mod, source, cancel=None):
    with library._db() as db:
        result = json.loads(call("reference_review", db.native, library, profile, mod, source, cancel))
    result["directory_identity"] = tuple(result["directory_identity"])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, *, accept_identity=False, cancel=None):
    with library._db() as db:
        call("reference_apply", db.native, library, json.dumps(asdict(plan)), accept_identity, cancel, _checkpoint)


def undoable(library, profile):
    with library._db() as db:
        return set(call("reference_undoable", db.native, profile))


def undo(library, profile, mod, cancel=None):
    with library._db() as db:
        call("reference_undo", db.native, library, profile, mod, cancel, _checkpoint)
