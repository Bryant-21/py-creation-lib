import math

import pytest

from creation_lib.nif.nif_file import NifBlock, NifFile
from creation_lib.nif.operations.mesh import update_bounds


def test_update_bounds_writes_nested_aabb_bounding_sphere_for_sub_index_shape():
    nif = NifFile()
    shape = NifBlock(
        block_id=0,
        type_name="BSSubIndexTriShape",
        fields=[
            ("Vertex Data", [
                {"Vertex": {"x": 0.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": 2.0, "y": 0.0, "z": 0.0}},
                {"Vertex": {"x": 0.0, "y": 4.0, "z": 0.0}},
            ]),
            ("Bounding Sphere", {
                "Center": {"x": 0.0, "y": 0.0, "z": 0.0},
                "Radius": 0.0,
            }),
        ],
    )
    nif.blocks.append(shape)

    result = update_bounds(nif)

    assert result.success
    bounds = shape.get_field("Bounding Sphere")
    assert bounds["Center"] == {
        "x": pytest.approx(1.0),
        "y": pytest.approx(2.0),
        "z": pytest.approx(0.0),
    }
    assert bounds["Radius"] == pytest.approx(math.sqrt(5.0))
    assert shape.get_field("Center") is None
    assert shape.get_field("Radius") is None
