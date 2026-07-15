"""Thin Python boundary for native ESP runtime entrypoints."""

from __future__ import annotations

from dataclasses import dataclass
from importlib import import_module
from typing import Any, Literal, Mapping

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_DHAT_ATEXIT_REGISTERED = False
BackendName = Literal["auto", "native"]


@dataclass(frozen=True, slots=True)
class RecordSummary:
    form_id: int
    signature: str
    editor_id: str | None = None


class NativeHandleId(int):
    """Python-side adapter for native integer handle IDs."""

    @property
    def _rust_handle(self) -> int:
        return int(self)

    def __getattr__(self, name: str) -> Any:
        def method(*args: Any, **kwargs: Any) -> Any:
            return plugin_handle_call(self, name, *args, **kwargs)

        return method


def _wrap_handle(handle: Any) -> Any:
    if isinstance(handle, int) and not isinstance(handle, NativeHandleId):
        return NativeHandleId(handle)
    return handle


def _load_umbrella_submodule() -> Any:
    umbrella = import_module("creation_lib._native")
    native_module = getattr(umbrella, "esp_authoring_core", None)
    if native_module is not None:
        return native_module
    try:
        return import_module("creation_lib._native.esp_authoring_core")
    except ImportError:
        pass
    # Fallback: umbrella may be a namespace package; try loading the .pyd directly.
    pyd = import_module("creation_lib._native")
    native_module = getattr(pyd, "esp_authoring_core", None)
    if native_module is not None:
        return native_module
    raise ImportError("esp_authoring_core not found in creation_lib._native")


def load_native_module() -> Any:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        if _NATIVE_MODULE is None:
            raise RuntimeError("esp_authoring_core is required for creation_lib.esp")
        return _NATIVE_MODULE
    _NATIVE_IMPORT_ATTEMPTED = True
    try:
        _NATIVE_MODULE = _load_umbrella_submodule()
    except ImportError as exc:
        try:
            _NATIVE_MODULE = import_module("esp_authoring_core")
        except ImportError as umbrella_exc:
            raise RuntimeError("esp_authoring_core is required for creation_lib.esp") from umbrella_exc
    return _NATIVE_MODULE


def _require_native_function(name: str) -> Any:
    module = load_native_module()
    native_fn = getattr(module, name, None)
    if native_fn is None:
        raise RuntimeError(f"esp_authoring_core is missing required function {name}()")
    return native_fn


def _optional_native_function(name: str) -> Any | None:
    return getattr(load_native_module(), name, None)


def dhat_heap_start(output_path: str | None = None) -> bool:
    global _DHAT_ATEXIT_REGISTERED
    native_fn = _require_native_function("dhat_heap_start_native")
    started = bool(native_fn(output_path))
    if started and not _DHAT_ATEXIT_REGISTERED:
        import atexit

        atexit.register(dhat_heap_stop)
        _DHAT_ATEXIT_REGISTERED = True
    return started


def dhat_heap_stop() -> bool:
    native_fn = _require_native_function("dhat_heap_stop_native")
    return bool(native_fn())


def trim_allocator() -> None:
    """Force mimalloc to decommit cached free pages back to the OS.

    No-op if the native extension predates the trim hook (older `.pyd`).
    """
    native_fn = _optional_native_function("trim_allocator_native")
    if native_fn is not None:
        native_fn()


def _is_handle_id(handle: Any) -> bool:
    return isinstance(handle, int)


def _strings_from_payload(payload: Any) -> dict[str, Any]:
    if isinstance(payload, dict):
        return payload
    default_language, localized_rows, by_language_rows, table_type_rows = payload
    by_language: dict[str, dict[int, str]] = {}
    for row_language, string_id, text in by_language_rows:
        by_language.setdefault(str(row_language), {})[int(string_id)] = str(text)
    result: dict[str, Any] = {
        "localized_default_language": default_language,
        "localized_strings_by_language": by_language,
        "localized_string_table_types": {
            int(string_id): str(table_type)
            for string_id, table_type in table_type_rows
        },
    }
    if localized_rows:
        result["localized_strings"] = {
            int(string_id): str(text)
            for string_id, text in localized_rows
        }
    return result


def _subrecord_payload_from_row(row: Any) -> dict[str, Any]:
    if isinstance(row, dict):
        return row
    return {
        "signature": row[0],
        "data": bytes(row[1]),
        "semantic_type": row[2],
    }


def _header_payload_from_row(row: Any) -> dict[str, Any]:
    if isinstance(row, dict):
        return row
    return {
        "version": row[0],
        "num_records": row[1],
        "next_object_id": row[2],
        "author": row[3],
        "description": row[4],
        "masters": list(row[5]),
        "master_sizes": list(row[6]),
        "overridden_forms": list(row[7]),
        "flags": row[8],
        "extra_subrecords": [_subrecord_payload_from_row(item) for item in row[9]],
        "version_control": row[10],
        "form_version": row[11],
        "version2": row[12],
        "hedr_raw": None if row[13] is None else bytes(row[13]),
        "raw_subrecords": [_subrecord_payload_from_row(item) for item in row[14]],
    }


def _metadata_from_payload(payload: Any) -> dict[str, Any]:
    if isinstance(payload, dict):
        return payload
    if len(payload) == 2 and not isinstance(payload[0], str):
        meta = _metadata_from_payload(payload[0])
        meta.update(_strings_from_payload(payload[1]))
        return meta
    return {
        "plugin_name": payload[0],
        "file_path": payload[1],
        "game": payload[2],
        "header_size": payload[3],
        "header": _header_payload_from_row(payload[4]),
        "record_count": payload[5],
        "localized_default_language": payload[6],
    }


def _record_context_from_payload(payload: Any) -> dict[str, Any] | None:
    if payload is None or isinstance(payload, dict):
        return payload
    return {
        "record_signature": payload[0],
        "record_form_version": payload[1],
        "record_version2": payload[2],
    }


def _metadata(handle: int) -> dict[str, Any]:
    native_fn = _optional_native_function("plugin_handle_get_meta")
    if callable(native_fn):
        return _metadata_from_payload(native_fn(handle))
    return _metadata_from_payload(_require_native_function("plugin_handle_metadata")(handle))


