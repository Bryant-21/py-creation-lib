from __future__ import annotations

from collections import Counter

from creation_lib.inspection.records import form_key, named_fields
from creation_lib.inspection.collision import collision_report


def placed_rows(graph, plugin, placements, resolver, *, base_types=(), collision=False):
    cache, collision_cache = {}, {}
    for placement in placements:
        if placement["signature"] not in {"REFR", "ACHR", "PGRE", "PHZD", "PMIS", "PARW", "PBAR", "PBEA", "PCON", "PFLA"}:
            continue
        fid = placement["form_id"]
        raw_base = graph.inspect(plugin, fid)["base_form_id"]
        base_key = form_key(plugin, raw_base) if raw_base else None
        if base_key not in cache:
            versions = graph.resolve(base_key) if base_key else []
            if versions:
                source, summary = versions[-1]
                models = graph.inspect(source, summary["form_id"])["assets"]
                paths = [asset["path"] for asset in models if asset["field"] in {"MODL", "MOD2", "MOD3", "MOD4", "MOD5"}]
                resolved = [dict(resolver.resolve(path)) for path in paths]
                statuses = {r["status"] for r in resolved}
                status = next((s for s in ("invalid_path", "missing", "malformed_path") if s in statuses), "available" if resolved else "no_model_path")
                base = {"form_key": base_key, "signature": summary["signature"], "editor_id": summary["editor_id"],
                        "winner": source.plugin_name, "models": resolved, "status": status}
            else:
                base = {"form_key": base_key, "signature": None, "editor_id": None, "models": [],
                        "status": "unresolved_base" if base_key else "missing_base"}
            cache[base_key] = base
        base = cache[base_key]
        if base_types and base["signature"] is not None and base["signature"] not in base_types:
            continue
        row = {"form_key": form_key(plugin, fid), "signature": placement["signature"],
               "group_type": placement.get("group_type"), "base": base, "status": base["status"]}
        if collision:
            decoded = graph.record(plugin, fid)
            fields = named_fields(decoded)["fields"] if decoded else {}
            row["placement"] = {key: value for key, value in fields.items() if key in {"DATA", "Placement", "Position", "Rotation", "XSCL", "Scale"}}
            row["collision"] = []
            for model in base["models"]:
                key = model["path"]
                if key not in collision_cache:
                    if model["status"] == "available":
                        try:
                            collision_cache[key] = collision_report(resolver.read(model))
                        except (RuntimeError, ValueError, OSError) as error:
                            collision_cache[key] = {"complete": False, "issues": [{"message": str(error)}]}
                    else:
                        collision_cache[key] = {"complete": False, "issues": [{"message": model["status"]}]}
                row["collision"].append({"model": key, **collision_cache[key]})
        yield row


def model_census(rows):
    bases = {}
    for row in rows:
        key = row["base"]["form_key"] or "<missing>"
        if key not in bases:
            bases[key] = {**row["base"], "placement_count": 0, "sample_placements": []}
        bases[key]["placement_count"] += 1
        if len(bases[key]["sample_placements"]) < 10:
            bases[key]["sample_placements"].append(row["form_key"])
    records = list(bases.values())
    return {"records": records, "summary": {"bases": len(records),
            "placements": sum(r["placement_count"] for r in records),
            "status_counts": dict(Counter(r["status"] for r in records))}}
