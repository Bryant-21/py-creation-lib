"""Material pipeline — texture loading and material property extraction.

Reads BSLightingShaderProperty / BSEffectShaderProperty from NIF blocks,
resolves texture paths, loads textures into ModernGL, and returns a
Material dataclass with all GPU resources and uniform values.
"""
from __future__ import annotations
import logging
import math
import os
import hashlib
from pathlib import Path

import moderngl
import glm

import numpy as np

from creation_lib.renderer.scene_renderer import Material
from creation_lib.renderer.dds_loader import (
    DecodedTexture,
    decode_texture_bytes,
    load_cubemap,
    load_cubemap_bytes,
    load_texture,
    upload_decoded,
)

_log = logging.getLogger("nif_editor.material")

# Cache size limits
_MAX_TEX_CACHE = 256      # GPU texture cache entries; scene materials may hold evicted textures
_MAX_DECODE_CACHE = 512   # Decoded CPU bytes
_MAX_RESOLVE_CACHE = 2048 # Path resolution results
_MAX_MATERIAL_CACHE = 256 # Parsed material files


def _lru_put(cache: dict, key, value, max_size: int, on_evict=None):
    """Insert into a bounded dict, evicting oldest entries if over max_size."""
    if key in cache:
        cache.pop(key)
    cache[key] = value
    while len(cache) > max_size:
        evicted_key, evicted_val = next(iter(cache.items()))
        del cache[evicted_key]
        if on_evict:
            on_evict(evicted_val)


def _release_texture(tex):
    """Safely release a ModernGL texture."""
    try:
        tex.release()
    except Exception:
        pass


def _texture_is_usable(tex) -> bool:
    if tex is None or not callable(getattr(tex, "use", None)):
        return False
    mglo = getattr(tex, "mglo", None)
    return mglo is None or callable(getattr(mglo, "use", None))


def _cached_texture(key: str):
    tex = _tex_cache.get(key)
    if _texture_is_usable(tex):
        return tex
    if tex is not None:
        _tex_cache.pop(key, None)
    return None


# Texture cache: resolved path -> ModernGL texture (persists across NIF loads, LRU bounded)
_tex_cache: dict[str, moderngl.Texture] = {}

# Decode cache: resolved path -> DecodedTexture (persists across NIF loads, LRU bounded)
_decode_cache: dict[str, object] = {}  # value: DecodedTexture | None

# Resolve cache: (tex_path_str, texture_dirs_tuple) -> resolved Path (persists, LRU bounded)
_resolve_cache: dict[tuple, Path] = {}

# Material cache: resolved material source key -> (texture_paths_or_none, parsed_dict) (persists, LRU bounded)
_material_cache: dict[str, tuple] = {}

# Material cache sources: material cache key -> absolute loose material path.
# BA2-backed materials are intentionally absent because watchdog can only track files.
_material_cache_sources: dict[str, str] = {}

# Directory indexes: base_dir_str -> DirectoryIndex (built on first use, reused)
_dir_indexes: dict[str, "DirectoryIndex"] = {}


def _get_dir_index(base_dir: Path) -> "DirectoryIndex | None":
    """Get or create a DirectoryIndex for a directory (cached across loads)."""
    key = str(base_dir)
    idx = _dir_indexes.get(key)
    if idx is not None:
        return idx
    if not base_dir.is_dir():
        return None
    try:
        from creation_lib.db.dir_index import DirectoryIndex
        import time as _time
        _log.info("Building directory index: %s", base_dir)
        _t0 = _time.perf_counter()
        idx = DirectoryIndex(base_dir)
        _log.info(
            "Built directory index: %s (%.0f ms, %d files)",
            base_dir, (_time.perf_counter() - _t0) * 1000, idx.file_count,
        )
        _dir_indexes[key] = idx
        return idx
    except Exception as exc:
        _log.debug("Failed to build dir index for %s: %s", base_dir, exc)
        return None


def clear_texture_cache():
    """Full cache wipe — use when switching game profiles or shutting down."""
    for tex in _tex_cache.values():
        _release_texture(tex)
    _tex_cache.clear()
    _decode_cache.clear()
    _resolve_cache.clear()
    _material_cache.clear()
    _material_cache_sources.clear()
    for idx in _dir_indexes.values():
        idx.close()
    _dir_indexes.clear()


def on_nif_unload(nif_id: str):
    """Called when a NIF is closed. Caches persist (shared textures benefit other NIFs)."""
    pass


def get_loaded_texture_paths() -> list[str]:
    """Return absolute file paths of textures currently in the GPU cache.

    Only returns loose-file textures (skips cubemaps and BA2-sourced entries
    that aren't real filesystem paths).
    """
    return [
        k for k in _tex_cache
        if not k.startswith("cube:") and os.path.isabs(k) and os.path.isfile(k)
    ]


def invalidate_material_cache(abs_path: str) -> list[str]:
    """Drop parsed material cache entries sourced from abs_path."""
    norm = os.path.normcase(os.path.abspath(abs_path))
    removed = []
    for key, source in list(_material_cache_sources.items()):
        if os.path.normcase(os.path.abspath(source)) != norm:
            continue
        _material_cache.pop(key, None)
        _material_cache_sources.pop(key, None)
        removed.append(key)
    return removed


