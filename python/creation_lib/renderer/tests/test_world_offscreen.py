from __future__ import annotations

from dataclasses import dataclass
import struct

import pytest

from creation_lib.renderer.world_offscreen import render_world_scene_offscreen


@dataclass(frozen=True)
class _OfflineRenderJob:
    output_path: str
    width: int
    height: int


@dataclass(frozen=True)
class _WorldReport:
    ok: bool


def test_offscreen_render_writes_png(tmp_path) -> None:
    scene = _WorldScene()
    output_path = tmp_path / "tiny.png"

    report = render_world_scene_offscreen(
        scene,
        _OfflineRenderJob(output_path=str(output_path), width=64, height=64),
    )

    assert report.ok is True
    assert scene.rendered_jobs == [str(output_path)]
    png = output_path.read_bytes()
    assert png.startswith(b"\x89PNG\r\n\x1a\n")
    assert struct.unpack(">II", png[16:24]) == (64, 64)


def test_offscreen_render_validates_positive_dimensions(tmp_path) -> None:
    with pytest.raises(ValueError, match="width and height must be positive"):
        render_world_scene_offscreen(
            _WorldScene(),
            _OfflineRenderJob(output_path=str(tmp_path / "tiny.png"), width=0, height=64),
        )


class _WorldScene:
    def __init__(self) -> None:
        self.rendered_jobs: list[str] = []

    def render_offline(self, job: _OfflineRenderJob) -> _WorldReport:
        self.rendered_jobs.append(job.output_path)
        return _WorldReport(ok=True)

    def query_visible(self, *_args) -> dict:
        return {
            "ok": True,
            "counts": {"visible_instances": 2},
            "data": {
                "batches": [
                    {"kind": "terrain", "instance_count": 1},
                    {"kind": "static", "instance_count": 1},
                ]
            },
        }
