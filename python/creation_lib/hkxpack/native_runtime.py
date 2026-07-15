"""Thin Python boundary for native havok_native hkxpack entrypoints."""
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
    return callable(getattr(module, "hkx_load_to_json", None))


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


def native_available() -> bool:
    return load_native_module() is not None


def _require_native() -> Any:
    module = load_native_module()
    if module is None:
        raise RuntimeError("havok_native is not available")
    return module


# ---------------------------------------------------------------------------
# HKX load/save
# ---------------------------------------------------------------------------

def hkx_load_to_json(data: bytes) -> dict:
    """Parse any HKX bytes and return {"format": "tagxml", "content": "<xml>"}."""
    raw = _require_native().hkx_load_to_json(bytes(data))
    return json.loads(raw)


def hkx_save_from_json(payload: dict) -> bytes:
    """Accept a hkx_load_to_json payload dict and return HKX packfile bytes."""
    return bytes(_require_native().hkx_save_from_json(json.dumps(payload)))


def hkx_to_xml(data: bytes) -> str:
    """Parse HKX bytes and return TagXML string."""
    return str(_require_native().hkx_to_xml(bytes(data)))


def xml_to_hkx(xml_str: str) -> bytes:
    """Pack a TagXML string to HKX packfile bytes."""
    return bytes(_require_native().xml_to_hkx(str(xml_str)))


def hkx_detect_format(data: bytes) -> tuple[str, str] | None:
    """Detect HKX format. Returns (kind, version) or None."""
    native = _require_native()
    result = native.hkx_detect_format(bytes(data))
    if result is None:
        return None
    kind, version = result
    return (str(kind), str(version))


# ---------------------------------------------------------------------------
# Descriptor registry
# ---------------------------------------------------------------------------

def descriptor_registry_get(class_name: str, version: str | None = None) -> dict | None:
    """Return a ClassDescriptor-like dict for the given class, or None."""
    native = _require_native()
    fn = getattr(native, "descriptor_registry_get", None)
    if not callable(fn):
        raise RuntimeError("descriptor_registry_get not found in havok_native")
    raw = fn(str(class_name)) if version is None else fn(str(class_name), str(version))
    if raw is None:
        return None
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def descriptor_registry_get_all_members(class_name: str, version: str | None = None) -> list[dict]:
    """Return all members (including inherited) for the given class."""
    native = _require_native()
    fn = getattr(native, "descriptor_registry_get_all_members", None)
    if not callable(fn):
        raise RuntimeError("descriptor_registry_get_all_members not found in havok_native")
    raw = fn(str(class_name)) if version is None else fn(str(class_name), str(version))
    return json.loads(raw) if isinstance(raw, str) else list(raw)


def descriptor_registry_get_enum_value(
    class_name: str, enum_name: str, int_value: int, version: str | None = None
) -> str:
    """Resolve an integer enum value to its string name."""
    native = _require_native()
    fn = getattr(native, "descriptor_registry_get_enum_value", None)
    if not callable(fn):
        raise RuntimeError("descriptor_registry_get_enum_value not found in havok_native")
    args = (str(class_name), str(enum_name), int(int_value))
    if version is not None:
        args = args + (str(version),)
    return str(fn(*args))


def descriptor_registry_get_enum_int(
    class_name: str, enum_name: str, str_value: str, version: str | None = None
) -> int:
    """Resolve a string enum name to its integer value."""
    native = _require_native()
    fn = getattr(native, "descriptor_registry_get_enum_int", None)
    if not callable(fn):
        raise RuntimeError("descriptor_registry_get_enum_int not found in havok_native")
    args = (str(class_name), str(enum_name), str(str_value))
    if version is not None:
        args = args + (str(version),)
    return int(fn(*args))


# ---------------------------------------------------------------------------
# Spline compression
# ---------------------------------------------------------------------------

def havok_compress_spline(frames_json: str, duration: float, fps: float) -> bytes:
    """Compress per-frame transform frames to spline-compressed binary blob."""
    return bytes(_require_native().havok_compress_spline(str(frames_json), float(duration), float(fps)))
