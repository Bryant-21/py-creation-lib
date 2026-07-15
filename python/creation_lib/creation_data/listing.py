"""List and count items across all domains."""

from __future__ import annotations

from creation_lib.db.db import open_db, count_by_column
from ._config import clamp
from ._db_resolver import get_db_path, db_available
from ._wiki import script_type_from_page


def list_items(
    domain: str,
    record_type: str = "",
    source: str = "",
    category: str = "",
    extends: str = "",
    mod_name: str = "",
    max_results: int = 50,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict] | list[str] | dict:
    """List or count items in a domain with optional filters."""
    max_results = clamp(max_results)

    if domain == "wiki_categories":
        db_path = get_db_path("wiki", game, db_dir)
        return count_by_column(db_path, "pages", "category")

    if domain == "wiki_record_types":
        db_path = get_db_path("wiki", game, db_dir)
        conn = open_db(db_path)
        rows = conn.execute(
            "SELECT filename FROM pages WHERE category = 'record_type' ORDER BY filename"
        ).fetchall()
        return [row[0] for row in rows]

    if domain == "script_types":
        db_path = get_db_path("wiki", game, db_dir)
        conn = open_db(db_path)
        rows = conn.execute(
            "SELECT filename, title FROM pages WHERE category='papyrus' AND wiki_category='script_api'"
        ).fetchall()
        values = []
        seen = set()
        for row in rows:
            script_type = script_type_from_page(dict(row))
            if not script_type:
                continue
            key = script_type.lower()
            if key in seen:
                continue
            seen.add(key)
            values.append(script_type)
        values.sort(key=str.lower)
        return values

    if domain == "scripts":
        db_path = get_db_path("scripts", game, db_dir)
        conn = open_db(db_path)
        sql = "SELECT script_name, extends, source, category FROM scripts WHERE 1=1"
        params: list = []
        if source:
            sql += " AND source = ?"
            params.append(source)
        if category:
            sql += " AND category = ?"
            params.append(category)
        if extends:
            sql += " AND extends = ?"
            params.append(extends.lower())
        sql += " ORDER BY script_name LIMIT ?"
        params.append(max_results)
        rows = conn.execute(sql, params).fetchall()
        return [dict(r) for r in rows]

    if domain == "extends_types":
        db_path = get_db_path("scripts", game, db_dir)
        return count_by_column(db_path, "scripts", "extends")

    if domain == "record_types":
        db_path = get_db_path("records", game, db_dir)
        return count_by_column(db_path, "records", "record_type")

    if domain == "records":
        db_path = get_db_path("records", game, db_dir)
        conn = open_db(db_path)
        sql = "SELECT form_key, editor_id, record_type, name, source FROM records WHERE 1=1"
        params = []
        if record_type:
            sql += " AND record_type = ?"
            params.append(record_type)
        if source:
            sql += " AND source = ?"
            params.append(source)
        sql += " ORDER BY editor_id LIMIT ?"
        params.append(max_results)
        rows = conn.execute(sql, params).fetchall()
        return [dict(r) for r in rows]

    if domain == "behaviors":
        # Legacy alias — redirect to havok DB
        db_path = get_db_path("havok", game, db_dir)
        return count_by_column(db_path, "havok_behaviors", "category")

    if domain == "havok":
        db_path = get_db_path("havok", game, db_dir)
        conn = open_db(db_path)
        result = {}
        for table, label in [
            ("havok_projects", "projects"),
            ("havok_behaviors", "behaviors"),
            ("havok_animations", "animations"),
            ("havok_skeletons", "skeletons"),
            ("havok_manifests", "manifests"),
        ]:
            row = conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()
            result[label] = row[0] if row else 0
        return result

    if domain == "nifs":
        db_path = get_db_path("nifs", game, db_dir)
        return count_by_column(db_path, "nifs", "category")

    if domain == "nif_categories":
        db_path = get_db_path("nifs", game, db_dir)
        return count_by_column(db_path, "nifs", "category")

    if domain == "ext_mods":
        if not db_available("ext_records", game, db_dir):
            return [{"info": "No external mods database found. Add mods to external_mods/ and run preprocess_external.py."}]
        ext_db = get_db_path("ext_records", game, db_dir)
        conn = open_db(ext_db)
        mods: dict[str, dict] = {}
        for table in ("ext_records", "ext_scripts", "ext_readmes"):
            try:
                rows = conn.execute(f"SELECT mod_name, COUNT(*) as cnt FROM {table} GROUP BY mod_name").fetchall()
            except Exception:
                continue
            for r in rows:
                name = r[0]
                if name not in mods:
                    mods[name] = {"mod_name": name, "records": 0, "scripts": 0, "readmes": 0, "nifs": 0, "behaviors": 0}
                if table == "ext_records":
                    mods[name]["records"] = r[1]
                elif table == "ext_scripts":
                    mods[name]["scripts"] = r[1]
                elif table == "ext_readmes":
                    mods[name]["readmes"] = r[1]
        # Count NIFs from nifs.db for external sources
        if db_available("nifs", game, db_dir):
            try:
                nifs_db = get_db_path("nifs", game, db_dir)
                nifs_conn = open_db(nifs_db)
                rows = nifs_conn.execute(
                    "SELECT source, COUNT(*) as cnt FROM nifs WHERE source LIKE 'ext:%' GROUP BY source"
                ).fetchall()
                for r in rows:
                    mod_name_val = r[0].replace("ext:", "", 1)
                    if mod_name_val not in mods:
                        mods[mod_name_val] = {"mod_name": mod_name_val, "records": 0, "scripts": 0, "readmes": 0, "nifs": 0, "behaviors": 0}
                    mods[mod_name_val]["nifs"] = r[1]
            except Exception:
                pass
        # Count behaviors from havok DB for external sources
        if db_available("havok", game, db_dir):
            try:
                beh_db = get_db_path("havok", game, db_dir)
                beh_conn = open_db(beh_db)
                rows = beh_conn.execute(
                    "SELECT source, COUNT(*) as cnt FROM havok_behaviors WHERE source LIKE 'ext:%' GROUP BY source"
                ).fetchall()
                for r in rows:
                    mod_name_val = r[0].replace("ext:", "", 1)
                    if mod_name_val not in mods:
                        mods[mod_name_val] = {"mod_name": mod_name_val, "records": 0, "scripts": 0, "readmes": 0, "nifs": 0, "behaviors": 0}
                    mods[mod_name_val]["behaviors"] = r[1]
            except Exception:
                pass
        return list(mods.values())

    return {"error": f"Unknown domain '{domain}'."}
