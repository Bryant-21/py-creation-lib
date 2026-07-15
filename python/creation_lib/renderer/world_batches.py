from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class WorldBatch:
    id: str
    kind: str
    mesh_buffer: str
    instance_buffer: str
    material_id: str
    instance_count: int
    pick_buffer: str | None = None
    debug_buffers: dict[str, str] = field(default_factory=dict)


@dataclass(frozen=True)
class WorldBatchManifest:
    batches: list[WorldBatch]

    @classmethod
    def from_report_data(cls, data: dict[str, Any]) -> "WorldBatchManifest":
        return cls(
            batches=[
                WorldBatch(
                    id=str(item["id"]),
                    kind=str(item["kind"]),
                    mesh_buffer=str(item["mesh_buffer"]),
                    instance_buffer=str(item["instance_buffer"]),
                    material_id=str(item["material_id"]),
                    instance_count=int(item["instance_count"]),
                    pick_buffer=_optional_str(item.get("pick_buffer")),
                    debug_buffers=_string_map(item.get("debug_buffers", {})),
                )
                for item in data.get("batches", [])
            ]
        )


def group_batches_by_kind(batches: list[WorldBatch]) -> dict[str, list[WorldBatch]]:
    grouped: dict[str, list[WorldBatch]] = {}
    for batch in batches:
        grouped.setdefault(batch.kind, []).append(batch)
    return grouped


def _optional_str(value: Any) -> str | None:
    if value is None:
        return None
    return str(value)


def _string_map(value: Any) -> dict[str, str]:
    if not isinstance(value, dict):
        return {}
    return {str(key): str(buffer_id) for key, buffer_id in value.items()}
