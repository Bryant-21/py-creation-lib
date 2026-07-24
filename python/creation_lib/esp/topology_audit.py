from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from creation_lib.esp import native_runtime


_CELL_TOPOLOGY_METRICS = (
    "child_groups",
    "persistent_groups",
    "temporary_groups",
    "visible_distant_groups",
    "land_records",
    "land_in_temporary_group",
    "misplaced_land_records",
    "navm_records",
    "navm_in_temporary_group",
    "misplaced_navm_records",
)

_CELL_SECTION_RECORD_METRICS = (
    "persistent_records",
    "temporary_records",
    "visible_distant_records",
)

_WORLD_TOPOLOGY_SUMMARY = (
    "exterior_cells",
    "unique_cell_coordinates",
    "non_exterior_cells",
    "cell_child_groups",
    "persistent_groups",
    "temporary_groups",
    "visible_distant_groups",
    "land_records",
    "navm_records",
    "valid_land_records",
    "valid_navm_records",
)

_WORLD_ANOMALIES = (
    "duplicate_cell_coordinates",
    "cells_missing_coordinates",
    "cells_missing_child_groups",
    "cells_missing_land",
    "cells_with_duplicate_land",
    "orphan_cell_groups",
    "orphan_land_records",
    "orphan_navm_records",
    "misplaced_land_records",
    "misplaced_navm_records",
)

_PLUGIN_ANOMALIES = (
    "flat_exterior_cells",
    "flat_land_records",
    "flat_navm_records",
    "duplicate_record_form_ids",
)


def audit_plugin_topology(
    plugin_path: str | Path, *, game: str | None
) -> dict[str, Any]:
    return native_runtime.audit_plugin_topology(str(Path(plugin_path)), game)


def audit_topology_pair(
    source_path: str | Path,
    output_path: str | Path,
    *,
    source_game: str | None,
    target_game: str | None,
) -> dict[str, Any]:
    source = audit_plugin_topology(source_path, game=source_game)
    output = audit_plugin_topology(output_path, game=target_game)
    return {
        "schema_version": 1,
        "source": source,
        "output": output,
        "equality": compare_topology_reports(source, output),
    }


