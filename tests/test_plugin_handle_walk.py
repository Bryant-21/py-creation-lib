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
def test_walk_single_root_follows_formid_edges_and_assets_without_global_indexes() -> None:
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

    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is False
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is False

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

    # A rooted walk must not have built the global refs/assets index sections.
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is False
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is False


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_empty_roots_use_full_index_and_primary_source_handle() -> None:
    plugin = Plugin.new("WalkFullIndexes.esp", game="fo4")
    root = plugin.new_record("WEAP")
    root.editor_id = "RootWeapon"
    plugin.add_record(root)

    # An empty root list falls back to the full core/refs/assets index build.
    result = native_runtime.plugin_handle_walk_dependencies(
        [plugin._rust_handle], [], [], _policy(max_depth=0), strict_unresolved_masters=True,
    )
    assert [item["form_key"] for item in result["reached_records"]] == [
        _form_key("WalkFullIndexes.esp", root.form_id)
    ]
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "core") is True
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "refs") is True
    assert native_runtime.plugin_handle_debug_section_loaded(plugin._rust_handle, "assets") is True

    # With multiple handles and empty roots, it stays on the primary (first) handle.
    secondary = Plugin.new("WalkSecondary.esp", game="fo4")
    secondary_root = secondary.new_record("WEAP")
    secondary_root.editor_id = "SecondaryRoot"
    secondary.add_record(secondary_root)
    multi_result = native_runtime.plugin_handle_walk_dependencies(
        [plugin._rust_handle, secondary._rust_handle], [], [], _policy(max_depth=0),
        strict_unresolved_masters=True,
    )
    assert [item["form_key"] for item in multi_result["reached_records"]] == [
        _form_key("WalkFullIndexes.esp", root.form_id)
    ]

    # An empty-plugin cell-slice query on a missing worldspace reports it and returns empty shapes.
    cell_result = native_runtime.plugin_handle_collect_cell_slice_roots(
        plugin._rust_handle, worldspace_editor_id="MissingWorld",
        min_x=0, min_y=0, max_x=0, max_y=0, include_worldspace_persistent_cell=False,
    )
    assert cell_result["cell_form_keys"] == []
    assert cell_result["placed_form_keys"] == []
    assert any("MissingWorld" in warning for warning in cell_result["warnings"])


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_reports_unresolved_local_form_key_when_not_strict() -> None:
    local_plugin = Plugin.new("WalkLocal.esp", game="fo4")
    local_root = local_plugin.new_record("WEAP")
    local_root.editor_id = "RootWeapon"
    local_root.add_subrecord("CNAM", (0x01000001).to_bytes(4, "little"), semantic_type="formid")
    local_plugin.add_record(local_root)

    local_result = local_plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkLocal.esp", local_root.form_id)],
        policy_json=_policy(), strict=False,
    )
    assert local_result["errors"] == []
    assert local_result["unresolved_form_keys"] == ["01000001"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
@pytest.mark.parametrize("strict", [True, False])
def test_walk_unresolved_master_reference(strict: bool) -> None:
    """A ref into a missing master is an error in strict mode, and recorded in
    unresolved_form_keys otherwise."""
    master_plugin = Plugin.new("WalkMasterRef.esp", game="fo4", masters=["MissingMaster.esm"])
    master_root = master_plugin.new_record("WEAP")
    master_root.editor_id = "RootWeapon"
    master_root.add_subrecord("CNAM", (0x00000900).to_bytes(4, "little"), semantic_type="formid")
    master_plugin.add_record(master_root)

    master_result = master_plugin.walk_dependencies(
        root_form_keys=[_form_key("WalkMasterRef.esp", master_root.form_id)],
        policy_json=_policy(), strict=strict,
    )
    if strict:
        assert master_result["errors"] == ["Unresolved FormKey: MissingMaster.esm:000900"]
        assert master_result["unresolved_form_keys"] == []
    else:
        assert master_result["errors"] == []
        assert master_result["unresolved_form_keys"] == ["MissingMaster.esm:000900"]


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_walk_normalizes_roots_and_resolves_references_through_masters() -> None:
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

    # Passing both a plugin-prefix-lowercased alias and the canonical root key
    # should normalize to a single visit, and resolve through the master handle.
    result = plugin.walk_dependencies(
        master_plugins=[master],
        root_form_keys=[
            "walksource.esp:800",
            _form_key("WalkSource.esp", root.form_id),
        ],
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
        ("nif", "Meshes/MasterChild.nif", _form_key("WalkMaster.esm", master_child.form_id), 1),
    }


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
@pytest.mark.parametrize("max_depth", [1, 0])
def test_walk_reverse_passes_inject_race_and_skm_records(max_depth: int) -> None:
    """reverse_race pulls in a RACE referencing the root's keyword; reverse_skm
    pulls in a KSSM sound mapping and (depth-permitting) the SNDR it points at."""
    plugin = Plugin.new("WalkReverse.esp", game="fo4")
    keyword = plugin.new_record("KYWD")
    keyword.editor_id = "SharedKeyword"
    plugin.add_record(keyword)
    race = plugin.new_record("RACE")
    race.editor_id = "CreatureRace"
    race.add_subrecord("KWDA", int(keyword.form_id).to_bytes(4, "little"), semantic_type="formid_array")
    plugin.add_record(race)
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
        root_form_keys=[_form_key("WalkReverse.esp", root.form_id)],
        policy_json=_policy(asset_kinds=["sound"], reverse_passes=["race", "skm"], max_depth=max_depth),
    )

    reverse_records = {
        (item["form_key"], item["signature"], item["walker_pass"])
        for item in result["reached_records"]
        if item["walker_pass"] in ("reverse_race", "reverse_skm")
    }
    assert (_form_key("WalkReverse.esp", race.form_id), "RACE", "reverse_race") in reverse_records
    if max_depth == 0:
        assert (_form_key("WalkReverse.esp", skm.form_id), "KSSM", "reverse_skm") in reverse_records
        assert (_form_key("WalkReverse.esp", sound.form_id), "SNDR", "reverse_skm") not in reverse_records
    else:
        assert (_form_key("WalkReverse.esp", sound.form_id), "SNDR", "reverse_skm") in reverse_records
        assert {
            (item["asset_kind"], item["source_path"], item["walker_pass"])
            for item in result["assets"]
        } == {("sound", "Sound/FX/Weapon.wav", "reverse_skm")}
