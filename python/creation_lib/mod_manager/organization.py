import json

from .backend import call


def entries(db, profile_id):
    return json.loads(call("organization_entries", db.native, profile_id))


def digest(rows):
    return call("organization_digest", json.dumps(rows))


def commit_history(db, profile_id, before, kind):
    return call("organization_history", db.native, profile_id, json.dumps(before), kind)


def _checkpoint():
    pass


def bulk(library, profile_id, mod_ids, *, enabled=None, group=None, priority=None, expected=None, cancel=None):
    library._runtime_write_guard()
    with library._db() as db:
        call("organization_bulk", db.native, profile_id, list(mod_ids), enabled, group, priority, expected, cancel, _checkpoint)
        library._runtime_write_guard()


def separator(library, profile_id, name, group="", cancel=None):
    with library._db() as db:
        return call("organization_separator", db.native, profile_id, name, group, cancel)


def empty_mod(library, profile_id, name, after="", cancel=None):
    return call("transaction_empty_mod", library, profile_id, name, after, cancel)


def undo(library, profile_id, cancel=None):
    with library._db() as db:
        call("organization_undo", db.native, profile_id, cancel)
