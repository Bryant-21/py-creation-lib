"""Patch discovery and utilities for multi-plugin mods.

A mod can have optional patch plugins stored under ``patches/<name>/yaml/``.
Each patch's ``plugin.yaml`` carries the patch's plugin name (and extension).
"""
from __future__ import annotations

import re
from pathlib import Path


def list_patches(mod_dir: Path) -> list[str]:
    """Return sorted list of patch names by scanning ``patches/`` subfolders.

    A valid patch has ``patches/<name>/yaml/plugin.yaml``.
    Returns empty list if no ``patches/`` directory exists.
    """
    patches_dir = mod_dir / "patches"
    if not patches_dir.is_dir():
        return []
    return sorted(
        d.name
        for d in patches_dir.iterdir()
        if d.is_dir() and (d / "yaml" / "plugin.yaml").is_file()
    )


def get_patch_yaml_dir(mod_dir: Path, patch_name: str) -> Path:
    """Return ``patches/<name>/yaml/`` path for a named patch."""
    return mod_dir / "patches" / patch_name / "yaml"


def get_patch_plugin_ext(mod_dir: Path, patch_name: str) -> str:
    """Read plugin extension from a patch's ``plugin.yaml``. Falls back to ``"esp"``."""
    from creation_lib.esp.authoring import get_plugin_ext
    return get_plugin_ext(mod_dir, yaml_dir=get_patch_yaml_dir(mod_dir, patch_name))


def get_patch_plugin_name(mod_dir: Path, patch_name: str) -> str:
    """Return the full plugin filename for a patch (e.g. ``"M50_Madsen_Munitions.esp"``).

    Reads the ``plugin:`` field from ``plugin.yaml``. Falls back to
    ``<patch_name>.esp`` when the manifest is missing or malformed.
    """
    plugin_yaml = mod_dir / "patches" / patch_name / "yaml" / "plugin.yaml"
    if plugin_yaml.is_file():
        try:
            text = plugin_yaml.read_text(encoding="utf-8", errors="replace")[:2048]
        except OSError:
            text = ""
        m = re.search(r"^plugin:\s*(\S+)", text, re.MULTILINE)
        if m:
            return m.group(1)
    return f"{patch_name}.esp"


def get_all_plugin_names(mod_dir: Path, mod_name: str) -> list[str]:
    """Return list of all plugin filenames (main + patches) for a mod."""
    from creation_lib.esp.authoring import get_plugin_ext

    main_ext = get_plugin_ext(mod_dir)
    names = [f"{mod_name}.{main_ext}"]
    for patch_name in list_patches(mod_dir):
        names.append(get_patch_plugin_name(mod_dir, patch_name))
    return names
