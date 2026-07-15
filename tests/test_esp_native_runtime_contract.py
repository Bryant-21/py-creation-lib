from __future__ import annotations

from types import SimpleNamespace

import pytest

import creation_lib.esp.native_runtime as native_runtime

PLUGIN_SENTINEL = object()


@pytest.fixture(autouse=True)
def _reset_native_runtime_cache(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(native_runtime, "_NATIVE_MODULE", None)
    monkeypatch.setattr(native_runtime, "_NATIVE_IMPORT_ATTEMPTED", False)


def test_load_native_module_caches_missing_extension(monkeypatch: pytest.MonkeyPatch) -> None:
    attempts: list[str] = []

    def fake_import_module(name: str) -> None:
        attempts.append(name)
        raise ImportError("esp_authoring_core is missing")

    monkeypatch.setattr(native_runtime, "import_module", fake_import_module)

    with pytest.raises(RuntimeError, match=r"esp_authoring_core is required for creation_lib\.esp"):
        native_runtime.load_native_module()
    with pytest.raises(RuntimeError, match=r"esp_authoring_core is required for creation_lib\.esp"):
        native_runtime.load_native_module()
    assert attempts == ["creation_lib._native", "esp_authoring_core"]


def test_load_native_module_falls_back_to_umbrella_submodule(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    umbrella = SimpleNamespace(esp_authoring_core=SimpleNamespace(plugin_handle_load=object()))
    calls: list[str] = []

    def fake_import_module(name: str) -> object:
        calls.append(name)
        if name == "creation_lib._native":
            return umbrella
        if name == "esp_authoring_core":
            raise ImportError(name)
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", fake_import_module)

    module = native_runtime.load_native_module()

    assert module is umbrella.esp_authoring_core
    assert calls == ["creation_lib._native"]


def test_plugin_handle_load_forwards_to_function_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []
    handle = object()

    def plugin_handle_load(*args: object) -> object:
        calls.append(args)
        return handle

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_load=plugin_handle_load),
    )

    result = native_runtime.plugin_handle_load(
        "Example.esp",
        game="fo4",
        strings_dir="Strings",
        language="en",
        eager_compressed=False,
    )

    assert result is handle
    assert calls == [("Example.esp", "fo4", "Strings", "en", False)]


def test_plugin_handle_new_forwards_to_function_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []
    handle = object()

    def plugin_handle_new(*args: object) -> object:
        calls.append(args)
        return handle

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_new=plugin_handle_new),
    )

    result = native_runtime.plugin_handle_new("NewPlugin.esp", "fo4")

    assert result is handle
    assert calls == [("NewPlugin.esp", "fo4")]


def test_plugin_handle_from_bytes_forwards_to_function_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []
    handle = object()

    def plugin_handle_from_bytes(*args: object) -> object:
        calls.append(args)
        return handle

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_from_bytes=plugin_handle_from_bytes),
    )

    result = native_runtime.plugin_handle_from_bytes(
        b"TES4",
        plugin_name="Bytes.esp",
        game="fo4",
        auto_load_strings=True,
        strings_dir="Strings",
        language="en",
        file_path="Bytes.esp",
    )

    assert result is handle
    assert calls == [(b"TES4", "Bytes.esp", "fo4", True, "Strings", "en", "Bytes.esp")]


def test_plugin_handle_import_text_forwards_to_function_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []
    handle = object()

    def plugin_handle_import_text(*args: object) -> object:
        calls.append(args)
        return handle

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_import_text=plugin_handle_import_text),
    )

    result = native_runtime.plugin_handle_import_text("{}", format="json", game="fo4")

    assert result is handle
    assert calls == [("{}", "json", "fo4")]


def test_plugin_handle_close_forwards_to_function_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []

    def plugin_handle_close(*args: object) -> bool:
        calls.append(args)
        return True

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_close=plugin_handle_close),
    )

    result = native_runtime.plugin_handle_close(99)

    assert result is True
    assert calls == [(99,)]


