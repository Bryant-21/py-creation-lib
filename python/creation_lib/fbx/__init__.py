"""Autodesk FBX SDK-backed import and export helpers."""

from .import_fbx import (
    extract_fbx_meshes,
    import_fbx_scene,
    import_fbx_to_shape,
    load_fbx_skin_data,
)
from .nif_to_fbx import export_nif_to_fbx, FbxExportOptions
from .sdk import HAS_FBX, fbx

__all__ = [
    "HAS_FBX",
    "fbx",
    "extract_fbx_meshes",
    "import_fbx_scene",
    "import_fbx_to_shape",
    "load_fbx_skin_data",
    "export_nif_to_fbx",
    "FbxExportOptions",
]
