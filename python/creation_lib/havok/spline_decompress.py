"""Spline decompression binding for the native Havok backend."""
from __future__ import annotations

from dataclasses import dataclass


@dataclass
class SplineTransform:
    """One bone's transform at one frame."""

    translation: tuple[float, float, float]
    rotation: tuple[float, float, float, float]
    scale: tuple[float, float, float]


def _transform_from_native(data: dict) -> SplineTransform:
    return SplineTransform(
        translation=tuple(float(value) for value in data.get("translation", [0.0, 0.0, 0.0]))[:3],
        rotation=tuple(float(value) for value in data.get("rotation", [0.0, 0.0, 0.0, 1.0]))[:4],
        scale=tuple(float(value) for value in data.get("scale", [1.0, 1.0, 1.0]))[:3],
    )


def decompress_spline(
    data: bytes,
    num_transform_tracks: int,
    num_float_tracks: int,
    num_frames: int,
    max_frames_per_block: int,
    num_blocks: int,
    block_offsets: list[int],
    float_block_offsets: list[int],
    mask_and_quant_size: int,
    block_duration: float,
    block_inverse_duration: float,
    frame_duration: float,
) -> list[list[SplineTransform]]:
    """Decompress a spline-compressed animation via the Rust Havok backend."""
    from creation_lib.havok.native_runtime import decompress_spline_native

    frames = decompress_spline_native(
        data,
        {
            "num_transform_tracks": num_transform_tracks,
            "num_float_tracks": num_float_tracks,
            "num_frames": num_frames,
            "max_frames_per_block": max_frames_per_block,
            "num_blocks": num_blocks,
            "block_offsets": block_offsets,
            "float_block_offsets": float_block_offsets,
            "mask_and_quant_size": mask_and_quant_size,
            "block_duration": block_duration,
            "block_inverse_duration": block_inverse_duration,
            "frame_duration": frame_duration,
        },
    )
    return [[_transform_from_native(transform) for transform in frame] for frame in frames]
