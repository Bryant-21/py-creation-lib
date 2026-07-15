"""Thin Python compatibility layer for the SciRS2-backed native module."""

from __future__ import annotations

import numpy as np

from creation_lib._native import scientific_native as _native


class CKDTree:
    def __init__(self, data: np.ndarray):
        data = np.asarray(data, dtype=np.float64)
        if data.ndim != 2:
            raise ValueError("KDTree data must be a 2D array")
        if data.shape[0] == 0:
            raise ValueError("KDTree data must contain at least one point")
        self.data = np.ascontiguousarray(data)

    def query(self, x: np.ndarray, k: int = 1):
        query = np.asarray(x, dtype=np.float64)
        single = query.ndim == 1
        if single:
            query = query.reshape(1, -1)
        elif query.ndim != 2:
            query = query.reshape(-1, self.data.shape[1])
        query = np.ascontiguousarray(query)

        distances, indices = _native.kdtree_query(self.data, query, int(k))
        distances = np.asarray(distances)
        indices = np.asarray(indices, dtype=np.intp)
        if int(k) == 1:
            distances = distances[:, 0]
            indices = indices[:, 0]
            if single:
                return float(distances[0]), int(indices[0])
            return distances, indices
        if single:
            return distances[0], indices[0]
        return distances, indices


def convex_hull_triangles(vertices: np.ndarray | list) -> list[tuple[int, int, int]]:
    points = np.asarray(vertices, dtype=np.float64)
    if points.ndim != 2 or points.shape[1] != 3:
        raise ValueError("convex hull vertices must be an Nx3 array")
    return [tuple(map(int, tri)) for tri in _native.convex_hull_triangles(points.tolist())]


def butter_filter(data: np.ndarray, cutoff: float, fs: float, filter_type: str, order: int = 4) -> np.ndarray:
    nyquist = 0.5 * float(fs)
    if nyquist <= 0:
        raise ValueError("sampling rate must be positive")
    normal_cutoff = float(cutoff) / nyquist
    filtered = _native.butter_lfilter(
        np.asarray(data, dtype=np.float64),
        normal_cutoff,
        filter_type,
        int(order),
    )
    return np.asarray(filtered, dtype=np.float32)


def distance_transform_indices_2d(mask: np.ndarray) -> np.ndarray:
    return np.asarray(_native.distance_transform_indices_2d(np.asarray(mask, dtype=np.bool_)), dtype=np.intp)


def label_2d(mask: np.ndarray) -> tuple[np.ndarray, int]:
    labels, count = _native.label_2d_native(np.asarray(mask, dtype=np.bool_))
    return np.asarray(labels, dtype=np.intp), int(count)


def find_objects_2d(labels: np.ndarray, count: int | None = None) -> list[tuple[slice, slice] | None]:
    labels = np.asarray(labels, dtype=np.uintp)
    objects = _native.find_objects_2d_native(labels)
    max_label = int(count) if count is not None else max((int(obj[0]) for obj in objects), default=0)
    slices: list[tuple[slice, slice] | None] = [None] * max_label
    for label, min_row, max_row, min_col, max_col in objects:
        label = int(label)
        if label <= 0 or label > max_label:
            continue
        slices[label - 1] = (slice(int(min_row), int(max_row)), slice(int(min_col), int(max_col)))
    return slices


def gaussian_filter1d(data: np.ndarray, sigma: float) -> np.ndarray:
    return np.asarray(_native.gaussian_filter1d(np.asarray(data, dtype=np.float64), float(sigma)), dtype=np.float32)


def median_filter1d(data: np.ndarray, size: int) -> np.ndarray:
    return np.asarray(_native.median_filter1d(np.asarray(data, dtype=np.float64), int(size)), dtype=np.float32)


def pchip_interpolate(x: np.ndarray, y: np.ndarray, xi: np.ndarray) -> np.ndarray:
    return np.asarray(
        _native.pchip_interpolate(
            np.asarray(x, dtype=np.float64),
            np.asarray(y, dtype=np.float64),
            np.asarray(xi, dtype=np.float64),
        ),
        dtype=np.float32,
    )


def cubic_interpolate(x: np.ndarray, y: np.ndarray, xi: np.ndarray) -> np.ndarray:
    return np.asarray(
        _native.cubic_interpolate(
            np.asarray(x, dtype=np.float64),
            np.asarray(y, dtype=np.float64),
            np.asarray(xi, dtype=np.float64),
        ),
        dtype=np.float32,
    )
