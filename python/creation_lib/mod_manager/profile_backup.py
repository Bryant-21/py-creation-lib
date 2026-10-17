"""Recoverable local profile metadata snapshots; no payload copy or game write."""
from dataclasses import asdict, dataclass
import json

from .backend import call


def encode(value):
    return call("metadata_encode", json.dumps(value))


def digest(data):
    return call("metadata_digest", json.dumps(data))


def snapshot(db, profile_id):
    return json.loads(call("metadata_snapshot", db.native, profile_id))


def _operation(library, action, profile_id, backup_id="", plan=None, cancel=None):
    with library._db() as db:
        return json.loads(call("profile_backup", db.native, db, library, action, profile_id, backup_id,
                               json.dumps(plan), cancel))


def capture(library, profile_id, cancel=None):
    return _operation(library, "capture", profile_id, cancel=cancel)


def history(library, profile_id):
    return _operation(library, "history", profile_id)


@dataclass(frozen=True)
class Restore:
    profile_id: str
    backup_id: str
    expected: str
    backup_digest: str
    name: str
    entries: int
    files: int



def review(library, profile_id, backup_id, cancel=None):
    return Restore(**_operation(library, "review", profile_id, backup_id, cancel=cancel))


def apply(library, plan, cancel=None):
    _operation(library, "apply", plan.profile_id, plan.backup_id, asdict(plan), cancel)


def remove(library, profile_id, backup_id, cancel=None):
    _operation(library, "remove", profile_id, backup_id, cancel=cancel)
