from __future__ import annotations

import gc
import json
from pathlib import Path
import struct
from types import SimpleNamespace
from typing import Any

import pytest
import yaml

from creation_lib.esp import Plugin, build_authoring_dir, export_authoring_dir, export_json, import_json
import creation_lib.esp.api as esp_api
import creation_lib.esp.plugin as plugin_module
from creation_lib.esp.native_runtime import load_native_module


pytestmark = pytest.mark.skipif(load_native_module() is None, reason="esp_authoring_core is not installed")


def _rebuild_via_streaming(authoring_dir: Path, *, game: str = "fo4", jobs: int | None = None) -> Plugin:
    """Stream-build a .esp from `authoring_dir` and load it as a Plugin."""
    rebuilt_esp = authoring_dir.parent / f"{authoring_dir.name}.rebuilt.esp"
    build_authoring_dir(authoring_dir, rebuilt_esp, game=game, jobs=jobs)
    return Plugin.load(rebuilt_esp, game=game, backend="native")


def _native_handle_metadata(
    *,
    plugin_name: str,
    file_path: str | None,
    game: str | None = "fo4",
    record_count: int = 0,
    header_flags: int = 0,
) -> dict[str, Any]:
    return {
        "plugin_name": plugin_name,
        "file_path": file_path,
        "game": game,
        "header_size": 24,
        "record_count": record_count,
        "localized_default_language": "en",
        "localized_strings_by_language": {},
        "localized_string_table_types": {},
        "header": {
            "version": 1.0,
            "num_records": record_count,
            "next_object_id": 0x800,
            "author": "",
            "description": "",
            "masters": [],
            "master_sizes": [],
            "overridden_forms": [],
            "flags": header_flags,
            "extra_subrecords": [],
            "version_control": 0,
            "raw_subrecords": [],
        },
    }


