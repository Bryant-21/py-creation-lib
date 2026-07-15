"""Backend-aware public ESP API wrappers."""

from __future__ import annotations

import json
from pathlib import Path
import re
from typing import Any, Mapping

from . import native_runtime as _native_runtime
from .plugin import Plugin


def _safe_plugin_name(plugin_name: str | None) -> str:
    candidate = Path(plugin_name or "Plugin.esp").name
    return candidate or "Plugin.esp"


def _restore_imported_plugin_identity(plugin: Plugin, *, plugin_name: str, game: str | None) -> Plugin:
    handle = getattr(plugin, "_rust_handle", None)
    if handle is not None:
        try:
            _native_runtime.plugin_handle_call(handle, "set_logical_identity", plugin_name, game=game, file_path=None)
        except RuntimeError:
            pass
    plugin.file_path = None
    plugin._plugin_name = plugin_name
    if game is not None:
        plugin.game = game
    return plugin



def _load_authoring_dir_metadata(path: str | Path) -> tuple[str, str | None]:
    authoring_dir = Path(path)
    for candidate in (authoring_dir / "plugin.json", authoring_dir / "plugin.yaml"):
        if not candidate.is_file():
            continue
        text = candidate.read_text(encoding="utf-8")
        if candidate.suffix.lower() == ".json":
            pm = re.search(r'"plugin"\s*:\s*"([^"]*)"', text[:512])
            gm = re.search(r'"game"\s*:\s*"([^"]*)"', text[:512])
        else:
            pm = re.search(r"^plugin:\s+(\S+)", text[:512], re.MULTILINE)
            gm = re.search(r"^game:\s+(\S+)", text[:512], re.MULTILINE)
        plugin_name = pm.group(1) if pm else None
        game = gm.group(1) if gm else None
        if plugin_name:
            return plugin_name, game
    return "Plugin.esp", None


def _require_no_progress(backend: str | None, operation: str, progress: Any) -> bool:
    if progress is None:
        return False
    _native_runtime.normalize_backend(backend)
    raise NotImplementedError(f"{operation} does not support progress callbacks in the native backend yet")


def _require_default_strings(backend: str | None, operation: str, strings: Mapping[int, str] | None) -> bool:
    if strings is None:
        return False
    _native_runtime.normalize_backend(backend)
    raise NotImplementedError(f"{operation} does not support explicit strings mappings in the native backend yet")


def export_data(
    plugin: Plugin,
    *,
    mode: str = "lossless",
    strings: Mapping[int, str] | None = None,
    backend: str = "auto",
) -> dict[str, Any]:
    _require_default_strings(backend, "export_data()", strings)
    _native_runtime.should_use_native_backend(backend, "export_plugin_text_native")
    return json.loads(export_json(plugin, mode=mode, backend=backend))


def export_json(
    plugin: Plugin,
    *,
    mode: str = "lossless",
    strings: Mapping[int, str] | None = None,
    indent: int = 2,
    backend: str = "auto",
) -> str:
    _require_default_strings(backend, "export_json()", strings)
    if indent != 2:
        _native_runtime.normalize_backend(backend)
        raise NotImplementedError("export_json() only supports the default indent in the native backend today")
    _native_runtime.should_use_native_backend(backend, "export_plugin_text_native")
    handle = getattr(plugin, "_rust_handle", None)
    if handle is None:
        raise RuntimeError("export_json() requires a handle-backed plugin")
    text = str(_native_runtime.plugin_handle_call(handle, "export_plugin_text", mode, "json"))
    if mode.lower() == "authoring":
        plugin._localized_strings_loaded = False
    return text


