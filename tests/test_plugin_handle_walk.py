"""Native plugin handle dependency walker."""
from __future__ import annotations

import json

import pytest

import creation_lib.esp.native_runtime as native_runtime
from creation_lib.esp.plugin import Plugin


def _try_load_native() -> object | None:
    try:
        return native_runtime.load_native_module()
    except Exception:
        return None


_NATIVE_AVAILABLE = _try_load_native() is not None


def _form_key(plugin_name: str, form_id: int) -> str:
    return f"{plugin_name}:{form_id & 0x00FFFFFF:06X}"


def _policy(**overrides: object) -> str:
    payload = {
        "follow_signatures": None,
        "asset_kinds": None,
        "reverse_passes": [],
        "behavior_bundle": False,
        "character_assets": False,
        "animation_lookup": False,
        "max_depth": None,
    }
    payload.update(overrides)
    return json.dumps(payload)


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_single_root_follows_formid_edges_and_assets() -> None:
    plugin = Plugin.new("WalkSynthetic.esp", game="fo4")
    child = plugin.new_record("MISC")
    child.editor_id = "ChildMisc"
    child.add_subrecord("MODL", b"Meshes\\Child.nif\0")
    plugin.add_record(child)
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("CNAM", int(child.form_id).to_bytes(4, "little"), semantic_type="formid")
    root.add_subrecord("MODL", b"Meshes\\Root.nif\0")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkSynthetic.esp", root.form_id)],
        policy_json=_policy(asset_kinds=["nif"]),
    )

    records = {(item["form_key"], item["walk_depth"]) for item in result["reached_records"]}
    assert records == {
        (_form_key("WalkSynthetic.esp", root.form_id), 0),
        (_form_key("WalkSynthetic.esp", child.form_id), 1),
    }
    assert {
        (item["asset_kind"], item["source_path"], item["source_form_key"], item["walk_depth"])
        for item in result["assets"]
    } == {
        ("nif", "Meshes/Root.nif", _form_key("WalkSynthetic.esp", root.form_id), 0),
        ("nif", "Meshes/Child.nif", _form_key("WalkSynthetic.esp", child.form_id), 1),
    }


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_rooted_walk_does_not_build_global_refs_or_assets_sections() -> None:
    plugin = Plugin.new("WalkNoGlobalIndexes.esp", game="fo4")
    child = plugin.new_record("MISC")
    child.editor_id = "ChildMisc"
    child.add_subrecord("MODL", b"Meshes\\Child.nif\0")
    plugin.add_record(child)
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("CNAM", int(child.form_id).to_bytes(4, "little"), semantic_type="formid")
    root.add_subrecord("MODL", b"Meshes\\Root.nif\0")
    plugin.add_record(root)

    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is False
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is False

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkNoGlobalIndexes.esp", root.form_id)],
        policy_json=_policy(asset_kinds=["nif"]),
    )

    assert [item["signature"] for item in result["reached_records"]] == ["WEAP", "MISC"]
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is False
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is False


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_empty_root_walk_preserves_full_index_path() -> None:
    plugin = Plugin.new("WalkFullIndexes.esp", game="fo4")
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    plugin.add_record(root)

    result = native_runtime.plugin_handle_walk_dependencies(
        [plugin._rust_handle],
        [],
        [],
        _policy(max_depth=0),
        strict_unresolved_masters=True,
    )

    assert [item["form_key"] for item in result["reached_records"]] == [
        _form_key("WalkFullIndexes.esp", root.form_id)
    ]
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "core") is True
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is True
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is True


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_rooted_walk_reports_timing_breakdown() -> None:
    plugin = Plugin.new("WalkTiming.esp", game="fo4")
    root = plugin.new_record("MISC")
    root.editor_id = "RootMisc"
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkTiming.esp", root.form_id)],
        policy_json=_policy(max_depth=0),
    )

    timing = result.get("timing")
    assert isinstance(timing, dict)
    assert set(timing) >= {
        "locator_ms",
        "main_walk_ms",
        "reverse_race_ms",
        "reverse_skm_ms",
        "total_ms",
    }
    assert all(isinstance(timing[key], int) and timing[key] >= 0 for key in timing)


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_reports_unresolved_form_keys_when_not_strict() -> None:
    plugin = Plugin.new("WalkUnresolved.esp", game="fo4")
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("CNAM", (0x01000001).to_bytes(4, "little"), semantic_type="formid")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkUnresolved.esp", root.form_id)],
        policy_json=_policy(),
        strict=False,
    )

    assert result["errors"] == []
    assert result["unresolved_form_keys"] == ["01000001"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_normalizes_equivalent_roots_before_visiting() -> None:
    plugin = Plugin.new("WalkNormalize.esp", game="fo4")
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[
            "walknormalize.esp:800",
            _form_key("WalkNormalize.esp", root.form_id),
        ],
        policy_json=_policy(max_depth=0),
    )

    assert [item["form_key"] for item in result["reached_records"]] == [
        _form_key("WalkNormalize.esp", root.form_id)
    ]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_empty_roots_stays_on_primary_source_handle() -> None:
    primary = Plugin.new("WalkPrimary.esp", game="fo4")
    primary_root = primary.new_record("WEAP")
    primary_root.editor_id = "PrimaryRoot"
    primary.add_record(primary_root)

    secondary = Plugin.new("WalkSecondary.esp", game="fo4")
    secondary_root = secondary.new_record("WEAP")
    secondary_root.editor_id = "SecondaryRoot"
    secondary.add_record(secondary_root)

    result = native_runtime.plugin_handle_walk_dependencies(
        [primary._rust_handle, secondary._rust_handle],
        [],
        [],
        _policy(max_depth=0),
        strict_unresolved_masters=True,
    )

    assert [item["form_key"] for item in result["reached_records"]] == [
        _form_key("WalkPrimary.esp", primary_root.form_id)
    ]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_native_cell_slice_api_returns_expected_shape_for_empty_plugin() -> None:
    plugin = Plugin.new("EmptyWorld.esp", game="fo4")

    result = native_runtime.plugin_handle_collect_cell_slice_roots(
        plugin._rust_handle,
        worldspace_editor_id="MissingWorld",
        min_x=0,
        min_y=0,
        max_x=0,
        max_y=0,
        include_worldspace_persistent_cell=False,
    )

    assert result["cell_form_keys"] == []
    assert result["placed_form_keys"] == []
    assert result["cell_children"] == {}
    assert result["cell_grids"] == {}
    assert any("MissingWorld" in warning for warning in result["warnings"])
    assert set(result["timing"]) >= {"world_lookup_ms", "cell_traversal_ms", "total_ms"}


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_strict_unresolved_master_reference_reports_form_key() -> None:
    plugin = Plugin.new("WalkStrict.esp", game="fo4", masters=["MissingMaster.esm"])
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("CNAM", (0x00000900).to_bytes(4, "little"), semantic_type="formid")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkStrict.esp", root.form_id)],
        policy_json=_policy(),
        strict=True,
    )

    assert result["errors"] == ["Unresolved FormKey: MissingMaster.esm:000900"]
    assert result["unresolved_form_keys"] == []


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_non_strict_unresolved_master_reference_records_form_key() -> None:
    plugin = Plugin.new("WalkNonStrict.esp", game="fo4", masters=["MissingMaster.esm"])
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("CNAM", (0x00000900).to_bytes(4, "little"), semantic_type="formid")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkNonStrict.esp", root.form_id)],
        policy_json=_policy(),
        strict=False,
    )

    assert result["errors"] == []
    assert result["unresolved_form_keys"] == ["MissingMaster.esm:000900"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_resolves_references_through_master_handles() -> None:
    master = Plugin.new("WalkMaster.esm", game="fo4")
    master_child = master.new_record("MISC")
    master_child.editor_id = "MasterChild"
    master_child.add_subrecord("MODL", b"Meshes\\MasterChild.nif\0")
    master.add_record(master_child)

    plugin = Plugin.new("WalkSource.esp", game="fo4", masters=["WalkMaster.esm"])
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord(
        "CNAM",
        (master_child.form_id & 0x00FFFFFF).to_bytes(4, "little"),
        semantic_type="formid",
    )
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        master_plugins=[master],
        root_form_keys=[_form_key("WalkSource.esp", root.form_id)],
        policy_json=_policy(asset_kinds=["nif"]),
    )

    assert result["errors"] == []
    assert result["unresolved_form_keys"] == []
    assert {
        (item["form_key"], item["signature"], item["walk_depth"])
        for item in result["reached_records"]
    } == {
        (_form_key("WalkSource.esp", root.form_id), "WEAP", 0),
        (_form_key("WalkMaster.esm", master_child.form_id), "MISC", 1),
    }
    assert {
        (item["asset_kind"], item["source_path"], item["source_form_key"], item["walk_depth"])
        for item in result["assets"]
    } == {
        (
            "nif",
            "Meshes/MasterChild.nif",
            _form_key("WalkMaster.esm", master_child.form_id),
            1,
        )
    }


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_reverse_race_injects_race_referencing_root_keyword() -> None:
    plugin = Plugin.new("WalkRace.esp", game="fo4")
    keyword = plugin.new_record("KYWD")
    keyword.editor_id = "AnimKeyword"
    plugin.add_record(keyword)
    race = plugin.new_record("RACE")
    race.editor_id = "CreatureRace"
    race.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    plugin.add_record(race)
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkRace.esp", root.form_id)],
        policy_json=_policy(reverse_passes=["race"], max_depth=0),
    )

    injected = [
        item for item in result["reached_records"] if item["walker_pass"] == "reverse_race"
    ]
    assert [(item["form_key"], item["signature"]) for item in injected] == [
        (_form_key("WalkRace.esp", race.form_id), "RACE")
    ]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_reverse_skm_walks_keyword_mapping_sound_descriptors() -> None:
    plugin = Plugin.new("WalkSkm.esp", game="fo4")
    keyword = plugin.new_record("KYWD")
    keyword.editor_id = "WeaponKeyword"
    plugin.add_record(keyword)
    sound = plugin.new_record("SNDR")
    sound.editor_id = "WeaponFireSound"
    sound.add_subrecord("ANAM", b"Data\\Sound\\FX\\Weapon.wav\0")
    plugin.add_record(sound)
    skm = plugin.new_record("KSSM")
    skm.editor_id = "WeaponSoundMapping"
    skm.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    skm.add_subrecord("CNAM", int(sound.form_id).to_bytes(4, "little"), semantic_type="formid")
    plugin.add_record(skm)
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkSkm.esp", root.form_id)],
        policy_json=_policy(asset_kinds=["sound"], reverse_passes=["skm"], max_depth=1),
    )

    reverse_records = {
        (item["form_key"], item["signature"], item["walker_pass"])
        for item in result["reached_records"]
        if item["walker_pass"] == "reverse_skm"
    }
    assert reverse_records == {
        (_form_key("WalkSkm.esp", skm.form_id), "KSSM", "reverse_skm"),
        (_form_key("WalkSkm.esp", sound.form_id), "SNDR", "reverse_skm"),
    }
    assert {
        (item["asset_kind"], item["source_path"], item["walker_pass"])
        for item in result["assets"]
    } == {("sound", "Sound/FX/Weapon.wav", "reverse_skm")}


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_reverse_skm_respects_max_depth() -> None:
    plugin = Plugin.new("WalkSkmBounded.esp", game="fo4")
    keyword = plugin.new_record("KYWD")
    keyword.editor_id = "WeaponKeyword"
    plugin.add_record(keyword)
    sound = plugin.new_record("SNDR")
    sound.editor_id = "WeaponFireSound"
    plugin.add_record(sound)
    skm = plugin.new_record("KSSM")
    skm.editor_id = "WeaponSoundMapping"
    skm.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    skm.add_subrecord("CNAM", int(sound.form_id).to_bytes(4, "little"), semantic_type="formid")
    plugin.add_record(skm)
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    root.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    plugin.add_record(root)

    result = plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkSkmBounded.esp", root.form_id)],
        policy_json=_policy(reverse_passes=["skm"], max_depth=0),
    )

    assert {
        (item["form_key"], item["signature"], item["walker_pass"])
        for item in result["reached_records"]
        if item["walker_pass"] == "reverse_skm"
    } == {(_form_key("WalkSkmBounded.esp", skm.form_id), "KSSM", "reverse_skm")}
