"""Tests for complex mesh collision shapes (MOPP, compressed mesh)."""
import numpy as np
import pytest


def _make_nif():
    """Create a minimal NifFile for testing."""
    from creation_lib.nif.nif_file import NifFile
    return NifFile()


def test_create_mopp_shape():
    """Create bhkMoppBvTreeShape from triangle mesh geometry."""
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()

    # Cube vertices in NIF space
    verts = np.array([
        [-10, -10, -10], [10, -10, -10], [10, 10, -10], [-10, 10, -10],
        [-10, -10, 10], [10, -10, 10], [10, 10, 10], [-10, 10, 10],
    ], dtype=np.float32)
    triangles = np.array([
        [0, 1, 2], [0, 2, 3], [4, 6, 5], [4, 7, 6],
        [0, 4, 5], [0, 5, 1], [2, 6, 7], [2, 7, 3],
        [0, 3, 7], [0, 7, 4], [1, 5, 6], [1, 6, 2],
    ], dtype=np.int32)

    result = create_mopp_shape(nif, verts, triangles, radius=0.005)

    assert result is not None
    mopp_id, packed_id, data_id = result

    # Verify block types
    mopp_block = nif.get_block(mopp_id)
    assert mopp_block.type_name == "bhkMoppBvTreeShape"

    packed_block = nif.get_block(packed_id)
    assert packed_block.type_name == "bhkPackedNiTriStripsShape"

    data_block = nif.get_block(data_id)
    assert data_block.type_name == "hkPackedNiTriStripsData"

    # Verify MOPP code was set
    mopp_code = mopp_block.get_field("MOPP Code")
    assert mopp_code is not None
    assert mopp_code.get("Data Size", 0) > 0

    # Verify triangle data
    assert data_block.get_field("Num Triangles") == 12
    assert data_block.get_field("Num Vertices") == 8

    # Empty geometry should return None.
    empty_verts = np.array([], dtype=np.float32).reshape(0, 3)
    empty_tris = np.array([], dtype=np.int32).reshape(0, 3)
    assert create_mopp_shape(nif, empty_verts, empty_tris) is None

    # Inter-block references: bhkMoppBvTreeShape.Shape -> bhkPackedNiTriStripsShape
    # and bhkPackedNiTriStripsShape.Data -> hkPackedNiTriStripsData.
    assert mopp_block.get_field("Shape") == packed_id
    assert packed_block.get_field("Data") == data_id


def test_extract_mopp_geometry():
    """Extract geometry back from a MOPP shape."""
    from creation_lib.nif.operations.collision_mesh import (
        create_mopp_shape,
        extract_mopp_geometry,
    )

    nif = _make_nif()

    verts_in = np.array([
        [0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0],
    ], dtype=np.float32)
    tris_in = np.array([[0, 1, 2], [0, 2, 3]], dtype=np.int32)

    result = create_mopp_shape(nif, verts_in, tris_in)
    assert result is not None
    mopp_id, _, _ = result

    verts_out, tris_out, materials_out = extract_mopp_geometry(nif, mopp_id)

    # Vertex count and triangle count should match
    assert len(verts_out) == 4
    assert len(tris_out) == 2

    # Vertices should round-trip through Havok scale within tolerance
    # (float32 precision loss from divide then multiply by 69.99125)
    np.testing.assert_allclose(verts_out, verts_in, atol=0.01)


def test_extract_geometry_wrong_block_raises():
    """Extracting from the wrong block type should raise ValueError, for both shape kinds."""
    from creation_lib.nif.operations.collision_mesh import (
        extract_mopp_geometry, extract_compressed_mesh,
    )

    nif = _make_nif()
    nif.add_block("NiNode")

    with pytest.raises(ValueError, match="not bhkMoppBvTreeShape"):
        extract_mopp_geometry(nif, 0)
    with pytest.raises(ValueError, match="not bhkCompressedMeshShapeData"):
        extract_compressed_mesh(nif, 0)


