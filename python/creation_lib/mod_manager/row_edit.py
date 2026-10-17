from dataclasses import dataclass
import json

from .backend import call

FIELDS = {"notes", "category", "version", "nexus", "priority"}


@dataclass(frozen=True)
class Review:
    library_id: str
    profile_id: str
    mod_id: str
    field: str
    value: str
    expected: str


def _state(db, profile, mod, field):
    return json.loads(call("metadata_state", db.native, profile, mod, field))


def review(library, profile, mod, field, cancel=None):
    with library._db() as db:
        value, expected = call("metadata_review", db.native, profile, mod, field, cancel)
        return Review(library.library_id, profile, mod, field, value, expected)


def _value(field, value):
    result = json.loads(call("metadata_value", field, json.dumps(value)))
    return tuple(result) if isinstance(result, list) else result


def _checkpoint():
    pass


def apply(library, plan, value, cancel=None):
    library._runtime_write_guard()
    with library._db() as db:
        call("metadata_apply", db.native, plan.profile_id, plan.mod_id, plan.field, json.dumps(value),
             plan.expected, library.library_id, plan.library_id, cancel, _checkpoint)
        library._runtime_write_guard()
