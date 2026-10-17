from dataclasses import dataclass
from pathlib import Path
from .backend import call

LOCK = ".modbox21-f4se-root.lock"
MAX_FILE = 16 * 1024**2


@dataclass(frozen=True)
class Review:
    library_id: str
    target_id: str
    target_signature: str
    game_signature: tuple
    files: tuple
    receipt_id: str = ""
    receipt_signature: str = ""
    restore: bool = False
    temporaries: tuple = ()


def _checkpoint(phase):
    pass


def _snapshot(path):
    return call("f4se_root_snapshot", path)


def _base(library, target_id):
    return Path(call("f4se_root_base", library, target_id))


def _read(library, target_id, receipt_id):
    return call("f4se_root_read", library, target_id, receipt_id)


def _copy(source, destination, identifier, expected):
    return call("f4se_root_copy", source, destination, identifier, expected)


def history(library, target_id):
    return call("f4se_root_history", library, target_id)


def review(library, target_id, cancel=None):
    return call("f4se_root_review", library, target_id, cancel)


def apply(library, plan, *, accept_root_writes=False, cancel=None):
    return call("f4se_root_apply", library, plan, accept_root_writes, cancel)


def review_restore(library, target_id, receipt_id="", cancel=None):
    return call("f4se_root_review_restore", library, target_id, receipt_id, cancel)


def restore(library, plan, *, accept_root_writes=False, cancel=None):
    return call("f4se_root_restore", library, plan, accept_root_writes, cancel)
