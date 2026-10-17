from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call

LIMIT = 64 * 1024**2


@dataclass(frozen=True)
class Review:
    operation_id: str
    mod_id: str
    name: str
    kind: str
    token: str
    files: int
    bytes: int


def _paths(library, identifier):
    return tuple(Path(path) for path in call("history_prune_paths", library, identifier))


def _review(library, db, identifier, cancel=None, progress=None):
    plan, operation, manifest, directories = json.loads(call("history_prune_review", library, db.native, identifier, cancel, progress))
    return Review(**plan), operation, manifest, tuple(directories)


def _write(path, record):
    call("history_prune_write", path, json.dumps(record), LIMIT)


def _checkpoint(name):
    pass


def _operation(library, action, identifier="", plan=None, cancel=None, progress=None):
    return json.loads(call("history_prune_operation", library, action, identifier,
                           json.dumps(asdict(plan) if plan else None), LIMIT, _checkpoint, cancel, progress))


def review(library, identifier, cancel=None, progress=None):
    return Review(**_operation(library, "review", identifier, cancel=cancel, progress=progress))


def _recover_locked(library):
    return _operation(library, "recover_locked")


def recover(library):
    return _operation(library, "recover")


def remove(library, plan, cancel=None, progress=None):
    return _operation(library, "remove", plan=plan, cancel=cancel, progress=progress)
