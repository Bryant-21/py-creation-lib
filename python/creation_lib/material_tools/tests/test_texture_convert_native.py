from __future__ import annotations

import numpy as np

from creation_lib.material_tools import native_runtime


def _rgba_bytes(px: tuple[float, float, float, float]) -> bytes:
    return np.array([[px]], dtype=np.float32).tobytes()


def _rgba_from_bytes(data: bytes) -> np.ndarray:
    return np.frombuffer(data, dtype="<f4").reshape(1, 1, 4)


def test_materials_native_exposes_texture_conversion_functions():
    module = native_runtime.load_native_module()

    assert callable(getattr(module, "fo76_bundle_to_fo4_f32", None))
    assert callable(getattr(module, "fo76_normal_to_fo4_f32", None))
    assert callable(getattr(module, "passthrough_rgba_f32", None))


def test_native_runtime_bundle_wrapper_returns_source_formula_values():
    result = native_runtime.fo76_bundle_to_fo4_f32(
        _rgba_bytes((1.0, 0.0, 0.0, 1.0)),
        _rgba_bytes((0.0, 0.0, 0.0, 1.0)),
        _rgba_bytes((0.5, 1.0, 0.0, 1.0)),
        1,
        1,
        1,
        1,
        1,
        1,
        ao_multiplier=0.5,
        specular_multiplier=1.0,
        gloss_multiplier=1.0,
        spec_offset=0.8,
        emit_lighting_alpha_glow=False,
    )

    np.testing.assert_allclose(_rgba_from_bytes(result["diffuse"])[0, 0], [1.0, 0.0, 0.0, 1.0])
    np.testing.assert_allclose(
        _rgba_from_bytes(result["specgloss"])[0, 0], [0.22, 0.5, 0.0, 1.0]
    )
    assert "glow" not in result


def test_native_runtime_normal_wrapper_transforms_signed_values():
    result = native_runtime.fo76_normal_to_fo4_f32(
        np.array([[[-1.0, 0.0, 1.0, 0.5]]], dtype=np.float32).tobytes(),
        1,
        1,
    )

    np.testing.assert_allclose(_rgba_from_bytes(result)[0, 0], [0.0, 0.5, 1.0, 0.75])


def test_numpy_wrapper_returns_shaped_arrays():
    from creation_lib.material_tools.texture_convert_native import (
        TextureConversionParams,
        fo76_bundle_to_fo4_arrays,
    )

    diffuse = np.array([[[1.0, 0.0, 0.0, 1.0]]], dtype=np.float32)
    reflectivity = np.array([[[0.0, 0.0, 0.0, 1.0]]], dtype=np.float32)
    lighting = np.array([[[0.5, 1.0, 0.0, 0.75]]], dtype=np.float32)

    result = fo76_bundle_to_fo4_arrays(
        diffuse,
        reflectivity,
        lighting,
        TextureConversionParams(),
        emit_lighting_alpha_glow=True,
    )

    assert result.diffuse.shape == (1, 1, 4)
    assert result.specgloss.shape == (1, 1, 4)
    assert result.glow is not None
    np.testing.assert_allclose(result.diffuse[0, 0], [1.0, 0.0, 0.0, 1.0])
    np.testing.assert_allclose(result.specgloss[0, 0], [0.22, 0.5, 0.0, 1.0])
    np.testing.assert_allclose(result.glow[0, 0], [0.375, 0.75, 0.0, 1.0])


def test_native_runtime_converts_texture_set_paths(tmp_path):
    from creation_lib.dds import native_runtime as dds_native

    diffuse = tmp_path / "armor_d.dds"
    reflectivity = tmp_path / "armor_r.dds"
    lighting = tmp_path / "armor_l.dds"
    out_dir = tmp_path / "out"

    dds_native.write_dds_rgba(str(diffuse), 1, 1, bytes([255, 0, 0, 255]), format="R8G8B8A8_UNORM")
    dds_native.write_dds_rgba(str(reflectivity), 1, 1, bytes([0, 0, 0, 255]), format="R8G8B8A8_UNORM")
    dds_native.write_dds_rgba(str(lighting), 1, 1, bytes([128, 255, 0, 192]), format="R8G8B8A8_UNORM")

    result = native_runtime.convert_texture_set_paths({
        "source_game": "fo76",
        "target_game": "fo4",
        "inputs": [
            {"role": "diffuse", "path": str(diffuse)},
            {"role": "reflectivity", "path": str(reflectivity)},
            {"role": "lighting", "path": str(lighting)},
        ],
        "outputs": [
            {"role": "diffuse", "path": str(out_dir / "armor_d.dds"), "format": "R8G8B8A8_UNORM"},
            {"role": "specular", "path": str(out_dir / "armor_s.dds"), "format": "R8G8B8A8_UNORM"},
            {"role": "glow", "path": str(out_dir / "armor_g.dds"), "format": "R8G8B8A8_UNORM"},
        ],
        "params": {
            "ao_multiplier": 0.5,
            "specular_multiplier": 1.0,
            "gloss_multiplier": 1.0,
            "spec_offset": 0.8,
        },
    })

    assert {item["role"] for item in result["converted"]} == {"diffuse", "specular", "glow"}
    assert (out_dir / "armor_d.dds").exists()
    assert (out_dir / "armor_s.dds").exists()
    assert (out_dir / "armor_g.dds").exists()


def test_native_runtime_fo76_normal_path_preserves_normalized_rgba_except_blue(tmp_path):
    from creation_lib.dds import native_runtime as dds_native

    normal = tmp_path / "armor_n.dds"
    out_dir = tmp_path / "out"
    dds_native.write_dds_rgba(
        str(normal),
        1,
        1,
        bytes([64, 128, 255, 191]),
        format="R8G8B8A8_UNORM",
    )

    result = native_runtime.convert_texture_set_paths({
        "source_game": "fo76",
        "target_game": "fo4",
        "inputs": [
            {"role": "normal", "path": str(normal)},
        ],
        "outputs": [
            {"role": "normal", "path": str(out_dir / "armor_n.dds"), "format": "R8G8B8A8_UNORM"},
        ],
    })

    assert {item["role"] for item in result["converted"]} == {"normal"}
    assert dds_native.read_dds_rgba(str(out_dir / "armor_n.dds"))["rgba"] == bytes(
        [64, 128, 0, 191]
    )
