"""Handle-derived Windows process identities; recovery never terminates by PID."""
from __future__ import annotations

import ctypes as ct
from ctypes import wintypes as w
from functools import lru_cache
import os

from .paths import ManagerError
from .backend import call


@lru_cache(maxsize=1)
def kernel_api():
    if os.name != "nt":
        raise ManagerError("Runtime process identities require Windows.")
    api = ct.WinDLL("kernel32", use_last_error=True)
    for name, args, result in (
        ("GetProcessId", [w.HANDLE], w.DWORD),
        ("GetProcessTimes", [w.HANDLE] + [ct.POINTER(w.FILETIME)] * 4, w.BOOL),
        ("GetCurrentProcess", [], w.HANDLE),
        ("OpenProcess", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE),
        ("WaitForSingleObject", [w.HANDLE, w.DWORD], w.DWORD),
        ("CloseHandle", [w.HANDLE], w.BOOL),
    ):
        function = getattr(api, name)
        function.argtypes, function.restype = args, result
    return api


def valid_identity(value):
    return call("valid_process_identity", value)


def from_handle(pid, handle, *, kernel=None):
    return call("process_from_handle", pid, handle, kernel)


def current_identity():
    return call("current_process_identity")


def process_state(identity, *, kernel=None):
    return call("process_identity_state", identity, kernel)


def session_liveness(status):
    return call("session_process_liveness", status)


def terminate_owned(process, identity):
    return call("terminate_owned_process", process, identity)
