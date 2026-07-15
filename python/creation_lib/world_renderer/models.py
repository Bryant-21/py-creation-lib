from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any


SUPPORTED_GAMES = {"oblivion", "fo3", "fnv", "skyrimse", "fo4", "fo76"}


@dataclass(frozen=True)
class RenderSettings:
    include_terrain: bool = True
    include_statics: bool = True
    include_static_collections: bool = True
    include_markers: bool = True
    include_water: bool = True
    include_disabled_refs: bool = False
    include_lights: bool = True
    include_foliage: bool = True
    render_mode: str = "lit"
    debug_buffer: str = ""

    def to_json(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class CellBounds:
    min_x: int = -1
    min_y: int = -1
    max_x: int = 1
    max_y: int = 1

    def to_json(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class CameraState:
    position: tuple[float, float, float] = (0.0, -4096.0, 2048.0)
    target: tuple[float, float, float] = (0.0, 0.0, 0.0)
    up: tuple[float, float, float] = (0.0, 0.0, 1.0)
    fov_degrees: float = 60.0
    near: float = 1.0
    far: float = 250000.0

    def to_json(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class OfflineRenderJob:
    output_path: str
    width: int = 1920
    height: int = 1080
    camera: CameraState = field(default_factory=CameraState)
    settings: RenderSettings = field(default_factory=RenderSettings)

    def to_json(self) -> dict[str, Any]:
        data = asdict(self)
        data["camera"] = self.camera.to_json()
        data["settings"] = self.settings.to_json()
        return data
