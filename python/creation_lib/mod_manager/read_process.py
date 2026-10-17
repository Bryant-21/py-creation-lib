import ctypes as ct
from ctypes import wintypes as w

from .backend import call
from .native_runtime import ProcessInfo, StartupInfo


class Accounting(ct.Structure):
    _fields_ = [("times", ct.c_longlong * 4), ("faults", w.DWORD), ("total", w.DWORD),
                ("active", w.DWORD), ("terminated", w.DWORD)]


class ReadProcess:
    def __init__(self, argv, output, errors):
        call("reader_initialize", self, argv, output, errors)

    def _bind(self):
        self.kernel = ct.WinDLL("kernel32", use_last_error=True)
        bindings = {
            "CreateJobObjectW": ([ct.c_void_p, w.LPCWSTR], w.HANDLE),
            "SetInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD], w.BOOL),
            "QueryInformationJobObject": ([w.HANDLE, ct.c_int, ct.c_void_p, w.DWORD, ct.POINTER(w.DWORD)], w.BOOL),
            "InitializeProcThreadAttributeList": ([ct.c_void_p, w.DWORD, w.DWORD, ct.POINTER(ct.c_size_t)], w.BOOL),
            "UpdateProcThreadAttribute": ([ct.c_void_p, w.DWORD, ct.c_size_t, ct.c_void_p, ct.c_size_t, ct.c_void_p, ct.c_void_p], w.BOOL),
            "DeleteProcThreadAttributeList": ([ct.c_void_p], None),
            "CreateProcessW": ([w.LPCWSTR, w.LPWSTR, ct.c_void_p, ct.c_void_p, w.BOOL, w.DWORD, ct.c_void_p, w.LPCWSTR, ct.POINTER(StartupInfo), ct.POINTER(ProcessInfo)], w.BOOL),
            "GetCurrentProcess": ([], w.HANDLE),
            "DuplicateHandle": ([w.HANDLE, w.HANDLE, w.HANDLE, ct.POINTER(w.HANDLE), w.DWORD, w.BOOL, w.DWORD], w.BOOL),
            "GetExitCodeProcess": ([w.HANDLE, ct.POINTER(w.DWORD)], w.BOOL),
            "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
            "CloseHandle": ([w.HANDLE], w.BOOL),
        }
        for name, (arguments, result) in bindings.items():
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result

    def poll(self):
        return call("reader_poll", self)

    def close(self):
        return call("reader_close", self)
