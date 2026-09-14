"""Shader compilation and default texture management for ModernGL."""
from __future__ import annotations
from pathlib import Path
import logging
import re
import moderngl
import numpy as np

_log = logging.getLogger("renderer.shaders")
_SHADER_DIR = Path(__file__).parent / "shaders"

# Regex matching #include "path/to/file.glsl"
_INCLUDE_RE = re.compile(r'^\s*#include\s+"([^"]+)"\s*$', re.MULTILINE)

# ---------------------------------------------------------------------------
# Per-context caches — compiled programs and textures survive across
# SceneRenderer instances that share the same GL context, so shaders
# are compiled only once even when multiple workspaces (NIF editor,
# aligner, bone editor) each create their own renderer.
# ---------------------------------------------------------------------------
_program_cache: dict[tuple[int, str], "moderngl.Program"] = {}
_texture_cache: dict[tuple[int, str], "moderngl.Texture | tuple"] = {}
_include_map_cache: dict[str, str] | None = None


def resolve_includes(
    source: str,
    include_map: dict[str, str],
    _seen: set[str] | None = None,
) -> str:
    """Recursively inline ``#include "path"`` directives from ``include_map`` (path -> content).

    Raises ValueError on circular or missing includes.
    """
    if _seen is None:
        _seen = set()

    def _replace(match: re.Match) -> str:
        path = match.group(1)
        if path in _seen:
            raise ValueError(f"Circular include detected: {path}")
        if path not in include_map:
            raise ValueError(f"Include not found: {path}")
        _seen.add(path)
        content = include_map[path]
        # Recursively resolve nested includes
        resolved = resolve_includes(content, include_map, _seen)
        _seen.discard(path)
        return resolved

    return _INCLUDE_RE.sub(_replace, source)


def inject_defines(source: str, defines: dict[str, str | None]) -> str:
    """Inject #define lines into GLSL source after the #version line.

    Args:
        source: GLSL source text.
        defines: Map of define name -> value (None for valueless defines).

    Returns:
        Source with #define lines inserted after #version.
    """
    if not defines:
        return source

    define_lines = []
    for name, value in sorted(defines.items()):
        if value is None:
            define_lines.append(f"#define {name}")
        else:
            define_lines.append(f"#define {name} {value}")
    define_block = "\n".join(define_lines)

    lines = source.split("\n")
    # Find #version line and insert after it
    for i, line in enumerate(lines):
        if line.strip().startswith("#version"):
            lines.insert(i + 1, define_block)
            return "\n".join(lines)

    # No #version line found — prepend defines
    return define_block + "\n" + source


def load_include_map(shaders_dir: Path | None = None) -> dict[str, str]:
    """Read all .glsl files in includes/ and materials/ subdirs into a dict.

    Keys are relative paths like "includes/common.glsl".
    Uses a module-level cache since shader files don't change at runtime.
    """
    global _include_map_cache
    if shaders_dir is None:
        shaders_dir = _SHADER_DIR
    if shaders_dir == _SHADER_DIR and _include_map_cache is not None:
        return _include_map_cache
    result: dict[str, str] = {}
    for subdir in ("includes", "materials"):
        d = shaders_dir / subdir
        if not d.is_dir():
            continue
        for f in d.iterdir():
            if f.suffix == ".glsl":
                key = f"{subdir}/{f.name}"
                result[key] = f.read_text(encoding="utf-8")
    if shaders_dir == _SHADER_DIR:
        _include_map_cache = result
    return result


def compile_program(
    ctx: moderngl.Context, name: str,
    vert_src: str, frag_src: str,
) -> moderngl.Program:
    """Compile and cache a shader program from source strings."""
    ck = (id(ctx), f"source:{name}")
    cached = _program_cache.get(ck)
    if cached is not None:
        _log.debug("Shader cache hit: %s", name)
        return cached
    prog = ctx.program(vertex_shader=vert_src, fragment_shader=frag_src)
    _program_cache[ck] = prog
    return prog


def load_shader(ctx: moderngl.Context, name: str) -> moderngl.Program:
    """Load and compile a vertex+fragment shader pair from the shaders/ dir."""
    ck = (id(ctx), f"shader:{name}")
    cached = _program_cache.get(ck)
    if cached is not None:
        _log.debug("Shader cache hit: %s", name)
        return cached
    vert_path = _SHADER_DIR / f"{name}.vert"
    frag_path = _SHADER_DIR / f"{name}.frag"
    vert_src = vert_path.read_text()
    frag_src = frag_path.read_text()
    prog = ctx.program(vertex_shader=vert_src, fragment_shader=frag_src)
    _log.info("Compiled shader: %s", name)
    _program_cache[ck] = prog
    return prog


def load_composed_shader(
    ctx: moderngl.Context,
    role: str,
    game_id: str = "fo4",
    defines: dict[str, str | None] | None = None,
) -> moderngl.Program:
    """Compile a composed shader from compose/ with #include preprocessing.

    ``role`` is "default" or "effect"; ``game_id`` is "fo4", "skyrimse", "fo76",
    "starfield", or "gamebryo"; ``defines`` are extra #defines (e.g.
    ``{"HAS_PALETTE": None}``).
    """
    defines_str = ",".join(
        f"{k}={v}" for k, v in sorted((defines or {}).items())
    )
    ck = (id(ctx), f"composed:{game_id}/{role}/{defines_str}")
    cached = _program_cache.get(ck)
    if cached is not None:
        _log.debug("Shader cache hit: %s/%s", game_id, role)
        return cached

    compose_dir = _SHADER_DIR / "compose"

    # Fragment shader: game-specific for "default", shared for "effect"
    if role == "default":
        if game_id in {"gamebryo", "ob_default"}:
            frag_path = compose_dir / "ob_default.frag"
        else:
            frag_path = compose_dir / f"{game_id}_default.frag"
    else:
        frag_path = compose_dir / f"shared_{role}.frag"

    # Vertex shader: game-specific override wins if present, else shared.
    # Starfield uses a view-space vertex shader for the stf_default port, so
    # {game_id}_{role}.vert takes precedence over shared_{role}.vert.
    game_vert = compose_dir / f"{game_id}_{role}.vert"
    vert_path = game_vert if game_vert.exists() else (compose_dir / f"shared_{role}.vert")

    if not frag_path.exists():
        raise FileNotFoundError(f"Composed shader not found: {frag_path}")
    if not vert_path.exists():
        raise FileNotFoundError(f"Composed vertex shader not found: {vert_path}")

    frag_src = frag_path.read_text(encoding="utf-8")
    vert_src = vert_path.read_text(encoding="utf-8")

    # Resolve #include directives
    include_map = load_include_map(_SHADER_DIR)
    frag_src = resolve_includes(frag_src, include_map)
    vert_src = resolve_includes(vert_src, include_map)

    # Inject #defines
    if defines:
        frag_src = inject_defines(frag_src, defines)

    prog = ctx.program(vertex_shader=vert_src, fragment_shader=frag_src)
    _log.info("Compiled composed shader: %s/%s (defines=%s)", game_id, role,
              list(defines.keys()) if defines else "none")
    _program_cache[ck] = prog
    return prog



