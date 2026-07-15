"""Starfield render engine — direct port of tools/sf_render_test.py.

This is a parallel rendering path used by the editor whenever a Starfield NIF
is loaded. It bypasses the editor's shared shaders, Material/Mesh dataclasses,
material pipeline, and `_draw_node` walk entirely. Instead it builds its own
scene from the .nif file (using creation_lib.nif), parses .mat files itself, builds its
own VBO/VAO + GL program, and renders into the editor's existing FBO.

Why a parallel path: the editor's shared SF shader and material backend
diverged from NifSkope's stf_default math in subtle, hard-to-pin-down ways.
Rather than keep patching, we lift the working standalone wholesale.

Source of truth: tools/sf_render_test.py. If something looks wrong here,
diff against that file — it must stay byte-equivalent for the GLSL/math.

Editor integration:
    sf_scene = SFScene(ctx, nif_path, extracted_dir, exr_path)
    sf_scene.render(view, proj, light_dir_world,
                    tone_map_scale=0.1, brightness_scale=0.1,
                    env_intensity=8.0, light_intensity=1.0,
                    ambient=(0.7, 0.7, 0.7))
    sf_scene.release()  # when reloading or closing
"""
from __future__ import annotations

import json
import logging
import struct
import time
from dataclasses import dataclass, field
from pathlib import Path

import cv2  # OPENCV_IO_ENABLE_OPENEXR must be set in app.py before any cv2 import
import numpy as np
import moderngl

from creation_lib.nif.nif_file import NifFile
from creation_lib.dds import load_image as dds_load_image

_log = logging.getLogger("nif_editor.sf_engine")

MAX_LAYERS = 3            # shader-side per-mesh layer cap (NifSkope supports 6)
MAX_TEXTURES = 32         # texture unit budget for the lighting shader


# =============================================================================
# .mesh parsing (UV2-aware — verbatim from tools/sf_render_test.py:142-282)
# =============================================================================

@dataclass
class SFMesh:
    positions: np.ndarray
    normals: np.ndarray
    tangents: np.ndarray
    bitangents: np.ndarray
    colors: np.ndarray
    uv1: np.ndarray
    uv2: np.ndarray
    triangles: np.ndarray


