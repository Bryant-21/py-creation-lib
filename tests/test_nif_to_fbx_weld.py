"""Tests for the NIF→FBX vertex weld pass.

NIF BSTriShape stores split verts (own UV/normal/tangent per vert). The FBX
exporter must weld verts that share a position so Max/other DCCs see one
control point per unique position, with UV/normal seams preserved on polygon
corners.
"""
from __future__ import annotations

import shutil
from pathlib import Path

import pytest

from creation_lib.fbx.nif_to_fbx import FbxExportOptions, _build_weld_map, export_nif_to_fbx
from creation_lib.fbx.sdk import HAS_FBX, fbx
from creation_lib.nif.nif_file import NifFile


def _test_dir(name: str) -> Path:
    root = Path(__file__).resolve().parents[2] / "output" / "fbx_weld_tests" / name
    if root.exists():
        shutil.rmtree(root)
    root.mkdir(parents=True, exist_ok=True)
    return root


def _build_split_vert_nif() -> NifFile:
    """Build a minimal NIF with one BSTriShape containing a UV seam.

    Two triangles sharing an edge (3 unique positions each side = 4 unique
    positions total), but the shared edge is a UV seam so it's stored as
    6 split verts instead of 4. Layout:

      v0 ---- v1/v4        positions:   v0=(0,0,0)   v1=(1,0,0)
      |  tri0  / |                       v2=(0,1,0)   v3=(1,1,0)
      |       /  |         seam split:   v4 shares pos with v1
      |      /   |                       v5 shares pos with v2
      v2/v5 ---- v3

      tri0 = (v0, v1, v2)     UVs on "left" chart
      tri1 = (v4, v3, v5)     UVs on "right" chart (different UVs at seam)
    """
    nif = NifFile.new("fo4")
    root = nif.get_block(0)
    root.set_field("Name", "Root")

    shape = nif.add_block("BSTriShape")
    shape.set_field("Name", "SeamQuad")
    shape.set_field(
        "Vertex Data",
        [
            # v0: corner, left chart
            {
                "Vertex": {"x": 0.0, "y": 0.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 0.0, "v": 0.0},
            },
            # v1: seam vert, left chart UV
            {
                "Vertex": {"x": 1.0, "y": 0.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 0.49, "v": 0.0},
            },
            # v2: seam vert, left chart UV
            {
                "Vertex": {"x": 0.0, "y": 1.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 0.49, "v": 1.0},
            },
            # v3: corner, right chart
            {
                "Vertex": {"x": 1.0, "y": 1.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 1.0, "v": 1.0},
            },
            # v4: same POS as v1, right chart UV (UV seam split)
            {
                "Vertex": {"x": 1.0, "y": 0.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 0.51, "v": 0.0},
            },
            # v5: same POS as v2, right chart UV (UV seam split)
            {
                "Vertex": {"x": 0.0, "y": 1.0, "z": 0.0},
                "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "UV": {"u": 0.51, "v": 1.0},
            },
        ],
    )
    shape.set_field("Num Vertices", 6)
    shape.set_field(
        "Triangles",
        [
            {"v1": 0, "v2": 1, "v3": 2},
            {"v1": 4, "v2": 3, "v3": 5},
        ],
    )
    shape.set_field("Num Triangles", 2)
    shape.set_field("Num Children", 0)
    shape.set_field("Children", [])
    shape.set_field("Num Extra Data List", 0)
    shape.set_field("Extra Data List", [])

    root.set_field("Children", [shape.block_id])
    root.set_field("Num Children", 1)
    return nif


def test_build_weld_map_collapses_split_verts():
    nif = _build_split_vert_nif()
    shape = next(b for b in nif.blocks if b.type_name == "BSTriShape")
    vertex_data = shape.get_field("Vertex Data")

    orig_to_welded, welded_positions = _build_weld_map(vertex_data)

    # 6 split verts collapse to 4 unique positions
    assert len(orig_to_welded) == 6
    assert len(welded_positions) == 4

    # v1 and v4 share a position → same welded idx
    assert orig_to_welded[1] == orig_to_welded[4]
    # v2 and v5 share a position → same welded idx
    assert orig_to_welded[2] == orig_to_welded[5]
    # v0 and v3 have unique positions
    assert len({orig_to_welded[0], orig_to_welded[1], orig_to_welded[2], orig_to_welded[3]}) == 4


def test_build_weld_map_rounds_to_six_decimals():
    """Positions within 1e-6 of each other should weld; outside should not."""
    vertex_data = [
        {"Vertex": {"x": 1.0, "y": 2.0, "z": 3.0}},
        {"Vertex": {"x": 1.0000001, "y": 2.0, "z": 3.0}},  # within 1e-6 → weld
        {"Vertex": {"x": 1.001, "y": 2.0, "z": 3.0}},       # 1e-3 away → distinct
    ]
    orig_to_welded, welded_positions = _build_weld_map(vertex_data)
    assert orig_to_welded[0] == orig_to_welded[1]
    assert orig_to_welded[0] != orig_to_welded[2]
    assert len(welded_positions) == 2


