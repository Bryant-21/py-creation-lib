"""Per-download process locks, allowing independent owned transfers to overlap."""
from .backend import call, NativeContext


def job_lock(library, identifier):
    return NativeContext(call("download_library_lock", library, identifier))
