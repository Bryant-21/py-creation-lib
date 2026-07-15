from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from creation_lib.renderer.world_batches import WorldBatch, WorldBatchManifest


@dataclass
class WorldGpuBatch:
    batch: WorldBatch
    vao: Any | None
    mesh_buffer: Any | None
    instance_buffer: Any | None
    material_id: str
    instance_count: int

    @property
    def uploaded(self) -> bool:
        return self.mesh_buffer is not None and self.instance_buffer is not None


class WorldUploadCache:
    def __init__(self, ctx: Any | None = None) -> None:
        self.ctx = ctx
        self._buffers: dict[str, Any] = {}

    def upload_manifest(self, scene: Any, manifest: WorldBatchManifest) -> list[WorldGpuBatch]:
        return [self.upload_batch(scene, batch) for batch in manifest.batches]

    def upload_batch(self, scene: Any, batch: WorldBatch) -> WorldGpuBatch:
        mesh_buffer = self._upload_buffer(scene, batch.mesh_buffer)
        instance_buffer = self._upload_buffer(scene, batch.instance_buffer)
        return WorldGpuBatch(
            batch=batch,
            vao=None,
            mesh_buffer=mesh_buffer,
            instance_buffer=instance_buffer,
            material_id=batch.material_id,
            instance_count=batch.instance_count,
        )

    def release_unused(self, active_buffer_ids: set[str]) -> None:
        stale_ids = [buffer_id for buffer_id in self._buffers if buffer_id not in active_buffer_ids]
        for buffer_id in stale_ids:
            _release(self._buffers.pop(buffer_id))

    def clear(self) -> None:
        self.release_unused(set())

    def _upload_buffer(self, scene: Any, buffer_id: str) -> Any | None:
        if self.ctx is None:
            return None
        if buffer_id not in self._buffers:
            data = scene.get_buffer(buffer_id)
            self._buffers[buffer_id] = self.ctx.buffer(data)
        return self._buffers[buffer_id]


def _release(value: Any) -> None:
    release = getattr(value, "release", None)
    if callable(release):
        release()
