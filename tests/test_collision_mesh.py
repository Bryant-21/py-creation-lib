"""Tests for complex mesh collision shapes (MOPP, compressed mesh)."""
import os
from pathlib import Path

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


def test_create_mopp_shape_empty():
    """Empty geometry should return None."""
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()
    verts = np.array([], dtype=np.float32).reshape(0, 3)
    tris = np.array([], dtype=np.int32).reshape(0, 3)

    result = create_mopp_shape(nif, verts, tris)
    assert result is None


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


def test_extract_mopp_geometry_wrong_block():
    """Extracting from a non-MOPP block should raise ValueError."""
    from creation_lib.nif.operations.collision_mesh import extract_mopp_geometry

    nif = _make_nif()
    nif.add_block("NiNode")

    with pytest.raises(ValueError, match="not bhkMoppBvTreeShape"):
        extract_mopp_geometry(nif, 0)


def test_generate_collision_mopp():
    """generate_collision() with shape_type='mopp' creates full hierarchy."""
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

    result = generate_collision(nif, 0, shape_type="mopp",
                                source_block_ids=[mesh.block_id])

    assert result.success, f"Failed: {result.description}"
    assert "mopp" in result.description.lower()

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
    assert shape_block.type_name == "bhkMoppBvTreeShape"


def test_mopp_shape_block_references():
    """Verify inter-block references are correct."""
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()

    verts = np.array([
        [0, 0, 0], [10, 0, 0], [0, 10, 0],
    ], dtype=np.float32)
    tris = np.array([[0, 1, 2]], dtype=np.int32)

    mopp_id, packed_id, data_id = create_mopp_shape(nif, verts, tris)

    # bhkMoppBvTreeShape.Shape → bhkPackedNiTriStripsShape
    mopp_block = nif.get_block(mopp_id)
    assert mopp_block.get_field("Shape") == packed_id

    # bhkPackedNiTriStripsShape.Data → hkPackedNiTriStripsData
    packed_block = nif.get_block(packed_id)
    assert packed_block.get_field("Data") == data_id


FO3_EXTRACTED_DIR = Path(
    os.environ.get("FO3_EXTRACTED_DIR")
    or Path(__file__).resolve().parents[2] / "extracted" / "fo3"
)
FO3_PISTOL = str(FO3_EXTRACTED_DIR / "meshes/weapons/1handpistol/10mmpistol.nif")


@pytest.mark.skipif(
    not os.path.exists(FO3_PISTOL),
    reason="FO3 extracted data not available",
)
def test_read_fo3_10mm_pistol_collision():
    """Read and inspect FO3 10mm pistol collision (bhkConvexVerticesShape)."""
    from creation_lib.nif.nif_file import NifFile

    nif = NifFile.load(FO3_PISTOL)

    # Block 4 is bhkConvexVerticesShape (confirmed via modkit nif inspect)
    block = nif.get_block(4)
    assert block.type_name == "bhkConvexVerticesShape"

    verts = block.get_field("Vertices")
    assert len(verts) == 19
    assert verts[0]["x"] == pytest.approx(-0.6357, abs=0.01)

    # Block 5 is bhkRigidBody
    rb = nif.get_block(5)
    assert rb.type_name == "bhkRigidBody"

    # Block 6 is bhkCollisionObject
    co = nif.get_block(6)
    assert co.type_name == "bhkCollisionObject"


def _strips_to_triangles(strips: list[list[int]]) -> list[tuple[int, int, int]]:
    """Convert triangle strip indices to triangle list."""
    tris = []
    for strip in strips:
        for i in range(len(strip) - 2):
            if i % 2 == 0:
                tri = (strip[i], strip[i + 1], strip[i + 2])
            else:
                tri = (strip[i], strip[i + 2], strip[i + 1])
            # Skip degenerate triangles
            if tri[0] != tri[1] and tri[1] != tri[2] and tri[0] != tri[2]:
                tris.append(tri)
    return tris


@pytest.mark.skipif(
    not os.path.exists(FO3_PISTOL),
    reason="FO3 extracted data not available",
)
def test_fo3_mopp_generation():
    """Generate MOPP collision for FO3 10mm pistol mesh and verify output."""
    from creation_lib.nif.nif_file import NifFile
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = NifFile.load(FO3_PISTOL)

    # FO3 uses NiTriStripsData — extract vertices and convert strips to triangles
    verts_list = []
    tris_list = []
    vert_offset = 0
    for block in nif.blocks:
        if block.type_name != "NiTriStripsData":
            continue
        raw_verts = block.get_field("Vertices")
        if not raw_verts or len(raw_verts) < 3:
            continue
        v = np.array(
            [[rv["x"], rv["y"], rv["z"]] for rv in raw_verts],
            dtype=np.float32,
        )
        verts_list.append(v)

        # Convert strips to triangles
        raw_strips = block.get_field("Points") or []
        strip_tris = _strips_to_triangles(raw_strips)
        for tri in strip_tris:
            tris_list.append((tri[0] + vert_offset, tri[1] + vert_offset, tri[2] + vert_offset))
        vert_offset += len(v)

    if not verts_list or not tris_list:
        pytest.skip("No suitable geometry found in FO3 pistol NIF")

    combined_verts = np.vstack(verts_list)
    combined_tris = np.array(tris_list, dtype=np.int32)

    result = create_mopp_shape(nif, combined_verts, combined_tris, radius=0.1)
    assert result is not None

    mopp_id, packed_id, data_id = result
    mopp_block = nif.get_block(mopp_id)
    assert mopp_block.type_name == "bhkMoppBvTreeShape"

    mopp_code = mopp_block.get_field("MOPP Code")
    assert mopp_code is not None
    assert mopp_code.get("Data Size", 0) > 0


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