@pytest.mark.skipif(not HAS_FBX, reason="Autodesk FBX SDK not installed")
def test_export_welds_control_points_and_preserves_uv_seam():
    """Round-trip: split-vert NIF → FBX → reload FBX → verify weld + seam."""
    tmp = _test_dir("weld_roundtrip")
    out_path = tmp / "seam_quad.fbx"

    nif = _build_split_vert_nif()
    # Disable skeleton/material/weight paths — they're irrelevant to the
    # weld assertion and the default skeleton path trips on the list-form
    # Translation produced by NifFile.new() (pre-existing limitation, out
    # of scope for this change).
    opts = FbxExportOptions(
        include_skeleton=False,
        include_weights=False,
        include_materials=False,
    )
    result = export_nif_to_fbx(nif, str(out_path), options=opts)
    assert result == str(out_path), "export_nif_to_fbx should return output path"
    assert out_path.exists()

    # Reload with FBX SDK and inspect the mesh
    manager = fbx.FbxManager.Create()
    try:
        ios = fbx.FbxIOSettings.Create(manager, "IOSROOT")
        manager.SetIOSettings(ios)
        scene = fbx.FbxScene.Create(manager, "ReloadScene")

        importer = fbx.FbxImporter.Create(manager, "")
        assert importer.Initialize(str(out_path), -1, ios), (
            f"FBX importer failed to open: {out_path}"
        )
        assert importer.Import(scene)
        importer.Destroy()

        # Find the SeamQuad mesh node
        mesh_node = None
        root_node = scene.GetRootNode()

        def _walk(node):
            nonlocal mesh_node
            for i in range(node.GetChildCount()):
                child = node.GetChild(i)
                attr = child.GetNodeAttribute()
                if attr and attr.GetAttributeType() == fbx.FbxNodeAttribute.EType.eMesh:
                    if child.GetName() == "SeamQuad":
                        mesh_node = child
                        return
                _walk(child)

        _walk(root_node)
        assert mesh_node is not None, "SeamQuad mesh not found in reloaded FBX"

        mesh = mesh_node.GetNodeAttribute()

        # --- Weld assertion: 6 split verts → 4 welded control points ---
        assert mesh.GetControlPointsCount() == 4, (
            f"Expected 4 welded control points, got {mesh.GetControlPointsCount()}"
        )

        # --- Polygon count ---
        assert mesh.GetPolygonCount() == 2

        # --- UV seam preserved: per-polygon-vertex UVs with 6 UV entries ---
        uv_element = mesh.GetElementUV(0)
        assert uv_element is not None
        assert (
            uv_element.GetMappingMode()
            == fbx.FbxLayerElement.EMappingMode.eByPolygonVertex
        ), "UVs must be per-polygon-vertex to preserve seams"
        assert (
            uv_element.GetReferenceMode()
            == fbx.FbxLayerElement.EReferenceMode.eIndexToDirect
        )
        # Direct array: one UV per original NIF vert (6 split verts)
        assert uv_element.GetDirectArray().GetCount() == 6
        # Index array: 2 tris × 3 corners = 6 entries
        assert uv_element.GetIndexArray().GetCount() == 6

        # --- Normals also per-polygon-vertex ---
        norm_element = mesh.GetElementNormal(0)
        assert norm_element is not None
        assert (
            norm_element.GetMappingMode()
            == fbx.FbxLayerElement.EMappingMode.eByPolygonVertex
        )

    finally:
        manager.Destroy()


@pytest.mark.skipif(not HAS_FBX, reason="Autodesk FBX SDK not installed")
def test_export_preserves_unwelded_mesh_counts():
    """A mesh with no duplicate positions should produce 1:1 control points."""
    tmp = _test_dir("no_weld_needed")
    out_path = tmp / "triangle.fbx"

    nif = NifFile.new("fo4")
    root = nif.get_block(0)
    root.set_field("Name", "Root")

    shape = nif.add_block("BSTriShape")
    shape.set_field("Name", "Tri")
    shape.set_field(
        "Vertex Data",
        [
            {"Vertex": {"x": 0.0, "y": 0.0, "z": 0.0},
             "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
             "UV": {"u": 0.0, "v": 0.0}},
            {"Vertex": {"x": 1.0, "y": 0.0, "z": 0.0},
             "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
             "UV": {"u": 1.0, "v": 0.0}},
            {"Vertex": {"x": 0.0, "y": 1.0, "z": 0.0},
             "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
             "UV": {"u": 0.0, "v": 1.0}},
        ],
    )
    shape.set_field("Num Vertices", 3)
    shape.set_field("Triangles", [{"v1": 0, "v2": 1, "v3": 2}])
    shape.set_field("Num Triangles", 1)
    shape.set_field("Num Children", 0)
    shape.set_field("Children", [])
    shape.set_field("Num Extra Data List", 0)
    shape.set_field("Extra Data List", [])
    root.set_field("Children", [shape.block_id])
    root.set_field("Num Children", 1)

    opts = FbxExportOptions(
        include_skeleton=False,
        include_weights=False,
        include_materials=False,
    )
    assert export_nif_to_fbx(nif, str(out_path), options=opts) == str(out_path)

    manager = fbx.FbxManager.Create()
    try:
        ios = fbx.FbxIOSettings.Create(manager, "IOSROOT")
        manager.SetIOSettings(ios)
        scene = fbx.FbxScene.Create(manager, "ReloadScene")
        importer = fbx.FbxImporter.Create(manager, "")
        assert importer.Initialize(str(out_path), -1, ios)
        assert importer.Import(scene)
        importer.Destroy()

        mesh_node = None
        root_node = scene.GetRootNode()

        def _walk(node):
            nonlocal mesh_node
            for i in range(node.GetChildCount()):
                child = node.GetChild(i)
                attr = child.GetNodeAttribute()
                if (
                    attr
                    and attr.GetAttributeType() == fbx.FbxNodeAttribute.EType.eMesh
                    and child.GetName() == "Tri"
                ):
                    mesh_node = child
                    return
                _walk(child)

        _walk(root_node)
        assert mesh_node is not None

        mesh = mesh_node.GetNodeAttribute()
        assert mesh.GetControlPointsCount() == 3
        assert mesh.GetPolygonCount() == 1
    finally:
        manager.Destroy()
