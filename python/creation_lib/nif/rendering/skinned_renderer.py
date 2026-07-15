"""GPU skinned mesh renderer with optional weight heatmap visualization.

Loads NIF meshes with bone weights, uploads bone matrix palettes to GPU,
and renders skinned meshes in the viewport. Supports weight visualization
mode for displaying per-bone influence as a color heatmap.

"""
from __future__ import annotations

import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

import moderngl
import numpy as np

_log = logging.getLogger("nif.rendering.skinned")

_SHADER_DIR = Path(__file__).parent / "shaders"


@dataclass
class SegmentSubmesh:
    """A contiguous range of indices in a sorted index buffer for one segment."""
    segment_id: int
    start_index: int   # Byte offset into the index buffer (index * 4 for uint32)
    tri_count: int     # Number of triangles in this submesh
    color: tuple[float, float, float] = (0.5, 0.5, 0.5)


@dataclass
class SkinnedMesh:
    """A GPU-uploaded skinned mesh ready for rendering."""
    vao: moderngl.VertexArray
    index_count: int
    bone_names: list[str]  # Bone names from BSSkin, for mapping to skeleton
    inv_bind_transforms: list[np.ndarray] = field(default_factory=list)  # Per-bone 4x4 inverse bind pose
    bind_world_transforms: dict[str, np.ndarray] = field(default_factory=dict)  # NIF skeleton bind pose worlds
    diffuse_texture_id: int | None = None
    segment_submeshes: list[SegmentSubmesh] = field(default_factory=list)  # Sorted segment ranges
    vertex_vbo: Optional[moderngl.Buffer] = None  # Reference to position+normal+uv VBO for runtime updates


