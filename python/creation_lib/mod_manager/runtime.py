"""Bounded x64 FO4 launch plans: metadata-only import, explicit runtime scans.

The project filespace references payloads in place and routes writes to owned output.
Explicit legacy USVFS plans retain their separate source-write acknowledgement.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path

from .backend import call
from .launcher import Executable, LaunchPlan


SUPPORTED_VERSION = "0.5.7.2"
NOTICE = "usvfs - User-Space Virtual File System, Copyright (C) Sebastian Herbord"
REPOSITORY = "https://github.com/ModOrganizer2/usvfs"
WRITE_POLICY = "New Data files use this manager's overwrite. Programs can modify existing mapped source files; USVFS has no copy-on-write protection."


def pe_machine(path):
    from .backend import call
    return call("pe_machine", path)


def bundle(path):
    return call("runtime_bundle", path)


def known_folder(identifier):
    return Path(call("runtime_known_folder", identifier))


def fo4_user_targets():
    return call("runtime_user_targets")


def profile_signature(library, profile_id):
    return call("runtime_profile_signature", library, profile_id)


def captured_signature(profile, rows):
    return call("runtime_captured_signature", profile, rows)


def validate_f4se_configuration(game_root, library_root, profile_id, files):
    result = call("runtime_validate_f4se_configuration", game_root, library_root, profile_id, files)
    return Path(result) if result is not None else None


@dataclass(frozen=True)
class RuntimePlan:
    launch: LaunchPlan
    library_root: str
    profile_signature: str
    game_root: str
    bundle_root: str
    files: tuple[tuple[str, str, tuple], ...]
    snapshots: tuple[tuple[str, str, str], ...]
    plugin_target: str
    ini_target: str
    save_subpath: str
    minimal_ini: tuple[str, ...] = ()
    target_signatures: tuple[tuple[str, tuple], ...] = ()
    runtime: str = "modbox-vfs-fo4-x64"
    metadata_root: str = ""
    target_id: str = ""
    root_files: tuple = ()
    compatibility: tuple[str, ...] = ()
    experimental: bool = False
    save_root: str = ""
    managed_runtime: bool = False
    protected_roots: tuple[str, ...] = ()
    session_id: str = ""
    mcp_port: int | None = None
    disable_mcp: bool = False
    isolate_documents: bool = False

    def record(self):
        return asdict(self)


def prepare(library, profile_id, executable: Executable, game_root, bundle_root, cancel=None, progress=None, *, user_targets=None, isolate_documents=None):
    return call("runtime_prepare", library, profile_id, executable, game_root, bundle_root, cancel, progress, user_targets, isolate_documents)


def local_mod_target_for_user(path):
    from .mo2 import local_mod_target
    return local_mod_target(path)


def session_status(session):
    return call("runtime_session_status", session)


def helper_command(request):
    return call("runtime_helper_command", request, Path(__file__).resolve().parents[2])


def authorize_helper(process, session, profile_id):
    return call("runtime_authorize_helper", process, session, profile_id)


def start(library, plan, *, accept_source_writes=False, accept_minimal_ini=False, accept_experimental=False, cancel=None, spawn=None):
    return call("runtime_start", library, plan, accept_source_writes, accept_minimal_ini, accept_experimental, cancel, spawn)


def active_sessions(root):
    return call("runtime_active_sessions", root)


def steam_running():
    return call("runtime_steam_running")


def process_names():
    return set(call("runtime_process_names"))
