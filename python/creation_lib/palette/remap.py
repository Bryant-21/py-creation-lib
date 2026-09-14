"""Fallout 4 palette remap texture generator.

Reverse-engineered algorithm: K-means zone clustering → 2G-R-B sort →
32-column gradient texture + quantized greyscale remap texture.

The FO4 shader reads remap pixel value (0-255) → gradient column (0-31),
then uses NIF fColorRemappingIndex (0.0-1.0) → gradient row.
Column 0 = neutral grey (no paint), column 31 = full material color.
"""
from __future__ import annotations

from dataclasses import dataclass

import numpy as np

def valid_remap_values(width: int = 32) -> list[int]:
    """Return valid remap pixel values for a given gradient width."""
    return [round(col / (width - 1) * 255) for col in range(width)]

VALID_REMAP_VALUES: list[int] = valid_remap_values(32)

NEUTRAL_GREY = np.array([115, 111, 107], dtype=np.uint8)  # FO4 no-paint base color


@dataclass
class PaletteZone:
    index: int            # sort rank (0 = lowest 2G-R-B, N-1 = highest)
    avg_color: np.ndarray  # float32 RGB [0,255]
    pixel_mask: np.ndarray  # bool H×W — which source pixels belong here
    remap_value: int      # quantized 0-255 written to remap texture
    gradient_column: int  # 0-31


@dataclass
class PaletteResult:
    remap: np.ndarray     # uint8 H×W single-channel greyscale
    gradient: np.ndarray  # uint8 32×32 RGBA
    zones: list[PaletteZone]


def build_gradient(zones: list[PaletteZone], width: int = 32, banded: bool = False) -> np.ndarray:
    """Build a 32×width RGBA gradient texture from a list of PaletteZones."""
    max_col = width - 1
    grad = np.zeros((32, width, 4), dtype=np.uint8)
    grad[:, :, 3] = 255

    grad[:, 0, :3] = NEUTRAL_GREY

    col_to_color: dict[int, np.ndarray] = {}
    for zone in zones:
        col = zone.gradient_column
        if 1 <= col <= max_col:
            col_to_color[col] = zone.avg_color

    grey_f = NEUTRAL_GREY.astype(np.float32)

    if banded:
        for col, color in col_to_color.items():
            color_f = np.clip(color, 0, 255).astype(np.float32)
            for band in range(8):
                t = band / 7.0
                blended = (grey_f + t * (color_f - grey_f)).round().astype(np.uint8)
                grad[band * 4: band * 4 + 4, col, :3] = blended
    else:
        for col, color in col_to_color.items():
            color_f = np.clip(color, 0, 255).astype(np.float32)
            for row in range(32):
                t = row / 31.0
                blended = (grey_f + t * (color_f - grey_f)).round().astype(np.uint8)
                grad[row, col, :3] = blended

    assigned_cols = sorted(col_to_color.keys())
    if assigned_cols:
        for col in range(1, width):
            if col not in col_to_color:
                nearest = min(assigned_cols, key=lambda c: (abs(c - col), c))
                grad[:, col, :] = grad[:, nearest, :]

    return grad


def build_final_strip(zones: list[PaletteZone], width: int = 32) -> np.ndarray:
    """Build a width x 4 RGBA strip with full zone colors (row-31 equivalent).

    This is the compact output FO4 actually reads for single-palette textures.
    Column 0 = neutral grey. Zone columns = zone avg_color. Unassigned columns
    copy from nearest assigned column. All 4 rows are identical.
    """
    max_col = width - 1
    strip = np.zeros((4, width, 4), dtype=np.uint8)
    strip[:, :, 3] = 255

    strip[:, 0, :3] = NEUTRAL_GREY

    col_to_color: dict[int, np.ndarray] = {}
    for zone in zones:
        col = zone.gradient_column
        if 1 <= col <= max_col:
            c = np.clip(zone.avg_color, 0, 255).round().astype(np.uint8)
            strip[:, col, :3] = c
            col_to_color[col] = c

    assigned_cols = sorted(col_to_color.keys())
    if assigned_cols:
        for col in range(1, width):
            if col not in col_to_color:
                nearest = min(assigned_cols, key=lambda c: (abs(c - col), c))
                strip[:, col, :] = strip[:, nearest, :]

    return strip


