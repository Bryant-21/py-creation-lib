from dataclasses import dataclass
import json
from pathlib import Path

from .backend import call


@dataclass(frozen=True)
class Save:
    name: str
    path: str
    size: int
    modified: int
    companion: str = ""
    identity: tuple[int, int] | None = None


@dataclass(frozen=True)
class Index:
    profile_id: str
    game: str
    root: str
    files: tuple[Save, ...]
    external: bool = False
    identity: tuple[int, int] | None = None


def _index(value):
    return Index(**{**value, "files": tuple(Save(**{**item, "identity": tuple(item["identity"]) if item["identity"] is not None else None}) for item in value["files"]),
                    "identity": tuple(value["identity"]) if value["identity"] is not None else None})


def inventory(library, profile_id, cancel=None, progress=None):
    return _index(json.loads(call("saves_inventory", library, profile_id, None, cancel, progress)))


def inventory_folder(library, profile_id, folder, cancel=None, progress=None):
    return _index(json.loads(call("saves_inventory", library, profile_id, Path(folder), cancel, progress)))


def fo4_header(save, cancel=None):
    return json.loads(call("saves_fo4_header", save, cancel))
