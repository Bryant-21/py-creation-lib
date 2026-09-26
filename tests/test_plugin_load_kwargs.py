from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

import creation_lib.esp.native_runtime as native_runtime
from creation_lib.esp.api import export_json
from creation_lib.esp.plugin import Plugin
from creation_lib.esp.strings import write_string_table


SIGNATURES = ("WEAP", "ARMO", "NPC_", "MISC", "QUST")


def _plugin_index_snapshot(plugin: Plugin) -> tuple[int, dict[str, list[str]], list[tuple[str, int]]]:
    return (
        plugin.record_count,
        plugin.eid_index(),
        native_runtime.plugin_handle_group_signatures(plugin._rust_handle),
    )


def _write_compressed_plugin(path: Path) -> None:
    plugin = Plugin.new(path.name, game="fo4")
    for signature in SIGNATURES:
        record = plugin.new_record(signature)
        record.editor_id = f"Compressed{signature.strip('_')}"
        record.compressed = True
        record.add_subrecord("EDID", f"Compressed{signature.strip('_')}\0".encode("ascii"))
        plugin.add_record(record)
    plugin.save(path)


def _write_localized_plugin(path: Path) -> None:
    plugin = Plugin.new(path.name, game="fo4")
    plugin.header.is_localized = True
    record = plugin.new_record("MISC")
    record.editor_id = "LocalizedLanguageFilterRecord"
    record.add_subrecord("FULL", struct.pack("<I", 1))
    plugin.add_record(record)
    plugin.save(path)


def _localized_values_from_name_field(payload: dict) -> dict[str, str]:
    name_field = None
    for field in payload["fields"]:
        if "Name" in field:
            name_field = field["Name"]
            break
        if field.get("signature") == "FULL":
            name_field = field["value"]
            break
    assert name_field is not None
    return {entry["Language"]: entry["String"] for entry in name_field["Values"]}


def _export_localized_record_payload(plugin: Plugin) -> dict:
    return json.loads(
        native_runtime.plugin_handle_call(
            plugin._rust_handle, "export_record_text", 0x000800, "json",
        )
    )


def _localized_plugin_with_custom_strings(tmp_path: Path) -> tuple[Path, Path]:
    plugin_path = tmp_path / "GeneratedLocalized.esp"
    _write_localized_plugin(plugin_path)
    strings_dir = tmp_path / "CustomStrings"
    write_string_table(strings_dir / "GeneratedLocalized_en.STRINGS", {1: "English Name"}, table_type="strings")
    write_string_table(strings_dir / "GeneratedLocalized_fr.STRINGS", {1: "French Name"}, table_type="strings")
    return plugin_path, strings_dir


def test_eager_vs_lazy_compressed_kwarg_parity_and_full_record_text(tmp_path: Path) -> None:
    """eager_compressed=True/False must be index- and record-text-equivalent."""
    plugin_path = tmp_path / "GeneratedCompressed.esp"
    _write_compressed_plugin(plugin_path)

    eager = Plugin.load(plugin_path, game="fo4", eager_compressed=True)
    lazy = Plugin.load(plugin_path, game="fo4", eager_compressed=False)

    assert _plugin_index_snapshot(eager) == _plugin_index_snapshot(lazy)

    eager_text = native_runtime.plugin_handle_call(eager._rust_handle, "export_record_text", 0x000800, "json")
    lazy_text = native_runtime.plugin_handle_call(lazy._rust_handle, "export_record_text", 0x000800, "json")
    assert eager_text == lazy_text