def test_plugin_handle_get_reads_metadata_for_handle_ids(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    meta_calls: list[int] = []
    string_calls: list[tuple[int, object]] = []
    metadata = {
        "plugin_name": "HandleId.esp",
        "file_path": "C:/mods/HandleId.esp",
        "game": "fo4",
        "header_size": 24,
        "record_count": 3,
        "localized_default_language": "en",
        "header": {
            "version": 1.0,
            "num_records": 3,
            "next_object_id": 0x803,
            "author": "Native",
            "description": "Handle ID metadata",
            "masters": ["Fallout4.esm"],
            "master_sizes": [123],
            "overridden_forms": [],
            "flags": 0x80,
            "extra_subrecords": [],
            "version_control": 0,
            "raw_subrecords": [],
        },
    }
    strings = {
        "localized_strings_by_language": {"en": {1: "Name"}},
        "localized_string_table_types": {1: "strings"},
    }

    def plugin_handle_get_meta(handle_id: int) -> dict[str, object]:
        meta_calls.append(handle_id)
        return metadata

    def plugin_handle_get_strings(handle_id: int, language: object = None) -> dict[str, object]:
        string_calls.append((handle_id, language))
        return strings

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_get_meta=plugin_handle_get_meta,
            plugin_handle_get_strings=plugin_handle_get_strings,
        ),
    )

    assert native_runtime.plugin_handle_get(42, "plugin_name") == "HandleId.esp"
    assert native_runtime.plugin_handle_get(42, "record_count") == 3
    assert native_runtime.plugin_handle_get(42, "masters") == ["Fallout4.esm"]
    assert native_runtime.plugin_handle_get(42, "is_localized") is True
    header = native_runtime.plugin_handle_get(42, "header")
    assert header.author == "Native"
    assert header.next_object_id == 0x803
    assert native_runtime.plugin_handle_get(42, "localized_strings_by_language") == {"en": {1: "Name"}}
    assert native_runtime.plugin_handle_get(42, "localized_string_table_types") == {1: "strings"}
    assert meta_calls == [42, 42, 42, 42, 42]
    assert string_calls == [(42, None), (42, None)]


def test_plugin_from_native_handle_fetches_strings_lazily(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from creation_lib.esp.plugin import Plugin

    meta_calls: list[int] = []
    string_calls: list[tuple[int, object]] = []
    metadata = {
        "plugin_name": "LazyStrings.esp",
        "file_path": "C:/mods/LazyStrings.esp",
        "game": "fo4",
        "header_size": 24,
        "record_count": 0,
        "localized_default_language": "en",
        "header": {
            "version": 1.0,
            "num_records": 0,
            "next_object_id": 0x800,
            "author": "",
            "description": "",
            "masters": ["Fallout4.esm"],
            "master_sizes": [123],
            "overridden_forms": [],
            "flags": 0x80,
            "extra_subrecords": [],
            "version_control": 0,
            "raw_subrecords": [],
        },
    }

    def plugin_handle_get_meta(handle_id: int) -> dict[str, object]:
        meta_calls.append(handle_id)
        return metadata

    def plugin_handle_get_strings(handle_id: int, language: object = None) -> dict[str, object]:
        string_calls.append((handle_id, language))
        return {
            "localized_default_language": "en",
            "localized_strings_by_language": {"en": {1: "Name"}},
            "localized_string_table_types": {1: "strings"},
        }

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_get_meta=plugin_handle_get_meta,
            plugin_handle_get_strings=plugin_handle_get_strings,
        ),
    )

    plugin = Plugin._from_native_handle(42)

    assert plugin.plugin_name == "LazyStrings.esp"
    assert plugin.header.masters == ["Fallout4.esm"]
    assert string_calls == []
    assert plugin.localized_strings_by_language == {"en": {1: "Name"}}
    assert string_calls == [(42, None)]
    assert meta_calls == [42, 42, 42, 42, 42, 42]


def test_plugin_handle_call_routes_handle_ids_to_function_entrypoints(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []

    def plugin_handle_save(*args: object) -> None:
        calls.append(("save", *args))

    def plugin_handle_export_plugin_text(*args: object) -> str:
        calls.append(("export", *args))
        return '{"plugin":"HandleId.esp"}'

    def plugin_handle_export_record_text(*args: object) -> str:
        calls.append(("export_record", *args))
        return '{"record":"HandleId.Record"}'

    def plugin_handle_to_bytes(*args: object) -> bytes:
        calls.append(("to_bytes", *args))
        return b"TES4"

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_save=plugin_handle_save,
            plugin_handle_export_plugin_text=plugin_handle_export_plugin_text,
            plugin_handle_export_record_text=plugin_handle_export_record_text,
            plugin_handle_to_bytes=plugin_handle_to_bytes,
        ),
    )

    native_runtime.plugin_handle_call(77, "save", "out.esp")
    text = native_runtime.plugin_handle_call(77, "export_plugin_text", "lossless", "json")
    record_text = native_runtime.plugin_handle_call(77, "export_record_text", 0x01001234, "yaml")
    data = native_runtime.plugin_handle_call(77, "to_bytes")

    assert text == '{"plugin":"HandleId.esp"}'
    assert record_text == '{"record":"HandleId.Record"}'
    assert data == b"TES4"
    assert calls == [
        ("save", 77, "out.esp"),
        ("export", 77, "lossless", "json"),
        ("export_record", 77, 0x01001234, "yaml"),
        ("to_bytes", 77),
    ]


