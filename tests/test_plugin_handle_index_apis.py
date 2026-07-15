"""Native plugin handle FormKey-shaped index APIs."""
from __future__ import annotations

import pytest

import creation_lib.esp.native_runtime as native_runtime
from creation_lib.esp.plugin import Plugin


def _try_load_native() -> object | None:
    try:
        return native_runtime.load_native_module()
    except Exception:
        return None


_NATIVE_AVAILABLE = _try_load_native() is not None


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_formkey_index_apis_stay_native_backed() -> None:
    plugin = Plugin.new("NativeIndex.esp", game="fo4")
    plugin.add_master("Fallout4.esm")

    target = plugin.new_record("MISC")
    target.editor_id = "NativeTarget"
    plugin.add_record(target)

    source = plugin.new_record("MISC")
    source.editor_id = "NativeSource"
    source.add_subrecord(
        "YNAM",
        int(target.form_id).to_bytes(4, "little"),
        semantic_type="formid",
    )
    plugin.add_record(source)

    override = plugin.new_record("MISC", form_id=0x0000ABCD)
    override.editor_id = "NativeOverride"
    plugin.add_record(override)

    assert plugin.index_stats()["record_count"] == 3

    assert plugin.eid_index()["nativesource"] == ["NativeIndex.esp:000801"]
    assert plugin.eid_index()["nativetarget"] == ["NativeIndex.esp:000800"]
    assert plugin.eid_index()["nativeoverride"] == ["Fallout4.esm:00ABCD"]
    assert plugin.get_referenced_form_keys("NativeIndex.esp:000801") == [
        "NativeIndex.esp:000800"
    ]
    assert plugin.get_referenced_form_keys("nativeindex.esp:000801") == [
        "NativeIndex.esp:000800"
    ]
    assert plugin.get_referencing_form_keys("NativeIndex.esp:000800") == [
        "NativeIndex.esp:000801"
    ]
    assert plugin.get_referencing_form_keys("nativeindex.esp:000800") == [
        "NativeIndex.esp:000801"
    ]

    late = plugin.new_record("KYWD")
    late.editor_id = "LateKeyword"
    plugin.add_record(late)

    assert plugin.eid_index()["latekeyword"] == ["NativeIndex.esp:000802"]
    assert plugin.index_stats()["record_count"] == 4
    assert plugin._rust_handle is not None


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_invalidates_when_masters_change() -> None:
    plugin = Plugin.new("MasterInvalidation.esp", game="fo4")

    override = plugin.new_record("MISC", form_id=0x0000ABCD)
    override.editor_id = "MaybeOverride"
    plugin.add_record(override)

    assert plugin.eid_index()["maybeoverride"] == ["MasterInvalidation.esp:00ABCD"]

    plugin.add_master("Fallout4.esm")

    assert plugin.eid_index()["maybeoverride"] == ["Fallout4.esm:00ABCD"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_invalidates_when_source_masters_are_ensured() -> None:
    plugin = Plugin.new("EnsureMastersInvalidation.esp", game="fo4")

    override = plugin.new_record("MISC", form_id=0x0000ABCD)
    override.editor_id = "MaybeOverride"
    plugin.add_record(override)

    assert plugin.eid_index()["maybeoverride"] == ["EnsureMastersInvalidation.esp:00ABCD"]

    native_runtime.plugin_handle_call(
        plugin._rust_handle,
        "ensure_source_masters",
        ["Fallout4.esm"],
        None,
    )

    assert plugin.eid_index()["maybeoverride"] == ["Fallout4.esm:00ABCD"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_invalidates_when_plugin_identity_changes() -> None:
    plugin = Plugin.new("BeforeRename.esp", game="fo4")

    record = plugin.new_record("MISC")
    record.editor_id = "RenameRecord"
    plugin.add_record(record)

    assert plugin.eid_index()["renamerecord"] == ["BeforeRename.esp:000800"]

    native_runtime.plugin_handle_call(
        plugin._rust_handle,
        "set_logical_identity",
        "AfterRename.esp",
        "fo4",
        None,
    )

    assert plugin.eid_index()["renamerecord"] == ["AfterRename.esp:000800"]
    exported = native_runtime.plugin_handle_call(
        plugin._rust_handle,
        "export_record_text",
        0x000800,
        "json",
    )
    assert "RenameRecord" in exported


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_invalidates_when_saved_path_changes(tmp_path) -> None:
    plugin = Plugin.new("BeforeSave.esp", game="fo4")

    record = plugin.new_record("MISC")
    record.editor_id = "SavedRecord"
    plugin.add_record(record)

    assert plugin.eid_index()["savedrecord"] == ["BeforeSave.esp:000800"]

    plugin.save(tmp_path / "AfterSave.esp")

    assert plugin.eid_index()["savedrecord"] == ["AfterSave.esp:000800"]
    exported = native_runtime.plugin_handle_call(
        plugin._rust_handle,
        "export_record_text",
        0x000800,
        "json",
    )
    assert "SavedRecord" in exported


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_stats_counts_asset_kinds() -> None:
    plugin = Plugin.new("AssetStats.esp", game="fo4")

    record = plugin.new_record("WEAP")
    record.editor_id = "AssetWeapon"
    record.add_subrecord("MODL", b"Meshes\\AssetWeapon.nif\0")
    plugin.add_record(record)

    assert plugin.assets_by_kind("nif") == [("AssetStats.esp:000800", "Meshes/AssetWeapon.nif")]
    assert plugin.index_stats()["asset_kind_count"] == 1
