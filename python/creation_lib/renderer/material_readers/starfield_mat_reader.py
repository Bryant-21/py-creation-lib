"""Starfield .mat JSON material reader.

Parses .mat files (JSON format), extracts all Summary layers (Layer1-Layer6)
and Blenders, maps semantic names to unified texture keys, and handles import
chain resolution for inherited materials.
"""
from __future__ import annotations
import json
import logging
from pathlib import Path

from .base import MaterialData, LayerData, BlenderData, AlphaSettings, DecalSettings, EmissiveSettings

_log = logging.getLogger("nif_editor.material_readers.starfield_mat")

# Starfield .mat semantic name -> unified key mapping
_SEMANTIC_MAP = {
    "albedo": "diffuse",
    "normal": "normal",
    "roughness": "roughness",
    "metalness": "metallic",
    "ao": "ao",
    "ambientocclusion": "ao",
    "emissive": "glow",
    "height": "height",
    "opacity": "opacity",
}

# Maximum import chain depth to prevent infinite loops
_MAX_IMPORT_DEPTH = 10

# Layer keys in order
_LAYER_KEYS = [f"Layer{i}" for i in range(1, 7)]

# Blender mode mapping (Starfield internal names -> our canonical names)
_BLEND_MODE_MAP = {
    "lerp": "linear",
    "linear": "linear",
    "additive": "additive",
    "add": "additive",
    "positioncontrast": "position_contrast",
    "position_contrast": "position_contrast",
    "multiply": "multiply",
    "screen": "screen",
}


def read_mat_json(path: Path | str, _depth: int = 0) -> MaterialData:
    """Parse a Starfield .mat JSON file and return MaterialData.

    Args:
        path: Path to the .mat file.
        _depth: Internal recursion depth for import chain.

    Returns:
        MaterialData with all layers, blenders, and metallic-roughness model.
    """
    path = Path(path)
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as e:
        _log.warning("Failed to read .mat file %s: %s", path, e)
        return MaterialData(material_model="metallic-roughness")

    # Handle import chain (child overrides parent)
    if _depth < _MAX_IMPORT_DEPTH:
        for import_path in raw.get("Import", []):
            import_p = Path(import_path)
            # Try relative to the current .mat file's directory
            if not import_p.is_absolute():
                import_p = path.parent / import_p
            if import_p.exists():
                parent = _load_raw_json(import_p)
                if parent:
                    raw = _merge_mat(parent, raw)

    # Extract all layers + blenders. Blenders are nested under each layer
    # past Layer1 (mirrors tools/sf_render_test.py:parse_mat line 368-376):
    # Layer{i}.Blender holds the blend config between Layer{i-1} and Layer{i}.
    summary = raw.get("Summary", {})
    layers: list[LayerData] = []
    blenders: list[BlenderData] = []
    for i, key in enumerate(_LAYER_KEYS, start=1):
        layer_json = summary.get(key)
        if layer_json is None:
            continue
        layers.append(_extract_layer(layer_json))
        if i > 1:
            blender_json = layer_json.get("Blender") or {}
            blenders.append(_extract_blender(blender_json))

    # Backward compat: texture_paths = Layer1 textures
    texture_paths = layers[0].texture_paths if layers else {}

    # Parse Objects section for component data
    alpha_settings, decal_settings, emissive_settings, normal_intensities = \
        _extract_objects_section(raw)

    # Apply normal intensities to layers
    for layer_idx, intensity in normal_intensities.items():
        if layer_idx < len(layers):
            layers[layer_idx].normal_intensity = intensity

    # Decal-ness often comes only from the parent imported ShaderModel (e.g.
    # `1LayerStandardDecal.mat`), which may be missing from extracted/. Like
    # tools/sf_render_test.py:parse_mat, fall back to a filename + Import-list
    # heuristic so tombstone-style decals get polygon offset + alpha blending.
    filename_l = (raw.get("Filename") or path.name).lower()
    parent_imports = [str(p).lower() for p in (raw.get("Import") or [])]
    parent_imports.append(str(summary.get("Layer1", {}).get("Parent", "")).lower())
    looks_like_decal = (
        "decal" in filename_l
        or any("decal" in p for p in parent_imports)
    )
    if looks_like_decal:
        if decal_settings is None:
            decal_settings = DecalSettings(
                is_decal=True,
                material_overall_alpha=1.0,
                render_layer=0,
                blend_mode=0,
            )
        else:
            decal_settings.is_decal = True
        if alpha_settings is None:
            alpha_settings = AlphaSettings(
                has_opacity=True,
                is_decal=True,
                alpha_test_threshold=0.5,
            )
        else:
            alpha_settings.has_opacity = True
            alpha_settings.is_decal = True

    return MaterialData(
        texture_paths=texture_paths,
        params={},
        material_model="metallic-roughness",
        layers=layers,
        blenders=blenders,
        alpha_settings=alpha_settings,
        decal_settings=decal_settings,
        emissive_settings=emissive_settings,
    )


