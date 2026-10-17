"""Reversible metadata-only target removal; all owned and external files remain retained."""
from dataclasses import asdict, dataclass
import json

from .launch_targets import Target
from .backend import call


@dataclass(frozen=True)
class Review:
    library_id: str
    target_id: str
    name: str
    retained_root: str
    expected: str
    restore: bool = False
    root_history: str = ""


def removed(library, db=None):
    if db is None:
        with library._db() as connection:
            return removed(library, connection)
    return [Target(**record) for record in json.loads(call("target_removed", db.native))]


def review(library, identifier, *, restore=False, cancel=None):
    with library._db() as db:
        return Review(**json.loads(call("target_removal_review", db.native, library, identifier, restore, cancel)))


def _checkpoint():
    pass


def apply(library, plan, cancel=None):
    with library.owned.lock(), library._db() as db:
        return call("target_removal_apply", db.native, library, json.dumps(asdict(plan)), cancel, _checkpoint)
