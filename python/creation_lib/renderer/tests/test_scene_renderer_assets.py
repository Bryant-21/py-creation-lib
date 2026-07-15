from __future__ import annotations

from pathlib import Path

from creation_lib.renderer import scene_renderer


def test_starfield_environment_asset_is_packaged() -> None:
    assert scene_renderer._starfield_environment_asset().is_file()


def test_scene_renderer_does_not_use_cwd_resource_path() -> None:
    source = Path(scene_renderer.__file__).read_text(encoding="utf-8")
    cwd_relative_asset = "/".join(["resource", "monochrome_studio_02_1k.exr"])

    assert cwd_relative_asset not in source
