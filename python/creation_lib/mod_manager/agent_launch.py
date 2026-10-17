"""Headless command adapter for the shared Rust mod-manager backend."""
import argparse
import ctypes as ct
from ctypes import wintypes as w
import json
import subprocess
import sys

from .backend import call
from .paths import ManagerError


AGENT_DEFAULTS = (
    "Fallout4Custom.ini:General:bAlwaysActive=1",
    "Fallout4Custom.ini:Controls:bBackgroundMouse=1",
    "Fallout4Prefs.ini:Display:bFull Screen=0",
    "Fallout4Prefs.ini:Display:iSize W=1920",
    "Fallout4Prefs.ini:Display:iSize H=1080",
    "F4SE/Plugins/HighFPSPhysicsFix.ini:Window:LockCursor=false",
    "F4SE/Plugins/HighFPSPhysicsFix_custom.ini:Window:LockCursor=false",  # read after the main INI when present
)


def default_agent_library():
    return call("agent_default_library")


def instances(root=None):
    return call("agent_instances", root)


def empty_mod(library_root, profile, name, after=""):
    return call("agent_empty_mod", library_root, profile, name, after)


def archives(library_root, profile, archive="", member="", target=""):
    return call("agent_archives", library_root, profile, archive, member, target)


def categories(library_root, game, refresh=False, merge=False, api_key=""):
    return call("agent_categories", library_root, game, refresh, merge, api_key)


def nexus_auth(action, api_key="", remember=False):
    return call("agent_nexus_auth", action, api_key, remember)


def create_instance(library_root, game, game_root, name="Default"):
    return call("agent_create_instance", library_root, game, game_root, name)


def install(library_root, profile, package, **options):
    return call("agent_install", library_root, profile, package, json.dumps(options))


def actions():
    return call("agent_actions")


def workflow(library_root, profile, steps, *, target="", apply=False):
    return call("agent_workflow", library_root, profile, target, json.dumps(steps), apply)


def sync(instance, library_root):
    return call("agent_sync", instance, library_root)


def parse_override(spec):
    return call("agent_parse_override", spec)


def set_ini_values(text, sections):
    return call("agent_set_ini_values", text, sections)


def apply_launch_settings(library, plan, profile_files, specs, ini_source="auto"):
    return call("agent_apply_launch_settings", library, plan, profile_files, specs, ini_source)


def detached_spawn(args, *, popen=subprocess.Popen, **kwargs):
    return call("agent_detached_spawn", args, popen, kwargs)


def processes(status):
    return call("agent_processes", status)


def launch(library_root, profile, executable="f4se", *, overrides=(), agent_defaults=False, ini_source="auto",
           accept_minimal_ini=False, wait_game=0.0, dry_run=False, user_targets=None, cancel=None, mcp_port=None, disable_mcp=False, isolate_documents=None):
    return call("agent_launch", library_root, profile, executable, {
        "overrides": overrides, "agent_defaults": agent_defaults, "ini_source": ini_source,
        "accept_minimal_ini": accept_minimal_ini, "wait_game": wait_game, "dry_run": dry_run,
        "user_targets": user_targets, "cancel": cancel, "mcp_port": mcp_port, "disable_mcp": disable_mcp,
        "isolate_documents": isolate_documents,
    })


def import_saves(library_root, profile, names=(), *, latest=False, folder=None):
    return call("agent_import_saves", library_root, profile, names, latest, folder)


def launch_options(library_root, profile, isolate_documents=None):
    return call("agent_launch_options", library_root, profile, isolate_documents)


def status(library_root, session=None):
    return call("agent_status", library_root, session)


def stop(library_root, pid, *, timeout=30.0):
    return call("agent_stop", library_root, int(pid), timeout)


def _kernel():
    api = ct.WinDLL("kernel32", use_last_error=True)
    api.TerminateProcess.argtypes, api.TerminateProcess.restype = [w.HANDLE, w.UINT], w.BOOL
    api.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ct.POINTER(w.DWORD)]
    api.QueryFullProcessImageNameW.restype = w.BOOL
    return api


def main(argv=None):
    parser = argparse.ArgumentParser(prog="python -m creation_lib.mod_manager.agent_launch", description=__doc__.split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    listing = commands.add_parser("instances", help="List MO2 instances (read-only)")
    listing.add_argument("--root", default="")
    for name in ("sync", "launch", "import-saves", "status", "stop"):
        command = commands.add_parser(name)
        command.add_argument("--library", default="", help="Agent library (default: <toolkit db>/mod_manager/agent-library)")
        if name in {"sync", "launch"}:
            command.add_argument("--mo2", default="", required=name == "sync", help="MO2 instance folder or ModOrganizer.ini")
    launching = commands.choices["launch"]
    launching.add_argument("--profile", required=True)
    launching.add_argument("--exe", default="f4se", help="Executable title/id; 'f4se' selects the instance game's f4se_loader.exe")
    launching.add_argument("--mcp-port", type=int, help="Per-process MCP port; omitted selects an available port")
    launching.add_argument("--disable-mcp", action="store_true", help="Keep B21_MCP disabled for this launch")
    launching.add_argument("--isolate-documents", action=argparse.BooleanOptionalAction, default=None,
                           help="Use private session Documents files instead of shared saves and logs")
    launching.add_argument("--agent-defaults", action="store_true", help="Background mouse, always active, windowed 1920x1080, HighFPSPhysicsFix LockCursor=false")
    launching.add_argument("--ini", action="append", default=[], metavar="FILE:SECTION:KEY=VALUE",
                           help="FILE is Fallout4.ini/Fallout4Prefs.ini/Fallout4Custom.ini, or a path relative to Data, "
                                "e.g. F4SE/Plugins/FO4CS.ini:Menu:iFocusLossMode=2")
    launching.add_argument("--ini-source", choices=("auto", "profile", "global"), default="auto")
    launching.add_argument("--accept-minimal-ini", action="store_true")
    launching.add_argument("--wait-game", type=float, default=0.0, help="Seconds to wait for Fallout4.exe to appear")
    launching.add_argument("--dry-run", action="store_true", help="Plan and write the mapping report without starting anything")
    saving = commands.choices["import-saves"]
    saving.add_argument("--profile", required=True)
    saving.add_argument("--save", action="append", default=[], help="Save filename to copy (with its .f4se companion)")
    saving.add_argument("--latest", action="store_true", help="Also copy the newest save in the folder")
    saving.add_argument("--folder", default="", help="Source save folder (default: Documents/My Games/Fallout4/Saves)")
    commands.choices["status"].add_argument("--session", default="")
    commands.choices["stop"].add_argument("--pid", type=int, required=True)
    commands.choices["stop"].add_argument("--timeout", type=float, default=30.0)
    args = parser.parse_args(argv)
    try:
        result = call("agent_dispatch", args)
    except ManagerError as exc:
        print(json.dumps({"error": str(exc)}))
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
