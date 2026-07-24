"""Thin Python boundary for native havok_native entrypoints."""
from __future__ import annotations

import json
from importlib import import_module
import os
import threading
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_LOAD_LOCK = threading.Lock()


def _looks_like_native_module(module: Any | None) -> bool:
    if module is None:
        return False
    return callable(getattr(module, "hkx_roundtrip_bytes", None))


def configure_native_resources() -> None:
    if "CREATION_LIB_RESOURCE_DIR" in os.environ:
        return
    try:
        from creation_lib.paths import get_resource_dir

        os.environ["CREATION_LIB_RESOURCE_DIR"] = str(get_resource_dir())
    except Exception:
        pass


def _load_umbrella_submodule() -> Any | None:
    configure_native_resources()
    try:
        umbrella = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(umbrella, "havok_native", None)
    if _looks_like_native_module(module):
        return module
    try:
        extension = import_module("creation_lib._native")
    except ImportError:
        return None
    module = getattr(extension, "havok_native", None)
    return module if _looks_like_native_module(module) else None


def load_native_module() -> Any | None:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        return _NATIVE_MODULE
    with _NATIVE_LOAD_LOCK:
        if _NATIVE_IMPORT_ATTEMPTED:
            return _NATIVE_MODULE
        try:
            configure_native_resources()
            mod = import_module("havok_native")
            if not _looks_like_native_module(mod):
                mod = import_module("havok_native.havok_native")
            if _looks_like_native_module(mod):
                _NATIVE_MODULE = mod
        except ImportError:
            pass
        if _NATIVE_MODULE is None:
            _NATIVE_MODULE = _load_umbrella_submodule()
        _NATIVE_IMPORT_ATTEMPTED = True
        return _NATIVE_MODULE


def native_available() -> bool:
    return load_native_module() is not None


def _require_native() -> Any:
    module = load_native_module()
    if module is None:
        raise RuntimeError("havok_native is not available")
    return module


# ---------------------------------------------------------------------------
# HKX format utilities
# ---------------------------------------------------------------------------

def hkx_detect_format_raw(data: bytes) -> str:
    result = _require_native().hkx_detect_format(data)
    if isinstance(result, tuple):
        return str(result[0])
    return str(result)


def hkx_roundtrip_bytes_raw(data: bytes) -> bytes:
    return bytes(_require_native().hkx_roundtrip_bytes(data))


# ---------------------------------------------------------------------------
# Collision blob builders
# ---------------------------------------------------------------------------

def _float_rows(rows) -> list[list[float]]:
    return [[float(value) for value in row] for row in rows]


def _int_rows(rows) -> list[list[int]]:
    return [[int(value) for value in row] for row in rows]


def fo4_polytope_collision_blob_raw(
    vertices,
    friction: float,
    restitution: float,
    layer: int,
    mass: float,
) -> bytes:
    """Build a single FO4 hknpConvexPolytopeShape packfile blob via the native module.

    `vertices`: Nx3 array-like of float32 (NIF-space).
    Raises RuntimeError if the native module is unavailable or does not
    expose fo4_polytope_collision_blob.
    """
    native = _require_native()
    fn = getattr(native, "fo4_polytope_collision_blob", None)
    if not callable(fn):
        raise RuntimeError(
            "fo4_polytope_collision_blob not found in havok_native — "
            "rebuild native after Phase 2 is integrated"
        )
    return bytes(fn(_float_rows(vertices), float(friction), float(restitution), int(layer), float(mass)))


def fo4_compound_collision_blob_raw(
    sub_shapes,
    friction: float,
    restitution: float,
    layer: int,
    mass: float,
) -> bytes:
    """Build a FO4 hknpDynamicCompoundShape packfile blob via the native module.

    `sub_shapes`: list of (transform_flat, kind, vertices, triangles_or_none) tuples:
      transform_flat — 16 floats, row-major 4×4
      kind           — "polytope" or "compressed_mesh"
      vertices       — list of [x, y, z] floats
      triangles_or_none — list of [i0, i1, i2] ints, or None
    Raises RuntimeError if native module is unavailable.
    """
    native = _require_native()
    fn = getattr(native, "fo4_compound_collision_blob", None)
    if not callable(fn):
        raise RuntimeError(
            "fo4_compound_collision_blob not found in havok_native — "
            "rebuild native after Phase 2 is integrated"
        )
    return bytes(fn(sub_shapes, float(friction), float(restitution), int(layer), float(mass)))