def test_plugin_handle_call_routes_formkey_index_apis(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []

    def plugin_handle_record_eid_index(*args: object) -> dict[str, list[str]]:
        calls.append(("eid_index", *args))
        return {"nativerecord": ["Native.esp:000800"]}

    def plugin_handle_get_referenced_form_keys(*args: object) -> list[str]:
        calls.append(("refs", *args))
        return ["Native.esp:000801"]

    def plugin_handle_index_stats(*args: object) -> dict[str, int]:
        calls.append(("stats", *args))
        return {"record_count": 1}

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_record_eid_index=plugin_handle_record_eid_index,
            plugin_handle_get_referenced_form_keys=plugin_handle_get_referenced_form_keys,
            plugin_handle_index_stats=plugin_handle_index_stats,
        ),
    )

    eid_index = native_runtime.plugin_handle_call(88, "record_eid_index")
    refs = native_runtime.plugin_handle_call(
        88,
        "get_referenced_form_keys",
        "Native.esp:000800",
    )
    stats = native_runtime.plugin_handle_call(88, "index_stats")

    assert eid_index == {"nativerecord": ["Native.esp:000800"]}
    assert refs == ["Native.esp:000801"]
    assert stats == {"record_count": 1}
    assert calls == [
        ("eid_index", 88),
        ("refs", 88, "Native.esp:000800"),
        ("stats", 88),
    ]


def test_plugin_handle_record_summary_uses_handle_id_entrypoint(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[int, int]] = []

    def plugin_handle_record_summary(handle_id: int, form_id: int) -> tuple[int, str, str]:
        calls.append((handle_id, form_id))
        return (0xFF000800, "MISC", "RootItem")

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_record_summary=plugin_handle_record_summary),
    )

    item = native_runtime.plugin_handle_record_summary(99, 0xFF000800)

    assert calls == [(99, 0xFF000800)]
    assert item == native_runtime.RecordSummary(0xFF000800, "MISC", "RootItem")


@pytest.mark.parametrize(
    ("backend", "expected"),
    [
        (None, "auto"),
        ("auto", "auto"),
        ("native", "native"),
        (" Native ", "native"),
    ],
)
def test_normalize_backend_accepts_supported_values(backend: str | None, expected: str) -> None:
    assert native_runtime.normalize_backend(backend) == expected


def test_normalize_backend_rejects_unknown_values() -> None:
    with pytest.raises(ValueError, match="Unsupported ESP backend"):
        native_runtime.normalize_backend("rust-only")