def generate_brdf_lut_epic(ctx: moderngl.Context, size: int = 512) -> moderngl.Texture:
    """Generate a BRDF LUT texture for PBR split-sum IBL.

    Standard Epic split-sum LUT (RG16F: scale/bias). Used by FO76 and any
    shader expecting the Unreal split-sum form. Starfield uses a different
    LUT — see generate_sf_pbr_lut().
    """
    ck = (id(ctx), f"tex:brdf_lut_epic:{size}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached

    vert_src = (_SHADER_DIR / "brdf_lut.vert").read_text(encoding="utf-8")
    frag_src = (_SHADER_DIR / "brdf_lut.frag").read_text(encoding="utf-8")
    prog = ctx.program(vertex_shader=vert_src, fragment_shader=frag_src)

    lut_tex = ctx.texture((size, size), 2, dtype='f2')
    lut_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    lut_tex.repeat_x = False
    lut_tex.repeat_y = False

    fbo = ctx.framebuffer(color_attachments=[lut_tex])
    fbo.use()
    ctx.clear(0.0, 0.0, 0.0, 1.0)

    vao = ctx.vertex_array(prog, [])
    vao.render(mode=moderngl.TRIANGLES, vertices=3)

    vao.release()
    fbo.release()
    prog.release()

    _log.info("Generated BRDF LUT: %dx%d RG16F", size, size)
    _texture_cache[ck] = lut_tex
    return lut_tex


def generate_sf_pbr_lut(ctx: moderngl.Context,
                        size: int = 512) -> moderngl.Texture:
    """Generate NifSkope's Starfield PBR LUT (4-channel RGBA16F).

    Ports py_creation_lib/python/creation_lib/libfo76utils/src/pbr_lut.cpp to a GLSL fragment shader so the
    values exactly match what NifSkope's stf_default.frag was written against:
      R = indirect specular F (normalized)
      G = indirect specular G
      B = fresnel_n(NdotV)   (polynomial for n=1.5 dielectric)
      A = fresnel_n(roughness)
    """
    ck = (id(ctx), f"tex:sf_pbr_lut:{size}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached

    vert_src = (_SHADER_DIR / "brdf_lut.vert").read_text(encoding="utf-8")
    frag_src = (_SHADER_DIR / "sf_pbr_lut.frag").read_text(encoding="utf-8")
    prog = ctx.program(vertex_shader=vert_src, fragment_shader=frag_src)

    lut_tex = ctx.texture((size, size), 4, dtype='f2')
    lut_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    lut_tex.repeat_x = False
    lut_tex.repeat_y = False

    fbo = ctx.framebuffer(color_attachments=[lut_tex])
    fbo.use()
    ctx.clear(0.0, 0.0, 0.0, 0.0)

    vao = ctx.vertex_array(prog, [])
    vao.render(mode=moderngl.TRIANGLES, vertices=3)

    vao.release()
    fbo.release()
    prog.release()

    _log.info("Generated Starfield PBR LUT: %dx%d RGBA16F (NifSkope-compatible)",
              size, size)
    _texture_cache[ck] = lut_tex
    return lut_tex


_GRID_VERT = """
#version 330
uniform mat4 u_mvp;
in vec3 in_position;
void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);
}
"""

_GRID_FRAG = """
#version 330
uniform vec4 u_color;
out vec4 fragColor;
void main() {
    fragColor = u_color;
}
"""

_WIREFRAME_VERT = """
#version 330
uniform mat4 u_mvp;
in vec3 in_position;
void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);
}
"""

_WIREFRAME_FRAG = """
#version 330
uniform vec4 u_color;
out vec4 fragColor;
void main() {
    fragColor = u_color;
}
"""


def load_grid_shader(ctx: moderngl.Context) -> moderngl.Program:
    ck = (id(ctx), "inline:grid")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(vertex_shader=_GRID_VERT, fragment_shader=_GRID_FRAG)
    _program_cache[ck] = prog
    return prog


def load_wireframe_shader(ctx: moderngl.Context) -> moderngl.Program:
    ck = (id(ctx), "inline:wireframe")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(vertex_shader=_WIREFRAME_VERT, fragment_shader=_WIREFRAME_FRAG)
    _program_cache[ck] = prog
    return prog


_CP_VERT = """
#version 330
uniform mat4 u_mvp;
in vec3 in_position;
in vec4 in_color;
out vec4 vColor;
void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);
    vColor = in_color;
}
"""

_CP_FRAG = """
#version 330
in vec4 vColor;
out vec4 fragColor;
void main() {
    fragColor = vColor;
}
"""


def load_connect_point_shader(ctx: moderngl.Context) -> moderngl.Program:
    ck = (id(ctx), "inline:connect_point")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(vertex_shader=_CP_VERT, fragment_shader=_CP_FRAG)
    _program_cache[ck] = prog
    return prog


_NORMALS_VERT = """
#version 330
uniform mat4 u_mvp;
uniform mat4 u_model;
in vec3 in_position;
in vec3 in_normal;
out vec3 geom_normal;
void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);
    mat3 modelRot = mat3(u_model);
    geom_normal = normalize(modelRot * in_normal);
}
"""

_NORMALS_GEOM = """
#version 330
layout(triangles) in;
layout(line_strip, max_vertices = 6) out;
uniform mat4 u_mvp;
uniform float u_normal_length;
in vec3 geom_normal[];
out vec4 line_color;
void main() {
    for (int i = 0; i < 3; i++) {
        vec4 pos = gl_in[i].gl_Position;
        gl_Position = pos;
        line_color = vec4(0.0, 0.5, 1.0, 1.0);
        EmitVertex();
        vec3 n = geom_normal[i];
        gl_Position = u_mvp * (inverse(u_mvp) * pos + vec4(n * u_normal_length, 0.0));
        line_color = vec4(1.0, 1.0, 0.0, 1.0);
        EmitVertex();
        EndPrimitive();
    }
}
"""

_NORMALS_FRAG = """
#version 330
in vec4 line_color;
out vec4 fragColor;
void main() {
    fragColor = line_color;
}
"""


_UV_CHECKER_VERT = """
#version 330
uniform mat4 u_mvp;
in vec3 in_position;
in vec2 in_texcoord;
out vec2 vTexCoord;
void main() {
    gl_Position = u_mvp * vec4(in_position, 1.0);
    vTexCoord = in_texcoord;
}
"""

_UV_CHECKER_FRAG = """
#version 330
in vec2 vTexCoord;
out vec4 fragColor;
uniform sampler2D uv_checker_tex;
void main() {
    fragColor = texture(uv_checker_tex, vTexCoord);
}
"""


def load_normals_shader(ctx: moderngl.Context) -> moderngl.Program:
    ck = (id(ctx), "inline:normals")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(
        vertex_shader=_NORMALS_VERT,
        geometry_shader=_NORMALS_GEOM,
        fragment_shader=_NORMALS_FRAG,
    )
    _program_cache[ck] = prog
    return prog


def load_uv_checker_shader(ctx: moderngl.Context) -> moderngl.Program:
    ck = (id(ctx), "inline:uv_checker")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(
        vertex_shader=_UV_CHECKER_VERT,
        fragment_shader=_UV_CHECKER_FRAG,
    )
    _program_cache[ck] = prog
    return prog


def create_uv_checker_texture(ctx: moderngl.Context) -> moderngl.Texture:
    """Load uv_checker.png as a GPU texture for the UV checker render mode."""
    ck = (id(ctx), "tex:uv_checker")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    from PIL import Image
    img_path = _SHADER_DIR.parent / "assets" / "uv_checker.png"
    img = Image.open(img_path).convert("RGBA").transpose(Image.FLIP_TOP_BOTTOM)
    tex = ctx.texture(img.size, 4, img.tobytes())
    tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    tex.build_mipmaps()
    tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    _texture_cache[ck] = tex
    return tex


def load_vertex_points_shader(ctx: moderngl.Context) -> moderngl.Program:
    """Compile the vertex-points overlay shader (flat blue points)."""
    return load_shader(ctx, "vertex_points")


def create_default_diffuse(ctx: moderngl.Context) -> moderngl.Texture:
    """2x2 yellow checkerboard — NifSkope-style missing/unavailable texture."""
    ck = (id(ctx), "tex:default_diffuse")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    # Classic amber-yellow used by NIF viewers to indicate no texture
    y = b'\xff\xbf\x00\xff'  # amber yellow
    d = b'\xbf\x8f\x00\xff'  # darker amber
    pixels = y + d + d + y
    tex = ctx.texture((2, 2), 4, pixels)
    tex.filter = (moderngl.NEAREST, moderngl.NEAREST)
    _texture_cache[ck] = tex
    return tex


def create_default_normal(ctx: moderngl.Context) -> moderngl.Texture:
    """1x1 flat normal (0.5, 0.5, 1.0)."""
    ck = (id(ctx), "tex:default_normal")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    tex = ctx.texture((1, 1), 4, b'\x80\x80\xff\xff')
    _texture_cache[ck] = tex
    return tex


def create_default_spec(ctx: moderngl.Context) -> moderngl.Texture:
    """1x1 black specular."""
    ck = (id(ctx), "tex:default_spec")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    tex = ctx.texture((1, 1), 4, b'\x00\x00\x00\xff')
    _texture_cache[ck] = tex
    return tex


def _find_loose_texture(texture_dirs, candidates: list[str]) -> Path | None:
    """Fast-path lookup for a known loose texture without building dir indexes."""
    for base_dir in texture_dirs:
        for candidate in candidates:
            resolved = base_dir / Path(candidate)
            if resolved.is_file():
                return resolved
    return None


def create_default_env(ctx: moderngl.Context, texture_dirs=None,
                       ba2_mgr=None,
                       game_id: str = "fo4") -> tuple[moderngl.Texture, bool]:
    """Load the game's default cubemap, or generate a procedural fallback.

    Returns (texture, is_real) — is_real=True when the actual game cubemap was loaded.
    Game-specific procedural fallback: studio IBL for PBR games, outdoor sky for TBR.
    """
    # Try loading the real default cubemap — loose files only (don't trigger
    # lazy BA2 loading just for the cubemap; fall back to procedural instead).
    if texture_dirs:
        from .material_pipeline import _cached_load_cubemap

        candidates = [
            "textures/shared/cubemaps/mipblur_DefaultOutside1.dds",
            "Shared/Cubemaps/mipblur_DefaultOutside1.dds",
        ]
        resolved = _find_loose_texture(texture_dirs, candidates)
        if resolved is not None:
            tex = _cached_load_cubemap(ctx, resolved)
            # Check it's not the 1x1 fallback.
            if hasattr(tex, "size") and tex.size[0] > 1:
                _log.info("Loaded real default cubemap: %s", resolved)
                return tex, True

    # Procedural 2D latlong env map fallback (cached per game style)
    ck = (id(ctx), f"tex:default_env_procedural:{game_id}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        _log.debug("Env map cache hit (procedural/%s)", game_id)
        return cached, False
    tex = _generate_procedural_envmap(ctx, game_id)
    _log.info("Using procedural 512x256 %s env map", game_id)
    _texture_cache[ck] = tex
    return tex, False


def _generate_procedural_envmap(ctx: moderngl.Context,
                                game_id: str = "fo4") -> moderngl.Texture:
    """Generate a procedural latlong environment map.

    This is a 2D texture where U maps to horizontal angle (0=back, 0.5=front, 1=back)
    and V maps to vertical angle (0=top/sky, 1=bottom/ground).
    The shader's envMapUV() function converts reflection vectors to these coordinates.

    Game-specific styles:
      fo4/skyrimse: Outdoor sky — blue sky, horizon haze, single sun.
      starfield/fo76: Neutral studio IBL — bright, even, warm/cool contrast.
    """
    import math
    color_fn = (_procedural_studio_color if game_id in ("starfield", "fo76")
                else _procedural_sky_color)
    w, h = 512, 256
    pixels = bytearray(w * h * 4)
    for y in range(h):
        for x in range(w):
            theta = (float(x) / w) * 2.0 * math.pi
            phi = (0.5 - float(y) / h) * math.pi
            dx = math.sin(theta) * math.cos(phi)
            dy = math.sin(phi)
            dz = math.cos(theta) * math.cos(phi)
            r, g, b = color_fn(dx, dy, dz)
            offset = (y * w + x) * 4
            pixels[offset] = int(max(0, min(255, r * 255)))
            pixels[offset + 1] = int(max(0, min(255, g * 255)))
            pixels[offset + 2] = int(max(0, min(255, b * 255)))
            pixels[offset + 3] = 255
    tex = ctx.texture((w, h), 4, bytes(pixels))
    tex.build_mipmaps()
    tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    return tex


def _procedural_studio_color(dx: float, dy: float, dz: float) -> tuple[float, float, float]:
    """Neutral studio IBL for PBR metallic-roughness (Starfield/FO76).

    Bright overall so metals reflect enough light to show their albedo color.
    Warm key light from upper-right, cool fill from left, bright ground bounce.
    Multiple soft highlights for convincing metallic reflections.
    """
    import math
    length = math.sqrt(dx * dx + dy * dy + dz * dz)
    if length < 0.001:
        return (0.6, 0.6, 0.6)
    dx /= length; dy /= length; dz /= length

    sky_t = max(0, dy)
    ground_t = max(0, -dy)
    horizon = 1.0 - sky_t - ground_t

    # Upper hemisphere: warm neutral (studio ceiling with soft lights)
    sky_r = 0.55 + sky_t * 0.30
    sky_g = 0.55 + sky_t * 0.28
    sky_b = 0.58 + sky_t * 0.25

    # Ground: bright warm bounce (studio floor reflects key light)
    gnd_r = 0.40 + ground_t * 0.10
    gnd_g = 0.38 + ground_t * 0.08
    gnd_b = 0.35 + ground_t * 0.05

    # Horizon: bright band (studio walls / ambient wrap)
    hor_r, hor_g, hor_b = 0.55, 0.55, 0.58

    r = sky_r * sky_t + gnd_r * ground_t + hor_r * horizon
    g = sky_g * sky_t + gnd_g * ground_t + hor_g * horizon
    b = sky_b * sky_t + gnd_b * ground_t + hor_b * horizon

    # Key light: warm, upper-right-front (soft area light)
    key_x, key_y, key_z = 0.5, 0.6, 0.6
    key_len = math.sqrt(key_x**2 + key_y**2 + key_z**2)
    key_dot = (dx * key_x + dy * key_y + dz * key_z) / key_len
    key_broad = max(0, key_dot) ** 3 * 0.35
    key_core = max(0, key_dot) ** 32 * 0.50
    r += (key_broad + key_core) * 1.0
    g += (key_broad + key_core) * 0.95
    b += (key_broad + key_core) * 0.88

    # Fill light: cool, left side
    fill_x, fill_y, fill_z = -0.7, 0.3, 0.3
    fill_len = math.sqrt(fill_x**2 + fill_y**2 + fill_z**2)
    fill_dot = (dx * fill_x + dy * fill_y + dz * fill_z) / fill_len
    fill = max(0, fill_dot) ** 4 * 0.25
    r += fill * 0.80
    g += fill * 0.85
    b += fill * 1.00

    # Rim light: behind-above for edge highlights
    rim_x, rim_y, rim_z = 0.0, 0.4, -0.9
    rim_len = math.sqrt(rim_x**2 + rim_y**2 + rim_z**2)
    rim_dot = (dx * rim_x + dy * rim_y + dz * rim_z) / rim_len
    rim = max(0, rim_dot) ** 8 * 0.30
    r += rim * 0.95
    g += rim * 0.95
    b += rim * 1.00

    return (min(r, 1.0), min(g, 1.0), min(b, 1.0))


def _procedural_sky_color(dx: float, dy: float, dz: float) -> tuple[float, float, float]:
    """Outdoor sky IBL for FO4/Skyrim spec-gloss.

    Values are intentionally bright — this cubemap IS the surface color for
    metals in FO4's TBR. Chrome reflecting a dim cubemap looks like dark plastic.
    Real FO4 outdoor cubemaps have bright sky (~1.0), bright horizon (~0.7),
    and HDR sun spots.
    """
    import math
    length = math.sqrt(dx * dx + dy * dy + dz * dz)
    if length < 0.001:
        return (0.5, 0.5, 0.5)
    dx /= length
    dy /= length
    dz /= length

    # Vertical blend: sky (up) to ground (down)
    sky_t = max(0, dy)      # 0 at horizon, 1 at zenith
    ground_t = max(0, -dy)  # 0 at horizon, 1 at nadir

    # Sky gradient — bright blue-white (matching FO4 Commonwealth daytime)
    sky_r = 0.75 + sky_t * 0.20
    sky_g = 0.82 + sky_t * 0.15
    sky_b = 0.95 + sky_t * 0.05

    # Ground — medium gray (dirt/concrete reflects light)
    gnd_r = 0.25 - ground_t * 0.08
    gnd_g = 0.23 - ground_t * 0.06
    gnd_b = 0.20 - ground_t * 0.05

    # Horizon band — bright haze
    horizon = 1.0 - sky_t - ground_t
    hor_r, hor_g, hor_b = 0.65, 0.65, 0.70

    r = sky_r * sky_t + gnd_r * ground_t + hor_r * horizon
    g = sky_g * sky_t + gnd_g * ground_t + hor_g * horizon
    b = sky_b * sky_t + gnd_b * ground_t + hor_b * horizon

    # Broad sun glow (wide, bright — fills a large area of the cubemap)
    sun_dir_x, sun_dir_y, sun_dir_z = 0.3, 0.5, 0.8
    sun_len = math.sqrt(sun_dir_x**2 + sun_dir_y**2 + sun_dir_z**2)
    sun_dot = (dx * sun_dir_x + dy * sun_dir_y + dz * sun_dir_z) / sun_len
    # Broad glow (power=4) + tight sun core (power=64)
    broad_glow = max(0, sun_dot) ** 4 * 0.3
    sun_core = max(0, sun_dot) ** 64 * 0.8
    sun_intensity = broad_glow + sun_core
    r += sun_intensity * 1.0
    g += sun_intensity * 0.95
    b += sun_intensity * 0.85

    return (min(r, 1.0), min(g, 1.0), min(b, 1.0))


# ---------------------------------------------------------------------------
# Cubemap generation (real samplerCube for PBR IBL)
# ---------------------------------------------------------------------------

def _make_cubemap_face_dirs(face_idx: int, size: int) -> np.ndarray:
    """Return (size, size, 3) array of normalized directions for an OpenGL
    cubemap face. Face order: 0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z.

    Standard GL cubemap conventions: for each face, (s, t) pixel coords in
    [-1, 1] map to a 3D direction that passes through the pixel center on the
    unit cube. The face's "up" axis is chosen so filtering is continuous
    across face seams when GL_TEXTURE_CUBE_MAP_SEAMLESS is enabled.
    """
    # Pixel-centered UVs in [-1, 1]
    half = 1.0 - 1.0 / size
    s = np.linspace(-half, half, size)
    t = np.linspace(-half, half, size)
    ss, tt = np.meshgrid(s, t)  # both (size, size)
    ones = np.ones_like(ss)
    if face_idx == 0:     # +X (right)
        x, y, z = ones, -tt, -ss
    elif face_idx == 1:   # -X (left)
        x, y, z = -ones, -tt, ss
    elif face_idx == 2:   # +Y (top)
        x, y, z = ss, ones, tt
    elif face_idx == 3:   # -Y (bottom)
        x, y, z = ss, -ones, -tt
    elif face_idx == 4:   # +Z (front)
        x, y, z = ss, -tt, ones
    else:                 # -Z (back)
        x, y, z = -ss, -tt, -ones
    dirs = np.stack([x, y, z], axis=-1)
    norms = np.linalg.norm(dirs, axis=-1, keepdims=True)
    return dirs / norms


def _studio_cubemap_colors_np(dirs: np.ndarray, game_id: str,
                              with_lights: bool) -> np.ndarray:
    """Vectorized version of the procedural studio/sky color function.

    Returns (..., 3) RGB in [0,1]. When `with_lights` is False, omits sharp
    light highlights — used to generate an irradiance-style smooth ambient.
    """
    dx = dirs[..., 0]
    dy = dirs[..., 1]
    dz = dirs[..., 2]

    if game_id in ("starfield", "fo76"):
        sky_t = np.maximum(0.0, dy)
        ground_t = np.maximum(0.0, -dy)
        horizon = 1.0 - sky_t - ground_t

        sky_r = 0.55 + sky_t * 0.30
        sky_g = 0.55 + sky_t * 0.28
        sky_b = 0.58 + sky_t * 0.25
        gnd_r = 0.40 + ground_t * 0.10
        gnd_g = 0.38 + ground_t * 0.08
        gnd_b = 0.35 + ground_t * 0.05
        hor_r, hor_g, hor_b = 0.55, 0.55, 0.58

        r = sky_r * sky_t + gnd_r * ground_t + hor_r * horizon
        g = sky_g * sky_t + gnd_g * ground_t + hor_g * horizon
        b = sky_b * sky_t + gnd_b * ground_t + hor_b * horizon

        if with_lights:
            # Key light (warm, upper-right-front)
            kv = np.array([0.5, 0.6, 0.6], dtype=np.float64)
            kv /= np.linalg.norm(kv)
            kd = np.maximum(0.0, dx * kv[0] + dy * kv[1] + dz * kv[2])
            k_broad = kd ** 3 * 0.35
            k_core = kd ** 32 * 0.50
            k = k_broad + k_core
            r += k * 1.00
            g += k * 0.95
            b += k * 0.88

            # Fill light (cool, left)
            fv = np.array([-0.7, 0.3, 0.3], dtype=np.float64)
            fv /= np.linalg.norm(fv)
            fd = np.maximum(0.0, dx * fv[0] + dy * fv[1] + dz * fv[2])
            fill = fd ** 4 * 0.25
            r += fill * 0.80
            g += fill * 0.85
            b += fill * 1.00

            # Rim light
            rv = np.array([0.0, 0.4, -0.9], dtype=np.float64)
            rv /= np.linalg.norm(rv)
            rd = np.maximum(0.0, dx * rv[0] + dy * rv[1] + dz * rv[2])
            rim = rd ** 8 * 0.30
            r += rim * 0.95
            g += rim * 0.95
            b += rim * 1.00
    else:
        # FO4/Skyrim outdoor sky
        sky_t = np.maximum(0.0, dy)
        ground_t = np.maximum(0.0, -dy)
        horizon = 1.0 - sky_t - ground_t

        sky_r = 0.75 + sky_t * 0.20
        sky_g = 0.82 + sky_t * 0.15
        sky_b = 0.95 + sky_t * 0.05
        gnd_r = 0.25 - ground_t * 0.08
        gnd_g = 0.23 - ground_t * 0.06
        gnd_b = 0.20 - ground_t * 0.05
        hor_r, hor_g, hor_b = 0.65, 0.65, 0.70

        r = sky_r * sky_t + gnd_r * ground_t + hor_r * horizon
        g = sky_g * sky_t + gnd_g * ground_t + hor_g * horizon
        b = sky_b * sky_t + gnd_b * ground_t + hor_b * horizon

        if with_lights:
            sv = np.array([0.3, 0.5, 0.8], dtype=np.float64)
            sv /= np.linalg.norm(sv)
            sd = np.maximum(0.0, dx * sv[0] + dy * sv[1] + dz * sv[2])
            broad = sd ** 4 * 0.3
            core = sd ** 64 * 0.8
            sun = broad + core
            r += sun * 1.00
            g += sun * 0.95
            b += sun * 0.85

    return np.stack([
        np.clip(r, 0.0, 1.0),
        np.clip(g, 0.0, 1.0),
        np.clip(b, 0.0, 1.0),
    ], axis=-1)


def _build_procedural_cubemap(ctx: moderngl.Context, game_id: str,
                              face_size: int, with_lights: bool,
                              label: str) -> moderngl.TextureCube:
    """Build a procedural TextureCube by computing all 6 faces numerically."""
    face_datas = []
    for face_idx in range(6):
        dirs = _make_cubemap_face_dirs(face_idx, face_size)
        colors = _studio_cubemap_colors_np(dirs, game_id, with_lights=with_lights)
        rgba = np.empty((face_size, face_size, 4), dtype=np.uint8)
        rgba[..., :3] = (colors * 255.0).astype(np.uint8)
        rgba[..., 3] = 255
        face_datas.append(rgba.tobytes())

    data = b"".join(face_datas)
    cube = ctx.texture_cube((face_size, face_size), 4, data)
    try:
        cube.build_mipmaps()
    except Exception as e:
        _log.warning("TextureCube build_mipmaps failed for %s: %s", label, e)
    cube.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    _log.info("Generated procedural %s cubemap (%dx%d per face) for %s",
              label, face_size, face_size, game_id)
    return cube


def create_default_specular_cubemap(ctx: moderngl.Context,
                                    game_id: str = "starfield",
                                    face_size: int = 256) -> moderngl.TextureCube:
    """Procedural prefiltered specular cubemap (6 faces, full mip chain).

    Hardware mipmap generation gives roughness-based blur via LOD selection.
    Includes sharp highlights for visible metallic reflections.
    """
    ck = (id(ctx), f"texcube:spec:{game_id}:{face_size}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    cube = _build_procedural_cubemap(ctx, game_id, face_size,
                                     with_lights=True, label="specular")
    _texture_cache[ck] = cube
    return cube


def create_default_irradiance_cubemap(ctx: moderngl.Context,
                                      game_id: str = "starfield",
                                      face_size: int = 32) -> moderngl.TextureCube:
    """Procedural irradiance (diffuse) cubemap — smooth ambient without sharp
    highlights. Low resolution since diffuse IBL is inherently low-frequency.
    """
    ck = (id(ctx), f"texcube:irr:{game_id}:{face_size}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    cube = _build_procedural_cubemap(ctx, game_id, face_size,
                                     with_lights=False, label="irradiance")
    _texture_cache[ck] = cube
    return cube


# =============================================================================
# HDR EXR → GGX-prefiltered cubemap pipeline (NifSkope-equivalent IBL)
# Ported from tools/sf_render_test.py.
# =============================================================================

_PREFILTER_ROUGHNESS_LEVELS = 6
_IRRADIANCE_FACE_SIZE = 32

_CUBE_FACE_DIR_GLSL = r"""
    vec3 faceDir(int f, vec2 uv) {
        vec2 t = uv * 2.0 - 1.0;
        if (f == 0) return normalize(vec3( 1.0, -t.y, -t.x));  // +X
        if (f == 1) return normalize(vec3(-1.0, -t.y,  t.x));  // -X
        if (f == 2) return normalize(vec3( t.x,  1.0,  t.y));  // +Y
        if (f == 3) return normalize(vec3( t.x, -1.0, -t.y));  // -Y
        if (f == 4) return normalize(vec3( t.x, -t.y,  1.0));  // +Z
        return          normalize(vec3(-t.x, -t.y, -1.0));     // -Z
    }
"""

_FULLSCREEN_QUAD_VERT = r"""
    #version 330 core
    in vec2 in_pos;
    out vec2 vUV;
    void main() {
        vUV = in_pos * 0.5 + 0.5;
        gl_Position = vec4(in_pos, 0.0, 1.0);
    }
"""


def load_exr_equirect(path: Path) -> "np.ndarray":
    """Load an EXR latlong/equirect into an HWC float32 RGB array."""
    import os
    os.environ.setdefault("OPENCV_IO_ENABLE_OPENEXR", "1")
    import cv2  # type: ignore
    img = cv2.imread(str(path), cv2.IMREAD_UNCHANGED | cv2.IMREAD_ANYDEPTH)
    if img is None:
        raise RuntimeError(f"Failed to load EXR: {path}")
    if img.ndim == 2:
        img = np.stack([img, img, img], axis=-1)
    if img.shape[-1] == 4:
        img = img[:, :, :3]
    img = img[:, :, ::-1].astype(np.float32)  # BGR -> RGB
    return np.ascontiguousarray(img)


def _render_cube_faces(ctx: moderngl.Context, prog: moderngl.Program,
                       face_size: int,
                       face_uniform_name: str = "face") -> moderngl.TextureCube:
    """Render a fullscreen-quad program once per face into a new cubemap."""
    cube = ctx.texture_cube((face_size, face_size), 4, dtype="f2")
    cube.filter = (moderngl.LINEAR, moderngl.LINEAR)
    staging = ctx.texture((face_size, face_size), 4, dtype="f2")
    fbo = ctx.framebuffer(color_attachments=[staging])
    quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype=np.float32).tobytes())
    vao = ctx.vertex_array(prog, [(quad, "2f", "in_pos")])
    for face in range(6):
        fbo.use()
        ctx.clear(0.0, 0.0, 0.0, 1.0)
        prog[face_uniform_name].value = face
        vao.render(moderngl.TRIANGLE_STRIP)
        cube.write(face, staging.read())
    fbo.release()
    vao.release()
    quad.release()
    staging.release()
    return cube


def equirect_to_cubemap(ctx: moderngl.Context, equirect: "np.ndarray",
                        face_size: int = 512) -> moderngl.TextureCube:
    """Render an equirect HDR image into a 6-face RGBA16F cubemap with mipmaps."""
    h, w, _ = equirect.shape
    rgba = np.ones((h, w, 4), dtype=np.float32)
    rgba[:, :, :3] = equirect
    src = ctx.texture((w, h), 4, rgba.astype(np.float16).tobytes(), dtype="f2")
    src.filter = (moderngl.LINEAR, moderngl.LINEAR)
    src.repeat_x = True
    src.repeat_y = False

    cube = ctx.texture_cube((face_size, face_size), 4, dtype="f2")
    cube.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    staging = ctx.texture((face_size, face_size), 4, dtype="f2")

    prog = ctx.program(
        vertex_shader=_FULLSCREEN_QUAD_VERT,
        fragment_shader=r"""
            #version 330 core
            in vec2 vUV;
            out vec4 fragColor;
            uniform sampler2D src;
            uniform int face;
            const float PI = 3.14159265359;
            """
            + _CUBE_FACE_DIR_GLSL
            + r"""
            void main() {
                vec3 d = faceDir(face, vUV);
                float u = 0.5 + atan(d.z, d.x) / (2.0 * PI);
                float v = 0.5 - asin(clamp(d.y, -1.0, 1.0)) / PI;
                fragColor = texture(src, vec2(u, v));
            }
        """,
    )
    quad = ctx.buffer(np.array([-1, -1, 1, -1, -1, 1, 1, 1], dtype=np.float32).tobytes())
    vao = ctx.vertex_array(prog, [(quad, "2f", "in_pos")])
    src.use(0)
    prog["src"].value = 0

    fbo = ctx.framebuffer(color_attachments=[staging])
    for face in range(6):
        fbo.use()
        ctx.clear(0.0, 0.0, 0.0, 1.0)
        prog["face"].value = face
        vao.render(moderngl.TRIANGLE_STRIP)
        cube.write(face, staging.read())
    fbo.release()

    cube.build_mipmaps()
    vao.release()
    quad.release()
    prog.release()
    src.release()
    staging.release()
    return cube


def prefilter_cubemap_ggx(ctx: moderngl.Context, src_cube: moderngl.TextureCube,
                          levels: int = _PREFILTER_ROUGHNESS_LEVELS,
                          base_size: int = 256) -> list[moderngl.TextureCube]:
    """Return one GGX-prefiltered cubemap per roughness step (Epic split-sum)."""
    src_cube.build_mipmaps()
    src_cube.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)

    prog = ctx.program(
        vertex_shader=_FULLSCREEN_QUAD_VERT,
        fragment_shader=r"""
            #version 330 core
            in vec2 vUV;
            out vec4 fragColor;
            uniform samplerCube srcCube;
            uniform int face;
            uniform float roughness;
            uniform float srcFaceSize;

            const float PI = 3.14159265359;
            const uint SAMPLES = 256u;
            """
            + _CUBE_FACE_DIR_GLSL
            + r"""

            float radicalInverse_VdC(uint bits) {
                bits = (bits << 16u) | (bits >> 16u);
                bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
                bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
                bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
                bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
                return float(bits) * 2.3283064365386963e-10;
            }
            vec2 hammersley(uint i, uint N) {
                return vec2(float(i)/float(N), radicalInverse_VdC(i));
            }
            vec3 importanceSampleGGX(vec2 Xi, vec3 N, float r) {
                float a = r * r;
                float phi = 2.0 * PI * Xi.x;
                float cosT = sqrt((1.0 - Xi.y) / (1.0 + (a*a - 1.0) * Xi.y));
                float sinT = sqrt(1.0 - cosT * cosT);
                vec3 H = vec3(cos(phi)*sinT, sin(phi)*sinT, cosT);
                vec3 up = abs(N.z) < 0.999 ? vec3(0,0,1) : vec3(1,0,0);
                vec3 T = normalize(cross(up, N));
                vec3 B = cross(N, T);
                return normalize(T*H.x + B*H.y + N*H.z);
            }
            float distributionGGX(float NdH, float r) {
                float a = r * r;
                float a2 = a * a;
                float denom = NdH * NdH * (a2 - 1.0) + 1.0;
                return a2 / (PI * denom * denom);
            }

            void main() {
                vec3 N = faceDir(face, vUV);
                vec3 R = N;
                vec3 V = R;
                vec3 prefiltered = vec3(0.0);
                float totalWeight = 0.0;
                for (uint i = 0u; i < SAMPLES; ++i) {
                    vec2 Xi = hammersley(i, SAMPLES);
                    vec3 H = importanceSampleGGX(Xi, N, roughness);
                    vec3 L = normalize(2.0 * dot(V, H) * H - V);
                    float NdL = max(dot(N, L), 0.0);
                    if (NdL > 0.0) {
                        float NdH = max(dot(N, H), 0.0001);
                        float HdV = max(dot(H, V), 0.0001);
                        float D = distributionGGX(NdH, max(roughness, 0.001));
                        float pdf = D * NdH / (4.0 * HdV) + 0.0001;
                        float saTexel  = 4.0 * PI / (6.0 * srcFaceSize * srcFaceSize);
                        float saSample = 1.0 / (float(SAMPLES) * pdf + 0.0001);
                        float lod = roughness == 0.0 ? 0.0
                                   : 0.5 * log2(saSample / saTexel);
                        prefiltered += textureLod(srcCube, L, lod).rgb * NdL;
                        totalWeight += NdL;
                    }
                }
                fragColor = vec4(prefiltered / max(totalWeight, 0.0001), 1.0);
            }
        """,
    )
    src_cube.use(0)
    prog["srcCube"].value = 0
    prog["srcFaceSize"].value = float(src_cube.size[0])

    out: list[moderngl.TextureCube] = []
    for i in range(levels):
        roughness = i / max(levels - 1, 1)
        size = max(base_size >> i, 8)
        prog["roughness"].value = roughness
        cube = _render_cube_faces(ctx, prog, size)
        cube.filter = (moderngl.LINEAR, moderngl.LINEAR)
        out.append(cube)
        _log.info("Prefilter cube level %d: size=%d roughness=%.2f",
                  i, size, roughness)

    prog.release()
    return out