def fo4_compressed_mesh_collision_blob_raw(
    vertices,
    triangles,
    friction: float = 0.5,
    restitution: float = 0.4,
    layer: int = 1,
    mass: float = 0.0,
) -> bytes:
    """Build a FO4 hknpCompressedMeshShape packfile blob via the native module."""
    native = _require_native()
    fn = getattr(native, "fo4_compressed_mesh_collision_blob", None)
    if not callable(fn):
        raise RuntimeError("fo4_compressed_mesh_collision_blob not found in havok_native")
    return bytes(
        fn(
            _float_rows(vertices),
            _int_rows(triangles),
            float(friction),
            float(restitution),
            int(layer),
            float(mass),
        )
    )


def starfield_convex_collision_blob_raw(
    vertices,
    friction: float = 0.5,
    restitution: float = 0.4,
    layer: int = 1,
    mass: float = 0.0,
) -> bytes:
    """Build a Starfield hknpConvexShape packfile blob via the native module."""
    native = _require_native()
    fn = getattr(native, "starfield_convex_collision_blob", None)
    if not callable(fn):
        raise RuntimeError("starfield_convex_collision_blob not found in havok_native")
    return bytes(fn(_float_rows(vertices), float(friction), float(restitution), int(layer), float(mass)))


# ---------------------------------------------------------------------------
# Collision inspection
# ---------------------------------------------------------------------------

def collision_preview_native(
    blob: bytes,
    havok_scale: float = 1.0,
    body_id: int | None = None,
) -> dict:
    """Return a preview mesh dict from a Havok collision blob.

    `body_id` filters to the shape referenced by `bodyCinfos[body_id].shape`
    in the embedded `hknpPhysicsSystemData`; pass `None` for unfiltered.
    """
    native = _require_native()
    fn = getattr(native, "havok_collision_preview", None)
    if not callable(fn):
        raise RuntimeError("havok_collision_preview not found in havok_native")
    raw = fn(bytes(blob), float(havok_scale), body_id)
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def collision_summary_native(blob: bytes) -> dict:
    """Return a summary dict from a Havok collision blob."""
    native = _require_native()
    fn = getattr(native, "havok_collision_summary", None)
    if not callable(fn):
        raise RuntimeError("havok_collision_summary not found in havok_native")
    raw = fn(bytes(blob))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------

def walk_meshes_dir_native(root_path: str, source: str) -> list[dict]:
    """Walk a meshes directory and return file entry dicts."""
    native = _require_native()
    fn = getattr(native, "walk_meshes_dir", None)
    if not callable(fn):
        raise RuntimeError("walk_meshes_dir not found in havok_native")
    raw = fn(str(root_path), str(source))
    return json.loads(raw) if isinstance(raw, str) else list(raw)


def classify_category_native(rel_path: str) -> str:
    """Classify a file's category based on its relative path."""
    native = _require_native()
    fn = getattr(native, "classify_category", None)
    if not callable(fn):
        raise RuntimeError("classify_category not found in havok_native")
    return str(fn(str(rel_path)))


def classify_role_native(rel_path: str) -> str:
    """Classify a file's role in the Havok hierarchy based on path patterns."""
    native = _require_native()
    fn = getattr(native, "classify_role", None)
    if not callable(fn):
        raise RuntimeError("classify_role not found in havok_native")
    return str(fn(str(rel_path)))


# ---------------------------------------------------------------------------
# Manifest
# ---------------------------------------------------------------------------

def build_manifests_native(entries: list, character_data: dict, source: str) -> list[dict]:
    """Build manifests from discovered file entries."""
    native = _require_native()
    fn = getattr(native, "build_manifests", None)
    if not callable(fn):
        raise RuntimeError("build_manifests not found in havok_native")
    raw = fn(json.dumps(entries), json.dumps(character_data), str(source))
    return json.loads(raw) if isinstance(raw, str) else list(raw)


# ---------------------------------------------------------------------------
# Parsers
# ---------------------------------------------------------------------------

