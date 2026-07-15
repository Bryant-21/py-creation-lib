"""Native-backed plugin wrapper for TES4-family plugins."""

from __future__ import annotations

import struct
from pathlib import Path
from typing import Iterable, Mapping

from . import native_runtime as _native_runtime
from creation_lib.esp.model import (
    LOCAL_FORM_INDEX,
    FormRef,
    Group,
    PluginHeader,
    Record,
)
from creation_lib.esp.strings import (
    STRING_TABLE_EXTENSIONS,
    language_code,
    language_display_name,
    load_all_string_tables,
    load_string_tables,
    localized_table_type_for_signature,
    write_string_table,
)

_STRING_TABLE_SUFFIXES = (".strings", ".ilstrings", ".dlstrings")

def _strings_dir_contains_plugin(strings_dir: Path, plugin_stem: str) -> bool:
    prefix = f"{plugin_stem.lower()}_"
    if not strings_dir.exists():
        return False
    try:
        return any(
            item.is_file()
            and item.name.lower().startswith(prefix)
            and item.suffix.lower() in _STRING_TABLE_SUFFIXES
            for item in strings_dir.iterdir()
        )
    except OSError:
        return False


def _strings_dir_for_plugin(strings_dirs: Iterable[str | Path], plugin_stem: str) -> Path | None:
    fallback: Path | None = None
    for raw_candidate in strings_dirs:
        candidate = Path(raw_candidate)
        if fallback is None:
            fallback = candidate
        if _strings_dir_contains_plugin(candidate, plugin_stem):
            return candidate
    return fallback


def replace_plugin_with_localized_sidecars(
    temp_plugin_path: str | Path,
    target_plugin_path: str | Path,
) -> None:
    temp = Path(temp_plugin_path)
    target = Path(target_plugin_path)
    strings_dir = target.parent / "Strings"
    if strings_dir.is_dir():
        temp_prefix = f"{temp.stem}_"
        temp_prefix_key = temp_prefix.lower()
        for sidecar in sorted(strings_dir.iterdir(), key=lambda path: path.name.lower()):
            if (
                not sidecar.is_file()
                or sidecar.suffix.lower() not in _STRING_TABLE_SUFFIXES
                or not sidecar.name.lower().startswith(temp_prefix_key)
            ):
                continue
            final_name = f"{target.stem}_{sidecar.name[len(temp_prefix):]}"
            sidecar.replace(strings_dir / final_name)
    temp.replace(target)

LEGACY_HEADER_SIZE = 20
MODERN_HEADER_SIZE = 24
DEFAULT_HEDR_VERSION = {
    "oblivion": 0.8,
    "fo3": 0.94,
    "fnv": 0.94,
}

# Canonical copies live in py_creation_lib/native/esp/src/codec_constants.rs; keep in sync.
KNOWN_FORMID_SUBRECORDS: frozenset[str] = frozenset({
    "ANAM", "ATKR", "CNAM", "ECOR", "EFID", "EITM", "ETYP", "FTSF", "FTSM", "INAM", "LNAM", "PNAM",
    "RNAM", "SNAM", "SOFT", "SPLO", "TNAM", "VNAM", "VTCK", "WNAM", "YNAM", "ZNAM",
})
KNOWN_FORMID_ARRAY_SUBRECORDS: frozenset[str] = frozenset({"KWDA", "MODS", "ONAM", "SPOR"})


_MISSING_HANDLE_METHOD = object()


def _try_handle_call(handle: object, name: str, *args, **kwargs):
    try:
        return _native_runtime.plugin_handle_call(handle, name, *args, **kwargs)
    except RuntimeError:
        return _MISSING_HANDLE_METHOD