def reload_texture_inplace(ctx: moderngl.Context, abs_path: str) -> bool:
    """Reload a texture file and write new pixel data into the existing GL texture.

    Returns True on success. Returns False if the texture wasn't in cache,
    couldn't be decoded, or its dimensions changed (it is evicted from cache
    in that case — a full NIF reload is needed to pick it up).
    """
    from .dds_loader import decode_texture as _decode_file
    key = str(abs_path)
    if key not in _tex_cache:
        return False
    tex = _tex_cache[key]
    decoded = _decode_file(abs_path)
    if not decoded:
        _log.warning("Texture hot-reload: could not decode %s", abs_path)
        return False
    if decoded.size != (tex.width, tex.height):
        _log.warning(
            "Texture size changed for %s (%dx%d → %dx%d); "
            "evicted from cache. Reload the NIF to see the update.",
            abs_path, tex.width, tex.height, decoded.size[0], decoded.size[1],
        )
        _tex_cache.pop(key, None)
        _decode_cache.pop(key, None)
        return False
    try:
        tex.write(decoded.data)
    except Exception as e:
        _log.warning("Failed to write texture in-place for %s: %s", abs_path, e)
        return False
    _lru_put(_decode_cache, key, decoded, _MAX_DECODE_CACHE)
    _log.info("Hot-reloaded texture: %s", Path(abs_path).name)
    return True


def _cached_load(ctx: moderngl.Context, resolved: Path | str | bytes,
                  cache_key: str | None = None) -> moderngl.Texture:
    """Load a texture with caching. Uses pre-decoded data if available.

    resolved can be:
      - Path: load from filesystem (original behavior)
      - bytes: decode from in-memory data (BA2 extraction)

    cache_key overrides the cache key (used for BA2 textures).
    """
    if isinstance(resolved, bytes):
        key = cache_key or f"ba2:{id(resolved)}"
        cached = _cached_texture(key)
        if cached is not None:
            return cached
        decoded = decode_texture_bytes(resolved, name=key)
        tex = upload_decoded(ctx, decoded) if decoded else ctx.texture((1, 1), 4, b'\xff\x00\xff\xff')
        _lru_put(_tex_cache, key, tex, _MAX_TEX_CACHE)
        return tex

    key = cache_key or str(resolved)
    cached = _cached_texture(key)
    if cached is not None:
        return cached
    if key in _decode_cache:
        decoded = _decode_cache[key]
        tex = upload_decoded(ctx, decoded) if decoded is not None else ctx.texture((1, 1), 4, b'\xff\x00\xff\xff')
    else:
        tex = load_texture(ctx, key)
    _lru_put(_tex_cache, key, tex, _MAX_TEX_CACHE)
    return tex


def _cached_load_cubemap(
    ctx: moderngl.Context,
    resolved: Path | bytes,
    *,
    cache_key: str | None = None,
    decode_key: str | None = None,
) -> moderngl.Texture:
    """Load a cubemap with caching, reusing pre-decoded DDS data when available."""
    if isinstance(resolved, bytes):
        key = cache_key or f"ba2:cube:{id(resolved)}"
        source_key = decode_key or key
        cached = _cached_texture(key)
        if cached is not None:
            return cached
        if source_key in _decode_cache:
            decoded = _decode_cache[source_key]
            tex = (
                upload_decoded(ctx, decoded, build_mipmaps=False)
                if decoded is not None
                else ctx.texture((1, 1), 4, b"\x40\x40\x40\xff")
            )
        else:
            tex = load_cubemap_bytes(ctx, resolved, name=source_key)
            if tex is None:
                tex = ctx.texture((1, 1), 4, b"\x40\x40\x40\xff")
        _lru_put(_tex_cache, key, tex, _MAX_TEX_CACHE)
        return tex

    key = cache_key or ("cube:" + str(resolved))
    source_key = decode_key or str(resolved)
    cached = _cached_texture(key)
    if cached is not None:
        return cached
    if source_key in _decode_cache:
        decoded = _decode_cache[source_key]
        tex = (
            upload_decoded(ctx, decoded, build_mipmaps=False)
            if decoded is not None
            else ctx.texture((1, 1), 4, b"\x40\x40\x40\xff")
        )
    else:
        tex = load_cubemap(ctx, str(resolved))
    _lru_put(_tex_cache, key, tex, _MAX_TEX_CACHE)
    return tex

# FO4 texture slot mapping (BSShaderTextureSet)
SLOT_DIFFUSE = 0
SLOT_NORMAL = 1
SLOT_GLOW = 2
SLOT_GREYSCALE = 3  # greyscale-to-palette lookup texture
SLOT_CUBEMAP = 4
SLOT_ENVMASK = 5
SLOT_SUBSURFACE = 6
SLOT_SPECULAR = 7  # smooth-spec


def _texture_path_candidates(tex_path: str) -> list[str]:
    candidates: list[str] = []

    def add(candidate: str):
        candidate = candidate.lstrip("/")
        if candidate and candidate not in candidates:
            candidates.append(candidate)

    add(tex_path)
    if tex_path.lower().startswith("data/"):
        add(tex_path[5:])

    for candidate in tuple(candidates):
        if not candidate.lower().startswith("textures/"):
            add(f"Textures/{candidate}")

    return candidates


