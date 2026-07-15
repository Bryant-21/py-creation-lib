"""Shared Autodesk FBX SDK loader and DLL path setup."""

from __future__ import annotations

import os
import sys
from pathlib import Path

_CONFIGURED_SDK_ROOT: Path | None = None


def configure_fbx_sdk_root(path: str | Path | None) -> None:
    global _CONFIGURED_SDK_ROOT
    _CONFIGURED_SDK_ROOT = Path(path) if path else None


def _fbx_dll_dirs() -> list[Path]:
    dirs: list[Path] = []
    exe_dir = Path(sys.executable).resolve().parent
    meipass = Path(getattr(sys, "_MEIPASS", exe_dir))

    for candidate in (
        exe_dir,
        meipass,
        meipass.parent,
    ):
        if candidate.exists():
            dirs.append(candidate)

    if _CONFIGURED_SDK_ROOT is not None:
        candidate = _CONFIGURED_SDK_ROOT / "lib" / "x64" / "release"
        if candidate.exists():
            dirs.append(candidate)

    installed_sdk = Path(
        r"C:\Program Files\Autodesk\FBX\FBX SDK\2020.3.9\lib\x64\release"
    )
    if installed_sdk.exists():
        dirs.append(installed_sdk)

    unique_dirs: list[Path] = []
    seen: set[Path] = set()
    for candidate in dirs:
        resolved = candidate.resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        unique_dirs.append(resolved)
    return unique_dirs


_FBX_DLL_HANDLES = []
if hasattr(os, "add_dll_directory"):
    for _dll_dir in _fbx_dll_dirs():
        try:
            _FBX_DLL_HANDLES.append(os.add_dll_directory(str(_dll_dir)))
        except OSError:
            continue

def _load_fbx_sdk():
    """Import the Autodesk FBX SDK binding (`fbx.*.pyd`).

    A bare `import fbx` can resolve to this package (`fbx/__init__.py`) instead
    of the SDK when the package source is on `sys.path`. To avoid that shadowing
    we scan `sys.path` for the `.pyd` binding explicitly and fall back to a plain
    `import fbx` when the scan misses.
    """
    import importlib
    import importlib.util

    # Prefer an explicit search that skips `py_creation_lib/python/creation_lib/fbx/` (the shadowing dir).
    lib_fbx_dir = str(Path(__file__).resolve().parent)
    for entry in sys.path:
        if not entry:
            continue
        try:
            entry_resolved = str(Path(entry).resolve())
        except OSError:
            continue
        if entry_resolved == lib_fbx_dir:
            continue
        entry_path = Path(entry)
        if not entry_path.is_dir():
            continue
        for candidate in entry_path.iterdir():
            name = candidate.name.lower()
            if name == "fbx" or name.startswith("fbx."):
                if candidate.suffix.lower() == ".pyd" or (
                    candidate.is_dir() and candidate.name.lower() == "fbx"
                ):
                    spec = importlib.util.spec_from_file_location(
                        "fbx", str(candidate)
                    )
                    if spec and spec.loader:
                        try:
                            module = importlib.util.module_from_spec(spec)
                            sys.modules["fbx"] = module
                            spec.loader.exec_module(module)
                            if hasattr(module, "FbxManager"):
                                return module
                        except Exception:
                            sys.modules.pop("fbx", None)
                            continue

    # Fallback: plain import (may hit a shadow but at least doesn't crash).
    try:
        mod = importlib.import_module("fbx")
        if hasattr(mod, "FbxManager"):
            return mod
    except ImportError:
        pass
    return None


_sdk = _load_fbx_sdk()
if _sdk is not None:
    fbx = _sdk
    HAS_FBX = True
else:
    fbx = None
    HAS_FBX = False


__all__ = ["fbx", "HAS_FBX", "configure_fbx_sdk_root"]