class _PluginHeaderView:
    """Live proxy for plugin header fields, backed by handle or _header."""

    __slots__ = ("_plugin",)

    def __init__(self, plugin: "Plugin") -> None:
        self._plugin = plugin

    def _handle(self):
        return getattr(self._plugin, "_rust_handle", None)

    @property
    def masters(self) -> list[str]:
        h = self._handle()
        if h is not None:
            return list(_native_runtime.plugin_handle_get(h, "masters", []))
        hdr = self._plugin._header
        return hdr.masters if hdr is not None else []

    @masters.setter
    def masters(self, value: list[str]) -> None:
        h = self._handle()
        if h is not None:
            raise AttributeError("masters cannot be set directly on a handle-backed plugin; use add_master()")
        hdr = self._plugin._header
        if hdr is not None:
            hdr.masters = value

    @property
    def master_sizes(self) -> list[int]:
        h = self._handle()
        if h is not None:
            return list(_native_runtime.plugin_handle_get(h, "master_sizes", []))
        hdr = self._plugin._header
        return hdr.master_sizes if hdr is not None else []

    @master_sizes.setter
    def master_sizes(self, value: list[int]) -> None:
        h = self._handle()
        if h is not None:
            raise AttributeError("master_sizes cannot be set directly on a handle-backed plugin")
        hdr = self._plugin._header
        if hdr is not None:
            hdr.master_sizes = value

    @property
    def is_localized(self) -> bool:
        h = self._handle()
        if h is not None:
            return bool(_native_runtime.plugin_handle_get(h, "is_localized", False))
        hdr = self._plugin._header
        return bool(hdr.is_localized) if hdr is not None else False

    @is_localized.setter
    def is_localized(self, value: bool) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_is_localized", bool(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.is_localized = value

    @property
    def version(self) -> float:
        h = self._handle()
        if h is not None:
            return float(_native_runtime.plugin_handle_get(h, "header_version", 0.0))
        hdr = self._plugin._header
        return float(hdr.version) if hdr is not None else 0.0

    @version.setter
    def version(self, value: float) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_header_version", float(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.version = value

    @property
    def next_object_id(self) -> int:
        h = self._handle()
        if h is not None:
            return int(_native_runtime.plugin_handle_get(h, "next_object_id", 0x800))
        hdr = self._plugin._header
        return int(hdr.next_object_id) if hdr is not None else 0x800

    @next_object_id.setter
    def next_object_id(self, value: int) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_header_next_object_id", int(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.next_object_id = int(value)

    @property
    def author(self) -> str:
        h = self._handle()
        if h is not None:
            return _native_runtime.plugin_handle_get(h, "header_author", "")
        hdr = self._plugin._header
        return hdr.author if hdr is not None else ""

    @author.setter
    def author(self, value: str) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_header_author", str(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.author = value

    @property
    def description(self) -> str:
        h = self._handle()
        if h is not None:
            return _native_runtime.plugin_handle_get(h, "header_description", "")
        hdr = self._plugin._header
        return hdr.description if hdr is not None else ""

    @description.setter
    def description(self, value: str) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_header_description", str(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.description = value

    @property
    def flags(self) -> int:
        h = self._handle()
        if h is not None:
            return _native_runtime.plugin_handle_get(h, "header_flags", 0)
        hdr = self._plugin._header
        return hdr.flags if hdr is not None else 0

    @flags.setter
    def flags(self, value: int) -> None:
        h = self._handle()
        if h is not None:
            _native_runtime.plugin_handle_call(h, "set_header_flags", int(value))
            return
        hdr = self._plugin._header
        if hdr is not None:
            hdr.flags = int(value)

    def __getattr__(self, name: str):
        if name == "_plugin":
            raise AttributeError(name)
        h = self._handle()
        if h is None:
            hdr = self._plugin._header
            if hdr is not None:
                return getattr(hdr, name)
        if h is not None:
            value = _native_runtime.plugin_handle_get(h, name, None)
            if value is not None:
                return value
        raise AttributeError(name)

    def __setattr__(self, name: str, value) -> None:
        if name in ("_plugin",):
            object.__setattr__(self, name, value)
            return
        h = self._handle()
        if h is not None:
            try:
                _native_runtime.plugin_handle_call(h, f"set_header_{name}", value)
                return
            except RuntimeError:
                pass
            raise AttributeError(f"Cannot set {name!r} on handle-backed header proxy")
        hdr = self._plugin._header
        if hdr is not None:
            setattr(hdr, name, value)
            return
        raise AttributeError(f"Cannot set {name!r} on header proxy with no backing header")


