"""SQLite FTS5 search helpers — thin shim over the Rust db_native backend."""

from __future__ import annotations

from typing import Any

from .native_runtime import Database
from .native_runtime import (
    batch_lookup as _batch_lookup,
    count_by_column as _count_by_column,
    exact_lookup as _exact_lookup,
    fallback_word_search as _fallback_word_search,
    fts5_escape as _fts5_escape,
    fts_search as _fts_search,
)


def open_db(db_path: str) -> Database:
    """Open a database handle with sqlite3-style read-query compatibility."""
    return Database.open(db_path)


def fts5_escape(query: str) -> str:
    return _fts5_escape(query)


def fts_search(
    db_path: str,
    table: str,
    fts_table: str,
    query: str,
    filters: dict[str, str] | None = None,
    max_results: int = 10,
    columns: str = "t.*",
    search_columns: list[str] | None = None,
    offset: int = 0,
) -> list[dict]:
    return _fts_search(
        db_path,
        table,
        fts_table,
        query,
        filters=filters,
        max_results=max_results,
        columns=columns,
        search_columns=search_columns,
        offset=offset,
    )


def exact_lookup(db_path: str, table: str, key_column: str, key_value: str) -> dict | None:
    return _exact_lookup(db_path, table, key_column, key_value)


def batch_lookup(
    db_path: str,
    table: str,
    key_column: str,
    key_values: list[str],
    columns: str = "*",
) -> dict[str, dict]:
    return _batch_lookup(db_path, table, key_column, list(key_values), columns)


def count_by_column(db_path: str, table: str, column: str) -> dict[str, int]:
    return _count_by_column(db_path, table, column)


def fallback_word_search(
    db_path: str,
    table: str,
    fts_table: str,
    query: str,
    existing_hits: list[dict],
    id_column: str,
    filters: dict[str, str] | None = None,
    max_results: int = 10,
    search_columns: list[str] | None = None,
) -> list[dict]:
    return _fallback_word_search(
        db_path,
        table,
        fts_table,
        query,
        list(existing_hits),
        id_column,
        filters=filters,
        max_results=max_results,
        search_columns=search_columns,
    )


__all__ = [
    "open_db",
    "fts_search",
    "fts5_escape",
    "exact_lookup",
    "batch_lookup",
    "count_by_column",
    "fallback_word_search",
]