def compute_irradiance_cube(ctx: moderngl.Context, src_cube: moderngl.TextureCube,
                            face_size: int = _IRRADIANCE_FACE_SIZE) -> moderngl.TextureCube:
    """Cosine-weighted hemispherical convolution of src_cube for diffuse IBL."""
    src_cube.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    prog = ctx.program(
        vertex_shader=_FULLSCREEN_QUAD_VERT,
        fragment_shader=r"""
            #version 330 core
            in vec2 vUV;
            out vec4 fragColor;
            uniform samplerCube srcCube;
            uniform int face;
            const float PI = 3.14159265359;
            """
            + _CUBE_FACE_DIR_GLSL
            + r"""
            void main() {
                vec3 N = faceDir(face, vUV);
                vec3 up = abs(N.y) < 0.999 ? vec3(0,1,0) : vec3(0,0,1);
                vec3 right = normalize(cross(up, N));
                up = normalize(cross(N, right));

                vec3 irradiance = vec3(0.0);
                float nSamples = 0.0;
                const float delta = 0.025;
                for (float phi = 0.0; phi < 2.0 * PI; phi += delta) {
                    for (float theta = 0.0; theta < 0.5 * PI; theta += delta) {
                        vec3 ts = vec3(sin(theta) * cos(phi),
                                       sin(theta) * sin(phi),
                                       cos(theta));
                        vec3 sd = ts.x * right + ts.y * up + ts.z * N;
                        irradiance += texture(srcCube, sd).rgb
                                      * cos(theta) * sin(theta);
                        nSamples += 1.0;
                    }
                }
                fragColor = vec4(PI * irradiance / nSamples, 1.0);
            }
        """,
    )
    src_cube.use(0)
    prog["srcCube"].value = 0
    cube = _render_cube_faces(ctx, prog, face_size)
    prog.release()
    return cube