def _load_raw_json(path: Path) -> dict | None:
    """Load raw JSON from a .mat file."""
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as e:
        _log.debug("Failed to load parent .mat %s: %s", path, e)
        return None


def _merge_mat(parent: dict, child: dict) -> dict:
    """Deep merge parent into child (child wins on conflict)."""
    result = {}
    for key in set(parent) | set(child):
        if key in child and key in parent:
            if isinstance(child[key], dict) and isinstance(parent[key], dict):
                result[key] = _merge_mat(parent[key], child[key])
            else:
                result[key] = child[key]  # child wins
        elif key in child:
            result[key] = child[key]
        else:
            result[key] = parent[key]
    return result


def _extract_layer(layer_json: dict) -> LayerData:
    """Extract a single layer's textures, tint, opacity, and UV transform."""
    textures = _extract_textures(layer_json.get("Textures", {}))

    # Tint color
    tint = None
    tint_json = layer_json.get("TintColor")
    if isinstance(tint_json, dict):
        tint = (
            float(tint_json.get("R", 1.0)),
            float(tint_json.get("G", 1.0)),
            float(tint_json.get("B", 1.0)),
        )

    # Opacity
    opacity = float(layer_json.get("Opacity", 1.0))

    # UVStream — controls how this layer samples mesh UVs.
    # Matches tools/sf_render_test.py:_parse_summary_layer (line 433-438):
    # Channel is an int (0 = UV1, 1 = UV2), Scale/Offset use lowercase x/y keys.
    uv = layer_json.get("UVStream") or {}
    uv_channel = int(uv.get("Channel", 0) or 0)
    scale = uv.get("Scale") or {}
    offset = uv.get("Offset") or {}
    uv_scale = (float(scale.get("x", 1.0)), float(scale.get("y", 1.0)))
    uv_offset = (float(offset.get("x", 0.0)), float(offset.get("y", 0.0)))

    return LayerData(
        texture_paths=textures,
        tint_color=tint,
        opacity=opacity,
        uv_channel=uv_channel,
        uv_scale=uv_scale,
        uv_offset=uv_offset,
    )


def _extract_textures(textures_json: dict) -> dict[str, str]:
    """Extract texture paths from a layer's Textures dict."""
    result: dict[str, str] = {}
    for name, info in textures_json.items():
        if not isinstance(info, dict):
            continue
        # Skip replacement textures (placeholders)
        if info.get("UseReplacement", False):
            continue
        file_path = info.get("File", "")
        if not file_path:
            continue
        # Map semantic name to unified key
        unified = _SEMANTIC_MAP.get(name.lower(), name.lower())
        # Normalize path separators
        result[unified] = file_path.replace("\\", "/")
    return result


def _extract_blender(blender_json: dict) -> BlenderData:
    """Extract blender configuration."""
    # Mode
    raw_mode = str(blender_json.get("BlendMode", "linear")).lower().strip()
    mode = _BLEND_MODE_MAP.get(raw_mode, raw_mode)

    # Mask texture
    mask_tex = None
    mask_info = blender_json.get("MaskTexture")
    if isinstance(mask_info, dict):
        mask_file = mask_info.get("File", "")
        if mask_file and not mask_info.get("UseReplacement", False):
            mask_tex = mask_file.replace("\\", "/")
    elif isinstance(mask_info, str) and mask_info:
        mask_tex = mask_info.replace("\\", "/")

    # Vertex color channel
    vc_channel = None
    vc_raw = blender_json.get("VertexColorChannel")
    if isinstance(vc_raw, str) and vc_raw.lower() in ("r", "g", "b", "a"):
        vc_channel = vc_raw.lower()

    # Height blend
    threshold = float(blender_json.get("HeightBlendThreshold", 0.5))
    factor = float(blender_json.get("HeightBlendFactor", 1.0))

    # MaskIntensity + per-channel blend gates. Defaults match
    # tools/sf_render_test.py:_parse_summary_blender (line 460-464):
    # only Normal is True by default; for tombstone the detail blender has
    # only BlendTextureNormal=true so diffuse/rough/metal should NOT be mixed.
    mask_intensity = float(blender_json.get("MaskIntensity", 1.0))
    blend_albedo = bool(blender_json.get("BlendTextureAlbedo", False))
    blend_normal = bool(blender_json.get("BlendTextureNormal", True))
    blend_metal = bool(blender_json.get("BlendTextureMetal", False))
    blend_rough = bool(blender_json.get("BlendTextureRoughness", False))
    blend_ao = bool(blender_json.get("BlendTextureAo", False))

    return BlenderData(
        mode=mode,
        mask_texture=mask_tex,
        vertex_color_channel=vc_channel,
        height_blend_threshold=threshold,
        height_blend_factor=factor,
        mask_intensity=mask_intensity,
        blend_albedo=blend_albedo,
        blend_normal=blend_normal,
        blend_metal=blend_metal,
        blend_rough=blend_rough,
        blend_ao=blend_ao,
    )


