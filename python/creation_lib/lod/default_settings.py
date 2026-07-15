"""FO4 default LOD settings — mirrors LodSettings::fo4_default() from the Rust serde model.

Preferred path: import from the native single-source-of-truth via
``creation_lib._native.lodgen_native.default_settings_json()`` when available.
Falls back to a faithful hardcoded dict (derived from settings.rs fo4_default())
when the native module or export is absent, so callers work pre-rebuild.
"""
from __future__ import annotations

import json


def _terrain_level(quality: float, diffuse_mipmap: bool) -> dict:
    # All levels use 256x256 BC1 tiles (xLODGen golden corpus; see settings.rs
    # terrain_level()). Mips only on L4 diffuse; _msn always single-mip.
    return {
        "quality": quality,
        "max_vertices": 32767,
        "optimize_unseen": "Off",
        "diffuse_size": 256,
        "diffuse_format": "Bc1",
        "diffuse_mipmap": diffuse_mipmap,
        "normal_size": 256,
        "normal_format": "Bc1",
        "normal_mipmap": False,
        "normal_rise": 1.0,
    }


def _hardcoded_fo4_defaults() -> dict:
    return {
        "global": {
            "worldspaces": [],
            "lod_min": 4,
            "lod_max": 32,
            "stride": None,
            "align": 0,
            "southwest_cell": None,
            "bounds": None,
            "write_lodsettings": True,
            "workers": 0,
            "season": None,
            "chunk": None,
            "generate_terrain": True,
            "generate_objects": True,
            "generate_trees": True,
        },
        "terrain": {
            "levels": [
                _terrain_level(10.0, True),   # L4: diffuse mips only here
                _terrain_level(15.0, False),  # L8
                _terrain_level(20.0, False),  # L16
                _terrain_level(25.0, False),  # L32
            ],
            "protect_cell_borders": True,
            "hide_quads": False,
            "skirts": 256,
            "underside": False,
            "heightmaps": False,
            "brightness": 0.0,
            "contrast": 1.0,
            "gamma": [1.0, 1.0, 1.0],
            "vertex_color_intensity": 1.0,
            "bake_normals": False,
            "bake_specular": False,
            "default_diffuse_size": 128,
            "default_normal_size": 128,
            # Landless/ocean WATER block (settings.rs TerrainSettings.emit_water).
            "emit_water": True,
        },
        "objects": {
            "source": "records",
            "build_atlas": True,
            "atlas_size": 4096,
            "atlas_mip_flooding": False,
            "uv_range": 1.5,
            "diffuse_format": "Bc2",
            "normal_format": "Bc1",
            "specular_format": "Bc5",
            "max_tile_size": 512,
            "alpha_threshold": 128,
            "use_alpha_threshold": True,
            "use_backlight": False,
            "no_vertex_colors": False,
            "no_tangents": False,
            "remove_unseen_faces": True,
            "qem_decimate_full_model_lod": False,
            "qem_lod4_ratio": 0.35,
            "qem_lod8_ratio": 0.18,
            "qem_lod16_ratio": 0.08,
            "qem_lod32_ratio": 0.03,
            "meshopt_decimate_object_lod": False,
            "meshopt_decimate_model_lod": False,
            "meshopt_full_model_ratios": [0.30, 0.12, 0.025, 0.008],
            "meshopt_lod_model_ratios": [0.80, 0.55, 0.22, 0.08],
            "meshopt_alpha_ratios": [0.55, 0.30, 0.08, 0.025],
            "meshopt_target_errors": [0.005, 0.01, 0.025, 0.05],
            "meshopt_sloppy_from_lod": 16,
            "meshopt_quad_tri_budgets": [220000, 120000, 50000, 20000],
            "object_lod_top_model_count": 10,
            "object_lod_huge_bto_warn_mb": 64,
            "object_lod_model_cache_mb": 0,
            "fo76_bto_include_baked": True,
            "fo76_bto_include_global_atlas_baked": True,
            "fo76_bto_include_remeshed_baked": True,
            "fo76_bto_include_instances": True,
            "fo76_bto_include_tree_instances": True,
            "fo76_bto_atlas_pages": False,
            "fo76_bto_merge_atlassed_shapes": False,
            "fo76_bto_atlas_min_tile_size": 0,
            "fo76_bto_atlas_min_foliage_tile_size": 0,
            "fo76_bto_atlas_foliage_page_size": 0,
            "fo76_bto_atlas_foliage_max_tile_size": 0,
            "fo76_bto_atlas_min_alpha_tested_tile_size": 0,
            "fo76_bto_atlas_alpha_tested_page_size": 0,
            "fo76_bto_atlas_alpha_tested_max_tile_size": 0,
            "fo76_bto_atlas_from_lod": None,
            "fo76_bto_tree_billboard_from_lod": None,
            "fo76_bto_multibound_mode": "shape",
            "fo76_bto_node_layout": "fo4_per_shape",
        },
        "trees": {
            "trees_3d": True,
            "generate_billboards": False,
            "billboard_atlas_size": 2048,
            "billboard_brightness": 1.0,
        },
    }


def fo4_default_settings() -> dict:
    """Return the FO4 LOD default settings dict, byte-matching fo4_default() in Rust.

    Prefers the native single-source-of-truth when available (prevents future drift);
    falls back to the faithful hardcoded dict otherwise.
    """
    try:
        from creation_lib._native import lodgen_native  # type: ignore[import]
        return json.loads(lodgen_native.default_settings_json())
    except (ImportError, AttributeError):
        return _hardcoded_fo4_defaults()


DEFAULT_SETTINGS_JSON: str = json.dumps(_hardcoded_fo4_defaults())
