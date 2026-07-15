"""Thin Python boundary for native directxtex DDS entrypoints."""

from __future__ import annotations

from importlib import import_module
from typing import Any

_NATIVE_MODULE: Any | None = None
_NATIVE_IMPORT_ATTEMPTED = False


def _load_umbrella_submodule() -> Any | None:
    try:
        umbrella = import_module("creation_lib._native")
    except ImportError:
        return None
    native_module = getattr(umbrella, "directxtex_native", None)
    if native_module is not None:
        return native_module
    # Fallback: umbrella may be a namespace package; try loading the .pyd directly.
    try:
        pyd = import_module("creation_lib._native")
        return getattr(pyd, "directxtex_native", None)
    except ImportError:
        return None


def load_native_module() -> Any | None:
    global _NATIVE_MODULE, _NATIVE_IMPORT_ATTEMPTED
    if _NATIVE_IMPORT_ATTEMPTED:
        return _NATIVE_MODULE
    _NATIVE_IMPORT_ATTEMPTED = True
    # `directxtex_native` is built as a sub-crate of the private
    # `creation_lib._native` extension; load it from the umbrella.
    _NATIVE_MODULE = _load_umbrella_submodule()
    return _NATIVE_MODULE


def native_function_available(name: str) -> bool:
    module = load_native_module()
    return module is not None and getattr(module, name, None) is not None


def read_dds_rgba(path: str) -> dict[str, Any] | None:
    module = load_native_module()
    if module is None:
        return None
    native_fn = getattr(module, "read_dds_rgba", None)
    if native_fn is None:
        return None
    return native_fn(path)


def write_dds_rgba(
    output_path: str,
    width: int,
    height: int,
    rgba: bytes,
    *,
    format: str = "BC7_UNORM",
    generate_mips: bool = False,
    use_gpu: bool = True,
) -> bool:
    module = load_native_module()
    if module is None:
        return False
    native_fn = getattr(module, "write_dds_rgba", None)
    if native_fn is None:
        return False
    native_fn(
        output_path,
        width,
        height,
        rgba,
        format=format,
        generate_mips=generate_mips,
        use_gpu=use_gpu,
    )
    return True


def texdiag_info(path: str) -> dict[str, Any] | None:
    module = load_native_module()
    if module is None:
        return None
    native_fn = getattr(module, "texdiag_info", None)
    if native_fn is None:
        return None
    return native_fn(path)


def remix_fo76_texture_to_fo4(
    src_path: str,
    dst_path: str,
    *,
    role: str,
    format: str,
    ao_multiplier: float,
    specular_multiplier: float,
    gloss_multiplier: float,
    spec_offset: float,
) -> bool:
    module = load_native_module()
    if module is None:
        return False
    native_fn = getattr(module, "remix_fo76_texture_to_fo4", None)
    if native_fn is None:
        return False
    native_fn(
        src_path,
        dst_path,
        role,
        format,
        float(ao_multiplier),
        float(specular_multiplier),
        float(gloss_multiplier),
        float(spec_offset),
    )
    return True


def remix_fo76_bundle_to_fo4(
    diffuse_path: str,
    reflectivity_path: str,
    lighting_path: str,
    diffuse_out_path: str,
    specgloss_out_path: str,
    glow_out_path: str,
    *,
    diffuse_format: str,
    specgloss_format: str,
    glow_format: str,
    ao_multiplier: float,
    specular_multiplier: float,
    gloss_multiplier: float,
    spec_offset: float,
) -> bool:
    module = load_native_module()
    if module is None:
        return False
    native_fn = getattr(module, "remix_fo76_bundle_to_fo4", None)
    if native_fn is None:
        return False
    native_fn(
        diffuse_path,
        reflectivity_path,
        lighting_path,
        diffuse_out_path,
        specgloss_out_path,
        glow_out_path,
        diffuse_format,
        specgloss_format,
        glow_format,
        float(ao_multiplier),
        float(specular_multiplier),
        float(gloss_multiplier),
        float(spec_offset),
    )
    return True