class Plugin:
    """Python-facing plugin wrapper backed by the Rust runtime."""

    def __init__(
        self,
        *,
        plugin_name: str,
        file_path: Path | None = None,
        game: str | None = None,
        # Accepted for compatibility with older materialized plugin constructors.
        # Prefer Plugin.load(), Plugin.new(), Plugin.from_bytes(), or Plugin._from_native_handle().
        header_size: int = MODERN_HEADER_SIZE,
        header: PluginHeader | None = None,
        root_items: list[Group | Record] | None = None,
    ) -> None:
        self._plugin_name = plugin_name
        self.file_path = file_path
        self.game = game
        self._localized_strings: dict[int, str] = {}
        self._localized_strings_by_language: dict[str, dict[int, str]] = {}
        self._localized_string_table_types: dict[int, str] = {}
        self._localized_strings_loaded = False
        self.localized_default_language = "en"
        self.load_order = -1
        self._rust_handle = None
        self._header: PluginHeader | None = header
        self._root_items: list[Group | Record] | None = root_items
        self._header_size: int = header_size
        self._header_view: _PluginHeaderView = _PluginHeaderView(self)

    @property
    def plugin_name(self) -> str:
        if self.file_path is not None:
            return self.file_path.name
        return self._plugin_name

    @classmethod
    def load(
        cls,
        path: str | Path,
        *,
        game: str | None = None,
        strings_dir: str | Path | None = None,
        strings_dirs: Iterable[str | Path] | None = None,
        language: str | None = None,
        jobs: int | None = None,
        backend: str = "auto",
        eager_compressed: bool = True,
        lazy_index: bool = False,
    ) -> "Plugin":
        """Load a plugin from disk through the native handle API.

        ``jobs`` is retained for API compatibility and ignored by the native
        backend.

        ``lazy_index=True`` requests an index-only handle: the parsed record
        tree is dropped after load, keeping only the formid/eid index and
        on-demand record parsing. Use it for read-only handles (e.g. target
        masters) to avoid the multi-GB resident tree. Falls back to a full load
        if the native backend lacks the index-only entry point.
        """
        file_path = Path(path)
        _native_runtime.should_use_native_backend(backend, "load_plugin_native")
        resolved_strings_dir = strings_dir
        if resolved_strings_dir is None:
            sibling_strings_dir = file_path.parent / "Strings"
            if _strings_dir_contains_plugin(sibling_strings_dir, file_path.stem):
                resolved_strings_dir = sibling_strings_dir
        if resolved_strings_dir is None and strings_dirs is not None:
            resolved_strings_dir = _strings_dir_for_plugin(strings_dirs, file_path.stem)
        resolved_strings_dir_str = (
            str(resolved_strings_dir) if resolved_strings_dir is not None else None
        )
        if lazy_index:
            handle = _native_runtime.plugin_handle_load_index(
                str(file_path),
                game=game,
                strings_dir=resolved_strings_dir_str,
                language=language,
            )
            if handle is not None:
                return cls._from_native_handle(handle)
        handle = _native_runtime.plugin_handle_load(
            str(file_path),
            game=game,
            strings_dir=resolved_strings_dir_str,
            language=language,
            eager_compressed=eager_compressed,
        )
        return cls._from_native_handle(handle)

    @classmethod
    def _from_native_handle(cls, handle: object) -> "Plugin":
        plugin = cls(
            plugin_name=str(_native_runtime.plugin_handle_get(handle, "plugin_name", "Plugin.esp")),
            file_path=(
                Path(str(_native_runtime.plugin_handle_get(handle, "file_path")))
                if _native_runtime.plugin_handle_get(handle, "file_path", None)
                else None
            ),
            game=_native_runtime.plugin_handle_get(handle, "game", None),
        )
        plugin._rust_handle = handle
        default_language = _native_runtime.plugin_handle_get(handle, "localized_default_language", None)
        if default_language:
            plugin.localized_default_language = language_code(str(default_language))
        return plugin

    def _ensure_localized_strings_loaded(self) -> None:
        if self._localized_strings_loaded:
            return
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            self._localized_strings_loaded = True
            return
        payload = _native_runtime.plugin_handle_get_strings(handle)
        strings_by_language = payload.get("localized_strings_by_language", None)
        if strings_by_language is not None:
            self._localized_strings_by_language = {
                str(language): {int(key): str(value) for key, value in dict(values).items()}
                for language, values in dict(strings_by_language).items()
            }
        table_types = payload.get("localized_string_table_types", None)
        if table_types is not None:
            self._localized_string_table_types = {int(key): str(value) for key, value in dict(table_types).items()}
        default_language = payload.get("localized_default_language", None)
        if default_language:
            self.localized_default_language = language_code(str(default_language))
        self._localized_strings = dict(self._localized_strings_by_language.get(self.localized_default_language, {}))
        self._localized_strings_loaded = True

    @property
    def localized_strings(self) -> dict[int, str]:
        self._ensure_localized_strings_loaded()
        return self._localized_strings

    @localized_strings.setter
    def localized_strings(self, value: Mapping[int, str]) -> None:
        self._localized_strings = {int(key): str(text) for key, text in dict(value).items()}
        self._localized_strings_loaded = True

    @property
    def localized_strings_by_language(self) -> dict[str, dict[int, str]]:
        self._ensure_localized_strings_loaded()
        return self._localized_strings_by_language

    @localized_strings_by_language.setter
    def localized_strings_by_language(self, value: Mapping[str, Mapping[int, str]]) -> None:
        self._localized_strings_by_language = {
            language_code(str(language)): {int(key): str(text) for key, text in dict(values).items()}
            for language, values in dict(value).items()
        }
        self._localized_strings_loaded = True

    @property
    def localized_string_table_types(self) -> dict[int, str]:
        self._ensure_localized_strings_loaded()
        return self._localized_string_table_types

    @localized_string_table_types.setter
    def localized_string_table_types(self, value: Mapping[int, str]) -> None:
        self._localized_string_table_types = {int(key): str(table_type) for key, table_type in dict(value).items()}
        self._localized_strings_loaded = True

    @property
    def header(self) -> _PluginHeaderView:
        return self._header_view

    @header.setter
    def header(self, value: PluginHeader) -> None:
        self._header = value

    @property
    def masters(self) -> list[str]:
        return self.header.masters

    @masters.setter
    def masters(self, value: list[str]) -> None:
        self.header.masters = value

    @property
    def is_localized(self) -> bool:
        return self.header.is_localized

    @is_localized.setter
    def is_localized(self, value: bool) -> None:
        self.header.is_localized = bool(value)

    @property
    def root_items(self) -> list[Group | Record]:
        return self._root_items if self._root_items is not None else []

    @root_items.setter
    def root_items(self, value: list[Group | Record]) -> None:
        self._root_items = value

    @classmethod
    def from_bytes(
        cls,
        data: bytes,
        *,
        plugin_name: str,
        file_path: Path | None = None,
        game: str | None = None,
        auto_load_strings: bool = True,
        strings_dir: str | Path | None = None,
        language: str | None = None,
        jobs: int | None = None,
    ) -> "Plugin":
        """Parse a plugin from raw bytes through the native handle API."""
        _ = jobs
        handle = _native_runtime.plugin_handle_from_bytes(
            bytes(data),
            plugin_name,
            game,
            auto_load_strings,
            str(strings_dir) if strings_dir is not None else None,
            language,
            str(file_path) if file_path is not None else None,
        )
        plugin = cls._from_native_handle(handle)
        if file_path is not None:
            plugin.file_path = file_path
        plugin._plugin_name = plugin_name
        if game is not None:
            plugin.game = game
        return plugin

    @classmethod
    def new(
        cls,
        plugin_name: str,
        *,
        game: str = "fo4",
        masters: Iterable[str] | None = None,
        header_size: int | None = None,
    ) -> "Plugin":
        handle = _native_runtime.plugin_handle_new(plugin_name, game)
        plugin = cls._from_native_handle(handle)
        if header_size is not None:
            _native_runtime.plugin_handle_call(handle, "set_header_size", header_size)
        if game in DEFAULT_HEDR_VERSION:
            _native_runtime.plugin_handle_call(handle, "set_header_version", DEFAULT_HEDR_VERSION[game])
        for master_name in list(masters or []):
            plugin.add_master(master_name)
        return plugin

    @property
    def header_size(self) -> int:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return int(_native_runtime.plugin_handle_get(handle, "header_size", MODERN_HEADER_SIZE))
        return getattr(self, "_header_size", MODERN_HEADER_SIZE)

    @header_size.setter
    def header_size(self, value: int) -> None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            _native_runtime.plugin_handle_call(handle, "set_header_size", int(value))
            return
        self._header_size = value

    @property
    def records(self) -> list[Record | _native_runtime.RecordSummary]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None and self._root_items is None:
            summaries: list[_native_runtime.RecordSummary] = []
            for form_id in _native_runtime.plugin_handle_record_form_ids(handle):
                summary = _native_runtime.plugin_handle_record_summary(handle, form_id)
                if summary is not None:
                    summaries.append(summary)
            return summaries
        records: list[Record] = []
        for item in self.root_items:
            if isinstance(item, Record):
                records.append(item)
            else:
                records.extend(item.walk_records())
        return records

    @property
    def record_count(self) -> int:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return int(_native_runtime.plugin_handle_get(handle, "record_count", 0))
        return len(self.records)

    def record_index_rows(
        self,
        *,
        signatures: Iterable[str] | None = None,
        form_keys: Iterable[str] | None = None,
    ) -> list[tuple[str, str, str, int, int]]:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("record_index_rows requires a native-backed Plugin")
        return _native_runtime.plugin_handle_record_index_rows(
            handle,
            signatures=list(signatures) if signatures is not None else None,
            form_keys=list(form_keys) if form_keys is not None else None,
        )

    def collect_worldspace_terrain_ids(
        self,
        *,
        worldspace_editor_id: str,
        min_x: int,
        min_y: int,
        max_x: int,
        max_y: int,
    ) -> dict:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("collect_worldspace_terrain_ids requires a native-backed Plugin")
        return _native_runtime.plugin_handle_collect_worldspace_terrain_ids(
            handle,
            worldspace_editor_id=worldspace_editor_id,
            min_x=min_x,
            min_y=min_y,
            max_x=max_x,
            max_y=max_y,
        )

    def carry_worldspace_header_from_source(
        self,
        source_plugin: "Plugin",
        *,
        source_worldspace_editor_id: str,
        target_worldspace_editor_id: str,
    ) -> dict:
        target_handle = getattr(self, "_rust_handle", None)
        source_handle = getattr(source_plugin, "_rust_handle", None)
        if target_handle is None or source_handle is None:
            raise RuntimeError("worldspace header carry requires native-backed Plugins")
        return _native_runtime.plugin_handle_carry_worldspace_header_from_source(
            source_handle,
            target_handle,
            source_worldspace_editor_id=source_worldspace_editor_id,
            target_worldspace_editor_id=target_worldspace_editor_id,
        )

    def sync_cell_max_height_from_source(
        self,
        source_plugin: "Plugin",
        *,
        source_worldspace_editor_id: str,
        target_worldspace_editor_id: str,
    ) -> dict:
        target_handle = getattr(self, "_rust_handle", None)
        source_handle = getattr(source_plugin, "_rust_handle", None)
        if target_handle is None or source_handle is None:
            raise RuntimeError("cell max-height sync requires native-backed Plugins")
        return _native_runtime.plugin_handle_sync_cell_max_height_from_source(
            source_handle,
            target_handle,
            source_worldspace_editor_id=source_worldspace_editor_id,
            target_worldspace_editor_id=target_worldspace_editor_id,
        )

    def repair_term_marker_parameters_from_source(
        self, source_plugin: "Plugin", *, dry_run: bool = False
    ) -> list[dict]:
        target_handle = getattr(self, "_rust_handle", None)
        source_handle = getattr(source_plugin, "_rust_handle", None)
        if target_handle is None or source_handle is None:
            raise RuntimeError("TERM marker repair requires native-backed Plugins")
        return _native_runtime.plugin_handle_repair_term_marker_parameters_from_source(
            target_handle,
            source_handle,
            dry_run=dry_run,
        )

    @property
    def groups(self) -> list[Group]:
        return [item for item in self.root_items if isinstance(item, Group)]

    @property
    def group_signatures(self) -> list[tuple[str, int]]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return _native_runtime.plugin_handle_group_signatures(handle)
        return [(group.label_text, len(group.children)) for group in self.groups]

    def __len__(self) -> int:
        return self.record_count

    def get_record_by_form_id(self, form_id: int) -> Record | _native_runtime.RecordSummary | None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return _native_runtime.plugin_handle_record_summary(handle, int(form_id) & 0xFFFFFFFF)
        return None

    def _coerce_local_object_id(self, form_id: int) -> int | None:
        raw_form_id = int(form_id) & 0xFFFFFFFF
        if raw_form_id == 0:
            return 0
        if raw_form_id <= 0x00FFFFFF:
            return raw_form_id
        normalized = self.normalize_form_id(raw_form_id)
        if normalized.object_id == 0:
            return 0
        if normalized.plugin_name not in {None, self.plugin_name}:
            return None
        return normalized.object_id

    def get_referenced_form_ids(self, form_id: int) -> list[int]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "get_referenced_form_ids", int(form_id) & 0xFFFFFFFF)
            if result is not _MISSING_HANDLE_METHOD:
                return list(result)
        return []

    def get_referencing_form_ids(self, form_id: int) -> list[int]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "get_referencing_form_ids", int(form_id) & 0xFFFFFFFF)
            if result is not _MISSING_HANDLE_METHOD:
                return list(result)
        return []

    def get_form_id_chain(self, form_id: int) -> list[int]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "get_form_id_chain", int(form_id) & 0xFFFFFFFF)
            if result is not _MISSING_HANDLE_METHOD:
                return list(result)
        return []

    def get_addon_nodes_by_index_id(self, index_id: int) -> list[_native_runtime.RecordSummary]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return _native_runtime.plugin_handle_addon_node_summaries_by_index_id(
                handle,
                int(index_id),
            )
        return []

    def get_addon_node_by_index_id(self, index_id: int) -> _native_runtime.RecordSummary | None:
        records = self.get_addon_nodes_by_index_id(index_id)
        return records[0] if records else None

    def assets_by_kind(self, kind: str) -> list[tuple[str, str]]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "assets_by_kind", kind)
            if result is not _MISSING_HANDLE_METHOD:
                return [(str(form_key), str(path)) for form_key, path in result]
        return []

    def eid_index(self) -> dict[str, list[str]]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "record_eid_index")
            if result is not _MISSING_HANDLE_METHOD:
                return {str(key): list(value) for key, value in dict(result).items()}
        return {}

    def search_records(
        self,
        pattern: str,
        *,
        mode: str = "glob",
        match_full: bool = False,
        read_full: bool = False,
        signatures: Iterable[str] | None = None,
        case_sensitive: bool = False,
        limit: int | None = None,
    ) -> list[dict]:
        """Find records whose EditorID (and optionally full name) match ``pattern``.

        ``mode`` is one of glob/substring/regex. ``match_full`` also matches against the
        record's full name; ``read_full`` includes the full name in the output without
        matching on it. Either reads the lossless export per candidate; otherwise only a
        lightweight summary is fetched. ``signatures`` narrows the candidate set by record
        type. Cost is O(records) like ``list-records`` — for converted multi-GB ESMs prefer
        the indexed ``modkit data`` search.
        """
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            return []
        return _native_runtime.plugin_handle_search_records(
            handle,
            pattern,
            mode=mode,
            match_full=match_full,
            read_full=read_full,
            signatures=[str(sig) for sig in signatures] if signatures else None,
            case_sensitive=case_sensitive,
            limit=limit,
        )

    def get_referenced_form_keys(self, form_key: str) -> list[str]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "get_referenced_form_keys", form_key)
            if result is not _MISSING_HANDLE_METHOD:
                return list(result)
        return []

    def get_referenced_form_keys_by_subrecord(
        self,
        form_key: str,
        subrecord_sig: str,
    ) -> list[str]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(
                handle,
                "get_referenced_form_keys_by_subrecord",
                form_key,
                subrecord_sig,
            )
            if result is not _MISSING_HANDLE_METHOD:
                return [str(value) for value in result]
        return []

    def get_referencing_form_keys(self, form_key: str) -> list[str]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "get_referencing_form_keys", form_key)
            if result is not _MISSING_HANDLE_METHOD:
                return list(result)
        return []

    def index_stats(self) -> dict[str, int]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "index_stats")
            if result is not _MISSING_HANDLE_METHOD:
                return {str(key): int(value) for key, value in dict(result).items()}
        return {"record_count": self.record_count}

    def collect_assets(
        self,
        *,
        master_plugins: list["Plugin"] | None = None,
        asset_kinds: list[str] | None = None,
        signatures: list[str] | None = None,
        form_keys: list[str] | None = None,
    ) -> list[dict]:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("collect_assets requires a native handle")
        master_handles = [
            master_handle
            for plugin in (master_plugins or [])
            if (master_handle := getattr(plugin, "_rust_handle", None)) is not None
        ]
        return _native_runtime.plugin_handle_collect_assets(
            [handle],
            master_handles,
            asset_kinds=asset_kinds,
            signatures=signatures,
            form_keys=form_keys,
        )

    def collect_cell_slice_roots(self, bounds) -> dict:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("collect_cell_slice_roots requires a native handle")
        return _native_runtime.plugin_handle_collect_cell_slice_roots(
            handle,
            worldspace_editor_id=bounds.worldspace_editor_id,
            min_x=bounds.min_x,
            min_y=bounds.min_y,
            max_x=bounds.max_x,
            max_y=bounds.max_y,
            include_worldspace_persistent_cell=bounds.include_worldspace_persistent_cell,
        )

    def walk_dependencies(
        self,
        *,
        master_plugins: list["Plugin"] | None = None,
        root_form_keys: list[str] | None = None,
        policy_json: str = "{}",
        strict: bool = True,
    ) -> dict:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("walk_dependencies requires a native handle")
        master_handles = [
            master_handle
            for plugin in (master_plugins or [])
            if (master_handle := getattr(plugin, "_rust_handle", None)) is not None
        ]
        return _native_runtime.plugin_handle_walk_dependencies(
            [handle],
            master_handles,
            root_form_keys or [],
            policy_json,
            strict_unresolved_masters=strict,
        )

    def set_localized_strings(
        self,
        values: Mapping[int, str] | None,
        *,
        language: str | None = None,
        table_types: Mapping[int, str] | None = None,
    ) -> None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            normalized = {int(k): str(v) for k, v in (values or {}).items()}
            tt = {int(k): str(v) for k, v in (table_types or {}).items()} if table_types else None
            lang = language_code(language or "en")
            result = _try_handle_call(handle, "set_localized_strings", normalized, lang, tt)
            if result is not _MISSING_HANDLE_METHOD:
                self._localized_strings_loaded = False
                return
        normalized_language = language_code(language or self.localized_default_language)
        normalized_values = {int(key): str(value) for key, value in (values or {}).items()}
        self.localized_default_language = normalized_language
        self.localized_strings = dict(normalized_values)
        self.localized_strings_by_language[normalized_language] = dict(normalized_values)
        if table_types:
            for string_id, table_type in table_types.items():
                self.localized_string_table_types[int(string_id)] = str(table_type)

    def set_localized_strings_by_language(
        self,
        values_by_language: Mapping[str, Mapping[int, str]] | None,
        *,
        preferred_language: str | None = None,
        table_types: Mapping[int, str] | None = None,
    ) -> None:
        tables: dict[str, dict[int, str]] = {}
        for language, values in (values_by_language or {}).items():
            tables[language_code(language)] = {int(key): str(value) for key, value in values.items()}
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            normalized_tt = (
                {int(key): str(value) for key, value in table_types.items()}
                if table_types is not None
                else None
            )
            result = _try_handle_call(handle, "set_localized_strings_by_language", tables, preferred_language, normalized_tt)
            if result is not _MISSING_HANDLE_METHOD:
                self._localized_strings_loaded = False
                return
        self.localized_strings_by_language = tables
        if table_types is not None:
            self.localized_string_table_types = {int(key): str(value) for key, value in table_types.items()}
        if preferred_language is not None:
            self.localized_default_language = language_code(preferred_language)
        elif tables and self.localized_default_language not in tables:
            self.localized_default_language = "en" if "en" in tables else next(iter(tables))
        self.localized_strings = dict(self.localized_strings_by_language.get(self.localized_default_language, {}))

    def resolve_string(self, string_id: int, *, language: str | None = None) -> str | None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "resolve_string", int(string_id), language)
            return None if result is _MISSING_HANDLE_METHOD else result
        return None

    def resolve_string_values(self, string_id: int) -> dict[str, str]:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "resolve_string_values", int(string_id))
            if result is not _MISSING_HANDLE_METHOD:
                return {
                    language_display_name(str(language)): str(value)
                    for language, value in dict(result).items()
                }
        return {}

    def set_localized_field_values(
        self,
        string_id: int,
        values_by_language: Mapping[str, str],
        *,
        preferred_language: str | None = None,
        table_type: str | None = None,
    ) -> None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(
                handle,
                "set_localized_field_values",
                int(string_id),
                dict(values_by_language),
                preferred_language,
                table_type,
            )
            if result is not _MISSING_HANDLE_METHOD:
                self._localized_strings_loaded = False
                return
        key = int(string_id)
        for language, text in values_by_language.items():
            normalized_language = language_code(language)
            table = self.localized_strings_by_language.setdefault(normalized_language, {})
            table[key] = str(text)
        preferred = language_code(preferred_language or self.localized_default_language)
        self.localized_default_language = preferred
        preferred_values = self.localized_strings_by_language.get(preferred)
        if preferred_values is not None and key in preferred_values:
            self.localized_strings[key] = preferred_values[key]
        else:
            english = self.localized_strings_by_language.get("en")
            if english is not None and key in english:
                self.localized_strings[key] = english[key]
        if table_type:
            self.localized_string_table_types[key] = table_type

    def allocate_localized_string_id(self, *, preferred_start: int = 1) -> int:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            result = _try_handle_call(handle, "allocate_localized_string_id", preferred_start)
            if result is not _MISSING_HANDLE_METHOD:
                return int(result)
        used_ids = {int(key) for key in self.localized_strings.keys()}
        for table in self.localized_strings_by_language.values():
            used_ids.update(int(key) for key in table.keys())
        candidate = int(preferred_start)
        while candidate in used_ids:
            candidate += 1
        return candidate

    def _is_localized(self) -> bool:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return bool(_native_runtime.plugin_handle_get(handle, "is_localized", False))
        return bool(self._header.is_localized) if self._header is not None else False

    def save_localized_strings(self, plugin_path: str | Path | None = None) -> list[Path]:
        if not self._is_localized():
            return []
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            output_arg = str(Path(plugin_path)) if plugin_path is not None else None
            written_paths = _try_handle_call(handle, "save_localized_strings", output_arg)
            if written_paths is not _MISSING_HANDLE_METHOD:
                return [Path(p) for p in written_paths]
        target = Path(plugin_path) if plugin_path is not None else self.file_path
        if target is None:
            raise ValueError("No output path specified")
        localized_strings_by_language: dict[str, dict[int, str]] = self.localized_strings_by_language
        localized_strings: dict[int, str] = self.localized_strings
        localized_default_language: str = self.localized_default_language
        localized_string_table_types: dict[int, str] = self.localized_string_table_types
        if not localized_strings_by_language and localized_strings:
            localized_strings_by_language = {localized_default_language: dict(localized_strings)}
        if not localized_strings_by_language:
            return []
        strings_dir = target.parent / "Strings"
        written: list[Path] = []
        for language, values in sorted(localized_strings_by_language.items()):
            buckets = {table_type: {} for table_type in STRING_TABLE_EXTENSIONS}
            for string_id, text in values.items():
                table_type = localized_string_table_types.get(int(string_id), "strings")
                buckets.setdefault(table_type, {})[int(string_id)] = str(text)
            for table_type, table_values in buckets.items():
                if not table_values:
                    continue
                filename = f"{target.stem}_{language}{STRING_TABLE_EXTENSIONS[table_type]}"
                written.append(write_string_table(strings_dir / filename, table_values, table_type=table_type))
        return written

    def autoload_strings(
        self,
        *,
        strings_dir: str | Path | None = None,
        strings_dirs: Iterable[str | Path] | None = None,
        language: str | None = None,
    ) -> dict[int, str]:
        if not self._is_localized():
            return {}
        candidate_dirs: list[Path] = []
        if strings_dir is not None:
            candidate_dirs.append(Path(strings_dir))
        elif strings_dirs is not None:
            candidate_dirs.extend(Path(candidate) for candidate in strings_dirs)
        values: dict[int, str] = {}
        for candidate in candidate_dirs:
            loaded = load_string_tables(self.plugin_name, strings_dir=candidate, language=language)
            if not loaded:
                continue
            values = loaded
            break
        if values:
            self.set_localized_strings(values, language=language)
        return values

    def autoload_all_strings(
        self,
        *,
        strings_dir: str | Path | None = None,
        strings_dirs: Iterable[str | Path] | None = None,
    ) -> dict[str, dict[int, str]]:
        if not self._is_localized():
            return {}
        candidate_dirs: list[Path] = []
        if strings_dir is not None:
            candidate_dirs.append(Path(strings_dir))
        elif strings_dirs is not None:
            candidate_dirs.extend(Path(candidate) for candidate in strings_dirs)
        values_by_language: dict[str, dict[int, str]] = {}
        table_types: dict[int, str] = {}
        for candidate in candidate_dirs:
            loaded_values, loaded_types = load_all_string_tables(self.plugin_name, strings_dir=candidate)
            if not loaded_values:
                continue
            values_by_language = loaded_values
            table_types = loaded_types
            break
        if values_by_language:
            self.set_localized_strings_by_language(
                values_by_language,
                preferred_language="en" if "en" in values_by_language else next(iter(values_by_language)),
                table_types=table_types,
            )
        return values_by_language

    def normalize_form_id(self, raw: int) -> FormRef:
        return FormRef.from_raw(raw, plugin_name=self.plugin_name, masters=self.header.masters)

    def encode_form_ref(self, form_ref: FormRef, *, add_missing: bool = False) -> int:
        return form_ref.to_raw(
            target_plugin_name=self.plugin_name,
            masters=self.header.masters,
            add_missing=add_missing,
        )

    def add_master(self, master_name: str, *, size: int = 0) -> None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            _native_runtime.plugin_handle_call(handle, "add_master", master_name, size)
            return
        if self._header is None:
            raise RuntimeError("No header on pure-Python plugin")
        if master_name in self._header.masters:
            return
        self._header.masters.append(master_name)
        self._header.master_sizes.append(size)

    def add_recursive_masters(self, source_plugin: "Plugin") -> None:
        required = list(source_plugin.header.masters)
        overlap = min(len(required), len(self.header.masters))
        if self.header.masters[:overlap] != required[:overlap]:
            raise ValueError("Target plugin already has an incompatible master prefix")
        for master_name in required[len(self.header.masters):]:
            self.add_master(master_name)

    def ensure_source_chain(self, source_plugin: "Plugin", *, include_source_plugin: bool) -> None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            _native_runtime.plugin_handle_call(
                handle,
                "ensure_source_masters",
                list(source_plugin.header.masters),
                source_plugin.plugin_name if include_source_plugin else None,
            )
            return
        if self._header is None:
            raise RuntimeError("No header on pure-Python plugin")
        required = list(source_plugin.header.masters)
        if include_source_plugin:
            required.append(source_plugin.plugin_name)
        overlap = min(len(required), len(self._header.masters))
        if self._header.masters[:overlap] != required[:overlap]:
            raise ValueError("Target plugin already has an incompatible master prefix")
        for master_name in required[len(self._header.masters):]:
            self.add_master(master_name)

    def set_masters(self, masters: list[tuple[str, int]]) -> None:
        """Replace the master list with (name, size) pairs, remapping FormIDs by name.

        Native-backed only. The remap runs in Rust and matches refs by master
        *name*, so reordering preserves every reference; dropping a master leaves
        refs that pointed at it dangling (callers must repair those first).
        """
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise ValueError("set_masters requires the native ESP backend")
        _native_runtime.plugin_handle_call(
            handle,
            "set_masters",
            [(str(name), int(size)) for name, size in masters],
        )

    def allocate_form_id(self) -> int:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return int(_native_runtime.plugin_handle_call(handle, "allocate_form_id"))
        if self._header is None:
            raise RuntimeError("No header on pure-Python plugin")
        object_id = self._header.next_object_id & 0x00FFFFFF
        self._header.next_object_id = (object_id + 1) & 0x00FFFFFF
        return (LOCAL_FORM_INDEX << 24) | object_id

    def new_record(self, signature: str, *, form_id: int | None = None) -> Record:
        if self.header_size == MODERN_HEADER_SIZE:
            return Record(
                signature=signature,
                form_id=form_id if form_id is not None else self.allocate_form_id(),
                form_version=0,
                version2=0,
            )
        return Record(signature=signature, form_id=form_id if form_id is not None else self.allocate_form_id())

    def validate_record(self, record: Record) -> None:
        _native_runtime.validate_record_native(self, record)

    def find_top_group(self, signature: str) -> Group | None:
        wanted = signature.encode("ascii", errors="replace")
        for group in (item for item in self.root_items if isinstance(item, Group)):
            if group.group_type == 0 and group.label.rstrip(b"\x00") == wanted:
                return group
        return None

    def ensure_top_group(self, signature: str) -> Group:
        group = self.find_top_group(signature)
        if group is not None:
            return group
        if self._root_items is None:
            self._root_items = []
        new_group = Group(signature.encode("ascii"), 0, tail=b"\x00" * (self.header_size - 16))
        self._root_items.append(new_group)
        return new_group

    def add_record(self, record: Record) -> Record:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            record.form_id = _native_runtime.plugin_handle_add_record_raw(
                handle,
                record.signature,
                record.form_id,
                record.flags,
                record.version_control,
                record.form_version,
                record.version2,
                [
                    (subrecord.signature, bytes(subrecord.data), subrecord.semantic_type)
                    for subrecord in record.subrecords
                ],
            )
            return record
        if self._root_items is None:
            self._root_items = []
        self.ensure_top_group(record.signature).children.append(record)
        return record

    def remove_record(self, target: Record) -> bool:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return bool(_native_runtime.plugin_handle_call(handle, "remove_record", int(target.form_id) & 0xFFFFFFFF))
        return False

    def remove_record_by_form_id(self, form_id: int) -> bool:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            return False
        fid = int(form_id) & 0xFFFFFFFF
        # A bare 24-bit object id won't match the stored form_id, which carries the
        # owner index in its high byte; resolve to the actual record's form_id.
        if fid <= 0x00FFFFFF:
            summary = self.get_record_by_form_id(fid)
            if summary is not None and getattr(summary, "form_id", None) is not None:
                fid = int(summary.form_id) & 0xFFFFFFFF
        return bool(_native_runtime.plugin_handle_call(handle, "remove_record", fid))

    def upsert_authoring_record(self, record: dict) -> str:
        import json

        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            raise RuntimeError("upsert requires the native ESP backend")
        return _native_runtime.plugin_handle_replace_authoring_record(handle, json.dumps(record))

    def read_authoring_record(self, form_id: int) -> dict | None:
        import json

        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            return None
        text = _native_runtime.plugin_handle_read_authoring_record(handle, form_id)
        return json.loads(text) if text is not None else None

    def copy_record(self, record: Record, source_plugin: "Plugin") -> Record | _native_runtime.RecordSummary | None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            source_handle = getattr(source_plugin, "_rust_handle", None)
            if source_handle is None:
                raise ValueError("source_plugin must be native-backed for handle copy")
            self.ensure_source_chain(source_plugin, include_source_plugin=True)
            copied = _native_runtime.plugin_handle_copy_record(
                source_handle,
                int(record.form_id) & 0xFFFFFFFF,
                handle,
                as_new=True,
            )
            if copied is None:
                return None
            return _native_runtime.plugin_handle_record_summary(handle, copied)
        self.ensure_source_chain(source_plugin, include_source_plugin=True)
        clone = record.clone()
        self._remap_semantic_subrecords_from_source(clone, source_plugin)
        clone.form_id = self.allocate_form_id()
        self.add_record(clone)
        return clone

    def copy_override(self, record: Record, source_plugin: "Plugin") -> Record | _native_runtime.RecordSummary | None:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            source_handle = getattr(source_plugin, "_rust_handle", None)
            if source_handle is None:
                raise ValueError("source_plugin must be native-backed for handle copy")
            self.ensure_source_chain(source_plugin, include_source_plugin=True)
            copied = _native_runtime.plugin_handle_copy_record(
                source_handle,
                int(record.form_id) & 0xFFFFFFFF,
                handle,
                as_new=False,
            )
            if copied is None:
                return None
            return _native_runtime.plugin_handle_record_summary(handle, copied)
        self.ensure_source_chain(source_plugin, include_source_plugin=True)
        clone = record.clone()
        self._remap_semantic_subrecords_from_source(clone, source_plugin)
        clone.form_id = self.encode_form_ref(self._normalize_source_form_id(record.form_id, source_plugin), add_missing=True)
        self.add_record(clone)
        return clone

    def save(
        self,
        path: str | Path | None = None,
        *,
        backend: str = "auto",
        close_after_save: bool = False,
    ) -> Path:
        target = Path(path) if path is not None else self.file_path
        if target is None:
            raise ValueError("No output path specified")
        _native_runtime.should_use_native_backend(backend, "save_plugin_native")
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            _native_runtime.plugin_handle_call(handle, "save", str(target))
            self.file_path = target
            self._plugin_name = target.name
            if close_after_save:
                self.close()
            return target
        _native_runtime.save_plugin_native(self, str(target), game=self.game)
        self.file_path = target
        self._plugin_name = target.name
        if close_after_save:
            self.close()
        return target

    def close(self) -> bool:
        handle = getattr(self, "_rust_handle", None)
        if handle is None:
            return False
        closed = _native_runtime.plugin_handle_close(handle)
        self._rust_handle = None
        return bool(closed)

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "Plugin":
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        self.close()
        return False

    def to_bytes(self) -> bytes:
        handle = getattr(self, "_rust_handle", None)
        if handle is not None:
            return bytes(_native_runtime.plugin_handle_call(handle, "to_bytes"))
        return _native_runtime.plugin_to_bytes_native(self)

    def _normalize_source_form_id(self, raw: int, source_plugin: "Plugin") -> FormRef:
        normalized = source_plugin.normalize_form_id(raw)
        if ((raw >> 24) & 0xFF) == LOCAL_FORM_INDEX and normalized.object_id != 0:
            return FormRef(source_plugin.plugin_name, normalized.object_id, raw=raw)
        return normalized

    def _remap_semantic_subrecords_from_source(self, record: Record, source_plugin: "Plugin") -> None:
        for subrecord in record.subrecords:
            if subrecord.semantic_type == "formid" and len(subrecord.data) >= 4:
                normalized = self._normalize_source_form_id(subrecord.get_uint32(), source_plugin)
                subrecord.set_uint32(0, self.encode_form_ref(normalized, add_missing=True))
            elif subrecord.semantic_type == "formid_array" and len(subrecord.data) % 4 == 0:
                rewritten = bytearray()
                for offset in range(0, len(subrecord.data), 4):
                    raw = struct.unpack_from("<I", subrecord.data, offset)[0]
                    normalized = self._normalize_source_form_id(raw, source_plugin)
                    rewritten.extend(struct.pack("<I", self.encode_form_ref(normalized, add_missing=True)))
                subrecord.data = rewritten


def _make_stub_group(signature: str, header_size: int) -> Group:
    return Group(signature.encode("ascii"), 0, tail=b"\x00" * (header_size - 16))