def test_create_compressed_mesh_shape_empty():
    """Empty geometry should return None."""
    from creation_lib.nif.operations.collision_mesh import create_compressed_mesh_shape

    nif = _make_nif()
    verts = np.array([], dtype=np.float32).reshape(0, 3)
    tris = np.array([], dtype=np.int32).reshape(0, 3)

    result = create_compressed_mesh_shape(nif, verts, tris)
    assert result is None


def test_compressed_mesh_round_trip():
    """Create compressed mesh, extract geometry, verify match."""
    from creation_lib.nif.operations.collision_mesh import (
        create_compressed_mesh_shape, extract_compressed_mesh,
    )

    nif = _make_nif()

    verts_in = np.array([
        [0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0],
    ], dtype=np.float32)
    tris_in = np.array([[0, 1, 2], [0, 2, 3]], dtype=np.int32)

    shape_id, data_id = create_compressed_mesh_shape(nif, verts_in, tris_in)

    verts_out, tris_out, _ = extract_compressed_mesh(nif, data_id)

    assert len(verts_out) == len(verts_in)
    assert len(tris_out) == len(tris_in)
    # Vertices should be approximately equal (quantization introduces small error)
    np.testing.assert_allclose(verts_out, verts_in, atol=0.1)


def test_extract_compressed_mesh_wrong_block():
    """Extracting from a non-compressed-mesh block should raise ValueError."""
    from creation_lib.nif.operations.collision_mesh import extract_compressed_mesh

    nif = _make_nif()
    nif.add_block("NiNode")

    with pytest.raises(ValueError, match="not bhkCompressedMeshShapeData"):
        extract_compressed_mesh(nif, 0)


def test_generate_collision_compressed_mesh():
    """generate_collision() with shape_type='compressed_mesh' creates hierarchy."""
    from creation_lib.nif.operations.collision import generate_collision

    nif = _make_nif()

    root = nif.add_block("BSFadeNode")
    mesh = nif.add_block("BSTriShape")
    mesh.set_field("Vertex Data", [
        {"Vertex": {"x": 0, "y": 0, "z": 0}},
        {"Vertex": {"x": 10, "y": 0, "z": 0}},
        {"Vertex": {"x": 10, "y": 10, "z": 0}},
        {"Vertex": {"x": 0, "y": 10, "z": 0}},
    ])
    mesh.set_field("Triangles", [
        {"v1": 0, "v2": 1, "v3": 2}, {"v1": 0, "v2": 2, "v3": 3},
    ])
    mesh.set_field("Num Triangles", 2)
    root.set_field("Children", [mesh.block_id])
    root.set_field("Num Children", 1)

    result = generate_collision(nif, 0, shape_type="compressed_mesh",
                                source_block_ids=[mesh.block_id])

    assert result.success, f"Failed: {result.description}"
    assert "compressed_mesh" in result.description.lower()

    coll_id = root.get_field("Collision Object")
    assert coll_id is not None and coll_id >= 0

    shape_id = nif.get_block(nif.get_block(coll_id).get_field("Body")).get_field("Shape")
    shape_block = nif.get_block(shape_id)
    assert shape_block.type_name == "bhkCompressedMeshShape"


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


def test_convert_mopp_to_compressed_mesh():
    """Convert bhkMoppBvTreeShape -> bhkCompressedMeshShape."""
    from creation_lib.nif.operations.collision import convert_collision_shape
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()

    verts = np.array([
        [0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0],
    ], dtype=np.float32)
    tris = np.array([[0, 1, 2], [0, 2, 3]], dtype=np.int32)

    mopp_id, _, _ = create_mopp_shape(nif, verts, tris)

    result = convert_collision_shape(nif, mopp_id, "compressed_mesh")
    assert result.success
    assert "compressed_mesh" in result.description.lower()


def test_convert_unknown_source():
    """Converting from an unsupported block type should fail gracefully."""
    from creation_lib.nif.operations.collision import convert_collision_shape

    nif = _make_nif()
    nif.add_block("NiNode")

    result = convert_collision_shape(nif, 0, "mopp")
    assert not result.success
    assert "Cannot extract" in result.description


def test_convert_unknown_target():
    """Converting to an unknown target type should fail gracefully."""
    from creation_lib.nif.operations.collision import convert_collision_shape
    from creation_lib.nif.operations.collision_mesh import create_mopp_shape

    nif = _make_nif()

    verts = np.array([[0, 0, 0], [10, 0, 0], [0, 10, 0]], dtype=np.float32)
    tris = np.array([[0, 1, 2]], dtype=np.int32)
    mopp_id, _, _ = create_mopp_shape(nif, verts, tris)

    result = convert_collision_shape(nif, mopp_id, "unknown_type")
    assert not result.success
    assert "Unknown target" in result.description
