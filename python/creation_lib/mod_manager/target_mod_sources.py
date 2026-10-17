"""Explicit per-target mod source/version/enable overrides; no payload copies or shared edits."""
from dataclasses import asdict, dataclass
import json
from .backend import call

_UNSET = object()


@dataclass(frozen=True)
class Review:
    library_id: str
    target_id: str
    profile_id: str
    mod_id: str
    name: str
    shared_source: str
    record: dict
    expected: str
    identity: tuple = ()
    inventory: str = ""
    files: int = 0
    bytes: int = 0


def records(library, target_id, db=None):
    if db is None:
        with library._db() as connection:
            return records(library, target_id, connection)
    return json.loads(call("target_source_records", db.native, target_id))


def effective(row, override):
    from .library import ModRow
    result = json.loads(call("target_source_effective", json.dumps(asdict(row)), json.dumps(override)))
    result["categories"] = tuple(result["categories"])
    return ModRow(**result)


def review(library, target_id, profile_id, mod_id, *, source="", version="", enabled=None, nexus_file_id=_UNSET, reset=False, cancel=None):
    request = {"source": source, "version": version, "enabled": enabled, "reset": reset}
    if nexus_file_id is not _UNSET:
        request["nexus_file_id"] = nexus_file_id
    with library._db() as db:
        result = json.loads(call("target_source_review", db.native, library, target_id, profile_id, mod_id, json.dumps(request), cancel))
    result["identity"] = tuple(result["identity"])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, *, accept_identity=False, cancel=None):
    with library.owned.lock(), library._db() as db:
        return call("target_source_apply", db.native, library, json.dumps(asdict(plan)), accept_identity, cancel, _checkpoint)


def set_enabled(library, target_id, profile_id, mod_ids, enabled, cancel=None):
    with library.owned.lock(), library._db() as db:
        call("target_source_set_enabled", db.native, library, target_id, profile_id, json.dumps(list(mod_ids)), json.dumps(enabled), cancel, _checkpoint)