# Compute shader: copies a samplerCube into one mip level of a destination
# imageCube, stitching the GGX-prefiltered roughness chain (one cube per step)
# into a single cube. starfield_default.frag's textureLod(envMap, R, lod) then
# reads the GGX response at every roughness instead of build_mipmaps()'s
# box-filtered downsamples. moderngl has no per-mip cube write or per-face FBO
# attach, but bind_to_image(level=N) + imageCube + compute works.
_CUBE_BLIT_TO_MIP_CS = """
#version 430
layout(local_size_x=8, local_size_y=8, local_size_z=1) in;
layout(rgba16f, binding=0) writeonly uniform imageCube dstImg;
uniform samplerCube srcCube;
uniform int mipSize;

vec3 faceDir(int f, vec2 uv) {
    vec2 t = uv * 2.0 - 1.0;
    if (f == 0) return normalize(vec3( 1.0, -t.y, -t.x));
    if (f == 1) return normalize(vec3(-1.0, -t.y,  t.x));
    if (f == 2) return normalize(vec3( t.x,  1.0,  t.y));
    if (f == 3) return normalize(vec3( t.x, -1.0, -t.y));
    if (f == 4) return normalize(vec3( t.x, -t.y,  1.0));
    return          normalize(vec3(-t.x, -t.y, -1.0));
}

void main() {
    ivec3 g = ivec3(gl_GlobalInvocationID);
    if (g.x >= mipSize || g.y >= mipSize || g.z >= 6) return;
    vec2 uv = (vec2(g.xy) + 0.5) / float(mipSize);
    vec3 d = faceDir(g.z, uv);
    imageStore(dstImg, ivec3(g.x, g.y, g.z), texture(srcCube, d));
}
"""


