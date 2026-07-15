from __future__ import annotations

import json
from typing import Any


def _native() -> Any:
    from creation_lib._native import world_renderer_native

    return world_renderer_native


def _loads_report(raw: str) -> dict[str, Any]:
    return json.loads(raw)


def create_world_session(config: dict[str, Any]) -> int:
    return int(_native().create_world_session(json.dumps(config)))


def destroy_world_session(session_id: int) -> None:
    _native().destroy_world_session(session_id)


def list_worldspaces(session_id: int) -> dict[str, Any]:
    return _loads_report(_native().list_worldspaces(session_id))


def load_worldspace(
    session_id: int,
    worldspace: str,
    bounds: dict[str, Any],
    settings: dict[str, Any],
) -> int:
    return int(
        _native().load_worldspace(
            session_id,
            worldspace,
            json.dumps(bounds),
            json.dumps(settings),
        )
    )


def destroy_scene(scene_id: int) -> None:
    _native().destroy_scene(scene_id)


def scene_stats(scene_id: int) -> dict[str, Any]:
    return _loads_report(_native().scene_stats(scene_id))


def query_visible(
    scene_id: int,
    camera: dict[str, Any],
    settings: dict[str, Any],
) -> dict[str, Any]:
    return _loads_report(_native().query_visible(scene_id, json.dumps(camera), json.dumps(settings)))


def get_buffer(scene_id: int, buffer_id: str) -> bytes:
    return bytes(_native().get_buffer(scene_id, buffer_id))


def inspect_instance(scene_id: int, instance_id: int) -> dict[str, Any]:
    return _loads_report(_native().inspect_instance(scene_id, instance_id))


def render_offline(scene_id: int, render_job: dict[str, Any]) -> dict[str, Any]:
    return _loads_report(_native().render_offline(scene_id, json.dumps(render_job)))
