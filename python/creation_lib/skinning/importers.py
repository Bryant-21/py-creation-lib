"""Mesh importers — load geometry from common file formats into SkinData."""
from __future__ import annotations

import logging
from pathlib import Path

import numpy as np

from creation_lib.geometry.obj_import import load_obj_geometry

from .skin_data import SkinData

_log = logging.getLogger("skinning.importers")


def import_obj(path: str | Path) -> SkinData:
    """Import an OBJ file into SkinData (geometry only, empty weight arrays).

    Reads v, vn, vt, and f (triangles and quads, auto-triangulated; face formats
    v, v/vt, v/vt/vn, v//vn). Raises FileNotFoundError for a missing file and
    ValueError when no geometry is found.
    """
    path = Path(path)
    if not path.exists():
        raise FileNotFoundError(f"OBJ file not found: {path}")

    geometry = load_obj_geometry(path)
    n = len(geometry.vertices)
    m = len(geometry.triangles)

    _log.info("Imported OBJ: %d vertices, %d triangles from %s", n, m, path.name)

    return SkinData(
        vertices=np.array(geometry.vertices, dtype=np.float32),
        triangles=np.array(geometry.triangles, dtype=np.uint32),
        normals=np.array(geometry.normals, dtype=np.float32),
        uvs=np.array(geometry.uvs, dtype=np.float32),
        bone_names=[],
        weights=np.zeros((n, 4), dtype=np.float32),
        bone_indices=np.zeros((n, 4), dtype=np.int32),
        segment_ids=np.full(m, -1, dtype=np.int32),
        max_bones_per_vertex=4,
    )


def import_fbx(path: str | Path) -> SkinData:
    """Import an FBX file into SkinData using Autodesk FBX SDK."""
    from creation_lib.fbx import load_fbx_skin_data

    path = Path(path)
    if not path.exists():
        raise FileNotFoundError(f"FBX file not found: {path}")

    skin_data = load_fbx_skin_data(path)
    _log.info(
        "Imported FBX: %d vertices, %d triangles from %s",
        skin_data.num_vertices,
        skin_data.num_triangles,
        path.name,
    )
    return skin_data
