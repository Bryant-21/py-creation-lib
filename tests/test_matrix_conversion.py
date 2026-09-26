import glm
import pytest


def test_glm_list16_conversion_roundtrip_and_identity():
    from creation_lib.renderer.gizmo import glm_to_list16, list16_to_glm

    identity = glm.mat4(1.0)
    lst = glm_to_list16(identity)
    expected = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]
    for a, b in zip(lst, expected):
        assert abs(a - b) < 1e-6

    original = glm.translate(glm.mat4(1.0), glm.vec3(1, 2, 3))
    lst = glm_to_list16(original)
    assert len(lst) == 16
    result = list16_to_glm(lst)
    for col in range(4):
        for row in range(4):
            assert abs(original[col][row] - result[col][row]) < 1e-6
