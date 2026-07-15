from __future__ import annotations

from pathlib import Path
from typing import Sequence


def generate_anim_text_data(
    plugin_path: str | Path,
    *,
    game: str,
    source_meshes_root: str | Path,
    output_meshes_root: str | Path,
    base_meshes_root: str | Path | None = None,
    base_plugin_paths: Sequence[str | Path] = (),
    mod_prefix: str | None = None,
    progress_callback=None,
) -> int:
    from creation_lib._native import ck_native

    return ck_native.ck_generate_anim_text_data(
        str(plugin_path),
        game,
        str(source_meshes_root),
        str(output_meshes_root),
        str(base_meshes_root) if base_meshes_root is not None else None,
        [str(path) for path in base_plugin_paths],
        mod_prefix,
        progress_callback,
    )
