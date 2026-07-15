"""Test that auto-skin weight transfer produces quality results comparable to
hand-authored NIF weights.

Strategy:
  1. Load outfitf.nif — save its original (ground-truth) weights
  2. Load femalebody.nif as reference
  3. Clear weights on the outfit
  4. Run transfer_weights() to regenerate
  5. Compare generated vs original per-vertex and per-bone

Acceptance criteria:
  - Mean per-vertex weight error < 0.10  (strict: < 0.05)
  - 90%+ vertices have max bone match (dominant bone is the same)
  - No bone has >25% of its vertices assigned to the wrong dominant bone
"""
from __future__ import annotations

import os
from pathlib import Path

import numpy as np
import pytest

from creation_lib.skinning.reference_body import extract_skin_data_from_nif
from creation_lib.skinning.weight_transfer import transfer_weights
from creation_lib.skinning.normalization import normalize_weights

# Paths
FO4_EXTRACTED_DIR = Path(
    os.environ.get("FO4_EXTRACTED_DIR")
    or Path(__file__).resolve().parents[2] / "extracted" / "fo4"
)
OUTFIT_NIF = FO4_EXTRACTED_DIR / "meshes/armor/armoredcoat/outfitf.nif"
REFERENCE_NIF = FO4_EXTRACTED_DIR / "meshes/actors/character/characterassets/femalebody.nif"


@pytest.fixture(scope="module")
def outfit():
    if not OUTFIT_NIF.is_file():
        pytest.skip(f"FO4 extracted data not available (set FO4_EXTRACTED_DIR): {OUTFIT_NIF}")
    return extract_skin_data_from_nif(str(OUTFIT_NIF))


@pytest.fixture(scope="module")
def reference():
    if not REFERENCE_NIF.is_file():
        pytest.skip(f"FO4 extracted data not available (set FO4_EXTRACTED_DIR): {REFERENCE_NIF}")
    return extract_skin_data_from_nif(str(REFERENCE_NIF))


def _dominant_bone(weights: np.ndarray, bone_indices: np.ndarray) -> np.ndarray:
    """For each vertex, return the bone index with the highest weight."""
    n = len(weights)
    dominant = np.zeros(n, dtype=np.int32)
    for i in range(n):
        best_j = int(np.argmax(weights[i]))
        dominant[i] = bone_indices[i, best_j]
    return dominant


def _remap_bone_indices(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    src_bone_names: list[str],
    dst_bone_names: list[str],
) -> tuple[np.ndarray, np.ndarray]:
    """Remap bone indices from src bone list to dst bone list by name.

    Bones in src that don't exist in dst get weight 0.
    Returns new (weights, bone_indices) arrays with indices into dst_bone_names.
    """
    src_to_dst: dict[int, int] = {}
    for si, name in enumerate(src_bone_names):
        if name in dst_bone_names:
            src_to_dst[si] = dst_bone_names.index(name)

    n, max_b = weights.shape
    out_w = np.zeros_like(weights)
    out_bi = np.zeros_like(bone_indices)

    for i in range(n):
        # Collect all (dst_bone_idx, weight) for this vertex
        bw_map: dict[int, float] = {}
        for j in range(max_b):
            w = float(weights[i, j])
            bi = int(bone_indices[i, j])
            if w > 0 and bi in src_to_dst:
                dst_bi = src_to_dst[bi]
                bw_map[dst_bi] = bw_map.get(dst_bi, 0.0) + w

        # Sort by weight, keep top max_b
        sorted_bw = sorted(bw_map.items(), key=lambda x: -x[1])[:max_b]
        for j, (bi, w) in enumerate(sorted_bw):
            out_bi[i, j] = bi
            out_w[i, j] = w

        total = out_w[i].sum()
        if total > 0:
            out_w[i] /= total

    return out_w, out_bi