def test_should_use_native_backend_uses_capability_when_available(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(native_runtime, "native_function_available", lambda name: name == "load_plugin_native")
    assert native_runtime.should_use_native_backend("auto", "load_plugin_native") is True


def test_should_use_native_backend_raises_for_auto_without_module(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(native_runtime, "native_function_available", lambda name: False)
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: None)

    with pytest.raises(RuntimeError, match=r"missing required function load_plugin_native\(\)"):
        native_runtime.should_use_native_backend("auto", "load_plugin_native")


def test_should_use_native_backend_raises_for_native_without_capability(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(native_runtime, "native_function_available", lambda name: False)
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: SimpleNamespace())

    with pytest.raises(RuntimeError, match=r"missing required function load_plugin_native\(\)"):
        native_runtime.should_use_native_backend("native", "load_plugin_native")


@pytest.mark.parametrize(
    ("wrapper_name", "call_args", "call_kwargs"),
    [
        (
            "load_plugin_native",
            ("Example.esp",),
            {"game": "fo4", "jobs": 2, "strings_dir": "Strings", "language": "en"},
        ),
        (
            "save_plugin_native",
            (PLUGIN_SENTINEL, "Example.esp"),
            {"game": "fo4"},
        ),
        ("supported_games_native", (), {}),
        ("schema_json_for_game_native", ("fo4",), {}),
        (
            "export_authoring_dir_native",
            ("Example.esp", "authoring"),
            {"game": "fo4", "format": "yaml", "jobs": 4},
        ),
        (
            "build_authoring_dir_streaming_native",
            ("authoring", "Example.esp"),
            {"game": "fo4", "jobs": 2},
        ),
        (
            "export_plugin_text_native",
            ("Example.esp", "Example.yaml"),
            {"game": "fo4", "mode": "authoring", "format": "yaml"},
        ),
        (
            "import_plugin_text_native",
            ("Example.yaml", "Example.esp"),
            {"game": "fo4", "format": "yaml"},
        ),
    ],
)
def test_native_wrappers_raise_runtime_error_when_extension_is_missing(
    wrapper_name: str,
    call_args: tuple[object, ...],
    call_kwargs: dict[str, object],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: None)

    wrapper = getattr(native_runtime, wrapper_name)

    with pytest.raises(RuntimeError, match="missing required"):
        wrapper(*call_args, **call_kwargs)


@pytest.mark.parametrize(
    ("wrapper_name", "call_args", "call_kwargs"),
    [
        (
            "load_plugin_native",
            ("Example.esp",),
            {"game": "fo4", "jobs": 2, "strings_dir": "Strings", "language": "en"},
        ),
        (
            "save_plugin_native",
            (PLUGIN_SENTINEL, "Example.esp"),
            {"game": "fo4"},
        ),
        ("supported_games_native", (), {}),
        ("schema_json_for_game_native", ("fo4",), {}),
        (
            "export_authoring_dir_native",
            ("Example.esp", "authoring"),
            {"game": "fo4", "format": "yaml", "jobs": 4},
        ),
        (
            "build_authoring_dir_streaming_native",
            ("authoring", "Example.esp"),
            {"game": "fo4", "jobs": 2},
        ),
        (
            "export_plugin_text_native",
            ("Example.esp", "Example.yaml"),
            {"game": "fo4", "mode": "authoring", "format": "yaml"},
        ),
        (
            "import_plugin_text_native",
            ("Example.yaml", "Example.esp"),
            {"game": "fo4", "format": "yaml"},
        ),
    ],
)
def test_native_wrappers_raise_runtime_error_for_missing_entrypoints(
    wrapper_name: str,
    call_args: tuple[object, ...],
    call_kwargs: dict[str, object],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(native_runtime, "load_native_module", lambda: SimpleNamespace())

    wrapper = getattr(native_runtime, wrapper_name)

    with pytest.raises(RuntimeError, match="missing required"):
        wrapper(*call_args, **call_kwargs)


@pytest.mark.parametrize(
    ("wrapper_name", "native_attr", "call_args", "call_kwargs", "expected_native_args"),
    [
        (
            "load_plugin_native",
            "load_plugin_native",
            ("Example.esp",),
            {"game": "fo4", "jobs": 2, "strings_dir": "Strings", "language": "en"},
            ("Example.esp", "fo4", 2, "Strings", "en", True),
        ),
        (
            "save_plugin_native",
            "save_plugin_native",
            (PLUGIN_SENTINEL, "Example.esp"),
            {"game": "fo4"},
            (PLUGIN_SENTINEL, "Example.esp", "fo4"),
        ),
        ("supported_games_native", "supported_games", (), {}, ()),
        ("schema_json_for_game_native", "schema_json_for_game", ("fo4",), {}, ("fo4",)),
        (
            "export_authoring_dir_native",
            "export_authoring_dir_native",
            ("Example.esp", "authoring"),
            {"game": "fo4", "format": "yaml", "jobs": 4},
            ("Example.esp", "authoring", "fo4", "yaml", 4, None),
        ),
        (
            "build_authoring_dir_streaming_native",
            "build_authoring_dir_streaming_native",
            ("authoring", "Example.esp"),
            {"game": "fo4", "jobs": 2},
            ("authoring", "Example.esp", "fo4", 2, None),
        ),
        (
            "export_plugin_text_native",
            "export_plugin_text_native",
            ("Example.esp", "Example.yaml"),
            {"game": "fo4", "mode": "authoring", "format": "yaml"},
            ("Example.esp", "Example.yaml", "fo4", "authoring", "yaml"),
        ),
        (
            "import_plugin_text_native",
            "import_plugin_text_native",
            ("Example.yaml", "Example.esp"),
            {"game": "fo4", "format": "yaml"},
            ("Example.yaml", "Example.esp", "fo4", "yaml"),
        ),
    ],
)
def test_native_wrappers_forward_the_coarse_runtime_contract(
    wrapper_name: str,
    native_attr: str,
    call_args: tuple[object, ...],
    call_kwargs: dict[str, object],
    expected_native_args: tuple[object, ...],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[tuple[object, ...]] = []

    def recorder(*args: object) -> object:
        calls.append(args)
        if wrapper_name == "supported_games_native":
            return ["fo4", "fo76"]
        return {"wrapper": wrapper_name, "arg_count": len(args)}

    monkeypatch.setattr(
        native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(**{native_attr: recorder}),
    )

    wrapper = getattr(native_runtime, wrapper_name)
    result = wrapper(*call_args, **call_kwargs)

    if wrapper_name == "supported_games_native":
        assert result == ["fo4", "fo76"]
    else:
        assert result == {"wrapper": wrapper_name, "arg_count": len(expected_native_args)}
    assert calls == [expected_native_args]


def test_removed_authoring_dict_entrypoints_are_not_wrapped() -> None:
    removed = (
        "decode_" + "subrecord_native",
        "compact_" + "authoring_subrecord_native",
    )
    for name in removed:
        assert not hasattr(native_runtime, name)