class SkinnedRenderer:
    """Compiles skinning shaders and renders skinned meshes.

    Supports two rendering modes:
    - Normal mode: standard diffuse lighting with optional texture
    - Weight mode: heatmap visualization of bone weights for a selected bone
    """

    def __init__(self, ctx: moderngl.Context):
        self.ctx = ctx
        self.program: moderngl.Program | None = None
        self._logged_render = False
        self._compile_shaders()

    def _compile_shaders(self):
        """Compile skinned vertex + fragment shader program."""
        vert_path = _SHADER_DIR / "skinned.vert"
        frag_path = _SHADER_DIR / "skinned.frag"

        try:
            vert_src = vert_path.read_text()
            frag_src = frag_path.read_text()
            self.program = self.ctx.program(
                vertex_shader=vert_src,
                fragment_shader=frag_src,
            )
            _log.info("Skinned shader program compiled. Uniforms: %s",
                      list(self.program))
        except Exception as e:
            _log.error("Failed to compile skinned shaders: %s", e)
            self.program = None

    def build_skinned_mesh(self, nif_file, shape_block) -> SkinnedMesh | None:
        """Build a SkinnedMesh from a BSTriShape block with BSSkin data.

        Args:
            nif_file: NifFile instance
            shape_block: BSTriShape block with vertex data and skin instance

        Returns:
            SkinnedMesh or None if no skin data
        """
        if self.program is None:
            return None

        vertex_data_list = shape_block.get_field("Vertex Data") or []
        triangles_list = shape_block.get_field("Triangles") or []

        if not vertex_data_list or not triangles_list:
            return None

        n_verts = len(vertex_data_list)

        # Extract arrays
        positions = np.zeros((n_verts, 3), dtype=np.float32)
        normals = np.zeros((n_verts, 3), dtype=np.float32)
        uvs = np.zeros((n_verts, 2), dtype=np.float32)
        bone_weights = np.zeros((n_verts, 4), dtype=np.float32)
        bone_indices = np.zeros((n_verts, 4), dtype=np.int32)

        for i, vd in enumerate(vertex_data_list):
            v = vd.get("Vertex") or {}
            positions[i] = [
                float(v.get("x", 0)),
                float(v.get("y", 0)),
                float(v.get("z", 0)),
            ]

            n = vd.get("Normal")
            if n:
                normals[i] = [
                    float(n.get("x", 0)),
                    float(n.get("y", 0)),
                    float(n.get("z", 0)),
                ]

            uv = vd.get("UV")
            if uv:
                uvs[i] = [float(uv.get("u", 0)), float(uv.get("v", 0))]

            # Bone weights — two formats:
            # 1. Combined: [{"index": N, "weight": F}, ...] (NiSkinInstance)
            # 2. Separate: "Bone Weights" = [f, ...], "Bone Indices" = [i, ...] (BSSkin)
            bw_list = vd.get("Bone Weights") or vd.get("BoneWeights") or []
            bi_list = vd.get("Bone Indices") or []
            if isinstance(bw_list, list):
                if bw_list and isinstance(bw_list[0], dict):
                    # Combined format
                    for j, bw in enumerate(bw_list[:4]):
                        bone_indices[i, j] = int(bw.get("index", bw.get("Index", 0)))
                        bone_weights[i, j] = float(bw.get("weight", bw.get("Weight", 0)))
                else:
                    # Separate flat lists
                    for j in range(min(4, len(bw_list))):
                        bone_weights[i, j] = float(bw_list[j])
                    for j in range(min(4, len(bi_list))):
                        bone_indices[i, j] = int(bi_list[j])

        # Build index buffer
        indices = []
        for tri in triangles_list:
            if isinstance(tri, dict):
                indices.extend([
                    int(tri.get("v1", tri.get("V1", 0))),
                    int(tri.get("v2", tri.get("V2", 0))),
                    int(tri.get("v3", tri.get("V3", 0))),
                ])
            elif isinstance(tri, (list, tuple)) and len(tri) >= 3:
                indices.extend([int(tri[0]), int(tri[1]), int(tri[2])])

        if not indices:
            return None

        index_arr = np.array(indices, dtype=np.uint32)

        # Get bone names and inverse bind transforms from BSSkin
        bone_names = self._get_skin_bone_names(nif_file, shape_block)
        inv_bind_transforms = self._get_inv_bind_transforms(nif_file, shape_block)

        # Create GPU buffers
        vbo = self.ctx.buffer(np.column_stack([
            positions, normals, uvs,
        ]).astype(np.float32).tobytes())

        weight_vbo = self.ctx.buffer(bone_weights.tobytes())
        index_vbo = self.ctx.buffer(bone_indices.astype(np.int32).tobytes())
        ibo = self.ctx.buffer(index_arr.tobytes())

        # Partition color VBO (zeros — not used in bone editor)
        segment_color_vbo = self.ctx.buffer(
            np.zeros((n_verts, 3), dtype=np.float32).tobytes()
        )

        vao = self.ctx.vertex_array(
            self.program,
            [
                (vbo, "3f 3f 2f", "in_position", "in_normal", "in_uv"),
                (weight_vbo, "4f", "in_bone_weights"),
                (index_vbo, "4i", "in_bone_indices"),
                (segment_color_vbo, "3f", "in_segment_color"),
            ],
            index_buffer=ibo,
            index_element_size=4,
        )

        _log.info("Built skinned mesh: %d verts, %d tris, %d bones",
                  n_verts, len(indices) // 3, len(bone_names))

        return SkinnedMesh(
            vao=vao,
            index_count=len(indices),
            bone_names=bone_names,
            inv_bind_transforms=inv_bind_transforms,
            vertex_vbo=vbo,
        )

    def build_skinned_mesh_from_skin_data(self, skin,
                                          segment_colors: np.ndarray | None = None,
                                          ) -> SkinnedMesh | None:
        """Build a SkinnedMesh from pre-extracted SkinData arrays.

        This enables loading composite body meshes (body + hands + head) that
        have been merged via reference_body._merge_skin_data().

        Args:
            skin: SkinData instance with vertices, normals, uvs, weights,
                bone_indices, triangles, and bone_names.
            segment_colors: Optional (N, 3) float32 per-vertex segment
                colors. If None, zeros are used.

        Returns:
            SkinnedMesh or None if shader not compiled.
        """
        if self.program is None:
            return None

        n_verts = skin.num_vertices
        if n_verts == 0 or skin.num_triangles == 0:
            return None

        # Build vertex buffer: position(3f) + normal(3f) + uv(2f)
        vbo_data = np.column_stack([
            skin.vertices, skin.normals, skin.uvs,
        ]).astype(np.float32)
        vbo = self.ctx.buffer(vbo_data.tobytes())

        # Weight and bone index buffers
        weight_vbo = self.ctx.buffer(skin.weights.astype(np.float32).tobytes())
        index_vbo = self.ctx.buffer(skin.bone_indices.astype(np.int32).tobytes())

        # Partition color buffer
        if segment_colors is not None and len(segment_colors) == n_verts:
            pc_data = segment_colors.astype(np.float32)
        else:
            pc_data = np.zeros((n_verts, 3), dtype=np.float32)
        segment_color_vbo = self.ctx.buffer(pc_data.tobytes())

        # Index buffer
        index_arr = skin.triangles.flatten().astype(np.uint32)
        ibo = self.ctx.buffer(index_arr.tobytes())

        vao = self.ctx.vertex_array(
            self.program,
            [
                (vbo, "3f 3f 2f", "in_position", "in_normal", "in_uv"),
                (weight_vbo, "4f", "in_bone_weights"),
                (index_vbo, "4i", "in_bone_indices"),
                (segment_color_vbo, "3f", "in_segment_color"),
            ],
            index_buffer=ibo,
            index_element_size=4,
        )

        _log.info("Built skinned mesh from SkinData: %d verts, %d tris, %d bones",
                  n_verts, skin.num_triangles, len(skin.bone_names))

        return SkinnedMesh(
            vao=vao,
            index_count=len(index_arr),
            bone_names=list(skin.bone_names),
            inv_bind_transforms=list(skin.inv_bind_transforms) if skin.inv_bind_transforms else [],
            vertex_vbo=vbo,
        )

    def _get_skin_bone_names(self, nif_file, shape_block) -> list[str]:
        """Extract bone names from the BSSkin::Instance attached to a shape."""
        skin_ref = shape_block.get_field("Skin Instance") or shape_block.get_field("Skin")
        if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
            return []

        skin_id = int(skin_ref) if isinstance(skin_ref, int) else -1
        if skin_id < 0:
            return []

        skin_block = nif_file.get_block(skin_id)
        if skin_block is None:
            return []

        # BSSkin::Instance has a "Bones" field with refs to NiNode bone blocks
        bone_refs = skin_block.get_field("Bones") or []
        names = []
        for ref in bone_refs:
            bone_id = int(ref) if isinstance(ref, (int, float)) else -1
            if bone_id >= 0:
                bone_block = nif_file.get_block(bone_id)
                if bone_block:
                    name = bone_block.get_field("Name") or f"Bone_{bone_id}"
                    if isinstance(name, int):
                        # String table lookup
                        name = nif_file.get_string(name) or f"Bone_{bone_id}"
                    names.append(str(name))
            else:
                names.append(f"Bone_{len(names)}")
        return names

    def _get_inv_bind_transforms(self, nif_file, shape_block) -> list[np.ndarray]:
        """Extract inverse bind pose matrices from BSSkin::BoneData."""
        skin_ref = shape_block.get_field("Skin Instance") or shape_block.get_field("Skin")
        if skin_ref is None or (isinstance(skin_ref, int) and skin_ref < 0):
            return []

        skin_id = int(skin_ref) if isinstance(skin_ref, int) else -1
        if skin_id < 0:
            return []

        skin_block = nif_file.get_block(skin_id)
        if skin_block is None:
            return []

        data_ref = skin_block.get_field("Data")
        if data_ref is None or (isinstance(data_ref, int) and data_ref < 0):
            return []

        data_block = nif_file.get_block(int(data_ref))
        if data_block is None:
            return []

        bone_list = data_block.get_field("Bone List") or []
        transforms = []
        for bone_data in bone_list:
            mat = np.eye(4, dtype=np.float32)
            rot = bone_data.get("Rotation", {})
            trans = bone_data.get("Translation", {})

            # 3x3 rotation — NIF uses column-major naming (mCR = col C, row R)
            mat[0, 0] = float(rot.get("m11", 1))
            mat[0, 1] = float(rot.get("m21", 0))
            mat[0, 2] = float(rot.get("m31", 0))
            mat[1, 0] = float(rot.get("m12", 0))
            mat[1, 1] = float(rot.get("m22", 1))
            mat[1, 2] = float(rot.get("m32", 0))
            mat[2, 0] = float(rot.get("m13", 0))
            mat[2, 1] = float(rot.get("m23", 0))
            mat[2, 2] = float(rot.get("m33", 1))

            # Translation
            mat[0, 3] = float(trans.get("x", 0))
            mat[1, 3] = float(trans.get("y", 0))
            mat[2, 3] = float(trans.get("z", 0))

            transforms.append(mat)

        _log.info("Extracted %d inverse bind transforms from BSSkin::BoneData", len(transforms))
        return transforms

    def render(self, mesh: SkinnedMesh, mvp: tuple, model: tuple,
               normal_matrix: tuple, bone_matrices: list[np.ndarray] | None = None,
               light_dir=(0.5, 0.7, 1.0), light_color=(1.0, 1.0, 1.0),
               ambient=(0.3, 0.3, 0.3),
               weight_mode: bool = False, selected_bone_index: int = -1,
               segment_mode: bool = False, vertex_color_mode: bool = False,
               alpha: float = 1.0, show_mask: bool = False):
        """Render a skinned mesh with bone palette.

        Args:
            mesh: SkinnedMesh to render.
            mvp: Model-view-projection matrix as tuple.
            model: Model matrix as tuple.
            normal_matrix: Normal matrix (3x3) as tuple.
            bone_matrices: List of 4x4 bone matrices for skinning.
            light_dir: Directional light direction.
            light_color: Directional light color.
            ambient: Ambient light color.
            weight_mode: If True, render bone weight heatmap instead of
                normal shading.
            selected_bone_index: Which bone index to visualize weights for.
                -1 = show total weight sum.
            segment_mode: If True, render segment/dismemberment colors.
            vertex_color_mode: If True, render per-vertex colors from NIF.
            alpha: Opacity (0.0-1.0) for transparent overlay rendering.
            show_mask: If True, darken masked vertices.
        """
        if self.program is None or mesh is None:
            return

        def _set(name, value):
            if name in self.program:
                self.program[name].value = value

        _set("u_mvp", mvp)
        _set("u_model", model)
        _set("u_normal_matrix", normal_matrix)
        _set("u_light_dir", light_dir)
        _set("u_light_color", light_color)
        _set("u_ambient", ambient)
        _set("u_has_texture", mesh.diffuse_texture_id is not None)
        _set("u_alpha", alpha)
        _set("u_segment_mode", segment_mode)
        _set("u_use_submesh_color", False)
        _set("u_vertex_color_mode", vertex_color_mode)

        # Weight visualization uniforms
        _set("u_weight_mode", weight_mode)
        _set("u_selected_bone", selected_bone_index)  # -1 = total weight sum
        _set("u_show_mask", show_mask)

        if bone_matrices and len(bone_matrices) > 0:
            _set("u_skinned", True)
            # Write bone matrices as a contiguous buffer (column-major for OpenGL)
            # Pad to 128 with identity matrices (shader declares mat4[128])
            padded = [np.eye(4, dtype=np.float32)] * 128
            for i, m in enumerate(bone_matrices[:128]):
                padded[i] = m.T.astype(np.float32)
            mats = np.array(padded, dtype=np.float32)
            self.program["u_bone_matrices"].write(mats.tobytes())
            if not self._logged_render:
                _log.info("Wrote %d bone matrices (%d bytes) to GPU",
                          len(bone_matrices), mats.nbytes)
        else:
            _set("u_skinned", False)

        if not self._logged_render:
            _log.info("Rendering VAO: index_count=%d, program=%s",
                      mesh.index_count, self.program is not None)
            self._logged_render = True

        mesh.vao.render(moderngl.TRIANGLES)

    def render_submeshes(self, mesh: SkinnedMesh, mvp: tuple, model: tuple,
                         normal_matrix: tuple,
                         bone_matrices: list[np.ndarray] | None = None,
                         light_dir=(0.5, 0.7, 1.0), light_color=(1.0, 1.0, 1.0),
                         ambient=(0.3, 0.3, 0.3),
                         selected_segment_id: int = -1,
                         alpha: float = 1.0,
                         dim_factor: float = 0.25,
                         show_mask: bool = False):
        """Render segment submeshes with flat per-submesh colors (hard boundaries).

        Each segment submesh is rendered as a separate draw call with a
        solid color uniform, eliminating the per-vertex color bleeding that
        occurs at segment boundaries.

        Args:
            mesh: SkinnedMesh with populated segment_submeshes.
            selected_segment_id: Which segment to highlight (-1 = none).
            dim_factor: Dimming multiplier for non-selected segments.
        """
        if self.program is None or mesh is None:
            return
        if not mesh.segment_submeshes:
            return

        def _set(name, value):
            if name in self.program:
                self.program[name].value = value

        # Set common uniforms
        _set("u_mvp", mvp)
        _set("u_model", model)
        _set("u_normal_matrix", normal_matrix)
        _set("u_light_dir", light_dir)
        _set("u_light_color", light_color)
        _set("u_ambient", ambient)
        _set("u_has_texture", False)
        _set("u_alpha", alpha)
        _set("u_segment_mode", True)
        _set("u_use_submesh_color", True)
        _set("u_weight_mode", False)
        _set("u_vertex_color_mode", False)
        _set("u_selected_bone", -1)
        _set("u_show_mask", show_mask)

        if bone_matrices and len(bone_matrices) > 0:
            _set("u_skinned", True)
            padded = [np.eye(4, dtype=np.float32)] * 128
            for i, m in enumerate(bone_matrices[:128]):
                padded[i] = m.T.astype(np.float32)
            mats = np.array(padded, dtype=np.float32)
            self.program["u_bone_matrices"].write(mats.tobytes())
        else:
            _set("u_skinned", False)

        # Render each submesh with its own flat color
        for submesh in mesh.segment_submeshes:
            color = submesh.color
            if selected_segment_id >= 0 and submesh.segment_id != selected_segment_id:
                color = (color[0] * dim_factor, color[1] * dim_factor, color[2] * dim_factor)
            _set("u_submesh_color", color)
            mesh.vao.render(
                moderngl.TRIANGLES,
                vertices=submesh.tri_count * 3,
                first=submesh.start_index,
            )

        # Reset submesh color mode
        _set("u_use_submesh_color", False)

    def compute_bone_matrices(self, skeleton, mesh: SkinnedMesh,
                              deltas: dict | None = None,
                              anim_positions: dict | None = None,
                              anim_rotations: dict | None = None) -> list[np.ndarray]:
        """Compute skinning matrices for a skinned mesh.

        Uses a two-part approach for accuracy:
        1. bind_correction = nif_bind_world * bsskin_inv_bind (precomputed,
           exact identity rotation + model→world translation offset)
        2. anim_delta = current_world * inv(ref_hkx_world) (exact delta)
        3. skin_matrix = anim_delta * bind_correction

        This avoids HKX-vs-NIF rotation errors that compound in extreme poses.
        Falls back to direct HKX world * BSSkin when NIF bind data isn't available.
        """
        from creation_lib.bone_edit.quat_util import quat_multiply, quat_normalize

        has_inv_bind = len(mesh.inv_bind_transforms) == len(mesh.bone_names)
        has_bind_world = bool(mesh.bind_world_transforms)

        matrices = []
        for i, bone_name in enumerate(mesh.bone_names):
            # --- Determine current world transform for this bone ---
            if anim_positions and bone_name in anim_positions:
                pos = anim_positions[bone_name]
                cur_trans = np.array([pos[0], pos[1], pos[2]], dtype=np.float64)
                if anim_rotations and bone_name in anim_rotations:
                    cur_rot = self._mat3_to_quat(anim_rotations[bone_name])
                else:
                    cur_rot = np.array([0, 0, 0, 1], dtype=np.float64)
            elif anim_positions:
                # Animation active but this bone has no anim data (_skin bones).
                # Walk up parent chain to find nearest animated ancestor.
                bone_idx = skeleton.get_bone_index(bone_name)
                if bone_idx is None:
                    matrices.append(np.eye(4, dtype=np.float32))
                    continue

                chain = [bone_idx]
                cur = bone_idx
                found_anim = False
                while True:
                    pid = skeleton.parent_indices[cur]
                    if pid < 0 or pid >= len(skeleton.bone_names):
                        break
                    pname = skeleton.bone_names[pid]
                    if pname in anim_positions:
                        pos = anim_positions[pname]
                        anc_trans = np.array([pos[0], pos[1], pos[2]], dtype=np.float64)
                        anc_rot = anim_rotations[pname] if (anim_rotations and pname in anim_rotations) \
                            else np.eye(3, dtype=np.float64)
                        found_anim = True
                        break
                    chain.append(pid)
                    cur = pid

                if found_anim:
                    from creation_lib.bone_edit.skeleton import _quat_to_matrix
                    world_t = anc_trans.copy()
                    world_r = np.array(anc_rot, dtype=np.float64)
                    for idx in reversed(chain):
                        local_t = skeleton.ref_translations[idx]
                        local_r = _quat_to_matrix(skeleton.ref_rotations[idx])
                        world_t = world_r @ local_t + world_t
                        world_r = world_r @ local_r
                    cur_trans = world_t
                    cur_rot = self._mat3_to_quat(world_r)
                else:
                    world = skeleton.get_bone_world_transform(bone_name)
                    if world is None:
                        matrices.append(np.eye(4, dtype=np.float32))
                        continue
                    cur_trans = world["translation"].copy()
                    cur_rot = self._mat3_to_quat(world["rotation"])
            else:
                world = skeleton.get_bone_world_transform(bone_name)
                if world is None:
                    matrices.append(np.eye(4, dtype=np.float32))
                    continue
                cur_trans = world["translation"].copy()
                cur_rot = self._mat3_to_quat(world["rotation"])

            # Apply user delta if present
            if deltas and bone_name in deltas:
                delta = deltas[bone_name]
                cur_trans = cur_trans + delta.translation
                if not np.allclose(delta.rotation, [0, 0, 0, 1]):
                    cur_rot = quat_normalize(quat_multiply(delta.rotation, cur_rot))

            # --- Compute skin matrix ---
            current_mat = self._quat_trans_to_mat4(cur_rot, cur_trans)

            if has_inv_bind and has_bind_world and bone_name in mesh.bind_world_transforms:
                # Accurate path: delta * bind_correction
                # bind_correction = nif_bind_world * bsskin_inv (precomputed)
                nif_bw = mesh.bind_world_transforms[bone_name]
                bind_correction = (nif_bw @ mesh.inv_bind_transforms[i]).astype(np.float64)

                # ref_world from HKX skeleton
                ref = skeleton.get_bone_world_transform(bone_name)
                if ref is not None:
                    ref_rot = self._mat3_to_quat(ref["rotation"])
                    ref_mat = self._quat_trans_to_mat4(ref_rot, ref["translation"])
                    inv_ref = np.linalg.inv(ref_mat.astype(np.float64))
                    anim_delta = current_mat.astype(np.float64) @ inv_ref
                    mat = (anim_delta @ bind_correction).astype(np.float32)
                else:
                    mat = (current_mat @ mesh.inv_bind_transforms[i]).astype(np.float32)
            elif has_inv_bind:
                # Fallback: direct HKX world * BSSkin inv_bind
                mat = (current_mat @ mesh.inv_bind_transforms[i]).astype(np.float32)
            else:
                mat = current_mat.astype(np.float32)

            matrices.append(mat)

        return matrices

    @staticmethod
    def _quat_trans_to_mat4(q: np.ndarray, t: np.ndarray) -> np.ndarray:
        """Convert quaternion (x,y,z,w) + translation to 4x4 matrix (column-major)."""
        x, y, z, w = q
        mat = np.eye(4, dtype=np.float32)

        # Rotation part
        mat[0, 0] = 1 - 2 * (y * y + z * z)
        mat[0, 1] = 2 * (x * y - z * w)
        mat[0, 2] = 2 * (x * z + y * w)
        mat[1, 0] = 2 * (x * y + z * w)
        mat[1, 1] = 1 - 2 * (x * x + z * z)
        mat[1, 2] = 2 * (y * z - x * w)
        mat[2, 0] = 2 * (x * z - y * w)
        mat[2, 1] = 2 * (y * z + x * w)
        mat[2, 2] = 1 - 2 * (x * x + y * y)

        # Translation
        mat[0, 3] = t[0]
        mat[1, 3] = t[1]
        mat[2, 3] = t[2]

        return mat

    @staticmethod
    def _mat3_to_quat(m: np.ndarray) -> np.ndarray:
        """Convert 3x3 rotation matrix to quaternion (x,y,z,w)."""
        trace = m[0, 0] + m[1, 1] + m[2, 2]
        if trace > 0:
            s = 0.5 / np.sqrt(trace + 1.0)
            w = 0.25 / s
            x = (m[2, 1] - m[1, 2]) * s
            y = (m[0, 2] - m[2, 0]) * s
            z = (m[1, 0] - m[0, 1]) * s
        elif m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
            s = 2.0 * np.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2])
            w = (m[2, 1] - m[1, 2]) / s
            x = 0.25 * s
            y = (m[0, 1] + m[1, 0]) / s
            z = (m[0, 2] + m[2, 0]) / s
        elif m[1, 1] > m[2, 2]:
            s = 2.0 * np.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2])
            w = (m[0, 2] - m[2, 0]) / s
            x = (m[0, 1] + m[1, 0]) / s
            y = 0.25 * s
            z = (m[1, 2] + m[2, 1]) / s
        else:
            s = 2.0 * np.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1])
            w = (m[1, 0] - m[0, 1]) / s
            x = (m[0, 2] + m[2, 0]) / s
            y = (m[1, 2] + m[2, 1]) / s
            z = 0.25 * s
        return np.array([x, y, z, w], dtype=np.float64)