def compare_topology_reports(
    source: dict[str, Any], output: dict[str, Any]
) -> dict[str, Any]:
    source_worlds = _world_index(source)
    output_worlds = _world_index(output)
    comparisons: list[dict[str, Any]] = []
    missing_worldspaces: list[str] = []
    extra_worldspaces: list[str] = []
    ambiguous_worldspaces: list[dict[str, Any]] = []
    missing_cell_count = 0
    extra_cell_count = 0
    extra_cells_in_matched_worldspaces = 0
    extra_cells_in_extra_worldspaces = 0
    changed_cell_count = 0
    changed_section_record_count = 0

    for key in sorted(set(source_worlds) | set(output_worlds)):
        source_matches = source_worlds.get(key, [])
        output_matches = output_worlds.get(key, [])
        identity = _world_identity((source_matches or output_matches)[0])
        if len(source_matches) > 1 or len(output_matches) > 1:
            ambiguous_worldspaces.append(
                {
                    "worldspace": identity,
                    "source_count": len(source_matches),
                    "output_count": len(output_matches),
                }
            )
            missing_cell_count += sum(
                len(world.get("cells", [])) for world in source_matches
            )
            extra_cell_count += sum(
                len(world.get("cells", [])) for world in output_matches
            )
            continue
        if not source_matches:
            extra_worldspaces.append(identity)
            extra_world_cells = len(output_matches[0].get("cells", []))
            extra_cell_count += extra_world_cells
            extra_cells_in_extra_worldspaces += extra_world_cells
            continue
        if not output_matches:
            missing_worldspaces.append(identity)
            missing_cell_count += len(source_matches[0].get("cells", []))
            continue
        comparison = _compare_worldspace(source_matches[0], output_matches[0])
        comparisons.append(comparison)
        missing_cell_count += len(comparison["missing_cells"])
        extra_cell_count += len(comparison["extra_cells"])
        extra_cells_in_matched_worldspaces += len(comparison["extra_cells"])
        changed_cell_count += len(comparison["changed_cells"])
        changed_section_record_count += len(comparison["changed_section_record_counts"])

    source_plugin_anomalies = _anomaly_counts(
        source.get("anomalies", {}), _PLUGIN_ANOMALIES
    )
    output_plugin_anomalies = _anomaly_counts(
        output.get("anomalies", {}), _PLUGIN_ANOMALIES
    )
    plugin_summary_equal = source.get("summary", {}) == output.get("summary", {})
    plugin_anomalies_equal = source_plugin_anomalies == output_plugin_anomalies
    matched_worldspaces_equal = (
        not missing_worldspaces
        and not ambiguous_worldspaces
        and all(comparison["equal"] for comparison in comparisons)
    )
    equal = (
        matched_worldspaces_equal
        and not extra_worldspaces
        and plugin_summary_equal
        and plugin_anomalies_equal
    )
    return {
        "equal": equal,
        "source_covered": matched_worldspaces_equal,
        "matched_worldspaces_equal": matched_worldspaces_equal,
        "matched_worldspaces": len(comparisons),
        "missing_worldspaces": missing_worldspaces,
        "extra_worldspaces": extra_worldspaces,
        "ambiguous_worldspaces": ambiguous_worldspaces,
        "missing_cells": missing_cell_count,
        "extra_cells": extra_cell_count,
        "extra_cells_in_matched_worldspaces": extra_cells_in_matched_worldspaces,
        "extra_cells_in_extra_worldspaces": extra_cells_in_extra_worldspaces,
        "changed_cells": changed_cell_count,
        "changed_section_record_counts": changed_section_record_count,
        "plugin_summary_equal": plugin_summary_equal,
        "plugin_anomalies_equal": plugin_anomalies_equal,
        "source_plugin_anomalies": source_plugin_anomalies,
        "output_plugin_anomalies": output_plugin_anomalies,
        "worldspaces": comparisons,
    }


def render_topology_report(report: dict[str, Any], fmt: str) -> str:
    normalized = fmt.lower()
    if normalized in {"markdown", "table"}:
        return render_topology_markdown(report)
    if normalized == "pretty":
        return json.dumps(report, ensure_ascii=False, indent=2)
    if normalized in {"json", "compact"}:
        return json.dumps(report, ensure_ascii=False, separators=(",", ":"))
    raise ValueError(f"Unsupported topology report format: {fmt}")