def _resolve_texture_path(tex_path: str, texture_dirs: list[Path],
                          ba2_mgr=None) -> Path | bytes | None:
    """Resolve a NIF texture path to an absolute file path or BA2 bytes.

    Search order: loose files on disk first (case-insensitive walk through
    texture_dirs), then BA2 archives as fallback.

    Returns:
      - Path for loose files
      - bytes for BA2-extracted data
      - None if not found
    """
    if not tex_path:
        return None
    # Normalize separators and strip null terminators from BGSM/BGEM strings
    tex_path = tex_path.replace("\\", "/").strip().rstrip("\x00")

    # Check resolve cache (keyed on path + dirs so it's correct across configs)
    cache_key = (tex_path, tuple(str(d) for d in texture_dirs))
    if cache_key in _resolve_cache:
        return _resolve_cache[cache_key]

    # Try each search directory (loose files first)
    candidates = _texture_path_candidates(tex_path)
    for base_dir in texture_dirs:
        # Fast path: use directory index if available
        dir_idx = _get_dir_index(base_dir)
        if dir_idx is not None:
            for candidate in candidates:
                resolved = dir_idx.resolve(candidate)
                if resolved is not None:
                    _lru_put(_resolve_cache, cache_key, resolved, _MAX_RESOLVE_CACHE)
                    return resolved
        else:
            # Fallback: slow case-insensitive walk (small dirs without index)
            for candidate in candidates:
                resolved = _case_insensitive_resolve(base_dir, candidate)
                if resolved and resolved.exists():
                    _lru_put(_resolve_cache, cache_key, resolved, _MAX_RESOLVE_CACHE)
                    return resolved

    # BA2 fallback
    if ba2_mgr is not None:
        for candidate in candidates:
            data = ba2_mgr.find(candidate)
            if data is not None:
                return data

    return None


def load_texture_path(
    ctx: moderngl.Context,
    tex_path: str,
    texture_dirs: list[Path],
    ba2_mgr=None,
) -> moderngl.Texture | None:
    resolved = _resolve_texture_path(tex_path, texture_dirs, ba2_mgr)
    if resolved is None:
        return None
    if isinstance(resolved, bytes):
        normalized = tex_path.replace("\\", "/").strip().rstrip("\x00").lower()
        return _cached_load(ctx, resolved, cache_key=f"ba2:{normalized}")
    return _cached_load(ctx, resolved)


def _case_insensitive_resolve(base: Path, rel_path: str) -> Path | None:
    """Walk path segments case-insensitively from base."""
    current = base
    for segment in rel_path.split("/"):
        if not segment:
            continue
        if not current.is_dir():
            return None
        found = None
        seg_lower = segment.lower()
        try:
            for child in current.iterdir():
                if child.name.lower() == seg_lower:
                    found = child
                    break
        except PermissionError:
            return None
        if found is None:
            return None
        current = found
    return current if current.is_file() else None


def _parse_bgsm(filepath_or_bytes: Path | bytes) -> dict | None:
    """Parse a .bgsm material file using creation_lib.material_tools.

    Accepts a file Path or raw bytes (from BA2 extraction).
    Returns dict of texture paths and material params, or None on error.
    """
    try:
        import io
        from creation_lib.material_tools.bgsm_bin import read_bgsm
        if isinstance(filepath_or_bytes, bytes):
            data = read_bgsm(io.BytesIO(filepath_or_bytes))
        else:
            with open(filepath_or_bytes, "rb") as f:
                data = read_bgsm(f)
        def _s(v):
            return (v or "").rstrip("\x00")
        return {
            "type": "bgsm",
            "diffuse": _s(data.DiffuseTexture),
            "normal": _s(data.NormalTexture),
            "smooth_spec": _s(data.SmoothSpecTexture),
            "greyscale": _s(data.GreyscaleTexture),
            "envmap": _s(data.EnvmapTexture),
            "glow": _s(data.GlowTexture),
            "inner_layer": _s(getattr(data, "InnerLayerTexture", None)),
            "wrinkles": _s(getattr(data, "WrinklesTexture", None)),
            "displacement": _s(getattr(data, "DisplacementTexture", None)),
            "reflectivity": _s(data.SpecularTexture),
            "lighting": _s(getattr(data, "LightingTexture", None)),
            "flow": _s(getattr(data, "FlowTexture", None)),
            "distance_field_alpha": _s(getattr(data, "DistanceFieldAlphaTexture", None)),
            "pbr": bool(getattr(data, "PBR", False)),
            "version": data.header.version,
            "grayscale_to_palette_color": data.header.grayscale_to_palette_color,
            "palette_scale": data.GrayscaleToPaletteScale,
            # BGSM material params for shader
            "spec_color": (data.SpecularColor[0], data.SpecularColor[1], data.SpecularColor[2]),
            "spec_strength": data.SpecularMult,
            "glossiness": data.Smoothness,
            "fresnel_power": data.FresnelPower,
            # Emissive / glow params from BGSM
            "emit_enabled": data.EmitEnabled,
            "emittance_color": data.EmittanceColor,
            "emittance_mult": data.EmittanceMult,
            "lum_emittance": getattr(data, "LumEmittance", None),
            "glowmap": data.Glowmap,
            "subsurface_enabled": bool(getattr(data, "Translucency", False)),
            "subsurface_color": getattr(data, "TranslucencySubsurfaceColor", None),
            "subsurface_scale": getattr(data, "TranslucencyTransmissiveScale", None),
        }
    except Exception as e:
        _log.debug("Failed to parse BGSM %s: %s", filepath_or_bytes, e)
        return None


