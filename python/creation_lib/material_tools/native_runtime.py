"""Thin Python boundary for native materials_native entrypoints."""

from __future__ import annotations

import threading
from importlib import import_module
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False
_NATIVE_LOCK = threading.Lock()


def _load_umbrella_submodule() -> Any:
    umbrella = import_module("creation_lib._native")
    native_module = getattr(umbrella, "materials_native", None)
    if native_module is None:
        raise ImportError("creation_lib._native.materials_native is missing")
    return native_module


def load_native_module() -> Any:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    with _NATIVE_LOCK:
        if _NATIVE_IMPORT_ATTEMPTED:
            if _NATIVE_MODULE is None:
                raise RuntimeError("materials_native is required for material operations")
            return _NATIVE_MODULE
        try:
            _NATIVE_MODULE = _load_umbrella_submodule()
        except ImportError as exc:
            _NATIVE_IMPORT_ATTEMPTED = True
            raise RuntimeError("materials_native is required for material operations") from exc
        _NATIVE_IMPORT_ATTEMPTED = True
        return _NATIVE_MODULE


def _require_native_function(name: str) -> Any:
    module = load_native_module()
    native_fn = getattr(module, name, None)
    if native_fn is None:
        raise NotImplementedError(f"materials_native does not provide {name}() yet")
    return native_fn


def native_function_available(name: str) -> bool:
    return getattr(load_native_module(), name, None) is not None


def bethesda_crc32(data: bytes) -> int:
    return int(_require_native_function("bethesda_crc32")(data))


def resource_id_from_path(path: str) -> dict[str, int]:
    return dict(_require_native_function("resource_id_from_path")(path))


def parse_cdb(data: bytes) -> dict:
    return dict(_require_native_function("parse_cdb")(data))


def project_ce2_material(payload: dict, db_id: int) -> dict | None:
    result = _require_native_function("project_ce2_material")(payload, db_id)
    return None if result is None else dict(result)


def walk_component(
    blob_payload: dict,
    class_defs_payload: list[dict],
    objects_payload: list[dict] | None = None,
) -> dict | None:
    result = _require_native_function("walk_component")(
        blob_payload,
        class_defs_payload,
        objects_payload or [],
    )
    return None if result is None else dict(result)


def parse_bgsm(data: bytes) -> dict:
    return dict(_require_native_function("parse_bgsm")(data))


def write_bgsm(payload: dict) -> bytes:
    return bytes(_require_native_function("write_bgsm")(payload))


def parse_bgem(data: bytes) -> dict:
    return dict(_require_native_function("parse_bgem")(data))


def write_bgem(payload: dict) -> bytes:
    return bytes(_require_native_function("write_bgem")(payload))


def find_master_string(value: str) -> int:
    return int(_require_native_function("find_master_string")(value))


def inspect_bsrefl(data: bytes) -> dict:
    return dict(_require_native_function("inspect_bsrefl")(data))


def pbr_to_specgloss_f32(
    albedo: bytes,
    metallic: bytes,
    roughness: bytes,
    ao: bytes | None,
    pixel_count: int,
    *,
    ao_multiplier: float,
    specular_multiplier: float,
    gloss_multiplier: float,
    spec_offset: float,
) -> dict[str, bytes]:
    return dict(
        _require_native_function("pbr_to_specgloss_f32")(
            albedo,
            metallic,
            roughness,
            ao,
            int(pixel_count),
            float(ao_multiplier),
            float(specular_multiplier),
            float(gloss_multiplier),
            float(spec_offset),
        )
    )


def fo76_bundle_to_fo4_f32(
    diffuse: bytes,
    reflectivity: bytes,
    lighting: bytes,
    width: int,
    height: int,
    reflectivity_width: int,
    reflectivity_height: int,
    lighting_width: int,
    lighting_height: int,
    *,
    ao_multiplier: float,
    specular_multiplier: float,
    gloss_multiplier: float,
    spec_offset: float,
    emit_lighting_alpha_glow: bool = False,
) -> dict[str, bytes]:
    return dict(
        _require_native_function("fo76_bundle_to_fo4_f32")(
            diffuse,
            reflectivity,
            lighting,
            int(width),
            int(height),
            int(reflectivity_width),
            int(reflectivity_height),
            int(lighting_width),
            int(lighting_height),
            float(ao_multiplier),
            float(specular_multiplier),
            float(gloss_multiplier),
            float(spec_offset),
            bool(emit_lighting_alpha_glow),
        )
    )


def fo76_normal_to_fo4_f32(normal: bytes, width: int, height: int) -> bytes:
    return bytes(
        _require_native_function("fo76_normal_to_fo4_f32")(
            normal,
            int(width),
            int(height),
        )
    )


def passthrough_rgba_f32(rgba: bytes, width: int, height: int) -> bytes:
    return bytes(
        _require_native_function("passthrough_rgba_f32")(
            rgba,
            int(width),
            int(height),
        )
    )


def convert_texture_set_paths(payload: dict) -> dict:
    return dict(_require_native_function("convert_texture_set_paths")(payload))