def plugin_handle_get_meta(handle: Any) -> dict[str, Any]:
    if not _is_handle_id(handle):
        return {}
    return _metadata(handle)


def plugin_handle_get_strings(handle: Any, language: str | None = None) -> dict[str, Any]:
    if not _is_handle_id(handle):
        return {}
    native_fn = _optional_native_function("plugin_handle_get_strings")
    if callable(native_fn):
        return _strings_from_payload(native_fn(handle, language))
    meta = _metadata(handle)
    return {
        "localized_default_language": meta.get("localized_default_language"),
        "localized_strings_by_language": meta.get("localized_strings_by_language"),
        "localized_string_table_types": meta.get("localized_string_table_types"),
    }


def plugin_handle_max_object_id(handle: Any) -> int:
    if not _is_handle_id(handle):
        return 0
    native_fn = _require_native_function("plugin_handle_max_object_id")
    return int(native_fn(handle))


def _subrecord_from_payload(payload: Mapping[str, Any]) -> Any:
    from creation_lib.esp.model import Subrecord

    return Subrecord(str(payload["signature"]), payload.get("data"), payload.get("semantic_type"))


def _header_from_payload(payload: Mapping[str, Any]) -> Any:
    from creation_lib.esp.model import PluginHeader

    return PluginHeader(
        version=float(payload.get("version", 1.0)),
        num_records=int(payload.get("num_records", 0)),
        next_object_id=int(payload.get("next_object_id", 0x800)),
        author=str(payload.get("author", "")),
        description=str(payload.get("description", "")),
        masters=list(payload.get("masters", [])),
        master_sizes=[int(v) for v in payload.get("master_sizes", [])],
        overridden_forms=[int(v) for v in payload.get("overridden_forms", [])],
        flags=int(payload.get("flags", 0)),
        extra_subrecords=[_subrecord_from_payload(item) for item in payload.get("extra_subrecords", [])],
        version_control=int(payload.get("version_control", 0)),
        form_version=payload.get("form_version"),
        version2=payload.get("version2"),
        hedr_raw=payload.get("hedr_raw"),
        _raw_subrecords=[_subrecord_from_payload(item) for item in payload.get("raw_subrecords", [])],
    )


def _plugin_handle_method(handle: Any, name: str) -> Any | None:
    if not _is_handle_id(handle):
        return None
    if name == "add_record":
        native_fn = _optional_native_function("plugin_handle_add_record_raw")
        if callable(native_fn):
            def call(record: Any) -> int:
                subrecords = [
                    (
                        str(getattr(subrecord, "signature")),
                        bytes(getattr(subrecord, "data", b"") or b""),
                        getattr(subrecord, "semantic_type", None),
                    )
                    for subrecord in (getattr(record, "subrecords", None) or [])
                ]
                return int(
                    native_fn(
                        handle,
                        str(getattr(record, "signature")),
                        int(getattr(record, "form_id")) & 0xFFFFFFFF,
                        int(getattr(record, "flags", 0) or 0) & 0xFFFFFFFF,
                        int(getattr(record, "version_control", 0) or 0) & 0xFFFFFFFF,
                        getattr(record, "form_version", None),
                        getattr(record, "version2", None),
                        subrecords,
                    )
                )
            return call
    direct = {
        "to_bytes": "plugin_handle_to_bytes",
        "save": "plugin_handle_save",
        "allocate_form_id": "plugin_handle_allocate_form_id",
        "max_object_id": "plugin_handle_max_object_id",
        "add_master": "plugin_handle_add_master",
        "set_masters": "plugin_handle_set_masters",
        "ensure_source_masters": "plugin_handle_ensure_source_masters",
        "remove_record": "plugin_handle_remove_record",
        "add_record_raw": "plugin_handle_add_record_raw",
        "apply_placed_record_position_offset": "plugin_handle_apply_placed_record_position_offset",
        "sanitize_subrecord_payloads": "plugin_handle_sanitize_subrecord_payloads",
        "record_summary": "plugin_handle_record_summary",
        "has_record": "plugin_handle_has_record",
        "record_payload_hash": "plugin_handle_record_payload_hash",
        "force_build_refs_section": "plugin_handle_force_build_refs_section",
        "assets_by_kind": "plugin_handle_assets_by_kind",
        "record_eid_index": "plugin_handle_record_eid_index",
        "local_object_ids": "plugin_handle_local_object_ids",
        "owned_object_ids": "plugin_handle_owned_object_ids",
        "record_form_ids": "plugin_handle_record_form_ids",
        "record_form_ids_with_subrecords": "plugin_handle_record_form_ids_with_subrecords",
        "record_subrecords": "plugin_handle_record_subrecords",
        "set_record_subrecords": "plugin_handle_set_record_subrecords",
        "used_master_indices": "plugin_handle_used_master_indices",
        "apply_object_id_mapping": "plugin_handle_apply_object_id_mapping",
        "null_refs_to_master": "plugin_handle_null_refs_to_master",
        "copy_record": "plugin_handle_copy_record",
        "merge_conflict_to_patch": "plugin_handle_merge_conflict_to_patch",
        "undelete_and_disable_refs": "plugin_handle_undelete_and_disable_refs",
        "addon_node_summaries_by_index_id": "plugin_handle_addon_node_summaries_by_index_id",
        "get_referenced_form_ids": "plugin_handle_get_referenced_form_ids",
        "get_referencing_form_ids": "plugin_handle_get_referencing_form_ids",
        "get_referenced_form_keys": "plugin_handle_get_referenced_form_keys",
        "get_referenced_form_keys_by_subrecord": "plugin_handle_get_referenced_form_keys_by_subrecord",
        "get_referencing_form_keys": "plugin_handle_get_referencing_form_keys",
        "get_form_id_chain": "plugin_handle_get_form_id_chain",
        "index_stats": "plugin_handle_index_stats",
        "record_context_for_form_id": "plugin_handle_record_context_for_form_id",
        "resolve_string": "plugin_handle_resolve_string",
        "resolve_string_values": "plugin_handle_resolve_string_values",
        "set_localized_strings": "plugin_handle_set_localized_strings",
        "set_localized_strings_by_language": "plugin_handle_set_localized_strings_by_language",
        "set_localized_field_values": "plugin_handle_set_localized_field_values",
        "allocate_localized_string_id": "plugin_handle_allocate_localized_string_id",
        "save_localized_strings": "plugin_handle_save_localized_strings",
        "set_logical_identity": "plugin_handle_set_logical_identity",
        "export_plugin_text": "plugin_handle_export_plugin_text",
        "export_record_text": "plugin_handle_export_record_text",
        "extract_dialogue_text": "plugin_handle_extract_dialogue_text",
        "export_authoring_dir": "plugin_handle_export_authoring_dir",
    }.get(name)
    if direct:
        native_fn = _optional_native_function(direct)
        if callable(native_fn):
            def call(*args: Any, **kwargs: Any) -> Any:
                payload = native_fn(handle, *args, **kwargs)
                if direct == "plugin_handle_record_context_for_form_id":
                    return _record_context_from_payload(payload)
                return payload
            return call
    if name.startswith("set_header_"):
        field = name.removeprefix("set_header_")
        native_fn = _optional_native_function("plugin_handle_set_header_field")
        if callable(native_fn):
            return lambda value: native_fn(handle, field, value)
    if name in {"set_is_localized", "set_header_size"}:
        field = "is_localized" if name == "set_is_localized" else "header_size"
        native_fn = _optional_native_function("plugin_handle_set_header_field")
        if callable(native_fn):
            return lambda value: native_fn(handle, field, value)
    return None


