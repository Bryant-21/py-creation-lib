from dataclasses import dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Rename:
    profile_id: str
    mod_id: str
    name: str
    expected: str


def _state(db, profile_id, mod_id):
    return json.loads(call("metadata_state", db.native, profile_id, mod_id))


def review(library, profile_id, mod_id, cancel=None):
    with library._db() as db:
        name, expected = call("mod_name_review", db.native, profile_id, mod_id, cancel)
        return Rename(profile_id, mod_id, name, expected)


def _checkpoint():
    pass


def apply(library, plan, name, cancel=None):
    from .runtime import active_sessions
    with library.owned.lock(), library._db() as db:
        call("mod_name_apply", db.native, plan.profile_id, plan.mod_id, json.dumps(name), plan.expected,
             bool(active_sessions(library.root)), cancel, _checkpoint)