def _parse_bgem(filepath_or_bytes: Path | bytes) -> dict | None:
    """Parse a .bgem effect material file using creation_lib.material_tools.

    Accepts a file Path or raw bytes (from BA2 extraction).
    Returns dict of texture paths and material params, or None on error.
    """
    try:
        import io
        from creation_lib.material_tools.bgem_bin import read_bgem
        if isinstance(filepath_or_bytes, bytes):
            data = read_bgem(io.BytesIO(filepath_or_bytes))
        else:
            with open(filepath_or_bytes, "rb") as f:
                data = read_bgem(f)
        def _s(v):
            return (v or "").rstrip("\x00")
        return {
            "type": "bgem",
            "diffuse": _s(data.BaseTexture),
            "normal": _s(data.NormalTexture),
            "greyscale": _s(data.GrayscaleTexture),
            "cubemap": _s(data.EnvmapTexture),
            "envmask": _s(data.EnvmapMaskTexture),
            "specular": _s(data.SpecularTexture) if data.SpecularTexture else "",
            "lighting": _s(getattr(data, "LightingTexture", None)),
            "glow": _s(getattr(data, "GlowTexture", None)),
            "glass_roughness_scratch": _s(getattr(data, "GlassRoughnessScratch", None)),
            "glass_dirt_overlay": _s(getattr(data, "GlassDirtOverlay", None)),
            # BGEM-specific params
            "base_color": data.BaseColor,
            "base_color_scale": data.BaseColorScale,
            "env_mapping": data.EnvironmentMapping,
            "env_mapping_mask_scale": data.EnvironmentMappingMaskScale,
            "falloff_enabled": data.FalloffEnabled,
            "falloff_start_angle": data.FalloffStartAngle,
            "falloff_stop_angle": data.FalloffStopAngle,
            "falloff_start_opacity": data.FalloffStartOpacity,
            "falloff_stop_opacity": data.FalloffStopOpacity,
            "lighting_influence": data.LightingInfluence,
            # Greyscale palette flags
            "grayscale_to_palette_alpha": data.GrayscaleToPaletteAlpha,
            "grayscale_to_palette_color": data.header.grayscale_to_palette_color,
            "falloff_color_enabled": data.FalloffColorEnabled,
            "effect_lighting_enabled": data.EffectLightingEnabled,
            # Base header alpha/blend fields — authoritative for blending setup
            "header_alpha": data.header.alpha,
            "header_blend_mode": data.header.alpha_blend_mode0,
            "header_blend_src": data.header.alpha_blend_mode1,
            "header_blend_dst": data.header.alpha_blend_mode2,
            "header_alpha_test": data.header.alpha_test,
            "header_alpha_test_ref": data.header.alpha_test_ref,
            "header_zbuffer_write": data.header.zbuffer_write,
            "alpha_blend": data.header.alpha_blend_mode0 != 0,
        }
    except Exception as e:
        _log.debug("Failed to parse BGEM %s: %s", filepath_or_bytes, e)
        return None


def _parse_mat(filepath_or_bytes: Path | bytes) -> dict | None:
    """Parse a Starfield .mat material file.

    Accepts a file Path or raw bytes (from BA2 extraction).
    Returns dict of texture paths, layer info, and material params, or None on error.
    """
    try:
        import json as _json
        from creation_lib.renderer.material_readers.starfield_mat_reader import read_mat_json
        from creation_lib.renderer.material_readers.starfield_mat_reader import (
            _extract_layer, _extract_blender,
            _LAYER_KEYS,
        )
        from creation_lib.renderer.material_readers.base import MaterialData

        if isinstance(filepath_or_bytes, bytes):
            raw = _json.loads(filepath_or_bytes.decode("utf-8"))
            summary = raw.get("Summary", {})
            layers = []
            blenders = []
            # Blenders are nested inside each Layer (Summary.LayerN.Blender),
            # NOT under top-level Summary.BlenderN. Layer1 has no preceding
            # blender; Layer2+ each contribute one blender.
            for i, key in enumerate(_LAYER_KEYS, start=1):
                layer_json = summary.get(key)
                if layer_json is None:
                    continue
                layers.append(_extract_layer(layer_json))
                if i > 1:
                    blender_json = layer_json.get("Blender") or {}
                    blenders.append(_extract_blender(blender_json))
            texture_paths = layers[0].texture_paths if layers else {}
            data = MaterialData(
                texture_paths=texture_paths, params={},
                material_model="metallic-roughness",
                layers=layers, blenders=blenders,
            )
        else:
            data = read_mat_json(filepath_or_bytes)

        if not data.texture_paths and not data.layers:
            return None

        result = {
            "type": "mat",
            **data.texture_paths,
            "_material_model": data.material_model,
            "_layer_count": len(data.layers),
            "_layers": data.layers,
            "_blenders": data.blenders,
            "_alpha_settings": data.alpha_settings,
            "_decal_settings": data.decal_settings,
            "_emissive_settings": data.emissive_settings,
        }
        return result
    except Exception as e:
        _log.debug("Failed to parse .mat %s: %s", filepath_or_bytes, e)
        return None


