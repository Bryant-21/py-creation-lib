"""Starfield scene backend.

Owns the ``SFScene`` from ``creation_lib.renderer.sf_engine``, a separate draw
path because Starfield's PBR shader, layered material model, and cubemap pipeline
don't fit the FO4 shader's uniforms.

``AttachmentNode`` subtrees under ``renderer.scene_root`` that SFScene failed to
load fall back to an internal ``Fo4Backend`` with the FO4-style
``programs["starfield"]`` shader: visible, but without PBR.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import TYPE_CHECKING, Any, Iterable

from creation_lib.renderer.scene_backend import NodeHandle, RenderState
from creation_lib.renderer.backends.fo4_backend import Fo4Backend

if TYPE_CHECKING:
    from creation_lib.renderer.scene_renderer import SceneRenderer


_log = logging.getLogger("nif_editor.sf_backend")
_PHASE4 = "sf_backend: not implemented until Phase 4 (see handoff)"


class SfBackend:
    """Starfield draw backend.

    Exposes ``sf_engine.SFScene`` (the draw loop) through the ``SceneBackend``
    protocol.
    """

    def __init__(self, renderer: "SceneRenderer") -> None:
        self._r = renderer
        self.sf_scene: Any = None  # sf_engine.SFScene | None
        self._sf_render_logged: bool = False
        # Draws AttachmentNodes SfBackend.attach_nif couldn't load through
        # SFScene, and the selection outline for the FO4-shaped SceneNode
        # clicked in the tree. Shares the host renderer's GL state.
        self._fo4_for_attach = Fo4Backend(renderer)

    # ----- Lifecycle (SF-specific entrypoints used by app.py) -----------

    def load_sf_scene(self, nif_path: Any, extracted_dir: Any,
                      exr_path: Any = None) -> None:
        """Build the Starfield engine scene for a NIF, replacing any loaded one."""
        self.unload_sf_scene()
        self._sf_render_logged = False
        try:
            from creation_lib.renderer.sf_engine import SFScene
            self.sf_scene = SFScene(
                self._r.ctx,
                Path(nif_path),
                Path(extracted_dir),
                Path(exr_path) if exr_path else None,
            )
            _log.info(
                "SF engine scene loaded: %d meshes from %s",
                len(self.sf_scene.meshes), nif_path,
            )
        except Exception:
            _log.exception("Failed to build SF engine scene for %s", nif_path)
            self.sf_scene = None

    def unload_sf_scene(self) -> None:
        """Release the parallel Starfield engine scene."""
        if self.sf_scene is not None:
            try:
                self.sf_scene.release()
            except Exception:
                _log.exception("sf_engine scene release failed")
        self.sf_scene = None

    def has_scene(self) -> bool:
        return self.sf_scene is not None

    # ----- Draw entrypoint called by SceneRenderer.render() -------------

    def render_full(self, camera: Any, lighting: Any, view: Any, proj: Any,
                    vp: Any, mode: Any, rm: Any) -> bool:
        """Run the full SF draw, grid, attachment fallback, overlays, and SSAO/composite.

        Returns False when no SF scene is loaded, so the caller falls through
        to the FO4 path.
        """
        import moderngl
        import numpy as _np

        r = self._r
        if self.sf_scene is None:
            return False

        sf_scene = self.sf_scene

        # PyGLM's `bytes(mat4)` disagrees with explicit element indexing
        # for rotations — use element-wise extraction so we unambiguously
        # produce a row-major math matrix (translation in last column).
        # sf_engine.render() will .T it for the GL column-major upload.
        view_np = _np.array(
            [[view[c][rr] for c in range(4)] for rr in range(4)],
            dtype=_np.float32)
        proj_np = _np.array(
            [[proj[c][rr] for c in range(4)] for rr in range(4)],
            dtype=_np.float32)

        # Light direction in world space: pulled from LightingSetup so
        # the Scene-menu light-position controls drive SF rendering. Fall
        # back to NifSkope's default if no lighting object is available.
        light_dir = _np.array([-0.3, -0.7, -0.6], dtype=_np.float32)
        sf_light_col = None
        sf_ambient = (0.7, 0.7, 0.7)
        sf_fill_dir_view = (0.0, 0.0, 0.0)
        sf_fill_col = (0.0, 0.0, 0.0)
        sf_mirror_dir_view = (0.0, 0.0, 0.0)
        sf_mirror_col = (0.0, 0.0, 0.0)
        sf_mirror_enabled = False
        if lighting is not None:
            try:
                # Key light direction (world), color (modulated by intensity).
                kd = lighting.key_dir
                light_dir = _np.array(
                    [float(kd.x), float(kd.y), float(kd.z)],
                    dtype=_np.float32)
                kc = lighting.key_color
                ki = float(lighting.key_intensity)
                sf_light_col = (float(kc.x) * ki, float(kc.y) * ki, float(kc.z) * ki)
                # The SF shader amplifies sf_ambient in sequence:
                #
                #   ambient *= irradianceCube * uEnvIntensity   (×8 default)
                #   amb     = ambient * uDbgAmbientBoost        (×4.2 preset)
                #   color  *= (uDbgExposure / 4.23)             (×2.84 preset)
                #
                # tools/sf_render_test.py is calibrated at ambientBoost=1.0 and
                # exposure=4.23. The editor's Starfield preset (ambientBoost≈4.2,
                # exposure≈12) makes that ~12× brighter and bleaches the frame
                # with skylight on, so the target magnitude is divided by both
                # amplifiers. Skylight off keeps a small fixed fallback.
                if getattr(lighting, "skylight", True):
                    ac = lighting.ambient_color
                    max_ch = max(float(ac.x), float(ac.y), float(ac.z), 1e-3)
                    amb_boost = float(getattr(r, "_dbg_ambientBoost", 1.0))
                    exposure = float(getattr(r, "_dbg_exposure", 4.23))
                    exposure_factor = max(1.0, exposure / 4.23)
                    combined = max(1.0, amb_boost) * exposure_factor
                    # Baseline 1.0 rather than sf_render_test.py's 0.7, which
                    # looks underexposed in the editor. Tunable; the /combined
                    # divisor keeps it preset-independent.
                    target_mag = 1.0 / combined
                    k = target_mag / max_ch
                    sf_ambient = (float(ac.x) * k, float(ac.y) * k, float(ac.z) * k)
                else:
                    sf_ambient = (0.05, 0.05, 0.05)
                # Fill light: matches FO4 shader — fill color is zero when
                # skylight is disabled.
                fd = lighting.fill_dir
                fd_world = _np.array(
                    [float(fd.x), float(fd.y), float(fd.z)],
                    dtype=_np.float32)
                fd_view = view_np[:3, :3] @ fd_world
                sf_fill_dir_view = (float(fd_view[0]), float(fd_view[1]), float(fd_view[2]))
                if getattr(lighting, "skylight", True):
                    fc = lighting.fill_color
                    sf_fill_col = (float(fc.x), float(fc.y), float(fc.z))
                # Mirror light: same color as the key light, reflected side.
                sf_mirror_enabled = bool(getattr(lighting, "mirror_light", False))
                if sf_mirror_enabled:
                    md = lighting.mirror_dir
                    md_world = _np.array(
                        [float(md.x), float(md.y), float(md.z)],
                        dtype=_np.float32)
                    md_view = view_np[:3, :3] @ md_world
                    sf_mirror_dir_view = (float(md_view[0]), float(md_view[1]), float(md_view[2]))
                    sf_mirror_col = sf_light_col
            except Exception:
                _log.exception("SF lighting state extraction failed — using defaults")

        # Scene-menu toggles + debug sliders pulled from the App instance.
        # Mirror the FO4 path exactly so presets and per-control checkboxes
        # behave identically across renderers.
        t = r.toggles
        sf_toggles: dict = {
            "diffuse":     t.diffuse,
            "normal":      t.normal,
            "spec":        t.specular,
            "lighting":    t.lighting,
            "vertexColor": t.vertex_colors,
            "envMap":      t.env_map,
        }
        sf_dbg: dict = {
            "envBoost":     getattr(r, "_dbg_envBoost", 1.0),
            "metalF0":      getattr(r, "_dbg_metalF0", 0.9),
            "diffuseBleed": getattr(r, "_dbg_diffuseBleed", 0.0),
            "exposure":     getattr(r, "_dbg_exposure", 4.23),
            "specBoost":    getattr(r, "_dbg_specBoost", 1.0),
            "ambientBoost": getattr(r, "_dbg_ambientBoost", 1.0),
        }

        # One-shot diagnostics the first time we render an SF scene so we
        # can spot frustum/coordinate mismatches before they become a
        # grey-screen mystery.
        if not self._sf_render_logged:
            _dlog = logging.getLogger("nif_editor.sf_engine")
            try:
                _dlog.info("SF render first-frame diagnostics:")
                _dlog.info("  fbo_size=%s", r._fbo_size)
                _dlog.info("  num_meshes=%d", len(sf_scene.meshes))
                _dlog.info("  scene bbox_min=%s bbox_max=%s",
                           sf_scene.bbox_min.tolist(), sf_scene.bbox_max.tolist())
                _dlog.info("  view_np=\n%s", view_np)
                _dlog.info("  proj_np=\n%s", proj_np)
                _dlog.info("  light_dir_world=%s", light_dir.tolist())
                _dlog.info("  camera eye=%s", tuple(camera.get_eye_position()))
                if sf_scene.meshes:
                    m0 = sf_scene.meshes[0]
                    _dlog.info("  mesh[0] name=%s model=\n%s",
                               m0.name, m0.model)
            except Exception:
                _dlog.exception("SF diagnostics dump failed")
            self._sf_render_logged = True

        # Shadow map and light-space matrix come from _render_shadow_map(),
        # which runs before the SF draw when supports_shadows is set.
        sf_shadow_enabled = bool(r._shadow_enabled)
        sf_shadow_map = r._shadow_depth_tex if sf_shadow_enabled else None
        sf_light_space_np = None
        if sf_shadow_enabled and sf_shadow_map is not None:
            lsm = r._light_space_matrix
            sf_light_space_np = _np.array(
                [[lsm[c][rr] for c in range(4)] for rr in range(4)],
                dtype=_np.float32)

        # Starfield .mesh winding renders invisible under the editor's default
        # CCW cull (tools/sf_render_test.py runs without CULL_FACE), so culling
        # is off for the SF draw.
        r.ctx.disable(moderngl.CULL_FACE)
        try:
            sf_scene.render(
                view_np, proj_np, light_dir,
                ambient=sf_ambient,
                light_col=sf_light_col,
                toggles=sf_toggles,
                dbg=sf_dbg,
                fill_dir_view=sf_fill_dir_view,
                fill_col=sf_fill_col,
                mirror_dir_view=sf_mirror_dir_view,
                mirror_col=sf_mirror_col,
                mirror_enabled=sf_mirror_enabled,
                ssao_enabled=r._ssao_enabled,
                shadow_enabled=sf_shadow_enabled,
                shadow_map=sf_shadow_map,
                light_space_matrix=sf_light_space_np,
            )
        except Exception:
            _log.exception("sf_engine.render failed — falling back to blank frame")
        r.ctx.enable(moderngl.CULL_FACE)

        # Draw grid after the SF scene so it composites over opaque geometry
        if r.grid and r.grid_visible:
            vp_tuple = tuple(c for col in vp for c in col)
            r.ctx.disable(moderngl.CULL_FACE)
            r.ctx.enable(moderngl.BLEND)
            r.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
            r.grid.render(vp_tuple)
            r.ctx.disable(moderngl.BLEND)
            r.ctx.enable(moderngl.CULL_FACE)

        # --- Attached-NIF FO4 fallback pass ---------------------------------
        # SfBackend.attach_nif loads attachments into sf_scene.meshes for PBR,
        # and app.attach_nif tags those AttachmentNodes ``_sf_loaded = True``.
        # Only attachments SF failed to load (unsupported NIF, missing assets)
        # draw here: visible, but without PBR.
        if r.scene_root is not None:
            sf_attach_prog = (
                r.programs.get("starfield")
                or r.programs.get("default")
            )
            fallback_attachments = [
                c for c in r.scene_root.children
                if getattr(c, "is_attachment", False)
                and not getattr(c, "_sf_loaded", False)
            ]
            if sf_attach_prog and fallback_attachments:
                fb = self._fo4_for_attach
                fb._setup_fo4_uniforms(sf_attach_prog, lighting, rm)
                if (r._current_effect_prog
                        and r._current_effect_prog is not sf_attach_prog):
                    fb._setup_fo4_uniforms(
                        r._current_effect_prog, lighting, rm)
                for child in fallback_attachments:
                    fb._draw_node(
                        child, sf_attach_prog, "opaque", use_alt_vao=False)
                for child in fallback_attachments:
                    fb._draw_node(
                        child, sf_attach_prog, "transparent", use_alt_vao=False)

        # --- Overlays (vertex points, collision, selection outline) --------
        # Drawn before SSAO/composite so SSAO captures them. Each is gated by
        # its own app toggle/state, mirroring the FO4 overlay section in
        # SceneRenderer.render(). Show Vertices uses SFScene's GL_POINTS
        # program over the SF mesh VBOs and skips invisible meshes.
        if r.toggles.show_vertices:
            try:
                sf_scene.draw_vertex_points(view_np, proj_np)
            except Exception:
                _log.exception("SF draw_vertex_points failed — skipping")

        # Show Collisions — delegates to the renderer's existing
        # collision overlay path. Walks scene_root for bhk* blocks;
        # SF NIFs have parallel FO4-loaded collision data so this
        # works without an SF-specific collision parser.
        if r._show_collision:
            try:
                vp_tuple = tuple(c for col in vp for c in col)
                r._render_collision_overlay(vp_tuple)
            except Exception:
                _log.exception("SF collision overlay failed — skipping")

        # Selection outline — when the user has a SceneNode selected
        # in the tree, draw the FO4-style glow outline around it.
        sel_mgr = r.selection_mgr
        sel_node = sel_mgr.selected if sel_mgr is not None else None
        if sel_node is not None and getattr(sel_node, "mesh", None) is not None:
            try:
                sp = getattr(r, "settings_panel", None)
                outline_style = sp.outline_style if sp else "glow"
                outline_color = sp.outline_color if sp else [0.3, 0.6, 1.0]
                if outline_style != "none":
                    fb = self._fo4_for_attach
                    if outline_style == "wireframe":
                        sf_attach_prog2 = (
                            r.programs.get("starfield")
                            or r.programs.get("default")
                        )
                        if sf_attach_prog2 is not None:
                            fb._render_selection_wireframe(
                                sel_node, sf_attach_prog2, vp, outline_color)
                    else:
                        fb._render_selection_outline(
                            sel_node, vp, outline_color)
            except Exception:
                _log.exception("SF selection outline failed — skipping")

        # SSAO + composite for SF. The SF FRAG_SRC writes view-space
        # normals to color attachment 1 (_fbo_normal_tex) when mrtEnabled
        # is set, so the existing _render_ssao + _composite_pass path
        # works unchanged. Skip cleanly when SSAO is disabled.
        if r._ssao_enabled and r._composite_tex:
            try:
                r._render_ssao(camera)
                r._composite_pass()
            except Exception:
                _log.exception("SF SSAO/composite pass failed — skipping")

        return True

    def get_fbo_texture_id(self) -> int:
        """Return the GL texture id the viewport panel should sample.

        SF writes view-space normals to MRT 1 when SSAO is enabled, so
        the composite path works just like FO4. Falls back to the raw
        fbo_texture when SSAO is off.
        """
        r = self._r
        if r._ssao_enabled and r._composite_tex:
            return r._composite_tex.glo
        if r.fbo_texture:
            return r.fbo_texture.glo
        return 0

    # ----- Protocol implementations -------------------------------------

    def load_nif(self, path: Path) -> None:
        # The single-arg protocol load_nif() can't carry the extracted_dir and
        # exr_path that SFScene needs.
        raise NotImplementedError(
            "Use load_sf_scene(nif_path, extracted_dir, exr_path) instead")

    def detach_meshes(self, meshes: list) -> None:
        """Remove a mesh list returned by ``attach_nif`` from the SF scene.

        Releases the meshes' GL resources and drops them from
        ``sf_scene.meshes``. No-op on a stale call.
        """
        if self.sf_scene is None or not meshes:
            return
        ids = {id(m) for m in meshes}
        survivors = []
        removed = 0
        for mesh in self.sf_scene.meshes:
            if id(mesh) in ids:
                # Release GL resources owned by this mesh. Overlay VAOs
                # are cached on the mesh itself by SFScene's overlay
                # methods; release them too if present.
                for attr in ("vao", "vbo", "ibo", "_points_vao", "_outline_vao"):
                    obj = getattr(mesh, attr, None)
                    if obj is not None:
                        try:
                            obj.release()
                        except Exception:
                            pass
                removed += 1
            else:
                survivors.append(mesh)
        self.sf_scene.meshes = survivors
        if removed:
            _log.info("detach_meshes: removed %d SF meshes", removed)

    def attach_nif(self, path: Path,
                   parent: NodeHandle | None = None) -> NodeHandle:
        """Load a child NIF as an attachment into the active SF scene.

        ``parent`` is the connect point's 4x4 numpy world transform (identity
        if None). Returns the added RenderMesh list as the NodeHandle, or an
        empty list when no SF scene is loaded or the load fails (logged at
        WARNING).
        """
        import numpy as _np
        if self.sf_scene is None:
            _log.warning("attach_nif called with no SF scene loaded — "
                         "attachment ignored: %s", path)
            return []
        if parent is None:
            parent_xform = _np.eye(4, dtype=_np.float32)
        else:
            parent_xform = _np.asarray(parent, dtype=_np.float32)
        return self.sf_scene.add_attached_nif(Path(path), parent_xform)

    def unload(self) -> None:
        self.unload_sf_scene()

    def render(self, camera: Any, lighting: Any, state: RenderState) -> None:
        # Unused: the host renderer calls render_full() directly through
        # the SfBackend reference (a concrete-type method, not part of
        # the protocol shape).
        raise NotImplementedError("Use render_full() — see Phase 4 plan")

    def render_shadow_casters(self, light_space_matrix: Any,
                              shadow_prog: Any = None) -> None:
        """Render every visible SF mesh depth-only into the bound shadow FBO.

        Called by ``_render_shadow_map`` after it binds the FBO and sets
        cull_face=front. SFScene builds per-mesh alt-VAOs against ``shadow_prog``.
        """
        if self.sf_scene is None or shadow_prog is None:
            return
        self.sf_scene.render_shadow_casters(light_space_matrix, shadow_prog)

    # Introspection over SF RenderMesh handles (an int index or a RenderMesh);
    # the editor tree still shows placeholder FO4 SceneNodes.

    def iter_nodes(self) -> Iterable[NodeHandle]:
        if self.sf_scene is None:
            return iter([])
        return iter(self.sf_scene.meshes)

    def node_visible(self, h: NodeHandle) -> bool:
        mesh = self._resolve_handle(h)
        return bool(mesh.visible) if mesh is not None else False

    def set_visible(self, h: NodeHandle, v: bool) -> None:
        """Toggle one SF mesh's visibility (the Starfield "Hide mesh part" toggle).

        Accepts an index into ``sf_scene.meshes`` or a ``RenderMesh``. No-op
        with no SF scene or a bad handle.
        """
        mesh = self._resolve_handle(h)
        if mesh is not None:
            mesh.visible = bool(v)

    def node_world_transform(self, h: NodeHandle) -> Any:
        mesh = self._resolve_handle(h)
        return mesh.model if mesh is not None else None

    def node_aabb(self, h: NodeHandle) -> Any:
        # SFScene tracks scene-level bbox, not per-mesh. Per-mesh AABB
        # tracking can be added if a real consumer needs it.
        return None

    def node_label(self, h: NodeHandle) -> str:
        mesh = self._resolve_handle(h)
        return mesh.name if mesh is not None else ""

    def _resolve_handle(self, h: NodeHandle):
        """Map a NodeHandle to the underlying RenderMesh.

        Accepts ints (index into sf_scene.meshes) and RenderMesh
        instances. Returns None for unknown handles or if no SF scene
        is loaded.
        """
        if self.sf_scene is None:
            return None
        if isinstance(h, int):
            if 0 <= h < len(self.sf_scene.meshes):
                return self.sf_scene.meshes[h]
            return None
        # Duck-type: RenderMesh has a .vao attribute
        if hasattr(h, "vao") and hasattr(h, "model"):
            return h
        return None

    def draw_vertex_points(self, vp: Any) -> None:
        """Protocol no-op: render_full() calls ``sf_scene.draw_vertex_points``
        with the view/proj numpy matrices it already computed.
        """
        return None

    def draw_collision(self, vp: Any) -> None:
        """Draw the collision wireframe via ``SceneRenderer._render_collision_overlay``.

        Starfield NIFs also load an FO4-style ``scene_root`` carrying the same
        bhk* blocks, so no SF-specific collision parser is needed.
        """
        r = self._r
        # Reuse the renderer's collision overlay verbatim. ``vp`` is the
        # view-projection tuple the existing FO4 path also passes.
        r._render_collision_overlay(vp)

    def draw_selection_outline(self, h: NodeHandle, vp: Any, color: Any) -> None:
        """Draw the selection outline.

        An SF RenderMesh handle is outlined by render_full() with SFScene's
        outline shader, so this returns early. Otherwise (usually an FO4-style
        SceneNode with a placeholder bbox mesh) the inner Fo4Backend outlines
        the bbox at its world location.
        """
        mesh = self._resolve_handle(h)
        if mesh is not None and self.sf_scene is not None:
            # Native SF outline path. The vp param is unused — SFScene
            # takes view+proj numpy matrices, which render_full() passes
            # directly to a private dispatcher.
            return None  # render_full() handles the SF-mesh case directly
        # Fall back to the FO4 outline against the SceneNode bbox mesh.
        # ``h`` here is a SceneNode (FO4 selection_mgr.selected). The
        # vp arg is a glm matrix as supplied by SceneRenderer.render().
        try:
            self._fo4_for_attach._render_selection_outline(h, vp, color)
        except Exception:
            _log.exception("draw_selection_outline FO4 fallback failed")

    # ----- Capability flags ---------------------------------------------

    @property
    def supports_ssao(self) -> bool:
        return True  # SF FRAG_SRC writes MRT normals when mrtEnabled

    @property
    def supports_shadows(self) -> bool:
        # SF FRAG_SRC has calcShadow + vPosWorld varying, and
        # render_shadow_casters walks sf_scene.meshes depth-only into
        # the host renderer's shadow FBO.
        return True

    @property
    def supports_collision_overlay(self) -> bool:
        return False
