"""Thin Python boundary for native palette_native entrypoints."""
from __future__ import annotations

import numpy as np

from creation_lib._native import palette_native as _native


def cluster_rgb(
    fit_pixels: np.ndarray,
    predict_pixels: np.ndarray,
    n_clusters: int,
    seed: int = 42,
    n_init: int = 3,
) -> tuple[np.ndarray, np.ndarray]:
    """Mini-batch K-means on RGB pixels.

    Returns (centers (n_clusters, 3) float32, labels (M,) int32).
    """
    if fit_pixels.dtype != np.float32:
        fit_pixels = fit_pixels.astype(np.float32)
    if predict_pixels.dtype != np.float32:
        predict_pixels = predict_pixels.astype(np.float32)
    fit_pixels = np.ascontiguousarray(fit_pixels)
    predict_pixels = np.ascontiguousarray(predict_pixels)
    return _native.cluster_rgb(
        fit_pixels, predict_pixels, int(n_clusters), int(seed), int(n_init)
    )