def attach_nif_bind_worlds(skeleton_nif_path: str, meshes: list[SkinnedMesh]):
    """Compute bone world transforms from NIF skeleton and attach to skinned meshes.

    This ensures the reference pose uses the same coordinate system as the
    BSSkin::BoneData inverse bind transforms, so they cancel out perfectly.
    Shared by bone_editor and aligner.

    Args:
        skeleton_nif_path: Path to the skeleton .nif file.
        meshes: List of SkinnedMesh instances to update.
    """
    from creation_lib.nif.nif_file import NifFile

    nif = NifFile.load(skeleton_nif_path)

    # Build parent map
    parent_map: dict[int, int] = {}
    for i, block in enumerate(nif.blocks):
        if block.type_name == "NiNode":
            children = block.get_field("Children") or []
            if isinstance(children, list):
                for child_id in children:
                    if isinstance(child_id, int) and child_id >= 0:
                        parent_map[child_id] = i

    # Build name-to-block map
    name_to_block: dict[str, int] = {}
    for i, block in enumerate(nif.blocks):
        if block.type_name == "NiNode":
            name = block.get_field("Name")
            if name:
                name_to_block[str(name)] = i

    # Compute world transforms via parent chain walk
    world_cache: dict[int, np.ndarray] = {}

    def _nif_local(block_id: int) -> np.ndarray:
        block = nif.get_block(block_id)
        if block is None:
            return np.eye(4, dtype=np.float64)
        trans = block.get_field("Translation") or {}
        rot = block.get_field("Rotation") or {}
        m = np.eye(4, dtype=np.float64)
        m[0, 0] = float(rot.get("m11", 1)); m[0, 1] = float(rot.get("m21", 0)); m[0, 2] = float(rot.get("m31", 0))
        m[1, 0] = float(rot.get("m12", 0)); m[1, 1] = float(rot.get("m22", 1)); m[1, 2] = float(rot.get("m32", 0))
        m[2, 0] = float(rot.get("m13", 0)); m[2, 1] = float(rot.get("m23", 0)); m[2, 2] = float(rot.get("m33", 1))
        m[0, 3] = float(trans.get("x", 0)); m[1, 3] = float(trans.get("y", 0)); m[2, 3] = float(trans.get("z", 0))
        return m

    def _nif_world(block_id: int) -> np.ndarray:
        if block_id in world_cache:
            return world_cache[block_id]
        local = _nif_local(block_id)
        pid = parent_map.get(block_id, -1)
        if pid < 0:
            world_cache[block_id] = local
        else:
            world_cache[block_id] = _nif_world(pid) @ local
        return world_cache[block_id]

    for mesh in meshes:
        bind_worlds = {}
        for bone_name in mesh.bone_names:
            bid = name_to_block.get(bone_name)
            if bid is not None:
                bind_worlds[bone_name] = _nif_world(bid).astype(np.float64)
        mesh.bind_world_transforms = bind_worlds
        _log.info("Attached %d/%d NIF bind world transforms to skinned mesh",
                  len(bind_worlds), len(mesh.bone_names))
