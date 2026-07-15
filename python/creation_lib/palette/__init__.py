"""Palette generation, quantization, and color mapping algorithms.

All functions accept explicit parameters -- no global config.
imagequant is optional; falls back to Pillow's built-in quantize.
"""

from __future__ import annotations

import json
import logging
import math
import os

import cv2
import numpy as np
from PIL import Image
from PIL.Image import Quantize, Palette
from creation_lib.scientific.native_runtime import (
    cubic_interpolate,
    distance_transform_indices_2d,
    find_objects_2d,
    gaussian_filter1d,
    label_2d,
    median_filter1d,
    pchip_interpolate,
)

_log = logging.getLogger("creation_lib.palette")

try:
    import imagequant
    HAS_IMAGEQUANT = True
except ImportError:
    HAS_IMAGEQUANT = False

SEMI_TRANSPARENT_ALPHA_THRESHOLD = 254


# ---------------------------------------------------------------------------
# Quantization
# ---------------------------------------------------------------------------

def quantize_image(img: Image.Image, method: str = "libimagequant",
                   final_colors: int = 128) -> Image.Image:
    """Quantize image using the specified method.

    Args:
        img: PIL Image to quantize.
        method: One of "median_cut", "max_coverage", "fast_octree",
                "libimagequant", "kmeans_adaptive", "uniform".
        final_colors: Target palette size.

    Returns:
        Quantized P-mode PIL Image.
    """
    method = (method or "median_cut").lower()
    final_colors = max(2, int(final_colors))

    if method == "median_cut":
        image = img.convert('RGB')
        return image.quantize(colors=final_colors, method=Quantize.MEDIANCUT,
                              dither=Image.Dither.FLOYDSTEINBERG)

    elif method == "max_coverage":
        image = img.convert('RGB')
        return image.quantize(colors=final_colors, method=Quantize.MAXCOVERAGE,
                              dither=Image.Dither.FLOYDSTEINBERG)

    elif method == "fast_octree":
        return img.quantize(colors=final_colors, method=Quantize.FASTOCTREE,
                            dither=Image.Dither.FLOYDSTEINBERG)

    elif method == "libimagequant":
        if HAS_IMAGEQUANT:
            try:
                return imagequant.quantize_pil_image(
                    img, dithering_level=0.5, max_colors=final_colors,
                    min_quality=90, max_quality=100,
                )
            except Exception as e:
                _log.warning("LibImageQuant failed: %s, falling back to median_cut", e)
        image = img.convert('RGB')
        return image.quantize(colors=final_colors, method=Quantize.MEDIANCUT,
                              dither=Image.Dither.FLOYDSTEINBERG)

    elif method == "kmeans_adaptive":
        return img.quantize(colors=final_colors, method=Quantize.FASTOCTREE,
                            kmeans=final_colors, dither=Image.Dither.FLOYDSTEINBERG)

    elif method == "uniform":
        image = img.convert('RGB')
        uniform_img = image.convert("P", palette=Palette.ADAPTIVE, colors=final_colors)
        return uniform_img.convert("RGB").quantize(colors=final_colors,
                                                    dither=Image.Dither.FLOYDSTEINBERG)

    else:
        image = img.convert('RGB')
        return image.quantize(colors=final_colors, method=Quantize.MEDIANCUT,
                              dither=Image.Dither.FLOYDSTEINBERG)


# ---------------------------------------------------------------------------
# Palette extraction helpers
# ---------------------------------------------------------------------------

def get_palette(q_img: Image.Image) -> np.ndarray:
    """Return the palette rows actually referenced by the P-mode image.

    Returns ndarray of shape (N, 3) where N = max_used_index + 1.
    """
    palette_raw = np.array(q_img.getpalette(), dtype=np.uint8).reshape(-1, 3)
    idx_img = np.array(q_img, dtype=np.uint8)
    if idx_img.size == 0:
        return palette_raw[:0]
    max_idx = int(idx_img.max())
    needed = min(max_idx + 1, palette_raw.shape[0])
    return palette_raw[:needed]


def get_palette_row(palette_img: Image.Image, y: int = 0) -> np.ndarray:
    """Extract a single row of colors from a palette image.

    Returns ndarray of shape (W, 3) uint8.
    """
    w, h = palette_img.size
    y = max(0, min(h - 1, y))
    row_pixels = np.array(palette_img)[y, :, :3]
    if row_pixels.ndim == 1:
        row_pixels = np.expand_dims(row_pixels, axis=0)
    return row_pixels.astype(np.uint8)


# ---------------------------------------------------------------------------
# Semi-transparent pixel handling
# ---------------------------------------------------------------------------

def apply_semi_transparent_mode(rgba: np.ndarray, mode: str = "mask",
                                threshold: int = SEMI_TRANSPARENT_ALPHA_THRESHOLD) -> np.ndarray:
    """Normalize semi-transparent pixels.

    Modes:
        "mask": set alpha<threshold to 0
        "nearest_fill": copy nearest opaque RGB into semi-transparent pixels
        "premultiply_snap": premultiply RGB by alpha, snap alpha to 0/255
    """
    if rgba is None or rgba.ndim != 3 or rgba.shape[2] < 4:
        return rgba

    mode = (mode or "mask").strip().lower()
    out = rgba.copy()
    alpha = out[:, :, 3].astype(np.uint8)
    solid = alpha >= threshold

    if mode == "mask":
        out[:, :, 3] = np.where(solid, 255, 0).astype(np.uint8)
        out[~solid, :3] = 0
        return out

    if mode == "nearest_fill":
        out[:, :, 3] = np.where(solid, 255, 0).astype(np.uint8)
        if solid.any():
            transparent = ~solid
            if transparent.any():
                nearest_indices = distance_transform_indices_2d(transparent)
                ny, nx = nearest_indices
                rgb = out[:, :, :3]
                rgb[transparent] = rgb[ny[transparent], nx[transparent]]
        else:
            out[:, :, :3] = 0
        return out

    if mode == "premultiply_snap":
        alpha_f = alpha.astype(np.float32) / 255.0
        premult = (out[:, :, :3].astype(np.float32) * alpha_f[:, :, None]).clip(0, 255).astype(np.uint8)
        out[:, :, :3] = premult
        out[:, :, 3] = np.where(solid, 255, 0).astype(np.uint8)
        out[~solid, :3] = 0
        return out

    return out


