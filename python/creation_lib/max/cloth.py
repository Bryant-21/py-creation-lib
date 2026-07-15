from __future__ import annotations

import base64
import json
from typing import Any

from creation_lib.havok.native_runtime import load_native_module as load_havok_native_module
from creation_lib.nif.native_runtime import load_native_module as load_nif_native_module
from creation_lib.nif.nif_file import NifFile


def extract_cloth_document(
    nif: NifFile,
    nif_bytes: bytes,
    supported_ids: set[int],
) -> dict[str, Any] | None:
    havok_native = load_havok_native_module()
    nif_native = load_nif_native_module()
    if havok_native is None or nif_native is None:
        return None

    try:
        blob = bytes(nif_native.cloth_extract_blob(nif_bytes))
    except Exception:
        return None

    cloth_block = _find_cloth_block(nif)
    if cloth_block is not None:
        supported_ids.add(cloth_block.block_id)

    setup, reverse_error = _reverse_setup(havok_native.cloth_reverse_to_setup, blob)
    blob_entry: dict[str, Any] = {
        "source_block_id": cloth_block.block_id if cloth_block is not None else -1,
        "name": str(cloth_block.get_field("Name") or "") if cloth_block is not None else "",
        "raw_blob_base64": base64.b64encode(blob).decode("ascii"),
        "summary": _native_json(havok_native.cloth_inspect_blob_json, blob),
        "inspect": _native_json(havok_native.cloth_inspect_full_json, blob),
        "setup": setup,
        "reverse_error": reverse_error,
    }
    return {"version": 1, "blobs": [blob_entry]}


def pack_cloth_document(nif_bytes: bytes, cloth_doc: dict[str, Any] | None) -> bytes:
    if not cloth_doc:
        return nif_bytes

    blobs = list(cloth_doc.get("blobs") or [])
    if not blobs:
        return nif_bytes

    havok_native = load_havok_native_module()
    nif_native = load_nif_native_module()
    if havok_native is None or nif_native is None:
        return nif_bytes

    blob_doc = blobs[0]
    if blob_doc.get("edited_setup"):
        blob = bytes(havok_native.cloth_bake(json.dumps(blob_doc["edited_setup"])))
    else:
        raw_blob_base64 = str(blob_doc.get("raw_blob_base64") or "")
        if not raw_blob_base64:
            return nif_bytes
        blob = base64.b64decode(raw_blob_base64, validate=True)
        try:
            if bytes(nif_native.cloth_extract_blob(nif_bytes)) == blob:
                return nif_bytes
        except Exception:
            pass

    return bytes(nif_native.cloth_pack_blob(nif_bytes, blob))


def _find_cloth_block(nif: NifFile) -> Any | None:
    return next((block for block in nif.blocks if block.type_name == "BSClothExtraData"), None)


def _native_json(fn: Any, blob: bytes) -> dict[str, Any]:
    try:
        value = fn(blob)
        text = value.decode("utf-8") if isinstance(value, bytes) else str(value)
        parsed = json.loads(text)
        return parsed if isinstance(parsed, dict) else {"value": parsed}
    except Exception as exc:
        return {"error": str(exc)}


def _reverse_setup(fn: Any, blob: bytes) -> tuple[dict[str, Any] | None, str]:
    setup = _native_json(fn, blob)
    error = setup.get("error")
    if error is not None:
        return None, str(error)
    return setup, ""
