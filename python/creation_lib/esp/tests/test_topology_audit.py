from __future__ import annotations

import json

from creation_lib.esp.topology_audit import (
    compare_topology_reports,
    render_topology_report,
)


def _cell(form_id: str, x: int, y: int, *, land: int = 1, navm: int = 1) -> dict:
    return {
        "form_id": form_id,
        "x": x,
        "y": y,
        "child_groups": 1,
        "persistent_groups": 1,
        "temporary_groups": 1,
        "visible_distant_groups": 0,
        "persistent_records": 2,
        "temporary_records": land + navm,
        "visible_distant_records": 0,
        "land_records": land,
        "land_in_temporary_group": land,
        "misplaced_land_records": 0,
        "navm_records": navm,
        "navm_in_temporary_group": navm,
        "misplaced_navm_records": 0,
    }


def _empty_world_anomalies() -> dict:
    return {
        "duplicate_cell_coordinates": [],
        "cells_missing_coordinates": [],
        "cells_missing_child_groups": [],
        "cells_missing_land": [],
        "cells_with_duplicate_land": [],
        "orphan_cell_groups": [],
        "orphan_land_records": [],
        "orphan_navm_records": [],
        "misplaced_land_records": [],
        "misplaced_navm_records": [],
    }


def _report(
    plugin: str,
    game: str,
    world_form_id: str,
    cells: list[dict],
    *,
    flat_land: int = 0,
) -> dict:
    summary = {
        "worldspaces": 1,
        "exterior_cells": len(cells),
        "unique_cell_coordinates": len({(cell["x"], cell["y"]) for cell in cells}),
        "persistent_groups": sum(cell["persistent_groups"] for cell in cells),
        "temporary_groups": sum(cell["temporary_groups"] for cell in cells),
        "land_records": sum(cell["land_records"] for cell in cells),
        "navm_records": sum(cell["navm_records"] for cell in cells),
        "valid_land_records": sum(cell["land_in_temporary_group"] for cell in cells),
        "valid_navm_records": sum(cell["navm_in_temporary_group"] for cell in cells),
        "missing_land_cells": 0,
        "duplicate_cell_coordinates": 0,
        "duplicate_land_cells": 0,
        "flat_exterior_cells": 0,
        "flat_land_records": flat_land,
        "flat_navm_records": 0,
        "orphan_land_records": 0,
        "orphan_navm_records": 0,
        "misplaced_land_records": 0,
        "misplaced_navm_records": 0,
    }
    world_summary = {
        "exterior_cells": len(cells),
        "unique_cell_coordinates": summary["unique_cell_coordinates"],
        "non_exterior_cells": 1,
        "cell_child_groups": len(cells),
        "persistent_groups": summary["persistent_groups"],
        "temporary_groups": summary["temporary_groups"],
        "visible_distant_groups": 0,
        "persistent_records": sum(cell["persistent_records"] for cell in cells),
        "temporary_records": sum(cell["temporary_records"] for cell in cells),
        "visible_distant_records": 0,
        "land_records": summary["land_records"],
        "navm_records": summary["navm_records"],
        "valid_land_records": summary["valid_land_records"],
        "valid_navm_records": summary["valid_navm_records"],
    }
    return {
        "plugin": plugin,
        "path": plugin,
        "game": game,
        "header_size": 24,
        "worldspaces": [
            {
                "editor_id": "WastelandNV",
                "form_id": world_form_id,
                "cells": cells,
                "summary": world_summary,
                "anomalies": _empty_world_anomalies(),
            }
        ],
        "summary": summary,
        "anomalies": {
            "flat_exterior_cells": [],
            "flat_land_records": [
                {"signature": "LAND", "form_id": f"{index:08X}", "top_group": "LAND"}
                for index in range(flat_land)
            ],
            "flat_navm_records": [],
            "duplicate_record_form_ids": [],
        },
    }


def test_comparison_ignores_remapped_form_ids_when_world_and_grid_match() -> None:
    source = _report("FalloutNV.esm", "fnv", "00000800", [_cell("00000801", -1, 2)])
    output = _report("Converted.esm", "fo4", "01001000", [_cell("01001001", -1, 2)])

    equality = compare_topology_reports(source, output)

    assert equality["equal"] is True
    assert equality["matched_worldspaces"] == 1
    assert equality["missing_cells"] == 0
    assert equality["changed_cells"] == 0
    assert equality["changed_section_record_counts"] == 0


def test_comparison_reports_missing_changed_and_flat_topology() -> None:
    source = _report(
        "FalloutNV.esm",
        "fnv",
        "00000800",
        [_cell("00000801", 0, 0), _cell("00000802", 1, 0)],
    )
    output = _report(
        "Converted.esm",
        "fo4",
        "01001000",
        [_cell("01001001", 0, 0, navm=0)],
        flat_land=1,
    )

    equality = compare_topology_reports(source, output)

    assert equality["equal"] is False
    assert equality["missing_cells"] == 1
    assert equality["changed_cells"] == 1
    assert equality["changed_section_record_counts"] == 1
    assert equality["plugin_anomalies_equal"] is False
    assert equality["worldspaces"][0]["missing_cells"] == [{"x": 1, "y": 0, "count": 1}]


def test_renderers_emit_compact_json_and_markdown() -> None:
    source = _report("FalloutNV.esm", "fnv", "00000800", [_cell("00000801", 0, 0)])
    output = _report("Converted.esm", "fo4", "01001000", [_cell("01001001", 0, 0)])
    report = {
        "schema_version": 1,
        "source": source,
        "output": output,
        "equality": compare_topology_reports(source, output),
    }

    compact = render_topology_report(report, "compact")
    markdown = render_topology_report(report, "markdown")

    assert json.loads(compact)["equality"]["equal"] is True
    assert '\n  "source"' not in compact
    assert markdown.startswith("# ESP topology audit\n")
    assert "| LAND in type 9 | 1 | 1 |" in markdown
    assert "**Result:** MATCH" in markdown
