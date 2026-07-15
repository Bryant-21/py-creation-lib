from __future__ import annotations

import json

from creation_lib.world_renderer import native_runtime


class FakeNative:
    def __init__(self) -> None:
        self.calls: list[tuple] = []

    def create_world_session(self, config_json: str) -> int:
        self.calls.append(("create_world_session", json.loads(config_json)))
        return 41

    def destroy_world_session(self, session_id: int) -> None:
        self.calls.append(("destroy_world_session", session_id))

    def list_worldspaces(self, session_id: int) -> str:
        self.calls.append(("list_worldspaces", session_id))
        return json.dumps({"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {}, "data": {"worldspaces": []}})

    def load_worldspace(self, session_id: int, worldspace: str, bounds_json: str, settings_json: str) -> int:
        self.calls.append(("load_worldspace", session_id, worldspace, json.loads(bounds_json), json.loads(settings_json)))
        return 42

    def destroy_scene(self, scene_id: int) -> None:
        self.calls.append(("destroy_scene", scene_id))

    def scene_stats(self, scene_id: int) -> str:
        self.calls.append(("scene_stats", scene_id))
        return json.dumps({"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {"cells": 1}, "data": {}})

    def query_visible(self, scene_id: int, camera_json: str, settings_json: str) -> str:
        self.calls.append(("query_visible", scene_id, json.loads(camera_json), json.loads(settings_json)))
        return json.dumps({"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {}, "data": {"batches": []}})

    def get_buffer(self, scene_id: int, buffer_id: str) -> bytes:
        self.calls.append(("get_buffer", scene_id, buffer_id))
        return b"abc"

    def inspect_instance(self, scene_id: int, instance_id: int) -> str:
        self.calls.append(("inspect_instance", scene_id, instance_id))
        return json.dumps({"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {}, "data": {"instance_id": instance_id}})

    def render_offline(self, scene_id: int, render_job_json: str) -> str:
        self.calls.append(("render_offline", scene_id, json.loads(render_job_json)))
        return json.dumps({"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {}, "data": {"output_path": "out.png"}})


def test_native_runtime_wraps_final_native_function_names(monkeypatch) -> None:
    fake = FakeNative()
    monkeypatch.setattr(native_runtime, "_native", lambda: fake)

    assert native_runtime.create_world_session({"game": "fo4"}) == 41
    native_runtime.destroy_world_session(41)
    assert native_runtime.list_worldspaces(41)["data"]["worldspaces"] == []
    assert native_runtime.load_worldspace(41, "TinyWorld", {"min_x": -1}, {"include_water": True}) == 42
    native_runtime.destroy_scene(42)
    assert native_runtime.scene_stats(42)["counts"]["cells"] == 1
    assert native_runtime.query_visible(42, {"position": [0, 0, 0]}, {"include_statics": True})["data"]["batches"] == []
    assert native_runtime.get_buffer(42, "positions") == b"abc"
    assert native_runtime.inspect_instance(42, 7)["data"]["instance_id"] == 7
    assert native_runtime.render_offline(42, {"output_path": "out.png"})["data"]["output_path"] == "out.png"

    assert [call[0] for call in fake.calls] == [
        "create_world_session",
        "destroy_world_session",
        "list_worldspaces",
        "load_worldspace",
        "destroy_scene",
        "scene_stats",
        "query_visible",
        "get_buffer",
        "inspect_instance",
        "render_offline",
    ]