def test_transfer_weights_quality(outfit, reference):
    """Core quality test: transfer weights and compare to ground truth."""
    # Save ground truth
    gt_weights = outfit.weights.copy()
    gt_bone_indices = outfit.bone_indices.copy()
    gt_bone_names = list(outfit.bone_names)

    # Run transfer
    gen_weights, gen_bone_indices, stats = transfer_weights(
        source=reference,
        target_vertices=outfit.vertices,
        target_triangles=outfit.triangles,
        method="hybrid",
    )

    # Normalize generated weights
    gen_weights, gen_bone_indices, _ = normalize_weights(
        gen_weights, gen_bone_indices, max_bones=4,
    )

    print(f"\nTransfer stats: {stats}")

    # Remap generated weights to ground-truth bone space for comparison.
    # Generated indices are in reference.bone_names space.
    gen_remapped_w, gen_remapped_bi = _remap_bone_indices(
        gen_weights, gen_bone_indices,
        reference.bone_names, gt_bone_names,
    )

    # --- Metric 1: Per-vertex weight error ---
    # For each vertex, compute error as sum of absolute weight differences
    # across all bones (using a dense representation).
    n_bones = len(gt_bone_names)
    n_verts = outfit.num_vertices

    def _to_dense(weights, bone_indices, n_bones, n_verts):
        dense = np.zeros((n_verts, n_bones), dtype=np.float32)
        for i in range(n_verts):
            for j in range(weights.shape[1]):
                bi = int(bone_indices[i, j])
                w = float(weights[i, j])
                if w > 0 and 0 <= bi < n_bones:
                    dense[i, bi] += w
        return dense

    gt_dense = _to_dense(gt_weights, gt_bone_indices, n_bones, n_verts)
    gen_dense = _to_dense(gen_remapped_w, gen_remapped_bi, n_bones, n_verts)

    per_vertex_error = np.abs(gt_dense - gen_dense).sum(axis=1)
    mean_error = float(per_vertex_error.mean())
    median_error = float(np.median(per_vertex_error))
    p90_error = float(np.percentile(per_vertex_error, 90))
    max_error = float(per_vertex_error.max())

    print(f"\nPer-vertex weight error:")
    print(f"  Mean:   {mean_error:.4f}")
    print(f"  Median: {median_error:.4f}")
    print(f"  P90:    {p90_error:.4f}")
    print(f"  Max:    {max_error:.4f}")

    # --- Metric 2: Dominant bone match rate ---
    gt_dominant = _dominant_bone(gt_weights, gt_bone_indices)
    gen_dominant = _dominant_bone(gen_remapped_w, gen_remapped_bi)
    dominant_match = (gt_dominant == gen_dominant).mean()
    print(f"\nDominant bone match rate: {dominant_match:.1%}")

    # --- Metric 3: Per-bone accuracy ---
    print(f"\nPer-bone accuracy (dominant bone match):")
    worst_bones = []
    for bi, name in enumerate(gt_bone_names):
        gt_mask = gt_dominant == bi
        count = int(gt_mask.sum())
        if count == 0:
            continue
        match_rate = float((gen_dominant[gt_mask] == bi).mean())
        if match_rate < 0.75:
            worst_bones.append((name, count, match_rate))
        print(f"  {name:30s}: {count:5d} verts, {match_rate:6.1%} match")

    if worst_bones:
        print(f"\nWorst bones (<75% match):")
        for name, count, rate in sorted(worst_bones, key=lambda x: x[2]):
            print(f"  {name}: {rate:.1%} ({count} verts)")

    # --- Metric 4: Spatial error distribution ---
    # Show where the worst errors are (Z-axis = height)
    z_vals = outfit.vertices[:, 2]
    z_bins = np.linspace(z_vals.min(), z_vals.max(), 6)
    print(f"\nError by height (Z):")
    for i in range(len(z_bins) - 1):
        mask = (z_vals >= z_bins[i]) & (z_vals < z_bins[i + 1])
        if mask.any():
            bin_error = float(per_vertex_error[mask].mean())
            print(f"  Z [{z_bins[i]:7.1f}, {z_bins[i+1]:7.1f}): "
                  f"{int(mask.sum()):4d} verts, mean_err={bin_error:.4f}")

    # Assertions
    assert mean_error < 0.10, (
        f"Mean per-vertex weight error {mean_error:.4f} > 0.10 threshold"
    )
    assert dominant_match > 0.90, (
        f"Dominant bone match rate {dominant_match:.1%} < 90% threshold"
    )
    for name, count, rate in worst_bones:
        assert rate > 0.25, (
            f"Bone '{name}' has only {rate:.1%} dominant match on {count} verts"
        )
