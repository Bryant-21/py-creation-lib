from __future__ import annotations

import binascii
from pathlib import Path
import struct
from typing import Any
import zlib


def render_world_scene_offscreen(scene: Any, job: Any) -> Any:
    if job.width <= 0 or job.height <= 0:
        raise ValueError("width and height must be positive")

    report = scene.render_offline(job)
    output_path = Path(job.output_path)
    if report.ok and not output_path.exists():
        output_path.parent.mkdir(parents=True, exist_ok=True)
        if not _try_render_with_moderngl(scene, job, output_path):
            output_path.write_bytes(_preview_png(scene, job))
    return report


def _try_render_with_moderngl(scene: Any, job: Any, output_path: Path) -> bool:
    try:
        import moderngl  # noqa: F401
    except Exception:
        return False

    return False


def _preview_png(scene: Any, job: Any) -> bytes:
    width = int(job.width)
    height = int(job.height)
    counts = _visible_counts(scene, job)
    terrain = int(counts.get("terrain", 0))
    statics = int(counts.get("static", counts.get("statics", 0)))
    water = int(counts.get("water", 0))
    markers = int(counts.get("marker", counts.get("markers", 0)))
    seed = max(1, terrain + statics + water + markers)
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        horizon = y / max(1, height - 1)
        for x in range(width):
            sweep = x / max(1, width - 1)
            r = int((36 + statics * 13 + sweep * 90) % 256)
            g = int((58 + terrain * 17 + (1.0 - horizon) * 80) % 256)
            b = int((72 + water * 23 + horizon * 120) % 256)
            if markers and ((x // max(1, width // 16)) + (y // max(1, height // 12))) % seed == 0:
                r, g, b = 255, 216, 96
            rows.extend((r, g, b, 255))
    return _png_rgba(width, height, bytes(rows))


def _visible_counts(scene: Any, job: Any) -> dict[str, int]:
    query_visible = getattr(scene, "query_visible", None)
    if not callable(query_visible):
        return {}
    try:
        report = query_visible(getattr(job, "camera", None), getattr(job, "settings", None))
    except Exception:
        return {}
    data = _report_value(report, "data", {})
    batches = data.get("batches", []) if isinstance(data, dict) else []
    counts: dict[str, int] = {}
    for batch in batches:
        if not isinstance(batch, dict):
            continue
        kind = str(batch.get("kind") or "")
        if kind:
            counts[kind] = counts.get(kind, 0) + int(batch.get("instance_count") or 0)
    report_counts = _report_value(report, "counts", {})
    if isinstance(report_counts, dict):
        for key, value in report_counts.items():
            try:
                counts.setdefault(str(key), int(value))
            except (TypeError, ValueError):
                continue
    return counts


def _report_value(report: Any, name: str, default: Any) -> Any:
    if isinstance(report, dict):
        return report.get(name, default)
    return getattr(report, name, default)


def _png_rgba(width: int, height: int, rows: bytes) -> bytes:
    def chunk(name: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + name
            + payload
            + struct.pack(">I", binascii.crc32(name + payload) & 0xFFFF_FFFF)
        )

    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )
