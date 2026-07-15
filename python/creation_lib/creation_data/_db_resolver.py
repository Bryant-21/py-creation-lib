"""Database path resolution, availability checks, and shared search helpers."""

from __future__ import annotations

import os

from creation_lib.db.db import open_db, fts5_escape
from creation_lib.db.embeddings import VecSearcher

from ._config import DOMAIN_EMBEDDING_TABLE

# Cached VecSearcher instances keyed by (game, domain, db_dir)
_vec_searchers: dict[str, VecSearcher | None] = {}

_SHARED_WIKI_DB_GAME = {
    "fnv": "fo3",
}


def resolve_game(game: str, default: str = "") -> str:
    """Resolve game ID with caller-provided default. No env var reads here."""
    if game:
        return game
    return default or "fo4"


def _resolve_db_dir(db_dir: str) -> str:
    if db_dir:
        return db_dir
    raise ValueError("db_dir is required")


def _candidate_db_dirs(db_dir: str) -> list[str]:
    return [_resolve_db_dir(db_dir)]


def _candidate_filenames(domain: str, game: str) -> list[str]:
    db_domain = "external_mods" if domain.startswith("ext_") else domain
    if db_domain == "records":
        return [f"{game}_records.db", f"records_{game}.db"]
    if db_domain == "wiki":
        db_game = _SHARED_WIKI_DB_GAME.get(game, game)
        filenames = [f"{db_game}_wiki.db"]
        if db_game != game:
            filenames.append(f"{game}_wiki.db")
        return filenames
    return [f"{game}_{db_domain}.db"]


def get_db_path(domain: str, game: str, db_dir: str) -> str:
    """Resolve database path for a domain and game. Raises FileNotFoundError if missing."""
    for candidate_dir in _candidate_db_dirs(db_dir):
        for filename in _candidate_filenames(domain, game):
            path = os.path.join(candidate_dir, filename)
            if os.path.exists(path):
                return path
    raise FileNotFoundError(
        f"No {domain} database for '{game}'. "
        f"Run: modkit index build --domain {domain} --game {game}"
    )


def db_available(domain: str, game: str, db_dir: str) -> bool:
    """Check if a database file exists for a domain and game."""
    for candidate_dir in _candidate_db_dirs(db_dir):
        for filename in _candidate_filenames(domain, game):
            if os.path.isfile(os.path.join(candidate_dir, filename)):
                return True
    return False


def get_vec_searcher(domain: str, game: str, db_dir: str) -> VecSearcher | None:
    """Lazy-load sqlite-vec searcher for a domain + game. Returns None if unavailable."""
    db_dir = _resolve_db_dir(db_dir)
    cache_key = f"{game}:{domain}:{db_dir}"
    if cache_key in _vec_searchers:
        return _vec_searchers[cache_key]
    if not db_available(domain, game, db_dir):
        _vec_searchers[cache_key] = None
        return None
    try:
        db_path = get_db_path(domain, game, db_dir)
        table_name = DOMAIN_EMBEDDING_TABLE.get(domain, "embeddings")
        searcher = VecSearcher(db_path, table_name=table_name)
        _vec_searchers[cache_key] = searcher
        return searcher
    except Exception:
        _vec_searchers[cache_key] = None
        return None


def havok_search(
    db_path: str,
    query: str,
    entity_type: str | None = None,
    category: str | None = None,
    source: str | None = None,
    max_results: int = 10,
    search_columns: list[str] | None = None,
) -> list[dict]:
    """Search the content-less havok_fts table and fetch entity details.

    Unlike fts_search(), this queries havok_fts directly (no JOIN with a
    backing table) then fetches full rows from the typed entity tables.
    """
    conn = open_db(db_path)
    escaped = fts5_escape(query)
    if not escaped:
        return []

    where_parts = ["havok_fts MATCH ?"]
    params: list = [escaped]
    if entity_type:
        where_parts.append("entity_type = ?")
        params.append(entity_type)

    sql = f"""
        SELECT name, id, entity_type, category, rank
        FROM havok_fts
        WHERE {' AND '.join(where_parts)}
        ORDER BY rank
        LIMIT ?
    """
    params.append(max_results)
    rows = conn.execute(sql, params).fetchall()

    table_map = {
        "project": "havok_projects",
        "behavior": "havok_behaviors",
        "animation": "havok_animations",
        "skeleton": "havok_skeletons",
        "manifest": "havok_manifests",
    }

    results = []
    for row in rows:
        etype = row["entity_type"]
        eid = row["id"]
        table = table_map.get(etype)
        if table:
            detail = conn.execute(f"SELECT * FROM {table} WHERE id = ?", (eid,)).fetchone()
            if detail:
                result = dict(detail)
                result["entity_type"] = etype
                if category and result.get("category", "") != category:
                    continue
                if source and result.get("source", "") != source:
                    continue
                results.append(result)

    return results