def assign_columns(sorted_avg_colors: list[np.ndarray], width: int = 32) -> list[PaletteZone]:
    """Assign gradient columns to pre-sorted zone average colors.

    Zones are mapped to columns 1..(width-1) (column 0 is the fixed neutral-grey slot).

    N=1 special case: single zone maps to last column (full paint).
    N>1: evenly distributed across columns 1..(width-1).
    """
    max_col = width - 1
    n = len(sorted_avg_colors)
    vals = valid_remap_values(width)
    zones = []
    for i, color in enumerate(sorted_avg_colors):
        if n == 1:
            col = max_col
        else:
            col = round(i / (n - 1) * (max_col - 1)) + 1  # spans 1..max_col
        zones.append(PaletteZone(
            index=i,
            avg_color=color.astype(np.float32),
            pixel_mask=np.zeros((0, 0), dtype=bool),
            remap_value=vals[col],
            gradient_column=col,
        ))
    return zones


def build_remap_texture(
    shape: tuple[int, int],
    label_map: np.ndarray,
    zones: list[PaletteZone],
) -> np.ndarray:
    """Build a uint8 H×W greyscale remap texture.

    shape: (H, W) of the source image.
    label_map: int32 H×W where each pixel has its cluster label (0..N-1).
    zones: PaletteZone list indexed by cluster label.
    Transparent pixels (label=-1) receive remap_value=0.
    """
    h, w = shape
    remap = np.zeros((h, w), dtype=np.uint8)
    for zone in zones:
        remap[label_map == zone.index] = zone.remap_value
    return remap


def zones_from_remap(remap: np.ndarray, width: int = 32) -> list[PaletteZone]:
    """Reconstruct PaletteZone objects from a greyscale remap image."""
    max_col = width - 1
    unique_vals = sorted(set(remap.ravel()) - {0})
    zones: list[PaletteZone] = []
    for i, val in enumerate(unique_vals):
        col = round(int(val) / 255 * max_col)
        col = max(1, min(max_col, col))
        zones.append(PaletteZone(
            index=i,
            avg_color=np.zeros(3, dtype=np.float32),
            pixel_mask=(remap == val),
            remap_value=int(val),
            gradient_column=col,
        ))
    zones.sort(key=lambda z: z.gradient_column)
    for i, z in enumerate(zones):
        z.index = i
    return zones


def sample_variant_colors(
    variant_image: "Image.Image",
    remap: np.ndarray,
    zones: list[PaletteZone],
) -> list[np.ndarray]:
    """Sample average colors per zone from a variant texture using the remap as mask.

    ``remap`` is the uint8 H×W zone mask; the variant is resized to match it.
    Returns one float32 RGB array per zone, in *zones* order.
    """
    from PIL import Image

    if variant_image.mode != "RGBA":
        variant_image = variant_image.convert("RGBA")
    h, w = remap.shape[:2]
    if variant_image.size != (w, h):
        variant_image = variant_image.resize((w, h), Image.BILINEAR)
    rgba = np.array(variant_image, dtype=np.float32)

    colors: list[np.ndarray] = []
    for zone in zones:
        mask = remap == zone.remap_value
        if mask.any():
            avg = rgba[mask, :3].mean(axis=0)
        else:
            avg = np.array([115.0, 111.0, 107.0], dtype=np.float32)  # neutral grey
        colors.append(avg.astype(np.float32))
    return colors


