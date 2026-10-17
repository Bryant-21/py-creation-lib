from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class Review:
    profile_id: str
    token: str
    files: int
    bytes: int
    directories: tuple[str, ...]


def root(library, profile_id):
    return Path(call("overwrite_root", library, profile_id))


def _paths(library, record):
    return tuple(Path(path) for path in call("overwrite_paths", library, json.dumps(record)))


def _verify(library, path, plan, cancel=None, progress=None):
    manifest, directories = json.loads(call("overwrite_verify", library, path, json.dumps(asdict(plan)), cancel, progress))
    return manifest, tuple(directories)


def _checkpoint(name):
    pass


def _operation(library, action, profile="", identifier="", plan=None, name="", cancel=None, progress=None):
    return json.loads(call("overwrite_operation", library, action, profile, identifier,
                           json.dumps(asdict(plan) if plan else None), name, _checkpoint, cancel, progress))


def _review(value):
    return Review(**{**value, "directories": tuple(value["directories"])})


def review(library, profile_id, cancel=None, progress=None):
    return _review(_operation(library, "review", profile_id, cancel=cancel, progress=progress))


def history(library, profile_id):
    return _operation(library, "history", profile_id)


def recover(library):
    return _operation(library, "recover")


def clear(library, plan, cancel=None, progress=None):
    return _operation(library, "clear", plan=plan, cancel=cancel, progress=progress)


def review_restore(library, profile_id, identifier, cancel=None, progress=None):
    return _review(_operation(library, "review_restore", profile_id, identifier, cancel=cancel, progress=progress))


def restore(library, identifier, plan, cancel=None, progress=None):
    return _operation(library, "restore", identifier=identifier, plan=plan, cancel=cancel, progress=progress)


def copy_mod(library, plan, name, cancel=None, progress=None):
    return _operation(library, "copy_mod", plan=plan, name=name, cancel=cancel, progress=progress)
