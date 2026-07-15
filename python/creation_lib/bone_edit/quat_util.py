"""Quaternion utility functions used by both the pose engine and the spline patcher.

Convention: quaternions are numpy arrays of shape (4,) in (x, y, z, w) order.
"""

from __future__ import annotations

import numpy as np


def quat_multiply(q1: np.ndarray, q2: np.ndarray) -> np.ndarray:
    """Hamilton product q1 * q2 in (x, y, z, w) order."""
    x1, y1, z1, w1 = q1
    x2, y2, z2, w2 = q2
    w = w1 * w2 - x1 * x2 - y1 * y2 - z1 * z2
    x = w1 * x2 + x1 * w2 + y1 * z2 - z1 * y2
    y = w1 * y2 - x1 * z2 + y1 * w2 + z1 * x2
    z = w1 * z2 + x1 * y2 - y1 * x2 + z1 * w2
    return np.array([x, y, z, w])


def quat_normalize(q: np.ndarray) -> np.ndarray:
    """Normalize a quaternion to unit length. Returns identity for zero-length input."""
    n = float(np.linalg.norm(q))
    if n < 1e-12:
        return np.array([0.0, 0.0, 0.0, 1.0])
    return q / n


def quat_conjugate(q: np.ndarray) -> np.ndarray:
    """Conjugate of (x, y, z, w). For unit quaternions this is the inverse."""
    return np.array([-q[0], -q[1], -q[2], q[3]])


def quat_to_matrix(q: np.ndarray) -> np.ndarray:
    """Convert (x, y, z, w) to a 3x3 rotation matrix."""
    x, y, z, w = q
    return np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w),     2 * (x * z + y * w)],
        [2 * (x * y + z * w),     1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w),     2 * (y * z + x * w),     1 - 2 * (x * x + y * y)],
    ], dtype=np.float64)


def mat_to_quat(m: np.ndarray) -> np.ndarray:
    """Convert a 3x3 rotation matrix to (x, y, z, w) quaternion.

    Branchless trace-based form picks the most numerically stable diagonal
    element to avoid divide-by-near-zero. Inverse of `quat_to_matrix`.
    """
    trace = m[0, 0] + m[1, 1] + m[2, 2]
    if trace > 0:
        s = 0.5 / np.sqrt(trace + 1.0)
        return np.array([
            (m[2, 1] - m[1, 2]) * s,
            (m[0, 2] - m[2, 0]) * s,
            (m[1, 0] - m[0, 1]) * s,
            0.25 / s,
        ])
    if m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
        s = 2.0 * np.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2])
        return np.array([
            0.25 * s,
            (m[0, 1] + m[1, 0]) / s,
            (m[0, 2] + m[2, 0]) / s,
            (m[2, 1] - m[1, 2]) / s,
        ])
    if m[1, 1] > m[2, 2]:
        s = 2.0 * np.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2])
        return np.array([
            (m[0, 1] + m[1, 0]) / s,
            0.25 * s,
            (m[1, 2] + m[2, 1]) / s,
            (m[0, 2] - m[2, 0]) / s,
        ])
    s = 2.0 * np.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1])
    return np.array([
        (m[0, 2] + m[2, 0]) / s,
        (m[1, 2] + m[2, 1]) / s,
        0.25 * s,
        (m[1, 0] - m[0, 1]) / s,
    ])


def quat_from_axis_angle(axis: np.ndarray, angle_rad: float) -> np.ndarray:
    """Build a unit quaternion from a rotation axis and angle in radians."""
    axis = np.asarray(axis, dtype=np.float64)
    n = float(np.linalg.norm(axis))
    if n < 1e-12:
        return np.array([0.0, 0.0, 0.0, 1.0])
    axis = axis / n
    s = np.sin(angle_rad * 0.5)
    c = np.cos(angle_rad * 0.5)
    return np.array([axis[0] * s, axis[1] * s, axis[2] * s, c])


def quat_from_to(v_from: np.ndarray, v_to: np.ndarray) -> np.ndarray:
    """Shortest-arc rotation that maps v_from onto v_to. Both must be non-zero."""
    a = v_from / max(float(np.linalg.norm(v_from)), 1e-12)
    b = v_to / max(float(np.linalg.norm(v_to)), 1e-12)
    d = float(np.dot(a, b))
    if d > 0.999999:
        return np.array([0.0, 0.0, 0.0, 1.0])
    if d < -0.999999:
        # 180 deg rotation — pick any orthogonal axis
        ortho = np.array([1.0, 0.0, 0.0]) if abs(a[0]) < 0.9 else np.array([0.0, 1.0, 0.0])
        axis = np.cross(a, ortho)
        axis /= max(float(np.linalg.norm(axis)), 1e-12)
        return np.array([axis[0], axis[1], axis[2], 0.0])
    axis = np.cross(a, b)
    s = np.sqrt((1.0 + d) * 2.0)
    inv_s = 1.0 / s
    return quat_normalize(np.array([axis[0] * inv_s, axis[1] * inv_s, axis[2] * inv_s, s * 0.5]))
