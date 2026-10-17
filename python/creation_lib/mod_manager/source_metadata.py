from dataclasses import asdict, dataclass
import json

from .backend import call

LIMIT = 1024**2
LABELS = {"version": "Version", "notes": "Notes", "tags": "Tags", "categories": "Categories", "nexus": "Nexus association"}


@dataclass(frozen=True)
class Field:
    name: str
    current: object
    value: object
    issue: str = ""
    warning: str = ""


@dataclass(frozen=True)
class Review:
    library_id: str
    profile_id: str
    mod_id: str
    name: str
    profiles: int
    source: str
    expected: str
    inputs: tuple
    fields: tuple[Field, ...]


def review(library, profile, mod, category_file="", cancel=None):
    with library._db() as db:
        result = json.loads(call("source_metadata_review", db.native, library, profile, mod, str(category_file), cancel))
    result["inputs"] = tuple((path, tuple(stamp), digest) for path, stamp, digest in result["inputs"])
    result["fields"] = tuple(Field(**{**value,
        "current": tuple(value["current"]) if isinstance(value["current"], list) else value["current"],
        "value": tuple(value["value"]) if isinstance(value["value"], list) else value["value"]}) for value in result["fields"])
    return Review(**result)


def _checkpoint():
    pass


def apply(library, plan, selected, cancel=None):
    with library._db() as db:
        call("source_metadata_apply", db.native, library, json.dumps(asdict(plan)), list(selected), cancel, _checkpoint)


def undoable(library, profile):
    with library._db() as db:
        return set(call("source_metadata_undoable", db.native, profile))


def undo(library, profile, mod, cancel=None):
    with library._db() as db:
        call("source_metadata_undo", db.native, library, profile, mod, cancel, _checkpoint)
