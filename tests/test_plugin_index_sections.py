"""Native plugin handle lazy index section loading."""
from __future__ import annotations

import pytest

import creation_lib.esp.native_runtime as nr
from creation_lib.esp.plugin import Plugin


def _try_load_native() -> object | None:
    try:
        return nr.load_native_module()
    except Exception:
        return None


_NATIVE_AVAILABLE = _try_load_native() is not None


def _load_section_test_plugin(tmp_path) -> Plugin:
    plugin = Plugin.new("SectionTest.esp", game="fo4")
    weapon = plugin.new_record("WEAP")
    weapon.editor_id = "SectionWeapon"
    weapon.add_subrecord("MODL", b"Meshes\\SectionWeapon.nif\0")
    plugin.add_record(weapon)
    source = plugin.new_record("MISC")
    source.editor_id = "SectionSource"
    source.add_subrecord(
        "YNAM",
        int(weapon.form_id).to_bytes(4, "little"),
        semantic_type="formid",
    )
    plugin.add_record(source)
    armor = plugin.new_record("ARMO")
    armor.editor_id = "SectionArmor"
    plugin.add_record(armor)
    plugin_path = tmp_path / "SectionTest.esp"
    plugin.save(plugin_path)
    return Plugin.load(plugin_path, game="fo4")


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_index_sections_load_lazily_and_independently(tmp_path) -> None:
    """Each index section (core/records/refs/assets) is built only when the
    API that needs it is first called, never eagerly and never as a side
    effect of building an unrelated section."""
    plugin = _load_section_test_plugin(tmp_path)
    handle = plugin._rust_handle

    assert sorted(nr.plugin_handle_group_signatures(handle)) == [
        ("ARMO", 1), ("MISC", 1), ("WEAP", 1),
    ]
    for section in ("core", "records", "refs", "assets"):
        assert nr.plugin_handle_debug_section_loaded(handle, section) is False

    assert plugin.eid_index()["sectionweapon"] == ["SectionTest.esp:000800"]
    assert nr.plugin_handle_debug_section_loaded(handle, "core") is True
    assert nr.plugin_handle_debug_section_loaded(handle, "records") is False
    assert nr.plugin_handle_debug_section_loaded(handle, "refs") is False
    assert nr.plugin_handle_debug_section_loaded(handle, "assets") is False

    assert plugin.get_referencing_form_ids(0x000800) == [0x000801]
    assert nr.plugin_handle_debug_section_loaded(handle, "refs") is True
    assert nr.plugin_handle_debug_section_loaded(handle, "assets") is False

    assert plugin.assets_by_kind("nif") == [
        ("SectionTest.esp:000800", "Meshes/SectionWeapon.nif")
    ]
    assert nr.plugin_handle_debug_section_loaded(handle, "assets") is True
    assert nr.plugin_handle_debug_section_loaded(handle, "records") is False


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_record_locator_resolves_loaded_and_many_records(tmp_path) -> None:
    plugin = _load_section_test_plugin(tmp_path)
    for raw_form_id, signature in (
        (0x000800, "WEAP"), (0x000801, "MISC"), (0x000802, "ARMO"),
    ):
        record = plugin.get_record_by_form_id(raw_form_id)
        assert record is not None
        assert record.form_id == raw_form_id
        assert record.signature == signature

    many = Plugin.new("RecordLocatorTest.esp", game="fo4")
    for i in range(60):
        signature = ("WEAP", "ARMO", "MISC")[i % 3]
        record = many.new_record(signature)
        record.editor_id = f"RecordLocator{i:02d}"
        many.add_record(record)
    many_path = tmp_path / "RecordLocatorTest.esp"
    many.save(many_path)
    many = Plugin.load(many_path, game="fo4")
    handle = many._rust_handle
    nr.plugin_handle_force_build_records_section(handle)
    assert nr.plugin_handle_debug_section_loaded(handle, "records") is True
    for offset in range(50):
        raw_form_id = 0x000800 + offset
        expected_signature = ("WEAP", "ARMO", "MISC")[offset % 3]
        record = many.get_record_by_form_id(raw_form_id)
        assert record is not None
        assert record.form_id == raw_form_id
        assert record.signature == expected_signature
