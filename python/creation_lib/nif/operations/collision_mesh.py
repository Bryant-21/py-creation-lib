"""Complex mesh collision shapes -- MOPP and compressed mesh.

Creates bhkMoppBvTreeShape (Skyrim/FO3) and bhkCompressedMeshShape (Skyrim SE)
hierarchies from triangle mesh geometry. Complements collision.py (simple shapes).
"""
from __future__ import annotations

from typing import TYPE_CHECKING

import numpy as np

from .collision import HAVOK_SCALE

if TYPE_CHECKING:
    from creation_lib.nif.nif_file import NifFile

# Havok radius defaults per game era
RADIUS_SKYRIM = 0.005
RADIUS_OBLIVION = 0.1  # Also FO3/FNV


def create_mopp_shape(
    nif: NifFile,
    verts_nif: np.ndarray,
    triangles: np.ndarray,
    radius: float = RADIUS_SKYRIM,
    havok_scale_factor: float = HAVOK_SCALE,
    material: int = 0,
) -> tuple[int, int, int] | None:
    """Create bhkMoppBvTreeShape -> bhkPackedNiTriStripsShape -> hkPackedNiTriStripsData.

    ``verts_nif`` are in NIF space. ``radius`` is 0.005 for Skyrim, 0.1 for
    FO3/Oblivion; ``material`` applies to every triangle. Returns
    ``(mopp_block_id, packed_shape_id, data_block_id)`` or None.
    """
    from .mopp_compiler import compile_mopp

    if len(triangles) == 0:
        return None

    havok_scale = 1.0 / havok_scale_factor
    scaled_verts = verts_nif * havok_scale

    # Convert to tuples for MOPP compiler
    vert_tuples = [(float(v[0]), float(v[1]), float(v[2])) for v in scaled_verts]
    tri_tuples = [(int(t[0]), int(t[1]), int(t[2])) for t in triangles]

    mopp_bytes, origin, mopp_scale = compile_mopp(
        vert_tuples, tri_tuples, radius=radius
    )
    if not mopp_bytes:
        return None

    # Create hkPackedNiTriStripsData
    data_block = nif.add_block("hkPackedNiTriStripsData")

    # Set triangle data
    tri_data = []
    for tri in triangles:
        tri_data.append({
            "Triangle": {
                "v1": int(tri[0]),
                "v2": int(tri[1]),
                "v3": int(tri[2]),
            },
            "Welding Info": 0,
            "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
        })
    data_block.set_field("Num Triangles", len(triangles))
    data_block.set_field("Triangles", tri_data)

    # Set vertex data (in Havok space)
    vert_data = [
        {"x": float(v[0]), "y": float(v[1]), "z": float(v[2])}
        for v in scaled_verts
    ]
    data_block.set_field("Num Vertices", len(scaled_verts))
    data_block.set_field("Vertices", vert_data)

    # Create bhkPackedNiTriStripsShape
    packed_block = nif.add_block("bhkPackedNiTriStripsShape")
    packed_block.set_field("Radius", radius)
    packed_block.set_field(
        "Scale", {"x": 1.0, "y": 1.0, "z": 1.0, "w": 0.0}
    )
    packed_block.set_field("Radius Copy", radius)
    packed_block.set_field(
        "Scale Copy", {"x": 1.0, "y": 1.0, "z": 1.0, "w": 0.0}
    )
    packed_block.set_field("Data", data_block.block_id)

    # Create bhkMoppBvTreeShape
    mopp_block = nif.add_block("bhkMoppBvTreeShape")
    mopp_block.set_field("Shape", packed_block.block_id)
    mopp_block.set_field("Scale", 1.0)
    mopp_block.set_field("MOPP Code", {
        "Offset": {
            "x": origin[0],
            "y": origin[1],
            "z": origin[2],
            "w": mopp_scale,
        },
        "Data Size": len(mopp_bytes),
        "Data": list(mopp_bytes),
    })

    return mopp_block.block_id, packed_block.block_id, data_block.block_id


