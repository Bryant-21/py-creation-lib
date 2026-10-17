"""Executable metadata and explicit, reviewed process launches.

Importing an executable never runs it. A direct launch has no virtual filesystem.
Profile launches fail closed until a runtime adapter supplies a validated plan.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path
import json

from .backend import call


MODES = {"profile": "Selected mod profile", "direct": "Direct (no virtual mods)"}


def preferred_game(manager, explicit="", settings=None):
    return call("manager_preferred_game", manager, explicit, settings)


def selected_executable(manager, selected_id):
    return call("manager_selected_executable", manager, selected_id)


@dataclass(frozen=True)
class Executable:
    id: str
    title: str
    binary: str
    arguments: str = ""
    working_directory: str = ""
    mode: str = "profile"
    instance_id: str = ""
    hidden: bool = False
    steam_app_id: str = ""

    def record(self):
        return asdict(self)


def definition(**fields):
    return Executable(**json.loads(call("executable_definition", json.dumps(Executable(**fields).record()))))


def imported_definitions(configuration, instance_id):
    return (Executable(**value) for value in json.loads(call("executable_imported", json.dumps(configuration), instance_id)))


def windows_arguments(text):
    from .backend import call
    return call("windows_arguments", text)


def local_path(raw, label, *, directory=False):
    return Path(call("launch_local_path", str(raw), label, directory))

def fingerprint(path):
    info = Path(path).stat()
    return info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns


@dataclass(frozen=True)
class LaunchPlan:
    executable: Executable
    argv: tuple[str, ...]
    cwd: str
    signature: tuple
    profile_id: str
    profile_name: str
    runtime: str = "direct"


def review(executable, profile_id="", profile_name=""):
    result = json.loads(call("executable_review", json.dumps(executable.record()), profile_id, profile_name, fingerprint))
    result["executable"] = Executable(**result["executable"])
    result["argv"], result["signature"] = tuple(result["argv"]), tuple(result["signature"])
    return LaunchPlan(**result)


def start(plan, *, spawn=None, disable_mcp=False):
    return call("executable_start", plan, spawn, disable_mcp)
