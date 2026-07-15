from __future__ import annotations

from collections.abc import Sequence
import json
from pathlib import Path
from typing import Callable

import click

from creation_lib.world_renderer import CellBounds, OfflineRenderJob, WorldReport, WorldScene, WorldSceneBuilder


def _load_offscreen_renderer() -> Callable[[WorldScene, OfflineRenderJob], WorldReport] | None:
    try:
        from creation_lib.renderer.world_offscreen import render_world_scene_offscreen
    except ImportError:
        return None
    return render_world_scene_offscreen


@click.command()
@click.argument("output", type=click.Path(path_type=Path))
@click.option("--game", required=True)
@click.option("--plugin", "plugin_paths", multiple=True, type=click.Path(path_type=Path))
@click.option("--data-path", "data_paths", multiple=True, type=click.Path(path_type=Path))
@click.option("--archive", "archive_paths", multiple=True, type=click.Path(path_type=Path))
@click.option("--worldspace", required=True)
@click.option("--min-x", default=-1, type=int)
@click.option("--min-y", default=-1, type=int)
@click.option("--max-x", default=1, type=int)
@click.option("--max-y", default=1, type=int)
@click.option("--width", default=1920, type=int)
@click.option("--height", default=1080, type=int)
@click.option("--report", "report_path", type=click.Path(path_type=Path))
def cli(
    output: Path,
    game: str,
    plugin_paths: Sequence[Path],
    data_paths: Sequence[Path],
    archive_paths: Sequence[Path],
    worldspace: str,
    min_x: int,
    min_y: int,
    max_x: int,
    max_y: int,
    width: int,
    height: int,
    report_path: Path | None,
) -> None:
    if width <= 0 or height <= 0:
        raise click.ClickException("width and height must be positive")

    builder = WorldSceneBuilder(
        game=game,
        plugin_paths=[str(path) for path in plugin_paths],
        data_paths=[str(path) for path in data_paths],
        archive_paths=[str(path) for path in archive_paths],
    )
    job = OfflineRenderJob(output_path=str(output), width=width, height=height)
    with builder.open() as session:
        scene = session.load_worldspace(
            worldspace,
            CellBounds(min_x=min_x, min_y=min_y, max_x=max_x, max_y=max_y),
        )
        try:
            renderer = _load_offscreen_renderer()
            report = renderer(scene, job) if renderer is not None else scene.render_offline(job)
        finally:
            scene.close()

    report_json = report.to_json()
    if report_path is not None:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report_json, indent=2, sort_keys=True), encoding="utf-8")
    click.echo(json.dumps(report_json, indent=2, sort_keys=True))


if __name__ == "__main__":
    cli()
