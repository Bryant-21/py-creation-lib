from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path

from .backend import call


def identity(value: str):
    return call("manager_identity", value)


def read_text(path: Path, limit=8 * 1024**2):
    return call("metadata_read_text", path, limit)


def read_ini(path: Path):
    return call("mo2_read_ini", Path(path), read_text)


@dataclass(frozen=True)
class ImportedMod:
    id: str
    name: str
    source: str
    kind: str
    enabled: bool
    group: str
    version: str = ""
    category: str = ""
    notes: str = ""
    nexus_game: str = ""
    nexus_mod_id: int = 0
    nexus_file_id: int = 0
    tags: str = ""
    categories: tuple[str, ...] = ()


@dataclass
class ImportedProfile:
    id: str
    name: str
    source: str
    mods: list[ImportedMod]
    files: dict[str, str]


@dataclass
class LinkedSource:
    name: str
    alias: str
    target: str
    source: str
    status: str
    can_use_target: bool


@dataclass
class ImportPreview:
    root: str
    mods_root: str
    profiles_root: str
    game: str
    profiles: list[ImportedProfile]
    warnings: list[str] = field(default_factory=list)
    configuration: dict = field(default_factory=dict)
    linked_sources: list[LinkedSource] = field(default_factory=list)
    inventory: list[ImportedMod] = field(default_factory=list)


def qt_string(raw):
    return call("mo2_qt_string", raw)


def instance_configuration(config, root, base, mods, profiles):
    return json.loads(call("mo2_configuration", config, root, base, mods, profiles))


def _ordinary_source(path):
    return Path(call("mo2_ordinary_source", Path(path)))


def local_mod_target(path):
    return call("local_mod_target", Path(path))


def _mod(value):
    return ImportedMod(**{**value, "categories": tuple(value["categories"])})


def inspect_instance(path: str | Path, cancel=None, source_mappings=None):
    mappings = json.dumps({str(key): value for key, value in (source_mappings or {}).items()}, default=str)
    result = json.loads(call("mo2_inspect", Path(path), cancel, mappings, read_text, local_mod_target, _ordinary_source))
    result["profiles"] = [ImportedProfile(**{**value, "mods": [_mod(mod) for mod in value["mods"]]}) for value in result["profiles"]]
    result["inventory"] = [_mod(value) for value in result["inventory"]]
    result["linked_sources"] = [LinkedSource(**value) for value in result["linked_sources"]]
    return ImportPreview(**result)