# ---------------------------------------------------------------------------
# Island NPZ state helpers
# ---------------------------------------------------------------------------

def load_island_npz(npz_path: str) -> tuple[dict, np.ndarray, list[tuple[str, int, int]]]:
    """Load palette island metadata and masks from a saved NPZ.

    Returns (metadata_dict, mask_stack_bool, islands_list).
    islands_list is a list of tuples: (name, gray_start, gray_end).
    """
    if not npz_path or not os.path.isfile(npz_path):
        raise FileNotFoundError(f"NPZ not found: {npz_path}")

    data = np.load(npz_path, allow_pickle=False)
    raw_meta = data.get("metadata")
    if raw_meta is None:
        raise ValueError("Missing metadata in NPZ")
    if hasattr(raw_meta, "item"):
        raw_meta = raw_meta.item()
    metadata = json.loads(str(raw_meta))

    mask_stack = data.get("masks")
    if mask_stack is None:
        raise ValueError("Missing masks in NPZ")
    mask_stack = mask_stack.astype(bool)

    islands = []
    for entry in metadata.get("islands", []):
        name = entry.get("name", "")
        gs = int(entry.get("gray_start", 0))
        ge = int(entry.get("gray_end", 0))
        islands.append((name, gs, ge))

    return metadata, mask_stack, islands


# ---------------------------------------------------------------------------
# Island auto-balancing
# ---------------------------------------------------------------------------

def autobalance_island_ranges(islands: list[tuple[str, int, int]],
                              masks: list[np.ndarray],
                              rgb_array: np.ndarray,
                              palette_size: int) -> list[tuple[str, int, int]]:
    """Shift palette index boundaries so under-utilized islands receive slots
    from neighboring islands with spare capacity."""
    if not islands or palette_size <= 1:
        return islands

    island_stats = []
    for (name, g0, g1), mask in zip(islands, masks):
        size = max(1, int(g1) - int(g0) + 1)
        unique_colors = 0
        if mask is not None and mask.any():
            arr = rgb_array[mask]
            if arr.size > 0:
                unique_colors = int(np.unique(arr.reshape(-1, 3), axis=0).shape[0])

        deficit = max(0, unique_colors - size)
        extra = max(0, size - unique_colors)
        island_stats.append({
            'name': name, 'g0': int(g0), 'g1': int(g1), 'size': size,
            'unique': unique_colors, 'deficit': deficit, 'extra': extra,
        })

    total_deficit = sum(s['deficit'] for s in island_stats)
    total_extra = sum(s['extra'] for s in island_stats)

    if total_deficit == 0 or total_extra == 0:
        return [(s['name'], s['g0'], s['g1']) for s in island_stats]

    n = len(island_stats)

    def clamp_and_fix(idx: int):
        s = island_stats[idx]
        s['g0'] = max(0, min(s['g0'], palette_size - 1))
        s['g1'] = max(0, min(s['g1'], palette_size - 1))
        if s['g1'] < s['g0']:
            s['g1'] = s['g0']
        s['size'] = s['g1'] - s['g0'] + 1

    progress = True
    while progress:
        progress = False
        for i in range(n):
            rec = island_stats[i]
            rec_need = rec['deficit']
            if rec_need <= 0:
                continue

            take_left = 0
            if i - 1 >= 0:
                left = island_stats[i - 1]
                left_available = min(max(0, left['extra']), max(0, (left['g1'] - left['g0'])))
                if left_available > 0 and rec['g0'] > 0 and left['g1'] + 1 == rec['g0']:
                    take_left = min(rec_need, left_available)

            take_right = 0
            if i + 1 < n:
                right = island_stats[i + 1]
                right_available = min(max(0, right['extra']), max(0, (right['g1'] - right['g0'])))
                if right_available > 0 and rec['g1'] < palette_size - 1 and rec['g1'] + 1 == right['g0']:
                    take_right = min(rec_need - take_left, right_available)

            if take_left == 0 and take_right == 0:
                if i - 1 >= 0 and rec_need > 0:
                    left = island_stats[i - 1]
                    if left['g1'] + 1 == rec['g0'] and (left['g1'] - left['g0'] + 1) > 1:
                        take_left = min(rec_need, (left['g1'] - left['g0']))
                if i + 1 < n and (rec_need - take_left) > 0:
                    right = island_stats[i + 1]
                    if rec['g1'] + 1 == right['g0'] and (right['g1'] - right['g0'] + 1) > 1:
                        take_right = min(rec_need - take_left, (right['g1'] - right['g0']))

            if take_left > 0:
                left = island_stats[i - 1]
                left['g1'] -= take_left
                rec['g0'] -= take_left
                left['extra'] = max(0, left['g1'] - left['g0'] + 1 - left['unique'])
                clamp_and_fix(i - 1)
                clamp_and_fix(i)
                rec_need -= take_left
                progress = True

            if take_right > 0:
                right = island_stats[i + 1]
                right['g0'] += take_right
                rec['g1'] += take_right
                right['extra'] = max(0, right['g1'] - right['g0'] + 1 - right['unique'])
                clamp_and_fix(i + 1)
                clamp_and_fix(i)
                progress = True

            rec['size'] = rec['g1'] - rec['g0'] + 1
            rec['deficit'] = max(0, rec['unique'] - rec['size'])

    return [(s['name'], s['g0'], s['g1']) for s in island_stats]


# ---------------------------------------------------------------------------
# Island creation from RGBA
# ---------------------------------------------------------------------------

