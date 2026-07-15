"""FO4/Skyrim/FO76 material backend.

Handles BGSM/BGEM material files, SpecGloss texture binding,
BSLightingShaderProperty / BSEffectShaderProperty uniform setup.
"""
from __future__ import annotations
import logging
import math

import moderngl
import glm

from creation_lib.renderer.scene_renderer import Material
from creation_lib.renderer.material_readers.base import RenderFlags
from creation_lib.renderer.material_pipeline import (
    _resolve_texture_path, _cached_load, _cached_load_cubemap,
    _extract_alpha_flags, _extract_shader_params, _get_shape_property_block,
    _get_decoded,
)

_log = logging.getLogger("nif_editor.fo4_material")


class FO4MaterialBackend:
    """Material backend for FO4, Skyrim SE, and FO76."""

    def build_material(self, ctx, nif, shape_block, texture_dirs, ba2_mgr=None):
        """Build a Material from a BSTriShape's shader properties.

        """
        mat = Material()
        shape_name = shape_block.get_field("Name") or shape_block.type_name
        shader_prop = _get_shape_property_block(
            nif, shape_block, "Shader Property", "BSShaderProperty"
        )
        if not shader_prop:
            return mat

        block_type = shader_prop.type_name
        is_effect = "BSEffectShaderProperty" in block_type

        # Alpha flags
        mat.alpha_flags, mat.alpha_threshold, mat.blend_src, mat.blend_dst = \
            _extract_alpha_flags(nif, shape_block)

        if is_effect:
            self._build_effect_material(mat, shader_prop)
        else:
            self._build_lighting_material(mat, shader_prop)

        # Texture loading
        tex_paths = self._get_texture_paths(nif, shader_prop, block_type,
                                            texture_dirs, ba2_mgr)
        self._apply_material_overrides(mat, tex_paths, is_effect, shader_prop)
        self._load_textures(ctx, mat, tex_paths, texture_dirs, ba2_mgr, is_effect)

        return mat

    def get_render_flags(self, mat: Material) -> RenderFlags:
        """Derive render flags from FO4 material."""
        is_blended = (mat.alpha_flags & 8) != 0
        return RenderFlags(
            depth_write=not is_blended,
            depth_test=True,
            polygon_offset=(-1.0, -1.0) if is_blended else None,
            blend_enabled=is_blended,
            blend_src=mat.blend_src,
            blend_dst=mat.blend_dst,
            alpha_flags=mat.alpha_flags,
            alpha_threshold=mat.alpha_threshold,
            double_sided=mat.double_sided,
        )

    def set_uniforms(self, program, mat: Material, default_textures: dict):
        """Bind textures and set uniforms. Delegates to renderer's existing methods."""
        # This is handled by renderer._set_material_uniforms / _set_effect_uniforms
        # The backend just provides the Material; the renderer binds it.
        pass

    def _build_effect_material(self, mat, shader_prop):
        """Extract BSEffectShaderProperty fields into Material."""
        mat.is_effect_shader = True
        if mat.alpha_flags == 0 and mat.blend_src == 0 and mat.blend_dst == 0:
            mat.alpha_flags |= 8

        base_color = shader_prop.get_field("Base Color")
        if hasattr(base_color, "r"):
            mat.emissive_color = glm.vec4(
                float(base_color.r), float(base_color.g),
                float(base_color.b), float(getattr(base_color, "a", 1.0)))
        elif isinstance(base_color, dict):
            mat.emissive_color = glm.vec4(
                float(base_color.get("r", 1.0)), float(base_color.get("g", 1.0)),
                float(base_color.get("b", 1.0)), float(base_color.get("a", 1.0)))
        elif isinstance(base_color, (list, tuple)) and len(base_color) >= 3:
            mat.emissive_color = glm.vec4(
                float(base_color[0]), float(base_color[1]),
                float(base_color[2]), float(base_color[3]) if len(base_color) > 3 else 1.0)

        mat.emissive_mult = float(shader_prop.get_field("Base Color Scale") or 1.0)

        sf1 = shader_prop.get_field("Shader Flags 1") or []
        sf2 = shader_prop.get_field("Shader Flags 2") or []
        if isinstance(sf1, list):
            mat.use_falloff = "Use_Falloff" in sf1
            mat.greyscale_color = "GreyscaleToPalette_Color" in sf1
            mat.greyscale_alpha = "GreyscaleToPalette_Alpha" in sf1
            mat.has_rgb_falloff = ("Receive_Shadows" in sf1 or "RGB_Falloff" in sf1)
        has_effect_lighting = False
        if isinstance(sf2, list):
            mat.double_sided = "Double_Sided" in sf2
            has_effect_lighting = "Effect_Lighting" in sf2

        mat.falloff_params = glm.vec4(
            float(shader_prop.get_field("Falloff Start Angle") or 1.0),
            float(shader_prop.get_field("Falloff Stop Angle") or 0.0),
            float(shader_prop.get_field("Falloff Start Opacity") or 1.0),
            float(shader_prop.get_field("Falloff Stop Opacity") or 0.0))
        mat.falloff_depth = float(shader_prop.get_field("Soft Falloff Depth") or 1.0)

        if has_effect_lighting:
            li_raw = shader_prop.get_field("Lighting Influence") or 0
            li_val = float(li_raw)
            mat.lighting_influence = li_val / 255.0 if li_val > 1.0 else li_val

        mat.env_reflection = float(shader_prop.get_field("Environment Map Scale") or 1.0)

        uv_scale = shader_prop.get_field("UV Scale") or {}
        uv_offset = shader_prop.get_field("UV Offset") or {}
        mat.uv_scale_offset = glm.vec4(
            float(uv_scale.get("U", 1.0)) if isinstance(uv_scale, dict) else 1.0,
            float(uv_scale.get("V", 1.0)) if isinstance(uv_scale, dict) else 1.0,
            float(uv_offset.get("U", 0.0)) if isinstance(uv_offset, dict) else 0.0,
            float(uv_offset.get("V", 0.0)) if isinstance(uv_offset, dict) else 0.0)

    def _build_lighting_material(self, mat, shader_prop):
        """Extract BSLightingShaderProperty fields into Material."""
        params = _extract_shader_params(shader_prop)
        mat.spec_color = params["spec_color"]
        mat.spec_strength = params["spec_strength"]
        mat.spec_glossiness = params["spec_glossiness"]
        mat.fresnel_power = params["fresnel_power"]
        mat.uv_scale_offset = params["uv_scale_offset"]
        mat.greyscale_color = params["greyscale_color"]
        mat.palette_scale = params["palette_scale"]
        mat.has_emit = params["has_emit"]
        mat.glow_color = params["glow_color"]
        mat.glow_mult = params["glow_mult"]

        sf2 = shader_prop.get_field("Shader Flags 2") or []
        if isinstance(sf2, list):
            mat.double_sided = "Double_Sided" in sf2

    def _get_texture_paths(self, nif, shader_prop, block_type, texture_dirs, ba2_mgr):
        """Get texture paths — delegates to material_pipeline._get_texture_paths for FO4."""
        from .material_pipeline import _get_texture_paths
        return _get_texture_paths(nif, shader_prop, block_type, texture_dirs, ba2_mgr)

    def _apply_material_overrides(self, mat, tex_paths, is_effect, shader_prop):
        """Apply BGSM/BGEM overrides to Material."""
        if not is_effect:
            if tex_paths.pop("_grayscale_to_palette_color", False):
                mat.greyscale_color = True
            bgsm_palette_scale = tex_paths.pop("_palette_scale", None)
            if bgsm_palette_scale is not None:
                mat.palette_scale = float(bgsm_palette_scale)
        else:
            tex_paths.pop("_grayscale_to_palette_color", None)
            tex_paths.pop("_palette_scale", None)

        mat_data = tex_paths.pop("_material_data", None)
        if mat_data:
            mat_type = mat_data.get("type", "")
            if mat_type == "bgsm":
                if mat_data.get("pbr"):
                    mat.material_model = "metallic-roughness"
                sc = mat_data.get("spec_color")
                if sc:
                    mat.spec_color = glm.vec3(sc[0], sc[1], sc[2])
                if mat_data.get("spec_strength") is not None:
                    mat.spec_strength = float(mat_data["spec_strength"])
                if mat_data.get("glossiness") is not None:
                    mat.spec_glossiness = float(mat_data["glossiness"])
                if mat_data.get("fresnel_power") is not None:
                    mat.fresnel_power = float(mat_data["fresnel_power"])
                if mat_data.get("subsurface_enabled"):
                    mat.subsurface_enabled = True
                    sc = mat_data.get("subsurface_color")
                    if sc and isinstance(sc, (list, tuple)) and len(sc) >= 3:
                        mat.subsurface_color = glm.vec3(float(sc[0]), float(sc[1]), float(sc[2]))
                    if mat_data.get("subsurface_scale") is not None:
                        mat.subsurface_scale = float(mat_data["subsurface_scale"])
                if mat_data.get("emit_enabled"):
                    mat.has_emit = True
                    ec = mat_data.get("emittance_color")
                    if ec and isinstance(ec, (list, tuple)) and len(ec) >= 3:
                        mat.glow_color = glm.vec3(float(ec[0]), float(ec[1]), float(ec[2]))
                    if mat_data.get("emittance_mult") is not None:
                        mat.glow_mult = float(mat_data["emittance_mult"])
                    lum = mat_data.get("lum_emittance")
                    if lum is not None and float(lum) > 0.0:
                        mat.glow_mult = max(mat.glow_mult, float(lum) / 100.0)
            elif mat_type == "bgem":
                mat.is_effect_shader = True
                has_ni_alpha = (mat.alpha_flags != 0 or mat.blend_src != 0
                                or mat.blend_dst != 0)
                if not has_ni_alpha:
                    if mat_data.get("header_alpha_test"):
                        mat.alpha_flags |= 4
                        mat.alpha_threshold = int(mat_data.get("header_alpha_test_ref", 128)) / 255.0
                    header_blend = int(mat_data.get("header_blend_mode", 0))
                    if header_blend != 0:
                        mat.alpha_flags |= 8
                        mat.blend_src = int(mat_data.get("header_blend_src", 6))
                        mat.blend_dst = int(mat_data.get("header_blend_dst", 7))
                env_scale = mat_data.get("env_mapping_mask_scale")
                if env_scale is not None:
                    mat.env_map_scale = float(env_scale)
                bc = mat_data.get("base_color")
                header_alpha = float(mat_data.get("header_alpha", 1.0))
                if bc and isinstance(bc, (list, tuple)) and len(bc) >= 3:
                    mat.emissive_color = glm.vec4(
                        float(bc[0]), float(bc[1]), float(bc[2]), math.sqrt(header_alpha))
                if mat_data.get("base_color_scale") is not None:
                    mat.emissive_mult = float(mat_data["base_color_scale"])
                if mat_data.get("falloff_enabled"):
                    mat.use_falloff = True
                    mat.falloff_params = glm.vec4(
                        float(mat_data.get("falloff_start_angle", 1.0)),
                        float(mat_data.get("falloff_stop_angle", 0.0)),
                        float(mat_data.get("falloff_start_opacity", 1.0)),
                        float(mat_data.get("falloff_stop_opacity", 0.0)))
                if mat_data.get("effect_lighting_enabled"):
                    if mat_data.get("lighting_influence") is not None:
                        li = float(mat_data["lighting_influence"])
                        mat.lighting_influence = li / 255.0 if li > 1.0 else li
                if mat_data.get("env_mapping"):
                    mat.env_reflection = float(mat_data.get("env_mapping_mask_scale", 1.0))
                if mat_data.get("grayscale_to_palette_alpha"):
                    mat.greyscale_alpha = True
                if mat_data.get("grayscale_to_palette_color"):
                    mat.greyscale_color = True
                if mat_data.get("falloff_color_enabled"):
                    mat.has_rgb_falloff = True

        if not mat_data:
            raw_name = shader_prop.get_field("Name") or ""
            if isinstance(raw_name, list):
                raw_name = "".join(str(c) for c in raw_name)
            if raw_name.lower().rstrip("\x00").endswith(".bgem"):
                mat.is_effect_shader = True
                mat.alpha_flags |= 8

    def _load_textures(self, ctx, mat, tex_paths, texture_dirs, ba2_mgr, is_effect):
        """Load textures for FO4 material."""
        from .material_pipeline import _resolve_texture_path

        def _resolve_and_load(tex_path_str):
            resolved = _resolve_texture_path(tex_path_str, texture_dirs, ba2_mgr)
            if resolved is None:
                return None
            if isinstance(resolved, bytes):
                cache_key = f"ba2:{tex_path_str.lower().replace(chr(92), '/')}"
                return _cached_load(ctx, resolved, cache_key=cache_key)
            return _cached_load(ctx, resolved)

        # Diffuse
        if tex_paths.get("diffuse"):
            diffuse_path = tex_paths["diffuse"]
            if not diffuse_path.lower().endswith("_n.dds"):
                tex = _resolve_and_load(diffuse_path)
                if tex:
                    mat.diffuse_tex = tex

        # Normal
        if tex_paths.get("normal"):
            tex = _resolve_and_load(tex_paths["normal"])
            if tex:
                mat.normal_tex = tex

        # Specular (FO4 uses combined smooth_spec)
        if tex_paths.get("specular"):
            tex = _resolve_and_load(tex_paths["specular"])
            if tex:
                mat.spec_tex = tex

        # Glow
        if tex_paths.get("glow"):
            tex = _resolve_and_load(tex_paths["glow"])
            if tex:
                mat.glow_tex = tex
                mat.has_glow_map = True
            else:
                mat.glow_color = glm.vec3(0.0)
                mat.has_emit = False

        # FO76 PBR: LightingTexture (_l) is packed gloss/AO/scattering data.
        if tex_paths.get("lighting"):
            tex = _resolve_and_load(tex_paths["lighting"])
            if tex:
                mat.textures["lighting"] = tex
                mat.lighting_has_emissive_alpha = self._texture_has_emissive_alpha(
                    tex_paths["lighting"], texture_dirs, ba2_mgr
                )

        # FO76 PBR: SpecularTexture (_r) is reflectance at normal incidence.
        if tex_paths.get("reflectivity"):
            tex = _resolve_and_load(tex_paths["reflectivity"])
            if tex:
                mat.textures["reflectivity"] = tex

        # Greyscale
        load_greyscale = (mat.greyscale_color or mat.greyscale_alpha or is_effect)
        if load_greyscale and tex_paths.get("greyscale"):
            tex = _resolve_and_load(tex_paths["greyscale"])
            if tex:
                tex.repeat_x = False
                tex.repeat_y = False
                tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
                mat.greyscale_tex = tex

        # Cubemap
        if tex_paths.get("cubemap"):
            resolved = _resolve_texture_path(tex_paths["cubemap"], texture_dirs, ba2_mgr)
            if resolved is not None:
                normalized = tex_paths["cubemap"].lower().replace(chr(92), "/")
                if isinstance(resolved, bytes):
                    tex = _cached_load_cubemap(
                        ctx,
                        resolved,
                        cache_key=f"ba2:cube:{normalized}",
                        decode_key=f"ba2:{normalized}",
                    )
                    if tex:
                        mat.env_tex = tex
                        mat.has_env_map = True
                else:
                    mat.env_tex = _cached_load_cubemap(ctx, resolved)
                    mat.has_env_map = True

        # Env mask
        if tex_paths.get("envmask"):
            tex = _resolve_and_load(tex_paths["envmask"])
            if tex:
                mat.env_mask_tex = tex
                mat.has_env_mask = True

        # Effect shader source texture flag
        if mat.is_effect_shader:
            mat.has_source_texture = mat.diffuse_tex is not None

    def _texture_has_emissive_alpha(self, tex_path_str, texture_dirs, ba2_mgr) -> bool:
        decoded = _get_decoded(tex_path_str, texture_dirs, ba2_mgr)
        if decoded is None or decoded.components < 4:
            return False
        alpha = decoded.data[3::4]
        return bool(alpha) and min(alpha) < 250
