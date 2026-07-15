"""Numpy compatibility wrappers for materials_native texture conversion."""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from creation_lib.material_tools import native_runtime


@dataclass(frozen=True)
class TextureConversionParams:
    ao_multiplier: float = 0.5
    specular_multiplier: float = 1.0
    gloss_multiplier: float = 1.0
    spec_offset: float = 0.8


@dataclass(frozen=True)
class Fo76BundleArrays:
    diffuse: np.ndarray
    specgloss: np.ndarray
    glow: np.ndarray | None = None


def _rgba_float32(arr: np.ndarray, name: str) -> np.ndarray:
    values = np.ascontiguousarray(arr, dtype=np.float32)
    if values.ndim != 3 or values.shape[2] != 4:
        raise ValueError(f"{name} must have shape (height, width, 4); got {values.shape}")
    return values


def _decode_rgba(data: bytes, height: int, width: int) -> np.ndarray:
    return np.frombuffer(data, dtype="<f4").reshape(height, width, 4).copy()


def fo76_bundle_to_fo4_arrays(
    diffuse: np.ndarray,
    reflectivity: np.ndarray,
    lighting: np.ndarray,
    params: TextureConversionParams | None = None,
    *,
    emit_lighting_alpha_glow: bool = False,
) -> Fo76BundleArrays:
    params = params or TextureConversionParams()
    diffuse_f = _rgba_float32(diffuse, "diffuse")
    reflectivity_f = _rgba_float32(reflectivity, "reflectivity")
    lighting_f = _rgba_float32(lighting, "lighting")
    height, width = diffuse_f.shape[:2]
    reflectivity_height, reflectivity_width = reflectivity_f.shape[:2]
    lighting_height, lighting_width = lighting_f.shape[:2]

    payload = native_runtime.fo76_bundle_to_fo4_f32(
        diffuse_f.tobytes(),
        reflectivity_f.tobytes(),
        lighting_f.tobytes(),
        width,
        height,
        reflectivity_width,
        reflectivity_height,
        lighting_width,
        lighting_height,
        ao_multiplier=params.ao_multiplier,
        specular_multiplier=params.specular_multiplier,
        gloss_multiplier=params.gloss_multiplier,
        spec_offset=params.spec_offset,
        emit_lighting_alpha_glow=emit_lighting_alpha_glow,
    )

    return Fo76BundleArrays(
        diffuse=_decode_rgba(payload["diffuse"], height, width),
        specgloss=_decode_rgba(payload["specgloss"], height, width),
        glow=(_decode_rgba(payload["glow"], height, width) if "glow" in payload else None),
    )


def fo76_normal_to_fo4_array(normal: np.ndarray) -> np.ndarray:
    normal_f = _rgba_float32(normal, "normal")
    height, width = normal_f.shape[:2]
    payload = native_runtime.fo76_normal_to_fo4_f32(normal_f.tobytes(), width, height)
    return _decode_rgba(payload, height, width)
