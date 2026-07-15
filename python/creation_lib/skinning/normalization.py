"""Weight normalization — enforce max bones per vertex and sum-to-one."""
from __future__ import annotations

import numpy as np


def normalize_weights(
    weights: np.ndarray,
    bone_indices: np.ndarray,
    max_bones: int = 4,
    locked_bones: set[int] | None = None,
) -> tuple[np.ndarray, np.ndarray, int]:
    """Normalize weights: enforce max bones per vertex, sum to 1.0.

    Steps:
        1. For each vertex, sort bone influences by weight descending.
        2. Keep top *max_bones* influences (respecting locked bones).
        3. Normalize remaining weights to sum to 1.0.
        4. Clamp all weights to [0, 1].

    Args:
        weights: (N, K) float32 weight values.
        bone_indices: (N, K) int32 bone indices.
        max_bones: Maximum number of bone influences per vertex.
        locked_bones: Set of bone indices whose weights should not be removed
            during the top-K pruning step.

    Returns:
        weights: normalized (N, max_bones) float32.
        bone_indices: reordered (N, max_bones) int32.
        modified_count: number of vertices that were modified.
    """
    weights = np.array(weights, dtype=np.float32, copy=True)
    bone_indices = np.array(bone_indices, dtype=np.int32, copy=True)
    n_verts = weights.shape[0]
    if n_verts == 0:
        return (
            np.zeros((0, max_bones), dtype=np.float32),
            np.zeros((0, max_bones), dtype=np.int32),
            0,
        )

    current_k = weights.shape[1]
    locked = locked_bones or set()
    modified_count = 0

    out_w = np.zeros((n_verts, max_bones), dtype=np.float32)
    out_bi = np.zeros((n_verts, max_bones), dtype=np.int32)

    for i in range(n_verts):
        # Gather all non-zero influences
        influences: list[tuple[int, float]] = []
        for j in range(current_k):
            w = float(weights[i, j])
            bi = int(bone_indices[i, j])
            if w > 0:
                influences.append((bi, w))

        if not influences:
            continue

        # Sort by weight descending and merge duplicate bone slots.
        merged: dict[int, float] = {}
        for bi, w in influences:
            merged[bi] = merged.get(bi, 0.0) + w

        locked_influences = [
            (bi, np.clip(w, 0.0, 1.0))
            for bi, w in merged.items()
            if bi in locked
        ]
        unlocked_influences = [
            (bi, np.clip(w, 0.0, 1.0))
            for bi, w in merged.items()
            if bi not in locked
        ]
        locked_influences.sort(key=lambda x: -x[1])
        unlocked_influences.sort(key=lambda x: -x[1])

        if len(locked_influences) >= max_bones:
            kept = locked_influences[:max_bones]
        else:
            remaining_slots = max_bones - len(locked_influences)
            unlocked_kept = unlocked_influences[:remaining_slots]

            locked_total = sum(w for _, w in locked_influences)
            avail = max(0.0, 1.0 - locked_total)
            unlocked_total = sum(w for _, w in unlocked_kept)
            if unlocked_total > 0:
                unlocked_kept = [
                    (bi, float(np.clip((w / unlocked_total) * avail, 0.0, 1.0)))
                    for bi, w in unlocked_kept
                ]
            kept = locked_influences + unlocked_kept

        # Check if anything changed
        orig_set = set()
        for j in range(min(current_k, max_bones)):
            w = float(weights[i, j])
            bi = int(bone_indices[i, j])
            if w > 0:
                orig_set.add((bi, w))
        new_set = set(kept)

        # For no locked bones, retain the old behavior: normalize all kept
        # influences to sum to one.
        if not locked:
            total = sum(w for _, w in kept)
            if total > 0:
                kept = [(bi, np.clip(w / total, 0.0, 1.0)) for bi, w in kept]
            else:
                kept = [(bi, 0.0) for bi, _ in kept]

        kept.sort(key=lambda x: -x[1])

        # Write output
        for j, (bi, w) in enumerate(kept):
            out_bi[i, j] = bi
            out_w[i, j] = w

        # Detect modification
        if orig_set != set(kept) or len(influences) != len(kept):
            modified_count += 1

    return out_w, out_bi, modified_count
