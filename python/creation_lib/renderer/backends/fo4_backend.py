"""Fallout 4 scene backend.

Implements the FO4 draw helpers (``_draw_node``, ``_draw_shadow_node``,
``_setup_fo4_uniforms``) against renderer-owned state via ``self._r.*``.
``SceneRenderer`` owns the FO4 scene graph (``scene_root``), the shader programs,
and the top-level draw loop. Some ``SceneBackend`` methods (e.g. ``attach_nif``,
``draw_selection_outline``) raise ``NotImplementedError``; the host renderer
drives those paths directly.
"""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any, Iterable

from creation_lib.renderer.scene_backend import NodeHandle, RenderState

if TYPE_CHECKING:
    from creation_lib.renderer.scene_renderer import SceneRenderer


_PHASE2 = "fo4_backend: not implemented until Phase 2 (see handoff)"


class Fo4Backend:
    """FO4 draw backend.

    The back-reference to the owning ``SceneRenderer`` provides FBOs, shader
    programs, default textures, and the GL context without duplicating that state.
    """

    def __init__(self, renderer: "SceneRenderer") -> None:
        self._r = renderer
        # Per-frame FO4 draw-loop state: set by SceneRenderer.render() before
        # each draw and read by _draw_node. SceneRenderer forwards it through
        # a @property.
        #
        # scene_root stays on SceneRenderer: app.py sets it before render()
        # runs _ensure_backend(), so it could land on a backend about to be
        # swapped on a game change, and picking, animation, and bounds share it.
        self._current_effect_prog: Any = None

    # ----- Lifecycle -----------------------------------------------------

    def load_nif(self, path: Path) -> None:
        raise NotImplementedError(_PHASE2)

    def attach_nif(self, path: Path, parent: NodeHandle | None) -> NodeHandle:
        raise NotImplementedError(_PHASE2)

    def unload(self) -> None:
        # No-op: SceneRenderer owns scene_root and clears it directly.
        return None

    # ----- Per-frame draw ------------------------------------------------

    def render(self, camera: Any, lighting: Any, state: RenderState) -> None:
        raise NotImplementedError(_PHASE2)

    def render_shadow_casters(self, light_space_matrix: Any) -> None:
        raise NotImplementedError(_PHASE2)

    # ----- Scene introspection ------------------------------------------

    def iter_nodes(self) -> Iterable[NodeHandle]:
        raise NotImplementedError(_PHASE2)

    def node_visible(self, h: NodeHandle) -> bool:
        raise NotImplementedError(_PHASE2)

    def set_visible(self, h: NodeHandle, v: bool) -> None:
        raise NotImplementedError(_PHASE2)

    def node_world_transform(self, h: NodeHandle) -> Any:
        raise NotImplementedError(_PHASE2)

    def node_aabb(self, h: NodeHandle) -> Any:
        raise NotImplementedError(_PHASE2)

    def node_label(self, h: NodeHandle) -> str:
        raise NotImplementedError(_PHASE2)

    # ----- Overlays ------------------------------------------------------

    def draw_vertex_points(self, vp: Any) -> None:
        raise NotImplementedError(_PHASE2)

    def draw_collision(self, vp: Any) -> None:
        raise NotImplementedError(_PHASE2)

    def draw_selection_outline(self, h: NodeHandle, vp: Any, color: Any) -> None:
        raise NotImplementedError(_PHASE2)

    # ----- FO4-specific helpers (not on Protocol) -----------------------
    #
    # FO4-specific shader plumbing, reachable from the FO4 draw block.
    # Not part of the ``SceneBackend`` Protocol.

    def _draw_node(self, node, program, pass_type: str = "opaque",
                   use_alt_vao: bool = False) -> None:
        """Recursively draw a SceneNode and its children.

        Two-pass rendering: opaque first, then transparent (alpha blend).
        Renderer-owned state is read via ``self._r.*``.
        """
        import moderngl  # local import: avoid pulling moderngl into the
        # backends package on import (renderer.py already requires it).

        r = self._r
        if not node.visible:
            return
        if node.mesh:
            mat = node.mesh.material
            is_blended = (mat.alpha_flags & 8) != 0
            is_decal = getattr(mat, '_is_decal', False)
            # Decals participate in the transparent pass (alpha-blended over the
            # body) even when their NiAlphaProperty bit-3 isn't set — Starfield
            # .mat files signal decal-ness via DecalSettingsComponent or the
            # filename/Import heuristic instead. Treat them as transparent for
            # both pass filtering and the blend/depth-write block below.
            needs_transparent = is_blended or is_decal

            # Only draw in the appropriate pass
            if (pass_type == "opaque" and not needs_transparent) or \
               (pass_type == "transparent" and needs_transparent):

                # Effect shaders use a dedicated program (game-specific)
                if mat.is_effect_shader and not use_alt_vao:
                    draw_prog = self._current_effect_prog or r.programs.get("effect", program)
                else:
                    draw_prog = program

                # Set per-object uniforms
                model = node.world_transform
                if "u_model" in draw_prog:
                    draw_prog["u_model"].value = tuple(c for col in model for c in col)
                mvp = r._current_vp * model
                if "u_mvp" in draw_prog:
                    draw_prog["u_mvp"].value = tuple(c for col in mvp for c in col)

                # Global mesh opacity (cloth maker transparency)
                if "u_mesh_alpha" in draw_prog:
                    mesh_alpha = getattr(r.toggles, 'mesh_alpha', 1.0)
                    draw_prog["u_mesh_alpha"].value = mesh_alpha
                    if mesh_alpha < 1.0:
                        r.ctx.enable(moderngl.BLEND)
                        r.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)

                # Set material uniforms (skip for alt-VAO programs like
                # uv_checker/normals that don't use material textures)
                if not use_alt_vao:
                    if mat.is_effect_shader:
                        r._set_effect_uniforms(draw_prog, mat)
                        if r._current_lighting:
                            r._current_lighting.set_uniforms(draw_prog)
                        if "cameraPos" in draw_prog:
                            draw_prog["cameraPos"].value = r._current_camera_pos
                    elif mat.material_model == "metallic-roughness" and hasattr(mat, '_roughness_tex'):
                        # Starfield/FO76 backend handles its own texture binding
                        from creation_lib.renderer.sf_material import SFMaterialBackend
                        # Pick game-specific cubemaps; default to Starfield.
                        # IBL resources are lazily built in _ensure_game_ibl() so
                        # guard with getattr in case the backend hasn't activated yet.
                        if r._current_game_id == "fo76":
                            spec_cube = getattr(r, '_fo76_specular_cube', r.default_env)
                            irr_cube = getattr(r, '_fo76_irradiance_cube', r.default_env)
                        else:
                            spec_cube = getattr(r, '_sf_specular_cube', r.default_env)
                            irr_cube = getattr(r, '_sf_irradiance_cube', r.default_env)
                        sf_backend = SFMaterialBackend(
                            brdf_lut=getattr(r, '_sf_pbr_lut', None),
                            specular_cube=spec_cube,
                            irradiance_cube=irr_cube,
                        )
                        sf_backend.bind_textures(draw_prog, mat, {
                            "diffuse": r.default_diffuse,
                            "normal": r.default_normal,
                            "spec": r.default_spec,
                        })
                    else:
                        r._set_material_uniforms(draw_prog, mat)

                # Double-sided: disable face culling
                if mat.double_sided:
                    r.ctx.disable(moderngl.CULL_FACE)

                # Decals get polygon offset even when fully opaque — otherwise
                # coplanar decal geometry z-fights with the body underneath.
                needs_poly_offset = needs_transparent

                # Enable/disable blending per-mesh. Decals share the transparent
                # path (depth_mask off, BLEND on) so the alpha mask actually
                # composites onto the body underneath instead of writing opaque.
                if needs_transparent:
                    r.ctx.enable(moderngl.BLEND)
                    # Disable depth writes for transparent objects so objects
                    # drawn later (grid, other transparent meshes) aren't
                    # depth-blocked by alpha=0 fragments
                    r.ctx.depth_mask = False
                    # LEQUAL so coplanar effect-shader meshes (e.g. fx overlays
                    # on top of opaque geometry) pass the depth test instead of
                    # being z-fought out by previously-written opaque depths.
                    r.ctx.depth_func = '<='
                    # Decals don't carry NiAlphaProperty blend factors, so fall
                    # back to standard SRC_ALPHA/ONE_MINUS_SRC_ALPHA for them.
                    if is_blended and (mat.blend_src or mat.blend_dst):
                        src = r._BLEND_MAP[mat.blend_src & 0xF]
                        dst = r._BLEND_MAP[mat.blend_dst & 0xF]
                        r.ctx.blend_func = (src, dst)
                    else:
                        r.ctx.blend_func = (
                            moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA
                        )

                if needs_poly_offset:
                    # Bias decal/overlay depth toward the camera, eliminating
                    # z-fighting with coplanar opaque geometry.
                    r.ctx.enable_direct(0x8037)  # GL_POLYGON_OFFSET_FILL
                    render_layer = getattr(mat, '_render_layer', 0)
                    factor = -2.0 - float(render_layer)
                    r.ctx.polygon_offset = (factor, factor)

                if use_alt_vao or draw_prog is not program:
                    # Effect shaders / alt visualizations need a VAO
                    # bound to their program (ModernGL VAOs store their program)
                    vao = r._get_alt_vao(node.mesh, draw_prog)
                    if vao:
                        vao.render()
                else:
                    node.mesh.vao.render()

                if needs_transparent:
                    r.ctx.disable(moderngl.BLEND)
                    r.ctx.depth_mask = True
                    r.ctx.depth_func = '<'
                if needs_poly_offset:
                    r.ctx.polygon_offset = (0.0, 0.0)
                    r.ctx.disable_direct(0x8037)  # GL_POLYGON_OFFSET_FILL
                if mat.double_sided:
                    r.ctx.enable(moderngl.CULL_FACE)

        for child in node.children:
            self._draw_node(child, program, pass_type, use_alt_vao)

    def _render_selection_wireframe(self, sel_node, prog, vp,
                                    color=None) -> None:
        """Draw wireframe overlay on the selected mesh using outline color."""
        import moderngl
        r = self._r
        outline_prog = r.programs.get("outline")
        if not outline_prog:
            return
        if color is None:
            color = [0.3, 0.6, 1.0]

        vao = r._get_alt_vao(sel_node.mesh, outline_prog)
        if not vao:
            return

        model = sel_node.world_transform
        mvp = vp * model
        outline_prog["u_mvp"].value = tuple(c for col in mvp for c in col)

        # Slight extrusion along normals to prevent z-fighting
        outline_prog["u_width"].value = sel_node.bound_radius * 0.002
        cr, cg, cb = color
        outline_prog["u_color"].value = (cr, cg, cb, 0.9)

        r.ctx.wireframe = True
        r.ctx.disable(moderngl.DEPTH_TEST)
        r.ctx.disable(moderngl.CULL_FACE)
        r.ctx.enable(moderngl.BLEND)
        r.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)

        # Thicker lines
        r.ctx.line_width = 1.1

        vao.render()

        r.ctx.line_width = 1.0
        r.ctx.wireframe = False
        r.ctx.enable(moderngl.CULL_FACE)
        r.ctx.disable(moderngl.BLEND)
        r.ctx.enable(moderngl.DEPTH_TEST)

    def _render_selection_outline(self, sel_node, vp, color=None) -> None:
        """Render glowy outline via back-face extrusion (two passes for glow)."""
        import moderngl
        r = self._r
        outline_prog = r.programs.get("outline")
        if not outline_prog:
            return
        if color is None:
            color = [0.3, 0.6, 1.0]

        # Get or create alt VAO for the outline shader (needs in_position + in_normal)
        vao = r._get_alt_vao(sel_node.mesh, outline_prog)
        if not vao:
            return

        # Estimate outline width from bounding sphere
        # Clamp minimum relative to mesh size (not a fixed absolute)
        base_width = sel_node.bound_radius * 0.015

        model = sel_node.world_transform
        mvp = vp * model
        if "u_mvp" in outline_prog:
            outline_prog["u_mvp"].value = tuple(c for col in mvp for c in col)

        # No depth test (outline shows through other meshes)
        # Cull FRONT faces (only back faces visible = outline shell)
        r.ctx.disable(moderngl.DEPTH_TEST)
        r.ctx.enable(moderngl.BLEND)
        r.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
        r.ctx.front_face = "ccw"  # default
        r.ctx.cull_face = "front"
        r.ctx.enable(moderngl.CULL_FACE)

        cr, cg, cb = color

        # Pass 1: outer glow (larger extrusion, lower alpha)
        outline_prog["u_width"].value = base_width * 4.0
        outline_prog["u_color"].value = (cr * 0.7, cg * 0.7, cb * 0.7, 0.25)
        vao.render()

        # Pass 2: core outline (smaller extrusion, higher alpha)
        outline_prog["u_width"].value = base_width * 1.5
        outline_prog["u_color"].value = (cr, cg, cb, 0.85)
        vao.render()

        # Restore state
        r.ctx.cull_face = "back"
        r.ctx.disable(moderngl.BLEND)
        r.ctx.enable(moderngl.DEPTH_TEST)

    def _draw_shadow_node(self, node, prog, light_vp) -> None:
        """Draw nodes into shadow map (depth only)."""
        r = self._r
        if not node.visible:
            return
        if node.mesh:
            mat = node.mesh.material
            # Skip transparent objects
            if (mat.alpha_flags & 8) == 0:
                model = node.world_transform
                light_mvp = light_vp * model
                if "u_light_mvp" in prog:
                    prog["u_light_mvp"].value = tuple(
                        c for col in light_mvp for c in col
                    )
                vao = r._get_alt_vao(node.mesh, prog)
                if vao:
                    vao.render()
        for child in node.children:
            self._draw_shadow_node(child, prog, light_vp)

    def _setup_fo4_uniforms(self, prog, lighting, rm) -> None:
        """Set camera, debug toggle, lighting, and render-mode uniforms.

        Reads shared GL/render state from the host renderer (camera pos,
        view matrix, SSAO/shadow flags, shadow map texture).
        """
        r = self._r
        if "cameraPos" in prog:
            prog["cameraPos"].value = r._current_camera_pos

        _toggle_map = {
            "toggle_diffuse":     r.toggles.diffuse,
            "toggle_normal":      r.toggles.normal,
            "toggle_spec":        r.toggles.specular,
            "toggle_lighting":    r.toggles.lighting,
            "toggle_vertexColor": r.toggles.vertex_colors,
            "toggle_envMap":      r.toggles.env_map,
        }
        for name, val in _toggle_map.items():
            if name in prog:
                prog[name].value = 1.0 if val else 0.0

        if rm:
            rm.apply_shader_uniforms(prog)
        if lighting:
            lighting.set_uniforms(prog)

        # TBR debug tuning uniforms (stored directly on renderer by toolbar)
        for name, attr, default in [
            ("dbg_envBoost", "_dbg_envBoost", 1.0),
            ("dbg_metalF0", "_dbg_metalF0", 0.9),
            ("dbg_diffuseBleed", "_dbg_diffuseBleed", 0.0),
            ("dbg_exposure", "_dbg_exposure", 4.23),
            ("dbg_specBoost", "_dbg_specBoost", 1.0),
            ("dbg_ambientBoost", "_dbg_ambientBoost", 1.0),
        ]:
            if name in prog:
                prog[name].value = getattr(r, attr, default)

        # View matrix for SSAO view-space normals
        if "u_view" in prog:
            prog["u_view"].value = tuple(c for col in r._current_view for c in col)

        # MRT flag
        if "mrtEnabled" in prog:
            prog["mrtEnabled"].value = 1.0 if r._ssao_enabled else 0.0

        # Normal scale (always 1.0 for non-Starfield; Starfield sets per-material)
        if "normalScale" in prog:
            prog["normalScale"].value = 1.0

        # Shadow uniforms
        if "shadowEnabled" in prog:
            prog["shadowEnabled"].value = 1.0 if r._shadow_enabled else 0.0
        if r._shadow_enabled and r._shadow_depth_tex:
            r._shadow_depth_tex.use(6)
            if "shadowMap" in prog:
                prog["shadowMap"].value = 6
            if "lightSpaceMatrix" in prog:
                prog["lightSpaceMatrix"].value = tuple(
                    c for col in r._light_space_matrix for c in col
                )

    # ----- Capability flags ---------------------------------------------

    @property
    def supports_ssao(self) -> bool:
        return True

    @property
    def supports_shadows(self) -> bool:
        return True

    @property
    def supports_collision_overlay(self) -> bool:
        return True