def _extract_alpha_flags(nif, shape_block) -> tuple[int, float, int, int]:
    """Extract alpha test/blend flags from NiAlphaProperty.

    Returns (alpha_flags, alpha_threshold, blend_src, blend_dst).
    """
    alpha_block = _get_shape_property_block(
        nif, shape_block, "Alpha Property", "NiAlphaProperty"
    )
    if not alpha_block:
        return 0, 0.0, 0, 0
    flags = alpha_block.get_field("Flags") or 0
    # NiAlphaProperty flags layout:
    #   bit 0:    alpha blend enable
    #   bits 1-4: source blend mode
    #   bits 5-8: destination blend mode
    #   bit 9:    alpha test enable
    #   bits 10-12: alpha test function (0=always, 1=<, 2==, 3=<=, 4=>, 5=!=, 6=>=, 7=never)
    #   bit 13:   no sorter
    alpha_test_enabled = (flags >> 9) & 0x1
    test_func = (flags >> 10) & 0x7
    blend_enabled = (flags >> 0) & 0x1
    alpha_flags = test_func | (blend_enabled << 3)
    # Only use threshold when alpha test is enabled (NifSkope uses 0.0 when off)
    threshold_val = (alpha_block.get_field("Threshold") or 128) if alpha_test_enabled else 0
    # Extract src/dst blend factors from NiAlphaProperty flags
    # Bits 1-4 = source blend mode, Bits 5-8 = destination blend mode
    blend_src = (flags >> 1) & 0xF
    blend_dst = (flags >> 5) & 0xF
    return alpha_flags, threshold_val / 255.0, blend_src, blend_dst


def _extract_shader_params(shader_prop) -> dict:
    """Extract material uniforms from BSLightingShaderProperty."""
    spec_color_val = shader_prop.get_field("Specular Color")
    if hasattr(spec_color_val, "r"):
        sc = [float(spec_color_val.r), float(spec_color_val.g), float(spec_color_val.b)]
    elif isinstance(spec_color_val, dict):
        sc = [float(spec_color_val.get("r", 1.0)),
              float(spec_color_val.get("g", 1.0)),
              float(spec_color_val.get("b", 1.0))]
    elif isinstance(spec_color_val, (list, tuple)) and len(spec_color_val) >= 3:
        sc = [float(x) for x in spec_color_val[:3]]
    else:
        sc = [1.0, 1.0, 1.0]

    uv_scale = shader_prop.get_field("UV Scale") or {}
    uv_offset = shader_prop.get_field("UV Offset") or {}

    # Check shader flags — older games store these as lists of flag-name
    # strings; FO4 stores a packed integer (bit 22 = Own_Emit, bit 4 =
    # GreyscaleToPalette_Color).  BGSM overrides these below when present.
    sf1 = shader_prop.get_field("Shader Flags 1") or []
    if isinstance(sf1, list):
        greyscale_color = "GreyscaleToPalette_Color" in sf1
        has_emit = "Own_Emit" in sf1
    else:
        sf1_int = int(sf1) if sf1 else 0
        greyscale_color = bool(sf1_int & (1 << 4))
        has_emit = bool(sf1_int & (1 << 22))
    _log.debug("Shader Flags 1: type=%s, value=%s, greyscale=%s, emit=%s",
               type(sf1).__name__, sf1, greyscale_color, has_emit)

    palette_scale = float(shader_prop.get_field("Grayscale to Palette Scale") or 1.0)

    # Emissive color
    emit_color_val = shader_prop.get_field("Emissive Color")
    if hasattr(emit_color_val, "r"):
        ec = [float(emit_color_val.r), float(emit_color_val.g), float(emit_color_val.b)]
    elif isinstance(emit_color_val, dict):
        ec = [float(emit_color_val.get("r", 0)),
              float(emit_color_val.get("g", 0)),
              float(emit_color_val.get("b", 0))]
    elif isinstance(emit_color_val, (list, tuple)) and len(emit_color_val) >= 3:
        ec = [float(x) for x in emit_color_val[:3]]
    else:
        ec = [0.0, 0.0, 0.0]

    return {
        "spec_color": glm.vec3(*sc),
        "spec_strength": float(shader_prop.get_field("Specular Strength") or 1.0),
        "spec_glossiness": float(shader_prop.get_field("Glossiness") or 80.0) / 100.0,
        "fresnel_power": float(shader_prop.get_field("Fresnel Power") or 5.0),
        "uv_scale_offset": glm.vec4(
            float(uv_scale.get("U", 1.0)) if isinstance(uv_scale, dict) else 1.0,
            float(uv_scale.get("V", 1.0)) if isinstance(uv_scale, dict) else 1.0,
            float(uv_offset.get("U", 0.0)) if isinstance(uv_offset, dict) else 0.0,
            float(uv_offset.get("V", 0.0)) if isinstance(uv_offset, dict) else 0.0,
        ),
        "greyscale_color": greyscale_color,
        "palette_scale": palette_scale,
        "has_emit": has_emit,
        "glow_color": glm.vec3(*ec),
        "glow_mult": float(shader_prop.get_field("Emissive Multiple") or 1.0),
    }


def _get_ref_id(ref) -> int:
    """Extract block index from a reference value."""
    if isinstance(ref, (int, float)):
        return int(ref)
    if isinstance(ref, dict):
        return int(ref.get("value", ref.get("Value", -1)))
    return -1


