"""Papyrus wiki tools: function lookup, script API, hierarchy walking."""

from __future__ import annotations

from creation_lib.db.db import open_db
from ._db_resolver import get_db_path
from ._wiki import (
    function_member_script_from_page,
    function_name_from_page,
    script_extends_from_page,
    script_type_from_page,
    section_text_from_page,
)


def _load_pages(db_path: str, *, category: str | None = None, wiki_category: str | None = None) -> list[dict]:
    conn = open_db(db_path)
    sql = "SELECT * FROM pages WHERE 1=1"
    params: list[str] = []
    if category:
        sql += " AND category = ?"
        params.append(category)
    if wiki_category:
        sql += " AND wiki_category = ?"
        params.append(wiki_category)
    rows = conn.execute(sql, params).fetchall()
    return [dict(row) for row in rows]


def _script_page_for_type(db_path: str, script_type: str) -> dict | None:
    target = script_type.strip().lower()
    if not target:
        return None
    for page in _load_pages(db_path, category="papyrus", wiki_category="script_api"):
        if script_type_from_page(page).lower() == target:
            return page
    return None


def _script_lineage(db_path: str, script_type: str) -> list[str]:
    lineage: list[str] = []
    visited: set[str] = set()
    current = script_type.strip()
    while current:
        key = current.lower()
        if key in visited:
            break
        visited.add(key)
        lineage.append(current)
        page = _script_page_for_type(db_path, current)
        if page is None:
            break
        current = script_extends_from_page(page)
    return lineage


def get_function(
    function_name: str,
    script_type: str = "",
    game: str = "",
    *,
    db_dir: str,
) -> list[dict] | dict:
    """Look up a Papyrus function by name, optionally filtered by parent script."""
    if not function_name or not function_name.strip():
        return {"error": "function_name is required."}
    db_path = get_db_path("wiki", game, db_dir)
    target_name = function_name.strip().lower()
    lineage = _script_lineage(db_path, script_type) if script_type else []
    lineage_lookup = {item.lower() for item in lineage}

    rows = []
    for page in _load_pages(db_path, category="papyrus", wiki_category="function"):
        name = function_name_from_page(page)
        if name.lower() != target_name:
            continue
        parent = function_member_script_from_page(page)
        if script_type and (not parent or parent.lower() not in lineage_lookup):
            continue
        page["function_name"] = name
        page["parent_script"] = parent
        page["syntax"] = section_text_from_page(page, "Syntax")
        page["return_type"] = section_text_from_page(page, "Return Value")
        page["content_type"] = "wiki"
        if script_type and parent and parent.lower() != script_type.strip().lower():
            page["inherited_from"] = parent
        rows.append(page)

    if not rows:
        return {"error": f"Function '{function_name}' not found."}
    return rows


def list_functions(
    script_type: str,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """List all functions for a Papyrus script type."""
    db_path = get_db_path("wiki", game, db_dir)
    lineage_lookup = {item.lower() for item in _script_lineage(db_path, script_type)}
    results = []
    for page in _load_pages(db_path, category="papyrus", wiki_category="function"):
        parent = function_member_script_from_page(page)
        if not parent or parent.lower() not in lineage_lookup:
            continue
        page["function_name"] = function_name_from_page(page)
        page["parent_script"] = parent
        page["syntax"] = section_text_from_page(page, "Syntax")
        page["return_type"] = section_text_from_page(page, "Return Value")
        results.append(page)
    results.sort(key=lambda row: row.get("function_name", "").lower())
    return results


def get_script_api(
    script_type: str,
    game: str = "",
    *,
    db_dir: str,
) -> dict:
    """Get the full API page for a Papyrus script type (e.g. Actor, ObjectReference)."""
    if not script_type or not script_type.strip():
        return {"error": "script_type is required."}
    db_path = get_db_path("wiki", game, db_dir)
    row = _script_page_for_type(db_path, script_type)
    if row:
        row["script_type"] = script_type_from_page(row)
        row["extends"] = script_extends_from_page(row)
        row["content_type"] = "wiki"
        return row
    return {"error": f"Script API page for '{script_type}' not found."}


def get_script_hierarchy(
    script_type: str,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Walk the extends chain for a Papyrus script type.
    Returns the hierarchy from the given type up to the root.
    """
    hierarchy = []
    visited = set()
    current = script_type
    db_path = get_db_path("wiki", game, db_dir)
    while current and current not in visited:
        visited.add(current)
        row = _script_page_for_type(db_path, current)
        if row is None:
            hierarchy.append({"script_type": current, "extends": "", "error": "page not found"})
            break
        hierarchy.append({
            "script_type": current,
            "extends": script_extends_from_page(row),
            "filename": row["filename"],
        })
        current = script_extends_from_page(row)
    return hierarchy
