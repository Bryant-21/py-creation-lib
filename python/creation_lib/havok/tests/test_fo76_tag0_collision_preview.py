from pathlib import Path

import pytest

from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob
from creation_lib.havok.native_runtime import collision_summary_native
from creation_lib.nif.nif_file import NifFile


def _fo76_fissure_collision_blob() -> bytes:
    path = (
        Path(__file__).resolve().parents[5]
        / "extracted"
        / "fo76"
        / "meshes"
        / "landscape"
        / "fissure"
        / "fissurerocklarge02.nif"
    )
    if not path.exists():
        pytest.skip("FO76 fissure fixture is not available")

    nif = NifFile.load(str(path))
    physics = nif.get_block(3)
    binary_data = physics.get_field("Binary Data")
    return bytes(binary_data["Data"])


def _fo76_tree_log_collision_blob() -> bytes:
    path = (
        Path(__file__).resolve().parents[5]
        / "extracted"
        / "fo76"
        / "meshes"
        / "landscape"
        / "trees"
        / "treeforestlog03.nif"
    )
    if not path.exists():
        pytest.skip("FO76 tree log fixture is not available")

    nif = NifFile.load(str(path))
    physics = nif.get_block(3)
    binary_data = physics.get_field("Binary Data")
    return bytes(binary_data["Data"])


def _fo76_scol_cm005627d3_collision_blob() -> bytes:
    path = (
        Path(__file__).resolve().parents[5]
        / "extracted"
        / "fo76"
        / "meshes"
        / "scol"
        / "seventysix.esm"
        / "cm005627d3.nif"
    )
    if not path.exists():
        pytest.skip("FO76 CM005627D3 fixture is not available")

    nif = NifFile.load(str(path))
    physics = nif.get_block(14)
    binary_data = physics.get_field("Binary Data")
    return bytes(binary_data["Data"])


def test_fo76_2015_tag0_convex_polytope_preview_uses_hkvector4_vertices():
    blob = _fo76_fissure_collision_blob()

    meshes = extract_preview_meshes_from_blob(blob, havok_scale=69.99125, body_id=0)

    assert len(meshes) == 1
    mesh = meshes[0]["mesh"]
    assert len(mesh["vertices"]) == 92
    assert len(mesh["triangles"]) > 0


def test_fo76_2015_tag0_convex_polytope_summary_reports_payload_geometry():
    blob = _fo76_fissure_collision_blob()

    summary = collision_summary_native(blob)

    assert summary["geometry_status"] == "ok"
    polytope = next(
        obj for obj in summary["objects"] if obj["class_name"] == "hknpConvexPolytopeShape"
    )
    assert polytope["n_vertices"] == 92
    assert polytope["n_faces"] == 107
    assert polytope["geometry_source"] == "tag0_payload"


def test_fo76_2015_tag0_compound_shape_body_preview_uses_child_capsules():
    blob = _fo76_tree_log_collision_blob()

    meshes = extract_preview_meshes_from_blob(blob, havok_scale=69.99125, body_id=0)

    assert len(meshes) == 5
    assert {mesh["shape_type"] for mesh in meshes} == {"capsule"}
    assert all(len(mesh["mesh"]["vertices"]) > 0 for mesh in meshes)
    assert all(len(mesh["mesh"]["triangles"]) > 0 for mesh in meshes)


def test_fo76_2015_tag0_compound_shape_summary_classifies_compound():
    blob = _fo76_tree_log_collision_blob()

    summary = collision_summary_native(blob)

    assert summary["shape_kind"] == "compound_polytope"
    assert summary["n_subshapes"] == 5
    assert any(obj["class_name"] == "hknpCompoundShape" for obj in summary["objects"])


def test_fo76_2015_tag0_scol_cm005627d3_compressed_mesh_preview():
    blob = _fo76_scol_cm005627d3_collision_blob()

    meshes = extract_preview_meshes_from_blob(blob, havok_scale=69.99125, body_id=0)

    assert len(meshes) == 1
    mesh = meshes[0]["mesh"]
    assert len(mesh["triangles"]) == 240
    assert all(
        0 <= vertex_index < len(mesh["vertices"])
        for triangle in mesh["triangles"]
        for vertex_index in (triangle["v1"], triangle["v2"], triangle["v3"])
    )
