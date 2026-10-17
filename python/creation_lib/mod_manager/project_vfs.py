import ctypes as ct
from ctypes import wintypes as w

from .backend import call
from .native_runtime import NativeVFS, StartupInfo, ProcessInfo


class NativeFilespace(NativeVFS):
    def __init__(self, root, session):
        call("vfs_initialize", self, root, session, True)

    def _load(self, dll):
        self.library = ct.WinDLL(dll, use_last_error=True, winmode=0x100 | 0x800)
        self.library.MBVFS_Version.argtypes, self.library.MBVFS_Version.restype = [], w.DWORD
        self.library.MBVFS_Inject.argtypes, self.library.MBVFS_Inject.restype = [w.HANDLE, w.LPCWSTR], w.DWORD
        self.kernel = ct.WinDLL("kernel32", use_last_error=True)
        bindings = {
            "CloseHandle": ([w.HANDLE], w.BOOL), "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
            "OpenProcess": ([w.DWORD, w.BOOL, w.DWORD], w.HANDLE), "GetProcessId": ([w.HANDLE], w.DWORD),
            "GetProcessTimes": ([w.HANDLE] + [ct.POINTER(w.FILETIME)] * 4, w.BOOL),
            "IsProcessInJob": ([w.HANDLE, w.HANDLE, ct.POINTER(w.BOOL)], w.BOOL),
            "GetExitCodeProcess": ([w.HANDLE, ct.POINTER(w.DWORD)], w.BOOL), "CreateJobObjectW": ([ct.c_void_p, w.LPCWSTR], w.HANDLE),
            "SetInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD], w.BOOL),
            "QueryInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD, ct.POINTER(w.DWORD)], w.BOOL),
            "InitializeProcThreadAttributeList": ([ct.c_void_p, w.DWORD, w.DWORD, ct.POINTER(ct.c_size_t)], w.BOOL),
            "UpdateProcThreadAttribute": ([ct.c_void_p, w.DWORD, ct.c_size_t, ct.c_void_p, ct.c_size_t, ct.c_void_p, ct.c_void_p], w.BOOL),
            "DeleteProcThreadAttributeList": ([ct.c_void_p], None), "ResumeThread": ([w.HANDLE], w.DWORD),
            "CreateProcessW": ([w.LPCWSTR, w.LPWSTR, ct.c_void_p, ct.c_void_p, w.BOOL, w.DWORD,
                                ct.c_void_p, w.LPCWSTR, ct.POINTER(StartupInfo), ct.POINTER(ProcessInfo)], w.BOOL),
        }
        for name, (arguments, result) in bindings.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result

    def configure(self, owner, config, protected=()):
        return call("filespace_configure", self, owner, config, protected)

    def link_file(self, source, target):
        return call("vfs_link", self, source, target, True, False, False)

    def link_directory(self, source, target, *, create=False):
        return call("vfs_link", self, source, target, True, True, create)

    def _write_config(self):
        return call("filespace_write_config", self)

    def start(self, argv, cwd, *, before_resume=None, environment=None):
        return call("vfs_start", self, argv, cwd, before_resume, environment, True)

    def running_processes(self):
        return call("vfs_running", self, True)