def export_yaml(
    plugin: Plugin,
    *,
    mode: str = "lossless",
    strings: Mapping[int, str] | None = None,
    backend: str = "auto",
) -> str:
    _require_default_strings(backend, "export_yaml()", strings)
    _native_runtime.should_use_native_backend(backend, "export_plugin_text_native")
    handle = getattr(plugin, "_rust_handle", None)
    if handle is None:
        raise RuntimeError("export_yaml() requires a handle-backed plugin")
    text = str(_native_runtime.plugin_handle_call(handle, "export_plugin_text", mode, "yaml"))
    if mode.lower() == "authoring":
        plugin._localized_strings_loaded = False
    return text


def export_authoring_dir(
    plugin: Plugin,
    path: str | Path,
    *,
    jobs: int | None = None,
    format: str = "json",
    progress: Any = None,
    backend: str = "auto",
) -> None:
    _require_no_progress(backend, "export_authoring_dir()", progress)
    _native_runtime.should_use_native_backend(backend, "export_authoring_dir_native")
    handle = getattr(plugin, "_rust_handle", None)
    if handle is None:
        raise RuntimeError("export_authoring_dir() requires a handle-backed plugin")
    _native_runtime.plugin_handle_call(
        handle,
        "export_authoring_dir",
        str(Path(path)),
        format=format,
        jobs=jobs,
    )
    plugin._localized_strings_loaded = False


def import_data(data: Mapping[str, Any], *, backend: str = "auto") -> Plugin:
    _native_runtime.should_use_native_backend(backend, "import_plugin_text_native")
    return import_json(json.dumps(data), backend=backend)


def import_json(text: str, *, backend: str = "auto") -> Plugin:
    _native_runtime.should_use_native_backend(backend, "import_plugin_text_native")
    handle = _native_runtime.plugin_handle_import_text(text, "json")
    plugin = Plugin._from_native_handle(handle)
    return _restore_imported_plugin_identity(plugin, plugin_name=plugin._plugin_name, game=plugin.game)


def import_yaml(text: str, *, backend: str = "auto") -> Plugin:
    _native_runtime.should_use_native_backend(backend, "import_plugin_text_native")
    handle = _native_runtime.plugin_handle_import_text(text, "yaml")
    plugin = Plugin._from_native_handle(handle)
    return _restore_imported_plugin_identity(plugin, plugin_name=plugin._plugin_name, game=plugin.game)


def build_authoring_dir(
    source_dir: str | Path,
    output_path: str | Path,
    *,
    game: str | None = None,
    jobs: int | None = None,
    master_esm_paths: list[str] | None = None,
) -> None:
    """Stream-build a .esp directly from a YAML/JSON authoring dir.

    Records are read, encoded, and written one at a time -- the full plugin
    tree is never materialized in RAM. Peak memory scales with `jobs` × the
    largest record's parsed JsonValue, not with plugin size.

    Args:
        source_dir: Authoring dir containing plugin.{yaml,json} + records/
        output_path: Target .esp path
        game: Override the game ID stored in the manifest
        jobs: Parallel decode thread count.
            * None (default): global rayon pool (= num_cpus). Fastest;
              Starfield-scale plugins peak around 8-10 GB RSS.
            * 1: serial decode. Lowest memory (~1 GB peak on Starfield)
              but ~30 min wall-clock for 1.4 GB plugin.
            * 4: middle ground (~3 GB peak, ~12 min on Starfield).
        master_esm_paths: Full filesystem paths to the plugin's masters.
            The first readable master is scanned to derive the engine's
            canonical top-level GRUP order — required to keep KYWD before
            COBJ etc. so CK doesn't report `[FORMS] Unable to find keyword`.
            When None or unreadable, falls back to a hardcoded baseline.
            CLI commands resolve env-based game data paths and pass them
            here; lib code stays env-free.

    `game` defaults to whatever's recorded in the authoring dir's manifest.
    """
    _, manifest_game = _load_authoring_dir_metadata(source_dir)
    resolved_game = game or manifest_game
    _native_runtime.build_authoring_dir_streaming_native(
        str(Path(source_dir)),
        str(Path(output_path)),
        game=resolved_game,
        jobs=jobs,
        master_esm_paths=master_esm_paths,
    )
