from __future__ import annotations

from creation_lib.renderer.world_batches import WorldBatchManifest, group_batches_by_kind
from creation_lib.renderer.world_upload import WorldUploadCache


def test_world_batch_manifest_parses_native_report() -> None:
    manifest = WorldBatchManifest.from_report_data(
        {
            "batches": [
                {
                    "id": "static:0",
                    "kind": "static",
                    "mesh_buffer": "mesh:static:0",
                    "instance_buffer": "instances:static:0",
                    "material_id": "static:default",
                    "instance_count": 3,
                }
            ]
        }
    )

    assert manifest.batches[0].kind == "static"
    assert manifest.batches[0].instance_count == 3
    assert manifest.batches[0].debug_buffers == {}


def test_world_batch_manifest_parses_optional_pick_and_debug_buffers() -> None:
    manifest = WorldBatchManifest.from_report_data(
        {
            "batches": [
                {
                    "id": "static:0",
                    "kind": "static",
                    "mesh_buffer": "mesh:static:0",
                    "instance_buffer": "instances:static:0",
                    "material_id": "static:default",
                    "instance_count": 3,
                    "pick_buffer": "pick:static:0",
                    "debug_buffers": {"normal": "debug:normal:0", "depth": "debug:depth:0"},
                }
            ]
        }
    )

    batch = manifest.batches[0]
    assert batch.pick_buffer == "pick:static:0"
    assert batch.debug_buffers == {"normal": "debug:normal:0", "depth": "debug:depth:0"}


def test_world_batches_group_by_kind() -> None:
    manifest = WorldBatchManifest.from_report_data(
        {
            "batches": [
                {"id": "terrain:0", "kind": "terrain", "mesh_buffer": "m1", "instance_buffer": "i1", "material_id": "mat", "instance_count": 1},
                {"id": "static:0", "kind": "static", "mesh_buffer": "m2", "instance_buffer": "i2", "material_id": "mat", "instance_count": 2},
            ]
        }
    )

    grouped = group_batches_by_kind(manifest.batches)

    assert [batch.kind for batch in grouped["terrain"]] == ["terrain"]
    assert [batch.kind for batch in grouped["static"]] == ["static"]


def test_world_upload_cache_can_be_instantiated_without_context() -> None:
    manifest = WorldBatchManifest.from_report_data(
        {
            "batches": [
                {"id": "static:0", "kind": "static", "mesh_buffer": "m1", "instance_buffer": "i1", "material_id": "mat", "instance_count": 2},
            ]
        }
    )
    cache = WorldUploadCache()

    gpu_batches = cache.upload_manifest(_NoBufferScene(), manifest)

    assert len(gpu_batches) == 1
    assert gpu_batches[0].uploaded is False


class _NoBufferScene:
    def get_buffer(self, buffer_id: str) -> bytes:
        raise AssertionError(f"unexpected buffer request: {buffer_id}")
