from dataclasses import asdict, dataclass
import json
from pathlib import Path
import uuid

from .backend import call
from .profile_state import effective_ini

MAX_BYTES, MAX_ENTRIES = 64 * 1024**2, 100_000


@dataclass(frozen=True)
class Entry:
    id: str
    name: str
    kind: str
    enabled: bool
    group: str
    version: str = ""


@dataclass(frozen=True)
class Recipe:
    path: str
    checksum: str
    library: str
    profile: str
    name: str
    game: str
    entries: tuple[Entry, ...]
    files: tuple[tuple[str, str], ...]
    minimal_inis: tuple[str, ...]


@dataclass(frozen=True)
class ImportPlan:
    recipe: Recipe
    template: str
    name: str
    mapping: tuple[tuple[str, str], ...]
    expected: str
    target: str
    existing: bool


@dataclass(frozen=True)
class Candidate:
    id: str
    name: str
    source: str
    kind: str


def _recipe(record):
    record["entries"] = tuple(Entry(**item) for item in record["entries"])
    record["files"] = tuple(tuple(item) for item in record["files"])
    record["minimal_inis"] = tuple(record["minimal_inis"])
    return Recipe(**record)


def _checkpoint(stage):
    pass


def export(library, profile_id, cancel=None, progress=None):
    return Path(call("recipe_export", library, profile_id, uuid.uuid4().hex, MAX_BYTES, MAX_ENTRIES,
                     effective_ini, _checkpoint, cancel, progress))


def read(path, cancel=None):
    return _recipe(json.loads(call("recipe_read", Path(path), MAX_BYTES, MAX_ENTRIES, cancel)))


def _operation(library, action, template="", recipe=None, mapping=None, name="", cancel=None):
    from .runtime import active_sessions
    return json.loads(call("recipe_operation", library, action, template, json.dumps(asdict(recipe) if recipe else None),
                           json.dumps(mapping), name, MAX_BYTES, MAX_ENTRIES, _checkpoint, active_sessions, cancel))


def suggestions(library, template_id, recipe):
    return _operation(library, "suggestions", template_id, recipe)


def candidates(library, game):
    return tuple(Candidate(**row) for row in _operation(library, "candidates", name=game))


def review(library, template_id, recipe, mapping, name, cancel=None):
    record = _operation(library, "review", template_id, recipe, mapping, name, cancel)
    record["recipe"] = recipe
    record["mapping"] = tuple(tuple(item) for item in record["mapping"])
    return ImportPlan(**record)


def apply(library, plan, cancel=None):
    return tuple(_operation(library, "apply", plan.template, plan.recipe, asdict(plan), cancel=cancel))
