"""Content retrieval functions for the creation_data service layer."""

from __future__ import annotations

import os

from creation_lib.db.db import open_db, exact_lookup
from ._config import strip_hit
from ._db_resolver import get_db_path, db_available, _resolve_db_dir


def get_content(
    domain: str,
    id: str,
    game: str = "",
    *,
    db_dir: str,
) -> dict:
    """Get the full content of a record, wiki page, script, behavior, or NIF by its ID.

    Args:
        domain: One of "records", "scripts", "wiki", "behaviors", "nifs",
                "ext_records", "ext_scripts", "havok"
        id: The identifier (FormKey, script name, page filename, behavior ID, NIF ID)
        game: Game profile ID (e.g. "fo4", "skyrimse").
        db_dir: Override database directory. Defaults to project data/ dir.

    Returns:
        dict with full content and a content_type field.
    """
    if domain == "wiki":
        db_path = get_db_path("wiki", game, db_dir)
        row = exact_lookup(db_path, "pages", "filename", id)
        if row is None:
            conn = open_db(db_path)
            for variant in [id, f"{id}_Script", f"{id}_(Papyrus)"]:
                rows = conn.execute(
                    "SELECT * FROM pages WHERE LOWER(filename) = LOWER(?)", (variant,)
                ).fetchall()
                if rows:
                    row = dict(rows[0])
                    break
        if row is None:
            conn = open_db(db_path)
            rows = conn.execute(
                """
                SELECT * FROM pages
                WHERE LOWER(filename) LIKE LOWER(?) OR LOWER(title) LIKE LOWER(?)
                ORDER BY CASE WHEN LOWER(filename) = LOWER(?) THEN 0 ELSE 1 END,
                         LENGTH(filename) ASC
                """,
                (f"{id}%", f"{id}%", id),
            ).fetchall()
            if rows:
                row = dict(rows[0])
        if row is None:
            return {"error": f"Page '{id}' not found in wiki."}
        row["content_type"] = "wiki"
        return row

    if domain == "ext_scripts":
        db_path = get_db_path("ext_scripts", game, db_dir)
        row = exact_lookup(db_path, "ext_scripts", "script_id", id)
        if row is None:
            conn = open_db(db_path)
            rows = conn.execute(
                "SELECT * FROM ext_scripts WHERE LOWER(script_id) = LOWER(?)", (id,)
            ).fetchall()
            if rows:
                row = dict(rows[0])
        if row is None:
            return {"error": f"External script '{id}' not found."}
        script_path = row.get("script_path", "")
        if script_path and os.path.isfile(script_path):
            try:
                with open(script_path, encoding="utf-8", errors="replace") as f:
                    row["source_code"] = f.read()
            except Exception:
                pass
        strip_hit(row)
        row["content_type"] = "script"
        return row

    if domain == "scripts":
        db_path = get_db_path("scripts", game, db_dir)
        row = exact_lookup(db_path, "scripts", "script_name", id)
        if row is None:
            conn = open_db(db_path)
            rows = conn.execute(
                "SELECT * FROM scripts WHERE LOWER(script_name) = LOWER(?)", (id,)
            ).fetchall()
            if rows:
                row = dict(rows[0])
        if row is None:
            return {"error": f"Script '{id}' not found."}
        script_path = row.get("script_path", "")
        if script_path and os.path.isfile(script_path):
            try:
                with open(script_path, encoding="utf-8", errors="replace") as f:
                    row["source_code"] = f.read()
            except Exception:
                pass
        strip_hit(row)
        row["content_type"] = "script"
        return row

    if domain == "ext_records":
        db_path = get_db_path("ext_records", game, db_dir)
        row = exact_lookup(db_path, "ext_records", "form_key", id)
        if row is None:
            return {"error": f"External record '{id}' not found."}
        yaml_path = row.get("yaml_path", "")
        if yaml_path and os.path.isfile(yaml_path):
            try:
                with open(yaml_path, encoding="utf-8", errors="replace") as f:
                    row["yaml_content"] = f.read()
            except Exception:
                pass
        strip_hit(row)
        row["content_type"] = "yaml"
        return row

    if domain == "records":
        db_path = get_db_path("records", game, db_dir)
        row = exact_lookup(db_path, "records", "form_key", id)
        if row is None:
            return {"error": f"Record '{id}' not found."}
        yaml_path = row.get("yaml_path", "")
        if yaml_path and os.path.isfile(yaml_path):
            try:
                with open(yaml_path, encoding="utf-8", errors="replace") as f:
                    row["yaml_content"] = f.read()
            except Exception:
                pass
        strip_hit(row)
        row["content_type"] = "yaml"
        return row

    if domain == "behaviors":
        # Legacy alias — redirect to havok DB
        db_path = get_db_path("havok", game, db_dir)
        row = exact_lookup(db_path, "havok_behaviors", "id", id)
        if row is None:
            return {"error": f"Behavior '{id}' not found."}
        conn = open_db(db_path)
        events = [row_e["event_name"] for row_e in conn.execute(
            "SELECT event_name FROM behavior_events WHERE behavior_id = ?", (id,)
        ).fetchall()]
        variables = [{"name": row_v["variable_name"], "type": row_v["variable_type"]} for row_v in conn.execute(
            "SELECT variable_name, variable_type FROM behavior_variables WHERE behavior_id = ?", (id,)
        ).fetchall()]
        sequences = [row_s["sequence_name"] for row_s in conn.execute(
            "SELECT sequence_name FROM behavior_sequences WHERE behavior_id = ?", (id,)
        ).fetchall()]
        transitions = [{"name": row_t["transition_name"], "duration": row_t["duration"]} for row_t in conn.execute(
            "SELECT transition_name, duration FROM behavior_transitions WHERE behavior_id = ?", (id,)
        ).fetchall()]
        result = dict(row)
        result.pop("content", None)
        result["events"] = events
        result["variables"] = variables
        result["sequences"] = sequences
        result["transitions"] = transitions
        result["content_type"] = "behavior"
        return result

    if domain == "havok":
        db_path = get_db_path("havok", game, db_dir)
        conn = open_db(db_path)

        # Try each entity table until we find a match
        for table, etype in [
            ("havok_manifests", "manifest"),
            ("havok_behaviors", "behavior"),
            ("havok_projects", "project"),
            ("havok_animations", "animation"),
            ("havok_skeletons", "skeleton"),
        ]:
            row = conn.execute(f"SELECT * FROM {table} WHERE id = ?", (id,)).fetchone()
            if row:
                result = dict(row)
                result["content_type"] = etype
                result.pop("content", None)

                if etype == "behavior":
                    result["events"] = [r[0] for r in conn.execute(
                        "SELECT event_name FROM behavior_events WHERE behavior_id=?", (id,))]
                    result["variables"] = [{"name": r[0], "type": r[1]} for r in conn.execute(
                        "SELECT variable_name, variable_type FROM behavior_variables WHERE behavior_id=?", (id,))]
                    result["sequences"] = [r[0] for r in conn.execute(
                        "SELECT sequence_name FROM behavior_sequences WHERE behavior_id=?", (id,))]
                    result["transitions"] = [{"name": r[0], "duration": r[1]} for r in conn.execute(
                        "SELECT transition_name, duration FROM behavior_transitions WHERE behavior_id=?", (id,))]

                elif etype == "manifest":
                    result["files"] = [dict(r) for r in conn.execute(
                        "SELECT * FROM havok_manifest_files WHERE manifest_id=?", (id,))]
                    result["dependencies"] = [dict(r) for r in conn.execute(
                        "SELECT * FROM havok_manifest_deps WHERE manifest_id=?", (id,))]

                return result

        return {"error": f"ID '{id}' not found in havok database"}

    if domain == "nifs":
        db_path = get_db_path("nifs", game, db_dir)
        row = exact_lookup(db_path, "nifs", "id", id)
        if row is None:
            return {"error": f"NIF '{id}' not found."}
        conn = open_db(db_path)
        behavior_refs = [r["behavior_path"] for r in conn.execute(
            "SELECT behavior_path FROM nif_behavior_refs WHERE nif_id = ?", (id,)
        ).fetchall()]
        textures = [r["texture_path"] for r in conn.execute(
            "SELECT texture_path FROM nif_textures WHERE nif_id = ?", (id,)
        ).fetchall()]
        materials = [r["material_path"] for r in conn.execute(
            "SELECT material_path FROM nif_materials WHERE nif_id = ?", (id,)
        ).fetchall()]
        sequences = [r["sequence_name"] for r in conn.execute(
            "SELECT sequence_name FROM nif_sequences WHERE nif_id = ?", (id,)
        ).fetchall()]
        block_types = {r["type_name"]: r["count"] for r in conn.execute(
            "SELECT type_name, count FROM nif_block_types WHERE nif_id = ?", (id,)
        ).fetchall()}
        result = dict(row)
        result.pop("content", None)
        result["has_particles"] = bool(result.get("has_particles", 0))
        result["has_behavior"] = bool(result.get("has_behavior", 0))
        result["has_controllers"] = bool(result.get("has_controllers", 0))
        result["behavior_refs"] = behavior_refs
        result["textures"] = textures
        result["materials"] = materials
        result["sequences"] = sequences
        result["block_types"] = block_types
        # Fetch material->texture mappings for this NIF's materials
        material_textures = {}
        if materials:
            placeholders = ",".join("?" * len(materials))
            mat_tex_rows = conn.execute(
                f"SELECT material_path, material_type, texture_slot, texture_path "
                f"FROM nif_material_textures WHERE material_path IN ({placeholders})",
                [m.lower() for m in materials],
            ).fetchall()
            for r in mat_tex_rows:
                mp = r["material_path"]
                mt = r["material_type"]
                ts = r["texture_slot"]
                tp = r["texture_path"]
                if mp not in material_textures:
                    material_textures[mp] = {"type": mt, "textures": {}}
                material_textures[mp]["textures"][ts] = tp
        result["material_textures"] = material_textures
        # Build abs_path from source_path + path
        source_path = result.get("source_path", "")
        nif_path = result.get("path", "")
        if source_path and nif_path:
            result["abs_path"] = os.path.join(source_path, nif_path).replace("\\", "/")
        result["content_type"] = "nif"
        return result

    return {"error": f"Unknown domain '{domain}'. Valid: records, scripts, wiki, behaviors, nifs, ext_records, ext_scripts"}