def _get_shape_property_block(nif, shape_block, field_name: str, property_type: str):
    """Resolve a shape property from a direct field or legacy Properties[] list."""
    direct_ref = shape_block.get_field(field_name)
    ref_id = _get_ref_id(direct_ref) if direct_ref is not None else -1
    if ref_id >= 0:
        try:
            prop = nif.get_block(ref_id)
        except (IndexError, KeyError):
            prop = None
        if prop:
            return prop

    props = shape_block.get_field("Properties") or []
    if not isinstance(props, list):
        return None
    schema = getattr(nif, "schema", None)
    for prop_ref in props:
        prop_id = _get_ref_id(prop_ref)
        if prop_id < 0:
            continue
        try:
            prop = nif.get_block(prop_id)
        except (IndexError, KeyError):
            continue
        if not prop:
            continue
        if schema and schema.is_subtype_of(prop.type_name, property_type):
            return prop
        if prop.type_name == property_type:
            return prop
    return None


def _get_material_name(shader_prop) -> tuple[str, str]:
    mat_name = shader_prop.get_field("Name") or ""
    if isinstance(mat_name, list):
        mat_name = "".join(str(c) for c in mat_name)
    if not isinstance(mat_name, str):
        mat_name = str(mat_name)
    mat_name = mat_name.strip().rstrip("\x00")
    return mat_name, mat_name.lower()


def _is_material_file_name(mat_name_lower: str) -> bool:
    return (
        mat_name_lower.endswith(".bgsm")
        or mat_name_lower.endswith(".bgem")
        or mat_name_lower.endswith(".mat")
    )


def _resolve_material_file(
    mat_name: str,
    texture_dirs: list[Path],
    ba2_mgr=None,
) -> Path | bytes | None:
    resolved = _resolve_texture_path(mat_name, texture_dirs, ba2_mgr)
    if not resolved:
        resolved = _resolve_texture_path(f"Materials/{mat_name}", texture_dirs, ba2_mgr)
    return resolved


def _material_cache_key(mat_name_lower: str, resolved: Path | bytes) -> str:
    if isinstance(resolved, Path):
        return "file:" + os.path.normcase(os.path.abspath(resolved))
    digest = hashlib.blake2b(resolved, digest_size=16).hexdigest()
    return f"ba2:{mat_name_lower}:{digest}"


def _missing_material_cache_key(
    mat_name_lower: str,
    texture_dirs: list[Path],
    ba2_mgr=None,
) -> str:
    dirs_key = "|".join(os.path.normcase(os.path.abspath(d)) for d in texture_dirs)
    return f"missing:{mat_name_lower}:{dirs_key}:ba2={ba2_mgr is not None}"


def _remember_material_source(cache_key: str, resolved: Path | bytes) -> None:
    if isinstance(resolved, Path):
        _material_cache_sources[cache_key] = str(resolved)


def _infer_sibling_texture_path(
    texture_path: str,
    from_suffix: str,
    to_suffix: str,
    texture_dirs: list[Path],
    ba2_mgr=None,
) -> str:
    if not texture_path:
        return ""
    clean = texture_path.replace("\\", "/").strip().rstrip("\x00")
    if not clean.lower().endswith(from_suffix):
        return ""
    candidate = clean[: -len(from_suffix)] + to_suffix
    return candidate if _resolve_texture_path(candidate, texture_dirs, ba2_mgr) is not None else ""


