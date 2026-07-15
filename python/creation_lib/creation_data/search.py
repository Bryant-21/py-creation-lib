"""Search functions for the creation-data service layer."""

from __future__ import annotations

import glob as _glob
import os

from creation_lib.db.db import open_db, fts_search, batch_lookup, fallback_word_search
from ._config import SEARCH_DOMAINS, TYPE_PRIORITY, DEFAULT_PRIORITY, clamp, strip_hit
from ._db_resolver import get_db_path, db_available, get_vec_searcher, havok_search, _resolve_db_dir


# ---------------------------------------------------------------------------
# search()
# ---------------------------------------------------------------------------

def search(
    domain: str,
    query: str = "",
    record_type: str = "",
    source: str = "",
    category: str = "",
    extends: str = "",
    mod_name: str = "",
    full_text: bool = False,
    cross_game: bool = False,
    max_results: int = 10,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Keyword (FTS5) search across game data domains.

    All database resolution uses *game* and *db_dir* explicitly —
    no environment variables are read.
    """
    if domain not in SEARCH_DOMAINS:
        raise ValueError(f"Unknown domain '{domain}'. Valid: {', '.join(sorted(SEARCH_DOMAINS))}")
    max_results = clamp(max_results)
    db_dir_resolved = _resolve_db_dir(db_dir)

    # -- Scripts: filter-only (no query) -----------------------------------
    if domain == "scripts" and not (query and query.strip()):
        if not any([extends, source, category]):
            raise ValueError("Query cannot be empty (or provide at least one filter: source, category, extends).")
        db_path = get_db_path("scripts", game, db_dir)
        conn = open_db(db_path)
        sql = "SELECT * FROM scripts WHERE 1=1"
        params: list = []
        if extends:
            sql += " AND LOWER(extends) = LOWER(?)"
            params.append(extends)
        if source:
            sql += " AND source = ?"
            params.append(source)
        if category:
            sql += " AND category = ?"
            params.append(category)
        sql += " LIMIT ?"
        params.append(max_results)
        rows = conn.execute(sql, params).fetchall()
        hits = [dict(r) for r in rows]
        for h in hits:
            strip_hit(h)
            for field in ("properties", "functions", "events"):
                if h.get(field) and len(h[field]) > 300:
                    h[field] = h[field][:300] + "..."
        return hits

    if not query or not query.strip():
        raise ValueError("Query cannot be empty.")

    # -- Wiki --------------------------------------------------------------
    if domain == "wiki":
        db_path = get_db_path("wiki", game, db_dir)
        filters = {}
        if category:
            filters["category"] = category
        hits = fts_search(db_path, "pages", "pages_fts", query, filters, max_results)
        for h in hits:
            h["snippet"] = h.pop("content", "")[:500].replace("\n", " ").strip()
        return hits

    # -- Scripts (with query) ----------------------------------------------
    if domain == "scripts":
        db_path = get_db_path("scripts", game, db_dir)
        filters = {}
        if source:
            filters["source"] = source
        if category:
            filters["category"] = category
        if extends:
            filters["extends"] = extends.lower()
        hits = fts_search(db_path, "scripts", "scripts_fts", query, filters, max_results)
        hits = fallback_word_search(db_path, "scripts", "scripts_fts", query, hits, "script_id", filters, max_results)
        for h in hits:
            strip_hit(h)
            for field in ("properties", "functions", "events"):
                if h.get(field) and len(h[field]) > 300:
                    h[field] = h[field][:300] + "..."
        return hits

    # -- External scripts --------------------------------------------------
    if domain == "ext_scripts":
        ext_db = get_db_path("ext_scripts", game, db_dir)
        ext_filters: dict = {}
        if mod_name:
            ext_filters["mod_name"] = mod_name
        if extends:
            ext_filters["extends"] = extends.lower()
        hits = fts_search(ext_db, "ext_scripts", "ext_scripts_fts", query, ext_filters, max_results)
        for h in hits:
            strip_hit(h)
            h["source"] = f"ext:{h.pop('mod_name', '')}"
            for field in ("properties", "functions", "events"):
                if h.get(field) and len(h[field]) > 300:
                    h[field] = h[field][:300] + "..."
        return hits

    # -- Records -----------------------------------------------------------
    if domain == "records":
        db_path = get_db_path("records", game, db_dir)
        filters = {}
        if record_type:
            filters["record_type"] = record_type
        if source:
            filters["source"] = source
        # Over-fetch 5x for type-priority re-ranking
        overfetch = max_results * 5
        hits = fts_search(db_path, "records", "records_fts", query, filters, overfetch,
                          columns="t.form_key, t.editor_id, t.record_type, t.name, t.source, t.keywords")
        hits = fallback_word_search(db_path, "records", "records_fts", query, hits, "form_key", filters, overfetch)
        # Re-rank by type priority
        for i, h in enumerate(hits):
            h["_priority"] = TYPE_PRIORITY.get(h.get("record_type", ""), DEFAULT_PRIORITY)
            h["_rank_pos"] = i
            strip_hit(h)
        hits.sort(key=lambda h: (h["_priority"], h["_rank_pos"]))
        for h in hits:
            h.pop("_priority", None)
            h.pop("_rank_pos", None)
        hits = hits[:max_results]
        return hits

    # -- External records --------------------------------------------------
    if domain == "ext_records":
        ext_db = get_db_path("ext_records", game, db_dir)
        ext_filters = {}
        if record_type:
            ext_filters["record_type"] = record_type
        if mod_name:
            ext_filters["mod_name"] = mod_name
        hits = fts_search(ext_db, "ext_records", "ext_records_fts", query, ext_filters, max_results,
                          columns="t.form_key, t.editor_id, t.record_type, t.name, t.mod_name, t.keywords")
        for h in hits:
            strip_hit(h)
            h["source"] = f"ext:{h.pop('mod_name', '')}"
        return hits

    # -- Behaviors (legacy alias → havok_behaviors) ------------------------
    if domain == "behaviors":
        db_path = get_db_path("havok", game, db_dir)
        filters = {}
        if category:
            filters["category"] = category
        if source:
            filters["source"] = source
        behavior_search_cols = None if full_text else ["name", "id", "category"]
        hits = fts_search(db_path, "havok_behaviors", "havok_fts", query, filters, max_results, search_columns=behavior_search_cols)

        # Cross-game: also search other available game havok DBs
        if cross_game and len(hits) < max_results:
            for db_file in _glob.glob(os.path.join(db_dir_resolved, "*_havok.db")):
                other_game = os.path.basename(db_file).replace("_havok.db", "")
                if other_game == game:
                    continue
                remaining = max_results - len(hits)
                if remaining <= 0:
                    break
                xhits = fts_search(db_file, "havok_behaviors", "havok_fts", query, filters, remaining, search_columns=behavior_search_cols)
                for xh in xhits:
                    xh["source_game"] = other_game
                hits.extend(xhits)
            hits = hits[:max_results]

        for h in hits:
            h["snippet"] = (h.pop("content", "") or "")[:300].replace("\n", " ").strip()
        result = []
        for h in hits:
            entry = {
                "id": h.get("id", ""),
                "name": h.get("name", ""),
                "category": h.get("category", ""),
                "source": h.get("source", ""),
                "source_path": h.get("source_path", ""),
                "graph_path": h.get("graph_path", ""),
                "node_count": h.get("node_count", 0),
                "usable": h.get("usable", ""),
                "snippet": h.get("snippet", ""),
            }
            if "source_game" in h:
                entry["source_game"] = h["source_game"]
            result.append(entry)
        return result

    # -- Havok (all entity types) ------------------------------------------
    if domain == "havok":
        db_path = get_db_path("havok", game, db_dir)
        hits = havok_search(db_path, query, entity_type=None,
                            category=category, source=source, max_results=max_results)

        # Cross-game support
        if cross_game and len(hits) < max_results:
            for db_file in _glob.glob(os.path.join(db_dir_resolved, "*_havok.db")):
                other_game = os.path.basename(db_file).replace("_havok.db", "")
                if other_game == game:
                    continue
                remaining = max_results - len(hits)
                if remaining <= 0:
                    break
                extra = havok_search(db_file, query, entity_type=None,
                                     category=category, source=source, max_results=remaining)
                for xh in extra:
                    xh["source_game"] = other_game
                hits.extend(extra)
            hits = hits[:max_results]

        for h in hits:
            h["snippet"] = (h.pop("content", "") or "")[:300].replace("\n", " ").strip()
        return hits

    # -- NIFs --------------------------------------------------------------
    if domain == "nifs":
        db_path = get_db_path("nifs", game, db_dir)
        filters = {}
        if category:
            filters["category"] = category
        if source:
            filters["source"] = source
        nif_search_cols = None if full_text else ["name", "path", "category"]
        hits = fts_search(db_path, "nifs", "nifs_fts", query, filters, max_results, search_columns=nif_search_cols)
        hits = fallback_word_search(db_path, "nifs", "nifs_fts", query, hits, "id", filters, max_results, search_columns=nif_search_cols)

        # Cross-game: also search other available game NIF DBs
        if cross_game and len(hits) < max_results:
            for db_file in _glob.glob(os.path.join(db_dir_resolved, "*_nifs.db")):
                other_game = os.path.basename(db_file).replace("_nifs.db", "")
                if other_game == game:
                    continue
                remaining = max_results - len(hits)
                if remaining <= 0:
                    break
                xhits = fts_search(db_file, "nifs", "nifs_fts", query, filters, remaining, search_columns=nif_search_cols)
                for xh in xhits:
                    xh["source_game"] = other_game
                hits.extend(xhits)
            hits = hits[:max_results]

        result = []
        for h in hits:
            entry = {
                "id": h.get("id", ""),
                "name": h.get("name", ""),
                "path": h.get("path", ""),
                "category": h.get("category", ""),
                "source": h.get("source", ""),
                "root_type": h.get("root_type", ""),
                "block_count": h.get("block_count", 0),
                "has_particles": bool(h.get("has_particles", 0)),
                "has_behavior": bool(h.get("has_behavior", 0)),
                "has_controllers": bool(h.get("has_controllers", 0)),
                "snippet": (h.pop("content", "") or "")[:300].replace("\n", " ").strip(),
            }
            if "source_game" in h:
                entry["source_game"] = h["source_game"]
            result.append(entry)
        return result

    raise ValueError(f"Domain '{domain}' search not implemented yet.")


# ---------------------------------------------------------------------------
# semantic_search()
# ---------------------------------------------------------------------------

def semantic_search(
    domain: str,
    query: str,
    record_type: str = "",
    source: str = "",
    category: str = "",
    extends: str = "",
    mod_name: str = "",
    cross_game: bool = False,
    max_results: int = 10,
    game: str = "",
    *,
    db_dir: str,
    model_ready: bool = True,
) -> list[dict]:
    """Semantic (AI embedding) search across game data.

    Falls back to keyword search if *model_ready* is False or the
    embeddings index is unavailable.
    """
    if domain not in SEARCH_DOMAINS:
        raise ValueError(f"Unknown domain '{domain}'. Valid: {', '.join(sorted(SEARCH_DOMAINS))}")
    max_results = clamp(max_results)
    if not query or not query.strip():
        raise ValueError("Query cannot be empty.")

    _fallback_kw = dict(domain=domain, query=query, record_type=record_type,
                        source=source, category=category, extends=extends,
                        mod_name=mod_name, cross_game=cross_game,
                        max_results=max_results, game=game, db_dir=db_dir)

    if not model_ready:
        return search(**_fallback_kw)

    vec = get_vec_searcher(domain, game, db_dir)
    if vec is None:
        return search(**_fallback_kw)

    # -- Wiki --------------------------------------------------------------
    if domain == "wiki":
        db_path = get_db_path("wiki", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        row_map = batch_lookup(db_path, "pages", "filename", doc_ids)
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            row["snippet"] = row.pop("content", "")[:500].replace("\n", " ").strip()
            row["score"] = score_map[doc_id]
            hits.append(row)
        return hits

    # -- Scripts -----------------------------------------------------------
    if domain == "scripts":
        db_path = get_db_path("scripts", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        conn = open_db(db_path)
        placeholders = ",".join("?" * len(doc_ids))
        rows = conn.execute(
            f"SELECT * FROM scripts WHERE script_id IN ({placeholders})", doc_ids
        ).fetchall()
        row_map = {dict(r)["script_id"]: dict(r) for r in rows}
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            strip_hit(row)
            row["score"] = score_map[doc_id]
            hits.append(row)
        return hits

    # -- External scripts --------------------------------------------------
    if domain == "ext_scripts":
        ext_vec = get_vec_searcher("ext_scripts", game, db_dir)
        if ext_vec is None:
            return search(domain=domain, query=query, extends=extends,
                          mod_name=mod_name, max_results=max_results, game=game, db_dir=db_dir)
        ext_db = get_db_path("ext_scripts", game, db_dir)
        ext_results = ext_vec.search(query, k=max_results)
        ext_ids = [doc_id for doc_id, _ in ext_results]
        ext_score_map = {doc_id: score for doc_id, score in ext_results}
        ext_row_map = batch_lookup(ext_db, "ext_scripts", "script_id", ext_ids)
        hits = []
        for doc_id in ext_ids:
            row = ext_row_map.get(doc_id)
            if row is None:
                continue
            if mod_name and row.get("mod_name", "") != mod_name:
                continue
            strip_hit(row)
            row["score"] = ext_score_map[doc_id]
            hits.append(row)
        return hits

    # -- Records -----------------------------------------------------------
    if domain == "records":
        db_path = get_db_path("records", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        row_map = batch_lookup(db_path, "records", "form_key", doc_ids)
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            strip_hit(row)
            row["score"] = score_map[doc_id]
            hits.append(row)
        return hits

    # -- External records --------------------------------------------------
    if domain == "ext_records":
        ext_vec = get_vec_searcher("ext_records", game, db_dir)
        if ext_vec is None:
            return search(domain=domain, query=query, record_type=record_type,
                          mod_name=mod_name, max_results=max_results, game=game, db_dir=db_dir)
        ext_db = get_db_path("ext_records", game, db_dir)
        ext_results = ext_vec.search(query, k=max_results)
        ext_ids = [doc_id for doc_id, _ in ext_results]
        ext_score_map = {doc_id: score for doc_id, score in ext_results}
        ext_row_map = batch_lookup(ext_db, "ext_records", "form_key", ext_ids)
        hits = []
        for doc_id in ext_ids:
            row = ext_row_map.get(doc_id)
            if row is None:
                continue
            if mod_name and row.get("mod_name", "") != mod_name:
                continue
            strip_hit(row)
            row["score"] = ext_score_map[doc_id]
            hits.append(row)
        return hits

    # -- Behaviors (legacy alias → havok_behaviors) ------------------------
    if domain == "behaviors":
        db_path = get_db_path("havok", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        row_map = batch_lookup(db_path, "havok_behaviors", "id", doc_ids)
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            if source and row.get("source", "") != source:
                continue
            if category and row.get("category", "") != category:
                continue
            row["snippet"] = (row.pop("content", "") or "")[:300].replace("\n", " ").strip()
            row["score"] = score_map[doc_id]
            hits.append({
                "id": row.get("id", ""),
                "name": row.get("name", ""),
                "category": row.get("category", ""),
                "source": row.get("source", ""),
                "source_path": row.get("source_path", ""),
                "graph_path": row.get("graph_path", ""),
                "node_count": row.get("node_count", 0),
                "usable": row.get("usable", ""),
                "snippet": row.get("snippet", ""),
                "score": row.get("score", 0),
            })
        return hits

    # -- Havok (all entity types) ------------------------------------------
    if domain == "havok":
        db_path = get_db_path("havok", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        row_map = batch_lookup(db_path, "havok_behaviors", "id", doc_ids)
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            if source and row.get("source", "") != source:
                continue
            if category and row.get("category", "") != category:
                continue
            row["snippet"] = (row.pop("content", "") or "")[:300].replace("\n", " ").strip()
            row["score"] = score_map[doc_id]
            hits.append({
                "id": row.get("id", ""),
                "name": row.get("name", ""),
                "category": row.get("category", ""),
                "source": row.get("source", ""),
                "source_path": row.get("source_path", ""),
                "graph_path": row.get("graph_path", ""),
                "node_count": row.get("node_count", 0),
                "usable": row.get("usable", ""),
                "snippet": row.get("snippet", ""),
                "score": row.get("score", 0),
            })
        return hits

    # -- NIFs --------------------------------------------------------------
    if domain == "nifs":
        db_path = get_db_path("nifs", game, db_dir)
        results = vec.search(query, k=max_results)
        doc_ids = [doc_id for doc_id, _ in results]
        score_map = {doc_id: score for doc_id, score in results}
        row_map = batch_lookup(db_path, "nifs", "id", doc_ids)
        hits = []
        for doc_id in doc_ids:
            row = row_map.get(doc_id)
            if row is None:
                continue
            if source and row.get("source", "") != source:
                continue
            if category and row.get("category", "") != category:
                continue
            hits.append({
                "id": row.get("id", ""),
                "name": row.get("name", ""),
                "path": row.get("path", ""),
                "category": row.get("category", ""),
                "source": row.get("source", ""),
                "root_type": row.get("root_type", ""),
                "block_count": row.get("block_count", 0),
                "has_particles": bool(row.get("has_particles", 0)),
                "has_behavior": bool(row.get("has_behavior", 0)),
                "has_controllers": bool(row.get("has_controllers", 0)),
                "snippet": (row.pop("content", "") or "")[:300].replace("\n", " ").strip(),
                "score": score_map[doc_id],
            })
        return hits

    # Fallback for any unhandled domain
    return search(**_fallback_kw)


# ---------------------------------------------------------------------------
# search_by_keyword()
# ---------------------------------------------------------------------------

def search_by_keyword(
    keyword: str,
    record_type: str = "",
    max_results: int = 20,
    game: str = "",
    *,
    db_dir: str,
) -> list[dict]:
    """Find all records that have a specific keyword (by EditorID or FormKey)."""
    if not keyword or not keyword.strip():
        raise ValueError("keyword is required.")
    max_results = clamp(max_results)
    db_path = get_db_path("records", game, db_dir)
    conn = open_db(db_path)

    # Resolve keyword to FormKey if not already a FormKey
    if ":" not in keyword:
        kw_rows = conn.execute(
            "SELECT form_key FROM records WHERE LOWER(editor_id) = LOWER(?) AND record_type = 'Keywords'",
            (keyword,),
        ).fetchall()
        if not kw_rows:
            raise ValueError(f"Keyword '{keyword}' not found in Keywords records.")
        form_key = kw_rows[0][0]
    else:
        form_key = keyword

    # Find records whose keywords column contains this FormKey
    sql = "SELECT form_key, editor_id, record_type, name, source FROM records WHERE keywords LIKE ?"
    params: list = [f"%{form_key}%"]
    if record_type:
        sql += " AND record_type = ?"
        params.append(record_type)
    sql += " LIMIT ?"
    params.append(max_results)

    rows = conn.execute(sql, params).fetchall()
    if not rows:
        raise ValueError(f"No records found with keyword '{keyword}' (FormKey: {form_key}).")
    return [dict(r) for r in rows]