def get_behavior_xml(
    behavior_id: str,
    game: str = "",
    *,
    db_dir: str,
) -> dict:
    """Get the raw XML content of a behavior file from the cache.

    Args:
        behavior_id: The behavior ID (e.g. "fo4/UniqueBehaviors/FlamerFX/Behavior")
        game: Game profile ID.
        db_dir: Override database directory.

    Returns:
        dict with 'xml' key containing the full XML string, or 'error' if not found.
    """
    db_dir_resolved = _resolve_db_dir(db_dir)
    # Look up in havok DB
    row = None
    if db_available("havok", game, db_dir):
        db_path = get_db_path("havok", game, db_dir)
        row = exact_lookup(db_path, "havok_behaviors", "id", behavior_id)
    if not behavior_id or not behavior_id.strip():
        return {"error": "behavior_id is required."}
    if row is None:
        return {"error": f"Behavior '{behavior_id}' not found."}
    source = row.get("source", "")
    source_path = row.get("source_path", "")
    if source.startswith("ext:"):
        ext_mod = source[4:]
        cache_dir = os.path.join("xml_external_cache", ext_mod)
    else:
        cache_dir = f"xml_{game}_cache"
    xml_path = os.path.join(db_dir_resolved, cache_dir, source_path)
    if not os.path.isfile(xml_path):
        return {"error": f"XML file not found at '{xml_path}' for behavior '{behavior_id}'."}
    try:
        with open(xml_path, encoding="utf-8", errors="replace") as f:
            xml_content = f.read()
    except Exception as e:
        return {"error": f"Failed to read XML for '{behavior_id}': {e}"}
    return {"xml": xml_content, "id": behavior_id, "name": row.get("name", ""), "graph_path": row.get("graph_path", "")}
