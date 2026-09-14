"""PBR renderer with shadow map for trimesh-backed preview surfaces."""
from __future__ import annotations
from dataclasses import dataclass, field
from pathlib import Path
import logging

import glm
import moderngl
import numpy as np

_log = logging.getLogger("creation_lib.renderer.simple")
_SHADER_DIR = Path(__file__).parent / "shaders"
_SHADOW_SIZE = 1024


def compute_tangents(
    positions: np.ndarray,
    normals: np.ndarray,
    uvs: np.ndarray,
    faces: np.ndarray,
) -> np.ndarray:
    """Compute normalized (N, 3) float32 per-vertex tangents from UV gradients.

    MikkTSpace-compatible: accumulate per-triangle tangents, then normalize per
    vertex.
    """
    tangents = np.zeros_like(positions)

    for tri in faces:
        i0, i1, i2 = tri
        p0, p1, p2 = positions[i0], positions[i1], positions[i2]
        uv0, uv1, uv2 = uvs[i0], uvs[i1], uvs[i2]

        edge1 = p1 - p0
        edge2 = p2 - p0
        duv1 = uv1 - uv0
        duv2 = uv2 - uv0

        denom = duv1[0] * duv2[1] - duv2[0] * duv1[1]
        if abs(denom) < 1e-8:
            continue
        f = 1.0 / denom

        t = f * (duv2[1] * edge1 - duv1[1] * edge2)
        tangents[i0] += t
        tangents[i1] += t
        tangents[i2] += t

    # Normalize and orthogonalize against normals
    for i in range(len(tangents)):
        n = normals[i]
        t = tangents[i]
        # Gram-Schmidt orthogonalize
        t = t - n * np.dot(n, t)
        length = np.linalg.norm(t)
        if length > 1e-6:
            tangents[i] = t / length
        else:
            # Fallback: pick arbitrary tangent perpendicular to normal
            if abs(n[0]) < 0.9:
                tangents[i] = np.cross(n, [1, 0, 0])
            else:
                tangents[i] = np.cross(n, [0, 1, 0])
            tangents[i] /= np.linalg.norm(tangents[i])

    return tangents.astype(np.float32)


@dataclass
class PBRMaterial:
    """PBR metallic-roughness material (matches glTF 2.0 model)."""
    albedo_tex: moderngl.Texture | None = None
    normal_tex: moderngl.Texture | None = None
    mr_tex: moderngl.Texture | None = None         # metallic (B) + roughness (G)
    ao_tex: moderngl.Texture | None = None
    emissive_tex: moderngl.Texture | None = None
    base_color: tuple = (0.8, 0.8, 0.8, 1.0)
    metallic: float = 0.0
    roughness: float = 0.5
    emissive_factor: tuple = (0.0, 0.0, 0.0)


@dataclass
class SimpleMesh:
    """GPU mesh with PBR material."""
    vao: moderngl.VertexArray | None
    vbo: moderngl.Buffer | None
    ibo: moderngl.Buffer | None
    num_indices: int
    material: PBRMaterial
    transform: glm.mat4 = field(default_factory=lambda: glm.mat4(1.0))