def _extract_objects_section(raw: dict) -> tuple[AlphaSettings | None, DecalSettings | None, EmissiveSettings | None, dict[int, float]]:
    """Parse the Objects section of a Starfield .mat file.

    Returns (alpha_settings, decal_settings, emissive_settings, layer_normal_intensities).
    layer_normal_intensities maps layer index (0-based) -> floatParam value.
    """
    objects = raw.get("Objects", [])
    if not objects:
        return None, None, None, {}

    alpha = None
    decal = None
    emissive = None
    normal_intensities: dict[int, float] = {}

    for obj in objects:
        if not isinstance(obj, dict):
            continue
        obj_type = obj.get("Type", "") or obj.get("type", "")
        data = obj.get("Data", obj)  # some formats nest under Data

        if "AlphaSettingsComponent" in obj_type:
            alpha = AlphaSettings(
                has_opacity=bool(data.get("HasOpacity", False)),
                opacity_source_layer=int(data.get("OpacitySourceLayer", 0)),
                is_decal=bool(data.get("IsDecal", False)),
                alpha_test_threshold=float(data.get("AlphaTestThreshold", 0.5)),
            )

        elif "DecalSettingsComponent" in obj_type:
            decal = DecalSettings(
                is_decal=bool(data.get("IsDecal", False)),
                material_overall_alpha=float(data.get("MaterialOverallAlpha", 1.0)),
                write_mask=int(data.get("WriteMask", 0xFFFFFFFF)),
                render_layer=int(data.get("RenderLayer", 0)),
                blend_mode=int(data.get("BlendMode", 0)),
                is_projected=bool(data.get("IsProjected", False)),
                use_parallax_occlusion=bool(data.get("UseParallaxOcclusionMapping", False)),
                parallax_scale=float(data.get("ParallaxOcclusionScale", 0.0)),
                max_parallax_steps=int(data.get("MaxParallaxOcclusionSteps", 200)),
            )

        elif "EmissiveSettingsComponent" in obj_type:
            emissive = EmissiveSettings(
                is_enabled=bool(data.get("IsEnabled", False)),
                emissive_source_layer=int(data.get("EmissiveSourceLayer", 0)),
                luminous_emittance=float(data.get("LuminousEmittance", 0.0)),
                adaptive_emittance=bool(data.get("AdaptiveEmittance", False)),
            )

        # Per-texture floatParam (normal intensity) — look for TextureSet objects
        # that contain a NormalTexture with a floatParam
        elif "TextureSet" in obj_type or "LayeredMaterial" in obj_type:
            _extract_normal_intensities(data, normal_intensities)

    return alpha, decal, emissive, normal_intensities


def _extract_normal_intensities(data: dict, result: dict[int, float]):
    """Extract per-layer normal intensity (floatParam) from TextureSet objects."""
    # The .mat Objects section stores TextureSet data with a "Textures" list
    # Each texture entry can have a "FloatParam" or "floatParam" field
    textures = data.get("Textures", {})
    layer_idx = int(data.get("LayerIndex", data.get("layerIndex", 0)))

    for name, info in textures.items() if isinstance(textures, dict) else []:
        if not isinstance(info, dict):
            continue
        if name.lower() == "normal":
            fp = info.get("FloatParam", info.get("floatParam", None))
            if fp is not None:
                result[layer_idx] = float(fp)