def _decode_udec(packed: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    x = (packed & 0x3FF).astype(np.float32) / 511.0 - 1.0
    y = ((packed >> 10) & 0x3FF).astype(np.float32) / 511.0 - 1.0
    z = ((packed >> 20) & 0x3FF).astype(np.float32) / 511.0 - 1.0
    w = (packed >> 30) & 0x3
    return np.column_stack((x, y, z)), w


def parse_sf_mesh_uv2(data: bytes) -> SFMesh | None:
    if len(data) < 20:
        return None
    pos = 0
    version = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if version > 2:
        _log.warning("Unknown .mesh version %d", version)
        return None
    indices_size = struct.unpack_from("<I", data, pos)[0]; pos += 4
    num_tris = indices_size // 3
    if num_tris == 0:
        return None
    tris = np.frombuffer(data, dtype=np.uint16, count=indices_size, offset=pos)
    pos += indices_size * 2
    tris = tris.reshape(num_tris, 3).astype(np.uint32)

    scale, _weights_per_vert, num_pos = struct.unpack_from("<fII", data, pos); pos += 12
    if scale <= 0.0 or num_pos == 0:
        return None

    raw_pos = np.frombuffer(data, dtype=np.int16, count=num_pos * 3, offset=pos)
    pos += num_pos * 6
    positions = raw_pos.reshape(num_pos, 3).astype(np.float32) / 32767.0 * scale

    num_uv1 = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if num_uv1 > 0:
        uv1 = np.frombuffer(data, dtype=np.float16, count=num_uv1 * 2, offset=pos)
        pos += num_uv1 * 4
        uv1 = uv1.reshape(num_uv1, 2).astype(np.float32)
    else:
        uv1 = np.zeros((num_pos, 2), dtype=np.float32)

    num_uv2 = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if num_uv2 > 0:
        uv2 = np.frombuffer(data, dtype=np.float16, count=num_uv2 * 2, offset=pos)
        pos += num_uv2 * 4
        uv2 = uv2.reshape(num_uv2, 2).astype(np.float32)
    else:
        uv2 = uv1.copy() if uv1.shape[0] else np.zeros((num_pos, 2), dtype=np.float32)

    num_colors = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if num_colors > 0:
        raw = np.frombuffer(data, dtype=np.uint8, count=num_colors * 4, offset=pos)
        pos += num_colors * 4
        bgra = raw.reshape(num_colors, 4).astype(np.float32) / 255.0
        colors = bgra[:, [2, 1, 0, 3]].copy()
    else:
        colors = np.ones((num_pos, 4), dtype=np.float32)

    num_normals = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if num_normals > 0:
        raw = np.frombuffer(data, dtype=np.uint32, count=num_normals, offset=pos)
        pos += num_normals * 4
        normals, _ = _decode_udec(raw)
    else:
        normals = None

    num_tangents = struct.unpack_from("<I", data, pos)[0]; pos += 4
    tangents = bitangents = None
    if num_tangents > 0:
        raw = np.frombuffer(data, dtype=np.uint32, count=num_tangents, offset=pos)
        pos += num_tangents * 4
        tangents, w = _decode_udec(raw)
        sign = np.where(w >= 2, 1.0, -1.0).astype(np.float32)
        if normals is not None:
            bitangents = np.cross(normals, tangents) * sign[:, None]

    if normals is None:
        normals = _smooth_normals(positions, tris)
    if tangents is None:
        tangents = np.zeros_like(normals)
        bitangents = np.zeros_like(normals)
    if bitangents is None:
        bitangents = np.cross(normals, tangents)

    def _pad(arr, n, dim):
        if arr.shape[0] >= n:
            return arr[:n]
        out = np.zeros((n, dim), dtype=np.float32)
        out[:arr.shape[0]] = arr
        return out

    positions = positions.astype(np.float32)
    return SFMesh(
        positions=positions,
        normals=_pad(normals, num_pos, 3),
        tangents=_pad(tangents, num_pos, 3),
        bitangents=_pad(bitangents, num_pos, 3),
        colors=_pad(colors, num_pos, 4),
        uv1=_pad(uv1, num_pos, 2),
        uv2=_pad(uv2, num_pos, 2),
        triangles=tris,
    )


def _smooth_normals(positions, tris):
    normals = np.zeros_like(positions)
    v0 = positions[tris[:, 0]]
    v1 = positions[tris[:, 1]]
    v2 = positions[tris[:, 2]]
    face = np.cross(v1 - v0, v2 - v0)
    for i in range(3):
        np.add.at(normals, tris[:, i], face)
    lengths = np.linalg.norm(normals, axis=1, keepdims=True)
    lengths[lengths < 1e-8] = 1.0
    return (normals / lengths).astype(np.float32)


# =============================================================================
# .mat parsing — verbatim from tools/sf_render_test.py:289-512
# =============================================================================

@dataclass
class LayerDef:
    albedo_path: str = ""
    normal_path: str = ""
    rough_path: str = ""
    metal_path: str = ""
    ao_path: str = ""
    opacity_path: str = ""
    emissive_path: str = ""
    height_path: str = ""
    uv_channel: int = 0
    uv_scale: tuple[float, float] = (1.0, 1.0)
    uv_offset: tuple[float, float] = (0.0, 0.0)
    tint: tuple[float, float, float] = (1.0, 1.0, 1.0)
    normal_intensity: float = 1.0


@dataclass
class BlenderDef:
    mask_path: str = ""
    mode: int = 0
    vc_channel: int = -1
    mask_intensity: float = 1.0
    blend_albedo: bool = False
    blend_normal: bool = True
    blend_metal: bool = False
    blend_rough: bool = False
    blend_ao: bool = False
    additive_normal: bool = False


@dataclass
class MatDef:
    filename: str = ""
    layers: list[LayerDef] = field(default_factory=list)
    blenders: list[BlenderDef] = field(default_factory=list)
    is_decal: bool = False
    has_opacity: bool = False
    alpha_test_threshold: float = 0.5


_BLEND_MODE_INT = {
    "linear": 0, "lerp": 0,
    "additive": 1, "add": 1,
    "positioncontrast": 2, "position_contrast": 2,
    "none": 3,
    "charactercombine": 4, "character_combine": 4,
    "skin": 5,
}
_VC_CHANNEL_INT = {"red": 0, "r": 0, "green": 1, "g": 1, "blue": 2, "b": 2,
                   "alpha": 3, "a": 3}


def parse_mat(path: Path) -> MatDef | None:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except Exception as e:
        _log.warning("parse_mat %s: %s", path, e)
        return None
    summary = raw.get("Summary", {})
    if not summary:
        _log.warning("parse_mat %s: no Summary section", path)
        return None

    mat = MatDef(filename=raw.get("Filename") or str(path.name))
    for i in range(1, 7):
        lj = summary.get(f"Layer{i}")
        if lj is None:
            continue
        layer = _parse_summary_layer(lj)
        mat.layers.append(layer)
        if i > 1:
            bj = lj.get("Blender") or {}
            mat.blenders.append(_parse_summary_blender(bj))

    _apply_objects_section(raw, mat)

    filename_l = (mat.filename or str(path)).lower()
    parent_imports = [str(p).lower() for p in (raw.get("Import") or [])]
    parent_imports.append(str(summary.get("Layer1", {}).get("Parent", "")).lower())
    looks_like_decal = (
        "decal" in filename_l
        or any("decal" in p for p in parent_imports)
    )
    if looks_like_decal:
        mat.is_decal = True
        mat.has_opacity = True

    return mat


def _parse_summary_layer(lj: dict) -> LayerDef:
    layer = LayerDef()
    textures = lj.get("Textures", {}) or {}
    for name, info in textures.items():
        if not isinstance(info, dict):
            continue
        if info.get("UseReplacement", False):
            continue
        fp = info.get("File", "")
        if not fp:
            continue
        key = name.lower()
        normalized = fp.replace("\\", "/")
        if key == "albedo":
            layer.albedo_path = normalized
        elif key == "normal":
            layer.normal_path = normalized
        elif key == "roughness":
            layer.rough_path = normalized
        elif key == "metalness":
            layer.metal_path = normalized
        elif key in ("ao", "ambientocclusion"):
            layer.ao_path = normalized
        elif key == "opacity":
            layer.opacity_path = normalized
        elif key == "emissive":
            layer.emissive_path = normalized
        elif key == "height":
            layer.height_path = normalized

    uv = lj.get("UVStream") or {}
    layer.uv_channel = int(uv.get("Channel", 0) or 0)
    scale = uv.get("Scale", {}) or {}
    offset = uv.get("Offset", {}) or {}
    layer.uv_scale = (float(scale.get("x", 1.0)), float(scale.get("y", 1.0)))
    layer.uv_offset = (float(offset.get("x", 0.0)), float(offset.get("y", 0.0)))

    tint_json = lj.get("TintColor") or {}
    if isinstance(tint_json, dict) and tint_json:
        layer.tint = (
            float(tint_json.get("R", tint_json.get("x", 1.0))),
            float(tint_json.get("G", tint_json.get("y", 1.0))),
            float(tint_json.get("B", tint_json.get("z", 1.0))),
        )
    return layer


def _parse_summary_blender(bj: dict) -> BlenderDef:
    blend = BlenderDef()
    mask = bj.get("BlendMask") or {}
    if isinstance(mask, dict) and not mask.get("UseReplacement", False):
        blend.mask_path = (mask.get("File", "") or "").replace("\\", "/")
    mode_raw = str(bj.get("BlendMode", "linear")).lower().replace(" ", "")
    blend.mode = _BLEND_MODE_INT.get(mode_raw, 0)
    vc_raw = str(bj.get("VertexColorChannel", "")).lower()
    blend.vc_channel = _VC_CHANNEL_INT.get(vc_raw, -1)
    blend.mask_intensity = float(bj.get("MaskIntensity", 1.0))
    blend.blend_albedo = bool(bj.get("BlendTextureAlbedo", False))
    blend.blend_normal = bool(bj.get("BlendTextureNormal", True))
    blend.blend_metal = bool(bj.get("BlendTextureMetal", False))
    blend.blend_rough = bool(bj.get("BlendTextureRoughness", False))
    blend.blend_ao = bool(bj.get("BlendTextureAo", False))
    blend.additive_normal = bool(bj.get("BlendTextureAdditiveNormal", False))
    return blend


def _apply_objects_section(raw: dict, mat: MatDef):
    pending_float: list[float] = []
    for obj in raw.get("Objects", []) or []:
        if not isinstance(obj, dict):
            continue
        components = obj.get("Components") or []
        is_texture_set = False
        float_param = None
        for c in components:
            if not isinstance(c, dict):
                continue
            t = c.get("Type", "")
            d = c.get("Data") or {}
            if t == "BSMaterial::MRTextureFile":
                is_texture_set = True
            elif t == "BSMaterial::MaterialParamFloat":
                try:
                    float_param = float(d.get("Value", "1.0"))
                except Exception:
                    float_param = 1.0
            elif t == "BSMaterial::DecalSettingsComponent":
                if str(d.get("IsDecal", "false")).lower() == "true":
                    mat.is_decal = True
            elif t == "BSMaterial::AlphaSettingsComponent":
                if str(d.get("HasOpacity", "false")).lower() == "true":
                    mat.has_opacity = True
                try:
                    mat.alpha_test_threshold = float(d.get("AlphaTestThreshold", 0.5))
                except Exception:
                    pass
        if is_texture_set and float_param is not None:
            pending_float.append(float_param)

    for i, val in enumerate(pending_float):
        if i < len(mat.layers):
            mat.layers[i].normal_intensity = val


# =============================================================================
# Texture / mesh path resolution — extracted_dir is injected by SFScene
# =============================================================================

def resolve_texture(rel_path: str, extracted_dir: Path) -> Path | None:
    if not rel_path:
        return None
    p = rel_path.replace("\\", "/").lstrip("/")
    if p.lower().startswith("data/"):
        p = p[5:]
    candidate = extracted_dir / p
    if candidate.exists():
        return candidate
    parent = (extracted_dir / p).parent
    if parent.exists():
        name = Path(p).name.lower()
        for child in parent.iterdir():
            if child.name.lower() == name:
                return child
    return None


def resolve_mesh(mesh_path: str, nif_path: Path, extracted_dir: Path) -> bytes | None:
    rel = f"geometries/{mesh_path.replace(chr(92), '/')}.mesh"
    for base in (extracted_dir, nif_path.parent, nif_path.parent.parent):
        candidate = base / rel
        if candidate.exists():
            try:
                return candidate.read_bytes()
            except OSError as e:
                _log.debug("read %s: %s", candidate, e)
    return None


def load_dds_to_gl(ctx: moderngl.Context, path: Path) -> moderngl.Texture | None:
    try:
        img = dds_load_image(str(path))
    except Exception as e:
        _log.warning("dds_load_image %s: %s", path, e)
        return None
    arr = np.array(img.convert("RGBA"), dtype=np.uint8)
    h, w = arr.shape[:2]
    tex = ctx.texture((w, h), 4, arr.tobytes())
    tex.build_mipmaps()
    tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
    tex.anisotropy = 16.0
    tex.repeat_x = True
    tex.repeat_y = True
    return tex


# =============================================================================
# EXR → cubemap — verbatim from tools/sf_render_test.py:574-869
# =============================================================================

def load_exr_equirect(path: Path) -> np.ndarray:
    img = cv2.imread(str(path), cv2.IMREAD_UNCHANGED | cv2.IMREAD_ANYDEPTH)
    if img is None:
        raise RuntimeError(f"Failed to load EXR: {path}")
    if img.ndim == 2:
        img = np.stack([img, img, img], axis=-1)
    if img.shape[-1] == 4:
        img = img[:, :, :3]
    img = img[:, :, ::-1].astype(np.float32)
    return np.ascontiguousarray(img)


_CUBE_FACE_DIR_GLSL = r"""
    vec3 faceDir(int f, vec2 uv) {
        vec2 t = uv * 2.0 - 1.0;
        if (f == 0) return normalize(vec3( 1.0, -t.y, -t.x));
        if (f == 1) return normalize(vec3(-1.0, -t.y,  t.x));
        if (f == 2) return normalize(vec3( t.x,  1.0,  t.y));
        if (f == 3) return normalize(vec3( t.x, -1.0, -t.y));
        if (f == 4) return normalize(vec3( t.x, -t.y,  1.0));
        return          normalize(vec3(-t.x, -t.y, -1.0));
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

_PREFILTER_ROUGHNESS_LEVELS = 6
_IRRADIANCE_FACE_SIZE = 32


def _render_cube_faces(ctx: moderngl.Context, prog: moderngl.Program,
                       face_size: int, face_uniform_name: str = "face") -> moderngl.TextureCube:
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


def prefilter_cubemap_ggx(ctx: moderngl.Context, src_cube: moderngl.TextureCube,
                          levels: int = _PREFILTER_ROUGHNESS_LEVELS,
                          base_size: int = 256) -> list[moderngl.TextureCube]:
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
            """ + _CUBE_FACE_DIR_GLSL + r"""
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
        _log.info("Prefilter cube level %d: size=%d roughness=%.2f", i, size, roughness)

    prog.release()
    return out


def compute_irradiance_cube(ctx: moderngl.Context, src_cube: moderngl.TextureCube,
                            face_size: int = _IRRADIANCE_FACE_SIZE) -> moderngl.TextureCube:
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
            """ + _CUBE_FACE_DIR_GLSL + r"""
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
                        irradiance += texture(srcCube, sd).rgb * cos(theta) * sin(theta);
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


def equirect_to_cubemap(ctx: moderngl.Context, equirect: np.ndarray,
                        face_size: int = 512) -> moderngl.TextureCube:
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
        vertex_shader=r"""
            #version 330 core
            in vec2 in_pos;
            out vec2 vUV;
            void main() {
                vUV = in_pos * 0.5 + 0.5;
                gl_Position = vec4(in_pos, 0.0, 1.0);
            }
        """,
        fragment_shader=r"""
            #version 330 core
            in vec2 vUV;
            out vec4 fragColor;
            uniform sampler2D src;
            uniform int face;
            const float PI = 3.14159265359;
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


# =============================================================================
# Shader sources — verbatim from tools/sf_render_test.py:876-1221
# =============================================================================

VERT_SRC = r"""
#version 330 core

uniform mat4 uView;
uniform mat4 uProj;
uniform mat4 uModel;
uniform mat3 uNormalMat;

layout (location = 0) in vec3 inPos;
layout (location = 1) in vec3 inNormal;
layout (location = 2) in vec3 inTangent;
layout (location = 3) in vec3 inBitangent;
layout (location = 4) in vec4 inColor;
layout (location = 5) in vec2 inUV1;
layout (location = 6) in vec2 inUV2;

out vec4 vTexCoord;
out vec4 vColor;
out mat3 vBtn;
out vec3 vViewDir;
out vec3 vLightDir;
// World-space position passed to the fragment shader so calcShadow()
// can transform it into light space and sample the shadow map. Added
// in Phase 4 of the scene-backend refactor (SF shadow casters).
out vec3 vPosWorld;

uniform vec3 uLightDirWorld;

void main() {
    vec4 worldPos = uModel * vec4(inPos, 1.0);
    vec4 viewPos  = uView  * worldPos;
    gl_Position   = uProj  * viewPos;

    vec3 N = normalize(uNormalMat * inNormal);
    vec3 T = normalize(uNormalMat * inTangent);
    vec3 B = normalize(uNormalMat * inBitangent);
    vBtn = mat3(B, T, N);

    vViewDir  = -viewPos.xyz;
    vLightDir = mat3(uView) * uLightDirWorld;
    vTexCoord = vec4(inUV1, inUV2);
    vColor    = inColor;
    vPosWorld = worldPos.xyz;
}
"""

FRAG_SRC = r"""
#version 330 core

#define MAX_LAYERS 3
#define MAX_TEX_UNITS 32

in vec4 vTexCoord;
in vec4 vColor;
in mat3 vBtn;
in vec3 vViewDir;
in vec3 vLightDir;
in vec3 vPosWorld;  // world-space position for shadow sampling (Phase 4)

// MRT: color attachment 0 = lit color, attachment 1 = view-space normal
// (RGBA16F, encoded *0.5+0.5). Matches the FO4 path so _render_ssao can
// sample _fbo_normal_tex unchanged.
layout(location = 0) out vec4 fragColor;
layout(location = 1) out vec4 fragNormal;

uniform float mrtEnabled;

// --- Shadow map sampling (Phase 4 of scene-backend refactor) ---
// Mirrors ui/editor/shaders/includes/lighting.glsl:6-34 verbatim so the
// SF wholesale-port shader matches the FO4 shadow look. Shadow only
// attenuates the direct sun (key light + its specular); ambient/IBL,
// fill, and mirror lights stay unshadowed.
uniform float     uShadowEnabled;
uniform mat4      uLightSpaceMatrix;
uniform sampler2D uShadowMap;

float calcShadow(vec3 worldPos)
{
    if (uShadowEnabled < 0.5)
        return 1.0;

    vec4 lsPos = uLightSpaceMatrix * vec4(worldPos, 1.0);
    vec3 projCoords = lsPos.xyz / lsPos.w;
    projCoords = projCoords * 0.5 + 0.5;

    if (projCoords.x < 0.0 || projCoords.x > 1.0 ||
        projCoords.y < 0.0 || projCoords.y > 1.0 ||
        projCoords.z > 1.0)
        return 1.0;

    float currentDepth = projCoords.z;
    float bias = 0.002;

    float shadow = 0.0;
    vec2 texelSize = 1.0 / vec2(textureSize(uShadowMap, 0));
    for (int x = -1; x <= 1; x++) {
        for (int y = -1; y <= 1; y++) {
            float pcfDepth = texture(uShadowMap, projCoords.xy + vec2(x, y) * texelSize).r;
            shadow += currentDepth - bias > pcfDepth ? 0.0 : 1.0;
        }
    }
    shadow /= 9.0;

    return mix(0.3, 1.0, shadow);
}

uniform sampler2D uTextures[MAX_TEX_UNITS];
uniform sampler2D uBrdfLUT;

uniform samplerCube uCubeMip0;
uniform samplerCube uCubeMip1;
uniform samplerCube uCubeMip2;
uniform samplerCube uCubeMip3;
uniform samplerCube uCubeMip4;
uniform samplerCube uCubeMip5;
uniform samplerCube uCubeIrradiance;
uniform bool  uHasCubeMap;
uniform bool  uHasSpecular;
uniform float uEnvLodBias;

uniform int   uNumLayers;
uniform int   uLayerAlbedoUnit[MAX_LAYERS];
uniform int   uLayerNormalUnit[MAX_LAYERS];
uniform int   uLayerRoughUnit [MAX_LAYERS];
uniform int   uLayerMetalUnit [MAX_LAYERS];
uniform int   uLayerAoUnit    [MAX_LAYERS];
uniform int   uLayerOpacityUnit[MAX_LAYERS];
uniform bool  uHasOpacity;
uniform float uAlphaThreshold;
uniform bool  uAlphaTest;

uniform vec2  uLayerUvScale   [MAX_LAYERS];
uniform vec2  uLayerUvOffset  [MAX_LAYERS];
uniform int   uLayerUvChannel [MAX_LAYERS];
uniform vec3  uLayerTint      [MAX_LAYERS];
uniform float uLayerNormalScale[MAX_LAYERS];

uniform int   uBlenderCount;
uniform int   uBlenderMaskUnit[MAX_LAYERS];
uniform int   uBlenderMode    [MAX_LAYERS];
uniform int   uBlenderVcChan  [MAX_LAYERS];
uniform float uBlenderMaskInt [MAX_LAYERS];
uniform bool  uBlendAlbedo    [MAX_LAYERS];
uniform bool  uBlendNormal    [MAX_LAYERS];
uniform bool  uBlendMetal     [MAX_LAYERS];
uniform bool  uBlendRough     [MAX_LAYERS];
uniform bool  uBlendAO        [MAX_LAYERS];
uniform bool  uBlendAddNormal [MAX_LAYERS];

uniform vec3  uLightSourceDiffuse;
uniform vec3  uLightSourceAmbient;
uniform float uToneMapScale;
uniform float uBrightnessScale;
uniform float uEnvIntensity;
uniform mat3  uEnvRotation;

// Scene-menu toggles (0.0 = off, 1.0 = on). Match FO4 shader names/semantics.
uniform float uToggleDiffuse;
uniform float uToggleNormal;
uniform float uToggleSpec;
uniform float uToggleLighting;
uniform float uToggleVertexColor;
uniform float uToggleEnvMap;

// Debug PBR tuning sliders. Defaults documented in renderer._setup_fo4_uniforms.
uniform float uDbgEnvBoost;        // 0..5  (neutral 1)
uniform float uDbgMetalF0;         // 0..1  (neutral 0.9)
uniform float uDbgDiffuseBleed;    // 0..2  (neutral 0)
uniform float uDbgExposure;        // 0.5..12 (neutral 4.23)
uniform float uDbgSpecBoost;       // 0..5  (neutral 1)
uniform float uDbgAmbientBoost;    // 0..5  (neutral 1)

// Extra lights (fill + mirror). Directions in VIEW space (CPU-side converted
// from world so we don't need to touch the vertex shader).
uniform vec3  uFillDirView;
uniform vec3  uFillCol;
uniform vec3  uMirrorDirView;
uniform vec3  uMirrorCol;
uniform float uMirrorEnabled;

float LightingFuncGGX_REF(float NdotH, float NdotL, float NdotV, float roughness) {
    float alpha = roughness * roughness;
    float alphaSqr = alpha * alpha;
    float denom = NdotH * NdotH;
    denom = (denom * alphaSqr) + (1.0 - denom);
    float D = alphaSqr / (denom * denom * 4.0);
    float k = alpha * 0.5;
    float G = NdotL / (mix(NdotL, 1.0, k) * mix(NdotV, 1.0, k));
    return D * G;
}

vec3 sampleCubeMip(samplerCube c, vec3 dir) { return texture(c, dir).rgb; }

vec3 sampleSpecCube(vec3 dir, float rough) {
    float idx = clamp(rough + uEnvLodBias, 0.0, 1.0) * 5.0;
    int lo = int(floor(idx));
    float t = idx - float(lo);
    vec3 a, b;
    if      (lo <= 0) { a = sampleCubeMip(uCubeMip0, dir); b = sampleCubeMip(uCubeMip1, dir); }
    else if (lo == 1) { a = sampleCubeMip(uCubeMip1, dir); b = sampleCubeMip(uCubeMip2, dir); }
    else if (lo == 2) { a = sampleCubeMip(uCubeMip2, dir); b = sampleCubeMip(uCubeMip3, dir); }
    else if (lo == 3) { a = sampleCubeMip(uCubeMip3, dir); b = sampleCubeMip(uCubeMip4, dir); }
    else              { a = sampleCubeMip(uCubeMip4, dir); b = sampleCubeMip(uCubeMip5, dir); }
    return mix(a, b, t);
}

vec3 sfTonemap(vec3 x, float y) {
    float a = 0.15, b = 0.50, c = 0.10, d = 0.20, e = 0.02, f = 0.30;
    vec3 z = x * (y * 4.22978723);
    z = (z * (a * z + b * c) + d * e) / (z * (a * z + b) + d * f) - e / f;
    return z / (y * 0.93333333);
}

vec4 sampleUnit(int unit, vec2 uv) {
    switch (unit) {
        case  0: return texture(uTextures[ 0], uv);
        case  1: return texture(uTextures[ 1], uv);
        case  2: return texture(uTextures[ 2], uv);
        case  3: return texture(uTextures[ 3], uv);
        case  4: return texture(uTextures[ 4], uv);
        case  5: return texture(uTextures[ 5], uv);
        case  6: return texture(uTextures[ 6], uv);
        case  7: return texture(uTextures[ 7], uv);
        case  8: return texture(uTextures[ 8], uv);
        case  9: return texture(uTextures[ 9], uv);
        case 10: return texture(uTextures[10], uv);
        case 11: return texture(uTextures[11], uv);
        case 12: return texture(uTextures[12], uv);
        case 13: return texture(uTextures[13], uv);
        case 14: return texture(uTextures[14], uv);
        case 15: return texture(uTextures[15], uv);
        case 16: return texture(uTextures[16], uv);
        case 17: return texture(uTextures[17], uv);
        case 18: return texture(uTextures[18], uv);
        case 19: return texture(uTextures[19], uv);
        case 20: return texture(uTextures[20], uv);
        case 21: return texture(uTextures[21], uv);
        case 22: return texture(uTextures[22], uv);
        case 23: return texture(uTextures[23], uv);
        case 24: return texture(uTextures[24], uv);
        case 25: return texture(uTextures[25], uv);
        case 26: return texture(uTextures[26], uv);
        case 27: return texture(uTextures[27], uv);
        case 28: return texture(uTextures[28], uv);
        case 29: return texture(uTextures[29], uv);
        case 30: return texture(uTextures[30], uv);
        default: return texture(uTextures[31], uv);
    }
}

vec2 layerUV(int i) {
    vec2 base = (uLayerUvChannel[i] == 0) ? vTexCoord.st : vTexCoord.pq;
    return base * uLayerUvScale[i] + uLayerUvOffset[i];
}

float getBlenderMask(int i) {
    float r = 1.0;
    if (uBlenderMaskUnit[i] >= 0)
        r = sampleUnit(uBlenderMaskUnit[i], layerUV(0)).r;
    if (uBlenderVcChan[i] >= 0)
        r *= vColor[uBlenderVcChan[i]];
    return r * uBlenderMaskInt[i];
}

void main() {
    vec2 uv0 = layerUV(0);
    vec3 baseMap = uLayerTint[0];
    if (uLayerAlbedoUnit[0] >= 0)
        baseMap = mix(vec3(1.0), sampleUnit(uLayerAlbedoUnit[0], uv0).rgb, uToggleDiffuse)
                * uLayerTint[0];

    vec3 normal = vec3(0.0, 0.0, 1.0);
    if (uLayerNormalUnit[0] >= 0 && uToggleNormal > 0.5) {
        vec2 nrg = sampleUnit(uLayerNormalUnit[0], uv0).rg * 2.0 - 1.0;
        nrg *= uLayerNormalScale[0];
        normal.rg = nrg;
        normal.b = sqrt(max(1.0 - dot(nrg, nrg), 0.0));
    }

    vec3 pbrMap = vec3(1.0, 0.0, 1.0);
    if (uLayerRoughUnit[0] >= 0)
        pbrMap.r = sampleUnit(uLayerRoughUnit[0], uv0).r;
    if (uLayerMetalUnit[0] >= 0)
        pbrMap.g = sampleUnit(uLayerMetalUnit[0], uv0).r;
    if (uLayerAoUnit[0] >= 0)
        pbrMap.b = sampleUnit(uLayerAoUnit[0], uv0).r;

    int numLayers = min(uNumLayers, MAX_LAYERS);
    for (int i = 1; i < MAX_LAYERS; ++i) {
        if (i >= numLayers) break;
        int bi = i - 1;
        int mode = uBlenderMode[bi];
        if (mode == 3) continue;

        vec2 uvI = layerUV(i);

        vec3 layerBase = uLayerTint[i];
        if (uLayerAlbedoUnit[i] >= 0)
            layerBase = sampleUnit(uLayerAlbedoUnit[i], uvI).rgb * uLayerTint[i];

        vec3 layerNormal = vec3(0.0, 0.0, 1.0);
        if (uLayerNormalUnit[i] >= 0) {
            vec2 nrg = sampleUnit(uLayerNormalUnit[i], uvI).rg * 2.0 - 1.0;
            nrg *= uLayerNormalScale[i];
            layerNormal.rg = nrg;
            layerNormal.b = sqrt(max(1.0 - dot(nrg, nrg), 0.0));
        }
        vec3 layerPBR = pbrMap;
        if (uLayerRoughUnit[i] >= 0) layerPBR.r = sampleUnit(uLayerRoughUnit[i], uvI).r;
        if (uLayerMetalUnit[i] >= 0) layerPBR.g = sampleUnit(uLayerMetalUnit[i], uvI).r;
        if (uLayerAoUnit[i]   >= 0)  layerPBR.b = sampleUnit(uLayerAoUnit[i],   uvI).r;

        float srcMask = clamp(getBlenderMask(bi), 0.0, 1.0);

        if (uBlendAlbedo[bi])
            baseMap = mix(baseMap, layerBase, srcMask);
        if (uBlendMetal[bi])
            pbrMap.g = mix(pbrMap.g, layerPBR.g, srcMask);
        if (uBlendRough[bi])
            pbrMap.r = mix(pbrMap.r, layerPBR.r, srcMask);
        if (uBlendAO[bi])
            pbrMap.b = mix(pbrMap.b, layerPBR.b, srcMask);
        if (uBlendNormal[bi]) {
            if (uBlendAddNormal[bi]) {
                normal.rg += layerNormal.rg * srcMask;
                normal.b = sqrt(max(1.0 - dot(normal.rg, normal.rg), 0.0));
            } else {
                normal = normalize(mix(normal, layerNormal, srcMask));
            }
        }
    }

    mat3 btn = mat3(normalize(vBtn[0]), normalize(vBtn[1]), normalize(vBtn[2]));
    if (!gl_FrontFacing) normal.z *= -1.0;
    vec3 N = normalize(btn * normal);

    // Vertex color contribution (gated by Scene-menu toggle).
    baseMap *= mix(vec3(1.0), vColor.rgb, uToggleVertexColor);

    vec3 V = normalize(vViewDir);
    vec3 L = normalize(vLightDir);
    vec3 R = reflect(-V, N);
    vec3 H = normalize(L + V);

    float NdotL  = dot(N, L);
    float NdotL0 = max(NdotL, 0.0);
    float NdotH  = clamp(dot(N, H), 0.0, 1.0);
    float NdotV  = abs(dot(N, V));
    float LdotH  = dot(L, H);

    vec3 reflectedWS = uEnvRotation * R;
    vec3 normalWS    = uEnvRotation * N;

    vec3 f0     = mix(vec3(0.04), baseMap, pbrMap.g * uDbgMetalF0);
    vec3 albedo = baseMap * (1.0 - pbrMap.g);
    // Diffuse bleed: let metal surfaces retain a fraction of albedo (debug aid).
    albedo += baseMap * pbrMap.g * uDbgDiffuseBleed;

    float roughness = pbrMap.r;
    vec3 spec = uLightSourceDiffuse;
    spec *= LightingFuncGGX_REF(NdotH, NdotL0, NdotV, clamp(roughness, 0.045, 0.95));

    vec3 diffuse = vec3(NdotL0);
    vec2 fDirect = textureLod(uBrdfLUT, vec2(LdotH, NdotL0), 0.0).ba;
    spec *= mix(f0, vec3(1.0), fDirect.x);
    vec4 envLUT = textureLod(uBrdfLUT, vec2(NdotV, roughness), 0.0);
    vec2 fDiff = vec2(fDirect.y, envLUT.b);
    fDiff = fDiff * (LdotH * LdotH * roughness * 2.0 - 0.5) + 1.0;
    diffuse *= (vec3(1.0) - f0) * fDiff.x * fDiff.y;

    vec3 refl = vec3(0.0);
    vec3 ambient = uLightSourceAmbient;
    if (uHasCubeMap) {
        refl = sampleSpecCube(reflectedWS, clamp(roughness, 0.0, 1.0)) * uEnvIntensity;
        refl *= ambient;
        ambient *= sampleCubeMip(uCubeIrradiance, normalWS) * uEnvIntensity;
    } else {
        ambient *= 0.08;
        refl = ambient;
    }
    vec3 f = mix(f0, vec3(1.0), envLUT.r);
    if (!uHasSpecular) {
        albedo = baseMap;
        diffuse = vec3(NdotL0);
        spec = vec3(0.0);
        f = vec3(0.0);
    } else {
        float fDiffEnv = envLUT.b * ((NdotV + 1.0) * roughness - 0.5) + 1.0;
        ambient *= (vec3(1.0) - f0) * fDiffEnv;
    }
    float ao = pbrMap.b;
    float specOcc = max((ao - 1.0) * ((NdotV * 1.125 - 2.625) * NdotV + 2.5) + 1.0, 0.0);
    refl *= f * envLUT.g;

    // Fill + mirror lights (diffuse only — matches FO4 behavior for fill/mirror).
    float NdotL_fill = max(dot(N, -normalize(uFillDirView)), 0.0);
    vec3  fillC      = NdotL_fill * uFillCol;
    float NdotL_mir  = max(dot(N, -normalize(uMirrorDirView)), 0.0);
    vec3  mirrorC    = NdotL_mir * uMirrorCol * uMirrorEnabled;

    vec3 amb = ambient * uDbgAmbientBoost;
    vec3 reflB = refl * uDbgEnvBoost * uToggleEnvMap;
    vec3 specB = spec * uDbgSpecBoost * uToggleSpec;

    // Shadow attenuates the direct sun only (key-light diffuse + specB).
    // Ambient/IBL/fill/mirror stay unshadowed so the scene doesn't go
    // pitch black when key-light fragments are occluded.
    float shadowF = calcShadow(vPosWorld);
    vec3 litColor = (diffuse * uLightSourceDiffuse * shadowF + fillC + mirrorC + amb) * albedo * ao;
    litColor += (specB * shadowF + reflB) * specOcc;

    // Lighting toggle: fall back to raw albedo when lighting is disabled.
    vec3 color = mix(baseMap, litColor, uToggleLighting);

    // Exposure: uDbgExposure is normalized so that 4.23 (FO4 default) is neutral.
    color *= (uDbgExposure / 4.23);

    color = sfTonemap(color * uBrightnessScale, uToneMapScale);

    float alpha = 1.0;
    if (uHasOpacity && uLayerOpacityUnit[0] >= 0) {
        alpha = sampleUnit(uLayerOpacityUnit[0], layerUV(0)).r;
    }
    if (uAlphaTest && alpha < uAlphaThreshold)
        discard;
    fragColor = vec4(color, alpha);

    // SSAO MRT output: view-space normal encoded to [0,1]. N is already in
    // view space (vBtn is built in view space by the vertex shader), so it
    // can be written directly. When mrtEnabled is off, write zero so
    // _render_ssao gets a deterministic empty mask if it samples this slot.
    if (mrtEnabled > 0.5) {
        fragNormal = vec4(N * 0.5 + 0.5, 1.0);
    } else {
        fragNormal = vec4(0.0);
    }
}
"""


# =============================================================================
# BRDF LUT — verbatim from tools/sf_render_test.py:1229-1318
# =============================================================================

def _fresnel_n_np(x: np.ndarray) -> np.ndarray:
    y = ((((x * -1.03202882 + 3.64690610) * x - 5.46708684) * x
          + 4.84513712) * x - 2.99292756) * x + 1.0
    return y * y


def _hammersley(i: np.ndarray, n: int) -> np.ndarray:
    bits = i.astype(np.uint32)
    bits = (bits << 16) | (bits >> 16)
    bits = ((bits & 0x55555555) << 1) | ((bits & 0xAAAAAAAA) >> 1)
    bits = ((bits & 0x33333333) << 2) | ((bits & 0xCCCCCCCC) >> 2)
    bits = ((bits & 0x0F0F0F0F) << 4) | ((bits & 0xF0F0F0F0) >> 4)
    bits = ((bits & 0x00FF00FF) << 8) | ((bits & 0xFF00FF00) >> 8)
    vdc = bits.astype(np.float64) * 2.3283064365386963e-10
    return np.stack([i.astype(np.float64) / n, vdc], axis=-1)


def generate_sf_brdf_lut(ctx: moderngl.Context, size: int = 512,
                         n_samples: int = 1024) -> moderngl.Texture:
    t0 = time.time()
    xs = (np.arange(size, dtype=np.float64) + 0.5) / size
    ys = (np.arange(size, dtype=np.float64) + 0.5) / size
    hammer = _hammersley(np.arange(n_samples), n_samples)
    out = np.zeros((size, size, 4), dtype=np.float32)
    v_x_col = np.sqrt(np.maximum(1.0 - xs * xs, 0.0))[:, None]
    v_z_col = xs[:, None]

    for yi, roughness in enumerate(ys):
        a = roughness * roughness
        a2 = a * a
        k = a * 0.5
        xi1 = hammer[:, 1]
        cos_theta = np.sqrt((1.0 - xi1) / (a2 * xi1 + (1.0 - xi1)))
        sin_theta = np.sqrt(np.clip(1.0 - cos_theta * cos_theta, 0.0, 1.0))
        phi = hammer[:, 0] * (2.0 * np.pi)
        h_x_row = (np.cos(phi) * sin_theta)[None, :]
        h_z_row = cos_theta[None, :]
        vDotH = np.maximum(h_x_row * v_x_col + h_z_row * v_z_col, 0.0)
        nDotH = np.maximum(h_z_row, 0.0)
        nDotL = np.maximum(nDotH * (vDotH + vDotH) - v_z_col, 0.0)
        denom = (nDotL * (1.0 - k) + k) * nDotH
        g = np.where(denom > 0.0, nDotL * vDotH / np.maximum(denom, 1e-20), 0.0)
        f = _fresnel_n_np(vDotH)
        s1 = (f * g).sum(axis=1)
        s2 = g.sum(axis=1)
        norm = n_samples * (xs * (1.0 - k) + k)
        s1 = np.where(norm > 0.0, s1 / norm, 0.0)
        s2 = np.where(norm > 0.0, s2 / norm, 0.0)
        out[yi, :, 0] = np.where(s2 > 0.0, s1 / np.maximum(s2, 1e-20), 0.0)
        out[yi, :, 1] = s2
        out[yi, :, 2] = _fresnel_n_np(xs)
        out[yi, :, 3] = float(_fresnel_n_np(np.array([roughness]))[0])

    _log.info("BRDF LUT generated %dx%d in %.2fs", size, size, time.time() - t0)
    tex = ctx.texture((size, size), 4, out.astype(np.float16).tobytes(), dtype="f2")
    tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
    tex.repeat_x = False
    tex.repeat_y = False
    return tex


# =============================================================================
# Scene — verbatim from tools/sf_render_test.py:1325-1798 with paths injected
# =============================================================================

@dataclass
class RenderMesh:
    name: str
    vao: moderngl.VertexArray
    vbo: moderngl.Buffer
    ibo: moderngl.Buffer
    model: np.ndarray
    mat: MatDef
    layer_texs: list[dict[str, "moderngl.Texture | None"]]
    blender_masks: list["moderngl.Texture | None"]
    # Per-mesh visibility flag honored by SFScene.render() — set via
    # SfBackend.set_visible() to power the editor's "Hide mesh part"
    # toggle in Starfield mode. Defaults True so existing meshes still
    # render unchanged after the field landed.
    visible: bool = True
    # Number of vertices in the VBO, recorded at upload time so the
    # vertex-points overlay can render the buffer as GL_POINTS without
    # round-tripping through the index buffer.
    num_verts: int = 0


class SFScene:
    """Parallel Starfield render scene. Self-contained: own GL program, own
    textures, own VBO/VAO, own draw loop. Editor calls render() per-frame.
    """

    def __init__(self, ctx: moderngl.Context, nif_path: Path,
                 extracted_dir: Path, exr_path: Path | None = None):
        self.ctx = ctx
        self.nif_path = Path(nif_path)
        self.extracted_dir = Path(extracted_dir)
        self.exr_path = Path(exr_path) if exr_path else None
        self.meshes: list[RenderMesh] = []
        self.tex_cache: dict[str, moderngl.Texture] = {}
        self.bbox_min = np.array([1e9, 1e9, 1e9], dtype=np.float32)
        self.bbox_max = np.array([-1e9, -1e9, -1e9], dtype=np.float32)
        self.env_lod_bias = 0.0

        self.prog = self.ctx.program(vertex_shader=VERT_SRC, fragment_shader=FRAG_SRC)
        self.brdf_lut = generate_sf_brdf_lut(ctx)
        self.prefilter_cubes: list[moderngl.TextureCube] = []
        self.cube_irr: moderngl.TextureCube | None = None

        self._load_nif()
        self._load_environment()

    # -- Environment ----------------------------------------------------------
    def _load_environment(self):
        if not self.exr_path or not self.exr_path.exists():
            _log.warning("EXR %s missing — environment disabled", self.exr_path)
            return
        try:
            equirect = load_exr_equirect(self.exr_path)
        except Exception as e:
            _log.warning("EXR load failed: %s", e)
            return
        _log.info("EXR loaded: %s shape=%s range=%.3f..%.3f",
                  self.exr_path.name, equirect.shape,
                  float(equirect.min()), float(equirect.max()))
        t0 = time.time()
        src_cube = equirect_to_cubemap(self.ctx, equirect, face_size=512)
        _log.info("source cube built in %.2fs", time.time() - t0)
        t0 = time.time()
        self.prefilter_cubes = prefilter_cubemap_ggx(
            self.ctx, src_cube, levels=_PREFILTER_ROUGHNESS_LEVELS, base_size=256)
        _log.info("%d prefilter cubes built in %.2fs",
                  len(self.prefilter_cubes), time.time() - t0)
        t0 = time.time()
        self.cube_irr = compute_irradiance_cube(self.ctx, src_cube,
                                                face_size=_IRRADIANCE_FACE_SIZE)
        _log.info("irradiance cube built in %.2fs", time.time() - t0)
        src_cube.release()

    # -- NIF walking ----------------------------------------------------------
    def _load_nif(self):
        nif = NifFile.load(str(self.nif_path))
        _log.info("Loaded NIF: %s (%d blocks)", self.nif_path.name, len(nif.blocks))

        # Standalone applies a Z-up→Y-up rotation here. The editor renderer's
        # camera works in NIF-native Z-up coordinates, so we use identity and
        # let the editor's camera/view matrix handle orientation.
        identity = np.eye(4, dtype=np.float32)
        root_block = nif.blocks[0] if nif.blocks else None
        if root_block is not None:
            self._walk_block(nif, root_block, identity)

        if self.bbox_max[0] < self.bbox_min[0]:
            self.bbox_min = np.array([-1, -1, -1], dtype=np.float32)
            self.bbox_max = np.array([ 1,  1,  1], dtype=np.float32)

    def add_attached_nif(self, nif_path: Path,
                         parent_xform: np.ndarray) -> list[RenderMesh]:
        """Load an additional NIF as an attachment with a parent transform.

        Walks the attached NIF and appends every BSGeometry it finds to
        ``self.meshes`` with ``parent_xform`` as the root transform — so
        the meshes inherit the connect-point world matrix the caller
        computed. Returns the list of newly added meshes so callers can
        track them (e.g. for set_visible toggles or later removal).

        Used by ``SfBackend.attach_nif`` to make Connect Points-based
        attachments visible in the wholesale-port draw path.
        """
        before = len(self.meshes)
        try:
            nif = NifFile.load(str(nif_path))
        except Exception:
            _log.exception("add_attached_nif: failed to load %s", nif_path)
            return []
        root_block = nif.blocks[0] if nif.blocks else None
        if root_block is None:
            return []
        self._walk_block(nif, root_block, parent_xform.astype(np.float32))
        added = self.meshes[before:]
        _log.info("add_attached_nif: %s -> %d new meshes", nif_path.name, len(added))
        return added

    def _walk_block(self, nif, block, parent_xform: np.ndarray):
        local = self._block_transform(block)
        world = parent_xform @ local

        tn = block.type_name
        if tn == "BSGeometry":
            mesh = self._load_bsgeometry(nif, block, world)
            if mesh is not None:
                self.meshes.append(mesh)
            return

        children = block.get_field("Children")
        if not children:
            return
        for ref in children:
            rid = ref.get("value") if isinstance(ref, dict) else ref
            if rid is None:
                continue
            rid = int(rid)
            if rid < 0:
                continue
            child = nif.get_block(rid)
            if child is not None:
                self._walk_block(nif, child, world)

    def _block_transform(self, block) -> np.ndarray:
        trans = block.get_field("Translation") or {}
        rot   = block.get_field("Rotation") or {}
        scale_raw = block.get_field("Scale")
        scale = float(scale_raw) if scale_raw is not None else 1.0

        if isinstance(trans, dict):
            tx = float(trans.get("x", 0))
            ty = float(trans.get("y", 0))
            tz = float(trans.get("z", 0))
        else:
            tx = ty = tz = 0.0

        if isinstance(rot, dict):
            r0 = (float(rot.get("m11", 1.0)),
                  float(rot.get("m21", 0.0)),
                  float(rot.get("m31", 0.0)))
            r1 = (float(rot.get("m12", 0.0)),
                  float(rot.get("m22", 1.0)),
                  float(rot.get("m32", 0.0)))
            r2 = (float(rot.get("m13", 0.0)),
                  float(rot.get("m23", 0.0)),
                  float(rot.get("m33", 1.0)))
            R = np.array([r0, r1, r2], dtype=np.float32)
        else:
            R = np.eye(3, dtype=np.float32)

        m = np.eye(4, dtype=np.float32)
        m[:3, :3] = R * scale
        m[0, 3] = tx
        m[1, 3] = ty
        m[2, 3] = tz
        return m

    def _load_bsgeometry(self, nif, block, world_xform: np.ndarray) -> RenderMesh | None:
        name = block.get_field("Name") or f"Geom_{block.block_id}"

        meshes_field = block.get_field("Meshes") or []
        mesh_path = ""
        for entry in meshes_field:
            if isinstance(entry, dict) and entry.get("Has Mesh"):
                m = entry.get("Mesh") or {}
                mp = m.get("Mesh Path") or ""
                if mp:
                    mesh_path = mp
                    break
        if not mesh_path:
            _log.debug("%s: no external mesh path", name)
            return None

        mesh_bytes = resolve_mesh(mesh_path, self.nif_path, self.extracted_dir)
        if mesh_bytes is None:
            _log.warning("%s: could not resolve mesh %s", name, mesh_path)
            return None

        sf = parse_sf_mesh_uv2(mesh_bytes)
        if sf is None:
            _log.warning("%s: mesh parse failed", name)
            return None

        shader_ref = block.get_field("Shader Property")
        shader_ref_id = shader_ref if isinstance(shader_ref, int) else (
            shader_ref.get("value", -1) if isinstance(shader_ref, dict) else -1
        )
        mat_name = ""
        if shader_ref_id is not None and int(shader_ref_id) >= 0:
            sp = nif.get_block(int(shader_ref_id))
            if sp is not None:
                mat_name = sp.get_field("Name") or ""
                if isinstance(mat_name, list):
                    mat_name = "".join(str(c) for c in mat_name)
                mat_name = mat_name.rstrip("\x00")

        mat = None
        if mat_name:
            mat_file = resolve_texture(mat_name, self.extracted_dir) \
                       or (self.extracted_dir / mat_name.replace("\\", "/"))
            if mat_file and mat_file.exists():
                mat = parse_mat(mat_file)
            else:
                _log.warning("%s: mat file not found: %s", name, mat_name)
        if mat is None:
            mat = MatDef(filename=mat_name or name, layers=[LayerDef()])

        model = world_xform.astype(np.float32)

        n = sf.positions.shape[0]
        interleaved = np.zeros((n, 3+3+3+3+4+2+2), dtype=np.float32)
        interleaved[:, 0:3]   = sf.positions
        interleaved[:, 3:6]   = sf.normals
        interleaved[:, 6:9]   = sf.tangents
        interleaved[:, 9:12]  = sf.bitangents
        interleaved[:, 12:16] = sf.colors
        interleaved[:, 16:18] = sf.uv1
        interleaved[:, 18:20] = sf.uv2
        vbo = self.ctx.buffer(interleaved.tobytes())
        ibo = self.ctx.buffer(sf.triangles.astype(np.uint32).tobytes())
        vao = self.ctx.vertex_array(
            self.prog,
            [(vbo, "3f 3f 3f 3f 4f 2f 2f",
              "inPos", "inNormal", "inTangent", "inBitangent", "inColor",
              "inUV1", "inUV2")],
            index_buffer=ibo,
        )

        world_positions = (sf.positions @ model[:3, :3].T) + model[:3, 3]
        self.bbox_min = np.minimum(self.bbox_min, world_positions.min(axis=0))
        self.bbox_max = np.maximum(self.bbox_max, world_positions.max(axis=0))

        layer_texs: list[dict[str, "moderngl.Texture | None"]] = []
        for layer in mat.layers[:MAX_LAYERS]:
            d: dict[str, moderngl.Texture | None] = {}
            d["albedo"]  = self._load_tex(layer.albedo_path)
            d["normal"]  = self._load_tex(layer.normal_path)
            d["rough"]   = self._load_tex(layer.rough_path)
            d["metal"]   = self._load_tex(layer.metal_path)
            d["ao"]      = self._load_tex(layer.ao_path)
            d["opacity"] = self._load_tex(layer.opacity_path)
            layer_texs.append(d)
        while len(layer_texs) < MAX_LAYERS:
            layer_texs.append({"albedo": None, "normal": None, "rough": None,
                               "metal": None, "ao": None, "opacity": None})

        blender_masks: list["moderngl.Texture | None"] = []
        for b in mat.blenders[:MAX_LAYERS]:
            blender_masks.append(self._load_tex(b.mask_path))
        while len(blender_masks) < MAX_LAYERS:
            blender_masks.append(None)

        return RenderMesh(
            name=name,
            vao=vao,
            vbo=vbo,
            ibo=ibo,
            model=model.astype(np.float32),
            mat=mat,
            layer_texs=layer_texs,
            blender_masks=blender_masks,
            num_verts=int(n),
        )

    def _load_tex(self, rel: str) -> "moderngl.Texture | None":
        if not rel:
            return None
        key = rel.lower()
        if key in self.tex_cache:
            return self.tex_cache[key]
        path = resolve_texture(rel, self.extracted_dir)
        if path is None:
            _log.debug("texture not found: %s", rel)
            return None
        tex = load_dds_to_gl(self.ctx, path)
        if tex is not None:
            self.tex_cache[key] = tex
        return tex

    # -- Bounding info for camera framing -------------------------------------
    def bounds(self) -> tuple[np.ndarray, float]:
        center = (self.bbox_min + self.bbox_max) * 0.5
        radius = float(np.linalg.norm(self.bbox_max - self.bbox_min) * 0.5)
        return center.astype(np.float32), max(radius, 1.0)

    # -- Render --------------------------------------------------------------
    def render(self, view: np.ndarray, proj: np.ndarray,
               light_dir_world: np.ndarray,
               tone_map_scale: float = 0.1, brightness_scale: float = 0.1,
               env_intensity: float = 8.0, light_intensity: float = 1.0,
               ambient: tuple[float, float, float] = (0.7, 0.7, 0.7),
               *,
               light_col: tuple[float, float, float] | None = None,
               toggles: dict | None = None,
               dbg: dict | None = None,
               fill_dir_view: tuple[float, float, float] = (0.0, 0.0, 0.0),
               fill_col: tuple[float, float, float] = (0.0, 0.0, 0.0),
               mirror_dir_view: tuple[float, float, float] = (0.0, 0.0, 0.0),
               mirror_col: tuple[float, float, float] = (0.0, 0.0, 0.0),
               mirror_enabled: bool = False,
               ssao_enabled: bool = False,
               shadow_enabled: bool = False,
               shadow_map: "moderngl.Texture | None" = None,
               light_space_matrix: "np.ndarray | None" = None):
        ctx = self.ctx
        prog = self.prog

        view = np.asarray(view, dtype=np.float32)
        proj = np.asarray(proj, dtype=np.float32)
        light_dir_world = np.asarray(light_dir_world, dtype=np.float32)

        prog["uView"].write(view.T.tobytes())
        prog["uProj"].write(proj.T.tobytes())
        prog["uLightDirWorld"].value = tuple(light_dir_world.tolist())
        if light_col is not None:
            prog["uLightSourceDiffuse"].value = tuple(float(v) for v in light_col)
        else:
            prog["uLightSourceDiffuse"].value = (light_intensity, light_intensity, light_intensity)
        prog["uLightSourceAmbient"].value = ambient
        prog["uToneMapScale"].value = float(tone_map_scale)
        prog["uBrightnessScale"].value = float(brightness_scale)
        prog["uEnvIntensity"].value = float(env_intensity)
        prog["uHasCubeMap"].value = bool(self.prefilter_cubes)
        prog["uHasSpecular"].value = True
        prog["uEnvLodBias"].value = float(self.env_lod_bias)
        prog["uEnvRotation"].write(np.eye(3, dtype=np.float32).tobytes())

        # Scene-menu toggles. Default ON (1.0) so calls that pass no toggles
        # see no change from pre-wiring behavior.
        _t = toggles or {}
        prog["uToggleDiffuse"].value     = 1.0 if _t.get("diffuse", True) else 0.0
        prog["uToggleNormal"].value      = 1.0 if _t.get("normal", True) else 0.0
        prog["uToggleSpec"].value        = 1.0 if _t.get("spec", True) else 0.0
        prog["uToggleLighting"].value    = 1.0 if _t.get("lighting", True) else 0.0
        prog["uToggleVertexColor"].value = 1.0 if _t.get("vertexColor", True) else 0.0
        prog["uToggleEnvMap"].value      = 1.0 if _t.get("envMap", True) else 0.0

        # Debug PBR sliders. Defaults mirror renderer._setup_fo4_uniforms.
        _d = dbg or {}
        prog["uDbgEnvBoost"].value      = float(_d.get("envBoost", 1.0))
        prog["uDbgMetalF0"].value       = float(_d.get("metalF0", 0.9))
        prog["uDbgDiffuseBleed"].value  = float(_d.get("diffuseBleed", 0.0))
        prog["uDbgExposure"].value      = float(_d.get("exposure", 4.23))
        prog["uDbgSpecBoost"].value     = float(_d.get("specBoost", 1.0))
        prog["uDbgAmbientBoost"].value  = float(_d.get("ambientBoost", 1.0))

        # Fill + mirror lights (view-space directions — caller converts from world).
        prog["uFillDirView"].value     = tuple(float(v) for v in fill_dir_view)
        prog["uFillCol"].value         = tuple(float(v) for v in fill_col)
        prog["uMirrorDirView"].value   = tuple(float(v) for v in mirror_dir_view)
        prog["uMirrorCol"].value       = tuple(float(v) for v in mirror_col)
        prog["uMirrorEnabled"].value   = 1.0 if mirror_enabled else 0.0

        # SSAO MRT: when enabled, the FRAG_SRC writes view-space normals to
        # color attachment 1 (_fbo_normal_tex) so _render_ssao can sample it
        # exactly like the FO4 path.
        prog["mrtEnabled"].value = 1.0 if ssao_enabled else 0.0

        # Shadow uniforms. When the host renderer hands us an
        # active shadow_map texture + light_space_matrix, bind them to the
        # SF program so calcShadow() in FRAG_SRC samples them. We use a
        # high texture unit (MAX_TEXTURES + 7) to avoid stomping the BRDF
        # LUT (MAX_TEXTURES) and the prefilter cubes (MAX_TEXTURES+1..6)
        # and the irradiance cube (MAX_TEXTURES+7 → bumped to +8 below).
        if "uShadowEnabled" in prog:
            sf_shadow_active = (
                shadow_enabled
                and shadow_map is not None
                and light_space_matrix is not None)
            prog["uShadowEnabled"].value = 1.0 if sf_shadow_active else 0.0
            if sf_shadow_active:
                shadow_unit = MAX_TEXTURES + 8
                shadow_map.use(shadow_unit)
                if "uShadowMap" in prog:
                    prog["uShadowMap"].value = shadow_unit
                if "uLightSpaceMatrix" in prog:
                    # GL is column-major; light_space_matrix arrives as a
                    # row-major numpy mat from the host renderer (which
                    # builds it via glm and then numpy-converts). Transpose
                    # before upload so columns end up in the right slots.
                    lsm = np.asarray(
                        light_space_matrix, dtype=np.float32).T
                    prog["uLightSpaceMatrix"].write(lsm.tobytes())

        brdf_unit = MAX_TEXTURES
        self.brdf_lut.use(brdf_unit)
        prog["uBrdfLUT"].value = brdf_unit

        cube_base = brdf_unit + 1
        for i in range(6):
            uname = f"uCubeMip{i}"
            unit = cube_base + i
            cube = self.prefilter_cubes[i] if i < len(self.prefilter_cubes) else None
            if cube is not None:
                cube.use(unit)
            if uname in prog:
                prog[uname].value = unit

        if self.cube_irr is not None and "uCubeIrradiance" in prog:
            irr_unit = cube_base + 6
            self.cube_irr.use(irr_unit)
            prog["uCubeIrradiance"].value = irr_unit

        if "uTextures" in prog:
            prog["uTextures"].write(struct.pack(f"<{MAX_TEXTURES}i", *range(MAX_TEXTURES)))

        # Opaque pass
        ctx.disable(moderngl.BLEND)
        ctx.depth_mask = True
        for mesh in self.meshes:
            if not mesh.visible:
                continue
            if mesh.mat.is_decal or mesh.mat.has_opacity:
                continue
            self._draw_mesh(prog, mesh, view)

        # Transparent / decal pass
        ctx.enable(moderngl.BLEND)
        ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
        ctx.depth_mask = False
        ctx.enable_direct(0x8037)  # GL_POLYGON_OFFSET_FILL
        ctx.polygon_offset = (-2.0, -2.0)
        for mesh in self.meshes:
            if not mesh.visible:
                continue
            if not (mesh.mat.is_decal or mesh.mat.has_opacity):
                continue
            self._draw_mesh(prog, mesh, view)
        ctx.polygon_offset = (0.0, 0.0)
        ctx.disable_direct(0x8037)
        ctx.depth_mask = True
        ctx.disable(moderngl.BLEND)

    def _draw_mesh(self, prog: moderngl.Program, mesh: RenderMesh, view: np.ndarray):
        model = mesh.model
        prog["uModel"].write(model.T.tobytes())
        mv3 = (view @ model)[:3, :3]
        normal_mat = np.linalg.inv(mv3).T.astype(np.float32)
        prog["uNormalMat"].write(normal_mat.T.tobytes())

        unit = 0
        assignments: list[tuple[int, "moderngl.Texture"]] = []
        layer_slot = []

        def take(tex):
            nonlocal unit
            if tex is None or unit >= MAX_TEXTURES:
                return -1
            assignments.append((unit, tex))
            u = unit
            unit += 1
            return u

        for li in range(MAX_LAYERS):
            d = mesh.layer_texs[li] if li < len(mesh.layer_texs) else {}
            layer_slot.append({
                "albedo":  take(d.get("albedo")),
                "normal":  take(d.get("normal")),
                "rough":   take(d.get("rough")),
                "metal":   take(d.get("metal")),
                "ao":      take(d.get("ao")),
                "opacity": take(d.get("opacity")),
            })

        blender_units: list[int] = []
        for mask in mesh.blender_masks[:MAX_LAYERS]:
            blender_units.append(take(mask))
        while len(blender_units) < MAX_LAYERS:
            blender_units.append(-1)

        for u, tex in assignments:
            tex.use(u)

        prog["uNumLayers"].value = len(mesh.mat.layers)

        albedo_units  = [layer_slot[i]["albedo"]  for i in range(MAX_LAYERS)]
        normal_units  = [layer_slot[i]["normal"]  for i in range(MAX_LAYERS)]
        rough_units   = [layer_slot[i]["rough"]   for i in range(MAX_LAYERS)]
        metal_units   = [layer_slot[i]["metal"]   for i in range(MAX_LAYERS)]
        ao_units      = [layer_slot[i]["ao"]      for i in range(MAX_LAYERS)]
        opacity_units = [layer_slot[i]["opacity"] for i in range(MAX_LAYERS)]

        uv_scales: list[float] = []
        uv_offsets: list[float] = []
        uv_channels: list[int] = []
        tints: list[float] = []
        normal_scales: list[float] = []
        for i in range(MAX_LAYERS):
            layer = mesh.mat.layers[i] if i < len(mesh.mat.layers) else LayerDef()
            uv_scales.extend([layer.uv_scale[0], layer.uv_scale[1]])
            uv_offsets.extend([layer.uv_offset[0], layer.uv_offset[1]])
            uv_channels.append(layer.uv_channel)
            tints.extend([layer.tint[0], layer.tint[1], layer.tint[2]])
            normal_scales.append(layer.normal_intensity)

        def set_ints(name: str, values: list[int]):
            if name in prog:
                prog[name].write(struct.pack(f"<{len(values)}i", *values))

        def set_floats(name: str, values: list[float]):
            if name in prog:
                prog[name].write(struct.pack(f"<{len(values)}f", *values))

        set_ints("uLayerAlbedoUnit",  albedo_units)
        set_ints("uLayerNormalUnit",  normal_units)
        set_ints("uLayerRoughUnit",   rough_units)
        set_ints("uLayerMetalUnit",   metal_units)
        set_ints("uLayerAoUnit",      ao_units)
        set_ints("uLayerOpacityUnit", opacity_units)
        if "uHasOpacity" in prog:
            prog["uHasOpacity"].value = bool(mesh.mat.has_opacity or mesh.mat.is_decal)
        if "uAlphaTest" in prog:
            prog["uAlphaTest"].value = bool(mesh.mat.has_opacity and not mesh.mat.is_decal)
        if "uAlphaThreshold" in prog:
            prog["uAlphaThreshold"].value = float(mesh.mat.alpha_test_threshold)
        set_floats("uLayerUvScale",  uv_scales)
        set_floats("uLayerUvOffset", uv_offsets)
        set_ints("uLayerUvChannel",  uv_channels)
        set_floats("uLayerTint",     tints)
        set_floats("uLayerNormalScale", normal_scales)

        blend_modes = []
        blend_vc   = []
        blend_int  = []
        blend_alb  = []
        blend_nor  = []
        blend_met  = []
        blend_rou  = []
        blend_ao   = []
        blend_add  = []
        for i in range(MAX_LAYERS):
            b = mesh.mat.blenders[i] if i < len(mesh.mat.blenders) else BlenderDef()
            blend_modes.append(b.mode)
            blend_vc.append(b.vc_channel)
            blend_int.append(b.mask_intensity)
            blend_alb.append(1 if b.blend_albedo else 0)
            blend_nor.append(1 if b.blend_normal else 0)
            blend_met.append(1 if b.blend_metal else 0)
            blend_rou.append(1 if b.blend_rough else 0)
            blend_ao.append(1 if b.blend_ao else 0)
            blend_add.append(1 if b.additive_normal else 0)

        if "uBlenderCount" in prog:
            prog["uBlenderCount"].value = len(mesh.mat.blenders)
        set_ints("uBlenderMaskUnit", blender_units)
        set_ints("uBlenderMode",     blend_modes)
        set_ints("uBlenderVcChan",   blend_vc)
        set_floats("uBlenderMaskInt", blend_int)
        set_ints("uBlendAlbedo",  blend_alb)
        set_ints("uBlendNormal",  blend_nor)
        set_ints("uBlendMetal",   blend_met)
        set_ints("uBlendRough",   blend_rou)
        set_ints("uBlendAO",      blend_ao)
        set_ints("uBlendAddNormal", blend_add)

        mesh.vao.render()

    # -- Overlay programs (vertex points, selection outline) -----------------
    #
    # These are tiny programs created lazily so the SF main program +
    # cubemap pipeline aren't burdened with extra compile work for users
    # who never toggle on the overlays. Per-mesh alternate VAOs are
    # cached on the RenderMesh itself via private ``_aux_*`` attributes
    # so we don't have to hash/lookup on each frame.

    def _ensure_points_prog(self):
        if getattr(self, "_points_prog", None) is not None:
            return self._points_prog
        self._points_prog = self.ctx.program(
            vertex_shader="""
                #version 330 core
                in vec3 inPos;
                uniform mat4 uModel;
                uniform mat4 uViewProj;
                void main() {
                    gl_Position = uViewProj * uModel * vec4(inPos, 1.0);
                    gl_PointSize = 3.0;
                }
            """,
            fragment_shader="""
                #version 330 core
                out vec4 fragColor;
                void main() { fragColor = vec4(0.3, 0.7, 1.0, 1.0); }
            """,
        )
        return self._points_prog

    def _get_points_vao(self, mesh: RenderMesh):
        vao = getattr(mesh, "_points_vao", None)
        if vao is not None:
            return vao
        prog = self._ensure_points_prog()
        # SFScene packs an interleaved VBO of 20 floats per vertex
        # (3 pos + 3 norm + 3 tan + 3 bit + 4 col + 2 uv1 + 2 uv2).
        # Stride = 80 bytes. Skip 68 bytes after the position to land
        # on the next vertex. No index buffer — we draw as POINTS over
        # the raw vertex range.
        vao = self.ctx.vertex_array(
            prog, [(mesh.vbo, "3f 68x", "inPos")],
        )
        mesh._points_vao = vao
        return vao

    def draw_vertex_points(self, view_np: np.ndarray, proj_np: np.ndarray) -> None:
        """Draw every visible mesh's vertices as small dots.

        Powers the editor's "Show Vertices" toggle for Starfield. Uses
        a tiny embedded program (no SF PBR cost) and per-mesh alt-VAOs
        cached on each RenderMesh. Honors mesh.visible so hidden parts
        don't show their vertices either.
        """
        prog = self._ensure_points_prog()
        view_proj = (proj_np @ view_np).T  # GL column-major
        if "uViewProj" in prog:
            prog["uViewProj"].write(view_proj.astype(np.float32).tobytes())
        self.ctx.enable_direct(0x8642)  # GL_PROGRAM_POINT_SIZE
        self.ctx.disable(moderngl.CULL_FACE)
        for mesh in self.meshes:
            if not mesh.visible:
                continue
            if "uModel" in prog:
                prog["uModel"].write(mesh.model.T.astype(np.float32).tobytes())
            vao = self._get_points_vao(mesh)
            vao.render(moderngl.POINTS, vertices=mesh.num_verts)
        self.ctx.enable(moderngl.CULL_FACE)
        self.ctx.disable_direct(0x8642)

    def _ensure_outline_prog(self):
        if getattr(self, "_outline_prog", None) is not None:
            return self._outline_prog
        self._outline_prog = self.ctx.program(
            vertex_shader="""
                #version 330 core
                in vec3 inPos;
                in vec3 inNormal;
                uniform mat4 uModel;
                uniform mat4 uViewProj;
                uniform float uWidth;
                void main() {
                    vec3 p = inPos + inNormal * uWidth;
                    gl_Position = uViewProj * uModel * vec4(p, 1.0);
                }
            """,
            fragment_shader="""
                #version 330 core
                uniform vec4 uColor;
                out vec4 fragColor;
                void main() { fragColor = uColor; }
            """,
        )
        return self._outline_prog

    def _get_outline_vao(self, mesh: RenderMesh):
        vao = getattr(mesh, "_outline_vao", None)
        if vao is not None:
            return vao
        prog = self._ensure_outline_prog()
        # Position (3f) at offset 0, normal (3f) at offset 12. Skip 56
        # bytes (14 floats) to reach the next vertex's position.
        vao = self.ctx.vertex_array(
            prog,
            [(mesh.vbo, "3f 3f 56x", "inPos", "inNormal")],
            index_buffer=mesh.ibo,
        )
        mesh._outline_vao = vao
        return vao

    def draw_selection_outline(self, mesh: RenderMesh,
                               view_np: np.ndarray, proj_np: np.ndarray,
                               color: tuple[float, float, float] = (0.3, 0.6, 1.0)) -> None:
        """Render a glowy outline shell for one mesh (back-face extrusion).

        Two-pass glow: an outer extrusion at low alpha, then a tighter
        core extrusion at higher alpha. Mirrors the FO4 selection outline
        visual style. Caller is responsible for picking the mesh — this
        method just draws the shell.
        """
        prog = self._ensure_outline_prog()
        view_proj = (proj_np @ view_np).T
        if "uViewProj" in prog:
            prog["uViewProj"].write(view_proj.astype(np.float32).tobytes())
        if "uModel" in prog:
            prog["uModel"].write(mesh.model.T.astype(np.float32).tobytes())

        # Width relative to mesh world AABB diagonal so the outline
        # scales with mesh size. Cheap heuristic — exact bbox lives on
        # the scene level.
        base_width = 0.02

        ctx = self.ctx
        ctx.disable(moderngl.DEPTH_TEST)
        ctx.enable(moderngl.BLEND)
        ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
        ctx.front_face = "ccw"
        ctx.cull_face = "front"
        ctx.enable(moderngl.CULL_FACE)

        cr, cg, cb = color
        vao = self._get_outline_vao(mesh)

        # Outer glow
        if "uWidth" in prog:
            prog["uWidth"].value = base_width * 4.0
        if "uColor" in prog:
            prog["uColor"].value = (cr * 0.7, cg * 0.7, cb * 0.7, 0.25)
        vao.render()

        # Core
        if "uWidth" in prog:
            prog["uWidth"].value = base_width * 1.5
        if "uColor" in prog:
            prog["uColor"].value = (cr, cg, cb, 0.85)
        vao.render()

        ctx.cull_face = "back"
        ctx.disable(moderngl.BLEND)
        ctx.enable(moderngl.DEPTH_TEST)

    # -- Shadow caster pass ----------------------------------------
    def _get_shadow_vao(self, mesh: RenderMesh, shadow_prog):
        """Build/return a depth-only alt-VAO for the shadow_depth program.

        SF meshes are drawn through their main VAO (bound to the SF PBR
        program). Shadow casting needs the same VBO/IBO bound to the
        shadow_depth program with attribute name ``in_position``. Cache
        the alt-VAO on the mesh; rebuild only if the program changes.
        """
        cache = getattr(mesh, "_shadow_vao_pair", None)
        if cache is not None and cache[0] is shadow_prog:
            return cache[1]
        vao = self.ctx.vertex_array(
            shadow_prog,
            [(mesh.vbo, "3f 68x", "in_position")],
            index_buffer=mesh.ibo,
        )
        mesh._shadow_vao_pair = (shadow_prog, vao)
        return vao

    def render_shadow_casters(self, light_vp: np.ndarray, shadow_prog) -> None:
        """Walk meshes and render them depth-only into the bound shadow FBO.

        Caller binds the shadow FBO and configures front-face culling
        before invoking this method. We just upload the per-mesh
        light-space MVP and render via cached alt-VAOs. Honors visible
        flag and skips opacity-tested / decal meshes (which can't reliably
        produce a sealed shadow shape).
        """
        if "u_light_mvp" not in shadow_prog:
            return
        for mesh in self.meshes:
            if not mesh.visible:
                continue
            if mesh.mat.is_decal or mesh.mat.has_opacity:
                continue
            # u_light_mvp is column-major in GL; build (light_vp @ model)
            # then transpose for the upload.
            light_mvp = (light_vp @ mesh.model).T.astype(np.float32)
            shadow_prog["u_light_mvp"].write(light_mvp.tobytes())
            self._get_shadow_vao(mesh, shadow_prog).render()

    # -- Cleanup --------------------------------------------------------------
    def release(self):
        for m in self.meshes:
            try: m.vao.release()
            except Exception: pass
            try: m.vbo.release()
            except Exception: pass
            try: m.ibo.release()
            except Exception: pass
            # Overlay VAOs cached on the mesh by draw_vertex_points /
            # draw_selection_outline / render_shadow_casters. All reference
            # the same vbo/ibo as the main vao so they don't own GL
            # resources beyond the vertex array object itself.
            for attr in ("_points_vao", "_outline_vao"):
                vao = getattr(m, attr, None)
                if vao is not None:
                    try: vao.release()
                    except Exception: pass
            shadow_pair = getattr(m, "_shadow_vao_pair", None)
            if shadow_pair is not None:
                try: shadow_pair[1].release()
                except Exception: pass
        self.meshes.clear()
        for attr in ("_points_prog", "_outline_prog"):
            prog = getattr(self, attr, None)
            if prog is not None:
                try: prog.release()
                except Exception: pass
                setattr(self, attr, None)
        for tex in self.tex_cache.values():
            try: tex.release()
            except Exception: pass
        self.tex_cache.clear()
        for c in self.prefilter_cubes:
            try: c.release()
            except Exception: pass
        self.prefilter_cubes.clear()
        if self.cube_irr is not None:
            try: self.cube_irr.release()
            except Exception: pass
            self.cube_irr = None
        try: self.brdf_lut.release()
        except Exception: pass
        try: self.prog.release()
        except Exception: pass
