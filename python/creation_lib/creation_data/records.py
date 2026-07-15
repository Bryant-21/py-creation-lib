"""Record-specific service functions for the creation-data service layer."""

from __future__ import annotations

import os

from creation_lib.db.db import open_db, exact_lookup, batch_lookup
from ._config import clamp, strip_hit
from ._db_resolver import get_db_path


def get_record(
    form_key: str,
    include_content: bool = False,
    game: str = "",
    *,
    db_dir: str,
) -> dict:
    """Get a game record by its FormKey.

    Returns the record dict, or an error dict if not found.
    """
    if not form_key or not form_key.strip():
        return {"error": "form_key is required (e.g. '004822:Fallout4.esm')."}
    db_path = get_db_path("records", game, db_dir)
    row = exact_lookup(db_path, "records", "form_key", form_key)
    if row is None:
        return {"error": f"Record '{form_key}' not found."}
    if include_content:
        yaml_path = row.get("yaml_path", "")
        if yaml_path and os.path.isfile(yaml_path):
            try:
                with open(yaml_path, encoding="utf-8", errors="replace") as f:
                    row["yaml_content"] = f.read()
            except Exception:
                pass
    strip_hit(row)
    return row


def get_references(
    form_key: str,
    record_type: str = "",
    max_results: int = 50,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Find all records that reference a given FormKey (reverse lookup).

    Raises ValueError if form_key is empty or no references are found.
    """
    if not form_key or not form_key.strip():
        raise ValueError("form_key is required (e.g. '067384:Fallout4.esm').")
    max_results = clamp(max_results)
    db_path = get_db_path("records", game, db_dir)
    conn = open_db(db_path)
    if record_type:
        rows = conn.execute(
            """SELECT r.form_key, r.editor_id, r.record_type, r.name, r.source, r.keywords
               FROM record_refs x
               JOIN records r ON r.form_key = x.referencing_form_key
               WHERE x.referenced_form_key = ? AND r.record_type = ?
               LIMIT ?""",
            (form_key, record_type, max_results),
        ).fetchall()
    else:
        rows = conn.execute(
            """SELECT r.form_key, r.editor_id, r.record_type, r.name, r.source, r.keywords
               FROM record_refs x
               JOIN records r ON r.form_key = x.referencing_form_key
               WHERE x.referenced_form_key = ?
               LIMIT ?""",
            (form_key, max_results),
        ).fetchall()
    results = [dict(r) for r in rows]
    if not results:
        raise ValueError(f"No records reference FormKey '{form_key}'.")
    return results


def lookup_editor_id(
    editor_id: str,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Look up records by EditorID (case-insensitive exact match).

    Raises ValueError if editor_id is empty or no records are found.
    """
    if not editor_id or not editor_id.strip():
        raise ValueError("editor_id is required.")
    db_path = get_db_path("records", game, db_dir)
    conn = open_db(db_path)
    rows = conn.execute(
        "SELECT form_key, editor_id, record_type, name, source FROM records WHERE LOWER(editor_id) = LOWER(?)",
        (editor_id,),
    ).fetchall()
    if not rows:
        raise ValueError(f"No records found with EditorID '{editor_id}'.")
    return [dict(r) for r in rows]


def resolve_keywords(
    form_key: str,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Get all keywords for a record, resolved to human-readable EditorIDs.

    Raises ValueError if form_key is empty or the record is not found.
    """
    if not form_key or not form_key.strip():
        raise ValueError("form_key is required.")
    db_path = get_db_path("records", game, db_dir)
    row = exact_lookup(db_path, "records", "form_key", form_key)
    if row is None:
        raise ValueError(f"Record '{form_key}' not found.")
    kw_str = row.get("keywords", "")
    if not kw_str:
        return []
    kw_fks = kw_str.split()
    row_map = batch_lookup(
        db_path, "records", "form_key", kw_fks, columns="form_key, editor_id, name"
    )
    results = []
    for fk in kw_fks:
        if fk in row_map:
            results.append(row_map[fk])
        else:
            results.append({"form_key": fk, "editor_id": "", "name": "(not found)"})
    return results


def count_references(
    form_key: str,
    game: str = "",
    *,
    db_dir: str,
) -> dict:
    """Count how many records reference a given FormKey.

    Returns a dict with form_key and reference_count, or an error dict.
    """
    if not form_key or not form_key.strip():
        return {"error": "form_key is required."}
    db_path = get_db_path("records", game, db_dir)
    conn = open_db(db_path)
    row = conn.execute(
        "SELECT COUNT(*) as cnt FROM record_refs WHERE referenced_form_key = ?",
        (form_key,),
    ).fetchone()
    return {"form_key": form_key, "reference_count": row[0]}
