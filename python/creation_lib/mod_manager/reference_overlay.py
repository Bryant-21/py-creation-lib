"""Owned additive updates; referenced providers are never copied or mutated."""
from dataclasses import dataclass, replace

from .backend import call


@dataclass(frozen=True)
class Review:
    library_id: str
    profile_id: str
    target_id: str
    base_id: str
    name: str
    source: str
    token: str


def records(db):
    import json
    from .backend import call
    return json.loads(call("overlay_records", db.native))


def effective(rows, db, profile_id, target_id=""):
    import json
    from dataclasses import asdict
    from .backend import call
    enabled = call("overlay_effective", json.dumps([asdict(row) for row in rows]), json.dumps(records(db)), profile_id, target_id)
    return [replace(row, enabled=value) for row, value in zip(rows, enabled)]


def capture(library, profile_id, base_id, cancel=None):
    return call("overlay_capture", library, profile_id, base_id, cancel)


def validate(library, profile_id, review, cancel=None):
    return call("overlay_validate", library, profile_id, review, cancel)


def install(library, review, package, plan, name, cancel=None, progress=None, *, nexus=None, enabled=True, validation=None):
    return call("overlay_install", library, review, package, plan, name, cancel, progress, nexus, enabled, validation)


def restore_links(db, snapshot):
    return call("overlay_restore_links", db, snapshot)