def plugin_handle_call(handle: Any, name: str, *args: Any, **kwargs: Any) -> Any:
    method = _plugin_handle_method(handle, name)
    if method is None:
        raise RuntimeError(f"esp_authoring_core is missing required plugin handle method {name}()")
    return method(*args, **kwargs)


def plugin_handle_get(handle: Any, name: str, default: Any = None) -> Any:
    if _is_handle_id(handle):
        if name in {"localized_strings_by_language", "localized_string_table_types"}:
            strings = plugin_handle_get_strings(handle)
            return strings.get(name, default)
        meta = _metadata(handle)
        header = dict(meta.get("header", {}))
        values = {
            "plugin_name": meta.get("plugin_name"),
            "file_path": meta.get("file_path"),
            "game": meta.get("game"),
            "header_size": meta.get("header_size"),
            "record_count": meta.get("record_count"),
            "localized_default_language": meta.get("localized_default_language"),
            "masters": header.get("masters"),
            "master_sizes": header.get("master_sizes"),
            "is_localized": bool(int(header.get("flags", 0)) & 0x80),
            "header_version": header.get("version"),
            "next_object_id": header.get("next_object_id"),
            "header_author": header.get("author"),
            "header_description": header.get("description"),
            "header_flags": header.get("flags"),
            "header": _header_from_payload(header),
        }
        return values.get(name, default)
    return default


def plugin_handle_set(handle: Any, name: str, value: Any) -> None:
    plugin_handle_call(handle, f"set_{name}", value)


def plugin_handle_load(
    plugin_path: str,
    *,
    game: str | None = None,
    strings_dir: str | None = None,
    language: str | None = None,
    eager_compressed: bool = True,
) -> Any:
    native_fn = _optional_native_function("plugin_handle_load")
    if callable(native_fn):
        return _wrap_handle(
            native_fn(
                plugin_path,
                game,
                strings_dir,
                language,
                eager_compressed,
            )
        )
    raise RuntimeError("esp_authoring_core is missing required function plugin_handle_load()")


def plugin_handle_load_index(
    plugin_path: str,
    *,
    game: str | None = None,
    strings_dir: str | None = None,
    language: str | None = None,
) -> Any:
    """Lazy/index-only load: drops the parsed tree, keeping only the formid/eid
    index + on-demand record parsing. For read-only handles (target masters)
    this avoids the multi-GB resident tree. Returns ``None`` if the native
    function is unavailable so callers can fall back to a full load."""
    native_fn = _optional_native_function("plugin_handle_load_index")
    if callable(native_fn):
        return _wrap_handle(native_fn(plugin_path, game, strings_dir, language))
    return None


def plugin_handle_new(plugin_name: str, game: str | None = None) -> Any:
    native_fn = _optional_native_function("plugin_handle_new")
    if callable(native_fn):
        return _wrap_handle(native_fn(plugin_name, game))
    raise RuntimeError("esp_authoring_core is missing required function plugin_handle_new()")


def plugin_handle_from_bytes(
    data: bytes,
    plugin_name: str,
    game: str | None = None,
    auto_load_strings: bool = True,
    strings_dir: str | None = None,
    language: str | None = None,
    file_path: str | None = None,
) -> Any:
    native_fn = _optional_native_function("plugin_handle_from_bytes")
    if callable(native_fn):
        return _wrap_handle(native_fn(data, plugin_name, game, auto_load_strings, strings_dir, language, file_path))
    raise RuntimeError("esp_authoring_core is missing required function plugin_handle_from_bytes()")


def plugin_handle_import_text(text: str, format: str = "json", game: str | None = None) -> Any:
    native_fn = _optional_native_function("plugin_handle_import_text")
    if callable(native_fn):
        return _wrap_handle(native_fn(text, format, game))
    raise RuntimeError("esp_authoring_core is missing required function plugin_handle_import_text()")


def plugin_handle_close(handle: Any) -> bool:
    native_fn = _optional_native_function("plugin_handle_close")
    if callable(native_fn):
        return bool(native_fn(handle))
    raise RuntimeError("esp_authoring_core is missing required function plugin_handle_close()")


def plugin_handle_group_signatures(handle: Any) -> list[tuple[str, int]]:
    """Return (label, child_count) for each top-level group, without materializing records."""
    native_fn = _require_native_function("plugin_handle_group_signatures")
    return [(str(label), int(count)) for label, count in native_fn(handle)]


def _record_summary_from_tuple(value: Any) -> RecordSummary:
    form_id, signature, editor_id = value
    return RecordSummary(int(form_id), str(signature), None if editor_id is None else str(editor_id))


def plugin_handle_group_record_summaries(handle: Any, group_signature: str) -> list[RecordSummary]:
    native_fn = _require_native_function("plugin_handle_group_record_summaries")
    return [_record_summary_from_tuple(item) for item in native_fn(handle, group_signature)]


