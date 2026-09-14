"""Thin Python boundary for native nif_core_native entrypoints."""

from __future__ import annotations

from importlib import import_module
import threading
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_IMPORT_LOCK = threading.Lock()


def _looks_like_native_module(module: Any | None) -> bool:
    if module is None:
        return False
    return callable(getattr(module, "load_nif", None))


def _load_umbrella_submodule() -> Any | None:
    try:
        umbrella = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(umbrella, "nif_core_native", None)
    if _looks_like_native_module(module):
        return module
    try:
        extension = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(extension, "nif_core_native", None)
    return module if _looks_like_native_module(module) else None


def load_native_module() -> Any | None:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        return _NATIVE_MODULE
    with _NATIVE_IMPORT_LOCK:
        if _NATIVE_IMPORT_ATTEMPTED:
            return _NATIVE_MODULE
        try:
            mod = import_module("nif_core_native")
            if not _looks_like_native_module(mod):
                mod = import_module("nif_core_native.nif_core_native")
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


def load_nif_raw(path: str) -> Any:
    """Load a NIF file using the native Rust reader. Returns a dict payload."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    return module.load_nif(path)


def nif_from_bytes_raw(data: bytes) -> Any:
    """Parse NIF bytes using the native Rust reader. Returns a dict payload."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    nif_from_bytes = getattr(module, "nif_from_bytes", None)
    if not callable(nif_from_bytes):
        raise RuntimeError("nif_core_native.nif_from_bytes is not available")
    return nif_from_bytes(data)


def nif_to_bytes_raw(payload: dict[str, Any]) -> bytes:
    """Serialize a native NIF dict payload using the Rust writer."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    nif_to_bytes = getattr(module, "nif_to_bytes", None)
    if not callable(nif_to_bytes):
        raise RuntimeError("nif_core_native.nif_to_bytes is not available")
    return bytes(nif_to_bytes(payload))


def new_nif_raw(game: str) -> Any:
    """Create a new NIF payload using the native Rust model."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    new_nif = getattr(module, "new_nif", None)
    if not callable(new_nif):
        raise RuntimeError("nif_core_native.new_nif is not available")
    return new_nif(game)


def save_nif_raw(payload: dict[str, Any], path: str) -> None:
    """Save a native NIF dict payload using the Rust writer."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    save_nif = getattr(module, "save_nif", None)
    if not callable(save_nif):
        raise RuntimeError("nif_core_native.save_nif is not available")
    save_nif(payload, path)


def convert_nif_file_raw(
    src: str,
    dst: str,
    source_game: str,
    target_game: str,
    bgsm_output_dir: str | None = None,
    options: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Convert a NIF file directly in the native runtime."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    convert_nif_file = getattr(module, "convert_nif_file", None)
    if not callable(convert_nif_file):
        raise RuntimeError("nif_core_native.convert_nif_file is not available")
    return convert_nif_file(
        src,
        dst,
        source_game,
        target_game,
        bgsm_output_dir,
        options or {},
    )


def validate_nif_file_raw(
    path: str,
    output_path: str | None = None,
    fix: bool = False,
    include_optional: bool = False,
) -> dict[str, Any]:
    """Audit a NIF and optionally apply safe fixes selected by its header."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    validate_nif_file = getattr(module, "validate_nif_file", None)
    if not callable(validate_nif_file):
        raise RuntimeError("nif_core_native.validate_nif_file is not available")
    return validate_nif_file(path, output_path, fix, include_optional)


def nif_features_raw() -> dict[str, Any]:
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    nif_features = getattr(module, "nif_features", None)
    if not callable(nif_features):
        raise RuntimeError("nif_core_native.nif_features is not available")
    return nif_features()


def nif_report_raw(
    path: str,
    processor: str,
    options: dict[str, Any] | None = None,
) -> dict[str, Any]:
    import json

    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    nif_report = getattr(module, "nif_report", None)
    if not callable(nif_report):
        raise RuntimeError("nif_core_native.nif_report is not available")
    return json.loads(nif_report(path, processor, json.dumps(options or {})))


def nif_process_raw(
    path: str,
    output_path: str,
    processor: str,
    options: dict[str, Any] | None = None,
) -> dict[str, Any]:
    import json

    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    nif_process = getattr(module, "nif_process", None)
    if not callable(nif_process):
        raise RuntimeError("nif_core_native.nif_process is not available")
    return json.loads(
        nif_process(path, output_path, processor, json.dumps(options or {}))
    )


def weapon_block_diff_raw(base_path: str, mod_path: str) -> list[int]:
    """Return block IDs present in the sibling NIF but absent from the base NIF."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    weapon_block_diff = getattr(module, "weapon_block_diff", None)
    if not callable(weapon_block_diff):
        raise RuntimeError("nif_core_native.weapon_block_diff is not available")
    return list(weapon_block_diff(base_path, mod_path))


def extract_attachment_raw(
    base_path: str,
    sibling_path: str,
    slot: int,
    output_attachment_path: str,
    anchor_node_name: str,
) -> dict[str, Any]:
    """Extract a slot attachment and patch the base NIF with connect points."""
    module = load_native_module()
    if module is None:
        raise RuntimeError("nif_core_native is not available")
    extract_attachment = getattr(module, "extract_attachment", None)
    if not callable(extract_attachment):
        raise RuntimeError("nif_core_native.extract_attachment is not available")
    return extract_attachment(
        base_path,
        sibling_path,
        int(slot),
        output_attachment_path,
        anchor_node_name,
    )
