from pathlib import Path
from app.paths import get_code_root
from .backend import NativeContext, call

SOURCE = get_code_root() / "py_creation_lib" / "native" / "mod_manager_vfs"
PAYLOAD_FILES = ("modbox_vfs.dll", "MinHook-LICENSE.txt", "Project-LICENSE.txt")


def build_lock(root, cancel):
    return NativeContext(call("native_build_lock", root, cancel))


def source_fingerprint():
    return call("native_source_fingerprint", SOURCE)


def validate(root, cancel=None):
    return Path(call("native_build_validate", root, cancel))


def _command(argv, cancel, progress):
    return call("native_build_command", argv, cancel, progress)


def ensure(cancel=None, progress=None, *, cache=None):
    return Path(call("native_build_ensure", SOURCE, Path(__file__).parent / "_runtime" / "project-vfs", cache, cancel, progress))
