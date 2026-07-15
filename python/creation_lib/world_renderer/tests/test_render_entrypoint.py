from __future__ import annotations

from click.testing import CliRunner

from creation_lib.world_renderer import WorldReport
from creation_lib.world_renderer.render import cli


def test_render_entrypoint_validates_size() -> None:
    result = CliRunner().invoke(
        cli,
        [
            "out.png",
            "--game",
            "fo4",
            "--worldspace",
            "TinyWorld",
            "--width",
            "0",
            "--height",
            "128",
        ],
    )

    assert result.exit_code != 0
    assert "width and height must be positive" in result.output


def test_render_entrypoint_loads_scene_and_prints_report(monkeypatch, tmp_path) -> None:
    calls: list[tuple] = []

    class FakeScene:
        def close(self):
            calls.append(("scene_close",))

    class FakeSession:
        def __enter__(self):
            calls.append(("session_enter",))
            return self

        def __exit__(self, exc_type, exc, tb):
            calls.append(("session_exit",))

        def load_worldspace(self, worldspace, bounds):
            calls.append(("load_worldspace", worldspace, bounds.to_json()))
            return FakeScene()

    class FakeBuilder:
        def __init__(self, game, plugin_paths, data_paths, archive_paths):
            calls.append(("builder", game, plugin_paths, data_paths, archive_paths))

        def open(self):
            calls.append(("open",))
            return FakeSession()

    def fake_renderer(scene, job):
        calls.append(("render", job.output_path, job.width, job.height))
        return WorldReport(ok=True, data={"output_path": job.output_path})

    import creation_lib.world_renderer.render as render_module

    monkeypatch.setattr(render_module, "WorldSceneBuilder", FakeBuilder)
    monkeypatch.setattr(render_module, "_load_offscreen_renderer", lambda: fake_renderer)

    result = CliRunner().invoke(
        cli,
        [
            str(tmp_path / "out.png"),
            "--game",
            "fo4",
            "--worldspace",
            "TinyWorld",
            "--width",
            "64",
            "--height",
            "32",
        ],
    )

    assert result.exit_code == 0, result.output
    assert '"ok": true' in result.output
    assert calls == [
        ("builder", "fo4", [], [], []),
        ("open",),
        ("session_enter",),
        ("load_worldspace", "TinyWorld", {"min_x": -1, "min_y": -1, "max_x": 1, "max_y": 1}),
        ("render", str(tmp_path / "out.png"), 64, 32),
        ("scene_close",),
        ("session_exit",),
    ]


def test_render_entrypoint_writes_report_file(monkeypatch, tmp_path) -> None:
    class FakeScene:
        def close(self):
            pass

    class FakeSession:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            pass

        def load_worldspace(self, *_args):
            return FakeScene()

    class FakeBuilder:
        def __init__(self, *_args, **_kwargs):
            pass

        def open(self):
            return FakeSession()

    def fake_renderer(_scene, job):
        return WorldReport(ok=True, data={"output_path": job.output_path})

    import creation_lib.world_renderer.render as render_module

    monkeypatch.setattr(render_module, "WorldSceneBuilder", FakeBuilder)
    monkeypatch.setattr(render_module, "_load_offscreen_renderer", lambda: fake_renderer)
    report_path = tmp_path / "render-report.json"

    result = CliRunner().invoke(
        cli,
        [
            str(tmp_path / "out.png"),
            "--game",
            "fo4",
            "--worldspace",
            "TinyWorld",
            "--report",
            str(report_path),
        ],
    )

    assert result.exit_code == 0, result.output
    assert '"ok": true' in report_path.read_text(encoding="utf-8")