def render_topology_markdown(report: dict[str, Any]) -> str:
    source = report["source"]
    output = report["output"]
    equality = report["equality"]
    status = "MATCH" if equality["equal"] else "DIFFERENT"
    lines = [
        "# ESP topology audit",
        "",
        f"**Result:** {status}",
        "",
        f"- Source: `{_markdown_text(source.get('path') or source.get('plugin'))}` ({source.get('game') or 'unspecified'})",
        f"- Output: `{_markdown_text(output.get('path') or output.get('plugin'))}` ({output.get('game') or 'unspecified'})",
        "",
        "## Summary",
        "",
        "| Metric | Source | Output |",
        "|---|---:|---:|",
    ]
    summary_metrics = (
        ("Worldspaces", "worldspaces"),
        ("Exterior cells", "exterior_cells"),
        ("Unique coordinates", "unique_cell_coordinates"),
        ("Persistent groups", "persistent_groups"),
        ("Temporary groups", "temporary_groups"),
        ("LAND records", "land_records"),
        ("LAND in type 9", "valid_land_records"),
        ("NAVM records", "navm_records"),
        ("NAVM in type 9", "valid_navm_records"),
        ("Missing LAND cells", "missing_land_cells"),
        ("Duplicate cell coordinates", "duplicate_cell_coordinates"),
        ("Duplicate LAND cells", "duplicate_land_cells"),
        ("Flat exterior cells", "flat_exterior_cells"),
        ("Flat LAND", "flat_land_records"),
        ("Flat NAVM", "flat_navm_records"),
        ("Orphan LAND", "orphan_land_records"),
        ("Orphan NAVM", "orphan_navm_records"),
        ("Misplaced LAND", "misplaced_land_records"),
        ("Misplaced NAVM", "misplaced_navm_records"),
    )
    for label, key in summary_metrics:
        lines.append(
            f"| {label} | {source.get('summary', {}).get(key, 0)} | {output.get('summary', {}).get(key, 0)} |"
        )

    lines.extend(
        [
            "",
            "## Source to output",
            "",
            f"- Matched worldspaces: {equality['matched_worldspaces']}",
            f"- Missing worldspaces: {len(equality['missing_worldspaces'])}",
            f"- Extra worldspaces: {len(equality['extra_worldspaces'])}",
            f"- Missing cell coordinates: {equality['missing_cells']}",
            f"- Extra cell coordinates: {equality['extra_cells']}",
            f"- Extra coordinates in matched worldspaces: {equality['extra_cells_in_matched_worldspaces']}",
            f"- Changed cell topology: {equality['changed_cells']}",
            f"- Changed persistent/temporary record counts: {equality['changed_section_record_counts']}",
            "",
            "| Worldspace | Equal | Missing cells | Extra cells | Changed topology | Changed section counts |",
            "|---|:---:|---:|---:|---:|---:|",
        ]
    )
    for world in equality["worldspaces"]:
        lines.append(
            "| {worldspace} | {equal} | {missing} | {extra} | {changed} | {section_changed} |".format(
                worldspace=_markdown_text(world["worldspace"]),
                equal="yes" if world["equal"] else "no",
                missing=len(world["missing_cells"]),
                extra=len(world["extra_cells"]),
                changed=len(world["changed_cells"]),
                section_changed=len(world["changed_section_record_counts"]),
            )
        )
    if not equality["worldspaces"]:
        lines.append("| _none_ | no | 0 | 0 | 0 | 0 |")

    for world in equality["worldspaces"]:
        if world["equal"]:
            continue
        lines.extend(["", f"### {_markdown_text(world['worldspace'])}", ""])
        if world["missing_cells"]:
            lines.append(f"- Missing: {_coordinate_list(world['missing_cells'])}")
        if world["extra_cells"]:
            lines.append(f"- Extra: {_coordinate_list(world['extra_cells'])}")
        if world["changed_cells"]:
            lines.append(f"- Changed: {_coordinate_list(world['changed_cells'])}")
        if world["changed_section_record_counts"]:
            lines.append(
                "- Persistent/temporary record counts changed: "
                + _coordinate_list(world["changed_section_record_counts"])
            )
        if not world["summary_equal"]:
            lines.append("- Aggregate worldspace counts differ.")
        if not world["anomalies_equal"]:
            lines.append("- Worldspace anomaly counts differ.")
    if equality["missing_worldspaces"]:
        lines.extend(
            ["", "Missing worldspaces: " + ", ".join(equality["missing_worldspaces"])]
        )
    if equality["extra_worldspaces"]:
        lines.extend(
            ["", "Extra worldspaces: " + ", ".join(equality["extra_worldspaces"])]
        )
    if equality["ambiguous_worldspaces"]:
        lines.extend(
            [
                "",
                "Ambiguous worldspaces: "
                + ", ".join(
                    item["worldspace"] for item in equality["ambiguous_worldspaces"]
                ),
            ]
        )
    return "\n".join(lines) + "\n"


def _world_index(report: dict[str, Any]) -> dict[str, list[dict[str, Any]]]:
    index: dict[str, list[dict[str, Any]]] = {}
    for world in report.get("worldspaces", []):
        editor_id = str(world.get("editor_id") or "").strip()
        key = (
            f"edid:{editor_id.casefold()}"
            if editor_id
            else f"form:{str(world.get('form_id', '')).casefold()}"
        )
        index.setdefault(key, []).append(world)
    return index


