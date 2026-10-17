"""Builds a previsbine repair patch for a load order from a check report.

The patch is one full ESP or ESM (never ESL) holding the rebuilt CELLs, with
``<Patch> - Main.ba2`` (group NIFs and tomes), ``<Patch> - Geometry.csg`` and
``<Patch>.cdx`` beside it. Only the output folder is written; install it like
any mod, loaded last.
"""

from __future__ import annotations

import datetime as _dt
import json
import shutil
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from creation_lib.previs.check import check_load_order
from creation_lib.previs.native_runtime import load_native_module
from creation_lib.previs.sources import LoadOrderSpec

DEFAULT_PATCH_NAME = "ModBox21 Previs Repair.esp"
_PREVIS_ACTIONS = {"rebuild_previs", "rebuild_precombines"}

Progress = Callable[[str, int, int], None]
Log = Callable[[str], None]


@dataclass
class BuildResult:
    output_dir: Path
    patch: Path | None
    archive: Path | None
    seed: dict
    precombines: dict | None
    previs: dict | None
    patch_report: dict | None
    recheck: dict | None
    skipped: list[dict] = field(default_factory=list)


def _header_flags(path: Path) -> int:
    with open(path, "rb") as handle:
        header = handle.read(12)
    return int.from_bytes(header[8:12], "little") if len(header) == 12 else 0


def is_light_plugin(path: Path) -> bool:
    return path.suffix.lower() == ".esl" or _header_flags(path) & 0x200 != 0


def loads_as_master(path: Path) -> bool:
    """Whether the game loads the plugin with the masters (.esm/.esl, or the
    ESM or ESL flag), ahead of every plain ESP."""
    return path.suffix.lower() in (".esm", ".esl") or _header_flags(path) & 0x201 != 0


def select_cells(report: dict) -> tuple[list[str], list[str]]:
    """Form keys whose precombines are rebuilt, and every CELL the seed needs:
    those plus each previs target, as its whole exterior cluster."""
    precombine = sorted(c["form_key"] for c in report["cells"] if c.get("action") == "rebuild_precombines")
    clusters = {c["form_key"]: c for c in report["clusters"]}
    seed = set(precombine)
    for cell in report["cells"]:
        if cell.get("action") not in _PREVIS_ACTIONS:
            continue
        cluster = clusters.get(cell.get("cluster") or "")
        seed.update(cluster["members"] if cluster and cluster.get("grid") else [cell["form_key"]])
    for cluster in report["clusters"]:
        if cluster.get("action") in _PREVIS_ACTIONS:
            seed.update(cluster["members"])
    return precombine, sorted(seed)


