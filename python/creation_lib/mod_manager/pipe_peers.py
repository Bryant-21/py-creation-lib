"""Read-only Windows pipe/process/SID verification; no tokens on disk or ACL changes."""
import ctypes as ct
from ctypes import wintypes as w
from functools import lru_cache

from .backend import call
from .process_identity import kernel_api


@lru_cache(maxsize=1)
def _apis():
    kernel = kernel_api()
    security = ct.WinDLL("advapi32", use_last_error=True)
    for api, name, args, result in (
        (kernel, "GetNamedPipeServerProcessId", [w.HANDLE, ct.POINTER(w.ULONG)], w.BOOL),
        (kernel, "GetNamedPipeClientProcessId", [w.HANDLE, ct.POINTER(w.ULONG)], w.BOOL),
        (security, "OpenProcessToken", [w.HANDLE, w.DWORD, ct.POINTER(w.HANDLE)], w.BOOL),
        (security, "GetTokenInformation", [w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD, ct.POINTER(w.DWORD)], w.BOOL),
        (security, "ConvertSidToStringSidW", [ct.c_void_p, ct.POINTER(w.LPWSTR)], w.BOOL),
        (kernel, "LocalFree", [ct.c_void_p], ct.c_void_p),
    ):
        function = getattr(api, name)
        function.argtypes, function.restype = args, result
    return kernel, security

@lru_cache(maxsize=1)
def _current_sid():
    return call("pipe_current_sid")


def process(pid):
    return call("pipe_process", pid)


def peer(connection, *, server):
    return call("pipe_peer", connection, server)
