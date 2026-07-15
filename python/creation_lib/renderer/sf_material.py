"""Starfield material backend.

Handles .mat JSON materials with full Objects section parsing,
separate roughness/metallic textures, multi-layer support,
per-layer normal maps, and decal/alpha/emissive settings.
"""
from __future__ import annotations
import logging
import struct as _struct

import moderngl
import glm

from creation_lib.renderer.scene_renderer import Material
from creation_lib.renderer.material_readers.base import (
    RenderFlags, AlphaSettings, DecalSettings, EmissiveSettings,
)
from creation_lib.renderer.material_pipeline import (
    _resolve_texture_path, _cached_load, _get_decoded,
    _get_ref_id, _extract_alpha_flags,
    _lru_put, _tex_cache, _MAX_TEX_CACHE, _release_texture,
    _material_cache, _MAX_MATERIAL_CACHE,
    upload_decoded, DecodedTexture,
    _bake_opacity_into_diffuse,
)

_log = logging.getLogger("nif_editor.sf_material")


class SFMaterialBackend:
    """Material backend for Starfield."""

    def __init__(self, brdf_lut: moderngl.Texture | None = None,
                 specular_cube: "moderngl.TextureCube | None" = None,
                 irradiance_cube: "moderngl.TextureCube | None" = None):
        self.brdf_lut = brdf_lut
        self.specular_cube = specular_cube
        self.irradiance_cube = irradiance_cube

    def build_material(self, ctx, nif, shape_block, texture_dirs, ba2_mgr=None):
        """Build a Material from a Starfield BSTriShape."""
        mat = Material()
        mat.material_model = "metallic-roughness"

        shape_name = shape_block.get_field("Name") or shape_block.type_name
        shader_ref = shape_block.get_field("Shader Property")
        ref_id = _get_ref_id(shader_ref) if shader_ref is not None else -1
        if ref_id < 0:
            return mat

        shader_prop = nif.get_block(ref_id)
        if not shader_prop:
            return mat

        block_type = shader_prop.type_name
        is_effect = "BSEffectShaderProperty" in block_type

        # Alpha flags from NiAlphaProperty
        mat.alpha_flags, mat.alpha_threshold, mat.blend_src, mat.blend_dst = \
            _extract_alpha_flags(nif, shape_block)

        # Effect shader setup
        if is_effect:
            mat.is_effect_shader = True
            if mat.alpha_flags == 0 and mat.blend_src == 0 and mat.blend_dst == 0:
                mat.alpha_flags |= 8

        # Get texture paths (includes .mat parsing)
        from .material_pipeline import _get_texture_paths
        tex_paths = _get_texture_paths(nif, shader_prop, block_type,
                                       texture_dirs, ba2_mgr)

        tex_paths.pop("_grayscale_to_palette_color", None)
        tex_paths.pop("_palette_scale", None)
        mat_data = tex_paths.pop("_material_data", None)

        # Store .mat parsed data for render flags
        self._alpha_settings = None
        self._decal_settings = None
        self._emissive_settings = None
        layer_normal_intensities = {}

        if mat_data and mat_data.get("type") == "mat":
            layers = mat_data.get("_layers", [])
            blenders = mat_data.get("_blenders", [])
            mat.layer_count = min(mat_data.get("_layer_count", 1), 3)

            # Propagate parsed .mat Objects-section settings to the Material so
            # the renderer (polygon offset, alpha flags) and shader (sfIsDecal,
            # sfDecalAlpha) can act on them.
            decal = mat_data.get("_decal_settings")
            if decal is not None:
                mat._is_decal = bool(getattr(decal, 'is_decal', False))
                mat._render_layer = int(getattr(decal, 'render_layer', 0))
                mat._decal_alpha = float(
                    getattr(decal, 'material_overall_alpha', 1.0))
            alpha_s = mat_data.get("_alpha_settings")
            if alpha_s is not None and getattr(alpha_s, 'is_decal', False):
                mat._is_decal = True

            # Extract layer normal intensities
            for i, layer in enumerate(layers):
                if hasattr(layer, 'normal_intensity'):
                    layer_normal_intensities[i] = layer.normal_intensity

            # Load textures for all layers
            self._load_starfield_textures(ctx, mat, tex_paths, layers, blenders,
                                         texture_dirs, ba2_mgr,
                                         layer_normal_intensities)
        else:
            # No .mat data — load basic textures
            self._load_basic_textures(ctx, mat, tex_paths, texture_dirs, ba2_mgr)

        # Store normal intensity for base layer
        mat._normal_scale = layer_normal_intensities.get(0, 1.0)
        mat._layer_normal_scales = [
            layer_normal_intensities.get(i, 1.0) for i in range(mat.layer_count)
        ]

        # Effect shader source texture flag
        if mat.is_effect_shader:
            mat.has_source_texture = mat.diffuse_tex is not None

        return mat

    def get_render_flags(self, mat: Material) -> RenderFlags:
        """Derive render flags from Starfield material + DecalSettings.

        Matches tools/sf_render_test.py SFScene.render (lines 1644-1666):
        `is_decal or has_opacity` → transparent pass with depth_mask=False,
        blend enabled, polygon offset (-2, -2). Decals must share the
        transparent pass to get the poly-offset z-bias correctly.
        """
        is_blended = (mat.alpha_flags & 8) != 0
        is_decal = getattr(mat, '_is_decal', False)
        render_layer = getattr(mat, '_render_layer', 0)
        needs_transparent = is_blended or is_decal

        offset = None
        if needs_transparent:
            factor = -2.0 - float(render_layer)
            offset = (factor, factor)

        return RenderFlags(
            depth_write=not needs_transparent,
            depth_test=True,
            polygon_offset=offset,
            render_layer=render_layer,
            blend_enabled=needs_transparent,
            blend_src=mat.blend_src,
            blend_dst=mat.blend_dst,
            alpha_flags=mat.alpha_flags,
            alpha_threshold=mat.alpha_threshold,
            double_sided=mat.double_sided,
        )

    def _load_starfield_textures(self, ctx, mat, tex_paths, layers, blenders,
                                  texture_dirs, ba2_mgr, normal_intensities):
        """Load all Starfield textures with separate roughness/metallic."""

        def _resolve_and_load(tex_path_str):
            if not tex_path_str:
                return None
            resolved = _resolve_texture_path(tex_path_str, texture_dirs, ba2_mgr)
            if resolved is None:
                return None
            if isinstance(resolved, bytes):
                cache_key = f"ba2:{tex_path_str.lower().replace(chr(92), '/')}"
                return _cached_load(ctx, resolved, cache_key=cache_key)
            return _cached_load(ctx, resolved)

        # --- Base layer (layer 0) ---
        # Diffuse with opacity baking
        diffuse_path = tex_paths.get("diffuse", "")
        opacity_path = tex_paths.get("opacity", "")
        if diffuse_path and not diffuse_path.lower().endswith("_n.dds"):
            if opacity_path:
                tex = _bake_opacity_into_diffuse(
                    ctx, diffuse_path, opacity_path, texture_dirs, ba2_mgr)
                if tex:
                    mat.diffuse_tex = tex
                    mat.alpha_flags |= 4 | 8
                    mat.alpha_threshold = 128 / 255.0
            if not mat.diffuse_tex:
                mat.diffuse_tex = _resolve_and_load(diffuse_path)

        # Normal map (base layer)
        if tex_paths.get("normal"):
            mat.normal_tex = _resolve_and_load(tex_paths["normal"])

        # SEPARATE roughness texture (not packed!)
        roughness_path = tex_paths.get("specular", "") or tex_paths.get("roughness", "")
        mat._roughness_tex = _resolve_and_load(roughness_path) if roughness_path else None

        # SEPARATE metallic texture (not packed!)
        metallic_path = tex_paths.get("metallic", "")
        mat._metallic_tex = _resolve_and_load(metallic_path) if metallic_path else None

        # AO texture (separate in Starfield .mat — AmbientOcclusion field)
        ao_path = tex_paths.get("ao", "")
        mat._ao_tex = _resolve_and_load(ao_path) if ao_path else None

        # Glow
        if tex_paths.get("glow"):
            tex = _resolve_and_load(tex_paths["glow"])
            if tex:
                mat.glow_tex = tex
                mat.has_glow_map = True

        # --- Additional layers ---
        for i in range(1, mat.layer_count):
            if i >= len(layers):
                break
            layer = layers[i]

            # Albedo
            albedo_tex = _resolve_and_load(layer.texture_paths.get("diffuse", ""))
            mat.layer_albedos.append(albedo_tex)

            # Normal (per-layer)
            normal_tex = _resolve_and_load(layer.texture_paths.get("normal", ""))
            mat.layer_normals.append(normal_tex)

            # Separate roughness + metallic for additional layers
            rough_tex = _resolve_and_load(layer.texture_paths.get("roughness", ""))
            metal_tex = _resolve_and_load(layer.texture_paths.get("metallic", ""))
            mat.layer_specs.append((rough_tex, metal_tex))  # tuple instead of packed

        # Layer tints and opacities
        for i in range(mat.layer_count):
            if i < len(layers):
                tint = layers[i].tint_color or (1.0, 1.0, 1.0)
                mat.layer_tints.append(tint)
                mat.layer_opacities.append(layers[i].opacity)
            else:
                mat.layer_tints.append((1.0, 1.0, 1.0))
                mat.layer_opacities.append(1.0)

        # Per-layer UV transforms (Starfield Summary.LayerN.UVStream).
        # Stored as flat lists padded to MAX_LAYERS=6 in the bind step.
        mat._layer_uv_channels = [
            int(layers[i].uv_channel) if i < len(layers) else 0
            for i in range(mat.layer_count)
        ]
        mat._layer_uv_scales = [
            tuple(layers[i].uv_scale) if i < len(layers) else (1.0, 1.0)
            for i in range(mat.layer_count)
        ]
        mat._layer_uv_offsets = [
            tuple(layers[i].uv_offset) if i < len(layers) else (0.0, 0.0)
            for i in range(mat.layer_count)
        ]

        # Blender data
        for i in range(min(len(blenders), mat.layer_count - 1)):
            b = blenders[i]
            mat.blend_modes.append(b.mode)
            mat.blend_vc_channels.append(b.vertex_color_channel)
            mat.blend_height_thresholds.append(b.height_blend_threshold)
            mat.blend_height_factors.append(b.height_blend_factor)
            mask_tex = _resolve_and_load(b.mask_texture) if b.mask_texture else None
            mat.blend_masks.append(mask_tex)

        # Per-blender channel gates + MaskIntensity.
        # Mirrors tools/sf_render_test.py:_parse_summary_blender (line 450+)
        # so only the channels the .mat actually enables get mixed in. For
        # tombstone the detail blender has only BlendTextureNormal=True.
        mat._blend_chan_albedo = [b.blend_albedo for b in blenders]
        mat._blend_chan_normal = [b.blend_normal for b in blenders]
        mat._blend_chan_metal = [b.blend_metal for b in blenders]
        mat._blend_chan_rough = [b.blend_rough for b in blenders]
        mat._blend_chan_ao = [b.blend_ao for b in blenders]
        mat._blend_mask_intensity = [b.mask_intensity for b in blenders]

    def _load_basic_textures(self, ctx, mat, tex_paths, texture_dirs, ba2_mgr):
        """Load textures when no .mat data is available."""
        def _resolve_and_load(tex_path_str):
            if not tex_path_str:
                return None
            resolved = _resolve_texture_path(tex_path_str, texture_dirs, ba2_mgr)
            if resolved is None:
                return None
            if isinstance(resolved, bytes):
                cache_key = f"ba2:{tex_path_str.lower().replace(chr(92), '/')}"
                return _cached_load(ctx, resolved, cache_key=cache_key)
            return _cached_load(ctx, resolved)

        if tex_paths.get("diffuse"):
            mat.diffuse_tex = _resolve_and_load(tex_paths["diffuse"])
        if tex_paths.get("normal"):
            mat.normal_tex = _resolve_and_load(tex_paths["normal"])
        mat._roughness_tex = _resolve_and_load(tex_paths.get("specular", ""))
        mat._metallic_tex = _resolve_and_load(tex_paths.get("metallic", ""))
        mat._ao_tex = _resolve_and_load(tex_paths.get("ao", ""))

    # MAX_LAYERS / MAX_TEXTURES match the new shader (starfield_default.frag),
    # which is a direct port of tools/sf_render_test.py FRAG_SRC.
    _MAX_LAYERS = 3
    _MAX_TEXTURES = 32

    def bind_textures(self, program, mat: Material, default_textures: dict):
        """Bind Starfield material to the wholesale-ported unit-array shader.

        Direct port of tools/sf_render_test.py SFScene._draw_mesh
        (lines 1668-1798). Allocates 2D textures to consecutive units 0..31,
        publishes the per-layer indirection arrays (uLayerAlbedoUnit[] etc.)
        plus uTextures[] sampler array binding, and binds the BRDF LUT and
        env cubes to fixed high units.
        """
        import numpy as _np
        MAX_LAYERS = self._MAX_LAYERS
        MAX_TEX = self._MAX_TEXTURES

        # ---- Build the unified per-layer texture list (index 0 = base) ----
        # The existing build_material path stores layer 0 in the legacy
        # mat.diffuse_tex / mat.normal_tex / mat._roughness_tex / mat._metallic_tex
        # / mat._ao_tex slots, and layers 1+ in mat.layer_albedos / mat.layer_normals
        # / mat.layer_specs (where layer_specs[i] is a (rough, metal) tuple).
        # Synthesize a single list of dicts so the standalone-style allocator
        # below can iterate uniformly.
        rough0 = getattr(mat, "_roughness_tex", None)
        metal0 = getattr(mat, "_metallic_tex", None)
        ao0 = getattr(mat, "_ao_tex", None)
        layer_texs: list[dict] = [{
            "albedo":  mat.diffuse_tex,
            "normal":  mat.normal_tex,
            "rough":   rough0,
            "metal":   metal0,
            "ao":      ao0,
            "opacity": None,  # opacity is baked into diffuse upstream
        }]
        n_extra = len(mat.layer_albedos)
        for i in range(n_extra):
            albedo_i = mat.layer_albedos[i] if i < len(mat.layer_albedos) else None
            normal_i = mat.layer_normals[i] if i < len(mat.layer_normals) else None
            rough_i = None
            metal_i = None
            if i < len(mat.layer_specs) and mat.layer_specs[i]:
                spec_i = mat.layer_specs[i]
                if isinstance(spec_i, tuple) and len(spec_i) == 2:
                    rough_i, metal_i = spec_i
            layer_texs.append({
                "albedo":  albedo_i,
                "normal":  normal_i,
                "rough":   rough_i,
                "metal":   metal_i,
                "ao":      None,
                "opacity": None,
            })
        n_layers = max(1, min(len(layer_texs), MAX_LAYERS))

        # ---- take() allocator: assign 2D textures to units 0..MAX_TEX-1 ----
        assignments: list[tuple[int, "moderngl.Texture"]] = []
        unit = [0]  # boxed for closure mutation

        def take(tex):
            if tex is None or unit[0] >= MAX_TEX:
                return -1
            u = unit[0]
            assignments.append((u, tex))
            unit[0] += 1
            return u

        layer_slot: list[dict] = []
        for li in range(MAX_LAYERS):
            if li < n_layers:
                d = layer_texs[li]
                layer_slot.append({
                    "albedo":  take(d.get("albedo")),
                    "normal":  take(d.get("normal")),
                    "rough":   take(d.get("rough")),
                    "metal":   take(d.get("metal")),
                    "ao":      take(d.get("ao")),
                    "opacity": take(d.get("opacity")),
                })
            else:
                layer_slot.append({
                    "albedo": -1, "normal": -1, "rough": -1,
                    "metal": -1, "ao": -1, "opacity": -1,
                })

        # ---- Blender masks ----
        blender_units: list[int] = []
        for mask in (mat.blend_masks or [])[:MAX_LAYERS]:
            blender_units.append(take(mask))
        while len(blender_units) < MAX_LAYERS:
            blender_units.append(-1)

        # ---- Bind every allocated 2D texture to its unit ----
        for u, tex in assignments:
            tex.use(u)

        # uTextures[] is a sampler2D[MAX_TEX_UNITS]; declare each sampler i
        # uses unit i. moderngl handles arrays via tuple-of-ints.
        if "uTextures" in program:
            program["uTextures"].value = tuple(range(MAX_TEX))

        # ---- BRDF LUT + env cubes (units 32+) ----
        LUT_UNIT = 32
        SPEC_CUBE_UNIT = 33
        IRR_CUBE_UNIT = 34
        if self.brdf_lut is not None:
            self.brdf_lut.use(LUT_UNIT)
            if "uBrdfLUT" in program:
                program["uBrdfLUT"].value = LUT_UNIT
        if self.specular_cube is not None:
            self.specular_cube.use(SPEC_CUBE_UNIT)
            if "uEnvSpec" in program:
                program["uEnvSpec"].value = SPEC_CUBE_UNIT
        if self.irradiance_cube is not None:
            self.irradiance_cube.use(IRR_CUBE_UNIT)
            if "uEnvIrradiance" in program:
                program["uEnvIrradiance"].value = IRR_CUBE_UNIT

        # ---- Per-layer arrays padded to MAX_LAYERS ----
        def set_ints(name: str, vals: list[int]):
            if name in program:
                program[name].value = tuple(int(v) for v in vals)

        def set_floats(name: str, vals: list[float]):
            if name in program:
                program[name].value = tuple(float(v) for v in vals)

        def set_vec2_array(name: str, flat_vals: list[float]):
            # flat_vals has length 2 * MAX_LAYERS; moderngl needs a tuple of
            # 2-tuples for a vec2[] uniform.
            if name in program:
                program[name].value = tuple(
                    (float(flat_vals[i * 2]), float(flat_vals[i * 2 + 1]))
                    for i in range(len(flat_vals) // 2)
                )

        def set_vec3_array(name: str, flat_vals: list[float]):
            # flat_vals has length 3 * MAX_LAYERS; moderngl needs a tuple of
            # 3-tuples for a vec3[] uniform.
            if name in program:
                program[name].value = tuple(
                    (float(flat_vals[i * 3]),
                     float(flat_vals[i * 3 + 1]),
                     float(flat_vals[i * 3 + 2]))
                    for i in range(len(flat_vals) // 3)
                )

        albedo_units  = [layer_slot[i]["albedo"]  for i in range(MAX_LAYERS)]
        normal_units  = [layer_slot[i]["normal"]  for i in range(MAX_LAYERS)]
        rough_units   = [layer_slot[i]["rough"]   for i in range(MAX_LAYERS)]
        metal_units   = [layer_slot[i]["metal"]   for i in range(MAX_LAYERS)]
        ao_units      = [layer_slot[i]["ao"]      for i in range(MAX_LAYERS)]
        opacity_units = [layer_slot[i]["opacity"] for i in range(MAX_LAYERS)]

        layer_uv_channels = getattr(mat, "_layer_uv_channels", []) or []
        layer_uv_scales = getattr(mat, "_layer_uv_scales", []) or []
        layer_uv_offsets = getattr(mat, "_layer_uv_offsets", []) or []
        layer_normal_scales = getattr(mat, "_layer_normal_scales", []) or []
        layer_tints = mat.layer_tints or []

        uv_scales: list[float] = []
        uv_offsets: list[float] = []
        uv_channels: list[int] = []
        tints: list[float] = []
        normal_scales: list[float] = []
        for i in range(MAX_LAYERS):
            if i < len(layer_uv_scales):
                sx, sy = layer_uv_scales[i]
            else:
                sx, sy = 1.0, 1.0
            if i < len(layer_uv_offsets):
                ox, oy = layer_uv_offsets[i]
            else:
                ox, oy = 0.0, 0.0
            uv_scales.extend([float(sx), float(sy)])
            uv_offsets.extend([float(ox), float(oy)])
            uv_channels.append(int(layer_uv_channels[i]) if i < len(layer_uv_channels) else 0)
            if i < len(layer_tints):
                t = layer_tints[i]
                tints.extend([float(t[0]), float(t[1]), float(t[2])])
            else:
                tints.extend([1.0, 1.0, 1.0])
            normal_scales.append(
                float(layer_normal_scales[i]) if i < len(layer_normal_scales) else 1.0
            )

        set_ints("uLayerAlbedoUnit",  albedo_units)
        set_ints("uLayerNormalUnit",  normal_units)
        set_ints("uLayerRoughUnit",   rough_units)
        set_ints("uLayerMetalUnit",   metal_units)
        set_ints("uLayerAoUnit",      ao_units)
        set_ints("uLayerOpacityUnit", opacity_units)
        set_vec2_array("uLayerUvScale",  uv_scales)
        set_vec2_array("uLayerUvOffset", uv_offsets)
        set_ints("uLayerUvChannel",   uv_channels)
        set_vec3_array("uLayerTint",  tints)
        set_floats("uLayerNormalScale", normal_scales)

        if "uNumLayers" in program:
            program["uNumLayers"].value = n_layers

        # ---- Alpha / opacity flags ----
        is_decal = bool(getattr(mat, "_is_decal", False))
        is_blended = (mat.alpha_flags & 8) != 0
        has_opacity = is_blended or is_decal
        if "uHasOpacity" in program:
            program["uHasOpacity"].value = bool(has_opacity)
        if "uAlphaTest" in program:
            # Alpha-test mode: discard < threshold. Decals already use blending,
            # not discard, so we only enable test for non-decal opacity meshes.
            program["uAlphaTest"].value = bool((mat.alpha_flags & 1) != 0 and not is_decal)
        if "uAlphaThreshold" in program:
            program["uAlphaThreshold"].value = float(mat.alpha_threshold or 0.0)

        # ---- Blender uniforms padded to MAX_LAYERS ----
        # Mode mapping mirrors tools/sf_render_test.py:_BLEND_MODE_INT
        _MODE_MAP = {
            "linear": 0, "lerp": 0,
            "additive": 1, "add": 1,
            "position_contrast": 2, "positioncontrast": 2,
            "none": 3,
            "multiply": 0,  # standalone has no entry, fall back to lerp
            "screen": 0,
        }
        _VC_MAP = {
            None: -1, "": -1, "none": -1,
            "r": 0, "red": 0,
            "g": 1, "green": 1,
            "b": 2, "blue": 2,
            "a": 3, "alpha": 3,
        }

        bm_chan_alb = getattr(mat, "_blend_chan_albedo", []) or []
        bm_chan_nor = getattr(mat, "_blend_chan_normal", []) or []
        bm_chan_met = getattr(mat, "_blend_chan_metal",  []) or []
        bm_chan_rou = getattr(mat, "_blend_chan_rough",  []) or []
        bm_chan_ao  = getattr(mat, "_blend_chan_ao",     []) or []
        bm_chan_add = getattr(mat, "_blend_chan_add_normal", []) or []
        bm_mask_int = getattr(mat, "_blend_mask_intensity", []) or []
        bm_modes = mat.blend_modes or []
        bm_vcs   = mat.blend_vc_channels or []

        n_blenders = len(mat.blend_masks or [])
        blend_modes_i: list[int] = []
        blend_vc_i:    list[int] = []
        blend_mi_f:    list[float] = []
        blend_alb_i:   list[int] = []
        blend_nor_i:   list[int] = []
        blend_met_i:   list[int] = []
        blend_rou_i:   list[int] = []
        blend_ao_i:    list[int] = []
        blend_add_i:   list[int] = []
        for i in range(MAX_LAYERS):
            if i < n_blenders:
                mode = bm_modes[i] if i < len(bm_modes) else "linear"
                blend_modes_i.append(_MODE_MAP.get(str(mode).lower().replace(" ", ""), 0))
                vc = bm_vcs[i] if i < len(bm_vcs) else None
                vc_str = vc.lower() if isinstance(vc, str) else vc
                blend_vc_i.append(_VC_MAP.get(vc_str, -1))
                blend_mi_f.append(float(bm_mask_int[i]) if i < len(bm_mask_int) else 1.0)
                blend_alb_i.append(1 if (i < len(bm_chan_alb) and bm_chan_alb[i]) else 0)
                # Default for blend_normal is True (per BlenderData defaults +
                # standalone _parse_summary_blender) — match it.
                if i < len(bm_chan_nor):
                    blend_nor_i.append(1 if bm_chan_nor[i] else 0)
                else:
                    blend_nor_i.append(1)
                blend_met_i.append(1 if (i < len(bm_chan_met) and bm_chan_met[i]) else 0)
                blend_rou_i.append(1 if (i < len(bm_chan_rou) and bm_chan_rou[i]) else 0)
                blend_ao_i.append (1 if (i < len(bm_chan_ao)  and bm_chan_ao[i])  else 0)
                blend_add_i.append(1 if (i < len(bm_chan_add) and bm_chan_add[i]) else 0)
            else:
                blend_modes_i.append(3)  # "none" — skipped by shader loop
                blend_vc_i.append(-1)
                blend_mi_f.append(1.0)
                blend_alb_i.append(0); blend_nor_i.append(0); blend_met_i.append(0)
                blend_rou_i.append(0); blend_ao_i.append(0); blend_add_i.append(0)

        if "uBlenderCount" in program:
            program["uBlenderCount"].value = int(n_blenders)
        set_ints("uBlenderMaskUnit", blender_units)
        set_ints("uBlenderMode",     blend_modes_i)
        set_ints("uBlenderVcChan",   blend_vc_i)
        set_floats("uBlenderMaskInt", blend_mi_f)
        set_ints("uBlendAlbedo",     blend_alb_i)
        set_ints("uBlendNormal",     blend_nor_i)
        set_ints("uBlendMetal",      blend_met_i)
        set_ints("uBlendRough",      blend_rou_i)
        set_ints("uBlendAO",         blend_ao_i)
        set_ints("uBlendAddNormal",  blend_add_i)

        # ---- Per-frame globals (matches tools/sf_render_test.py SFScene.render
        #      lines 1596-1666 — set here per-mesh because the editor doesn't
        #      yet have a once-per-frame SF hook. These are constant across
        #      meshes within a frame so the cost is negligible). ----
        if "uLightSourceDiffuse" in program:
            li = 1.0
            program["uLightSourceDiffuse"].value = (li, li, li)
        if "uLightSourceAmbient" in program:
            amb = 0.7
            program["uLightSourceAmbient"].value = (amb, amb, amb)
        if "uToneMapScale" in program:
            program["uToneMapScale"].value = 0.1
        if "uBrightnessScale" in program:
            program["uBrightnessScale"].value = 0.1
        if "uEnvIntensity" in program:
            program["uEnvIntensity"].value = 8.0
        if "uHasCubeMap" in program:
            program["uHasCubeMap"].value = bool(self.specular_cube is not None)
        if "uHasSpecular" in program:
            program["uHasSpecular"].value = True
        if "uEnvLodBias" in program:
            program["uEnvLodBias"].value = 0.0
        if "uEnvRotation" in program:
            program["uEnvRotation"].write(_np.eye(3, dtype="f4").tobytes())
        if "u_lightDirWorld" in program:
            # Match the standalone's default light direction (sf_render_test.py
            # App class). Editor renderer can override later by writing this
            # uniform per-frame before draw.
            program["u_lightDirWorld"].value = (-0.3, -0.7, -0.6)
