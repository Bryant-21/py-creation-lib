from dataclasses import asdict, dataclass
import json
from pathlib import Path

from .backend import call
from .plugin_state import _catalog_json


@dataclass(frozen=True)
class Review:
    library_id: str
    mod_id: str
    profile_id: str
    old: str
    new: str
    action: str
    expected: str
    original: dict
    inputs: tuple
    state: dict



def _file(path, cancel=None):
    return json.loads(call("plugin_file_fingerprint", path, cancel))


def _state(db):
    return json.loads(call("plugin_file_state", db))


def unavailable(library, catalog, plugin):
    return call("plugin_file_unavailable", library, _catalog_json(catalog), json.dumps(asdict(plugin) if plugin is not None else None))


def review(library, catalog, plugin, action, new_name="", cancel=None):
    data = json.loads(call("plugin_file_review", library, _catalog_json(catalog), json.dumps(asdict(plugin) if plugin is not None else None), action, new_name, cancel))
    return Review(**{**data, "inputs": tuple(tuple(item) for item in data["inputs"])})


def _paths(library, record):
    return tuple(Path(p) for p in call("plugin_file_paths", library, json.dumps(record)))


def _recover_one(library, record):
    return json.loads(call("plugin_file_recover_one", library, json.dumps(record)))


def recover(library):
    return json.loads(call("plugin_file_recover", library))


def _checkpoint(phase):
    pass


def apply(library, plan, cancel=None, writer=None, *, restore=None):
    return call("plugin_file_apply", library, json.dumps(asdict(plan)), cancel, writer, json.dumps(restore), _checkpoint)


def undo(library, identifier, cancel=None):
    return call("plugin_file_undo", library, identifier, cancel, _checkpoint)
