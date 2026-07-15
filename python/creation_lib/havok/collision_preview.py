"""Collision preview binding for the native Havok backend."""
from __future__ import annotations

from typing import Any


def extract_preview_meshes_from_blob(
    blob: bytes,
    havok_scale: float,
    body_id: int | None = None,
) -> list[dict[str, Any]]:
    """Extract preview meshes from a Havok collision blob via Rust."""
    from creation_lib.havok.native_runtime import collision_preview_native

    data = collision_preview_native(blob, havok_scale=havok_scale, body_id=body_id)
    return list(data.get("meshes") or [])