def build_patch(
    spec: LoadOrderSpec,
    report: dict,
    output_dir: Path,
    *,
    work_dir: Path,
    patch_name: str = DEFAULT_PATCH_NAME,
    into_plugin: Path | None = None,
    pack: bool = True,
    required_masters: list[str] = (),
    plugin_type: str = "esp",
    workers: int | None = None,
    small_model_cull: bool = False,
    progress: Progress | None = None,
    log: Log | None = None,
) -> BuildResult:
    """Rebuilds the report's CELLs into a new patch plugin, or with ``into_plugin``
    into a copy of that load-order plugin written to ``output_dir``.

    A patch is a plain ESP or, with ``plugin_type="esm"``, an ESM; it is an ESP
    whenever it masters a plain ESP, which an ESM would load ahead of. Its
    extension follows the type; ``patch_name`` gives its name.

    A plugin copy keeps its own records and takes only the stamps; its files
    are named after it. With ``pack`` they go in ``<Plugin> - Main.ba2``
    unless the plugin already ships one, else they stay loose.
    A patch masters ``required_masters`` (load-order names) whether or not it
    copies anything from them, as a patch for a mod against PPF.esm masters both.
    """
    native = load_native_module()
    say = log or (lambda _message: None)
    output_dir, work_dir = Path(output_dir), Path(work_dir)
    if into_plugin is not None:
        into_plugin = Path(into_plugin)
        patch_name = into_plugin.name
        if not any(p.name.lower() == patch_name.lower() for p in spec.plugins):
            raise ValueError(f"{patch_name} is not in the load order")
        if is_light_plugin(into_plugin):
            raise ValueError(f"{patch_name} is a light (ESL) plugin; the game ignores previsbines in those")
        plugins = list(spec.plugins)
    else:
        if plugin_type not in ("esp", "esm"):
            raise ValueError("the patch must be an esp or an esm")
        patch_name = Path(patch_name).stem + "." + plugin_type
        stem_lower = Path(patch_name).stem.lower()
        plugins = [p for p in spec.plugins if not (p.stem.lower() == stem_lower and p.suffix.lower() in (".esp", ".esm"))]
        if len(plugins) != len(spec.plugins):
            say(f"Left the existing {Path(patch_name).stem} out of the load order")
    stem = Path(patch_name).stem
    precombine_cells, seed_cells = select_cells(report)
    if not seed_cells:
        raise ValueError("the report has no CELLs to rebuild")
    shutil.rmtree(work_dir, ignore_errors=True)
    data_dir = work_dir / "data"
    data_dir.mkdir(parents=True)
    today = _dt.date.today()
    date = ((today.year - 2000) << 9) | (today.month << 5) | today.day

    def stage(name: str):
        return (lambda done, total: progress(name, done, total)) if progress else None

    say(f"Copying {len(seed_cells)} CELL(s) into the seed")
    seed = json.loads(native.write_patch_seed(json.dumps({
        "plugins": [str(p) for p in plugins],
        "base_plugins": spec.base_plugins,
        "cells": seed_cells,
        "precombine_cells": precombine_cells,
        "output": str(work_dir / patch_name),
        "date": date,
        "stand_in": into_plugin is not None,
        "required_masters": [] if into_plugin is not None else list(required_masters),
    })))
    for skip in seed["skipped"]:
        say(f"Skipped {skip['form_key']}: {skip['reason']}")
    seed_ids = {cell["form_key"]: cell["form_id"] for cell in seed["cells"] if not cell.get("context")}
    if not seed_ids:
        return BuildResult(output_dir, None, None, seed, None, None, None, None, seed["skipped"])
    if into_plugin is None and patch_name.lower().endswith(".esm"):
        by_name = {p.name.lower(): p for p in plugins}
        plain = [m for m in seed["masters"] if m.lower() in by_name and not loads_as_master(by_name[m.lower()])]
        if plain:
            patch_name = Path(patch_name).stem + ".esp"
            say(f"Writing {patch_name} instead of an ESM: it masters {', '.join(plain)}, which an ESM would load ahead of")

    loose = [Path(s["path"]) for s in spec.data_sources if s["kind"] == "loose"]
    archives = [s["path"] for s in spec.data_sources if s["kind"] == "archive"]
    generator = {
        "plugin": seed["plugin"],
        "master_dirs": list(dict.fromkeys(str(p.parent) for p in plugins)),
        # Highest priority first; the patch's own output beats everything.
        "loose_roots": [str(data_dir)] + [str(p) for p in reversed(loose)],
        "archives": list(reversed(archives)),
        "single_cdx": True,
    }

    precombines = None
    rebuild = [seed_ids[key] for key in precombine_cells if key in seed_ids]
    index_resources = None
    if rebuild:
        say(f"Building precombines for {len(rebuild)} CELL(s)")
        prepared = native.prepare_precombines(
            json.dumps({**generator, "output_dir": str(data_dir), "cells": rebuild}),
            str(work_dir / "precombine_report.json"),
            str(work_dir / "precombine_timing.json"),
            workers=workers,
            progress=stage("precombine"),
        )
        precombines = json.loads(prepared.finalize(today.year, today.month, today.day))
        index_resources = prepared.take_index_resources()
        say(f"Precombined {precombines['cells']} CELL(s) into {precombines['groups']} group(s); "
            f"{precombines['skipped_unsupported']} outside the supported subset")

    say("Solving previs")
    prepared = native.prepare_previs(
        json.dumps({
            **generator,
            "scene_dir": str(work_dir / "scenes"),
            "worlds": [],
            "interiors": True,
            "accept_reference_layer": False,
            "small_model_cull": small_model_cull,
            "solve_into": str(data_dir),
            # The rest of the seed is context: the ring previs reads targets from.
            "cells": list(seed_ids.values()),
        }),
        str(work_dir / "previs_plan.json"),
        str(work_dir / "previs_timing.json"),
        workers=workers,
        progress=stage("previs"),
        index_resources=index_resources,
    )
    previs = json.loads(prepared.finalize(today.year, today.month, today.day))
    for warning in previs["warnings"]:
        say(warning)
    say(f"Solved {previs['generated']} previs cluster(s); {previs['skipped_unsupported']} outside the supported subset")

    output_dir.mkdir(parents=True, exist_ok=True)
    patch = output_dir / patch_name
    if into_plugin is not None:
        if patch.resolve() == into_plugin.resolve():
            raise ValueError("write the stamped copy somewhere other than the plugin itself")
        patch_report = json.loads(native.write_into_plugin(json.dumps({
            "seed": seed, "target": str(into_plugin), "output": str(patch),
        })))
        say(f"Wrote {patch_name} with new previsbines for {len(patch_report['cells'])} CELL(s); "
            f"{len(patch_report['unchanged'])} could not be rebuilt")
        if patch_report["added_cells"]:
            say(f"{patch_name} now also overrides {len(patch_report['added_cells'])} CELL(s) its previs clusters need: "
                + ", ".join(patch_report["added_cells"]))
        if patch_report["added_children"]:
            say(f"{patch_name} now overrides {patch_report['added_children']} master reference(s) only to keep "
                "their date before the new precombines")
    else:
        patch_report = json.loads(native.write_patch_plugin(json.dumps({
            "seed": seed, "output": str(patch), "esm": patch_name.lower().endswith(".esm"),
        })))
        say(f"Wrote {patch_name} with {len(patch_report['cells'])} CELL(s); "
            f"{len(patch_report['unchanged'])} could not be rebuilt and were left out")
    for sidecar in (f"{stem} - Geometry.csg", f"{stem}.cdx"):
        source = work_dir / sidecar
        target = output_dir / sidecar
        if source.is_file():
            shutil.copy2(source, target)
        elif target.exists():
            target.unlink()

    from creation_lib.ba2 import native_runtime as ba2

    archive = output_dir / f"{stem} - Main.ba2"
    loose = None
    if archive.exists():
        archive.unlink()
    shipped = into_plugin is not None and (into_plugin.parent / archive.name).exists()
    if not any(data_dir.rglob("*.*")):
        archive = None
    elif pack and not shipped:
        say(f"Packing {archive.name}")
        ba2.pack_archive(str(data_dir), str(archive), "fo4", compress=True)
    else:
        if shipped:
            say(f"{patch_name} already ships {archive.name}; the meshes and tomes stay loose for you to add to it")
        archive = None
        loose = output_dir
        shutil.copytree(data_dir, loose, dirs_exist_ok=True)

    say("Checking the load order with the new previsbines")
    stamped_plugins = [patch if p.name.lower() == patch_name.lower() else p for p in plugins]
    if into_plugin is None:
        stamped_plugins.append(patch)
    new_sources = [{"kind": "archive", "path": str(archive)}] if archive else []
    if loose is not None:
        new_sources = [{"kind": "loose", "path": str(loose)}]
    recheck_spec = LoadOrderSpec(
        spec.label,
        spec.game_root,
        stamped_plugins,
        spec.data_sources + new_sources,
        spec.base_plugins,
    )
    recheck = check_load_order(recheck_spec, workers=workers, progress=progress)
    (output_dir / f"{stem} - report.json").write_text(json.dumps({
        "seed": seed,
        "precombines": precombines,
        "previs": previs,
        "patch": patch_report,
        "recheck_summary": recheck["summary"],
    }, indent=2), encoding="utf-8")
    return BuildResult(output_dir, patch, archive, seed, precombines, previs, patch_report, recheck, seed["skipped"])
