"""Thin Python boundary for native havok_native conversion entrypoints."""
from __future__ import annotations

import json
from importlib import import_module
import threading
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_LOAD_LOCK = threading.Lock()


def _looks_like_native_module(module: Any | None) -> bool:
    if module is None:
        return False
    return callable(getattr(module, "havok_convert_bytes", None))


def _load_umbrella_submodule() -> Any | None:
    from creation_lib.havok.native_runtime import _configure_native_resources

    _configure_native_resources()
    try:
        umbrella = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(umbrella, "havok_native", None)
    if _looks_like_native_module(module):
        return module
    try:
        extension = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(extension, "havok_native", None)
    return module if _looks_like_native_module(module) else None


def load_native_module() -> Any | None:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        return _NATIVE_MODULE
    with _NATIVE_LOAD_LOCK:
        if _NATIVE_IMPORT_ATTEMPTED:
            return _NATIVE_MODULE
        try:
            from creation_lib.havok.native_runtime import _configure_native_resources

            _configure_native_resources()
            mod = import_module("havok_native")
            if not _looks_like_native_module(mod):
                mod = import_module("havok_native.havok_native")
            if _looks_like_native_module(mod):
                _NATIVE_MODULE = mod
        except ImportError:
            _NATIVE_MODULE = _load_umbrella_submodule()
        if _NATIVE_MODULE is None:
            _NATIVE_MODULE = _load_umbrella_submodule()
        _NATIVE_IMPORT_ATTEMPTED = True
        return _NATIVE_MODULE


def _require_native() -> Any:
    module = load_native_module()
    if module is None:
        raise RuntimeError("havok_native is not available")
    return module


def havok_convert_bytes_native(data: bytes, target_version: int | str) -> bytes:
    """Convert an HKX blob in memory via the native Rust implementation."""
    if not isinstance(data, (bytes, bytearray)):
        raise TypeError(f"data must be bytes, got {type(data).__name__}")
    return bytes(_require_native().havok_convert_bytes(bytes(data), str(target_version)))


def havok_convert_file_native(src: str, dst: str, target_version: int | str) -> None:
    """Convert an HKX file on disk via the native Rust implementation."""
    _require_native().havok_convert_file(str(src), str(dst), str(target_version))


def havok_convert_batch_native(
    input_dir: str,
    output_dir: str,
    target_version: int | str,
    parallel: bool = True,
) -> dict:
    """Convert all HKX files in a directory via the native Rust implementation.

    Returns a dict with keys: converted, skipped, errors.
    ``parallel`` is accepted for API compatibility but Rust always uses rayon.
    """
    raw = _require_native().havok_convert_batch(
        str(input_dir), str(output_dir), str(target_version), True
    )
    return json.loads(raw) if isinstance(raw, str) else dict(raw)