def _get_texture_paths(nif, shader_prop, block_type: str,
                       texture_dirs: list[Path], ba2_mgr=None) -> dict[str, str]:
    """Extract texture paths from shader property."""
    paths: dict[str, str] = {}

    # --- Try loading BGSM/BGEM material file from the Name field first ---
    # Both BSEffectShaderProperty and BSLightingShaderProperty can reference
    # .bgem/.bgsm files.  The material file provides textures and params that
    # override whatever is stored directly on the NIF block.
    mat_name, mat_name_lower = _get_material_name(shader_prop)
    if mat_name and _is_material_file_name(mat_name_lower):
        # Resolve before checking the material cache. FO4 and FO76 can have
        # the same relative BGSM name with different slot contents.
        _log.debug("Material name on shader: %s", mat_name)
        resolved = _resolve_material_file(mat_name, texture_dirs, ba2_mgr)
        if resolved:
            cache_key = _material_cache_key(mat_name_lower, resolved)
            if cache_key in _material_cache:
                cached_paths, cached_mat = _material_cache[cache_key]
                if cached_paths is not None:
                    # Return a copy so callers can pop keys without affecting cache
                    result = dict(cached_paths)
                    result["_material_data"] = cached_mat
                    return result
                # cached_paths is None -> material couldn't be parsed,
                # fall through to NIF fallback below
            else:
                _remember_material_source(cache_key, resolved)
                _log.debug("Material resolved: %s (%s)", mat_name,
                           f"{len(resolved)} bytes" if isinstance(resolved, bytes) else resolved)
                if mat_name_lower.endswith(".mat"):
                    mat_data = _parse_mat(resolved)
                elif mat_name_lower.endswith(".bgem"):
                    mat_data = _parse_bgem(resolved)
                else:
                    mat_data = _parse_bgsm(resolved)
                if mat_data:
                    _log.debug("Material parsed: textures=%s", {
                        k: v for k, v in mat_data.items()
                        if isinstance(v, str) and v
                    })
                    paths["diffuse"] = mat_data.get("diffuse", "")
                    paths["normal"] = mat_data.get("normal", "")
                    is_fo76_pbr_bgsm = mat_data.get("type") == "bgsm" and bool(mat_data.get("pbr"))
                    specular_direct = "" if is_fo76_pbr_bgsm else mat_data.get("specular", "")
                    specular_fallback = "" if is_fo76_pbr_bgsm else mat_data.get("reflectivity", "")
                    paths["specular"] = (
                        mat_data.get("smooth_spec", "")
                        or specular_direct
                        or mat_data.get("roughness", "")
                        or specular_fallback
                    )
                    paths["metallic"] = mat_data.get("metallic", "")
                    paths["ao"] = mat_data.get("ambientocclusion", "")
                    paths["glow"] = mat_data.get("glow", "") or mat_data.get("emissive", "")
                    if is_fo76_pbr_bgsm and not paths["glow"]:
                        paths["glow"] = _infer_sibling_texture_path(
                            paths["diffuse"], "_d.dds", "_g.dds", texture_dirs, ba2_mgr
                        )
                    paths["reflectivity"] = mat_data.get("reflectivity", "")
                    paths["lighting"] = mat_data.get("lighting", "")
                    paths["greyscale"] = mat_data.get("greyscale", "")
                    paths["cubemap"] = mat_data.get("cubemap", "") or mat_data.get("envmap", "")
                    paths["envmask"] = mat_data.get("envmask", "")
                    paths["opacity"] = mat_data.get("opacity", "")
                    for key in (
                        "inner_layer",
                        "wrinkles",
                        "displacement",
                        "flow",
                        "distance_field_alpha",
                        "glass_roughness_scratch",
                        "glass_dirt_overlay",
                    ):
                        paths[key] = mat_data.get(key, "")
                    paths["_grayscale_to_palette_color"] = mat_data.get("grayscale_to_palette_color", False)
                    paths["_palette_scale"] = mat_data.get("palette_scale", 1.0)
                    paths["_material_data"] = mat_data
                    # Cache the texture paths (without _material_data) and mat_data separately
                    cached = {k: v for k, v in paths.items() if k != "_material_data"}
                    _lru_put(_material_cache, cache_key, (cached, mat_data), _MAX_MATERIAL_CACHE)
                    return paths
                else:
                    _log.warning("Material parse FAILED for %s", mat_name)
                    _lru_put(_material_cache, cache_key, (None, None), _MAX_MATERIAL_CACHE)
        else:
            missing_key = _missing_material_cache_key(mat_name_lower, texture_dirs, ba2_mgr)
            if missing_key in _material_cache:
                cached_paths, cached_mat = _material_cache[missing_key]
            else:
                _log.warning("Could not resolve material: %s (texture_dirs=%s, ba2=%s)",
                             mat_name, [str(d) for d in texture_dirs[:3]],
                             "yes" if ba2_mgr else "no")
                _lru_put(_material_cache, missing_key, (None, None), _MAX_MATERIAL_CACHE)
                cached_paths, cached_mat = None, None
            if cached_paths is not None:
                # Return a copy so callers can pop keys without affecting cache
                result = dict(cached_paths)
                result["_material_data"] = cached_mat
                return result

    # --- Fallback: read texture slots directly from the NIF block ---
    if "BSEffectShaderProperty" in block_type:
        paths["diffuse"] = shader_prop.get_field("Source Texture") or ""
        paths["normal"] = shader_prop.get_field("Normal Texture") or ""
        paths["cubemap"] = shader_prop.get_field("Env Map Texture") or ""
        paths["greyscale"] = shader_prop.get_field("Greyscale Texture") or ""
        paths["specular"] = shader_prop.get_field("Env Mask Texture") or ""
        return paths

    # Fall back to BSShaderTextureSet
    tex_set_ref = shader_prop.get_field("Texture Set")
    ref_id = _get_ref_id(tex_set_ref) if tex_set_ref is not None else -1
    if ref_id >= 0:
        tex_set = nif.get_block(ref_id)
        if tex_set:
            textures = tex_set.get_field("Textures") or []
            if len(textures) > SLOT_DIFFUSE:
                paths["diffuse"] = textures[SLOT_DIFFUSE] or ""
            if len(textures) > SLOT_NORMAL:
                paths["normal"] = textures[SLOT_NORMAL] or ""
            if len(textures) > SLOT_GLOW:
                paths["glow"] = textures[SLOT_GLOW] or ""
            if len(textures) > SLOT_GREYSCALE:
                paths["greyscale"] = textures[SLOT_GREYSCALE] or ""
            if len(textures) > SLOT_CUBEMAP:
                paths["cubemap"] = textures[SLOT_CUBEMAP] or ""
            if len(textures) > SLOT_ENVMASK:
                paths["envmask"] = textures[SLOT_ENVMASK] or ""
            if len(textures) > SLOT_SPECULAR:
                paths["specular"] = textures[SLOT_SPECULAR] or ""

    return paths


def _get_decoded(path_str: str, texture_dirs, ba2_mgr) -> "DecodedTexture | None":
    """Return a DecodedTexture from cache or decode fresh (no GPU upload)."""
    if not path_str:
        return None
    resolved = _resolve_texture_path(path_str, texture_dirs, ba2_mgr)
    if resolved is None:
        return None
    if isinstance(resolved, bytes):
        key = f"ba2:{path_str.lower().replace(chr(92), '/')}"
        if key in _decode_cache:
            return _decode_cache[key]
        return decode_texture_bytes(resolved, name=key)
    key = str(resolved)
    if key in _decode_cache:
        return _decode_cache[key]
    from .dds_loader import decode_texture
    return decode_texture(key)


