"""Thin Python boundary for native bsarchive_native entrypoints."""

from __future__ import annotations

from importlib import import_module
from threading import Lock
from typing import Any, Callable, Literal

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_LOAD_LOCK = Lock()
BackendName = Literal["auto", "native"]
_NATIVE_CAPABILITIES = (
    "list_archive",
    "archive_info",
    "archive_entry_count",
    "extract_one",
    "extract_archive",
    "pack_archive",
    "pack_archive_entries",
    "pack_archive_plans",
    "pack_mod_archives",
    "plan_archives",
)
_TEXTURE_ARCHIVE_TYPES = frozenset(
    {
        "fo76dds",
        "fo4ogdds",
        "fo4dds",
        "fo4xboxdds",
        "fo4psdds",
        "starfielddds",
        "sfdds",
    }
)


def _looks_like_native_module(module: Any | None) -> bool:
    if module is None:
        return False
    return any(callable(getattr(module, name, None)) for name in _NATIVE_CAPABILITIES)


def _load_umbrella_submodule() -> Any:
    umbrella = import_module("creation_lib._native")
    native_module = getattr(umbrella, "bsarchive_native", None)
    if _looks_like_native_module(native_module):
        return native_module
    # Fallback: umbrella may be a namespace package; try loading the .pyd directly.
    try:
        pyd = import_module("creation_lib._native")
        native_module = getattr(pyd, "bsarchive_native", None)
        if _looks_like_native_module(native_module):
            return native_module
    except ImportError:
        pass
    raise ImportError("creation_lib._native.bsarchive_native is missing")


def load_native_module() -> Any:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    with _NATIVE_LOAD_LOCK:
        if _NATIVE_IMPORT_ATTEMPTED:
            if _NATIVE_MODULE is None:
                raise RuntimeError("bsarchive_native is required for archive operations")
            return _NATIVE_MODULE
        _NATIVE_IMPORT_ATTEMPTED = True
        try:
            _NATIVE_MODULE = import_module("bsarchive_native")
            if not _looks_like_native_module(_NATIVE_MODULE):
                _NATIVE_MODULE = import_module("bsarchive_native.bsarchive_native")
        except ImportError:
            try:
                _NATIVE_MODULE = _load_umbrella_submodule()
            except ImportError as umbrella_exc:
                raise RuntimeError("bsarchive_native is required for archive operations") from umbrella_exc
        if not _looks_like_native_module(_NATIVE_MODULE):
            try:
                _NATIVE_MODULE = _load_umbrella_submodule()
            except ImportError:
                pass
        if not _looks_like_native_module(_NATIVE_MODULE):
            raise RuntimeError("bsarchive_native is required for archive operations")
        return _NATIVE_MODULE


def _require_native_function(name: str) -> Any:
    global _NATIVE_MODULE
    module = load_native_module()
    native_fn = getattr(module, name, None)
    if callable(native_fn):
        return native_fn
    with _NATIVE_LOAD_LOCK:
        native_fn = getattr(_NATIVE_MODULE, name, None)
        if callable(native_fn):
            return native_fn
        try:
            refreshed_module = _load_umbrella_submodule()
        except ImportError:
            refreshed_module = None
        refreshed_fn = getattr(refreshed_module, name, None)
        if callable(refreshed_fn):
            _NATIVE_MODULE = refreshed_module
            return refreshed_fn
    raise NotImplementedError(f"bsarchive_native does not provide {name}() yet")


def normalize_backend(backend: str | None) -> BackendName:
    if backend is None:
        return "auto"
    value = str(backend).strip().lower()
    if value not in {"auto", "native"}:
        raise ValueError(f"Unsupported archive backend: {backend!r}")
    return value  # type: ignore[return-value]


def native_function_available(name: str) -> bool:
    return getattr(load_native_module(), name, None) is not None


def should_use_native_backend(backend: str | None, capability: str) -> bool:
    _ = normalize_backend(backend)
    if native_function_available(capability):
        return True
    _require_native_function(capability)
    return False


# --- read path -------------------------------------------------------------


def list_archive(path: str) -> list[str] | None:
    return list(_require_native_function("list_archive")(path))


def archive_info(path: str) -> dict | None:
    return _require_native_function("archive_info")(path)


def archive_entry_count(path: str) -> int:
    return int(_require_native_function("archive_entry_count")(path))


def extract_one(archive: str, file_path: str) -> bytes | None:
    return _require_native_function("extract_one")(archive, file_path)


def extract_archive(
    archive: str,
    output_dir: str,
    *,
    format: str | None = None,
    workers: int = 0,
    progress: Callable[[dict], bool | None] | None = None,
) -> int | None:
    return _require_native_function("extract_archive")(archive, output_dir, format, workers, progress)


def pack_archive(
    source_dir: str,
    output_path: str,
    archive_type: str,
    *,
    compress: bool = True,
    compression_level: int | None = None,
    share_data: bool = False,
    manifest_path: str | None = None,
    jobs: int = 0,
    include_prefixes: list[str] | None = None,
    exclude_prefixes: list[str] | None = None,
) -> int | None:
    native_fn = _require_native_function("pack_archive")
    kwargs = {
        "compress": compress,
        "compression_level": compression_level,
        "share_data": share_data,
        "manifest_path": manifest_path,
        "jobs": jobs,
    }
    if include_prefixes is not None:
        kwargs["include_prefixes"] = include_prefixes
    if exclude_prefixes is not None:
        kwargs["exclude_prefixes"] = exclude_prefixes
    return native_fn(
        source_dir,
        output_path,
        archive_type,
        **kwargs,
    )


def pack_archive_entries(
    entries: list[tuple[str, str]],
    output_path: str,
    archive_type: str,
    *,
    texture_archive: bool = False,
    compress: bool = True,
    compression_level: int | None = None,
    share_data: bool = False,
    manifest_path: str | None = None,
    jobs: int = 0,
) -> int | None:
    if texture_archive and archive_type.strip().lower() not in _TEXTURE_ARCHIVE_TYPES:
        raise ValueError(
            "texture_archive=True requires a texture archive type: "
            + ", ".join(sorted(_TEXTURE_ARCHIVE_TYPES))
        )
    native_fn = _require_native_function("pack_archive_entries")
    return native_fn(
        entries,
        output_path,
        archive_type,
        compress=compress,
        compression_level=compression_level,
        share_data=share_data,
        manifest_path=manifest_path,
        jobs=jobs,
    )


def pack_archive_plans(
    plans: list[tuple[str, str, bool, list[tuple[str, str, int]]]],
    *,
    total_workers: int = 0,
    progress: Callable[[dict], bool | None] | None = None,
) -> int:
    native_fn = _require_native_function("pack_archive_plans")
    return native_fn(plans, total_workers=total_workers, progress=progress)


def pack_mod_archives(config: dict, progress: Callable[[dict], bool | None] | None = None) -> dict:
    native_fn = _require_native_function("pack_mod_archives")
    return native_fn(config, progress)


def plan_archives(
    mod_name: str,
    entries: list[tuple[str, str, int]],
    archive_ext: str,
    platform_suffix: str = "",
    archive_max_bytes: int = 16 * 1024**3,
    game: str | None = None,
    expanded_archives: bool = False,
) -> list[dict]:
    native_fn = _require_native_function("plan_archives")
    return native_fn(
        mod_name,
        entries,
        archive_ext,
        platform_suffix,
        archive_max_bytes,
        game,
        expanded_archives,
    )