def plugin_handle_record_summary(handle: Any, form_id: int) -> RecordSummary | None:
    native_fn = _require_native_function("plugin_handle_record_summary")
    result = native_fn(handle, int(form_id) & 0xFFFFFFFF)
    if result is None:
        return None
    return _record_summary_from_tuple(result)


def plugin_handle_has_record(handle: Any, form_id: int) -> bool:
    native_fn = _require_native_function("plugin_handle_has_record")
    return bool(native_fn(handle, int(form_id) & 0xFFFFFFFF))


def plugin_handle_record_payload_hash(handle: Any, form_id: int) -> str | None:
    native_fn = _require_native_function("plugin_handle_record_payload_hash")
    result = native_fn(handle, int(form_id) & 0xFFFFFFFF)
    return None if result is None else str(result)


def plugin_handle_extract_dialogue_text(handle: Any, format: str = "json") -> str:
    native_fn = _require_native_function("plugin_handle_extract_dialogue_text")
    return str(native_fn(int(handle), str(format)))


def plugin_handle_addon_node_summaries_by_index_id(handle: Any, index_id: int) -> list[RecordSummary]:
    native_fn = _require_native_function("plugin_handle_addon_node_summaries_by_index_id")
    return [_record_summary_from_tuple(item) for item in native_fn(handle, int(index_id))]


def plugin_handle_debug_section_loaded(handle: Any, section: str) -> bool:
    native_fn = _require_native_function("plugin_handle_debug_section_loaded")
    return bool(native_fn(handle, section))


def plugin_handle_force_build_records_section(handle: Any) -> None:
    native_fn = _require_native_function("plugin_handle_force_build_records_section")
    native_fn(handle)


def plugin_handle_force_build_refs_section(handle: Any) -> int:
    native_fn = _require_native_function("plugin_handle_force_build_refs_section")
    return int(native_fn(handle))


def plugin_handle_assets_by_kind(handle: Any, kind: str) -> list[tuple[str, str]]:
    native_fn = _require_native_function("plugin_handle_assets_by_kind")
    return [(str(form_key), str(path)) for form_key, path in native_fn(handle, kind)]


def plugin_handle_record_eid_index(handle: Any) -> dict[str, list[str]]:
    native_fn = _require_native_function("plugin_handle_record_eid_index")
    return {str(key): [str(value) for value in values] for key, values in dict(native_fn(handle)).items()}


def plugin_handle_record_index_rows(
    handle: Any,
    *,
    signatures: list[str] | None = None,
    form_keys: list[str] | None = None,
) -> list[tuple[str, str, str, int, int]]:
    native_fn = _require_native_function("plugin_handle_record_index_rows")
    return [
        (str(form_key), str(eid), str(signature), int(object_id), int(raw_form_id))
        for form_key, eid, signature, object_id, raw_form_id in native_fn(
            handle, signatures, form_keys
        )
    ]


def plugin_handle_local_object_ids(handle: Any) -> list[int]:
    native_fn = _require_native_function("plugin_handle_local_object_ids")
    return [int(value) for value in native_fn(handle)]


def plugin_handle_owned_object_ids(handle: Any) -> list[int]:
    native_fn = _require_native_function("plugin_handle_owned_object_ids")
    return [int(value) for value in native_fn(handle)]


def plugin_handle_record_form_ids(handle: Any, signatures: list[str] | None = None) -> list[int]:
    native_fn = _require_native_function("plugin_handle_record_form_ids")
    return [int(value) for value in native_fn(handle, signatures)]


def plugin_handle_validation_records(handle: Any) -> list[dict[str, Any]]:
    native_fn = _require_native_function("plugin_handle_validation_records")
    return [
        {
            "signature": str(signature),
            "form_id": int(form_id),
            "subrecords": [
                {"signature": str(subrecord_signature), "data": bytes(data)}
                for subrecord_signature, data in subrecords
            ],
        }
        for signature, form_id, subrecords in native_fn(int(handle))
    ]


def plugin_handle_search_records(
    handle: Any,
    pattern: str,
    *,
    mode: str = "glob",
    match_full: bool = False,
    read_full: bool = False,
    signatures: list[str] | None = None,
    case_sensitive: bool = False,
    limit: int | None = None,
) -> list[dict[str, Any]]:
    native_fn = _require_native_function("plugin_handle_search_records")
    rows = native_fn(
        handle,
        pattern,
        mode,
        match_full,
        read_full,
        signatures,
        case_sensitive,
        limit,
    )
    return [
        {
            "form_id": int(form_id),
            "signature": str(signature),
            "editor_id": editor_id,
            "full_name": full_name,
        }
        for form_id, signature, editor_id, full_name in rows
    ]


def plugin_handle_record_form_ids_with_subrecords(
    handle: Any, subrecord_signatures: list[str]
) -> list[int]:
    native_fn = _require_native_function("plugin_handle_record_form_ids_with_subrecords")
    return [int(value) for value in native_fn(handle, list(subrecord_signatures))]


def plugin_handle_record_subrecords(
    handle: Any, form_id: int
) -> list[tuple[str, bytes, str | None]] | None:
    native_fn = _require_native_function("plugin_handle_record_subrecords")
    result = native_fn(int(handle), int(form_id) & 0xFFFFFFFF)
    if result is None:
        return None
    return [(str(sig), bytes(data), semantic_type) for sig, data, semantic_type in result]


def plugin_handle_set_record_subrecords(
    handle: Any,
    form_id: int,
    subrecords: list[dict[str, bytes]] | list[tuple[str, bytes, str | None]],
) -> bool:
    native_fn = _require_native_function("plugin_handle_set_record_subrecords")
    payload: list[tuple[str, bytes, str | None]] = []
    for item in subrecords:
        if isinstance(item, dict):
            payload.append(
                (
                    str(item["signature"]),
                    bytes(item.get("data", b"")),
                    item.get("semantic_type"),
                )
            )
        else:
            sig, data, semantic_type = item
            payload.append((str(sig), bytes(data), semantic_type))
    return bool(native_fn(int(handle), int(form_id) & 0xFFFFFFFF, payload))


