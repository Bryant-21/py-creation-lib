"""Starfield scene backend.

Owns the wholesale-port SFScene
that lives in ``creation_lib.renderer.sf_engine`` and the entire bypass code path
that previously sat inline inside ``SceneRenderer.render()`` as an
``if game_id == "starfield"`` early-return branch.

The wholesale port exists because Starfield's PBR shader, layered
material model, and cubemap pipeline don't fit the FO4 shader's
uniforms. This backend preserves that architecture but moves it behind
the ``SceneBackend`` interface so it doesn't have to be a silent ``return``
branch in the host renderer.

The "attached NIF" interim fix is preserved as-is — it walks
``renderer.scene_root.children`` for ``AttachmentNode`` subtrees and
runs them through an internal ``Fo4Backend`` instance with the FO4-style
``programs["starfield"]`` shader. This is band-aid quality and should be
replaced by ``attach_nif`` once that's implemented (see
``project_sf_attach_decal_transparency.md`` in memory).
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
    """Starfield wholesale-port draw backend.

    Wraps ``ui.editor.sf_engine.SFScene`` (the actual draw loop) and
    re-exposes it through the ``SceneBackend`` protocol. Holds an
    internal ``Fo4Backend`` for the attached-NIF interim fix.
    """

    def __init__(self, renderer: "SceneRenderer") -> None:
        self._r = renderer
        self.sf_scene: Any = None  # ui.editor.sf_engine.SFScene | None
        self._sf_render_logged: bool = False
        # Internal Fo4Backend used by:
        #   - the attached-NIF interim pass (for AttachmentNodes that
        #     SfBackend.attach_nif couldn't load through SFScene)
        #   - the selection outline overlay (operates on the FO4-shaped
        #     SceneNode the user clicked in the tree)
        # Shares the host renderer's GL state via the same back-ref, so
        # there's no duplicated state.
        self._fo4_for_attach = Fo4Backend(renderer)

    # ----- Lifecycle (SF-specific entrypoints used by app.py) -----------

    def load_sf_scene(self, nif_path: Any, extracted_dir: Any,
                      exr_path: Any = None) -> None:
        """Build the parallel Starfield engine scene for a NIF.

        Destroys any previously loaded SF scene first. Mirrors the
        signature of the old ``SceneRenderer.load_sf_scene`` so the
        thin shim on the renderer stays a one-liner.
        """
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
        """Run the full SF draw + grid + attach pass + SSAO/composite.

        Returns True if anything was drawn (i.e. an SF scene was loaded).
        Returns False if there's no SF scene yet — caller falls through
        to the FO4 path.

        The attached-NIF interim fix is preserved between the grid pass
        and the SSAO pass.
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
                # Ambient: the SF shader applies *several* amplifiers in
                # sequence to whatever sf_ambient we pass it. Specifically:
                #
                #   ambient *= irradianceCube * uEnvIntensity   (×8 default)
                #   amb     = ambient * uDbgAmbientBoost        (×4.2 preset)
                #   color  *= (uDbgExposure / 4.23)             (×2.84 preset)
                #
                # The original 0.7 magnitude baseline came from
                # tools/sf_render_test.py which runs with ambientBoost=1.0
                # AND exposure=4.23 (factor 1.0). With the editor's
                # Starfield preset (ambientBoost≈4.2, exposure≈12) the
                # combined post-shader product is ~12× the standalone,
                # which bleaches the frame whenever skylight is enabled.
                #
                # Compensate by dividing the target magnitude by both
                # amplifiers so the post-shader product matches the
                # standalone calibration regardless of which preset the
                # user has loaded. Skylight=off keeps its small fixed
                # fallback (already in range).
                if getattr(lighting, "skylight", True):
                    ac = lighting.ambient_color
                    max_ch = max(float(ac.x), float(ac.y), float(ac.z), 1e-3)
                    amb_boost = float(getattr(r, "_dbg_ambientBoost", 1.0))
                    exposure = float(getattr(r, "_dbg_exposure", 4.23))
                    exposure_factor = max(1.0, exposure / 4.23)
                    combined = max(1.0, amb_boost) * exposure_factor
                    # Baseline magnitude. 0.7 matches tools/sf_render_test.py
                    # exactly but felt slightly underexposed in the editor —
                    # bumped to 1.0 (~43% brighter) so skylight reads with
                    # more presence. Tunable: lower for darker, higher for
                    # brighter. The /combined divisor keeps this preset-
                    # independent.
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

        # The standalone tools/sf_render_test.py runs without CULL_FACE,
        # and Starfield .mesh triangle winding produces invisible geometry
        # under the editor's default CCW cull. Disable culling for the
        # duration of the SF draw; restore after.
        # Shadow state pulled from the host renderer. The shadow map
        # texture and light-space matrix were populated by
        # _render_shadow_map(), which runs before the SF draw when the
        # supports_shadows capability flag is set.
        sf_shadow_enabled = bool(r._shadow_enabled)
        sf_shadow_map = r._shadow_depth_tex if sf_shadow_enabled else None
        sf_light_space_np = None
        if sf_shadow_enabled and sf_shadow_map is not None:
            lsm = r._light_space_matrix
            sf_light_space_np = _np.array(
                [[lsm[c][rr] for c in range(4)] for rr in range(4)],
                dtype=_np.float32)

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
        # SfBackend.attach_nif loads attachments natively into
        # sf_scene.meshes (above) so they render with PBR alongside the
        # main scene. AttachmentNodes whose SF load succeeded are tagged
        # with ``_sf_loaded = True`` by app.attach_nif and skipped here.
        # The fallback only runs for attachments where SF loading failed
        # (unsupported NIF, missing assets, etc.) — visual quality won't
        # match PBR but the geometry is at least visible.
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
        # These were silently broken under the old SF bypass because the
        # bypass returned early before the renderer's overlay section.
        # Run them here, before SSAO/composite, so SSAO captures them.
        # Each overlay is gated by its own app toggle/state, mirroring
        # the FO4 overlay section in SceneRenderer.render().
        # Show Vertices — uses SFScene's tiny GL_POINTS program over
        # the actual SF mesh VBOs. Skips invisible meshes.
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
        # The SF entry point is load_sf_scene() above, which takes the
        # extra extracted_dir + exr_path arguments the wholesale port
        # needs. The protocol's single-arg load_nif() is reserved for a
        # future generic entry point and is unused for SF today — calling
        # it would lose the EXR/extracted-dir context.
        raise NotImplementedError(
            "Use load_sf_scene(nif_path, extracted_dir, exr_path) instead")

    def detach_meshes(self, meshes: list) -> None:
        """Remove a previously-attached mesh list from the SF scene.

        Counterpart to ``attach_nif`` — call this from ``app.detach_nif``
        with the list returned by the original attach call. Releases the
        meshes' GL resources and removes them from ``sf_scene.meshes`` so
        they stop drawing. Silently no-ops on a stale call.
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

        ``parent`` is interpreted as a 4x4 numpy world transform (the
        connect-point world matrix the caller computed). If None,
        identity is used. Returns the list of newly added RenderMesh
        instances as the opaque NodeHandle so the caller can later flip
        their visibility or remove them.

        Returns an empty list if there's no SF scene loaded yet, or if
        the loader failed (logged at WARNING).
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

        The host renderer's _render_shadow_map binds the shadow FBO +
        sets cull_face=front, then calls this method with a numpy
        light-space matrix and the shadow_depth program. We delegate to
        SFScene.render_shadow_casters which builds per-mesh alt-VAOs
        against the shadow_depth program.
        """
        if self.sf_scene is None or shadow_prog is None:
            return
        self.sf_scene.render_shadow_casters(light_space_matrix, shadow_prog)

    # The SceneNode-flavoured introspection methods aren't meaningful
    # until the editor's tree shows SF mesh handles instead of the
    # placeholder FO4 SceneNodes. They're left as no-ops returning
    # sensible defaults so callers that probe them via getattr don't
    # crash. The "Hide mesh part" toggle works through set_visible
    # below by accepting either an int index or a RenderMesh handle.

    def iter_nodes(self) -> Iterable[NodeHandle]:
        if self.sf_scene is None:
            return iter([])
        return iter(self.sf_scene.meshes)

    def node_visible(self, h: NodeHandle) -> bool:
        mesh = self._resolve_handle(h)
        return bool(mesh.visible) if mesh is not None else False

    def set_visible(self, h: NodeHandle, v: bool) -> None:
        """Toggle visibility of a single SF mesh.

        Accepts either an integer index into ``sf_scene.meshes`` or a
        ``RenderMesh`` instance directly. Used to wire the "Hide mesh
        part" UI toggle for Starfield. Silently no-ops if there's no SF
        scene or the handle is bad — UI code shouldn't have to guard.
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
        """Draw vertex dots for every visible SF mesh.

        ``vp`` is unused — SFScene.draw_vertex_points takes raw view+proj
        numpy matrices that the host renderer composes inside
        render_full(). This method exists for protocol completeness;
        render_full() invokes the underlying scene method directly with
        the matrices it already has on hand.
        """
        # Intentional no-op in protocol form. render_full() invokes
        # self.sf_scene.draw_vertex_points(view_np, proj_np) with the
        # view/proj matrices it already computed for the main draw,
        # avoiding a redundant glm→numpy conversion.
        return None

    def draw_collision(self, vp: Any) -> None:
        """Draw the collision wireframe overlay.

        Delegates to ``SceneRenderer._render_collision_overlay`` because
        the existing collision system already walks ``scene_root`` for
        bhk* blocks — and Starfield NIFs have a parallel FO4-loaded
        scene_root with the same bhk* data. No SF-specific collision
        parser needed; the existing path Just Works once it actually
        gets called for SF (which it didn't before the lift because the
        old bypass returned early).
        """
        r = self._r
        # Reuse the renderer's collision overlay verbatim. ``vp`` is the
        # view-projection tuple the existing FO4 path also passes.
        r._render_collision_overlay(vp)

    def draw_selection_outline(self, h: NodeHandle, vp: Any, color: Any) -> None:
        """Draw a glowy outline around the selected mesh.

        Two paths:
        1. If ``h`` resolves to an SF RenderMesh, use SFScene's outline
           shader against that mesh's actual VBO/IBO — visually correct.
        2. Otherwise (the common case: user clicked an FO4-style
           SceneNode in the tree which carries a placeholder bbox mesh),
           delegate to the inner Fo4Backend so the bbox volume gets
           outlined at the right world location.
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
