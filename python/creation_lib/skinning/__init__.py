"""Skinning engine — headless library for bone weight manipulation.

Provides SkinData interchange format, weight transfer algorithms,
normalization, segment generation, brush operations, and importers.
No UI dependencies.
"""
from .skin_data import SegmentInfo, SkinData, SubSegmentInfo
from .weight_transfer import transfer_weights
from .normalization import normalize_weights
from .partitions import (
    assign_partitions_from_reference,
    assign_partitions_from_bones,
    rebuild_fo4_segments_from_body_parts,
    sync_fo4_segments_from_ids,
    generate_skin_partition_blocks,
)
from .brushes import (
    build_adjacency,
    paint_weight,
    smooth_weights,
    blur_weights,
    gradient_weights,
    mirror_weights,
    flood_fill_weight,
)
from .reference_body import (
    detect_skeleton,
    load_reference_body,
    extract_skin_data_from_nif,
)
from .importers import import_obj

__all__ = [
    "SegmentInfo",
    "SubSegmentInfo",
    "SkinData",
    "transfer_weights",
    "normalize_weights",
    "assign_partitions_from_reference",
    "assign_partitions_from_bones",
    "rebuild_fo4_segments_from_body_parts",
    "sync_fo4_segments_from_ids",
    "generate_skin_partition_blocks",
    "build_adjacency",
    "paint_weight",
    "smooth_weights",
    "blur_weights",
    "gradient_weights",
    "mirror_weights",
    "flood_fill_weight",
    "detect_skeleton",
    "load_reference_body",
    "extract_skin_data_from_nif",
    "import_obj",
]