@pytest.mark.parametrize("shape_type,expected_shape_type", [
    ("mopp", "bhkMoppBvTreeShape"),
    ("compressed_mesh", "bhkCompressedMeshShape"),
])
def test_generate_collision(shape_type, expected_shape_type):
    """generate_collision() creates a full bhkCollisionObject/Body/Shape hierarchy."""
    from creation_lib.nif.operations.collision import generate_collision

    nif = _make_nif()

    # Create a root node
    root = nif.add_block("BSFadeNode")

    # Create a BSTriShape with vertex and triangle data
    mesh = nif.add_block("BSTriShape")
    mesh.set_field("Vertex Data", [
        {"Vertex": {"x": -10, "y": -10, "z": -10}},
        {"Vertex": {"x": 10, "y": -10, "z": -10}},
        {"Vertex": {"x": 10, "y": 10, "z": -10}},
        {"Vertex": {"x": -10, "y": 10, "z": -10}},
        {"Vertex": {"x": -10, "y": -10, "z": 10}},
        {"Vertex": {"x": 10, "y": -10, "z": 10}},
        {"Vertex": {"x": 10, "y": 10, "z": 10}},
        {"Vertex": {"x": -10, "y": 10, "z": 10}},
    ])
    mesh.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2}, {"v1": 0, "v2": 2, "v3": 3},
        {"v1": 4, "v2": 6, "v3": 5}, {"v1": 4, "v2": 7, "v3": 6},
        {"v1": 0, "v2": 4, "v3": 5}, {"v1": 0, "v2": 5, "v3": 1},
        {"v1": 2, "v2": 6, "v3": 7}, {"v1": 2, "v2": 7, "v3": 3},
        {"v1": 0, "v2": 3, "v3": 7}, {"v1": 0, "v2": 7, "v3": 4},
        {"v1": 1, "v2": 5, "v3": 6}, {"v1": 1, "v2": 6, "v3": 2},
    ])
    mesh.set_field("Num Triangles", 12)

    # Link mesh as child of root
    root.set_field("Children", [mesh.block_id])
    root.set_field("Num Children", 1)

    result = generate_collision(nif, 0, shape_type=shape_type,
                                source_block_ids=[mesh.block_id])

    assert result.success, f"Failed: {result.description}"
    assert shape_type in result.description.lower()

    # Verify hierarchy exists
    coll_id = root.get_field("Collision Object")
    assert coll_id is not None and coll_id >= 0

    coll_block = nif.get_block(coll_id)
    assert coll_block.type_name == "bhkCollisionObject"

    body_id = coll_block.get_field("Body")
    body_block = nif.get_block(body_id)
    assert body_block.type_name == "bhkRigidBody"

    shape_id = body_block.get_field("Shape")
    shape_block = nif.get_block(shape_id)
    assert shape_block.type_name == expected_shape_type


# ---- bhkCompressedMeshShape (Skyrim SE) tests ----


def test_create_compressed_mesh_shape():
    """Create bhkCompressedMeshShape from triangle mesh."""
    from creation_lib.nif.operations.collision_mesh import create_compressed_mesh_shape

    nif = _make_nif()

    verts = np.array([
        [0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0],
        [0, 0, 10], [10, 0, 10], [10, 10, 10], [0, 10, 10],
    ], dtype=np.float32)
    tris = np.array([
        [0, 1, 2], [0, 2, 3], [4, 6, 5], [4, 7, 6],
        [0, 4, 5], [0, 5, 1], [2, 6, 7], [2, 7, 3],
        [0, 3, 7], [0, 7, 4], [1, 5, 6], [1, 6, 2],
    ], dtype=np.int32)

    result = create_compressed_mesh_shape(nif, verts, tris)
    assert result is not None
    shape_id, data_id = result

    shape = nif.get_block(shape_id)
    assert shape.type_name == "bhkCompressedMeshShape"

    data = nif.get_block(data_id)
    assert data.type_name == "bhkCompressedMeshShapeData"
    assert data.get_field("Num Big Tris") == 12
    assert data.get_field("Num Big Verts") == 8

    # Verify shape references data
    assert shape.get_field("Data") == data_id

    # Empty geometry should return None.
    empty_verts = np.array([], dtype=np.float32).reshape(0, 3)
    empty_tris = np.array([], dtype=np.int32).reshape(0, 3)
    assert create_compressed_mesh_shape(nif, empty_verts, empty_tris) is None

    # Round trip: extracted geometry matches input within quantization tolerance.
    from creation_lib.nif.operations.collision_mesh import extract_compressed_mesh
    verts_out, tris_out, _ = extract_compressed_mesh(nif, data_id)
    assert len(verts_out) == len(verts)
    assert len(tris_out) == len(tris)
    np.testing.assert_allclose(verts_out, verts, atol=0.1)


