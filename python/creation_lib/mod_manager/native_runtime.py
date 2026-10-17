import ctypes as ct
from ctypes import wintypes as w

from .backend import call
from .paths import ManagerError


class StartupInfo(ct.Structure):
    _fields_ = [("cb", w.DWORD), ("reserved", w.LPWSTR), ("desktop", w.LPWSTR), ("title", w.LPWSTR),
                ("x", w.DWORD), ("y", w.DWORD), ("xsize", w.DWORD), ("ysize", w.DWORD),
                ("xchars", w.DWORD), ("ychars", w.DWORD), ("fill", w.DWORD), ("flags", w.DWORD),
                ("show", w.WORD), ("reservedsize", w.WORD), ("reserved2", ct.POINTER(ct.c_ubyte)),
                ("stdin", w.HANDLE), ("stdout", w.HANDLE), ("stderr", w.HANDLE)]


class ProcessInfo(ct.Structure):
    _fields_ = [("process", w.HANDLE), ("thread", w.HANDLE), ("pid", w.DWORD), ("tid", w.DWORD)]


class StartupInfoEx(ct.Structure):
    _fields_ = [("startup", StartupInfo), ("attributes", ct.c_void_p)]


class JobBasic(ct.Structure):
    _fields_ = [("process_time", ct.c_longlong), ("job_time", ct.c_longlong), ("flags", w.DWORD),
                ("minimum_ws", ct.c_size_t), ("maximum_ws", ct.c_size_t), ("process_limit", w.DWORD),
                ("affinity", ct.c_size_t), ("priority", w.DWORD), ("scheduling", w.DWORD)]


class JobExtended(ct.Structure):
    _fields_ = [("basic", JobBasic), ("io", ct.c_ulonglong * 6), ("process_memory", ct.c_size_t),
                ("job_memory", ct.c_size_t), ("peak_process_memory", ct.c_size_t), ("peak_job_memory", ct.c_size_t)]


class NativeVFS:
    def __init__(self, root, session):
        call("vfs_initialize", self, root, session, False)

    def _load(self, dll):
        self.api = ct.WinDLL(dll, use_last_error=True, winmode=0x100 | 0x800)
        self.parameters = ct.CDLL(dll, use_last_error=True, winmode=0x100 | 0x800)
        self.kernel = ct.WinDLL("kernel32", use_last_error=True)
        self._bind()

    def _bind(self):
        functions = {
            "usvfsVersionString": ([], ct.c_char_p),
            "usvfsCreateVFS": ([ct.c_void_p], w.BOOL),
            "usvfsDisconnectVFS": ([], None),
            "usvfsVirtualLinkFile": ([w.LPCWSTR, w.LPCWSTR, ct.c_uint], w.BOOL),
            "usvfsVirtualLinkDirectoryStatic": ([w.LPCWSTR, w.LPCWSTR, ct.c_uint], w.BOOL),
            "usvfsGetVFSProcessList": ([ct.POINTER(ct.c_size_t), ct.POINTER(w.DWORD)], w.BOOL),
            "usvfsCreateProcessHooked": ([w.LPCWSTR, w.LPWSTR, ct.c_void_p, ct.c_void_p, w.BOOL, w.DWORD,
                                         ct.c_void_p, w.LPCWSTR, ct.POINTER(StartupInfo), ct.POINTER(ProcessInfo)], w.BOOL),
        }
        parameters = {"usvfsCreateParameters": ([], ct.c_void_p), "usvfsFreeParameters": ([ct.c_void_p], None),
                      "usvfsSetInstanceName": ([ct.c_void_p, ct.c_char_p], None),
                      "usvfsSetDebugMode": ([ct.c_void_p, w.BOOL], None),
                      "usvfsSetLogLevel": ([ct.c_void_p, ct.c_uint8], None),
                      "usvfsSetCrashDumpType": ([ct.c_void_p, ct.c_uint8], None)}
        kernel = {"CloseHandle": ([w.HANDLE], w.BOOL), "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
                  "OpenProcess": ([w.DWORD, w.BOOL, w.DWORD], w.HANDLE),
                  "GetProcessId": ([w.HANDLE], w.DWORD),
                  "GetProcessTimes": ([w.HANDLE] + [ct.POINTER(w.FILETIME)] * 4, w.BOOL),
                  "IsProcessInJob": ([w.HANDLE, w.HANDLE, ct.POINTER(w.BOOL)], w.BOOL),
                  "GetExitCodeProcess": ([w.HANDLE, ct.POINTER(w.DWORD)], w.BOOL),
                  "CreateJobObjectW": ([ct.c_void_p, w.LPCWSTR], w.HANDLE),
                  "SetInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD], w.BOOL),
                  "QueryInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD, ct.POINTER(w.DWORD)], w.BOOL),
                  "InitializeProcThreadAttributeList": ([ct.c_void_p, w.DWORD, w.DWORD, ct.POINTER(ct.c_size_t)], w.BOOL),
                  "UpdateProcThreadAttribute": ([ct.c_void_p, w.DWORD, ct.c_size_t, ct.c_void_p, ct.c_size_t, ct.c_void_p, ct.c_void_p], w.BOOL),
                  "DeleteProcThreadAttributeList": ([ct.c_void_p], None), "ResumeThread": ([w.HANDLE], w.DWORD)}
        for owner, bindings in ((self.api, functions), (self.parameters, parameters), (self.kernel, kernel)):
            for name, (arguments, result) in bindings.items():
                try:
                    function = getattr(owner, name)
                except AttributeError as exc:
                    raise ManagerError(f"USVFS bundle lacks the required export {name}.") from exc
                function.argtypes, function.restype = arguments, result

    def link_file(self, source, target):
        return call("vfs_link", self, source, target, False, False, False)

    def link_directory(self, source, target, *, create=False):
        return call("vfs_link", self, source, target, False, True, create)

    def start(self, argv, cwd, *, before_resume=None, environment=None):
        return call("vfs_start", self, argv, cwd, before_resume, environment, False)

    def running_processes(self):
        return call("vfs_running", self, False)

    def exit_code(self):
        return call("vfs_exit_code", self)

    def close(self):
        return call("vfs_close", self)