def build_variant_gradient(
    variants: list[list[np.ndarray]],
    zone_columns: list[int],
    band_height: int = 4,
    width: int = 32,
) -> np.ndarray:
    """Build a multi-band gradient texture with one band per color variant.

    Each *variants* entry is a list of float32 RGB arrays, one per zone, in
    *zone_columns* order; *zone_columns* are gradient columns (1..width-1).
    ``band_height`` defaults to 4, the FO4 minimum; ``width`` is 32, 64, or 128.
    Returns a uint8 (H x width x 4) RGBA array, H = len(variants) * band_height,
    max 128.
    """
    max_col = width - 1
    n_variants = min(len(variants), 128 // band_height)
    height = n_variants * band_height
    grad = np.zeros((height, width, 4), dtype=np.uint8)
    grad[:, :, 3] = 255

    grad[:, 0, :3] = NEUTRAL_GREY

    for vi in range(n_variants):
        row_start = vi * band_height
        row_end = row_start + band_height
        col_to_color: dict[int, np.ndarray] = {}
        for ci, col in enumerate(zone_columns):
            if 1 <= col <= max_col and ci < len(variants[vi]):
                col_to_color[col] = variants[vi][ci]

        for col, color in col_to_color.items():
            c = np.clip(color, 0, 255).round().astype(np.uint8)
            grad[row_start:row_end, col, :3] = c

        assigned_cols = sorted(col_to_color.keys())
        if assigned_cols:
            for col in range(1, width):
                if col not in col_to_color:
                    nearest = min(assigned_cols, key=lambda c: (abs(c - col), c))
                    grad[row_start:row_end, col, :] = grad[row_start:row_end, nearest, :]

    return grad


def auto_convert(source: "Image.Image", n_zones: int = 6, width: int = 32) -> PaletteResult:
    """Convert a color image to remap + gradient textures using K-means zone assignment.

    ``n_zones`` is the number of material zones (4-12 recommended); ``width``
    is the gradient width (32, 64, or 128). The result holds the remap, a
    width x 4 gradient strip, and the zones.
    """
    from creation_lib.palette.native_runtime import cluster_rgb
    from PIL import Image

    if source.mode != "RGBA":
        source = source.convert("RGBA")
    rgba = np.array(source, dtype=np.uint8)
    h, w = rgba.shape[:2]

    alpha_mask = rgba[:, :, 3] >= 128
    label_map = np.full((h, w), -1, dtype=np.int32)

    if alpha_mask.sum() == 0:
        remap = np.zeros((h, w), dtype=np.uint8)
        gradient = build_final_strip([], width=width)
        return PaletteResult(remap=remap, gradient=gradient, zones=[])

    small = np.array(
        Image.fromarray(rgba[:, :, :3]).resize((256, 256), Image.BILINEAR),
        dtype=np.float32,
    )
    pixels_small = small.reshape(-1, 3)
    opaque_pixels = rgba[alpha_mask, :3].astype(np.float32)

    n = max(1, min(n_zones, int(alpha_mask.sum())))

    km_centers, opaque_labels = cluster_rgb(
        pixels_small, opaque_pixels, n_clusters=n, seed=42, n_init=3,
    )
    label_map[alpha_mask] = opaque_labels

    # Compute zone average colors on full-resolution pixels, falling back to
    # the k-means centroid when a cluster has no full-res pixels.
    centers = np.array([
        rgba[label_map == i, :3].mean(axis=0) if (label_map == i).any()
        else km_centers[i]
        for i in range(n)
    ], dtype=np.float32)

    # Sort zones by 2G - R - B (ascending: rust/red first, olive/green last)
    metrics = 2 * centers[:, 1] - centers[:, 0] - centers[:, 2]
    sort_order = np.argsort(metrics)

    sorted_colors = [centers[i] for i in sort_order]
    zones = assign_columns(sorted_colors, width=width)

    # Rebuild masks and label_map using sorted order
    orig_to_sorted = {int(orig): sorted_i for sorted_i, orig in enumerate(sort_order)}
    sorted_label_map = np.full((h, w), -1, dtype=np.int32)
    for orig_label, sorted_label in orig_to_sorted.items():
        sorted_label_map[label_map == orig_label] = sorted_label

    for zone in zones:
        zone.pixel_mask = sorted_label_map == zone.index

    remap = build_remap_texture((h, w), sorted_label_map, zones)
    gradient = build_final_strip(zones, width=width)
    return PaletteResult(remap=remap, gradient=gradient, zones=zones)