def _stitch_prefiltered_into_cube(
    ctx: moderngl.Context,
    prefiltered: list[moderngl.TextureCube],
    base_size: int,
) -> moderngl.TextureCube:
    """Copy each prefiltered roughness cube into the matching mip level of a
    single destination cube via a compute shader.

    `prefiltered[i]` is the GGX cube at roughness=i/(N-1), with face size
    `base_size >> i` (clamped to >=8 in prefilter_cubemap_ggx). The compute
    shader resamples its level 0 at the mip size; for a 256 base and the
    256/128/64/32/16/8 chain the sizes match, so it is a 1:1 copy.
    """
    dst = ctx.texture_cube((base_size, base_size), 4, dtype="f2")
    # Allocate the full mip chain. We overwrite each level via compute below;
    # build_mipmaps() is the simplest way to make moderngl allocate level >0.
    dst.build_mipmaps()

    cs = ctx.compute_shader(_CUBE_BLIT_TO_MIP_CS)
    cs["srcCube"].value = 0

    for i, src in enumerate(prefiltered):
        mip_size = max(base_size >> i, 1)
        src.filter = (moderngl.LINEAR, moderngl.LINEAR)
        src.use(0)
        dst.bind_to_image(0, read=False, write=True, level=i)
        cs["mipSize"].value = mip_size
        gx = (mip_size + 7) // 8
        gy = (mip_size + 7) // 8
        cs.run(group_x=gx, group_y=gy, group_z=6)

    cs.release()
    dst.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    return dst


