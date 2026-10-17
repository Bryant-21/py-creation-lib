from dataclasses import dataclass
import json

from .backend import call


@dataclass(frozen=True)
class Category:
    name: str
    nexus_id: int
    usage: int


def name(value):
    return call("category_name", json.dumps(value))


def memberships(primary, encoded=""):
    return tuple(call("category_memberships", primary, encoded))


def normalized(values):
    return tuple(call("category_normalized", json.dumps(values)))


def matches_filter(values, category):
    return call("category_matches", list(values), category)


def _snapshot(db):
    configured, assigned = call("category_snapshot", db.native)
    return configured, [(key, tuple(values)) for key, values in assigned]


def token(db):
    return call("category_token", db.native)


def inventory(library):
    with library._db() as db:
        rows, token = call("category_inventory", db.native)
        return tuple(Category(*row) for row in rows), token


def nexus_catalog(library, client, game, refresh=False, merge=False, cancel=None):
    return call("nexus_categories", library, client, game, refresh, merge, cancel)


def save(library, value, nexus_id=0, cancel=None):
    with library._db() as db:
        call("category_save", db.native, json.dumps(value), json.dumps(nexus_id), cancel)


def rename(library, original, replacement, expected, cancel=None):
    with library._db() as db:
        call("category_rename", db.native, original, json.dumps(replacement), expected, cancel)


def remove(library, value, expected, cancel=None):
    with library._db() as db:
        call("category_remove", db.native, value, expected, cancel)


def assign(library, profile_id, mod_ids, category, cancel=None):
    with library._db() as db:
        call("category_assign", db.native, profile_id, list(mod_ids), json.dumps(category), cancel)