def auto_create_islands_from_rgba(rgba: np.ndarray,
                                  palette_size: int,
                                  desired_islands: int = 4,
                                  min_pixels: int = 8,
                                  semi_transparent_mode: str = "mask"
                                  ) -> tuple[list[tuple[str, int, int]], np.ndarray, bool]:
    """Create palette islands from an RGBA image using connected-component analysis.

    Returns (islands_list, mask_stack, overflow_flag).
    """
    if rgba is None or rgba.ndim != 3 or rgba.shape[2] < 4:
        raise ValueError("RGBA image required for island generation")

    if semi_transparent_mode != "none":
        rgba = apply_semi_transparent_mode(rgba, semi_transparent_mode, SEMI_TRANSPARENT_ALPHA_THRESHOLD)

    if palette_size <= 0:
        raise ValueError("Palette size must be greater than zero.")

    alpha = rgba[:, :, 3]
    rgb = rgba[:, :, :3]
    lab_image = _lab_image(rgb)
    non_transparent = alpha > 0

    if not non_transparent.any():
        raise ValueError("Image has no opaque pixels.")

    labels, num = label_2d(non_transparent)

    if num == 0:
        raise ValueError("No regions detected.")

    slices = find_objects_2d(labels, num)
    components = []

    for lbl in range(1, num + 1):
        sl = slices[lbl - 1]
        if sl is None:
            continue
        lbl_region = labels[sl]
        region_mask = lbl_region == lbl
        pixel_count = int(region_mask.sum())
        if pixel_count < min_pixels:
            continue

        region_rgb = rgb[sl][region_mask]
        region_lab = lab_image[sl][region_mask]
        if region_rgb.size == 0 or region_lab.size == 0:
            continue

        unique_colors = np.unique(region_rgb.reshape(-1, 3), axis=0)
        hist = _lab_histogram(region_lab)
        mean_lab = region_lab.mean(axis=0)

        components.append({
            "slice": sl, "mask": region_mask, "hist": hist,
            "mean_lab": mean_lab, "pixels": pixel_count,
            "unique_colors": set(map(tuple, unique_colors.tolist())),
            "regions": [(sl, region_mask)],
        })

    if not components:
        raise ValueError("No sufficiently large regions found.")

    # Group components into islands
    hist_weight = 0.75

    def _comp_group_score(comp, ref_hist, ref_mean):
        d_hist = _histogram_intersection_distance(comp["hist"], ref_hist)
        d_mean = _mean_lab_distance(comp["mean_lab"], ref_mean)
        base = hist_weight * d_hist + (1.0 - hist_weight) * d_mean
        if not _dominant_bin_guard(comp["hist"], ref_hist):
            base += 0.25
        return base

    components_sorted = sorted(components, key=lambda c: c["pixels"], reverse=True)
    seeds: list[dict] = []
    if components_sorted:
        seeds.append(components_sorted[0])
        remaining = components_sorted[1:]
        while len(seeds) < min(desired_islands, len(components_sorted)) and remaining:
            far_idx = None
            far_score = -1.0
            for idx, cand in enumerate(remaining):
                min_dist = min(_comp_group_score(cand, s["hist"], s["mean_lab"]) for s in seeds)
                if min_dist > far_score:
                    far_score = min_dist
                    far_idx = idx
            seeds.append(remaining.pop(far_idx))

    groups: list[dict] = []
    for seed in seeds:
        groups.append({
            "hist_centroid": seed["hist"],
            "mean_lab_centroid": seed["mean_lab"],
            "pixel_total": seed["pixels"],
            "unique_colors": set(seed["unique_colors"]),
            "regions": list(seed.get("regions", [(seed["slice"], seed["mask"])])),
        })

    for comp in components_sorted:
        if comp in seeds:
            continue
        best_idx = None
        best_score = math.inf
        for idx, grp in enumerate(groups):
            score = _comp_group_score(comp, grp["hist_centroid"], grp["mean_lab_centroid"])
            if score < best_score:
                best_score = score
                best_idx = idx
        if best_idx is None:
            continue
        grp = groups[best_idx]
        total_pixels = grp["pixel_total"] + comp["pixels"]
        grp["hist_centroid"] = (grp["hist_centroid"] * grp["pixel_total"] + comp["hist"] * comp["pixels"]) / total_pixels
        grp["mean_lab_centroid"] = (grp["mean_lab_centroid"] * grp["pixel_total"] + comp["mean_lab"] * comp["pixels"]) / total_pixels
        grp["pixel_total"] = total_pixels
        grp["unique_colors"].update(comp["unique_colors"])
        grp["regions"].extend(comp.get("regions", [(comp["slice"], comp["mask"])]))

    while len(groups) < desired_islands:
        groups.append({
            "hist_centroid": np.zeros(512, dtype=np.float32),
            "mean_lab_centroid": np.zeros(3, dtype=np.float32),
            "pixel_total": 0, "unique_colors": set(), "regions": [],
        })

    # Allocate proportional island capacities
    step = 8
    if palette_size < step:
        raise ValueError("Palette size must be at least 8.")

    total_units = palette_size // step
    uniq_counts = [max(0, len(g["unique_colors"])) for g in groups]
    sum_w = sum(uniq_counts)
    if sum_w == 0:
        uniq_counts = [max(0, int(g.get("pixel_total", 0))) for g in groups]
        sum_w = sum(uniq_counts)
    if sum_w == 0:
        uniq_counts = [1 for _ in groups]
        sum_w = len(uniq_counts)

    prelim_units = []
    fracs = []
    for w in uniq_counts:
        prop = (w / sum_w) * total_units if sum_w > 0 else 0.0
        units_floor = int(math.floor(prop))
        prelim_units.append(units_floor)
        fracs.append(prop - units_floor)

    used_units = sum(prelim_units)
    for i, w in enumerate(uniq_counts):
        if w > 0 and prelim_units[i] == 0:
            prelim_units[i] = 1
            used_units += 1

    def add_units(k):
        nonlocal used_units
        order = sorted(range(len(prelim_units)), key=lambda j: fracs[j], reverse=True)
        idx = 0
        while k > 0 and used_units < total_units and idx < len(order):
            prelim_units[order[idx]] += 1
            used_units += 1
            k -= 1
            idx += 1

    def remove_units(k):
        nonlocal used_units
        order = sorted(range(len(prelim_units)), key=lambda j: fracs[j])
        idx = 0
        while k > 0 and used_units > total_units and idx < len(order):
            j = order[idx]
            min_allowed = 1 if uniq_counts[j] > 0 else 0
            if prelim_units[j] > min_allowed:
                prelim_units[j] -= 1
                used_units -= 1
                k -= 1
            idx += 1

    if used_units < total_units:
        add_units(total_units - used_units)
    elif used_units > total_units:
        remove_units(used_units - total_units)

    # Build island specs
    island_specs = []
    current_start = 0
    for units in prelim_units:
        size = max(0, units * step)
        if size == 0:
            gray_start = current_start
            gray_end = current_start - 1
        else:
            gray_start = current_start
            gray_end = current_start + size - 1
        island_specs.append({"gray_start": gray_start, "gray_end": gray_end, "capacity": size})
        current_start += size

    island_data: list[dict | None] = [None] * len(island_specs)
    groups_sorted = sorted(enumerate(groups), key=lambda t: len(t[1]["unique_colors"]), reverse=True)
    specs_sorted = sorted(enumerate(island_specs), key=lambda t: t[1]["capacity"], reverse=True)

    overflow_flag = False
    for (grp_idx, grp), (spec_idx, spec) in zip(groups_sorted, specs_sorted):
        mask = np.zeros(non_transparent.shape, dtype=bool)
        for sl, m in grp.get("regions", []):
            mask[sl][m] = True
        island_data[spec_idx] = {
            "gray_start": spec["gray_start"], "gray_end": spec["gray_end"],
            "capacity": spec["capacity"], "unique_colors": set(grp["unique_colors"]),
            "mask": mask, "pixel_total": grp.get("pixel_total", 0),
        }
        if len(grp["unique_colors"]) > spec["capacity"]:
            overflow_flag = True

    for idx, spec in enumerate(island_specs):
        if island_data[idx] is None:
            island_data[idx] = {
                "gray_start": spec["gray_start"], "gray_end": spec["gray_end"],
                "capacity": spec["capacity"], "unique_colors": set(),
                "mask": np.zeros(non_transparent.shape, dtype=bool), "pixel_total": 0,
            }

    # Assign leftover pixels
    combined_mask = np.zeros(non_transparent.shape, dtype=bool)
    for isl in island_data:
        combined_mask |= isl["mask"]

    leftovers = non_transparent & ~combined_mask
    if leftovers.any():
        target_idx = max(range(len(island_data)),
                         key=lambda j: island_data[j]["capacity"] - len(island_data[j]["unique_colors"]))
        target = island_data[target_idx]
        target["mask"] |= leftovers
        leftover_colors = set(map(tuple, rgb[leftovers].reshape(-1, 3)))
        target["unique_colors"].update(leftover_colors)
        if len(target["unique_colors"]) > target["capacity"]:
            overflow_flag = True

    islands: list[tuple[str, int, int]] = []
    mask_stack = []
    for idx, isl in enumerate(island_data, start=1):
        islands.append((f"AutoIsland_{idx}", isl["gray_start"], isl["gray_end"]))
        mask_stack.append(isl["mask"].astype(bool, copy=False))

    mask_stack_arr = np.stack(mask_stack, axis=0) if mask_stack else np.zeros((0,) + non_transparent.shape, dtype=bool)
    return islands, mask_stack_arr, overflow_flag


