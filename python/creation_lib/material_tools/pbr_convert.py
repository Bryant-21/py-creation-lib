"""Shared PBR metallic-roughness -> spec-gloss math.

Used by both BACUP texture-remix workflows and
lib.material_tools.cdb_to_bgsm so that .mat -> BGSM translation and DDS
texture remix produce consistent results.

Ported from refs/fo76texconv/Texture Converter 0.8 - Source/src/
texture_processor.cpp (role_multipliers and spec/gloss reconstruction).
The C++ implementation operates per-pixel via nvtt::Surface; this module
keeps the existing numpy-shaped Python API while delegating the math to
``materials_native``.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from creation_lib.material_tools import native_runtime


@dataclass(frozen=True)
class PBRToSpecGlossParams:
    ao_multiplier: float = 1.0
    specular_multiplier: float = 1.0
    gloss_multiplier: float = 1.0
    spec_offset: float = 0.0


def pbr_to_specgloss(
    albedo: np.ndarray,      # (H, W, 3) float32 in [0, 1]
    metallic: np.ndarray,    # (H, W)    float32 in [0, 1]
    roughness: np.ndarray,   # (H, W)    float32 in [0, 1]
    ao: np.ndarray | None,   # (H, W)    float32 in [0, 1] or None
    params: PBRToSpecGlossParams,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Return (diffuse_rgb, specular_rgb, gloss) as float32 arrays in [0, 1].

    This follows the FO76 converter's ``TextureProcessor::ProcessNVTT`` math:
    thresholded reflectivity, AO lerp, grayscale specular fill, and gloss
    from inverse roughness.
    """
    albedo_f = np.ascontiguousarray(albedo, dtype=np.float32)
    metallic_f = np.ascontiguousarray(metallic, dtype=np.float32)
    roughness_f = np.ascontiguousarray(roughness, dtype=np.float32)

    if albedo_f.ndim != 3 or albedo_f.shape[-1] != 3:
        raise ValueError(f"albedo must have shape (H, W, 3); got {albedo_f.shape}")
    if metallic_f.shape != albedo_f.shape[:2]:
        raise ValueError(
            f"metallic must have shape {albedo_f.shape[:2]}; got {metallic_f.shape}"
        )
    if roughness_f.shape != albedo_f.shape[:2]:
        raise ValueError(
            f"roughness must have shape {albedo_f.shape[:2]}; got {roughness_f.shape}"
        )

    if ao is None:
        ao_bytes = None
    else:
        ao_f = np.ascontiguousarray(ao, dtype=np.float32)
        if ao_f.shape != albedo_f.shape[:2]:
            raise ValueError(f"ao must have shape {albedo_f.shape[:2]}; got {ao_f.shape}")
        ao_bytes = ao_f.tobytes()

    height, width = albedo_f.shape[:2]
    pixel_count = height * width
    payload = native_runtime.pbr_to_specgloss_f32(
        albedo_f.tobytes(),
        metallic_f.tobytes(),
        roughness_f.tobytes(),
        ao_bytes,
        pixel_count,
        ao_multiplier=params.ao_multiplier,
        specular_multiplier=params.specular_multiplier,
        gloss_multiplier=params.gloss_multiplier,
        spec_offset=params.spec_offset,
    )

    diffuse = np.frombuffer(payload["diffuse"], dtype="<f4").reshape(height, width, 3).copy()
    spec = np.frombuffer(payload["specular"], dtype="<f4").reshape(height, width, 3).copy()
    gloss = np.frombuffer(payload["gloss"], dtype="<f4").reshape(height, width).copy()

    return diffuse, spec, gloss