def test_native_backend_load_defers_python_materialization(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    handle = 1001
    metadata = _native_handle_metadata(
        plugin_name="Deferred.esp",
        file_path="C:/fake/Deferred.esp",
        record_count=1,
    )

    monkeypatch.setattr(
        plugin_module._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_metadata=lambda handle_id: metadata,
        ),
    )
    monkeypatch.setattr(plugin_module._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_load", lambda *_args, **_kwargs: handle)

    loaded = Plugin.load("Deferred.esp", game="fo4", backend="native")

    assert loaded._rust_handle == handle
    assert loaded.file_path == Path("C:/fake/Deferred.esp")
    assert loaded.record_count == 1
    assert len(loaded) == 1


def _release_via_close() -> Plugin:
    plugin = Plugin(plugin_name="CloseMe.esp", game="fo4")
    plugin._rust_handle = 4242
    assert plugin.close() is True
    assert plugin._rust_handle is None
    return plugin


def _release_via_del() -> None:
    plugin = Plugin(plugin_name="CloseMe.esp", game="fo4")
    plugin._rust_handle = 4242
    del plugin
    gc.collect()


def _release_via_context_manager() -> Plugin:
    plugin = Plugin(plugin_name="CloseMe.esp", game="fo4")
    plugin._rust_handle = 4242
    with plugin as bound:
        assert bound is plugin
    assert plugin._rust_handle is None
    return plugin


@pytest.mark.parametrize(
    "trigger",
    [_release_via_close, _release_via_del, _release_via_context_manager],
    ids=["close()", "__del__", "context-manager-exit"],
)
def test_plugin_release_paths_close_the_native_handle(monkeypatch: pytest.MonkeyPatch, trigger) -> None:
    gc.collect()
    closed_handles: list[int] = []

    def plugin_handle_close(handle_id: int) -> bool:
        closed_handles.append(handle_id)
        return True

    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_close", plugin_handle_close)

    trigger()

    assert closed_handles == [4242]


def test_plugin_save_can_close_native_handle(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    saved_paths: list[str] = []
    closed_handles: list[int] = []

    def plugin_handle_save(handle_id: int, output_path: str) -> None:
        saved_paths.append(output_path)

    def plugin_handle_close(handle_id: int) -> bool:
        closed_handles.append(handle_id)
        return True

    monkeypatch.setattr(plugin_module._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(
        plugin_module._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(plugin_handle_save=plugin_handle_save, plugin_handle_close=plugin_handle_close),
    )

    plugin = Plugin(plugin_name="SaveClose.esp", game="fo4")
    plugin._rust_handle = 4343

    target = tmp_path / "SaveClose.esp"
    result = plugin.save(target, backend="native", close_after_save=True)

    assert result == target
    assert plugin.file_path == target
    assert plugin._rust_handle is None
    assert saved_paths == [str(target)]
    assert closed_handles == [4343]


def test_native_backend_records_property_uses_summary_query(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    handle = 1003
    form_id_calls: list[int] = []
    summary_calls: list[tuple[int, int]] = []
    closed_handles: list[int] = []
    metadata = _native_handle_metadata(
        plugin_name="DeferredRecords.esp",
        file_path="C:/fake/DeferredRecords.esp",
        record_count=1,
    )

    def plugin_handle_record_form_ids(handle_id: int):
        form_id_calls.append(handle_id)
        return [0xFF000800]

    def plugin_handle_record_summary(handle_id: int, form_id: int):
        summary_calls.append((handle_id, form_id))
        return plugin_module._native_runtime.RecordSummary(form_id, "MISC", "DeferredRecordsItem")

    def plugin_handle_close(handle_id: int) -> bool:
        closed_handles.append(handle_id)
        return True

    monkeypatch.setattr(
        plugin_module._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_metadata=lambda handle_id: metadata,
            plugin_handle_close=plugin_handle_close,
        ),
    )
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_record_form_ids", plugin_handle_record_form_ids)
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_record_summary", plugin_handle_record_summary)
    monkeypatch.setattr(plugin_module._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_load", lambda *_args, **_kwargs: handle)

    loaded = Plugin.load("DeferredRecords.esp", game="fo4", backend="native")

    assert loaded.records == [plugin_module._native_runtime.RecordSummary(0xFF000800, "MISC", "DeferredRecordsItem")]
    assert form_id_calls == [handle]
    assert summary_calls == [(handle, 0xFF000800)]
    assert closed_handles == []
    assert loaded._rust_handle == handle


def test_native_backend_export_json_uses_handle_directly_for_lazy_plugins(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    handle = 1004
    export_calls: list[tuple[int, str, str]] = []
    metadata = _native_handle_metadata(
        plugin_name="LazyText.esp",
        file_path="C:/fake/LazyText.esp",
        record_count=0,
    )

    def plugin_handle_export_plugin_text(handle_id: int, mode: str = "lossless", format: str = "json") -> str:
        export_calls.append((handle_id, mode, format))
        return '{"plugin":"LazyText.esp","game":"fo4","mode":"lossless","items":[]}'

    monkeypatch.setattr(
        plugin_module._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_metadata=lambda handle_id: metadata,
            plugin_handle_export_plugin_text=plugin_handle_export_plugin_text,
        ),
    )
    monkeypatch.setattr(plugin_module._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_load", lambda *_args, **_kwargs: handle)

    loaded = Plugin.load("LazyText.esp", game="fo4", backend="native")
    text = export_json(loaded, backend="native")

    assert text == '{"plugin":"LazyText.esp","game":"fo4","mode":"lossless","items":[]}'
    assert export_calls == [(handle, "lossless", "json")]


def test_native_backend_plugin_save_uses_handle_directly_for_lazy_plugins(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    handle = 1005
    saved_paths: list[tuple[int, str]] = []
    metadata = _native_handle_metadata(
        plugin_name="LazySave.esp",
        file_path="C:/fake/LazySave.esp",
        record_count=0,
    )

    def plugin_handle_save(handle_id: int, output_path: str) -> None:
        saved_paths.append((handle_id, output_path))

    monkeypatch.setattr(
        plugin_module._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_metadata=lambda handle_id: metadata,
            plugin_handle_save=plugin_handle_save,
        ),
    )
    monkeypatch.setattr(plugin_module._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(plugin_module._native_runtime, "plugin_handle_load", lambda *_args, **_kwargs: handle)

    def _boom(*_args, **_kwargs):
        raise AssertionError("lazy save should not use the Python plugin save path")

    monkeypatch.setattr(plugin_module._native_runtime, "save_plugin_native", _boom)

    loaded = Plugin.load("LazySave.esp", game="fo4", backend="native")
    target = tmp_path / "LazySave.out.esp"
    loaded.save(target, backend="native")

    assert saved_paths == [(handle, str(target))]
    assert loaded.file_path == target
    assert loaded.plugin_name == target.name
    assert loaded._rust_handle == handle


def test_native_backend_lookup_and_reference_graph_queries_stay_native_backed(tmp_path: Path) -> None:
    plugin_path = tmp_path / "NativeLazyLookups.esp"
    plugin = Plugin.new(plugin_path.name, game="fo4")

    misc = plugin.new_record("MISC")
    misc.editor_id = "NativeLazyMisc"
    misc.full_name = "Native Lazy Misc"
    plugin.add_record(misc)

    addn = plugin.new_record("ADDN")
    addn.editor_id = "NativeLazyAddon"
    addn.add_subrecord("DATA", (77).to_bytes(4, "little"))
    plugin.add_record(addn)

    target = plugin.new_record("MISC")
    target.editor_id = "NativeTarget"
    plugin.add_record(target)

    source = plugin.new_record("MISC")
    source.editor_id = "NativeSource"
    source.add_subrecord("YNAM", int(target.form_id).to_bytes(4, "little"))
    plugin.add_record(source)

    caller = plugin.new_record("MISC")
    caller.editor_id = "NativeCaller"
    caller.add_subrecord("YNAM", int(source.form_id).to_bytes(4, "little"))
    plugin.add_record(caller)

    plugin.save(plugin_path, backend="native")
    loaded = Plugin.load(plugin_path, game="fo4", backend="native")

    found_by_form = loaded.get_record_by_form_id(misc.form_id)
    addon_records = loaded.get_addon_nodes_by_index_id(77)
    addon_record = loaded.get_addon_node_by_index_id(77)
    referenced = loaded.get_referenced_form_ids(source.form_id)
    referencing = loaded.get_referencing_form_ids(source.form_id)
    chain = loaded.get_form_id_chain(source.form_id)

    assert found_by_form is not None
    assert found_by_form.editor_id == "NativeLazyMisc"
    assert len(addon_records) == 1
    assert addon_records[0].editor_id == "NativeLazyAddon"
    assert addon_record is not None
    assert addon_record.editor_id == "NativeLazyAddon"
    assert referenced == [target.object_id]
    assert referencing == [caller.object_id]
    assert chain == [source.object_id, target.object_id, caller.object_id]
    assert loaded._rust_handle is not None


def test_native_backend_import_json_uses_handle_import_text(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    handle = 1007
    calls: list[tuple[str, str, str | None]] = []
    metadata = _native_handle_metadata(
        plugin_name="Imported.esm",
        file_path="C:/fake/imported.esp",
        record_count=0,
    )

    def fake_import_text(text: str, format: str = "json", game: str | None = None):
        calls.append((text, format, game))
        return handle

    def plugin_handle_set_logical_identity(
        handle_id: int,
        plugin_name: str,
        game: str | None = None,
        file_path: str | None = None,
    ) -> None:
        metadata["plugin_name"] = plugin_name
        metadata["game"] = game
        metadata["file_path"] = file_path

    monkeypatch.setattr(
        esp_api._native_runtime,
        "load_native_module",
        lambda: SimpleNamespace(
            plugin_handle_metadata=lambda handle_id: metadata,
            plugin_handle_set_logical_identity=plugin_handle_set_logical_identity,
        ),
    )
    monkeypatch.setattr(esp_api._native_runtime, "should_use_native_backend", lambda *_args, **_kwargs: True)
    monkeypatch.setattr(esp_api._native_runtime, "plugin_handle_import_text", fake_import_text, raising=False)

    payload = '{"plugin":"Imported.esm","game":"fo4","mode":"lossless","header":{},"items":[]}'
    imported = esp_api.import_json(payload, backend="native")

    assert calls == [(payload, "json", None)]
    assert imported._rust_handle == handle
    assert imported.plugin_name == "Imported.esm"
    assert imported.file_path is None


def test_native_backend_build_authoring_dir_recomputes_num_records(
    tmp_path: Path,
) -> None:
    authoring_dir = tmp_path / "zero_count_authoring_build"
    output_path = tmp_path / "ZeroCountBuilt.esp"
    (authoring_dir / "records" / "MISC").mkdir(parents=True)
    (authoring_dir / "records" / "KYWD").mkdir(parents=True)
    (authoring_dir / "plugin.json").write_text(
        json.dumps(
            {
                "plugin": "ZeroCount.esp",
                "game": "fo4",
                "header": {
                    "version": 1.0,
                    "num_records": 99,
                    "next_object_id": "000802",
                    "author": "",
                    "description": "",
                    "masters": [],
                    "master_sizes": [],
                    "overridden_forms": [],
                    "flags": "00000000",
                    "version_control": 0,
                    "extra_subrecords": [],
                },
            }
        ),
        encoding="utf-8",
    )
    (authoring_dir / "records" / "MISC" / "ZeroCountMisc.json").write_text(
        json.dumps(
            {
                "form_id": "000800:ZeroCount.esp",
                "subrecords": [
                    {"signature": "EDID", "data_hex": "5A65726F436F756E744D69736300"},
                ],
            }
        ),
        encoding="utf-8",
    )
    (authoring_dir / "records" / "KYWD" / "ZeroCountKeyword.json").write_text(
        json.dumps(
            {
                "form_id": "000801:ZeroCount.esp",
                "subrecords": [
                    {"signature": "EDID", "data_hex": "5A65726F436F756E744B6579776F726400"},
                ],
            }
        ),
        encoding="utf-8",
    )

    esp_api._native_runtime.build_authoring_dir_streaming_native(
        str(authoring_dir),
        str(output_path),
        game="fo4",
        jobs=8,
    )

    loaded = Plugin.load(output_path, game="fo4", backend="native")
    built = output_path.read_bytes()
    assert built[24:28] == b"HEDR"
    # HEDR.NumRecords = 2 records + 2 top-level groups (MISC, KYWD).
    assert struct.unpack_from("<I", built, 34)[0] == 4
    assert loaded.record_count == 2
    assert loaded.eid_index()["zerocountmisc"] == ["ZeroCountBuilt.esp:000800"]
    assert loaded.eid_index()["zerocountkeyword"] == ["ZeroCountBuilt.esp:000801"]


def test_native_backend_load_and_save_roundtrip(tmp_path: Path) -> None:
    plugin_path = tmp_path / "NativeRoundtrip.esp"
    plugin = Plugin.new(plugin_path.name, game="fo4")
    plugin.header.author = "Native Test"
    record = plugin.new_record("MISC")
    record.editor_id = "NativeRecord"
    record.full_name = "Native Name"
    plugin.add_record(record)
    plugin.save(plugin_path)

    loaded = Plugin.load(plugin_path, game="fo4", backend="native")
    assert loaded.eid_index()["nativerecord"] == ["NativeRoundtrip.esp:000800"]
    assert loaded.header.author == "Native Test"

    rebuilt_path = tmp_path / "NativeRoundtrip.rebuilt.esp"
    loaded.save(rebuilt_path, backend="native")
    rebuilt = Plugin.load(rebuilt_path, game="fo4")
    assert rebuilt.to_bytes() == plugin.to_bytes()


def test_native_backend_preserves_compressed_records(tmp_path: Path) -> None:
    plugin_path = tmp_path / "NativeCompressed.esp"
    plugin = Plugin.new(plugin_path.name, game="fo4")
    record = plugin.new_record("MISC")
    record.editor_id = "NativeCompressedRecord"
    record.compressed = True
    record.add_subrecord("DATA", bytes(range(128)) * 4)
    plugin.add_record(record)
    plugin.save(plugin_path, backend="native")

    loaded = Plugin.load(plugin_path, game="fo4", backend="native")
    assert loaded.get_record_by_form_id(0x000800) is not None
    assert loaded.to_bytes() == plugin.to_bytes()


def test_native_backend_authoring_dir_roundtrip(tmp_path: Path) -> None:
    plugin = Plugin.new("NativeAuthoring.esp", game="fo4")
    record = plugin.new_record("DOBJ")
    record.editor_id = "NativeAuthoringRecord"
    record.add_subrecord("DNAM", b"\x41\x41\x41\x43\x45\x23\x01\x00")
    plugin.add_record(record)

    authoring_dir = tmp_path / "authoring"
    export_authoring_dir(plugin, authoring_dir, format="yaml", backend="native", jobs=8)
    rebuilt = _rebuild_via_streaming(authoring_dir, jobs=8)

    assert rebuilt.to_bytes() == plugin.to_bytes()


def test_native_backend_groups_object_templates_with_schema_labels(tmp_path: Path) -> None:
    plugin = Plugin.new("NativeObjectTemplates.esp", game="fo4")
    record = plugin.new_record("WEAP")
    record.editor_id = "NativeObjectTemplateWeapon"
    record.add_subrecord("OBTE", struct.pack("<I", 1))
    record.add_subrecord(
        "OBTS",
        bytes.fromhex(
            "040000000000000000000000FFFF01000000"
            "619C0D000000015D9C0D00000001609C0D000000015F9C0D00000001"
        ),
    )
    plugin.add_record(record)

    json_dir = tmp_path / "authoring-json"
    export_authoring_dir(plugin, json_dir, format="json", backend="native")
    json_payload = json.loads(next((json_dir / "records" / "WEAP").glob("*.json")).read_text())
    json_keys = [next(iter(field)) for field in json_payload["fields"]]

    assert json_payload["eid"] == "NativeObjectTemplateWeapon"
    assert "Editor ID" not in json_keys
    assert "EDID" not in json_keys
    assert "ObjectTemplates" in json_keys
    assert "OBTS" not in json_keys
    assert "Object Mod Template Item" not in json_keys
    json_templates = next(field["ObjectTemplates"] for field in json_payload["fields"] if "ObjectTemplates" in field)
    assert json_templates[0]["Default"] is True
    assert len(json_templates[0]["Includes"]) == 4
    assert "Mod" in json_templates[0]["Includes"][0]

    yaml_dir = tmp_path / "authoring-yaml"
    export_authoring_dir(plugin, yaml_dir, format="yaml", backend="native")
    yaml_payload = yaml.safe_load(next((yaml_dir / "records" / "WEAP").glob("*.yaml")).read_text())
    yaml_keys = [next(iter(field)) for field in yaml_payload["fields"]]

    assert yaml_payload["eid"] == "NativeObjectTemplateWeapon"
    assert "Editor ID" not in yaml_keys
    assert "EDID" not in yaml_keys
    assert "ObjectTemplates" in yaml_keys
    assert "OBTS" not in yaml_keys
    yaml_templates = next(field["ObjectTemplates"] for field in yaml_payload["fields"] if "ObjectTemplates" in field)
    assert yaml_templates[0]["Default"] is True
    assert len(yaml_templates[0]["Includes"]) == 4

    assert _rebuild_via_streaming(json_dir).to_bytes() == plugin.to_bytes()
    assert _rebuild_via_streaming(yaml_dir).to_bytes() == plugin.to_bytes()


def test_native_backend_authoring_dir_paths_do_not_call_python_authoring_dir(
    tmp_path: Path,
) -> None:
    assert not hasattr(esp_api, "_python_export_authoring_dir")
    assert not hasattr(esp_api, "_python_import_authoring_dir")

    plugin = Plugin.new("NativeNoPythonAuthoring.esp", game="fo4")
    record = plugin.new_record("DOBJ")
    record.editor_id = "NativeNoPythonAuthoringRecord"
    record.add_subrecord("DNAM", b"\x41\x41\x41\x43\x45\x23\x01\x00")
    plugin.add_record(record)

    authoring_dir = tmp_path / "authoring"
    export_authoring_dir(plugin, authoring_dir, format="json", backend="native")
    rebuilt = _rebuild_via_streaming(authoring_dir)

    assert rebuilt.to_bytes() == plugin.to_bytes()

    with pytest.raises(ValueError, match="Unsupported ESP backend"):
        export_authoring_dir(plugin, tmp_path / "python-backend", format="json", backend="python")


def test_native_backend_wrapper_materialization_does_not_call_plugin_to_bytes() -> None:
    original_to_bytes = Plugin.to_bytes

    def _boom(self):
        raise AssertionError("Plugin.to_bytes() should not be used by the native wrapper")

    Plugin.to_bytes = _boom
    try:
        plugin = Plugin.new("NativeWrapperSave.esp", game="fo4")
        record = plugin.new_record("MISC")
        record.editor_id = "NativeWrapperSaveRecord"
        record.full_name = "Native Wrapper Save"
        plugin.add_record(record)

        text = export_json(plugin, mode="lossless", backend="native")
        rebuilt = import_json(text, backend="native")
    finally:
        Plugin.to_bytes = original_to_bytes

    rebuilt_text = export_json(rebuilt, mode="lossless", backend="native")
    assert "NativeWrapperSaveRecord" in rebuilt_text
    assert "Native Wrapper Save" in rebuilt_text
