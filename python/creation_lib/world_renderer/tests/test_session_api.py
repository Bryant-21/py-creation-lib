from __future__ import annotations

from creation_lib.world_renderer import RenderSettings, WorldSceneBuilder
from creation_lib.world_renderer.reports import WorldReport


def test_builder_creates_and_closes_native_session(monkeypatch) -> None:
    calls: list[tuple] = []

    def create_world_session(config):
        calls.append(("create", config))
        return 10

    def destroy_world_session(session_id):
        calls.append(("destroy_session", session_id))

    def list_worldspaces(session_id):
        calls.append(("list", session_id))
        return {"ok": True, "errors": [], "warnings": [], "timings_ms": {}, "counts": {}, "data": {"worldspaces": []}}

    from creation_lib.world_renderer import session as session_module

    monkeypatch.setattr(session_module.native_runtime, "create_world_session", create_world_session)
    monkeypatch.setattr(session_module.native_runtime, "destroy_world_session", destroy_world_session)
    monkeypatch.setattr(session_module.native_runtime, "list_worldspaces", list_worldspaces)

    builder = WorldSceneBuilder(
        game="fo4",
        plugin_paths=[],
        data_paths=[],
        archive_paths=[],
    )

    with builder.open() as session:
        report = session.list_worldspaces()
        session.close()

    assert report.ok is True
    assert report.data["worldspaces"] == []
    assert calls == [
        ("create", {"game": "fo4", "plugin_paths": [], "data_paths": [], "archive_paths": []}),
        ("list", 10),
        ("destroy_session", 10),
    ]


def test_builder_rejects_unsupported_game() -> None:
    builder = WorldSceneBuilder(game="starfield", plugin_paths=[], data_paths=[], archive_paths=[])

    try:
        builder.open()
    except ValueError as exc:
        assert "Unsupported world renderer game" in str(exc)
    else:
        raise AssertionError("expected unsupported game to raise ValueError")


def test_render_settings_round_trip_to_json() -> None:
    settings = RenderSettings(
        include_terrain=True,
        include_statics=True,
        include_markers=True,
        include_water=True,
        include_disabled_refs=False,
    )

    assert settings.to_json()["include_water"] is True
    assert settings.to_json()["include_disabled_refs"] is False


def test_world_report_round_trip_defaults() -> None:
    report = WorldReport.from_json({"ok": True, "data": {"worldspaces": []}})

    assert report.to_json()["warnings"] == []
    assert report.to_json()["data"]["worldspaces"] == []


def test_native_tiny_scene_round_trip() -> None:
    builder = WorldSceneBuilder(game="fo4", plugin_paths=[], data_paths=[], archive_paths=[])

    with builder.open() as session:
        worldspaces = session.list_worldspaces()
        scene = session.load_worldspace("TinyWorld")
        try:
            stats = scene.stats()
            visible = scene.query_visible()
            selected = scene.inspect_instance(2)
            mesh_bytes = scene.get_buffer("mesh:static:0")
        finally:
            scene.close()

    assert worldspaces.ok is True
    assert worldspaces.data["worldspaces"] == []
    assert stats.counts["instances"] == 5
    assert stats.counts["terrain_tiles"] == 1
    assert visible.counts["visible_instances"] == 4
    assert selected.data["base_form_key"] == "Tiny.esm:000100"
    assert mesh_bytes == b"\x01\x02\x03\x04"


def test_world_renderer_library_does_not_read_project_env(monkeypatch) -> None:
    import os

    reads: list[str] = []
    original_get = os.environ.get

    def spy_get(key, default=None):
        reads.append(key)
        return original_get(key, default)

    monkeypatch.setattr(os.environ, "get", spy_get)

    builder = WorldSceneBuilder(game="fo4", plugin_paths=[], data_paths=[], archive_paths=[])
    with builder.open() as session:
        session.list_worldspaces()

    assert "DEFAULT_GAME" not in reads
    assert "MODKIT_DB_DIR" not in reads
