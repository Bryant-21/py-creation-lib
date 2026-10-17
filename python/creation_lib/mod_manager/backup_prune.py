from dataclasses import asdict, dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Review:
    kind: str
    profile_id: str
    backup_id: str
    name: str
    token: str
    files: int
    bytes: int


def _checkpoint(name):
    pass


def _operation(library, action, kind="", profile="", identifier="", plan=None, cancel=None, progress=None):
    return json.loads(call("backup_prune_operation", library, action, kind, profile, identifier,
                           json.dumps(asdict(plan) if plan else None), _checkpoint, cancel, progress))


def review(library, kind, profile, identifier, cancel=None, progress=None):
    return Review(**_operation(library, "review", kind, profile, identifier, cancel=cancel, progress=progress))


def _recover_locked(library):
    return _operation(library, "recover_locked")


def recover(library):
    return _operation(library, "recover")


def remove(library, plan, cancel=None, progress=None):
    return _operation(library, "remove", plan=plan, cancel=cancel, progress=progress)