def _world_identity(world: dict[str, Any]) -> str:
    return str(world.get("editor_id") or world.get("form_id") or "<unknown>")


def _compare_worldspace(
    source: dict[str, Any], output: dict[str, Any]
) -> dict[str, Any]:
    source_cells = _cell_index(source, _CELL_TOPOLOGY_METRICS)
    output_cells = _cell_index(output, _CELL_TOPOLOGY_METRICS)
    source_section_records = _cell_index(source, _CELL_SECTION_RECORD_METRICS)
    output_section_records = _cell_index(output, _CELL_SECTION_RECORD_METRICS)
    missing_cells = [
        {"x": x, "y": y, "count": len(source_cells[(x, y)])}
        for x, y in sorted(set(source_cells) - set(output_cells))
    ]
    extra_cells = [
        {"x": x, "y": y, "count": len(output_cells[(x, y)])}
        for x, y in sorted(set(output_cells) - set(source_cells))
    ]
    changed_cells = []
    for x, y in sorted(set(source_cells) & set(output_cells)):
        if source_cells[(x, y)] == output_cells[(x, y)]:
            continue
        changed_cells.append(
            {
                "x": x,
                "y": y,
                "source": source_cells[(x, y)],
                "output": output_cells[(x, y)],
            }
        )
    changed_section_record_counts = []
    for x, y in sorted(set(source_section_records) & set(output_section_records)):
        if source_section_records[(x, y)] == output_section_records[(x, y)]:
            continue
        changed_section_record_counts.append(
            {
                "x": x,
                "y": y,
                "source": source_section_records[(x, y)],
                "output": output_section_records[(x, y)],
            }
        )
    source_anomalies = _anomaly_counts(source.get("anomalies", {}), _WORLD_ANOMALIES)
    output_anomalies = _anomaly_counts(output.get("anomalies", {}), _WORLD_ANOMALIES)
    source_summary = source.get("summary", {})
    output_summary = output.get("summary", {})
    summary_equal = all(
        source_summary.get(key, 0) == output_summary.get(key, 0)
        for key in _WORLD_TOPOLOGY_SUMMARY
    )
    section_record_counts_equal = all(
        source_summary.get(key, 0) == output_summary.get(key, 0)
        for key in _CELL_SECTION_RECORD_METRICS
    )
    anomalies_equal = source_anomalies == output_anomalies
    return {
        "worldspace": _world_identity(source),
        "equal": not missing_cells
        and not extra_cells
        and not changed_cells
        and summary_equal
        and anomalies_equal,
        "source_form_id": source.get("form_id"),
        "output_form_id": output.get("form_id"),
        "missing_cells": missing_cells,
        "extra_cells": extra_cells,
        "changed_cells": changed_cells,
        "changed_section_record_counts": changed_section_record_counts,
        "summary_equal": summary_equal,
        "section_record_counts_equal": section_record_counts_equal,
        "anomalies_equal": anomalies_equal,
        "source_anomalies": source_anomalies,
        "output_anomalies": output_anomalies,
    }


def _cell_index(
    world: dict[str, Any], metrics: tuple[str, ...]
) -> dict[tuple[int, int], list[dict[str, int]]]:
    index: dict[tuple[int, int], list[dict[str, int]]] = {}
    for cell in world.get("cells", []):
        coordinate = (int(cell["x"]), int(cell["y"]))
        values = {key: int(cell.get(key, 0)) for key in metrics}
        index.setdefault(coordinate, []).append(values)
    for cells in index.values():
        cells.sort(key=lambda value: json.dumps(value, sort_keys=True))
    return index


def _anomaly_counts(anomalies: dict[str, Any], keys: tuple[str, ...]) -> dict[str, int]:
    return {key: len(anomalies.get(key, [])) for key in keys}


def _coordinate_list(items: list[dict[str, Any]]) -> str:
    return ", ".join(f"({item['x']}, {item['y']})" for item in items)


def _markdown_text(value: Any) -> str:
    return str(value or "").replace("|", "\\|")
