"""High-level search API for UI and non-MCP consumers.

Wraps the raw db.py functions with domain knowledge (db paths, table names).
"""

import os
from pathlib import Path

from .db import (
    fts_search,
    exact_lookup,
    count_by_column,
    fallback_word_search,
    open_db,
)


# Domain -> (table, fts_table, id_column)  — filename is built dynamically
_DOMAIN_META = {
    "records":     ("records",      "records_fts",      "form_key"),
    "scripts":     ("scripts",      "scripts_fts",      "script_name"),
    "wiki":        ("pages",        "pages_fts",        "filename"),
    "behaviors":   ("behaviors",    "behaviors_fts",    "id"),
    "nifs":        ("nifs",         "nifs_fts",         "id"),
    "ext_records": ("ext_records",  "ext_records_fts",  "form_key"),
    "ext_scripts": ("ext_scripts",  "ext_scripts_fts",  "script_id"),
}

# Domain -> DB file suffix (most are just the domain name)
_DOMAIN_DB_NAME = {
    "ext_records": "external_mods",
    "ext_scripts": "external_mods",
}

_SHARED_WIKI_DB_GAME = {
    "fnv": "fo3",
}


def _domain_db_file(domain: str, game: str) -> str:
    """Return the database filename for domain + game."""
    db_name = _DOMAIN_DB_NAME.get(domain, domain)
    if db_name == "wiki":
        game = _SHARED_WIKI_DB_GAME.get(game, game)
    return f"{game}_{db_name}.db"


class GameDataStore:
    """High-level search API for UI and non-MCP consumers.

    Usage::

        store = GameDataStore(db_dir="data")
        results = store.search("records", "laser gun")

        store = GameDataStore(db_dir="data", game="skyrimse")
        results = store.search("records", "iron sword")
    """

    def __init__(self, db_dir: str, game: str = "fo4"):
        if not db_dir:
            raise ValueError("db_dir is required")
        self._db_dir = Path(db_dir)
        self._game = game

    def _db_path(self, domain: str) -> str:
        if domain not in _DOMAIN_META:
            raise ValueError(f"Unknown domain '{domain}'. Valid: {', '.join(sorted(_DOMAIN_META))}")
        return str(self._db_dir / _domain_db_file(domain, self._game))

    def _config(self, domain: str):
        if domain not in _DOMAIN_META:
            raise ValueError(f"Unknown domain '{domain}'.")
        db_file = _domain_db_file(domain, self._game)
        table, fts_table, id_col = _DOMAIN_META[domain]
        return (db_file, table, fts_table, id_col)

    def search(
        self,
        domain: str,
        query: str,
        limit: int = 20,
        offset: int = 0,
        **filters,
    ) -> list[dict]:
        """Search a domain using FTS5 full-text search.

        Args:
            domain: One of "records", "scripts", "wiki", "behaviors",
                    "nifs", "ext_records", "ext_scripts".
            query: Search text.
            limit: Maximum results.
            **filters: Domain-specific filters (record_type, source, extends, etc.)

        Returns:
            List of result dicts.
        """
        db_file, table, fts_table, id_col = self._config(domain)
        db_path = str(self._db_dir / db_file)

        clean_filters = {k: v for k, v in filters.items() if v}

        hits = fts_search(db_path, table, fts_table, query, clean_filters or None, limit, offset=offset)
        hits = fallback_word_search(
            db_path, table, fts_table, query, hits, id_col,
            clean_filters or None, limit,
        )
        return hits

    def get_content(self, domain: str, item_id: str) -> dict | None:
        """Retrieve a single item by its primary key.

        Returns the full row as a dict, or None if not found.
        """
        db_file, table, _, id_col = self._config(domain)
        db_path = str(self._db_dir / db_file)
        return exact_lookup(db_path, table, id_col, item_id)

    def list_items(self, domain: str, group_by: str) -> dict[str, int]:
        """Count items grouped by a column.

        Args:
            domain: The domain to query.
            group_by: Column to group by (e.g. "record_type", "category").

        Returns:
            {value: count} dict.
        """
        db_file, table, _, _ = self._config(domain)
        db_path = str(self._db_dir / db_file)
        return count_by_column(db_path, table, group_by)

    def is_available(self, domain: str) -> bool:
        """Check if a domain's database file exists."""
        try:
            db_path = self._db_path(domain)
            return os.path.isfile(db_path)
        except ValueError:
            return False


# Backward compatibility
Fo4DataStore = GameDataStore
