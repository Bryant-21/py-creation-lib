import numpy as np

from creation_lib.scientific.native_runtime import (
    CKDTree,
    butter_filter,
    convex_hull_triangles,
    distance_transform_indices_2d,
    label_2d,
)


def test_kdtree_query_matches_nearest_point():
    tree = CKDTree(np.array([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]], dtype=np.float32))

    distance, index = tree.query(np.array([0.25, 0.0, 0.0], dtype=np.float32))

    assert index == 0
    assert distance == 0.25


def test_convex_hull_tetrahedron_returns_faces():
    triangles = convex_hull_triangles(
        np.array(
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            dtype=np.float32,
        )
    )

    assert len(triangles) == 4


def test_distance_transform_indices_point_to_nearest_false_pixel():
    mask = np.ones((3, 3), dtype=bool)
    mask[0, 0] = False

    nearest = distance_transform_indices_2d(mask)

    assert tuple(nearest[:, 2, 2]) == (0, 0)


def test_label_2d_uses_four_connectivity():
    labels, count = label_2d(np.array([[True, False], [False, True]], dtype=bool))

    assert count == 2
    assert labels[0, 0] != labels[1, 1]


def test_butter_filter_preserves_shape():
    signal = np.ones(16, dtype=np.float32)

    filtered = butter_filter(signal, 100.0, 1000.0, "lowpass")

    assert filtered.shape == signal.shape