def build_environment_cubes(
    ctx: moderngl.Context,
    exr_path: Path,
) -> "tuple[moderngl.TextureCube, moderngl.TextureCube] | None":
    """Build (specular_prefiltered, irradiance) cubes from an HDR equirect EXR.

    The specular cube has its mip chain populated with the GGX-prefiltered
    roughness levels (mip i = prefilter at roughness i/(N-1)) so the editor's
    `textureLod(envMap, R, lod)` shader call gets a true GGX-correct response
    at every roughness — not just box-filtered downsamples of the mirror cube.
    Returns None on failure (caller falls back to procedural).
    """
    ck = (id(ctx), f"texcube:env_pair:{exr_path}")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached  # type: ignore[return-value]
    try:
        equirect = load_exr_equirect(exr_path)
    except Exception as e:
        _log.warning("build_environment_cubes: failed to load %s: %s", exr_path, e)
        return None
    try:
        src_cube = equirect_to_cubemap(ctx, equirect, face_size=512)
        prefiltered = prefilter_cubemap_ggx(
            ctx, src_cube, levels=_PREFILTER_ROUGHNESS_LEVELS, base_size=256)
        spec_cube = _stitch_prefiltered_into_cube(ctx, prefiltered, base_size=256)
        irrad_cube = compute_irradiance_cube(ctx, src_cube)
        for c in prefiltered:
            c.release()
        src_cube.release()
    except Exception as e:
        _log.warning("build_environment_cubes: GL pipeline failed: %s", e)
        return None
    _texture_cache[ck] = (spec_cube, irrad_cube)
    _log.info("build_environment_cubes: built spec+irrad from %s", exr_path)
    return (spec_cube, irrad_cube)


