"""ModernGL 3D scene renderer.

Manages FBO, shader programs, and draw calls for the NIF viewport.
"""
from __future__ import annotations
from dataclasses import dataclass, field, replace
from importlib import resources as importlib_resources
import logging
from pathlib import Path
import moderngl
import glm
import numpy as np

from typing import TYPE_CHECKING, Any
from creation_lib.renderer.render_toggles import RenderToggles
from creation_lib.renderer.render_modes import RenderMode, RenderModeManager

if TYPE_CHECKING:
    from creation_lib.renderer.backends.fo4_backend import Fo4Backend
    from creation_lib.renderer.backends.fo76_backend import Fo76Backend
    from creation_lib.renderer.backends.sf_backend import SfBackend

_log = logging.getLogger("nif_editor.renderer")
_LEGACY_SHADER_FAMILY = {
    "oblivion": "gamebryo",
    "fo3": "gamebryo",
    "fnv": "gamebryo",
}


def _shader_family_game_id(game_id: str) -> str:
    return _LEGACY_SHADER_FAMILY.get(game_id, game_id)


def _texture_is_usable(tex) -> bool:
    if tex is None or not callable(getattr(tex, "use", None)):
        return False
    mglo = getattr(tex, "mglo", None)
    return mglo is None or callable(getattr(mglo, "use", None))


def _usable_texture(tex):
    return tex if _texture_is_usable(tex) else None


def _bind_texture(tex, unit: int) -> bool:
    tex = _usable_texture(tex)
    if tex is None:
        return False
    tex.use(unit)
    return True


def _starfield_environment_asset():
    return importlib_resources.files(__package__).joinpath(
        "assets",
        "monochrome_studio_02_1k.exr",
    )


@dataclass
class Material:
    """Material properties extracted from BSLightingShaderProperty or BSEffectShaderProperty.

    Textures are stored in a semantic dict keyed by name (diffuse, normal,
    specular, cubemap, glow, greyscale, envmask).  Legacy field-name
    access (e.g. mat.diffuse_tex) still works via @property helpers.
    """
    # Semantic texture dict — preferred access path
    textures: dict[str, moderngl.Texture | None] = field(default_factory=dict)

    # Material model: "spec-gloss" (FO4/Skyrim), "metallic-roughness" (FO76/Starfield)
    material_model: str = "spec-gloss"

    spec_color: glm.vec3 = field(default_factory=lambda: glm.vec3(1.0))
    spec_strength: float = 1.0
    glossiness: float = 0.5
    fresnel_power: float = 5.0
    uv_scale_offset: glm.vec4 = field(default_factory=lambda: glm.vec4(1, 1, 0, 0))
    alpha_flags: int = 0
    alpha_threshold: float = 0.0
    has_env_map: bool = False
    has_env_mask: bool = False
    env_map_scale: float = 1.0
    has_palette: bool = False
    palette_scale: float = 1.0

    # Metallic-roughness (FO76/Starfield)
    metallic: float = 0.0
    roughness: float = 0.5

    # Starfield multi-layer
    layer_count: int = 1
    layer_albedos: list = field(default_factory=list)   # list[moderngl.Texture | None]
    layer_normals: list = field(default_factory=list)    # list[moderngl.Texture | None]
    layer_specs: list = field(default_factory=list)      # list[moderngl.Texture | None] (metallic/roughness)
    layer_tints: list = field(default_factory=list)      # list[tuple[float,float,float]]
    layer_opacities: list = field(default_factory=list)  # list[float]
    blend_masks: list = field(default_factory=list)      # list[moderngl.Texture | None]
    blend_modes: list = field(default_factory=list)      # list[str]
    blend_vc_channels: list = field(default_factory=list)  # list[str | None]
    blend_height_thresholds: list = field(default_factory=list)  # list[float]
    blend_height_factors: list = field(default_factory=list)     # list[float]

    # Glow / emissive (BSLightingShaderProperty with Own_Emit flag)
    has_glow_map: bool = False
    has_emit: bool = False
    glow_color: glm.vec3 = field(default_factory=lambda: glm.vec3(0))
    glow_mult: float = 1.0
    lighting_has_emissive_alpha: bool = False
    subsurface_enabled: bool = False
    subsurface_color: glm.vec3 = field(default_factory=lambda: glm.vec3(1.0))
    subsurface_scale: float = 1.0

    # Effect shader properties (BSEffectShaderProperty)
    is_effect_shader: bool = False
    emissive_color: glm.vec4 = field(default_factory=lambda: glm.vec4(1, 1, 1, 1))
    emissive_mult: float = 1.0
    use_falloff: bool = False
    has_rgb_falloff: bool = False
    falloff_params: glm.vec4 = field(default_factory=lambda: glm.vec4(1, 0, 1, 0))
    falloff_depth: float = 1.0
    lighting_influence: float = 0.0
    greyscale_alpha: bool = False
    has_source_texture: bool = False
    env_reflection: float = 1.0
    double_sided: bool = False
    blend_src: int = 0  # GL blend src factor (from NiAlphaProperty)
    blend_dst: int = 0  # GL blend dst factor (from NiAlphaProperty)

    # -- Convenience texture accessors (read/write) ----------------------------

    @property
    def diffuse_tex(self): return self.textures.get("diffuse")
    @diffuse_tex.setter
    def diffuse_tex(self, v): self.textures["diffuse"] = v

    @property
    def normal_tex(self): return self.textures.get("normal")
    @normal_tex.setter
    def normal_tex(self, v): self.textures["normal"] = v

    @property
    def spec_tex(self): return self.textures.get("specular")
    @spec_tex.setter
    def spec_tex(self, v): self.textures["specular"] = v

    @property
    def env_tex(self): return self.textures.get("cubemap")
    @env_tex.setter
    def env_tex(self, v): self.textures["cubemap"] = v

    @property
    def glow_tex(self): return self.textures.get("glow")
    @glow_tex.setter
    def glow_tex(self, v): self.textures["glow"] = v

    @property
    def greyscale_tex(self): return self.textures.get("greyscale")
    @greyscale_tex.setter
    def greyscale_tex(self, v): self.textures["greyscale"] = v

    @property
    def env_mask_tex(self): return self.textures.get("envmask")
    @env_mask_tex.setter
    def env_mask_tex(self, v): self.textures["envmask"] = v

    # Backward-compat aliases for renamed fields
    @property
    def spec_glossiness(self): return self.glossiness
    @spec_glossiness.setter
    def spec_glossiness(self, v): self.glossiness = v

    @property
    def greyscale_color(self): return self.has_palette
    @greyscale_color.setter
    def greyscale_color(self, v): self.has_palette = v

    @property
    def has_normal_map(self): return self.textures.get("normal") is not None
    @has_normal_map.setter
    def has_normal_map(self, v): pass  # derived from textures dict

    @property
    def has_spec_map(self): return self.textures.get("specular") is not None
    @has_spec_map.setter
    def has_spec_map(self, v): pass  # derived from textures dict


@dataclass
class Mesh:
    """GPU mesh data (VBO/IBO/VAO)."""
    vao: moderngl.VertexArray
    vbo: moderngl.Buffer
    ibo: moderngl.Buffer
    num_indices: int
    material: Material
    vbo_format: str = ""       # e.g. "3f 3f 2f" (for recreating VAOs)
    vbo_attrs: list[str] = field(default_factory=list)  # e.g. ["in_position", "in_normal", "in_texcoord"]


@dataclass
class SceneNode:
    """Scene graph node mirroring NIF hierarchy."""
    name: str
    block_id: int
    nif_id: str = ""  # Which NIF session this node belongs to
    transform: glm.mat4 = field(default_factory=lambda: glm.mat4(1.0))
    world_transform: glm.mat4 = field(default_factory=lambda: glm.mat4(1.0))
    children: list[SceneNode] = field(default_factory=list)
    mesh: Mesh | None = None
    visible: bool = True
    collision_overlay: Any = None  # Optional CollisionOverlay from nif_loader
    is_external_geometry: bool = False  # Starfield external .mesh placeholder
    external_mesh_path: str = ""  # path to external .mesh file

    # Bounding sphere for picking (world space, computed after load)
    bound_center: glm.vec3 = field(default_factory=lambda: glm.vec3(0))
    bound_radius: float = 0.0