def plugin_handle_remove_formid_subrecords(
    handle: Any,
    record_signature: str,
    subrecord_signature: str,
    target_form_id: int,
    *,
    dry_run: bool = False,
) -> list[dict[str, Any]]:
    native_fn = _require_native_function("plugin_handle_remove_formid_subrecords")
    rows = native_fn(
        int(handle),
        str(record_signature),
        str(subrecord_signature),
        int(target_form_id) & 0xFFFFFFFF,
        bool(dry_run),
    )
    return [
        {
            "form_id": int(form_id),
            "editor_id": editor_id,
            "removed": int(removed),
        }
        for form_id, editor_id, removed in rows
    ]


def plugin_handle_repair_term_marker_parameters_from_source(
    target_handle: Any,
    source_handle: Any,
    *,
    dry_run: bool = False,
) -> list[dict[str, Any]]:
    native_fn = _require_native_function(
        "plugin_handle_repair_term_marker_parameters_from_source"
    )
    rows = native_fn(int(target_handle), int(source_handle), bool(dry_run))
    return [
        {
            "form_id": int(form_id),
            "editor_id": editor_id,
            "removed": int(removed),
            "inserted": int(inserted),
        }
        for form_id, editor_id, removed, inserted in rows
    ]


def plugin_handle_record_flags(handle: Any, form_id: int) -> int | None:
    native_fn = _require_native_function("plugin_handle_record_flags")
    result = native_fn(int(handle), int(form_id) & 0xFFFFFFFF)
    return None if result is None else int(result)


def plugin_handle_set_record_flags(handle: Any, form_id: int, flags: int) -> int | None:
    """Set a record's header flags in place; returns the previous flags, or None if absent."""
    native_fn = _require_native_function("plugin_handle_set_record_flags")
    result = native_fn(int(handle), int(form_id) & 0xFFFFFFFF, int(flags) & 0xFFFFFFFF)
    return None if result is None else int(result)


def plugin_handle_add_record_raw(
    handle: Any,
    signature: str,
    form_id: int,
    flags: int,
    version_control: int,
    form_version: int | None,
    version2: int | None,
    subrecords: list[tuple[str, bytes, str | None]],
) -> int:
    native_fn = _require_native_function("plugin_handle_add_record_raw")
    return int(
        native_fn(
            int(handle),
            str(signature),
            int(form_id) & 0xFFFFFFFF,
            int(flags) & 0xFFFFFFFF,
            int(version_control) & 0xFFFFFFFF,
            None if form_version is None else int(form_version) & 0xFFFF,
            None if version2 is None else int(version2) & 0xFFFF,
            [(str(sig), bytes(data), semantic_type) for sig, data, semantic_type in subrecords],
        )
    )


def plugin_handle_replace_authoring_record(handle: Any, json_text: str) -> str:
    native_fn = _require_native_function("plugin_handle_replace_authoring_record")
    return str(native_fn(int(handle), str(json_text)))


def plugin_handle_replace_projected_cell_authoring_record_values_at_locations(
    handle: Any,
    values: list[tuple[dict[str, Any], str]],
) -> int:
    native_fn = _require_native_function(
        "plugin_handle_replace_projected_cell_authoring_record_values_at_locations"
    )
    import json

    return int(native_fn(int(handle), json.dumps(values)))


def plugin_handle_read_authoring_record(handle: Any, form_id: int) -> str | None:
    native_fn = _require_native_function("plugin_handle_read_authoring_record")
    return native_fn(int(handle), int(form_id) & 0xFFFFFFFF)


def plugin_handle_used_master_indices(handle: Any) -> list[int]:
    native_fn = _require_native_function("plugin_handle_used_master_indices")
    return [int(value) for value in native_fn(handle)]


def plugin_handle_apply_object_id_mapping(
    handle: Any,
    old_high: int,
    new_high: int,
    object_id_map: dict[int, int],
) -> int:
    native_fn = _require_native_function("plugin_handle_apply_object_id_mapping")
    pairs = [
        (int(old_id) & 0x00FFFFFF, int(new_id) & 0x00FFFFFF)
        for old_id, new_id in object_id_map.items()
    ]
    return int(native_fn(handle, int(old_high) & 0xFF, int(new_high) & 0xFF, pairs))


def plugin_handle_null_refs_to_master(handle: Any, index: int) -> int:
    native_fn = _require_native_function("plugin_handle_null_refs_to_master")
    return int(native_fn(handle, int(index) & 0xFF))


def plugin_handle_undelete_and_disable_refs(handle: Any, signatures: list[str]) -> list[int]:
    native_fn = _require_native_function("plugin_handle_undelete_and_disable_refs")
    return [int(value) for value in native_fn(handle, [str(value) for value in signatures])]


def plugin_handle_copy_record(
    source_handle: Any,
    source_form_id: int,
    target_handle: Any,
    *,
    as_new: bool = False,
) -> int | None:
    native_fn = _require_native_function("plugin_handle_copy_record")
    result = native_fn(
        int(source_handle),
        int(source_form_id) & 0xFFFFFFFF,
        int(target_handle),
        bool(as_new),
    )
    return None if result is None else int(result)


def plugin_handle_merge_conflict_to_patch(
    target_handle: Any,
    signature: str,
    chain: list[tuple[int, str, int, int]],
) -> bool:
    native_fn = _require_native_function("plugin_handle_merge_conflict_to_patch")
    payload = [
        (
            int(handle),
            str(plugin_name),
            int(load_order_index),
            int(form_id) & 0xFFFFFFFF,
        )
        for handle, plugin_name, load_order_index, form_id in chain
    ]
    return bool(native_fn(int(target_handle), str(signature), payload))


