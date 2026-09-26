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
@pytest.mark.parametrize("trigger", ["add_master", "ensure_source_masters", "identity_change", "save"])
def test_native_handle_index_invalidates_on_state_change(trigger: str, tmp_path) -> None:
    """The cached eid_index() must be invalidated (and rebuilt with fresh
    form keys) whenever masters, plugin identity, or the saved path change."""
    plugin = Plugin.new(f"BeforeChange_{trigger}.esp", game="fo4")

    if trigger in ("add_master", "ensure_source_masters"):
        # This record's raw form_id was minted while the plugin had zero
        # masters, so its mod-index byte is 0 (self). Adding a master shifts
        # "self" to index 1; the record must be rebased to the new self index
        # so it stays a local record rather than silently becoming an
        # override of whatever the new master defines at that object ID.
        override = plugin.new_record("MISC", form_id=0x0000ABCD)
        override.editor_id = "MaybeOverride"
        plugin.add_record(override)
        assert plugin.eid_index()["maybeoverride"] == [f"BeforeChange_{trigger}.esp:00ABCD"]

        if trigger == "add_master":
            plugin.add_master("Fallout4.esm")
        else:
            native_runtime.plugin_handle_call(
                plugin._rust_handle, "ensure_source_masters", ["Fallout4.esm"], None,
            )

        assert plugin.eid_index()["maybeoverride"] == [f"BeforeChange_{trigger}.esp:00ABCD"]
        return

    record = plugin.new_record("MISC")
    record.editor_id = "ChangeRecord"
    plugin.add_record(record)
    assert plugin.eid_index()["changerecord"] == [f"BeforeChange_{trigger}.esp:000800"]

    if trigger == "identity_change":
        native_runtime.plugin_handle_call(
            plugin._rust_handle, "set_logical_identity", f"AfterChange_{trigger}.esp", "fo4", None,
        )
        assert plugin.eid_index()["changerecord"] == [f"AfterChange_{trigger}.esp:000800"]
    else:  # save
        plugin.save(tmp_path / f"AfterChange_{trigger}.esp")
        assert plugin.eid_index()["changerecord"] == [f"AfterChange_{trigger}.esp:000800"]

    exported = native_runtime.plugin_handle_call(
        plugin._rust_handle, "export_record_text", 0x000800, "json",
    )
    assert "ChangeRecord" in exported


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_handle_index_stats_counts_asset_kinds() -> None:
    plugin = Plugin.new("AssetStats.esp", game="fo4")

    record = plugin.new_record("WEAP")
    record.editor_id = "AssetWeapon"
    record.add_subrecord("MODL", b"Meshes\\AssetWeapon.nif\0")
    plugin.add_record(record)

    assert plugin.assets_by_kind("nif") == [("AssetStats.esp:000800", "Meshes/AssetWeapon.nif")]
    assert plugin.index_stats()["asset_kind_count"] == 1