def _bake_opacity_into_diffuse(
    ctx: "moderngl.Context", diffuse_path: str, opacity_path: str,
    texture_dirs, ba2_mgr,
) -> "moderngl.Texture | None":
    """Bake a separate Starfield opacity texture into the diffuse alpha channel.

    Starfield decal materials store opacity as a dedicated BC4 single-channel
    texture instead of encoding it in the diffuse alpha.  Decode both, copy
    the opacity R channel into diffuse A, and upload the combined RGBA texture.
    """
    diff_dec = _get_decoded(diffuse_path, texture_dirs, ba2_mgr)
    opac_dec = _get_decoded(opacity_path, texture_dirs, ba2_mgr)
    if diff_dec is None:
        return None
    w, h = diff_dec.size
    arr = np.frombuffer(diff_dec.data, dtype=np.uint8).reshape(h, w, 4).copy()
    if opac_dec is not None and opac_dec.size == (w, h):
        oarr = np.frombuffer(opac_dec.data, dtype=np.uint8).reshape(h, w, 4)
        arr[:, :, 3] = oarr[:, :, 0]  # R channel of opacity → diffuse alpha
    else:
        # Opacity texture missing or size mismatch — leave diffuse alpha as-is
        _log.debug("Opacity bake skipped: opac=%s, diff_size=%s",
                   opac_dec.size if opac_dec else None, (w, h))
    combined = DecodedTexture(size=(w, h), components=4, data=arr.tobytes())
    return upload_decoded(ctx, combined)


def build_material(ctx: moderngl.Context, nif, shape_block,
                   texture_dirs: list[Path], ba2_mgr=None,
                   game_id: str = "fo4",
                   brdf_lut: "moderngl.Texture | None" = None) -> Material:
    """Build a Material by dispatching to the appropriate game backend."""
    if game_id == "starfield":
        # Auto-generate BRDF LUT if caller didn't provide one. The shader_pipeline
        # caches it module-level, so repeated calls are free.
        if brdf_lut is None:
            from .shader_pipeline import generate_sf_pbr_lut
            brdf_lut = generate_sf_pbr_lut(ctx)
        from .sf_material import SFMaterialBackend
        backend = SFMaterialBackend(brdf_lut=brdf_lut)
        _log.debug("build_material: dispatching to SFMaterialBackend (brdf_lut=%s)",
                   "set" if brdf_lut else "None")
    else:
        from .fo4_material import FO4MaterialBackend
        backend = FO4MaterialBackend()
    return backend.build_material(ctx, nif, shape_block, texture_dirs, ba2_mgr)


def _is_texture_collectable_shape(schema, type_name: str) -> bool:
    return (
        schema.is_subtype_of(type_name, "BSTriShape")
        or schema.is_subtype_of(type_name, "NiTriShape")
        or schema.is_subtype_of(type_name, "NiTriStrips")
    )


def collect_nif_texture_paths(
    nif, texture_dirs: list[Path], ba2_mgr=None,
) -> dict[str, "Path | bytes"]:
    """Walk all BSTriShape blocks and return resolved textures for pre-decode.

    Returns {cache_key: Path_or_bytes} for every texture referenced by the NIF
    (including BGSM/BGEM material textures and BA2-sourced textures).

    Used by nif_loader for parallel pre-decode before scene build.
    """
    result: dict[str, Path | bytes] = {}
    schema = nif.schema
    for block in nif.blocks:
        if not _is_texture_collectable_shape(schema, block.type_name):
            continue
        shader_prop = _get_shape_property_block(
            nif, block, "Shader Property", "BSShaderProperty"
        )
        if not shader_prop:
            continue
        tex_paths = _get_texture_paths(nif, shader_prop, shader_prop.type_name,
                                       texture_dirs, ba2_mgr)
        for key, path_str in tex_paths.items():
            if key.startswith("_") or not path_str or not isinstance(path_str, str):
                continue
            resolved = _resolve_texture_path(path_str, texture_dirs, ba2_mgr)
            if resolved is None:
                continue
            if isinstance(resolved, bytes):
                cache_key = f"ba2:{path_str.lower().replace(chr(92), '/')}"
                if cache_key not in result:
                    result[cache_key] = resolved
            else:
                cache_key = str(resolved)
                if cache_key not in result:
                    result[cache_key] = resolved
    return result


def collect_nif_material_paths(
    nif, texture_dirs: list[Path], ba2_mgr=None,
) -> dict[str, str]:
    """Return loose material files referenced by renderable NIF shapes.

    The result maps absolute material file path to the normalized NIF material
    name. BA2-backed materials are skipped because filesystem watchers cannot
    observe archive entries.
    """
    result: dict[str, str] = {}
    schema = nif.schema
    for block in nif.blocks:
        if not _is_texture_collectable_shape(schema, block.type_name):
            continue
        shader_prop = _get_shape_property_block(
            nif, block, "Shader Property", "BSShaderProperty"
        )
        if not shader_prop:
            continue
        mat_name, mat_name_lower = _get_material_name(shader_prop)
        if not mat_name or not _is_material_file_name(mat_name_lower):
            continue
        resolved = _resolve_material_file(mat_name, texture_dirs, ba2_mgr)
        if isinstance(resolved, Path):
            result[str(resolved)] = mat_name_lower
    return result


