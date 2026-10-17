"""Reviewed profile-local metadata removal; source payloads are never opened."""
from dataclasses import asdict, dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Review:
    profile_id: str
    profile_name: str
    mod_ids: tuple[str, ...]
    names: tuple[str, ...]
    kinds: tuple[str, ...]
    expected: str


def review(library, profile_id, selected, cancel=None):
    with library._db() as db:
        result = json.loads(call("reference_removal_review", db.native, profile_id, json.dumps(list(selected)), cancel))
    for name in ("mod_ids", "names", "kinds"):
        result[name] = tuple(result[name])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, cancel=None):
    with library.owned.lock(), library._db() as db:
        call("reference_removal_apply", db.native, library, json.dumps(asdict(plan)), cancel, _checkpoint)