def extract_mopp_geometry(
    nif: NifFile,
    mopp_block_id: int,
    havok_scale_factor: float = HAVOK_SCALE,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Extract mesh geometry from a bhkMoppBvTreeShape.

    Returns:
        (vertices_nif_space, triangles, material_indices)
    """
    mopp_block = nif.get_block(mopp_block_id)
    if not mopp_block or mopp_block.type_name != "bhkMoppBvTreeShape":
        raise ValueError(f"Block {mopp_block_id} is not bhkMoppBvTreeShape")

    packed_id = mopp_block.get_field("Shape")
    packed_block = nif.get_block(packed_id)
    if not packed_block:
        raise ValueError("Missing bhkPackedNiTriStripsShape")

    data_id = packed_block.get_field("Data")
    data_block = nif.get_block(data_id)
    if not data_block:
        raise ValueError("Missing hkPackedNiTriStripsData")

    # Extract vertices (Havok space -> NIF space)
    raw_verts = data_block.get_field("Vertices") or []
    verts = np.array(
        [[v["x"], v["y"], v["z"]] for v in raw_verts],
        dtype=np.float32,
    ) * havok_scale_factor

    # Extract triangles
    raw_tris = data_block.get_field("Triangles") or []
    tris = np.array(
        [
            [t["Triangle"]["v1"], t["Triangle"]["v2"], t["Triangle"]["v3"]]
            for t in raw_tris
        ],
        dtype=np.int32,
    )

    # Material indices (from sub shapes if present, else zeros)
    materials = np.zeros(len(tris), dtype=np.int32)

    return verts, tris, materials


# ---------------------------------------------------------------------------
# bhkCompressedMeshShape (Skyrim SE)
# ---------------------------------------------------------------------------


def create_compressed_mesh_shape(
    nif: NifFile,
    verts_nif: np.ndarray,
    triangles: np.ndarray,
    havok_scale_factor: float = HAVOK_SCALE,
    material: int = 0,
    radius: float = RADIUS_SKYRIM,
) -> tuple[int, int] | None:
    """Create bhkCompressedMeshShape + bhkCompressedMeshShapeData.

    Writes only the uncompressed "big verts/tris" arrays, no chunks. ``verts_nif``
    are in NIF space; ``material`` applies to every triangle. Returns
    ``(shape_block_id, data_block_id)`` or None.
    """
    if len(triangles) == 0:
        return None

    havok_scale = 1.0 / havok_scale_factor
    scaled_verts = verts_nif * havok_scale

    # Compute AABB in Havok space
    mins = scaled_verts.min(axis=0)
    maxs = scaled_verts.max(axis=0)

    # Create bhkCompressedMeshShapeData
    data_block = nif.add_block("bhkCompressedMeshShapeData")

    data_block.set_field("Bits Per Index", 17)
    data_block.set_field("Bits Per W Index", 18)
    data_block.set_field("Mask W Index", 0x3FFFF)
    data_block.set_field("Mask Index", 0x1FFFF)
    data_block.set_field("Error", 0.001)
    data_block.set_field("AABB", {
        "Min": {
            "x": float(mins[0]), "y": float(mins[1]),
            "z": float(mins[2]), "w": 0.0,
        },
        "Max": {
            "x": float(maxs[0]), "y": float(maxs[1]),
            "z": float(maxs[2]), "w": 0.0,
        },
    })

    # Default material
    data_block.set_field("Num Materials", 1)
    data_block.set_field("Chunk Materials", [{
        "Material": material,
        "Filter": {"Layer": 1, "Flags and Part Number": 0, "Group": 0},
    }])

    # Default transform (identity)
    data_block.set_field("Num Transforms", 1)
    data_block.set_field("Chunk Transforms", [{
        "Translation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 0.0},
        "Rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
    }])

    # Big verts (uncompressed Vector4 in Havok space)
    big_verts = [
        {"x": float(v[0]), "y": float(v[1]), "z": float(v[2]), "w": 0.0}
        for v in scaled_verts
    ]
    data_block.set_field("Num Big Verts", len(big_verts))
    data_block.set_field("Big Verts", big_verts)

    # Big tris (uncompressed)
    big_tris = []
    for tri in triangles:
        big_tris.append({
            "Triangle": {
                "v1": int(tri[0]),
                "v2": int(tri[1]),
                "v3": int(tri[2]),
            },
            "Material": material,
            "Welding Info": 0,
        })
    data_block.set_field("Num Big Tris", len(big_tris))
    data_block.set_field("Big Tris", big_tris)

    # No chunks (big verts/tris path)
    data_block.set_field("Num Chunks", 0)
    data_block.set_field("Chunks", [])

    # Unused material arrays
    data_block.set_field("Num Materials 32", 0)
    data_block.set_field("Materials 32", [])
    data_block.set_field("Num Materials 16", 0)
    data_block.set_field("Materials 16", [])
    data_block.set_field("Num Materials 8", 0)
    data_block.set_field("Materials 8", [])
    data_block.set_field("Num Named Materials", 0)
    data_block.set_field("Num Convex Piece A", 0)

    # Create bhkCompressedMeshShape
    shape_block = nif.add_block("bhkCompressedMeshShape")
    shape_block.set_field("Radius", radius)
    shape_block.set_field("Radius Copy", radius)
    shape_block.set_field("Scale", {"x": 1.0, "y": 1.0, "z": 1.0, "w": 0.0})
    shape_block.set_field("Scale Copy", {"x": 1.0, "y": 1.0, "z": 1.0, "w": 0.0})
    shape_block.set_field("Data", data_block.block_id)

    return shape_block.block_id, data_block.block_id


def extract_compressed_mesh(
    nif: NifFile,
    data_block_id: int,
    havok_scale_factor: float = HAVOK_SCALE,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Extract mesh geometry from bhkCompressedMeshShapeData.

    Handles both 'Big' (uncompressed) geometry and per-chunk compressed geometry.

    Returns:
        (vertices_nif_space, triangles, material_indices)
    """
    data_block = nif.get_block(data_block_id)
    if not data_block or data_block.type_name != "bhkCompressedMeshShapeData":
        raise ValueError(
            f"Block {data_block_id} is not bhkCompressedMeshShapeData"
        )

    all_verts: list[list[float]] = []
    all_tris: list[list[int]] = []
    all_materials: list[int] = []

    # 1. Big verts/tris (uncompressed, global vertices)
    big_verts = data_block.get_field("Big Verts") or []
    big_tris = data_block.get_field("Big Tris") or []

    if big_verts:
        for v in big_verts:
            all_verts.append([
                v["x"] * havok_scale_factor,
                v["y"] * havok_scale_factor,
                v["z"] * havok_scale_factor,
            ])

    for bt in big_tris:
        t = bt.get("Triangle", bt)
        all_tris.append([t["v1"], t["v2"], t["v3"]])
        all_materials.append(bt.get("Material", 0))

    # 2. Per-chunk compressed geometry
    transforms = data_block.get_field("Chunk Transforms") or []
    chunks = data_block.get_field("Chunks") or []

    for chunk in chunks:
        chunk_verts = chunk.get("Vertices", [])
        chunk_indices = chunk.get("Indices", [])
        chunk_strips = chunk.get("Strip Lengths", [])
        chunk_offset = chunk.get("Offset", {})
        material_idx = chunk.get("Material Index", 0)

        # Decode compressed vertices: ushort coords / 1000, plus chunk offset
        chunk_vert_base = len(all_verts)
        ox = chunk_offset.get("x", 0.0)
        oy = chunk_offset.get("y", 0.0)
        oz = chunk_offset.get("z", 0.0)

        for cv in chunk_verts:
            x = (cv.get("x", 0) / 1000.0 + ox) * havok_scale_factor
            y = (cv.get("y", 0) / 1000.0 + oy) * havok_scale_factor
            z = (cv.get("z", 0) / 1000.0 + oz) * havok_scale_factor
            all_verts.append([x, y, z])

        # Decode triangle strips to triangles
        idx_pos = 0
        if chunk_strips:
            for strip_len in chunk_strips:
                for i in range(strip_len - 2):
                    if i % 2 == 0:
                        all_tris.append([
                            chunk_indices[idx_pos + i] + chunk_vert_base,
                            chunk_indices[idx_pos + i + 1] + chunk_vert_base,
                            chunk_indices[idx_pos + i + 2] + chunk_vert_base,
                        ])
                    else:
                        all_tris.append([
                            chunk_indices[idx_pos + i] + chunk_vert_base,
                            chunk_indices[idx_pos + i + 2] + chunk_vert_base,
                            chunk_indices[idx_pos + i + 1] + chunk_vert_base,
                        ])
                    all_materials.append(material_idx)
                idx_pos += strip_len
        else:
            # Non-strip: plain triangle list
            for i in range(0, len(chunk_indices) - 2, 3):
                all_tris.append([
                    chunk_indices[i] + chunk_vert_base,
                    chunk_indices[i + 1] + chunk_vert_base,
                    chunk_indices[i + 2] + chunk_vert_base,
                ])
                all_materials.append(material_idx)

    verts = (
        np.array(all_verts, dtype=np.float32)
        if all_verts
        else np.empty((0, 3), dtype=np.float32)
    )
    tris = (
        np.array(all_tris, dtype=np.int32)
        if all_tris
        else np.empty((0, 3), dtype=np.int32)
    )
    materials = (
        np.array(all_materials, dtype=np.int32)
        if all_materials
        else np.empty(0, dtype=np.int32)
    )

    return verts, tris, materials