def parse_animation_xml_native(xml_str: str) -> dict:
    """Parse Havok animation XML and return metadata dict."""
    native = _require_native()
    fn = getattr(native, "parse_animation_xml", None)
    if not callable(fn):
        raise RuntimeError("parse_animation_xml not found in havok_native")
    raw = fn(str(xml_str))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def parse_character_xml_native(xml_str: str) -> dict:
    """Parse Havok character XML and return metadata dict."""
    native = _require_native()
    fn = getattr(native, "parse_character_xml", None)
    if not callable(fn):
        raise RuntimeError("parse_character_xml not found in havok_native")
    raw = fn(str(xml_str))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def parse_project_xml_native(xml_str: str) -> dict:
    """Parse Havok project XML and return metadata dict."""
    native = _require_native()
    fn = getattr(native, "parse_project_xml", None)
    if not callable(fn):
        raise RuntimeError("parse_project_xml not found in havok_native")
    raw = fn(str(xml_str))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def parse_skeleton_xml_native(xml_str: str) -> dict:
    """Parse Havok skeleton XML and return a dict with bone hierarchy."""
    native = _require_native()
    fn = getattr(native, "havok_parse_skeleton", None)
    if not callable(fn):
        raise RuntimeError("havok_parse_skeleton not found in havok_native")
    raw = fn(str(xml_str))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def parse_behavior_xml_native(xml_str: str) -> dict:
    """Parse Havok behavior XML and return metadata dict."""
    native = _require_native()
    fn = getattr(native, "havok_parse_behavior", None)
    if not callable(fn):
        raise RuntimeError("havok_parse_behavior not found in havok_native")
    raw = fn(str(xml_str))
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


# ---------------------------------------------------------------------------
# Animation reader/writer
# ---------------------------------------------------------------------------

def extract_clip_native(animation_xml: str, skeleton_xml: str | None = None) -> dict:
    """Extract full animation data from Havok XML into a clip dict."""
    native = _require_native()
    fn = getattr(native, "havok_extract_clip", None)
    if not callable(fn):
        raise RuntimeError("havok_extract_clip not found in havok_native")
    raw = fn(str(animation_xml), skeleton_xml)
    return json.loads(raw) if isinstance(raw, str) else dict(raw)


def write_animation_xml_native(clip_dict: dict, skeleton_bone_names: list[str] | None = None) -> str:
    """Serialize an AnimationClip dict to Havok animation XML string."""
    native = _require_native()
    fn = getattr(native, "havok_write_animation_xml", None)
    if not callable(fn):
        raise RuntimeError("havok_write_animation_xml not found in havok_native")
    return str(fn(json.dumps(clip_dict), list(skeleton_bone_names or [])))


def decompress_spline_native(blob_bytes: bytes, params: dict) -> list[dict]:
    """Decompress spline animation data into per-frame transform dicts."""
    native = _require_native()
    fn = getattr(native, "havok_decompress_spline", None)
    if not callable(fn):
        raise RuntimeError("havok_decompress_spline not found in havok_native")
    required = [
        "num_transform_tracks",
        "num_float_tracks",
        "num_frames",
        "max_frames_per_block",
        "num_blocks",
        "block_offsets",
        "float_block_offsets",
        "mask_and_quant_size",
        "block_duration",
        "block_inverse_duration",
        "frame_duration",
    ]
    missing = [key for key in required if key not in params]
    if missing:
        raise ValueError(f"decompress_spline_native missing params: {', '.join(missing)}")
    raw = fn(
        bytes(blob_bytes),
        int(params["num_transform_tracks"]),
        int(params["num_float_tracks"]),
        int(params["num_frames"]),
        int(params["max_frames_per_block"]),
        int(params["num_blocks"]),
        list(params["block_offsets"]),
        list(params["float_block_offsets"]),
        int(params["mask_and_quant_size"]),
        float(params["block_duration"]),
        float(params["block_inverse_duration"]),
        float(params["frame_duration"]),
    )
    return json.loads(raw) if isinstance(raw, str) else list(raw)


# ---------------------------------------------------------------------------
# Collision validation
# ---------------------------------------------------------------------------

def validate_collision_blob_native(blob: bytes, invariants_json: str) -> str:
    native = _require_native()
    fn = getattr(native, "validate_collision_blob", None)
    if fn is None:
        raise RuntimeError("validate_collision_blob not found in havok_native")
    return fn(bytes(blob), invariants_json)


# ---------------------------------------------------------------------------
# classxml generation
# ---------------------------------------------------------------------------

def generate_classxml_native(
    source_dir: str,
    patches_dir: str,
    output_base: str,
    targets_json: str,
    base_version_id: int = 53,
) -> None:
    """Generate versioned classxml directories via the native Rust implementation."""
    native = _require_native()
    fn = getattr(native, "generate_classxml", None)
    if not callable(fn):
        raise RuntimeError("generate_classxml not found in havok_native")
    fn(str(source_dir), str(patches_dir), str(output_base), str(targets_json), int(base_version_id))