# ---------------------------------------------------------------------------
# Greyscale mapping strategies
# ---------------------------------------------------------------------------

def map_luminosity_default(luminosity: np.ndarray, gray_start: int, gray_end: int,
                           palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Default luminosity-based linear mapping."""
    lum_min = luminosity.min()
    lum_max = luminosity.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (luminosity - lum_min) / (lum_max - lum_min)
    remapped = gray_start + normalized * (gray_end - gray_start)
    return np.rint(remapped * palette_to_game_scale).astype(np.uint8)


def map_guard_bands_quantile(luminosity: np.ndarray, gray_start: int, gray_end: int,
                             palette_to_game_scale: float, guard_band_width: int = 1) -> np.ndarray:
    """Hybrid: Guard bands + quantile distribution."""
    effective_start = gray_start + guard_band_width
    effective_end = gray_end - guard_band_width
    effective_range = max(1, effective_end - effective_start + 1)

    unique_lum, inverse_indices = np.unique(luminosity, return_inverse=True)
    num_unique = len(unique_lum)
    if num_unique == 0:
        return np.zeros_like(luminosity, dtype=np.uint8)
    if num_unique == 1:
        unique_palette_indices = np.array([effective_start + effective_range // 2])
    else:
        unique_palette_indices = np.linspace(effective_start, effective_end, num_unique)

    remapped = unique_palette_indices[inverse_indices]
    return np.rint(remapped * palette_to_game_scale).astype(np.uint8)


def map_quantile(luminosity: np.ndarray, gray_start: int, gray_end: int,
                 palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Quantile-based distribution without guard bands."""
    unique_lum, inverse_indices = np.unique(luminosity, return_inverse=True)
    num_unique = len(unique_lum)
    if num_unique == 0:
        return np.zeros_like(luminosity, dtype=np.uint8)
    if num_unique == 1:
        unique_palette_indices = np.array([gray_start + (gray_end - gray_start) // 2])
    else:
        unique_palette_indices = np.linspace(gray_start, gray_end, num_unique)
    remapped = unique_palette_indices[inverse_indices]
    return np.rint(remapped * palette_to_game_scale).astype(np.uint8)


def map_guard_bands(luminosity: np.ndarray, gray_start: int, gray_end: int,
                    palette_to_game_scale: float, guard_band_width: int = 1) -> np.ndarray:
    """Simple guard bands with luminosity mapping."""
    effective_start = gray_start + guard_band_width
    effective_end = gray_end - guard_band_width
    effective_range = max(1, effective_end - effective_start)
    lum_min = luminosity.min()
    lum_max = luminosity.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (luminosity - lum_min) / (lum_max - lum_min)
    remapped = effective_start + normalized * effective_range
    return np.rint(remapped * palette_to_game_scale).astype(np.uint8)


def map_reverse_luminosity(luminosity: np.ndarray, gray_start: int, gray_end: int,
                           palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Reverse luminosity mapping (dark -> high, bright -> low)."""
    lum_min = luminosity.min()
    lum_max = luminosity.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (luminosity - lum_min) / (lum_max - lum_min)
    remapped = gray_end - normalized * (gray_end - gray_start)
    return (remapped * palette_to_game_scale).astype(np.uint8)


def map_alternating_luminosity(luminosity: np.ndarray, gray_start: int, gray_end: int,
                               palette_to_game_scale: float, guard_band_width: int = 0,
                               island_index: int = 0) -> np.ndarray:
    """Alternating luminosity mapping (direction reverses per island)."""
    lum_min = luminosity.min()
    lum_max = luminosity.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (luminosity - lum_min) / (lum_max - lum_min)
    if island_index % 2 == 0:
        remapped = gray_start + normalized * (gray_end - gray_start)
    else:
        remapped = gray_end - normalized * (gray_end - gray_start)
    return (remapped * palette_to_game_scale).astype(np.uint8)


def map_nearest_neighbor_reserve(luminosity: np.ndarray, gray_start: int, gray_end: int,
                                 palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Reserve first and last pixels as guard bands."""
    effective_start = gray_start + 1
    effective_end = gray_end - 1
    if effective_end < effective_start:
        effective_start = gray_start
        effective_end = gray_end
    effective_range = max(1, effective_end - effective_start)
    lum_min = luminosity.min()
    lum_max = luminosity.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (luminosity - lum_min) / (lum_max - lum_min)
    remapped = effective_start + normalized * effective_range
    return (remapped * palette_to_game_scale).astype(np.uint8)


def map_smoothed_quantile(luminosity: np.ndarray, gray_start: int, gray_end: int,
                          palette_to_game_scale: float,
                          guard_band_width: int = 1,
                          bins: int = 256, sigma: float = 1.5,
                          alpha: float = 0.3) -> np.ndarray:
    """Smoothed-quantile mapping via blurred histogram ECDF."""
    eff_start = gray_start + max(0, int(guard_band_width))
    eff_end = gray_end - max(0, int(guard_band_width))
    rng = max(1, eff_end - eff_start)

    L = luminosity.astype(np.float32)
    Lmin, Lmax = float(L.min()), float(L.max())
    if not np.isfinite(Lmin) or not np.isfinite(Lmax):
        return np.zeros_like(luminosity, dtype=np.uint8)
    if Lmax - Lmin < 1.0:
        Lmax = Lmin + 1.0
    z = np.clip((L - Lmin) / (Lmax - Lmin), 0.0, 1.0)

    bins = int(max(16, bins))
    hist, edges = np.histogram(z, bins=bins, range=(0.0, 1.0), density=False)
    sigma = float(max(0.0, sigma))
    if sigma > 0.0:
        hist = gaussian_filter1d(hist.astype(np.float32), sigma=sigma)
    else:
        hist = hist.astype(np.float32)
    total = float(hist.sum())
    if total <= 0.0:
        return np.rint(np.full_like(z, eff_start + rng * 0.5) * palette_to_game_scale).astype(np.uint8)
    cdf = np.cumsum(hist)
    cdf /= (cdf[-1] + 1e-8)
    centers = 0.5 * (edges[:-1] + edges[1:])
    Fz = np.interp(z, centers, cdf, left=0.0, right=1.0).astype(np.float32)

    g_quant = eff_start + Fz * rng
    g_lin = eff_start + z * rng
    a = float(np.clip(alpha, 0.0, 1.0))
    g = (1.0 - a) * g_quant + a * g_lin
    return np.rint(g * palette_to_game_scale).astype(np.uint8)


def map_tempered_quantile(luminosity: np.ndarray, gray_start: int, gray_end: int,
                          palette_to_game_scale: float,
                          guard_band_width: int = 0,
                          alpha: float = 0.3) -> np.ndarray:
    """Blend quantile with linear luminosity mapping."""
    if guard_band_width and guard_band_width > 0:
        gq = map_guard_bands_quantile(luminosity, gray_start, gray_end, palette_to_game_scale, guard_band_width)
        gl = map_guard_bands(luminosity, gray_start, gray_end, palette_to_game_scale, guard_band_width)
    else:
        gq = map_quantile(luminosity, gray_start, gray_end, palette_to_game_scale, 0)
        gl = map_luminosity_default(luminosity, gray_start, gray_end, palette_to_game_scale, 0)
    a = float(np.clip(alpha, 0.0, 1.0))
    g = (1.0 - a) * gq.astype(np.float32) + a * gl.astype(np.float32)
    return np.rint(g).astype(np.uint8)


def map_spline_quantile(luminosity: np.ndarray, gray_start: int, gray_end: int,
                        palette_to_game_scale: float,
                        guard_band_width: int = 1,
                        profile: str = "even",
                        gamma: float = 1.0) -> np.ndarray:
    """Monotone spline mapping using data quantile anchors."""
    eff_start = gray_start + max(0, int(guard_band_width))
    eff_end = gray_end - max(0, int(guard_band_width))
    rng = max(1, eff_end - eff_start)

    L = luminosity.astype(np.float32)
    Lmin, Lmax = float(np.min(L)), float(np.max(L))
    if Lmax - Lmin < 1.0:
        Lmax = Lmin + 1.0
    z = np.clip((L - Lmin) / (Lmax - Lmin), 0.0, 1.0)

    qs = np.array([0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0], dtype=np.float32)
    try:
        xp = np.quantile(z, qs)
    except Exception:
        xp = qs.copy()

    eps = 1e-4
    for i in range(1, len(xp)):
        if xp[i] <= xp[i - 1]:
            xp[i] = min(1.0, xp[i - 1] + eps)

    q_out = qs.copy()
    gamma = float(max(1e-3, gamma))
    if profile == "compressed_ends":
        q_out = np.power(q_out, gamma)
    elif profile == "expanded_ends":
        q_out = np.power(q_out, 1.0 / gamma)

    y = eff_start + q_out * rng

    try:
        g = pchip_interpolate(xp, y, z.astype(np.float32)).astype(np.float32)
    except Exception:
        g = eff_start + z * rng

    return np.rint(g * palette_to_game_scale).astype(np.uint8)


def map_color_clustering(rgb_array: np.ndarray, mask: np.ndarray, gray_start: int, gray_end: int,
                         palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Hue-based color clustering."""
    island_rgb = rgb_array[mask]
    unique_colors, inverse = np.unique(island_rgb.reshape(-1, 3), axis=0, return_inverse=True)

    if unique_colors.shape[0] > 0:
        unique_rgb_img = unique_colors.reshape(1, -1, 3).astype(np.uint8)
        hsv_colors = cv2.cvtColor(unique_rgb_img, cv2.COLOR_RGB2HSV).reshape(-1, 3)
        sorted_indices = np.argsort(hsv_colors[:, 0])

        num_colors = len(unique_colors)
        palette_indices = np.linspace(gray_start, gray_end, num_colors).astype(int)

        color_to_index = np.zeros(num_colors, dtype=np.uint8)
        for i, sorted_idx in enumerate(sorted_indices):
            color_to_index[sorted_idx] = int(palette_indices[i] * palette_to_game_scale)

        result = np.zeros(mask.shape, dtype=np.uint8)
        result[mask] = color_to_index[inverse]
        return result
    else:
        return np.zeros(mask.shape, dtype=np.uint8)


def map_perceptual(rgb_array: np.ndarray, mask: np.ndarray, gray_start: int, gray_end: int,
                   palette_to_game_scale: float, guard_band_width: int = 0) -> np.ndarray:
    """Perceptual brightness using CIE Lab L* channel."""
    from skimage import color as skcolor

    island_rgb = rgb_array[mask]
    if island_rgb.size == 0:
        return np.zeros(mask.shape, dtype=np.uint8)

    island_rgb_float = island_rgb.astype(np.float32) / 255.0
    lab_pixels = skcolor.rgb2lab(island_rgb_float.reshape(-1, 3))
    perceptual_lum = lab_pixels[:, 0]

    lum_min, lum_max = perceptual_lum.min(), perceptual_lum.max()
    if lum_max - lum_min < 1:
        lum_max = lum_min + 1
    normalized = (perceptual_lum - lum_min) / (lum_max - lum_min)
    remapped = gray_start + normalized * (gray_end - gray_start)

    result = np.zeros(mask.shape, dtype=np.uint8)
    result[mask] = (remapped * palette_to_game_scale).astype(np.uint8)
    return result


# ---------------------------------------------------------------------------
# Palette application
# ---------------------------------------------------------------------------

def apply_palette_to_greyscale(palette_img: Image.Image, grey_img: Image.Image,
                               palette_row: np.ndarray | None = None,
                               filter_type: str = "linear") -> Image.Image:
    """Apply palette row to a greyscale image, preserving alpha if present.

    Args:
        palette_img: Palette image to sample from.
        grey_img: Greyscale image to colorize.
        palette_row: Pre-extracted palette row (W, 3). If None, extracted from palette_img.
        filter_type: "linear", "nearest", "cubic", "anchored_linear", "gaussian",
                     "cubic_gaussian".

    Returns:
        RGB or RGBA image.
    """
    if palette_row is None or palette_row.size == 0:
        palette_row = get_palette_row(palette_img)

    pw = palette_row.shape[0]

    if pw == 256:
        lut = palette_row
    else:
        lut = _build_lut(palette_row, pw, filter_type)

    # Extract greyscale channel and optional alpha
    alpha = None
    mode = grey_img.mode
    if mode == 'L':
        g = np.array(grey_img, dtype=np.uint8)
    elif mode == 'LA':
        arr = np.array(grey_img, dtype=np.uint8)
        g, alpha = arr[:, :, 0], arr[:, :, 1]
    elif mode in ('RGBA', 'RGBa'):
        arr = np.array(grey_img, dtype=np.uint8)
        g, alpha = arr[:, :, 0], arr[:, :, 3]
    elif mode == 'RGB':
        g = np.array(grey_img, dtype=np.uint8)[:, :, 0]
    else:
        g = np.array(grey_img.convert('L'), dtype=np.uint8)

    colored = lut[g]
    rgb_img = Image.fromarray(colored, mode='RGB')
    if alpha is not None:
        a_img = Image.fromarray(alpha, mode='L')
        return Image.merge('RGBA', (*rgb_img.split(), a_img))
    return rgb_img


def _build_lut(palette_row: np.ndarray, pw: int, filter_type: str) -> np.ndarray:
    """Build a 256-entry LUT from a palette row."""
    if filter_type == "nearest":
        indices = np.round(np.linspace(0, pw - 1, num=256)).astype(int)
        return palette_row[indices]

    elif filter_type == "linear":
        x = np.linspace(0, pw - 1, num=pw)
        xi = np.linspace(0, pw - 1, num=256)
        return np.stack([
            np.interp(xi, x, palette_row[:, c]).astype(np.uint8) for c in range(3)
        ], axis=1)

    elif filter_type == "cubic":
        x = np.linspace(0, pw - 1, num=pw)
        xi = np.linspace(0, pw - 1, num=256)
        lut = np.zeros((256, 3), dtype=np.uint8)
        for c in range(3):
            lut[:, c] = np.clip(cubic_interpolate(x, palette_row[:, c], xi), 0, 255).astype(np.uint8)
        return lut

    elif filter_type == "anchored_linear":
        gk = np.rint(np.linspace(0, 255, num=pw)).astype(int)
        lut = np.zeros((256, 3), dtype=np.uint8)
        lut[gk] = palette_row
        for k in range(pw - 1):
            start_g, end_g = int(gk[k]), int(gk[k + 1])
            if end_g <= start_g:
                continue
            span = end_g - start_g
            for c in range(3):
                start_v, end_v = int(palette_row[k, c]), int(palette_row[k + 1, c])
                lut[start_g:end_g, c] = np.linspace(start_v, end_v, span, endpoint=False).astype(np.uint8)
        first_g, last_g = int(gk[0]), int(gk[-1])
        if first_g > 0:
            lut[:first_g, :] = palette_row[0]
        if last_g < 255:
            lut[last_g + 1:, :] = palette_row[-1]
        return lut

    elif filter_type == "gaussian":
        x = np.linspace(0, pw - 1, num=pw)
        xi = np.linspace(0, pw - 1, num=256)
        sigma = max(1.0, pw / 64)
        lut = np.zeros((256, 3), dtype=np.uint8)
        for c in range(3):
            channel_2d = palette_row[:, c].reshape(1, -1).astype(np.float32)
            smoothed = cv2.GaussianBlur(channel_2d, (0, 0), sigmaX=sigma)
            lut[:, c] = np.interp(xi, x, smoothed.flatten()).astype(np.uint8)
        return lut

    elif filter_type == "cubic_gaussian":
        x = np.linspace(0, pw - 1, num=pw)
        xi = np.linspace(0, pw - 1, num=256)
        sigma = max(0.5, pw / 128)
        lut = np.zeros((256, 3), dtype=np.uint8)
        for c in range(3):
            channel_2d = palette_row[:, c].reshape(1, -1).astype(np.float32)
            smoothed = cv2.GaussianBlur(channel_2d, (0, 0), sigmaX=sigma)
            lut[:, c] = np.clip(cubic_interpolate(x, smoothed.flatten(), xi), 0, 255).astype(np.uint8)
        return lut

    else:
        # Default to linear
        x = np.linspace(0, pw - 1, num=pw)
        xi = np.linspace(0, pw - 1, num=256)
        return np.stack([
            np.interp(xi, x, palette_row[:, c]).astype(np.uint8) for c in range(3)
        ], axis=1)


# ---------------------------------------------------------------------------
# Palette post-processing
# ---------------------------------------------------------------------------

def postprocess_palette_row(palette_row: np.ndarray,
                            islands: list[tuple[str, int, int]],
                            guard_band_width: int = 0,
                            smoothing: str = "none",
                            smoothing_strength: float = 0.0) -> np.ndarray:
    """Apply guard-band fill and gradient smoothing to a palette row.

    Args:
        palette_row: (W, 3) uint8.
        islands: List of (name, gray_start, gray_end).
        guard_band_width: Width in indices to blend at boundaries.
        smoothing: "none", "gaussian", "median", "bilateral".
        smoothing_strength: 0..1 float.

    Returns:
        Processed palette row (W, 3) uint8.
    """
    if palette_row is None or palette_row.size == 0:
        return palette_row

    row = np.array(palette_row, copy=True)

    if guard_band_width and guard_band_width > 0 and islands:
        _fill_guard_bands(row, islands, guard_band_width)

    if smoothing and smoothing.lower() != "none" and smoothing_strength > 0:
        row = smooth_palette_gradient(row, method=smoothing.lower(), strength=smoothing_strength)

    return row


def smooth_palette_gradient(palette_row: np.ndarray, method: str = "gaussian",
                            strength: float = 1.0) -> np.ndarray:
    """Smooth harsh transitions in palette to reduce interpolation artifacts."""
    if strength <= 0.0 or palette_row.shape[0] < 3:
        return palette_row

    smoothed = palette_row.copy().astype(np.float32)

    if method == "gaussian":
        sigma = max(0.5, strength * palette_row.shape[0] / 32)
        for c in range(3):
            channel = smoothed[:, c].reshape(1, -1)
            blurred = cv2.GaussianBlur(channel, (0, 0), sigmaX=sigma)
            smoothed[:, c] = blurred.flatten()

    elif method == "median":
        kernel_size = max(3, int(strength * 9))
        if kernel_size % 2 == 0:
            kernel_size += 1
        for c in range(3):
            smoothed[:, c] = median_filter1d(smoothed[:, c], size=kernel_size)

    elif method == "bilateral":
        sigma_color = 25.0 * (1.0 - strength * 0.5)
        sigma_space = max(1.0, strength * palette_row.shape[0] / 16)
        palette_2d = palette_row.reshape(1, -1, 3).astype(np.uint8)
        smoothed_2d = cv2.bilateralFilter(palette_2d, d=-1,
                                          sigmaColor=sigma_color,
                                          sigmaSpace=sigma_space)
        smoothed = smoothed_2d.reshape(-1, 3).astype(np.float32)

    return np.clip(smoothed, 0, 255).astype(np.uint8)


def _fill_guard_bands(palette_row: np.ndarray, islands: list[tuple[str, int, int]],
                      guard_band_width: int, anchor_mask: np.ndarray | None = None) -> None:
    """Fill guard band indices with interpolated colors between islands."""
    if guard_band_width <= 0:
        return

    for i in range(len(islands) - 1):
        _, curr_start, curr_end = islands[i]
        _, next_start, next_end = islands[i + 1]

        if curr_end + 1 == next_start:
            curr_safe = curr_end - guard_band_width
            next_safe = next_start + guard_band_width

            if curr_safe >= curr_start and next_safe <= next_end:
                curr_color = palette_row[curr_safe].astype(np.float32)
                next_color = palette_row[next_safe].astype(np.float32)

                if curr_end < len(palette_row) and not (anchor_mask is not None and anchor_mask[curr_end]):
                    palette_row[curr_end] = (0.67 * curr_color + 0.33 * next_color).astype(np.uint8)
                if next_start < len(palette_row) and not (anchor_mask is not None and anchor_mask[next_start]):
                    palette_row[next_start] = (0.33 * curr_color + 0.67 * next_color).astype(np.uint8)


def fill_nearest_neighbor_guard_bands(palette_row: np.ndarray,
                                      islands: list[tuple[str, int, int]],
                                      anchor_mask: np.ndarray | None = None) -> None:
    """Fill first and last indices of each island with nearest neighbor colors."""
    for _, gray_start, gray_end in islands:
        island_size = gray_end - gray_start + 1
        if island_size > 2:
            if gray_start < len(palette_row) and gray_start + 1 < len(palette_row):
                if not (anchor_mask is not None and anchor_mask[gray_start]):
                    palette_row[gray_start] = palette_row[gray_start + 1]
            if gray_end < len(palette_row) and gray_end - 1 >= 0:
                if not (anchor_mask is not None and anchor_mask[gray_end]):
                    palette_row[gray_end] = palette_row[gray_end - 1]


def fill_transparent_with_nearest(img: np.ndarray, mask: np.ndarray) -> np.ndarray:
    """Fill transparent pixels by copying nearest non-transparent value (EDT-based)."""
    if mask is None or img.shape != mask.shape:
        return img
    if not mask.any():
        return img
    transparent = ~mask
    if not transparent.any():
        return img
    nearest_indices = distance_transform_indices_2d(transparent)
    nearest_y, nearest_x = nearest_indices
    filled = img.copy()
    filled[transparent] = img[nearest_y[transparent], nearest_x[transparent]]
    return filled


# ---------------------------------------------------------------------------
# Palette row reconstruction from recolored image
# ---------------------------------------------------------------------------

def build_palette_row_from_recolor(grey_img: Image.Image,
                                   recolor_img: Image.Image,
                                   islands: list[tuple[str, int, int]],
                                   mask_stack: np.ndarray,
                                   palette_size: int,
                                   quant_method: str | None = None) -> np.ndarray:
    """Reconstruct a palette row from a recolored image using saved island mappings.

    Args:
        grey_img: Grayscale atlas (values 0-255).
        recolor_img: Recolored version of the original source texture.
        islands: List of (name, gray_start, gray_end).
        mask_stack: Boolean mask stack (N, H, W).
        palette_size: Target palette width.
        quant_method: Optional quantization method to apply to recolor before extracting.

    Returns:
        palette_row: ndarray shape (palette_size, 3) uint8.
    """
    if grey_img is None or recolor_img is None:
        raise ValueError("Grey image and recolor image are required")

    grey_arr = np.array(grey_img.convert('L'), dtype=np.uint8)
    if quant_method is not None:
        quantized = quantize_image(recolor_img.convert('RGB'), quant_method)
    else:
        quantized = recolor_img.convert('RGB')

    recolor_alpha = np.array(recolor_img.convert('RGBA'), dtype=np.uint8)[:, :, 3]
    recolor_rgb = np.array(quantized.convert('RGB'), dtype=np.uint8)
    recolor_rgba = np.dstack([recolor_rgb, recolor_alpha])

    h, w = grey_arr.shape
    if recolor_rgba.shape[0] != h or recolor_rgba.shape[1] != w:
        raise ValueError("Recolored image size does not match greyscale")

    if mask_stack is not None and mask_stack.size > 0:
        if mask_stack.shape[1] != h or mask_stack.shape[2] != w:
            raise ValueError("Mask stack size does not match greyscale")
    else:
        mask_stack = np.zeros((0, h, w), dtype=bool)

    palette_indices = _map_grey_to_palette_indices(grey_arr, palette_size)
    palette_row = np.zeros((palette_size, 3), dtype=np.uint8)

    for idx, (name, gs, ge) in enumerate(islands):
        mask = mask_stack[idx] if idx < mask_stack.shape[0] else np.zeros((h, w), dtype=bool)
        if not mask.any():
            continue

        alpha = recolor_rgba[:, :, 3]
        valid = mask & (alpha > 0)
        if not valid.any():
            continue

        island_pal_indices = palette_indices[valid]
        island_rgb = recolor_rgba[:, :, :3][valid]

        color_map: dict[int, list] = {g: [] for g in range(gs, ge + 1)}
        for rgb, gray in zip(island_rgb, island_pal_indices):
            if gs <= gray <= ge:
                color_map[gray].append(rgb)

        for g in range(gs, min(ge + 1, palette_size)):
            entries = color_map.get(g, [])
            if entries:
                palette_row[g] = np.mean(np.stack(entries, axis=0), axis=0).astype(np.uint8)
            else:
                prev_val, next_val = None, None
                for gg in range(g - 1, gs - 1, -1):
                    if color_map.get(gg):
                        prev_val = gg
                        break
                for gg in range(g + 1, ge + 1):
                    if color_map.get(gg):
                        next_val = gg
                        break
                if prev_val is not None and next_val is not None:
                    t = (g - prev_val) / float(next_val - prev_val)
                    color = (1 - t) * np.mean(color_map[prev_val], axis=0) + t * np.mean(color_map[next_val], axis=0)
                    palette_row[g] = np.clip(color, 0, 255).astype(np.uint8)
                elif prev_val is not None:
                    palette_row[g] = np.mean(color_map[prev_val], axis=0).astype(np.uint8)
                elif next_val is not None:
                    palette_row[g] = np.mean(color_map[next_val], axis=0).astype(np.uint8)

    return palette_row


def _map_grey_to_palette_indices(grey: np.ndarray, palette_size: int) -> np.ndarray:
    if palette_size <= 1:
        return np.zeros_like(grey, dtype=np.int32)
    scale = 255.0 / float(palette_size - 1)
    mapped = np.rint(grey.astype(np.float32) / scale)
    return np.clip(mapped, 0, palette_size - 1).astype(np.int32)


# ---------------------------------------------------------------------------
# Internal helpers (Lab color space)
# ---------------------------------------------------------------------------

def _lab_image(rgb_image: np.ndarray) -> np.ndarray:
    """Convert RGB image array to Lab (float32)."""
    return np.array(Image.fromarray(rgb_image, mode='RGB').convert('LAB'), dtype=np.float32)


def _lab_histogram(lab_pixels: np.ndarray) -> np.ndarray:
    """Compute 8x8x8 (512-bin) Lab histogram normalized to 1."""
    l_bins = np.clip((lab_pixels[:, 0] / 100.0 * 8).astype(np.int32), 0, 7)
    a_bins = np.clip(((lab_pixels[:, 1] + 128.0) / 255.0 * 8).astype(np.int32), 0, 7)
    b_bins = np.clip(((lab_pixels[:, 2] + 128.0) / 255.0 * 8).astype(np.int32), 0, 7)
    idx = l_bins * 64 + a_bins * 8 + b_bins
    hist = np.bincount(idx, minlength=512).astype(np.float32)
    hist_sum = hist.sum()
    if hist_sum > 0:
        hist /= hist_sum
    return hist


def _histogram_intersection_distance(h1: np.ndarray, h2: np.ndarray) -> float:
    return 1.0 - float(np.minimum(h1, h2).sum())


def _mean_lab_distance(m1: np.ndarray, m2: np.ndarray) -> float:
    return float(np.linalg.norm(m1 - m2) / 100.0)


def _dominant_bin_guard(comp_hist: np.ndarray, grp_hist: np.ndarray,
                        share_gap_max: float = 0.25, center_tol: float = 15.0) -> bool:
    top_bin_comp = int(comp_hist.argmax())
    top_share_comp = float(comp_hist[top_bin_comp])
    top_bin_grp = int(grp_hist.argmax())
    top_share_grp = float(grp_hist[top_bin_grp])

    if top_bin_comp == top_bin_grp:
        share_gap = abs(top_share_comp - top_share_grp)
        if share_gap > share_gap_max:
            return False
        comp_center = _lab_bin_center(top_bin_comp)
        grp_center = _lab_bin_center(top_bin_grp)
        return float(np.linalg.norm(comp_center - grp_center)) <= center_tol

    if top_share_comp < 0.60 and top_share_grp < 0.60:
        comp_center = _lab_bin_center(top_bin_comp)
        grp_center = _lab_bin_center(top_bin_grp)
        if float(np.linalg.norm(comp_center - grp_center)) <= center_tol * 1.25:
            return True

    overlap = float(np.minimum(comp_hist, grp_hist).sum())
    return overlap >= 0.55


def _lab_bin_center(bin_index: int) -> np.ndarray:
    l_bin = bin_index // 64
    rem = bin_index % 64
    a_bin = rem // 8
    b_bin = rem % 8
    return np.array([
        (l_bin + 0.5) * (100.0 / 8.0),
        (a_bin + 0.5) * (255.0 / 8.0) - 128.0,
        (b_bin + 0.5) * (255.0 / 8.0) - 128.0,
    ], dtype=np.float32)