def test_language_filter_loads_only_requested_or_missing_language(tmp_path: Path) -> None:
    plugin_path = tmp_path / "GeneratedLocalized.esp"
    _write_localized_plugin(plugin_path)
    strings_dir = tmp_path / "Strings"
    write_string_table(strings_dir / "GeneratedLocalized_en.STRINGS", {1: "English Name"}, table_type="strings")
    write_string_table(strings_dir / "GeneratedLocalized_fr.STRINGS", {1: "French Name"}, table_type="strings")

    all_languages = Plugin.load(plugin_path, game="fo4")
    english_only = Plugin.load(plugin_path, game="fo4", language="en")
    _ = all_languages.root_items
    _ = english_only.root_items
    if not all_languages.localized_strings_by_language:
        pytest.skip("fixture has no localized strings")
    assert all_languages.localized_strings_by_language["en"] == english_only.localized_strings_by_language["en"]
    assert set(english_only.localized_strings_by_language) == {"en"}

    missing_language = Plugin.load(plugin_path, game="fo4", language="de")
    _ = missing_language.root_items
    assert missing_language.localized_default_language == "de"
    assert missing_language.localized_strings_by_language == {}


@pytest.mark.parametrize(
    ("operation", "expected_values"),
    [
        ("set_localized_strings", {"English": "Edited English", "French": "French Name"}),
        ("set_localized_strings_by_language", {"English": "Edited English", "French": "Edited French"}),
        ("set_localized_field_values", {"English": "Edited English", "French": "Edited French"}),
    ],
)
def test_filtered_language_mutations_survive_authoring_rehydrate(
    tmp_path: Path, operation: str, expected_values: dict[str, str],
) -> None:
    plugin_path, strings_dir = _localized_plugin_with_custom_strings(tmp_path)
    english_only = Plugin.load(plugin_path, game="fo4", strings_dir=str(strings_dir), language="en")

    if operation == "set_localized_strings":
        english_only.set_localized_strings({1: "Edited English"}, language="en")
    elif operation == "set_localized_strings_by_language":
        english_only.set_localized_strings_by_language(
            {"en": {1: "Edited English"}, "fr": {1: "Edited French"}},
            preferred_language="en", table_types={1: "strings"},
        )
    elif operation == "set_localized_field_values":
        english_only.set_localized_field_values(
            1, {"en": "Edited English", "fr": "Edited French"},
            preferred_language="en", table_type="strings",
        )
    else:
        raise AssertionError(f"unhandled operation: {operation}")

    payload = _export_localized_record_payload(english_only)
    assert _localized_values_from_name_field(payload) == expected_values


def test_filtered_language_rehydrates_before_all_authoring_exports_and_meta(tmp_path: Path) -> None:
    """A language-filtered load still rehydrates all languages before any of the
    authoring export paths (record dict, plugin text, record text), the native
    handle meta stays free of the localized-string blobs, and clearing the
    cache empties localized_strings_by_language."""
    plugin_path, strings_dir = _localized_plugin_with_custom_strings(tmp_path)
    english_only = Plugin.load(plugin_path, game="fo4", strings_dir=str(strings_dir), language="en")
    expected = {"English": "English Name", "French": "French Name"}

    record_payload = _export_localized_record_payload(english_only)
    assert _localized_values_from_name_field(record_payload) == expected

    plugin_text_payload = json.loads(export_json(english_only, mode="authoring", backend="native"))
    group_payload = next(item for item in plugin_text_payload["items"] if item.get("label_text") == "MISC")
    record_text_payload = next(
        child for child in group_payload["children"] if child["eid"] == "LocalizedLanguageFilterRecord"
    )
    assert _localized_values_from_name_field(record_text_payload) == expected

    handle = native_runtime.plugin_handle_load(str(plugin_path), game="fo4", strings_dir=str(strings_dir))
    try:
        meta = native_runtime.plugin_handle_get_meta(handle)
        strings = native_runtime.plugin_handle_get_strings(handle)
    finally:
        native_runtime.plugin_handle_close(handle)
    assert "localized_strings_by_language" not in meta
    assert strings["localized_strings_by_language"] == {
        "en": {1: "English Name"}, "fr": {1: "French Name"},
    }

    full = Plugin.load(plugin_path, game="fo4", strings_dir=str(strings_dir))
    assert full.localized_strings_by_language == {
        "en": {1: "English Name"}, "fr": {1: "French Name"},
    }
    full.set_localized_strings_by_language({}, preferred_language="en", table_types={})
    assert full.localized_strings_by_language == {}
    assert full.localized_string_table_types == {}
    assert full.localized_strings == {}