def plugin_handle_collect_cell_slice_roots(
    handle: Any,
    *,
    worldspace_editor_id: str,
    min_x: int,
    min_y: int,
    max_x: int,
    max_y: int,
    include_worldspace_persistent_cell: bool,
    worker_count: int | None = None,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_collect_cell_slice_roots")
    return dict(
        native_fn(
            int(handle),
            worldspace_editor_id,
            int(min_x),
            int(min_y),
            int(max_x),
            int(max_y),
            bool(include_worldspace_persistent_cell),
            None if worker_count is None else int(worker_count),
        )
    )


def plugin_handle_collect_cell_children(handle: Any, cell_form_id: int) -> list[dict[str, Any]]:
    """Return every record under a cell's child group as {form_id, form_key, signature, group_type}.

    Works for interior and exterior cells. cell_form_id is the local object id
    (low 24 bits); the high byte is ignored.
    """
    native_fn = _require_native_function("plugin_handle_collect_cell_children")
    return [dict(entry) for entry in native_fn(int(handle), int(cell_form_id) & 0x00FFFFFF)]


def plugin_handle_remove_records(handle: Any, form_ids: list[int]) -> int:
    """Remove every record whose form id is in form_ids in a single tree pass.

    form_ids are full form ids (high byte = owner index). Returns the count removed.
    """
    native_fn = _require_native_function("plugin_handle_remove_records")
    return int(native_fn(int(handle), [int(f) & 0xFFFFFFFF for f in form_ids]))


def plugin_handle_delete_records(
    handle: Any,
    form_ids: list[int],
    *,
    cascade: bool = False,
) -> dict[str, int]:
    native_fn = _require_native_function("plugin_handle_delete_records")
    removed, records_modified, refs_removed = native_fn(
        int(handle),
        [int(form_id) & 0xFFFFFFFF for form_id in form_ids],
        bool(cascade),
    )
    return {
        "removed": int(removed),
        "records_modified": int(records_modified),
        "refs_removed": int(refs_removed),
    }


def plugin_handle_collect_worldspace_terrain_ids(
    handle: Any,
    *,
    worldspace_editor_id: str,
    min_x: int,
    min_y: int,
    max_x: int,
    max_y: int,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_collect_worldspace_terrain_ids")
    return dict(
        native_fn(
            int(handle),
            worldspace_editor_id,
            int(min_x),
            int(min_y),
            int(max_x),
            int(max_y),
        )
    )


def plugin_handle_insert_cell_slice_children(
    target_handle: Any,
    children_by_target_cell: dict[str, dict[str, list[str]]],
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_insert_cell_slice_children")
    import json

    return dict(native_fn(int(target_handle), json.dumps(children_by_target_cell)))


def plugin_handle_sync_cell_locations_from_lctn(handle: Any) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_sync_cell_locations_from_lctn")
    return dict(native_fn(int(handle)))


def plugin_handle_sync_cell_regions_from_source(
    source_handle: Any,
    target_handle: Any,
    *,
    source_worldspace_editor_id: str,
    target_worldspace_editor_id: str,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_sync_cell_regions_from_source")
    return dict(
        native_fn(
            int(source_handle),
            int(target_handle),
            source_worldspace_editor_id,
            target_worldspace_editor_id,
        )
    )


def plugin_handle_sync_cell_max_height_from_source(
    source_handle: Any,
    target_handle: Any,
    *,
    source_worldspace_editor_id: str,
    target_worldspace_editor_id: str,
) -> dict[str, Any]:
    native_fn = _require_native_function(
        "plugin_handle_sync_cell_max_height_from_source"
    )
    return dict(
        native_fn(
            int(source_handle),
            int(target_handle),
            source_worldspace_editor_id,
            target_worldspace_editor_id,
        )
    )


def plugin_handle_carry_worldspace_header_from_source(
    source_handle: Any,
    target_handle: Any,
    *,
    source_worldspace_editor_id: str,
    target_worldspace_editor_id: str,
) -> dict[str, Any]:
    native_fn = _require_native_function(
        "plugin_handle_carry_worldspace_header_from_source"
    )
    return dict(
        native_fn(
            int(source_handle),
            int(target_handle),
            source_worldspace_editor_id,
            target_worldspace_editor_id,
        )
    )


def plugin_handle_copy_cell_slice_children(
    source_handle: Any,
    target_handle: Any,
    children_by_target_cell: dict[str, dict[str, list[str]]],
    offset: tuple[float, float, float] = (0.0, 0.0, 0.0),
    form_key_map: dict[str, str] | None = None,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_copy_cell_slice_children")
    import json

    return dict(
        native_fn(
            int(source_handle),
            int(target_handle),
            json.dumps(children_by_target_cell),
            float(offset[0]),
            float(offset[1]),
            float(offset[2]),
            json.dumps(form_key_map or {}),
        )
    )


def plugin_handle_synthesize_worldspace_persistent_cell(
    source_handle: Any,
    target_handle: Any,
    worldspace_editor_id: str,
    offset: tuple[float, float, float] = (0.0, 0.0, 0.0),
    form_key_map: dict[str, str] | None = None,
) -> dict[str, Any]:
    native_fn = _require_native_function(
        "plugin_handle_synthesize_worldspace_persistent_cell"
    )
    import json

    return dict(
        native_fn(
            int(source_handle),
            int(target_handle),
            str(worldspace_editor_id),
            float(offset[0]),
            float(offset[1]),
            float(offset[2]),
            json.dumps(form_key_map or {}),
        )
    )


def plugin_handle_collect_worldspace_persistent_base_keys(
    source_handle: Any,
    worldspace_editor_id: str,
) -> list[str]:
    native_fn = _require_native_function(
        "plugin_handle_collect_worldspace_persistent_base_keys"
    )
    return [
        str(value)
        for value in native_fn(int(source_handle), str(worldspace_editor_id))
    ]


def plugin_handle_collect_worldspace_persistent_base_keys_in_bounds(
    source_handle: Any,
    worldspace_editor_id: str,
    *,
    min_x: int,
    min_y: int,
    max_x: int,
    max_y: int,
) -> list[str]:
    native_fn = _require_native_function(
        "plugin_handle_collect_worldspace_persistent_base_keys_in_bounds"
    )
    return [
        str(value)
        for value in native_fn(
            int(source_handle),
            str(worldspace_editor_id),
            int(min_x),
            int(min_y),
            int(max_x),
            int(max_y),
        )
    ]


def plugin_handle_collect_water_manifest(
    handle: Any,
    *,
    worldspace_editor_id: str,
    min_x: int,
    min_y: int,
    max_x: int,
    max_y: int,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_collect_water_manifest")
    return dict(
        native_fn(
            int(handle),
            worldspace_editor_id,
            int(min_x),
            int(min_y),
            int(max_x),
            int(max_y),
        )
    )


def rewrite_formkeys_batch(records: Any, mappings: Mapping[str, Any]) -> Any:
    """Native batch FK rewrite. Mirrors FormKeyMapper.rewrite_formkeys over a list of records."""
    native_fn = _optional_native_function("rewrite_formkeys_batch")
    if native_fn is None:
        return None
    return native_fn(records, mappings)


def find_stale_formkeys_batch(records: Any, source_plugins: Any) -> list[str] | None:
    """Native batch stale-FK scan. Returns a deduped sorted list of FK strings."""
    native_fn = _optional_native_function("find_stale_formkeys_batch")
    if native_fn is None:
        return None
    return list(native_fn(records, list(source_plugins)))


def replace_formkeys_batch(records: Any, replacements: Mapping[str, Any]) -> Any:
    """Native batch FK replace with null=remove semantics."""
    native_fn = _optional_native_function("replace_formkeys_batch")
    if native_fn is None:
        return None
    return native_fn(records, replacements)


def plugin_handle_get_referenced_form_keys(handle: Any, form_key: str) -> list[str]:
    native_fn = _require_native_function("plugin_handle_get_referenced_form_keys")
    return [str(value) for value in native_fn(handle, form_key)]


def plugin_handle_get_referenced_form_keys_by_subrecord(
    handle: Any,
    form_key: str,
    subrecord_sig: str,
) -> list[str]:
    native_fn = _require_native_function("plugin_handle_get_referenced_form_keys_by_subrecord")
    return [str(value) for value in native_fn(handle, form_key, subrecord_sig)]


def plugin_handle_get_referencing_form_keys(handle: Any, form_key: str) -> list[str]:
    native_fn = _require_native_function("plugin_handle_get_referencing_form_keys")
    return [str(value) for value in native_fn(handle, form_key)]


def plugin_handle_index_stats(handle: Any) -> dict[str, int]:
    native_fn = _require_native_function("plugin_handle_index_stats")
    return {str(key): int(value) for key, value in dict(native_fn(handle)).items()}


def schema_forge_collect_corpus(
    plugin_path: str,
    *,
    game: str,
    sample_cap: int,
) -> dict[str, Any]:
    native_fn = _require_native_function("schema_forge_collect_corpus_native")
    payload = native_fn(str(plugin_path), game, int(sample_cap))
    if isinstance(payload, dict):
        return payload
    plugin_name, total_records, total_subrecords, observation_rows = payload
    return {
        "plugin_name": plugin_name,
        "total_records": total_records,
        "total_subrecords": total_subrecords,
        "observations": [
            {
                "record_sig": row[0],
                "subrecord_sig": row[1],
                "occurrence_index": row[2],
                "count": row[3],
                "length_histogram": dict(row[4]),
                "byte_samples": [bytes(sample) for sample in row[5]],
            }
            for row in observation_rows
        ],
    }


def validate_plugin_deep(
    handle_id: int,
    load_order: list[tuple[int, str]],
) -> list[dict[str, Any]]:
    """Run the xEdit-parity error checker against a single plugin handle.

    Includes everything `validate_plugin` emits PLUS schema-driven subrecord
    ordering and unused-data warnings, with xEdit-format path strings.

    Returned dict keys: severity, category, plugin_handle, plugin_name,
    message, form_id, path, signature.
    """
    native_fn = _require_native_function("validate_plugin_deep_native")
    issues: list[dict[str, Any]] = []
    for item in native_fn(int(handle_id), list(load_order)):
        if isinstance(item, dict):
            issues.append(item)
        else:
            issues.append(
                {
                    "severity": item[0],
                    "category": item[1],
                    "plugin_handle": item[2],
                    "plugin_name": item[3],
                    "message": item[4],
                    "form_id": item[5],
                    "path": item[6],
                    "signature": item[7],
                }
            )
    return issues


def scan_conflicts(
    handles: list[tuple[int, str, int]],
    *,
    signatures: list[str] | None = None,
) -> list[dict[str, Any]]:
    """Run the Rust cross-plugin conflict scanner.

    `handles` is a list of (handle_id, plugin_name, load_order_index) tuples
    in any order — the scanner sorts each chain by load_order_index.

    Returns a list of dicts, each with: form_id, signature, editor_id,
    status ('override' or 'conflict'), mergeable, and chain (list of
    {plugin_handle, plugin_name, load_order_index, payload_hash}).
    """
    native_fn = _require_native_function("scan_conflicts_native")
    reports: list[dict[str, Any]] = []
    for item in native_fn(list(handles), signatures):
        if isinstance(item, dict):
            reports.append(item)
        else:
            reports.append(
                {
                    "form_id": item[0],
                    "signature": item[1],
                    "editor_id": item[2],
                    "status": item[3],
                    "mergeable": item[4],
                    "chain": [
                        {
                            "plugin_handle": chain_item[0],
                            "plugin_name": chain_item[1],
                            "load_order_index": chain_item[2],
                            "form_id": chain_item[3],
                            "payload_hash": chain_item[4],
                        }
                        for chain_item in item[5]
                    ],
                }
            )
    return reports


def voice_reference_build_index(
    db_path: str,
    game: str,
    data_dir: str,
    strings_dir: str,
    language: str,
    cache_key: str,
    plugin_paths: list[str],
    archive_paths: list[str],
    *,
    force: bool = False,
) -> tuple[str, int, bool]:
    native_fn = _require_native_function("voice_reference_build_index")
    db_path_out, line_count, reused = native_fn(
        db_path,
        game,
        data_dir,
        strings_dir,
        language,
        cache_key,
        plugin_paths,
        archive_paths,
        force,
    )
    return str(db_path_out), int(line_count), bool(reused)


def voice_reference_read_index(db_path: str) -> list[tuple[Any, ...]]:
    native_fn = _require_native_function("voice_reference_read_index")
    return [tuple(item) for item in native_fn(db_path)]


def plugin_handle_collect_assets(
    source_handles: list[Any],
    master_handles: list[Any],
    *,
    asset_kinds: list[str] | None = None,
    signatures: list[str] | None = None,
    form_keys: list[str] | None = None,
) -> list[dict[str, Any]]:
    native_fn = _require_native_function("plugin_handle_collect_assets")
    payload = native_fn(
        [int(handle) for handle in source_handles],
        [int(handle) for handle in master_handles],
        asset_kinds,
        signatures,
        form_keys,
    )
    assets: list[dict[str, Any]] = []
    for item in payload:
        if isinstance(item, dict):
            assets.append(item)
        else:
            assets.append(
                {
                    "asset_type": item[0],
                    "source_path": item[1],
                    "source_form_key": item[2],
                    "source_record_signature": item[3] if len(item) > 4 else "",
                    "source_subrecord_sig": item[4] if len(item) > 4 else item[3],
                }
            )
    return assets


def plugin_handle_walk_dependencies(
    source_handles: list[Any],
    master_handles: list[Any],
    root_form_keys: list[str],
    policy_json: str,
    *,
    strict_unresolved_masters: bool = True,
) -> dict[str, Any]:
    native_fn = _require_native_function("plugin_handle_walk_dependencies")
    payload = native_fn(
        [int(handle) for handle in source_handles],
        [int(handle) for handle in master_handles],
        root_form_keys,
        policy_json,
        strict_unresolved_masters,
    )
    if isinstance(payload, dict):
        return payload
    records, assets, errors, unresolved_form_keys, timing = payload
    return {
        "reached_records": [
            {
                "form_key": record[0],
                "signature": record[1],
                "defined_in": record[2],
                "master_plugin": record[3],
                "is_override": record[4],
                "eid": record[5],
                "walk_depth": record[6],
                "walker_pass": record[7],
                "added_by_form_key": record[8],
            }
            for record in records
        ],
        "assets": [
            {
                "asset_kind": asset[0],
                "source_path": asset[1],
                "source_form_key": asset[2],
                "source_record_signature": asset[3] if len(asset) > 6 else "",
                "source_subrecord_sig": asset[4] if len(asset) > 6 else asset[3],
                "walk_depth": asset[5] if len(asset) > 6 else asset[4],
                "walker_pass": asset[6] if len(asset) > 6 else asset[5],
            }
            for asset in assets
        ],
        "errors": list(errors),
        "unresolved_form_keys": list(unresolved_form_keys),
        "timing": dict(timing),
    }


def normalize_backend(backend: str | None) -> BackendName:
    if backend is None:
        return "auto"
    value = str(backend).strip().lower()
    if value not in {"auto", "native"}:
        raise ValueError(f"Unsupported ESP backend: {backend!r}")
    return value  # type: ignore[return-value]


def native_function_available(name: str) -> bool:
    return getattr(load_native_module(), name, None) is not None


def should_use_native_backend(backend: str | None, capability: str) -> bool:
    _ = normalize_backend(backend)
    if native_function_available(capability):
        return True
    _require_native_function(capability)
    return False


def validate_record_native(plugin: Any, record: Any) -> None:
    _require_native_function("validate_record_native")(plugin, record)


def load_plugin_native(
    plugin_path: str,
    *,
    game: str | None = None,
    jobs: int | None = None,
    strings_dir: str | None = None,
    language: str | None = None,
    eager_compressed: bool = True,
) -> Any:
    return _require_native_function("load_plugin_native")(
        plugin_path,
        game,
        jobs,
        strings_dir,
        language,
        eager_compressed,
    )


def save_plugin_native(
    plugin: Any,
    output_path: str,
    *,
    game: str | None = None,
) -> Any:
    return _require_native_function("save_plugin_native")(plugin, output_path, game)


def plugin_to_bytes_native(plugin: Any) -> bytes:
    return bytes(_require_native_function("plugin_to_bytes_native")(plugin))


def supported_games_native() -> list[str]:
    return list(_require_native_function("supported_games")())


def schema_json_for_game_native(game: str) -> str:
    return _require_native_function("schema_json_for_game")(game)


def export_authoring_dir_native(
    plugin_path: str,
    out_dir: str,
    *,
    game: str | None = None,
    format: str = "json",
    jobs: int | None = None,
    skip_signatures: list[str] | None = None,
) -> Any:
    return _require_native_function("export_authoring_dir_native")(
        plugin_path,
        out_dir,
        game,
        format,
        jobs,
        skip_signatures,
    )


def build_authoring_dir_streaming_native(
    source_dir: str,
    output_path: str,
    *,
    game: str | None = None,
    jobs: int | None = None,
    master_esm_paths: list[str] | None = None,
) -> Any:
    """Stream-build a .esp from an authoring dir without materializing the
    plugin tree. Peak memory scales with `jobs` × largest record being parsed
    in flight — not with the whole plugin size.

    `jobs` controls the parallel-decode thread count. None = global rayon pool
    (= num_cpus, fastest, peak RSS up to ~9 GB on Starfield-scale plugins).
    Pass jobs=1 for the serial path (~1 GB peak, ~30 min wall-clock on
    Starfield). jobs=4 is a reasonable middle ground.

    `master_esm_paths` are filesystem paths to the plugin's masters (the
    first one is scanned to derive the canonical top-level GRUP order so
    KYWD lands before COBJ etc. — survives Bethesda content updates without
    code changes). When None or unreadable, falls back to a hardcoded
    baseline per game.
    """
    return _require_native_function("build_authoring_dir_streaming_native")(
        source_dir,
        output_path,
        game,
        jobs,
        master_esm_paths,
    )


def validate_authoring(source_dir: str) -> dict[str, Any]:
    """Validate a mod's authoring YAML dir. Returns ``{errors, checked}``.

    Policy applied entirely in Rust:
      * internal FormKey refs must resolve to a record defined in the mod
      * external FormKey refs must target a plugin declared in
        ``plugin.yaml -> header.masters``
      * ESL plugins: no ``.esp`` masters; own FormIDs must be ``<= 0x000FFF``

    No external content (master ESM YAML, records DB, etc.) is consulted.
    """
    payload = _require_native_function("validate_authoring")(source_dir)
    if isinstance(payload, dict):
        return payload
    errors, checked = payload
    return {
        "errors": [
            {
                "file": error[0],
                "line": error[1],
                "formkey": error[2],
                "reason": error[3],
            }
            for error in errors
        ],
        "checked": checked,
    }


def export_plugin_text_native(
    plugin_path: str,
    output_path: str,
    *,
    game: str | None = None,
    mode: str = "lossless",
    format: str = "json",
) -> Any:
    return _require_native_function("export_plugin_text_native")(
        plugin_path,
        output_path,
        game,
        mode,
        format,
    )


def import_plugin_text_native(
    source_path: str,
    output_path: str,
    *,
    game: str | None = None,
    format: str | None = None,
) -> Any:
    return _require_native_function("import_plugin_text_native")(
        source_path,
        output_path,
        game,
        format,
    )
