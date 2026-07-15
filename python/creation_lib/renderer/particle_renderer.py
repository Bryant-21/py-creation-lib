from __future__ import annotations

from dataclasses import dataclass
from typing import Any

import numpy as np

_CORNERS = np.array(
    [
        [-0.5, -0.5],
        [0.5, -0.5],
        [0.5, 0.5],
        [-0.5, 0.5],
    ],
    dtype=np.float32,
)
_BASE_INDICES = np.array([0, 1, 2, 0, 2, 3], dtype=np.uint32)


def pack_particle_vertices(batch: Any) -> tuple[np.ndarray, np.ndarray]:
    particle_count = int(len(batch.positions))
    vertices = np.zeros((particle_count * 4, 13), dtype=np.float32)
    indices = np.zeros((particle_count * 6,), dtype=np.uint32)

    for particle_index in range(particle_count):
        vertex_start = particle_index * 4
        index_start = particle_index * 6
        atlas_index = int(batch.atlas_indices[particle_index])
        atlas_offset_index = atlas_index % len(batch.atlas_offsets)
        u0, u1, v0, v1 = batch.atlas_offsets[atlas_offset_index]
        uvs = np.array(
            [
                [u0, v1],
                [u1, v1],
                [u1, v0],
                [u0, v0],
            ],
            dtype=np.float32,
        )

        particle_vertices = vertices[vertex_start:vertex_start + 4]
        particle_vertices[:, 0:3] = batch.positions[particle_index]
        particle_vertices[:, 3:5] = _CORNERS
        particle_vertices[:, 5:7] = uvs
        particle_vertices[:, 7:11] = batch.colors[particle_index]
        particle_vertices[:, 11] = batch.sizes[particle_index]
        particle_vertices[:, 12] = batch.rotations[particle_index]
        indices[index_start:index_start + 6] = _BASE_INDICES + vertex_start

    return vertices, indices


@dataclass
class _GpuBatch:
    vao: Any
    vbo: Any
    ibo: Any
    index_count: int
    texture: Any = None
    greyscale_texture: Any = None
    greyscale_color: bool = False
    greyscale_alpha: bool = False


class ParticleRenderer:
    def __init__(self, ctx: Any, program: Any):
        self.ctx = ctx
        self._program = program
        self._batches: dict[tuple[str, int], _GpuBatch] = {}

    def clear(self) -> None:
        for gpu_batch in self._batches.values():
            self._release_gpu_batch(gpu_batch)
        self._batches.clear()

    def update_batches(self, draw_batches: list[Any]) -> None:
        next_batches: dict[tuple[str, int], _GpuBatch] = {}

        for draw_batch in draw_batches:
            key = (draw_batch.nif_id, draw_batch.system_block_id)
            vertices, indices = pack_particle_vertices(draw_batch)
            old_batch = self._batches.pop(key, None)
            if old_batch is not None:
                self._release_gpu_batch(old_batch)
            if len(indices) == 0:
                continue
            next_batches[key] = self._create_gpu_batch(
                vertices,
                indices,
                texture=getattr(draw_batch, "texture", None),
                greyscale_texture=getattr(draw_batch, "greyscale_texture", None),
                greyscale_color=bool(getattr(draw_batch, "greyscale_color", False)),
                greyscale_alpha=bool(getattr(draw_batch, "greyscale_alpha", False)),
            )

        for old_batch in self._batches.values():
            self._release_gpu_batch(old_batch)
        self._batches = next_batches

    def render(
        self,
        vp_tuple: tuple[float, ...],
        camera_right: tuple[float, float, float],
        camera_up: tuple[float, float, float],
    ) -> None:
        if not self._batches:
            return

        import moderngl

        self._set_uniform("u_vp", vp_tuple)
        self._set_uniform("u_camera_right", camera_right)
        self._set_uniform("u_camera_up", camera_up)
        self._set_uniform("ParticleTexture", 0)
        self._set_uniform("GreyscaleTexture", 1)

        self.ctx.enable(moderngl.BLEND)
        self.ctx.blend_func = (moderngl.SRC_ALPHA, moderngl.ONE_MINUS_SRC_ALPHA)
        self.ctx.depth_mask = False
        try:
            for gpu_batch in self._batches.values():
                if gpu_batch.texture is not None:
                    gpu_batch.texture.use(0)
                    self._set_uniform("hasParticleTexture", True)
                else:
                    self._set_uniform("hasParticleTexture", False)
                if gpu_batch.greyscale_texture is not None:
                    gpu_batch.greyscale_texture.use(1)
                    self._set_uniform("hasGreyscaleTexture", True)
                else:
                    self._set_uniform("hasGreyscaleTexture", False)
                self._set_uniform("greyscaleColor", gpu_batch.greyscale_color)
                self._set_uniform("greyscaleAlpha", gpu_batch.greyscale_alpha)
                gpu_batch.vao.render(moderngl.TRIANGLES)
        finally:
            self.ctx.depth_mask = True
            self.ctx.disable(moderngl.BLEND)

    def _create_gpu_batch(
        self,
        vertices: np.ndarray,
        indices: np.ndarray,
        *,
        texture: Any = None,
        greyscale_texture: Any = None,
        greyscale_color: bool = False,
        greyscale_alpha: bool = False,
    ) -> _GpuBatch:
        vbo = self.ctx.buffer(vertices.tobytes())
        ibo = self.ctx.buffer(indices.tobytes())
        vao = self.ctx.vertex_array(
            self._program,
            [
                (
                    vbo,
                    "3f 2f 2f 4f 1f 1f",
                    "in_center",
                    "in_corner",
                    "in_uv",
                    "in_color",
                    "in_size",
                    "in_rotation",
                )
            ],
            ibo,
        )
        return _GpuBatch(
            vao=vao,
            vbo=vbo,
            ibo=ibo,
            index_count=int(len(indices)),
            texture=texture,
            greyscale_texture=greyscale_texture,
            greyscale_color=greyscale_color,
            greyscale_alpha=greyscale_alpha,
        )

    def _set_uniform(self, name: str, value: Any) -> None:
        if name in self._program:
            self._program[name].value = value

    @staticmethod
    def _release_gpu_batch(gpu_batch: _GpuBatch) -> None:
        for resource in (gpu_batch.vao, gpu_batch.vbo, gpu_batch.ibo):
            release = getattr(resource, "release", None)
            if release is not None:
                release()
