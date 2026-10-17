"""Thin boundary over ``material_upgrade_native`` (FO4 materials -> BGSM v22+).

All decisions (classes, formulas, metal rules, naming, the upgrade-vs-replace
selection, DDS/BGSM/NIF writing) live in Rust; this module only serialises
options and parses the JSON report.
"""

from __future__ import annotations

import json
import os
from dataclasses import asdict, dataclass, field
from importlib import import_module
from typing import Any, Callable

CLASSES = ("metal", "painted", "wood", "fabric", "stone", "organic", "other", "skin", "grass")
DEFAULT_CLASSES = ("metal", "painted", "wood", "fabric", "stone", "organic", "other")
METAL_MODES = ("off", "envmap", "mask", "user-mask")

ProgressCallback = Callable[[int, int, str], "bool | None"]


@dataclass
class UpgradeOptions:
    sources: list[str]
    # Read before ``sources`` and overridden by them, never upgraded (the game Data a mod references).
    base_sources: list[str] = field(default_factory=list)
    # Read after ``sources`` and winning over them, never upgraded (a converted mod's own assets).
    overlay_sources: list[str] = field(default_factory=list)
    # Overlay materials upgraded all the same (FO4-era leftovers the converter kept legacy).
    overlay_upgrade_paths: list[str] = field(default_factory=list)
    output: str = ""
    classes: list[str] = field(default_factory=lambda: list(DEFAULT_CLASSES))
    metal_mode: str = "off"
    metal_mask_dir: str | None = None
    smoothness_gain: float = 1.0
    knee: float | None = None
    include_materials: bool = True
    include_nifs: bool = False
    include_terrain: bool = False
    plugins: list[str] = field(default_factory=list)
    terrain_layer_size: int = 1024
    terrain_sidecars: bool = True
    terrain_worldspaces: list[str] = field(default_factory=list)
    exclude_paths: list[str] = field(default_factory=list)
    include_prefixes: list[str] = field(default_factory=list)
    workers: int = 4

    def to_json(self) -> str:
        payload = asdict(self)
        payload["sources"] = [os.fspath(p) for p in self.sources]
        payload["base_sources"] = [os.fspath(p) for p in self.base_sources]
        payload["overlay_sources"] = [os.fspath(p) for p in self.overlay_sources]
        payload["plugins"] = [os.fspath(p) for p in self.plugins]
        return json.dumps(payload)


def _native() -> Any:
    module = getattr(import_module("creation_lib._native"), "material_upgrade_native", None)
    if module is None:
        raise RuntimeError("creation_lib._native.material_upgrade_native is missing; rebuild the creation native extension")
    return module


def plan(options: UpgradeOptions) -> dict:
    """Dry run: per-unit decisions, outputs and a summary. Writes nothing."""
    return json.loads(_native().plan_upgrade_v22(options.to_json()))


def run(options: UpgradeOptions, progress: ProgressCallback | None = None) -> dict:
    """Upgrade into ``options.output``; returns the report (also written to the mod)."""
    return json.loads(_native().run_upgrade_v22(options.to_json(), progress))


def selected_keys(options: UpgradeOptions) -> list[str]:
    """Keys still upgraded after ``exclude_paths`` (the replace set) is applied."""
    return list(_native().select_upgrade_keys(options.to_json()))
