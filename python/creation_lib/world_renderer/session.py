from __future__ import annotations

from dataclasses import dataclass
from typing import Self

from creation_lib.world_renderer import native_runtime
from creation_lib.world_renderer.models import (
    SUPPORTED_GAMES,
    CameraState,
    CellBounds,
    OfflineRenderJob,
    RenderSettings,
)
from creation_lib.world_renderer.reports import WorldReport


@dataclass(frozen=True)
class WorldSceneBuilder:
    game: str
    plugin_paths: list[str]
    data_paths: list[str]
    archive_paths: list[str]

    def open(self) -> "WorldSession":
        game = self.game.strip().lower()
        if game not in SUPPORTED_GAMES:
            raise ValueError(f"Unsupported world renderer game: {self.game}")
        session_id = native_runtime.create_world_session(
            {
                "game": game,
                "plugin_paths": self.plugin_paths,
                "data_paths": self.data_paths,
                "archive_paths": self.archive_paths,
            }
        )
        return WorldSession(session_id)


@dataclass
class WorldSession:
    session_id: int
    _closed: bool = False

    def __enter__(self) -> Self:
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def close(self) -> None:
        if self._closed:
            return
        native_runtime.destroy_world_session(self.session_id)
        self._closed = True

    def list_worldspaces(self) -> WorldReport:
        return WorldReport.from_json(native_runtime.list_worldspaces(self.session_id))

    def load_worldspace(
        self,
        worldspace: str,
        bounds: CellBounds | None = None,
        settings: RenderSettings | None = None,
    ) -> "WorldScene":
        scene_id = native_runtime.load_worldspace(
            self.session_id,
            worldspace,
            (bounds or CellBounds()).to_json(),
            (settings or RenderSettings()).to_json(),
        )
        return WorldScene(scene_id)


@dataclass
class WorldScene:
    scene_id: int
    _closed: bool = False

    def __enter__(self) -> Self:
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()

    def close(self) -> None:
        if self._closed:
            return
        native_runtime.destroy_scene(self.scene_id)
        self._closed = True

    def stats(self) -> WorldReport:
        return WorldReport.from_json(native_runtime.scene_stats(self.scene_id))

    def query_visible(
        self,
        camera: CameraState | dict | None = None,
        settings: RenderSettings | None = None,
    ) -> WorldReport:
        camera_json = camera if isinstance(camera, dict) else (camera or CameraState()).to_json()
        return WorldReport.from_json(
            native_runtime.query_visible(
                self.scene_id,
                camera_json,
                (settings or RenderSettings()).to_json(),
            )
        )

    def get_buffer(self, buffer_id: str) -> bytes:
        return native_runtime.get_buffer(self.scene_id, buffer_id)

    def inspect_instance(self, instance_id: int) -> WorldReport:
        return WorldReport.from_json(native_runtime.inspect_instance(self.scene_id, instance_id))

    def render_offline(self, job: OfflineRenderJob) -> WorldReport:
        return WorldReport.from_json(native_runtime.render_offline(self.scene_id, job.to_json()))