class SceneRenderer:
    """ModernGL renderer: FBO management, shader compilation, scene drawing."""

    def __init__(self, ctx: moderngl.Context):
        self.ctx = ctx
        self.toggles = RenderToggles()

        # Optional UI manager hooks — set by the host app after construction
        self.render_mode_mgr: RenderModeManager | None = None
        self.selection_mgr = None       # SelectionManager | None
        self.connect_points = None      # ConnectPointDisplay | None
        self.light_display = None       # LightDisplay | None
        self.settings_panel = None      # SettingsPanel | None (provides outline_style / outline_color)
        self.active_nif_session = None  # set by editor to get per-session data
        self.active_nif_registry = None  # set by editor so attached NIF particles render too
        self.fbo: moderngl.Framebuffer | None = None
        self.fbo_texture: moderngl.Texture | None = None
        self.fbo_depth: moderngl.Renderbuffer | None = None
        self._fbo_size = (0, 0)

        # Scene root: deliberately kept on SceneRenderer (not on the
        # backend). It's shared infra used by picking, selection, and
        # animation — not exclusively a draw-loop concern. Also app.py
        # writes to it before render() picks the backend, so storing it
        # on the backend would mean assignments could land on a backend
        # about to be swapped out on game change.
        self.scene_root: SceneNode | None = None

        # Shader programs
        self.programs: dict[str, moderngl.Program] = {}
        self.particle_renderer = None

        # Grid
        self.grid = None
        self.grid_visible = True

        # Default textures
        self.default_diffuse = None
        self.default_normal = None
        self.default_spec = None
        self.default_env = None
        self._has_real_default_env = False
        self._env_update_attempted = False
        self._logged_auto_env = False

        # UV checker texture (loaded from uv_checker.png)
        self.uv_checker_texture: moderngl.Texture | None = None

        # Cache for alternate VAOs (normals/uv_checker/outline shaders)
        # Key: (mesh_vao.glo, program.glo) -> moderngl.VertexArray
        self._alt_vao_cache: dict[tuple[int, int], moderngl.VertexArray] = {}

        # Current view-projection matrix (set during render)
        self._current_vp = glm.mat4(1.0)
        self._current_view = glm.mat4(1.0)
        self._current_lighting = None
        self._current_camera_pos = (0.0, 0.0, 0.0)
        # _current_effect_prog also lives on the backend now. The
        # render() body still assigns to it via the @property shim below.

        # --- SSAO state ---
        self._ssao_enabled = False
        self._ssao_fbo: moderngl.Framebuffer | None = None
        self._ssao_blur_fbo: moderngl.Framebuffer | None = None
        self._ssao_tex: moderngl.Texture | None = None
        self._ssao_blur_tex: moderngl.Texture | None = None
        self._ssao_noise_tex: moderngl.Texture | None = None
        self._ssao_kernel: list[tuple[float, float, float]] = []
        # MRT: normal attachment for the main FBO
        self._fbo_normal_tex: moderngl.Texture | None = None
        self._fbo_depth_tex: moderngl.Texture | None = None
        # Composite ping-pong texture + FBO
        self._composite_tex: moderngl.Texture | None = None
        self._composite_fbo: moderngl.Framebuffer | None = None

        # --- Shadow map state ---
        self._shadow_enabled = False
        self._shadow_fbo: moderngl.Framebuffer | None = None
        self._shadow_depth_tex: moderngl.Texture | None = None
        self._shadow_size = 2048
        self._shadow_dirty = True
        self._shadow_light_dir = glm.vec3(0)
        self._light_space_matrix = glm.mat4(1.0)

        # Collision overlay toggle
        self._show_collision = False
        self._collision_vao: moderngl.VertexArray | None = None
        self._collision_vbo: moderngl.Buffer | None = None
        self._collision_color_vbo: moderngl.Buffer | None = None
        self._collision_num_verts = 0
        self._collision_dirty = True

        # --- Scene backend ---
        # Per-game draw delegate that owns scene_root, _current_effect_prog,
        # the FO4 draw walk, the shadow walk, and the selection overlays.
        # SceneRenderer keeps FBOs, post-processing, the camera, the grid,
        # and shadow infrastructure.
        #
        # Eager construction with fo4 as the default: scene_root /
        # _current_effect_prog become available the moment SceneRenderer
        # is constructed, before any NIF is loaded. _ensure_backend() will
        # rebuild on game switch.
        self.backend: "Fo4Backend | SfBackend | None" = None
        self._backend_game_id: str | None = None
        self._ensure_backend("fo4")

    # ----- Per-game backend factory + state shims -----------------------

    def _ensure_backend(self, game_id: str) -> None:
        """Build the per-game SceneBackend if it doesn't match game_id.

        The factory is the only place that needs to know the game→backend
        mapping. Switching games swaps the entire backend and resets
        backend-owned state (scene_root, etc.) — callers are expected to
        reload the scene after a game switch.
        """
        if self.backend is not None and self._backend_game_id == game_id:
            return
        # Compile game-specific shaders lazily on first activation.
        # FO4/default are pre-compiled in init_shaders(); all others wait until needed.
        shader_game_id = _shader_family_game_id(game_id)
        if game_id not in ("fo4", "default") and game_id not in self.programs:
            from .shader_pipeline import load_composed_shader
            try:
                self.programs[game_id] = load_composed_shader(
                    self.ctx, "default", shader_game_id)
                self.programs[f"{game_id}_effect"] = load_composed_shader(
                    self.ctx, "effect", shader_game_id)
            except Exception as e:
                _log.warning("Failed to compile %s shaders: %s", game_id, e)
        # Build IBL cubes + BRDF LUTs lazily per game (heavy GPU work, skipped for FO4).
        self._ensure_game_ibl(game_id)
        # Lazy import to avoid pulling creation_lib.renderer.backends at module
        # load time (renderer.py is imported very early in editor boot).
        from creation_lib.renderer.backends import make_backend
        self.backend = make_backend(game_id, self)
        self._backend_game_id = game_id

    def ensure_game_backend(self, game_id: str) -> None:
        """Ensure per-game shaders/IBL/backend are ready before scene upload.

        Some editor flows upload meshes before the first render() call for a
        newly-detected game. If the per-game program is only compiled lazily in
        render(), the upload path can build VAOs against the FO4/default shader
        on first load and only become visible after a second load. Expose a
        public wrapper so app.py can make the target game's renderer state ready
        on the UI thread before upload_nif_to_gpu() runs.
        """
        self._ensure_backend(game_id)

    def _ensure_game_ibl(self, game_id: str) -> None:
        """Build IBL cubemaps and BRDF LUTs for game_id if not yet built.

        All of this is heavy GPU work (EXR loading, mip prefiltering,
        512x512 LUT integration).  FO4 doesn't use IBL so we skip it;
        Starfield and FO76 each get their own cubemap pair on first activation.
        """
        if game_id == "starfield" and not getattr(self, "_sf_ibl_ready", False):
            from .shader_pipeline import (
                build_environment_cubes,
                create_default_specular_cubemap,
                create_default_irradiance_cubemap,
                generate_sf_pbr_lut,
            )
            sf_cubes = None
            sf_env_asset = _starfield_environment_asset()
            if sf_env_asset.is_file():
                with importlib_resources.as_file(sf_env_asset) as sf_env_path:
                    sf_cubes = build_environment_cubes(self.ctx, sf_env_path)
            if sf_cubes is not None:
                self._sf_specular_cube, self._sf_irradiance_cube = sf_cubes
                _log.info("Starfield IBL: using GGX-prefiltered HDR cube from %s", sf_env_path)
            else:
                self._sf_specular_cube = create_default_specular_cubemap(
                    self.ctx, game_id="starfield")
                self._sf_irradiance_cube = create_default_irradiance_cubemap(
                    self.ctx, game_id="starfield")
            self._sf_pbr_lut = generate_sf_pbr_lut(self.ctx)
            self._sf_ibl_ready = True

        elif game_id == "fo76" and not getattr(self, "_fo76_ibl_ready", False):
            from .shader_pipeline import (
                create_default_specular_cubemap,
                create_default_irradiance_cubemap,
                generate_brdf_lut_epic,
            )
            self._fo76_specular_cube = create_default_specular_cubemap(
                self.ctx, game_id="fo76")
            self._fo76_irradiance_cube = create_default_irradiance_cubemap(
                self.ctx, game_id="fo76")
            self._brdf_lut = generate_brdf_lut_epic(self.ctx)
            self._fo76_ibl_ready = True

    # _current_effect_prog @property shim: forwards to the active
    # backend so the per-frame assignment in render() and any reads
    # (currently only inside Fo4Backend._draw_node) stay transparent.
    # scene_root is NOT proxied — see __init__ comment.

    @property
    def _current_effect_prog(self):
        return self.backend._current_effect_prog if self.backend is not None else None

    @_current_effect_prog.setter
    def _current_effect_prog(self, value) -> None:
        if self.backend is not None:
            self.backend._current_effect_prog = value

    def ensure_fbo(self, width: int, height: int):
        """Create/resize FBO. Debounce: skip if delta < 8px."""
        w = max(1, int(width))
        h = max(1, int(height))
        if self._fbo_size != (0, 0):
            dw = abs(w - self._fbo_size[0])
            dh = abs(h - self._fbo_size[1])
            if dw < 8 and dh < 8:
                return
        # Release old
        for tex in [self.fbo_texture, self._fbo_normal_tex, self._fbo_depth_tex,
                     self._ssao_tex, self._ssao_blur_tex, self._composite_tex]:
            if tex:
                tex.release()
        if self.fbo_depth:
            self.fbo_depth.release()
        for fbo in [self.fbo, self._ssao_fbo, self._ssao_blur_fbo, self._composite_fbo]:
            if fbo:
                fbo.release()

        # Main scene FBO with MRT: color + view-space normals
        self.fbo_texture = self.ctx.texture((w, h), 4)
        self.fbo_texture.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._fbo_normal_tex = self.ctx.texture((w, h), 4, dtype='f2')  # RGBA16F
        self._fbo_normal_tex.filter = (moderngl.NEAREST, moderngl.NEAREST)
        # Use a depth texture (not renderbuffer) so SSAO can sample it
        self._fbo_depth_tex = self.ctx.depth_texture((w, h))
        self._fbo_depth_tex.compare_func = ''  # disable comparison for sampling
        self._fbo_depth_tex.filter = (moderngl.NEAREST, moderngl.NEAREST)
        self.fbo_depth = None
        self.fbo = self.ctx.framebuffer(
            color_attachments=[self.fbo_texture, self._fbo_normal_tex],
            depth_attachment=self._fbo_depth_tex,
        )

        # SSAO FBOs (half-resolution for performance)
        ssao_w, ssao_h = max(1, w // 2), max(1, h // 2)
        self._ssao_tex = self.ctx.texture((ssao_w, ssao_h), 1, dtype='f1')
        self._ssao_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._ssao_fbo = self.ctx.framebuffer(
            color_attachments=[self._ssao_tex],
        )
        self._ssao_blur_tex = self.ctx.texture((ssao_w, ssao_h), 1, dtype='f1')
        self._ssao_blur_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._ssao_blur_fbo = self.ctx.framebuffer(
            color_attachments=[self._ssao_blur_tex],
        )

        # Composite FBO (same resolution as main, for SSAO final composite)
        self._composite_tex = self.ctx.texture((w, h), 4)
        self._composite_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._composite_fbo = self.ctx.framebuffer(
            color_attachments=[self._composite_tex],
        )

        self._fbo_size = (w, h)

    def init_shaders(self):
        """Compile all shader programs. Call after ctx is available."""
        from .shader_pipeline import (
            load_composed_shader, load_grid_shader,
            load_wireframe_shader,
            load_normals_shader, load_uv_checker_shader,
            load_connect_point_shader, load_outline_shader,
            create_default_diffuse, create_default_normal,
            create_default_spec, create_default_env,
            load_vertex_points_shader, create_uv_checker_texture,
        )
        # Load composed shaders (with #include preprocessing)
        # FO4 default shader needs HAS_PALETTE define for greyscale-to-palette
        self.programs["default"] = load_composed_shader(
            self.ctx, "default", "fo4", defines={"HAS_PALETTE": None})
        self.programs["effect"] = load_composed_shader(
            self.ctx, "effect", "fo4")
        # Backward compat aliases — render_modes.py returns "fo4"
        self.programs["fo4"] = self.programs["default"]
        self.programs["fo4_effect"] = self.programs["effect"]
        # Non-FO4 game shaders (skyrimse, fo76, starfield) compile lazily
        # in _ensure_backend() when that game is first activated.
        self.programs["grid"] = load_grid_shader(self.ctx)
        self.programs["wireframe"] = load_wireframe_shader(self.ctx)
        self.programs["normals"] = load_normals_shader(self.ctx)
        self.programs["uv_checker"] = load_uv_checker_shader(self.ctx)
        self.programs["connect_point"] = load_connect_point_shader(self.ctx)
        self.programs["outline"] = load_outline_shader(self.ctx)
        self.programs["vertex_points"] = load_vertex_points_shader(self.ctx)
        self.ctx.enable(moderngl.PROGRAM_POINT_SIZE)

        from creation_lib.renderer.particle_renderer import ParticleRenderer

        shader_dir = Path(__file__).parent / "shaders"
        particle_vert = (shader_dir / "particle.vert").read_text()
        particle_frag = (shader_dir / "particle.frag").read_text()
        self.programs["particle"] = self.ctx.program(
            vertex_shader=particle_vert,
            fragment_shader=particle_frag,
        )
        self.particle_renderer = ParticleRenderer(self.ctx, self.programs["particle"])

        self.default_diffuse = create_default_diffuse(self.ctx)
        self.default_normal = create_default_normal(self.ctx)
        self.default_spec = create_default_spec(self.ctx)
        env_tex, is_real = create_default_env(self.ctx)
        self.default_env = env_tex
        self._has_real_default_env = is_real
        self.uv_checker_texture = create_uv_checker_texture(self.ctx)

        # IBL cubes + BRDF LUTs for Starfield/FO76 are built lazily in
        # _ensure_backend() when those games are first activated.

        # SSAO / composite / shadow shaders
        self._init_post_processing_shaders()
        self._init_shadow_map()

    def _init_post_processing_shaders(self):
        """Compile SSAO, blur, and composite shaders; generate kernel + noise."""
        import random
        import math
        from .shader_pipeline import compile_program, _SHADER_DIR

        # Shared fullscreen vertex shader
        vert_src = (_SHADER_DIR / "ssao.vert").read_text()

        # Compile SSAO pass (ssao.vert + ssao.frag)
        frag_src = (_SHADER_DIR / "ssao.frag").read_text()
        self.programs["ssao"] = compile_program(
            self.ctx, "ssao", vert_src, frag_src)

        # Compile blur pass (ssao.vert + ssao_blur.frag)
        blur_frag = (_SHADER_DIR / "ssao_blur.frag").read_text()
        self.programs["ssao_blur"] = compile_program(
            self.ctx, "ssao_blur", vert_src, blur_frag)

        # Compile composite pass (ssao.vert + composite.frag)
        comp_frag = (_SHADER_DIR / "composite.frag").read_text()
        self.programs["composite"] = compile_program(
            self.ctx, "composite", vert_src, comp_frag)

        # Cached fullscreen VAOs per post-processing program
        self._fullscreen_vaos: dict[int, moderngl.VertexArray] = {}

        # Generate SSAO hemisphere kernel (16 samples)
        random.seed(42)
        self._ssao_kernel = []
        for i in range(16):
            # Random point in hemisphere
            x = random.uniform(-1, 1)
            y = random.uniform(-1, 1)
            z = random.uniform(0, 1)
            length = math.sqrt(x*x + y*y + z*z)
            if length < 0.001:
                length = 1.0
            x /= length
            y /= length
            z /= length
            # Accelerating distribution: more samples near the origin
            scale = (i + 1) / 16.0
            scale = 0.1 + scale * scale * 0.9
            self._ssao_kernel.append((x * scale, y * scale, z * scale))

        # 4x4 noise texture (random rotation vectors in tangent plane)
        noise_data = bytearray(16 * 3)
        for i in range(16):
            angle = random.uniform(0, math.pi * 2)
            noise_data[i*3] = int((math.cos(angle) * 0.5 + 0.5) * 255)
            noise_data[i*3+1] = int((math.sin(angle) * 0.5 + 0.5) * 255)
            noise_data[i*3+2] = 0
        self._ssao_noise_tex = self.ctx.texture((4, 4), 3, bytes(noise_data))
        self._ssao_noise_tex.filter = (moderngl.NEAREST, moderngl.NEAREST)
        self._ssao_noise_tex.repeat_x = True
        self._ssao_noise_tex.repeat_y = True

    def _init_shadow_map(self):
        """Create shadow map FBO and compile shadow depth shader."""
        from .shader_pipeline import compile_program, _SHADER_DIR

        vert_src = (_SHADER_DIR / "shadow_depth.vert").read_text()
        frag_src = (_SHADER_DIR / "shadow_depth.frag").read_text()
        self.programs["shadow_depth"] = compile_program(
            self.ctx, "shadow_depth", vert_src, frag_src)

        sz = self._shadow_size
        self._shadow_depth_tex = self.ctx.depth_texture((sz, sz))
        self._shadow_depth_tex.compare_func = ''
        self._shadow_depth_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._shadow_fbo = self.ctx.framebuffer(
            depth_attachment=self._shadow_depth_tex
        )

    def _get_fullscreen_vao(self, prog: moderngl.Program) -> moderngl.VertexArray:
        """Get or create a fullscreen triangle VAO for a post-processing program."""
        key = prog.glo
        if key not in self._fullscreen_vaos:
            self._fullscreen_vaos[key] = self.ctx.vertex_array(prog, [])
        return self._fullscreen_vaos[key]

    def update_default_env(self, texture_dirs, ba2_mgr=None, game_id: str = "fo4"):
        """Try loading the real game cubemap, or generate a game-appropriate procedural one."""
        # Re-generate if switching games (different procedural style)
        if self._env_update_attempted and not self._has_real_default_env:
            prev_game = getattr(self, '_env_game_id', 'fo4')
            if prev_game != game_id:
                self._env_update_attempted = False  # allow re-gen for new game
        if self._env_update_attempted or self._has_real_default_env:
            return
        self._env_update_attempted = True
        self._env_game_id = game_id
        from .shader_pipeline import create_default_env
        import logging
        _rlog = logging.getLogger("nif_editor.renderer")
        old_env = self.default_env
        old_env_is_real = self._has_real_default_env
        env_tex, is_real = create_default_env(self.ctx, texture_dirs, ba2_mgr,
                                              game_id=game_id)
        _rlog.info("DEFAULT CUBEMAP: update_default_env called — is_real=%s", is_real)
        if old_env and old_env is not env_tex and not old_env_is_real:
            from .shader_pipeline import purge_cached_texture
            purge_cached_texture(old_env)
            old_env.release()
        self.default_env = env_tex
        self._has_real_default_env = is_real
        if is_real:
            _rlog.info("DEFAULT CUBEMAP: using REAL game cubemap")
        else:
            _rlog.info("DEFAULT CUBEMAP: using PROCEDURAL %s env map", game_id)
        self._logged_auto_env = False  # reset so auto-enable logs again

    def init_grid(self, scene_radius: float = 0.0):
        from creation_lib.renderer.grid import Grid
        if scene_radius > 0:
            # Scale grid proportionally: ~10x scene radius, ~50 divisions
            size = scene_radius * 10.0
            step = size / 50.0
        else:
            size = 200.0
            step = 4.0
        self.grid = Grid(self.ctx, self.programs["grid"], size=size, step=step)

    def _set_material_uniforms(self, program, mat: Material):
        """Bind material textures and set per-material uniforms."""
        # Texture units: 0=diffuse, 1=normal, 2=spec, 3=env.
        # FO76 also uses 10=lighting (_l) and 11=reflectivity (_r) when present.
        mat_diffuse = _usable_texture(mat.diffuse_tex)
        mat_normal = _usable_texture(mat.normal_tex)
        mat_spec = _usable_texture(mat.spec_tex)
        mat_env = _usable_texture(mat.env_tex)
        mat_env_mask = _usable_texture(mat.env_mask_tex)
        mat_greyscale = _usable_texture(mat.greyscale_tex)
        mat_glow = _usable_texture(mat.glow_tex)
        tex_d = mat_diffuse or _usable_texture(self.default_diffuse)
        tex_n = mat_normal or _usable_texture(self.default_normal)
        tex_s = mat_spec or _usable_texture(self.default_spec)
        tex_l = _usable_texture(mat.textures.get("lighting"))
        tex_r = _usable_texture(mat.textures.get("reflectivity"))

        _bind_texture(tex_d, 0)
        _bind_texture(tex_n, 1)
        _bind_texture(tex_s, 2)

        if "diffuseMap" in program:
            program["diffuseMap"].value = 0
        if "normalMap" in program:
            program["normalMap"].value = 1
        if "specMap" in program:
            program["specMap"].value = 2

        # Env map 2D latlong at unit 3 (FO4 / Skyrim / FO76 all use sampler2D
        # envMap — Starfield is the only game with a cubemap, and it uses its
        # own SFMaterialBackend.bind_textures path, not this one).
        if _bind_texture(mat_env, 3):
            if "envMap" in program:
                program["envMap"].value = 3
        elif _bind_texture(self.default_env, 3):
            if "envMap" in program:
                program["envMap"].value = 3

        if _bind_texture(mat_env_mask, 4):
            if "envMaskMap" in program:
                program["envMaskMap"].value = 4

        # Greyscale-to-palette texture (unit 5)
        if _bind_texture(mat_greyscale, 5):
            if "greyscaleMap" in program:
                program["greyscaleMap"].value = 5

        # Glow map (unit 7 — unit 6 is reserved for shadow map)
        if _bind_texture(mat_glow, 7):
            if "glowMap" in program:
                program["glowMap"].value = 7

        if _bind_texture(tex_l, 10):
            if "lightingMap" in program:
                program["lightingMap"].value = 10

        if _bind_texture(tex_r, 11):
            if "reflectivityMap" in program:
                program["reflectivityMap"].value = 11

        # BRDF LUT (unit 8) — for FO76 metallic-roughness shaders
        if "brdfLUT" in program and _bind_texture(getattr(self, '_brdf_lut', None), 8):
            program["brdfLUT"].value = 8

        # Auto-enable env map for specular materials using the default cubemap.
        # In FO4's TBR, the cubemap IS the metallic color — without it metals
        # are just dark plastic. Always enable for specular materials.
        has_env = mat_env is not None and mat.has_env_map
        env_scale = mat.env_map_scale
        if not has_env and mat_spec is not None and _usable_texture(self.default_env):
            has_env = True
            env_scale = 1.0
            if not self._logged_auto_env:
                self._logged_auto_env = True
                import logging
                logging.getLogger("nif_editor.renderer").info(
                    "ENV AUTO-ENABLE: has_spec_map=True, has_env_map was False → "
                    "enabling with default cubemap (real=%s, scale=%.1f)",
                    self._has_real_default_env, env_scale)

        # Per-material scalar uniforms
        for name, val in [
            ("hasNormalMap", 1.0 if mat_normal is not None else 0.0),
            ("hasSpecularMap", 1.0 if mat_spec is not None else 0.0),
            ("hasEnvMap", 1.0 if has_env else 0.0),
            ("hasEnvMask", 1.0 if mat_env_mask is not None and mat.has_env_mask else 0.0),
            ("greyscaleColor", 1.0 if mat.greyscale_color else 0.0),
            ("paletteScale", mat.palette_scale),
            ("envMapScale", env_scale),
            ("specStrength", mat.spec_strength),
            ("specGlossiness", mat.spec_glossiness),
            ("fresnelPower", mat.fresnel_power),
            ("alphaFlags", mat.alpha_flags),
            ("alphaThreshold", mat.alpha_threshold),
            ("hasEmit", 1.0 if mat.has_emit else 0.0),
            ("hasGlowMap", 1.0 if mat_glow is not None and mat.has_glow_map else 0.0),
            ("hasLightingMap", 1.0 if tex_l else 0.0),
            ("hasLightingEmissive", 1.0 if mat.lighting_has_emissive_alpha else 0.0),
            ("hasReflectivityMap", 1.0 if tex_r else 0.0),
            ("subsurfaceEnabled", 1.0 if mat.subsurface_enabled else 0.0),
            ("subsurfaceScale", mat.subsurface_scale),
            ("glowMult", mat.glow_mult),
        ]:
            if name in program:
                program[name].value = val

        if "specColor" in program:
            program["specColor"].value = tuple(mat.spec_color)
        if "glowColor" in program:
            program["glowColor"].value = tuple(mat.glow_color)
        if "subsurfaceColor" in program:
            program["subsurfaceColor"].value = tuple(mat.subsurface_color)
        if "normalScale" in program:
            program["normalScale"].value = getattr(mat, '_normal_scale', 1.0)
        if "uvScaleOffset" in program:
            program["uvScaleOffset"].value = tuple(mat.uv_scale_offset)

        # Starfield multi-layer uniforms
        # ModernGL arrays must be set via base name with packed flat values
        if "sfLayerCount" in program:
            program["sfLayerCount"].value = mat.layer_count

            # Pack tints: 3 vec3s = 9 floats (always set all 3 slots)
            tints = []
            for i in range(3):
                t = mat.layer_tints[i] if i < len(mat.layer_tints) else (1.0, 1.0, 1.0)
                tints.extend(t)
            if "sfLayerTint" in program:
                import struct as _struct
                program["sfLayerTint"].write(_struct.pack(f"<{len(tints)}f", *tints))

            # Pack opacities: 3 floats
            opacities = []
            for i in range(3):
                o = mat.layer_opacities[i] if i < len(mat.layer_opacities) else 1.0
                opacities.append(o)
            if "sfLayerOpacity" in program:
                import struct as _struct
                program["sfLayerOpacity"].write(_struct.pack(f"<{len(opacities)}f", *opacities))

        if mat.layer_count > 1:
            _BLEND_MODE_MAP = {
                "linear": 0, "additive": 1, "position_contrast": 2,
                "multiply": 3, "screen": 4,
            }
            _VC_MAP = {None: 0, "r": 1, "g": 2, "b": 3, "a": 4}

            # Layer textures (layers 2+ interleaved: albedo,spec pairs)
            # Unit 8=layerAlbedo1, 9=layerSpec1, 10=layerAlbedo2, 11=layerSpec2
            for i in range(min(mat.layer_count - 1, 2)):
                albedo_unit = 8 + i * 2
                spec_unit = 9 + i * 2
                if i < len(mat.layer_albedos) and _bind_texture(mat.layer_albedos[i], albedo_unit):
                    name = f"layerAlbedo{i + 1}"
                    if name in program:
                        program[name].value = albedo_unit
                if i < len(mat.layer_specs) and _bind_texture(mat.layer_specs[i], spec_unit):
                    name = f"layerSpec{i + 1}"
                    if name in program:
                        program[name].value = spec_unit

            # Blend masks (units 12–13)
            for i in range(min(mat.layer_count - 1, 2)):
                mask_unit = 12 + i
                has_mask = i < len(mat.blend_masks) and _bind_texture(mat.blend_masks[i], mask_unit)
                if has_mask:
                    name = f"blendMask{i}"
                    if name in program:
                        program[name].value = mask_unit
                # Per-blender uniforms: set full arrays after the loop
            # (accumulated below, written once after loop)

            # Build blender arrays (2 elements each)
            _has_blend_mask = [0.0, 0.0]
            for i in range(min(mat.layer_count - 1, 2)):
                has_mask_i = i < len(mat.blend_masks) and _texture_is_usable(mat.blend_masks[i])
                _has_blend_mask[i] = 1.0 if has_mask_i else 0.0

            import struct as _struct
            for uname, vals in [
                ("sfHasBlendMask", _has_blend_mask),
            ]:
                if uname in program:
                    program[uname].write(_struct.pack(f"<{len(vals)}f", *vals))
            # Integer arrays need int packing
            for uname, vals in [
                ("sfBlendMode", [_BLEND_MODE_MAP.get(
                    mat.blend_modes[i] if i < len(mat.blend_modes) else "linear", 0) for i in range(2)]),
                ("sfBlendVCChannel", [_VC_MAP.get(
                    mat.blend_vc_channels[i] if i < len(mat.blend_vc_channels) else None, 0) for i in range(2)]),
            ]:
                if uname in program:
                    program[uname].write(_struct.pack(f"<{len(vals)}i", *vals))
            for uname, vals in [
                ("sfBlendHeightThreshold", [
                    mat.blend_height_thresholds[i] if i < len(mat.blend_height_thresholds) else 0.5 for i in range(2)]),
                ("sfBlendHeightFactor", [
                    mat.blend_height_factors[i] if i < len(mat.blend_height_factors) else 1.0 for i in range(2)]),
            ]:
                if uname in program:
                    program[uname].write(_struct.pack(f"<{len(vals)}f", *vals))

    def _set_effect_uniforms(self, program, mat: Material):
        """Bind textures and set uniforms for effect shader (BSEffectShaderProperty)."""
        if not hasattr(self, '_effect_debug_logged'):
            self._effect_debug_logged = set()
        mat_id = id(mat)
        if mat_id not in self._effect_debug_logged:
            self._effect_debug_logged.add(mat_id)
            print(f"EFFECT UNIFORMS: glowColor={tuple(mat.emissive_color)}, "
                  f"glowMult={mat.emissive_mult:.3f}, hasSourceTex={mat.has_source_texture}, "
                  f"diffuse_tex={mat.diffuse_tex}, is_effect={mat.is_effect_shader}")
        # Texture units: 0=BaseMap, 1=GreyscaleMap, 2=NormalMap, 3=CubeMap, 4=SpecularMap
        mat_diffuse = _usable_texture(mat.diffuse_tex)
        mat_greyscale = _usable_texture(mat.greyscale_tex)
        mat_normal = _usable_texture(mat.normal_tex)
        mat_env = _usable_texture(mat.env_tex)
        mat_env_mask = _usable_texture(mat.env_mask_tex)
        mat_spec = _usable_texture(mat.spec_tex)
        tex_d = mat_diffuse or _usable_texture(self.default_diffuse)
        _bind_texture(tex_d, 0)
        if "BaseMap" in program:
            program["BaseMap"].value = 0

        _bind_texture(mat_greyscale, 1)
        if "GreyscaleMap" in program:
            program["GreyscaleMap"].value = 1

        tex_n = mat_normal or _usable_texture(self.default_normal)
        _bind_texture(tex_n, 2)
        if "NormalMap" in program:
            program["NormalMap"].value = 2

        if not _bind_texture(mat_env, 3):
            _bind_texture(self.default_env, 3)
        if "CubeMap" in program:
            program["CubeMap"].value = 3

        if not _bind_texture(mat_env_mask, 4):
            _bind_texture(mat_spec, 4)
        if "SpecularMap" in program:
            program["SpecularMap"].value = 4

        # Effect-specific uniforms
        for name, val in [
            ("hasSourceTexture", 1.0 if mat_diffuse is not None and mat.has_source_texture else 0.0),
            ("hasGreyscaleMap", 1.0 if mat_greyscale is not None else 0.0),
            ("hasNormalMap", 1.0 if mat_normal is not None else 0.0),
            ("hasCubeMap", 1.0 if mat_env is not None and mat.has_env_map else 0.0),
            ("hasEnvMask", 1.0 if mat_env_mask is not None or mat_spec is not None else 0.0),
            ("glowMult", mat.emissive_mult),
            ("useFalloff", 1.0 if mat.use_falloff else 0.0),
            ("hasRGBFalloff", 1.0 if mat.has_rgb_falloff else 0.0),
            ("greyscaleColor", 1.0 if mat.greyscale_color else 0.0),
            ("greyscaleAlpha", 1.0 if mat.greyscale_alpha else 0.0),
            ("lightingInfluence", mat.lighting_influence),
            ("envReflection", mat.env_reflection),
            ("falloffDepth", mat.falloff_depth),
            ("doubleSided", 1.0 if mat.double_sided else 0.0),
            ("alphaFlags", mat.alpha_flags),
            ("alphaThreshold", mat.alpha_threshold),
        ]:
            if name in program:
                program[name].value = val

        if "glowColor" in program:
            program["glowColor"].value = tuple(mat.emissive_color)
        if "falloffParams" in program:
            program["falloffParams"].value = tuple(mat.falloff_params)
        if "uvScaleOffset" in program:
            program["uvScaleOffset"].value = tuple(mat.uv_scale_offset)

    def _get_alt_vao(self, mesh: Mesh, program: moderngl.Program):
        """Get or create a VAO for an alternate shader program.

        Normals/UV-checker shaders have different attribute layouts than the
        main fo4 shader. We create VAOs lazily and cache them.
        """
        key = (mesh.vao.glo, program.glo)
        vao = self._alt_vao_cache.get(key)
        if vao is not None:
            return vao

        if not mesh.vbo_format or not mesh.vbo_attrs:
            return None

        # Build format string, converting unused attrs to padding
        fmt_parts = mesh.vbo_format.split()
        offset_parts = []
        for f_part, attr_name in zip(fmt_parts, mesh.vbo_attrs):
            if attr_name in program:
                offset_parts.append(f_part)
            else:
                # Convert float count to byte padding (e.g. "3f" = 12 bytes -> "12x")
                n_floats = int(f_part[:-1])
                offset_parts.append(f"{n_floats * 4}x")

        active_attrs = [a for a in mesh.vbo_attrs if a in program]
        if not active_attrs:
            return None

        vao = self.ctx.vertex_array(
            program,
            [(mesh.vbo, " ".join(offset_parts), *active_attrs)],
            index_buffer=mesh.ibo,
        )
        self._alt_vao_cache[key] = vao
        return vao

    def clear_alt_vao_cache(self):
        """Clear cached alternate VAOs (call when scene is rebuilt)."""
        for vao in self._alt_vao_cache.values():
            vao.release()
        self._alt_vao_cache.clear()
        # Also clear fullscreen VAOs and invalidate shadow map
        for vao in self._fullscreen_vaos.values():
            vao.release()
        self._fullscreen_vaos.clear()
        self._shadow_dirty = True
        self.clear_collision_overlay()
        if getattr(self, "particle_renderer", None) is not None:
            self.particle_renderer.clear()

    def clear_collision_overlay(self):
        """Release cached collision overlay buffers."""
        if self._collision_vbo:
            self._collision_vbo.release()
            self._collision_vbo = None
        if self._collision_color_vbo:
            self._collision_color_vbo.release()
            self._collision_color_vbo = None
        if self._collision_vao:
            self._collision_vao.release()
            self._collision_vao = None
        self._collision_num_verts = 0
        self._collision_dirty = True

    def _iter_particle_runtimes(self):
        registry = getattr(self, "active_nif_registry", None)
        if registry is not None:
            sessions = registry.all_sessions()
        else:
            active_session = getattr(self, "active_nif_session", None)
            sessions = [active_session] if active_session is not None else []

        for session in sessions:
            runtime = getattr(session, "particle_runtime", None)
            if runtime is not None and getattr(runtime, "has_particles", False):
                yield session, runtime

    def _particle_session_transform(self, session):
        scene_root = getattr(session, "scene_root", None)
        transform = getattr(scene_root, "world_transform", None)
        if transform is not None:
            return transform

        attachment_node = getattr(session, "attachment_node", None)
        transform = getattr(attachment_node, "world_transform", None)
        if transform is not None:
            return transform

        transform = getattr(attachment_node, "transform", None)
        if transform is not None:
            return transform

        return glm.mat4(1.0)

    def _find_particle_system_node(self, node, system_block_id):
        if node is None or system_block_id is None:
            return None
        if getattr(node, "block_id", None) == system_block_id:
            return node
        for child in getattr(node, "children", ()) or ():
            match = self._find_particle_system_node(child, system_block_id)
            if match is not None:
                return match
        return None

    def _particle_batch_transform(self, session, batch):
        emitter_object_block_id = getattr(batch, "emitter_object_block_id", None)
        system_block_id = getattr(batch, "system_block_id", None)
        scene_root = getattr(session, "scene_root", None)
        emitter_node = self._find_particle_system_node(scene_root, emitter_object_block_id)
        transform = getattr(emitter_node, "world_transform", None)
        if transform is not None:
            return transform
        system_node = self._find_particle_system_node(scene_root, system_block_id)
        transform = getattr(system_node, "world_transform", None)
        if transform is not None:
            return transform
        return self._particle_session_transform(session)

    def _transform_particle_batch(self, batch, transform):
        positions = np.asarray(batch.positions, dtype=np.float32)
        transformed_positions = np.empty_like(positions)
        for index, position in enumerate(positions):
            world_position = transform * glm.vec4(
                float(position[0]),
                float(position[1]),
                float(position[2]),
                1.0,
            )
            transformed_positions[index] = (world_position.x, world_position.y, world_position.z)

        return replace(batch, positions=transformed_positions)

    def _build_particle_draw_batches(self):
        draw_batches = []
        for session, runtime in self._iter_particle_runtimes():
            for batch in runtime.build_draw_batches():
                transform = self._particle_batch_transform(session, batch)
                draw_batches.append(self._transform_particle_batch(batch, transform))
        return draw_batches

    # NiAlphaProperty blend factor mapping (NifSkope blendMap[16])
    _BLEND_MAP = [
        moderngl.ONE,                    # 0
        moderngl.ZERO,                   # 1
        moderngl.SRC_COLOR,              # 2
        moderngl.ONE_MINUS_SRC_COLOR,    # 3
        moderngl.DST_COLOR,              # 4
        moderngl.ONE_MINUS_DST_COLOR,    # 5
        moderngl.SRC_ALPHA,              # 6
        moderngl.ONE_MINUS_SRC_ALPHA,    # 7
        moderngl.DST_ALPHA,              # 8
        moderngl.ONE_MINUS_DST_ALPHA,    # 9
        moderngl.ONE,                    # 10 (SRC_ALPHA_SATURATE not in moderngl)
        moderngl.ONE,                    # 11 (unused, fallback)
        moderngl.ONE,                    # 12
        moderngl.ONE,                    # 13
        moderngl.ONE,                    # 14
        moderngl.ONE,                    # 15
    ]

    def _compute_scene_bounds(self, node: SceneNode,
                               min_pt: glm.vec3, max_pt: glm.vec3):
        """Recursively compute axis-aligned bounding box of the scene."""
        if not node.visible:
            return
        if node.mesh and node.bound_radius > 0:
            c = node.bound_center
            r = node.bound_radius
            min_pt.x = min(min_pt.x, c.x - r)
            min_pt.y = min(min_pt.y, c.y - r)
            min_pt.z = min(min_pt.z, c.z - r)
            max_pt.x = max(max_pt.x, c.x + r)
            max_pt.y = max(max_pt.y, c.y + r)
            max_pt.z = max(max_pt.z, c.z + r)
        for child in node.children:
            self._compute_scene_bounds(child, min_pt, max_pt)

    def _render_shadow_map(self, lighting):
        """Render the scene from the key light's perspective into the shadow map."""
        if not self._shadow_fbo or not self.scene_root:
            return

        light_dir = glm.normalize(glm.vec3(
            *tuple(glm.normalize(lighting.key_dir))
        ))

        # Check if light direction changed (also check lighting's dirty flag)
        lighting_dirty = getattr(lighting, '_shadow_dirty', False)
        if glm.length(light_dir - self._shadow_light_dir) < 0.001 and not self._shadow_dirty and not lighting_dirty:
            return
        self._shadow_light_dir = light_dir
        self._shadow_dirty = False
        if lighting_dirty:
            lighting._shadow_dirty = False

        # Compute scene AABB
        min_pt = glm.vec3(1e10)
        max_pt = glm.vec3(-1e10)
        self._compute_scene_bounds(self.scene_root, min_pt, max_pt)
        if min_pt.x > max_pt.x:
            return  # empty scene

        center = (min_pt + max_pt) * 0.5
        extent = glm.length(max_pt - min_pt) * 0.5
        if extent < 0.01:
            extent = 10.0

        # Light view matrix: look from the light direction toward scene center
        light_pos = center + light_dir * extent * 2.0
        light_view = glm.lookAt(light_pos, center, glm.vec3(0, 0, 1))
        # Orthographic projection sized to encompass the scene
        light_proj = glm.ortho(-extent, extent, -extent, extent,
                                0.1, extent * 4.0)
        self._light_space_matrix = light_proj * light_view

        # Render to shadow FBO
        shadow_prog = self.programs.get("shadow_depth")
        if not shadow_prog:
            return

        self._shadow_fbo.use()
        self.ctx.clear(depth=1.0)
        self.ctx.enable(moderngl.DEPTH_TEST)
        self.ctx.enable(moderngl.CULL_FACE)
        # Use front-face culling to reduce peter-panning
        self.ctx.cull_face = "front"

        # Dispatch to the active backend. SfBackend walks sf_scene.meshes
        # depth-only via SFScene.render_shadow_casters; Fo4Backend walks
        # scene_root via the recursive _draw_shadow_node helper.
        from creation_lib.renderer.backends.sf_backend import SfBackend
        from creation_lib.renderer.backends.fo4_backend import Fo4Backend
        if isinstance(self.backend, SfBackend) and self.backend.has_scene():
            # Convert glm light_vp to row-major numpy then back to GL
            # column-major inside the SF caster pass.
            light_vp_glm = light_proj * light_view
            light_vp_np = np.array(
                [[light_vp_glm[c][r] for c in range(4)] for r in range(4)],
                dtype=np.float32)
            self.backend.render_shadow_casters(light_vp_np, shadow_prog)
        elif isinstance(self.backend, Fo4Backend):
            self.backend._draw_shadow_node(
                self.scene_root, shadow_prog, light_proj * light_view)

        self.ctx.cull_face = "back"

    def _render_ssao(self, camera):
        """Run the SSAO pass: sample hemisphere → blur → output AO texture."""
        if not self._ssao_fbo or not self._fbo_depth_tex or not self._fbo_normal_tex:
            return

        ssao_prog = self.programs.get("ssao")
        blur_prog = self.programs.get("ssao_blur")
        if not ssao_prog or not blur_prog:
            return

        aspect = self._fbo_size[0] / max(self._fbo_size[1], 1)
        proj = camera.get_projection_matrix(aspect)
        inv_proj = glm.inverse(proj)

        # --- SSAO pass ---
        self._ssao_fbo.use()
        self.ctx.clear(1.0, 0.0, 0.0, 1.0)
        self.ctx.disable(moderngl.DEPTH_TEST)
        self.ctx.disable(moderngl.CULL_FACE)

        # Bind inputs
        self._fbo_depth_tex.use(0)
        self._fbo_normal_tex.use(1)
        self._ssao_noise_tex.use(2)

        ssao_prog["depthTex"].value = 0
        ssao_prog["normalTex"].value = 1
        ssao_prog["noiseTex"].value = 2

        if "projection" in ssao_prog:
            ssao_prog["projection"].value = tuple(
                c for col in proj for c in col
            )
        if "invProjection" in ssao_prog:
            ssao_prog["invProjection"].value = tuple(
                c for col in inv_proj for c in col
            )
        ssao_w = max(1, self._fbo_size[0] // 2)
        ssao_h = max(1, self._fbo_size[1] // 2)
        if "screenSize" in ssao_prog:
            ssao_prog["screenSize"].value = (float(ssao_w), float(ssao_h))
        if "radius" in ssao_prog:
            ssao_prog["radius"].value = 0.5
        if "bias" in ssao_prog:
            ssao_prog["bias"].value = 0.025
        if "intensity" in ssao_prog:
            ssao_prog["intensity"].value = 1.5
        if "numSamples" in ssao_prog:
            ssao_prog["numSamples"].value = min(len(self._ssao_kernel), 32)

        # Set kernel samples — ModernGL exposes array uniform as 'samples', not 'samples[0]'
        if "samples" in ssao_prog:
            kernel = self._ssao_kernel[:32]
            # Pad to declared array size (32) with a unit +Z vector
            while len(kernel) < 32:
                kernel.append((0.0, 0.0, 1.0))
            ssao_prog["samples"].value = kernel

        # Render fullscreen triangle
        self._get_fullscreen_vao(ssao_prog).render(moderngl.TRIANGLES, vertices=3)

        # --- Horizontal blur ---
        self._ssao_blur_fbo.use()
        self.ctx.clear(1.0, 0.0, 0.0, 1.0)

        self._ssao_tex.use(0)
        self._fbo_depth_tex.use(1)
        blur_prog["aoTex"].value = 0
        blur_prog["depthTex"].value = 1
        if "texelSize" in blur_prog:
            blur_prog["texelSize"].value = (1.0 / ssao_w, 1.0 / ssao_h)
        if "blurDir" in blur_prog:
            blur_prog["blurDir"].value = (1.0, 0.0)

        self._get_fullscreen_vao(blur_prog).render(moderngl.TRIANGLES, vertices=3)

        # --- Vertical blur (back to ssao_tex) ---
        self._ssao_fbo.use()
        self.ctx.clear(1.0, 0.0, 0.0, 1.0)

        self._ssao_blur_tex.use(0)
        self._fbo_depth_tex.use(1)
        blur_prog["aoTex"].value = 0
        blur_prog["depthTex"].value = 1
        if "blurDir" in blur_prog:
            blur_prog["blurDir"].value = (0.0, 1.0)

        self._get_fullscreen_vao(blur_prog).render(moderngl.TRIANGLES, vertices=3)

    def _composite_pass(self):
        """Apply SSAO to the scene via a fullscreen composite pass.

        Reads fbo_texture (scene) + ssao_tex (AO), writes to _composite_tex.
        get_fbo_texture_id() returns composite_tex when SSAO is active.
        """
        comp_prog = self.programs.get("composite")
        if not comp_prog or not self._composite_fbo:
            return

        self._composite_fbo.use()
        self.ctx.disable(moderngl.DEPTH_TEST)
        self.ctx.disable(moderngl.CULL_FACE)

        self.fbo_texture.use(0)
        comp_prog["sceneTex"].value = 0

        if self._ssao_tex:
            self._ssao_tex.use(1)
            if "aoTex" in comp_prog:
                comp_prog["aoTex"].value = 1
        if "aoEnabled" in comp_prog:
            comp_prog["aoEnabled"].value = 1.0

        self._get_fullscreen_vao(comp_prog).render(moderngl.TRIANGLES, vertices=3)

    def load_sf_scene(self, nif_path, extracted_dir, exr_path=None):
        """Build the parallel Starfield render engine for a newly-loaded NIF.

        Thin shim — switches the active backend to SfBackend (if it
        isn't already) and forwards. The actual SFScene construction
        lives in ``SfBackend.load_sf_scene``; this shim keeps the call
        site in app.py unchanged.
        """
        self._ensure_backend("starfield")
        # _ensure_backend always populates self.backend
        self.backend.load_sf_scene(nif_path, extracted_dir, exr_path)  # type: ignore[union-attr]

    def unload_sf_scene(self):
        """Release the parallel Starfield engine scene.

        Thin shim that forwards to ``SfBackend.unload_sf_scene`` when
        SfBackend is active, no-op otherwise. Called by app.py when
        switching to a non-SF NIF.
        """
        from creation_lib.renderer.backends.sf_backend import SfBackend
        if isinstance(self.backend, SfBackend):
            self.backend.unload_sf_scene()

    def render(self, camera, lighting):
        """Render scene to FBO."""
        if not self.fbo:
            return

        # Check SSAO/shadow toggles
        self._ssao_enabled = self.toggles.ssao
        self._shadow_enabled = self.toggles.shadows

        # Resolve the active game profile and ensure the per-game scene
        # backend exists *before* the shadow pass — the shadow pass walks
        # the scene with Fo4Backend._draw_shadow_node, which needs
        # self.backend to be populated.
        game_id = "fo4"
        if self.active_nif_session is not None:
            try:
                session = self.active_nif_session
                if session and hasattr(session, 'game_profile') and session.game_profile:
                    game_id = session.game_profile.id
            except (KeyError, AttributeError):
                pass
        self._current_game_id = game_id
        self._ensure_backend(game_id)
        assert self.backend is not None  # _ensure_backend always populates it

        # --- Shadow map pass (before main scene) ---
        # Skip entirely for backends that don't sample the shadow map
        # (currently SfBackend). Driven by ``backend.supports_shadows``
        # so future backends pick up the right behaviour automatically.
        if (self._shadow_enabled and self.scene_root
                and self.backend is not None
                and self.backend.supports_shadows):
            self._render_shadow_map(lighting)

        self.fbo.use()
        bg = getattr(self, "bg_color", (0.18, 0.18, 0.20))
        self.ctx.clear(bg[0], bg[1], bg[2], 1.0)
        self.ctx.enable(moderngl.DEPTH_TEST)
        self.ctx.enable(moderngl.CULL_FACE)

        if not self.programs:
            self.ctx.screen.use()
            return

        aspect = self._fbo_size[0] / max(self._fbo_size[1], 1)
        view = camera.get_view_matrix()
        proj = camera.get_projection_matrix(aspect)
        vp = proj * view
        self._current_vp = vp
        self._current_view = view

        # Determine render mode
        rm = self.render_mode_mgr
        mode = rm.mode if rm else RenderMode.TEXTURED
        textured_base_enabled = True if rm is None else rm.should_draw_textured_base()
        wireframe_enabled = bool(rm and rm.is_enabled(RenderMode.WIREFRAME))
        normals_enabled = bool(rm and rm.is_enabled(RenderMode.NORMALS))
        uv_checker_enabled = bool(rm and rm.is_enabled(RenderMode.UV_CHECKER))

        # Store lighting/camera for effect shader nodes drawn with different program
        self._current_lighting = lighting
        self._current_camera_pos = tuple(camera.get_eye_position())

        # game_id and self.backend were resolved at the top of render()
        # so the shadow pass could use them. Just pick the FO4-shaped
        # program here — same fallback chain as before.
        fo4_prog = self.programs.get(game_id) or self.programs.get("fo4")
        # Effect shader program for this game (e.g. "starfield_effect", "fo76_effect")
        self._current_effect_prog = (
            self.programs.get(f"{game_id}_effect")
            or self.programs.get("effect")
        )

        # --- Starfield wholesale-port draw path -----------------------
        # When SfBackend is active and a scene is loaded, delegate the
        # whole frame (lighting, draw, grid, attached-NIF interim pass,
        # SSAO, composite) and return. The shadow pass above is already
        # skipped via ``backend.supports_shadows``.
        from creation_lib.renderer.backends.sf_backend import SfBackend
        if isinstance(self.backend, SfBackend) and self.backend.has_scene():
            self.backend.render_full(camera, lighting, view, proj, vp, mode, rm)
            if self.particle_renderer is not None:
                self.particle_renderer.clear()
            self.ctx.screen.use()
            return

        # FO4 draw path. Narrowed to Fo4Backend so the type checker is
        # happy with the ``self.backend._draw_node`` calls below, and so
        # we don't accidentally invoke FO4 helpers on SfBackend during
        # the brief window between a game switch and an SF scene load.
        from creation_lib.renderer.backends.fo4_backend import Fo4Backend
        if (self.scene_root and fo4_prog
                and isinstance(self.backend, Fo4Backend)):
            # --- Textured pass ---
            if textured_base_enabled:
                self.backend._setup_fo4_uniforms(fo4_prog, lighting, rm)
                # Set the same debug toggles/tuning on the effect program so
                # effect-shader meshes respect toggle_diffuse, toggle_lighting etc.
                if self._current_effect_prog and self._current_effect_prog is not fo4_prog:
                    self.backend._setup_fo4_uniforms(self._current_effect_prog, lighting, rm)
                self.backend._draw_node(self.scene_root, fo4_prog, "opaque", use_alt_vao=False)

            # --- Wireframe pass ---
            if wireframe_enabled:
                self._draw_wireframe_pass(self.scene_root, fo4_prog, "opaque", lighting, rm)

            # --- UV checker pass ---
            if uv_checker_enabled:
                uv_prog = self.programs.get("uv_checker")
                if uv_prog:
                    if self.uv_checker_texture:
                        self.uv_checker_texture.use(0)
                    if "uv_checker_tex" in uv_prog:
                        uv_prog["uv_checker_tex"].value = 0
                    self.backend._draw_node(self.scene_root, uv_prog, "opaque", use_alt_vao=True)

            # --- Normals overlay (on top of textured mesh) ---
            if normals_enabled:
                normals_prog = self.programs.get("normals")
                if normals_prog:
                    if "u_normal_length" in normals_prog:
                        normals_prog["u_normal_length"].value = 0.5
                    self.backend._draw_node(self.scene_root, normals_prog, "opaque", use_alt_vao=True)

        # Draw grid BEFORE transparent pass so transparent objects blend over it
        vp_tuple = tuple(c for col in vp for c in col)
        if self.grid and self.grid_visible:
            self.ctx.disable(moderngl.CULL_FACE)
            self.ctx.enable(moderngl.BLEND)
            self.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
            self.grid.render(vp_tuple)
            self.ctx.disable(moderngl.BLEND)
            self.ctx.enable(moderngl.CULL_FACE)

        if self.particle_renderer is not None:
            draw_batches = self._build_particle_draw_batches()

            if draw_batches:
                self.particle_renderer.update_batches(draw_batches)
                view_inv = glm.inverse(view)
                camera_right = glm.vec3(view_inv[0])
                camera_up = glm.vec3(view_inv[1])
                self.particle_renderer.render(vp_tuple, camera_right, camera_up)
            else:
                self.particle_renderer.clear()

        # --- Transparent pass (after grid, so alpha-blended meshes composite over it) ---
        if (self.scene_root and fo4_prog
                and isinstance(self.backend, Fo4Backend)):
            if textured_base_enabled:
                self.backend._setup_fo4_uniforms(fo4_prog, lighting, rm)
                if self._current_effect_prog and self._current_effect_prog is not fo4_prog:
                    self.backend._setup_fo4_uniforms(self._current_effect_prog, lighting, rm)
                self.backend._draw_node(self.scene_root, fo4_prog, "transparent", use_alt_vao=False)

            if wireframe_enabled:
                self._draw_wireframe_pass(self.scene_root, fo4_prog, "transparent", lighting, rm)

            if uv_checker_enabled:
                uv_prog = self.programs.get("uv_checker")
                if uv_prog:
                    self.backend._draw_node(self.scene_root, uv_prog, "transparent", use_alt_vao=True)

            if normals_enabled:
                normals_prog = self.programs.get("normals")
                if normals_prog:
                    self.backend._draw_node(self.scene_root, normals_prog, "transparent", use_alt_vao=True)

        # Draw vertex overlay
        if self.toggles.show_vertices:
            self.render_vertices(self.scene_root, view, proj)

        # Draw connect points
        if self.connect_points is not None:
            cp = self.connect_points
            if cp.visible and "connect_point" in self.programs:
                self.ctx.disable(moderngl.DEPTH_TEST)
                cp.render(self.programs["connect_point"], vp_tuple)
                self.ctx.enable(moderngl.DEPTH_TEST)

        # Draw light icons
        if self.light_display is not None:
            ld = self.light_display
            if ld.visible and ld._num_vertices > 0 and "connect_point" in self.programs:
                self.ctx.disable(moderngl.DEPTH_TEST)
                self.ctx.enable(moderngl.BLEND)
                self.ctx.blend_func = (
                    moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA
                )
                ld.render(self.programs["connect_point"], vp_tuple)
                self.ctx.disable(moderngl.BLEND)
                self.ctx.enable(moderngl.DEPTH_TEST)

        # Collision overlay
        if self._show_collision:
            self._render_collision_overlay(vp_tuple)

        # Selection outline
        if self.selection_mgr is not None and self.selection_mgr.selected:
            sel_node = self.selection_mgr.selected
            sp = getattr(self, "settings_panel", None)
            outline_style = sp.outline_style if sp else "glow"
            outline_color = sp.outline_color if sp else [0.3, 0.6, 1.0]
            if sel_node.mesh and outline_style != "none":
                if outline_style == "wireframe":
                    self.backend._render_selection_wireframe(sel_node, fo4_prog, vp, outline_color)
                else:
                    self.backend._render_selection_outline(sel_node, vp, outline_color)

        # --- SSAO pass (after all scene rendering) ---
        if self._ssao_enabled and self.scene_root:
            self._render_ssao(camera)
            self._composite_pass()

        # Restore default framebuffer
        self.ctx.screen.use()

    def _draw_wireframe_pass(self, node: SceneNode, prog, pass_type: str,
                             lighting, rm):
        self.backend._setup_fo4_uniforms(prog, lighting, rm)
        if "toggle_lighting" in prog:
            prog["toggle_lighting"].value = 0.0
        if "toggle_diffuse" in prog:
            prog["toggle_diffuse"].value = 0.0
        self.ctx.depth_func = "<="
        self.ctx.wireframe = True
        try:
            self.backend._draw_node(node, prog, pass_type, use_alt_vao=False)
        finally:
            self.ctx.wireframe = False
            self.ctx.depth_func = "<"

    def render_vertices(self, node: SceneNode | None, view: glm.mat4, proj: glm.mat4):
        """Render all mesh vertices as blue dots using the vertex_points shader."""
        if node is None:
            return
        prog = self.programs.get("vertex_points")
        if prog is None:
            return
        self._render_vertices_node(node, view, proj, prog)

    def _render_vertices_node(self, node: SceneNode, view: glm.mat4, proj: glm.mat4,
                               prog):
        """Recursively render vertex dots for a node and its children."""
        if not node.visible:
            return
        if node.mesh is not None:
            vao = self._get_alt_vao(node.mesh, prog)
            if vao is not None:
                mvp = proj * view * node.world_transform
                prog["u_mvp"].value = tuple(c for col in mvp for c in col)
                vao.render(moderngl.POINTS)
        for child in node.children:
            self._render_vertices_node(child, view, proj, prog)

    def _rebuild_collision_overlay(self, node: SceneNode):
        """Collect all collision overlay geometry from the scene tree."""
        self.clear_collision_overlay()

        if not node:
            return

        all_positions = []
        all_colors = []

        def _collect(n: SceneNode):
            overlay = getattr(n, 'collision_overlay', None)
            if overlay and hasattr(overlay, 'positions') and len(overlay.positions) > 0:
                # Transform positions to world space
                for pos in overlay.positions:
                    wp = n.world_transform * glm.vec4(float(pos[0]), float(pos[1]), float(pos[2]), 1.0)
                    all_positions.extend([wp.x, wp.y, wp.z])
                    all_colors.extend(overlay.color)
            for child in n.children:
                _collect(child)

        _collect(node)

        if not all_positions:
            return

        pos_data = np.array(all_positions, dtype=np.float32)
        col_data = np.array(all_colors, dtype=np.float32)

        prog = self.programs.get("connect_point")
        if not prog:
            return

        self._collision_vbo = self.ctx.buffer(pos_data.tobytes())
        self._collision_color_vbo = self.ctx.buffer(col_data.tobytes())
        self._collision_num_verts = len(all_positions) // 3

        self._collision_vao = self.ctx.vertex_array(prog, [
            (self._collision_vbo, "3f", "in_position"),
            (self._collision_color_vbo, "4f", "in_color"),
        ])
        self._collision_dirty = False

    def _render_collision_overlay(self, vp_tuple):
        """Render collision wireframe lines."""
        if not self._show_collision:
            return
        if self._collision_dirty and self.scene_root:
            self._rebuild_collision_overlay(self.scene_root)
        if not self._collision_vao or self._collision_num_verts == 0:
            return

        prog = self.programs.get("connect_point")
        if not prog:
            return

        self.ctx.disable(moderngl.DEPTH_TEST)
        self.ctx.disable(moderngl.CULL_FACE)
        self.ctx.enable(moderngl.BLEND)
        self.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
        self.ctx.line_width = 2.0

        prog["u_mvp"].value = vp_tuple
        self._collision_vao.render(moderngl.LINES)

        self.ctx.line_width = 1.0
        self.ctx.disable(moderngl.BLEND)
        self.ctx.enable(moderngl.CULL_FACE)
        self.ctx.enable(moderngl.DEPTH_TEST)

    def get_fbo_texture_id(self) -> int:
        """Return OpenGL texture handle for imgui.image()."""
        # When SSAO is active AND the composite pass ran (scene present),
        # the composited result is in _composite_tex.
        # Without a scene, the composite pass is skipped, so fall back to
        # fbo_texture (which still has the grid and background).
        # SfBackend: writes view-space normals to color attachment 1 when
        # SSAO is enabled, so the composite path works the same as FO4.
        # Delegated to the backend so future per-game backends pick the
        # right texture without another branch here.
        from creation_lib.renderer.backends.sf_backend import SfBackend
        if isinstance(self.backend, SfBackend) and self.backend.has_scene():
            return self.backend.get_fbo_texture_id()
        if self._ssao_enabled and self._composite_tex and self.scene_root:
            return self._composite_tex.glo
        if self.fbo_texture:
            return self.fbo_texture.glo
        return 0
