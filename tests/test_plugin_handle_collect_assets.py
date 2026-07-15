"""Native plugin handle asset collection."""
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


def _add_path_record(
    plugin: Plugin,
    signature: str,
    editor_id: str,
    subrecords: list[tuple[str, str]],
):
    record = plugin.new_record(signature)
    record.editor_id = editor_id
    for subrecord_sig, path in subrecords:
        record.add_subrecord(subrecord_sig, path.encode("utf-8") + b"\0")
    plugin.add_record(record)
    return record


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_collect_assets_returns_deduped_plugin_qualified_paths() -> None:
    plugin = Plugin.new("AssetCollect.esp", game="fo4")
    first = _add_path_record(plugin, "MISC", "FirstModel", [("MODL", r"Meshes\Shared.nif")])
    _add_path_record(plugin, "WEAP", "SecondModel", [("MODL", r"Meshes\Shared.nif")])
    texture = _add_path_record(plugin, "TXST", "TextureSet", [("TX00", r"Textures\Thing_d.dds")])

    assets = plugin.collect_assets()

    assert {
        (asset["asset_type"], asset["source_path"])
        for asset in assets
    } == {
        ("nif", "Meshes/Shared.nif"),
        ("texture", "Textures/Thing_d.dds"),
    }
    nif = next(asset for asset in assets if asset["asset_type"] == "nif")
    assert nif["source_form_key"] in {
        f"AssetCollect.esp:{first.form_id & 0x00FFFFFF:06X}",
        "AssetCollect.esp:000801",
    }
    tx = next(asset for asset in assets if asset["asset_type"] == "texture")
    assert tx["source_form_key"] == f"AssetCollect.esp:{texture.form_id & 0x00FFFFFF:06X}"
    assert tx["source_subrecord_sig"] == "TX00"


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_collect_assets_filters_by_kind_signature_and_form_key() -> None:
    plugin = Plugin.new("AssetFilters.esp", game="fo4")
    _add_path_record(plugin, "MISC", "LooseModel", [("MODL", r"Meshes\Loose.nif")])
    weapon = _add_path_record(
        plugin,
        "WEAP",
        "WeaponModel",
        [("MODL", r"Meshes\Weapon.nif"), ("ICON", r"Textures\Weapon.dds")],
    )

    weapon_fk = f"assetfilters.esp:{weapon.form_id & 0x00FFFFFF:06X}"

    assert [
        asset["source_path"]
        for asset in plugin.collect_assets(asset_kinds=["nif"], signatures=["WEAP"])
    ] == ["Meshes/Weapon.nif"]
    assert {
        asset["source_path"]
        for asset in plugin.collect_assets(form_keys=[weapon_fk])
    } == {"Meshes/Weapon.nif", "Textures/Weapon.dds"}


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_collect_assets_includes_master_handles() -> None:
    source = Plugin.new("Source.esp", game="fo4")
    master = Plugin.new("Master.esm", game="fo4")
    _add_path_record(source, "MISC", "SourceModel", [("MODL", r"Meshes\Source.nif")])
    _add_path_record(master, "MISC", "MasterModel", [("MODL", r"Meshes\Master.nif")])

    assert {
        asset["source_path"]
        for asset in source.collect_assets(master_plugins=[master], asset_kinds=["nif"])
    } == {"Meshes/Source.nif", "Meshes/Master.nif"}


@pytest.mark.skipif(not _NATIVE_AVAILABLE, reason="esp_authoring_core not installed")
def test_collect_assets_strips_data_prefix_from_sound_paths() -> None:
    plugin = Plugin.new("SoundPaths.esp", game="fo4")
    _add_path_record(plugin, "SNDR", "SoundDescriptor", [("ANAM", r"data\Sound\FX\Thing.wav")])

    assert [
        asset["source_path"]
        for asset in plugin.collect_assets(asset_kinds=["sound"])
    ] == ["Sound/FX/Thing.wav"]