def create_error_texture(ctx: moderngl.Context) -> moderngl.Texture:
    """1x1 magenta error texture."""
    ck = (id(ctx), "tex:error")
    cached = _texture_cache.get(ck)
    if cached is not None:
        return cached
    tex = ctx.texture((1, 1), 4, b'\xff\x00\xff\xff')
    _texture_cache[ck] = tex
    return tex


def purge_cached_texture(tex) -> None:
    """Remove *tex* from _texture_cache by identity so it can be safely released."""
    keys = [k for k, v in _texture_cache.items() if v is tex]
    for k in keys:
        del _texture_cache[k]


# ---------------------------------------------------------------------------
# Selection outline: back-face extrusion
# ---------------------------------------------------------------------------

_OUTLINE_EXTRUDE_VERT = """
#version 330
uniform mat4 u_mvp;
uniform float u_width;
in vec3 in_position;
in vec3 in_normal;
void main() {
    vec3 extruded = in_position + normalize(in_normal) * u_width;
    gl_Position = u_mvp * vec4(extruded, 1.0);
}
"""

_OUTLINE_EXTRUDE_FRAG = """
#version 330
uniform vec4 u_color;
out vec4 fragColor;
void main() {
    fragColor = u_color;
}
"""


def load_outline_shader(ctx: moderngl.Context) -> moderngl.Program:
    """Compile the selection outline shader (back-face extrusion)."""
    ck = (id(ctx), "inline:outline")
    cached = _program_cache.get(ck)
    if cached is not None:
        return cached
    prog = ctx.program(
        vertex_shader=_OUTLINE_EXTRUDE_VERT,
        fragment_shader=_OUTLINE_EXTRUDE_FRAG,
    )
    _program_cache[ck] = prog
    return prog