class SimpleRenderer:
    """PBR renderer with directional shadow map previews."""

    def __init__(self, ctx: moderngl.Context):
        self.ctx = ctx

        # FBO
        self.fbo: moderngl.Framebuffer | None = None
        self.fbo_texture: moderngl.Texture | None = None
        self._fbo_depth: moderngl.Renderbuffer | None = None
        self._fbo_size = (0, 0)

        # Shaders
        self._pbr_prog: moderngl.Program | None = None
        self._shadow_prog: moderngl.Program | None = None

        # Shadow map
        self._shadow_fbo: moderngl.Framebuffer | None = None
        self._shadow_depth_tex: moderngl.Texture | None = None
        self._light_space_matrix = glm.mat4(1.0)
        self._shadow_vaos: dict[int, moderngl.VertexArray] = {}  # mesh vbo id -> shadow VAO

        # Grid
        self.grid = None
        self.grid_visible = True

        # Scene bounds (updated on load)
        self._scene_center = glm.vec3(0)
        self._scene_radius = 10.0

        # Default 1x1 white texture
        self._default_tex: moderngl.Texture | None = None

        self._compile_shaders()
        self._init_shadow_map()
        self._init_default_texture()

    def _compile_shaders(self):
        """Compile PBR and shadow shaders from files."""
        pbr_vert = (_SHADER_DIR / "pbr.vert").read_text()
        pbr_frag = (_SHADER_DIR / "pbr.frag").read_text()
        self._pbr_prog = self.ctx.program(
            vertex_shader=pbr_vert, fragment_shader=pbr_frag)

        shadow_vert = (_SHADER_DIR / "shadow.vert").read_text()
        shadow_frag = (_SHADER_DIR / "shadow.frag").read_text()
        self._shadow_prog = self.ctx.program(
            vertex_shader=shadow_vert, fragment_shader=shadow_frag)

    def _init_shadow_map(self):
        """Create shadow map FBO."""
        sz = _SHADOW_SIZE
        self._shadow_depth_tex = self.ctx.depth_texture((sz, sz))
        self._shadow_depth_tex.compare_func = ''
        self._shadow_depth_tex.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._shadow_fbo = self.ctx.framebuffer(
            depth_attachment=self._shadow_depth_tex)

    def _init_default_texture(self):
        """Create a 1x1 white texture for meshes without textures."""
        self._default_tex = self.ctx.texture((1, 1), 4,
            b'\xff\xff\xff\xff')
        self._default_tex.filter = (moderngl.NEAREST, moderngl.NEAREST)

    def init_grid(self):
        """Create the ground grid. Call after shaders are compiled."""
        from .grid import Grid, compile_grid_shader
        grid_prog = compile_grid_shader(self.ctx)
        self.grid = Grid(self.ctx, grid_prog)

    def ensure_fbo(self, width: int, height: int):
        """Create/resize the offscreen FBO. Debounce: skip if delta < 8px."""
        w = max(1, int(width))
        h = max(1, int(height))
        if self._fbo_size != (0, 0):
            dw = abs(w - self._fbo_size[0])
            dh = abs(h - self._fbo_size[1])
            if dw < 8 and dh < 8:
                return
        # Release old
        if self.fbo_texture:
            self.fbo_texture.release()
        if self._fbo_depth:
            self._fbo_depth.release()
        if self.fbo:
            self.fbo.release()

        self.fbo_texture = self.ctx.texture((w, h), 4)
        self.fbo_texture.filter = (moderngl.LINEAR, moderngl.LINEAR)
        self._fbo_depth = self.ctx.depth_renderbuffer((w, h))
        self.fbo = self.ctx.framebuffer(
            color_attachments=[self.fbo_texture],
            depth_attachment=self._fbo_depth,
        )
        self._fbo_size = (w, h)

    def _upload_mesh(self, mesh, transform=None) -> SimpleMesh | None:
        """Upload a single mesh to GPU."""
        if not hasattr(mesh, 'vertices') or len(mesh.vertices) == 0:
            return None

        verts = np.asarray(mesh.vertices, dtype=np.float32)
        faces = np.asarray(mesh.faces, dtype=np.int32)
        normals = np.asarray(mesh.vertex_normals, dtype=np.float32)

        # UVs
        if mesh.visual and hasattr(mesh.visual, 'uv') and mesh.visual.uv is not None:
            uvs = np.asarray(mesh.visual.uv, dtype=np.float32)
        else:
            uvs = np.zeros((len(verts), 2), dtype=np.float32)

        # Tangents: check for glTF tangent attribute, else compute
        tangents = compute_tangents(verts, normals, uvs, faces)

        # Interleave: position(3) + normal(3) + texcoord(2) + tangent(3) = 11 floats
        vertex_data = np.hstack([verts, normals, uvs, tangents]).astype(np.float32)

        vbo = self.ctx.buffer(vertex_data.tobytes())
        ibo = self.ctx.buffer(faces.tobytes())

        vao = self.ctx.vertex_array(
            self._pbr_prog,
            [(vbo, "3f 3f 2f 3f", "in_position", "in_normal", "in_texcoord", "in_tangent")],
            index_buffer=ibo,
            index_element_size=4,
        )

        # Extract PBR material from visual
        material = self._extract_material(mesh)

        # Transform
        xform = glm.mat4(1.0)
        if transform is not None:
            # Convert numpy 4x4 to glm (column-major)
            t = np.asarray(transform, dtype=np.float32)
            xform = glm.mat4(*t.T.flatten())

        return SimpleMesh(
            vao=vao, vbo=vbo, ibo=ibo,
            num_indices=len(faces) * 3,
            material=material, transform=xform,
        )

    def _extract_material(self, mesh) -> PBRMaterial:
        """Extract PBR material"""
        mat = PBRMaterial()

        if not hasattr(mesh, 'visual') or mesh.visual is None:
            return mat

        visual = mesh.visual

        # Handle PBR material from glTF
        if hasattr(visual, 'material') and visual.material is not None:
            m = visual.material

            # Base color
            if hasattr(m, 'baseColorFactor') and m.baseColorFactor is not None:
                bc = m.baseColorFactor
                mat.base_color = tuple(float(x) for x in bc[:4])

            if hasattr(m, 'metallicFactor') and m.metallicFactor is not None:
                mat.metallic = float(m.metallicFactor)

            if hasattr(m, 'roughnessFactor') and m.roughnessFactor is not None:
                mat.roughness = float(m.roughnessFactor)

            if hasattr(m, 'emissiveFactor') and m.emissiveFactor is not None:
                ef = m.emissiveFactor
                mat.emissive_factor = tuple(float(x) for x in ef[:3])

            # Textures
            if hasattr(m, 'baseColorTexture') and m.baseColorTexture is not None:
                mat.albedo_tex = self._upload_texture(m.baseColorTexture)
            elif hasattr(visual, 'material') and hasattr(visual.material, 'image') and visual.material.image is not None:
                mat.albedo_tex = self._upload_texture(visual.material.image)

            if hasattr(m, 'normalTexture') and m.normalTexture is not None:
                mat.normal_tex = self._upload_texture(m.normalTexture)

            if hasattr(m, 'metallicRoughnessTexture') and m.metallicRoughnessTexture is not None:
                mat.mr_tex = self._upload_texture(m.metallicRoughnessTexture)

            if hasattr(m, 'occlusionTexture') and m.occlusionTexture is not None:
                mat.ao_tex = self._upload_texture(m.occlusionTexture)

            if hasattr(m, 'emissiveTexture') and m.emissiveTexture is not None:
                mat.emissive_tex = self._upload_texture(m.emissiveTexture)

        return mat

    def _upload_texture(self, image_or_tex) -> moderngl.Texture | None:
        """Upload a PIL Image or texture to GPU."""
        try:
            from PIL import Image
            if isinstance(image_or_tex, Image.Image):
                img = image_or_tex
            elif hasattr(image_or_tex, 'image'):
                img = image_or_tex.image
                if img is None:
                    return None
            else:
                return None

            img = img.convert("RGBA")
            data = img.tobytes()
            tex = self.ctx.texture(img.size, 4, data)
            tex.filter = (moderngl.LINEAR_MIPMAP_LINEAR, moderngl.LINEAR)
            tex.build_mipmaps()
            return tex
        except Exception as e:
            _log.warning("Failed to upload texture: %s", e)
            return None

    def _update_scene_bounds(self, scene_or_mesh):
        """Compute bounding sphere from bounds."""
        bounds = scene_or_mesh.bounds
        if bounds is None or len(bounds) != 2:
            return

        min_pt = glm.vec3(*bounds[0])
        max_pt = glm.vec3(*bounds[1])
        self._scene_center = (min_pt + max_pt) * 0.5
        self._scene_radius = glm.length(max_pt - min_pt) * 0.5

    def _get_shadow_vao(self, mesh: SimpleMesh) -> moderngl.VertexArray | None:
        """Get or create a shadow-pass VAO (position only) for a mesh."""
        if not mesh.vbo:
            return None
        key = mesh.vbo.glo
        if key in self._shadow_vaos:
            return self._shadow_vaos[key]
        # VBO layout: 3f(pos) 3f(norm) 2f(uv) 3f(tang) = 11 floats = 44 bytes stride
        # Shadow shader only needs in_position (first 3 floats)
        vao = self.ctx.vertex_array(
            self._shadow_prog,
            [(mesh.vbo, "3f 12x 8x 12x", "in_position")],
            index_buffer=mesh.ibo,
            index_element_size=4,
        )
        self._shadow_vaos[key] = vao
        return vao

    def _render_shadow_map(self, lighting):
        """Render depth from key light perspective."""
        if not self._shadow_fbo or not self._shadow_prog:
            return

        light_dir = glm.normalize(glm.vec3(*tuple(glm.normalize(lighting.key_dir))))
        center = self._scene_center
        radius = max(self._scene_radius, 1.0)

        light_pos = center + light_dir * radius * 3.0
        light_view = glm.lookAt(light_pos, center, glm.vec3(0, 0, 1))
        light_proj = glm.ortho(-radius, radius, -radius, radius,
                                0.1, radius * 6.0)
        self._light_space_matrix = light_proj * light_view

    def render(self, camera, lighting, meshes: list[SimpleMesh]):
        """Render meshes to FBO with PBR shading and shadows."""
        if not self.fbo or not self._pbr_prog:
            return

        # Shadow pass
        self._render_shadow_map(lighting)
        if self._shadow_fbo and self._shadow_prog:
            self._shadow_fbo.use()
            self.ctx.clear(depth=1.0)
            self.ctx.enable(moderngl.DEPTH_TEST)
            self.ctx.enable(moderngl.CULL_FACE)
            self.ctx.cull_face = "front"
            for mesh in meshes:
                if mesh.vbo:
                    light_mvp = self._light_space_matrix * mesh.transform
                    self._shadow_prog["u_light_mvp"].value = tuple(
                        c for col in light_mvp for c in col)
                    # Use shadow-specific VAO (position only, skipping normal/uv/tangent)
                    shadow_vao = self._get_shadow_vao(mesh)
                    if shadow_vao:
                        shadow_vao.render()
            self.ctx.cull_face = "back"

        # Main pass
        self.fbo.use()
        self.ctx.clear(0.18, 0.18, 0.20, 1.0)
        self.ctx.enable(moderngl.DEPTH_TEST)
        self.ctx.enable(moderngl.CULL_FACE)

        aspect = self._fbo_size[0] / max(self._fbo_size[1], 1)
        view = camera.get_view_matrix()
        proj = camera.get_projection_matrix(aspect)
        vp = proj * view
        cam_pos = tuple(camera.get_eye_position())

        prog = self._pbr_prog

        # Set lighting uniforms
        if "u_camera_pos" in prog:
            prog["u_camera_pos"].value = cam_pos
        if "u_key_dir" in prog:
            prog["u_key_dir"].value = tuple(lighting.key_dir)
        if "u_key_color" in prog:
            prog["u_key_color"].value = tuple(lighting.key_color)
        if "u_fill_dir" in prog:
            prog["u_fill_dir"].value = tuple(lighting.fill_dir)
        if "u_fill_color" in prog:
            prog["u_fill_color"].value = tuple(lighting.fill_color)
        if "u_ambient_color" in prog:
            prog["u_ambient_color"].value = tuple(lighting.ambient_color)

        # Shadow
        if "shadowEnabled" in prog:
            prog["shadowEnabled"].value = 1.0
        if self._shadow_depth_tex:
            self._shadow_depth_tex.use(5)
            if "shadowMap" in prog:
                prog["shadowMap"].value = 5

        # Draw each mesh
        for mesh in meshes:
            if not mesh.vao:
                continue

            model = mesh.transform
            mvp = vp * model
            normal_mat = glm.mat3(glm.transpose(glm.inverse(model)))

            if "u_mvp" in prog:
                prog["u_mvp"].value = tuple(c for col in mvp for c in col)
            if "u_model" in prog:
                prog["u_model"].value = tuple(c for col in model for c in col)
            if "u_normal_matrix" in prog:
                prog["u_normal_matrix"].value = tuple(c for col in normal_mat for c in col)
            if "u_light_space_matrix" in prog:
                prog["u_light_space_matrix"].value = tuple(
                    c for col in self._light_space_matrix for c in col)

            # Bind material
            mat = mesh.material

            def _bind_tex(tex, unit, uniform, has_uniform):
                t = tex or self._default_tex
                if t:
                    t.use(unit)
                if uniform in prog:
                    prog[uniform].value = unit
                if has_uniform in prog:
                    prog[has_uniform].value = 1.0 if tex else 0.0

            _bind_tex(mat.albedo_tex, 0, "albedoMap", "has_albedo")
            _bind_tex(mat.normal_tex, 1, "normalMap", "has_normal")
            _bind_tex(mat.mr_tex, 2, "mrMap", "has_mr")
            _bind_tex(mat.ao_tex, 3, "aoMap", "has_ao")
            _bind_tex(mat.emissive_tex, 4, "emissiveMap", "has_emissive")

            if "u_base_color" in prog:
                prog["u_base_color"].value = mat.base_color
            if "u_metallic" in prog:
                prog["u_metallic"].value = mat.metallic
            if "u_roughness" in prog:
                prog["u_roughness"].value = mat.roughness
            if "u_emissive_factor" in prog:
                prog["u_emissive_factor"].value = mat.emissive_factor

            mesh.vao.render()

        # Grid (after opaque meshes)
        if self.grid and self.grid_visible:
            vp_tuple = tuple(c for col in vp for c in col)
            self.ctx.disable(moderngl.CULL_FACE)
            self.grid.render(vp_tuple)
            self.ctx.enable(moderngl.CULL_FACE)

        self.ctx.screen.use()

    def get_fbo_texture_id(self) -> int:
        """Return OpenGL texture handle for imgui.image()."""
        if self.fbo_texture:
            return self.fbo_texture.glo
        return 0

    def get_scene_bounds(self) -> tuple[glm.vec3, float]:
        """Return (center, radius) of loaded scene."""
        return self._scene_center, self._scene_radius