# ---- Shape conversion tests ----


def test_convert_convex_to_mopp():
    """Convert bhkConvexVerticesShape -> bhkMoppBvTreeShape."""
    from creation_lib.nif.operations.collision import generate_collision, convert_collision_shape

    nif = _make_nif()

    # Create a root with a mesh
    root = nif.add_block("BSFadeNode")
    mesh = nif.add_block("BSTriShape")
    mesh.set_field("Vertex Data", [
        {"Vertex": {"x": -10, "y": -10, "z": -10}},
        {"Vertex": {"x": 10, "y": -10, "z": -10}},
        {"Vertex": {"x": 10, "y": 10, "z": -10}},
        {"Vertex": {"x": -10, "y": 10, "z": -10}},
        {"Vertex": {"x": -10, "y": -10, "z": 10}},
        {"Vertex": {"x": 10, "y": -10, "z": 10}},
        {"Vertex": {"x": 10, "y": 10, "z": 10}},
        {"Vertex": {"x": -10, "y": 10, "z": 10}},
    ])
    mesh.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2}, {"v1": 0, "v2": 2, "v3": 3},
        {"v1": 4, "v2": 6, "v3": 5}, {"v1": 4, "v2": 7, "v3": 6},
        {"v1": 0, "v2": 4, "v3": 5}, {"v1": 0, "v2": 5, "v3": 1},
        {"v1": 2, "v2": 6, "v3": 7}, {"v1": 2, "v2": 7, "v3": 3},
        {"v1": 0, "v2": 3, "v3": 7}, {"v1": 0, "v2": 7, "v3": 4},
        {"v1": 1, "v2": 5, "v3": 6}, {"v1": 1, "v2": 6, "v3": 2},
    ])
    mesh.set_field("Num Triangles", 12)
    root.set_field("Children", [mesh.block_id])
    root.set_field("Num Children", 1)

    # Generate convex hull collision first
    gen_result = generate_collision(nif, 0, shape_type="convex_hull",
                                    source_block_ids=[mesh.block_id])
    assert gen_result.success, f"Failed to generate: {gen_result.description}"

    # Find the convex shape block
    convex_id = None
    for block in nif.blocks:
        if block.type_name == "bhkConvexVerticesShape":
            convex_id = block.block_id
            break
    assert convex_id is not None, "No bhkConvexVerticesShape found"

    # Convert to MOPP
    result = convert_collision_shape(nif, convex_id, "mopp")
    assert result.success, f"Failed: {result.description}"
    assert "mopp" in result.description.lower()
    assert len(result.modified_block_ids) > 0

    # Chain: convert the resulting MOPP shape to a compressed mesh shape.
    mopp_id = None
    for block in nif.blocks:
        if block.type_name == "bhkMoppBvTreeShape":
            mopp_id = block.block_id
            break
    assert mopp_id is not None, "No bhkMoppBvTreeShape found"
    result = convert_collision_shape(nif, mopp_id, "compressed_mesh")
    assert result.success
    assert "compressed_mesh" in result.description.lower()


def test_convert_unknown_source_or_target_fails_gracefully():
    """Converting from/to an unsupported block type should fail gracefully."""
    from creation_lib.nif.operations.collision import convert_collision_shape
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()
    nif.add_block("NiNode")
    result = convert_collision_shape(nif, 0, "mopp")
    assert not result.success
    assert "Cannot extract" in result.description

    verts = np.array([[0, 0, 0], [10, 0, 0], [0, 10, 0]], dtype=np.float32)
    tris = np.array([[0, 1, 2]], dtype=np.int32)
    mopp_id, _, _ = create_mopp_shape(nif, verts, tris)

    result = convert_collision_shape(nif, mopp_id, "unknown_type")
    assert not result.success
    assert "Unknown target" in result.description
