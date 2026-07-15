"""Game data search library — SQLite FTS5 engine.

Public API:
    from creation_lib.db.db import fts_search, exact_lookup, ...
    from creation_lib.db.tokenizer import tokenize
    from creation_lib.db.store import GameDataStore
"""

from .db import (
    open_db,
    fts_search,
    fts5_escape,
    exact_lookup,
    batch_lookup,
    count_by_column,
    fallback_word_search,
)
from .tokenizer import tokenize
from .store import GameDataStore, Fo4DataStore  # Fo4DataStore is alias
from .record_loader import RecordLoader

__all__ = [
    "open_db",
    "fts_search",
    "fts5_escape",
    "exact_lookup",
    "batch_lookup",
    "count_by_column",
    "fallback_word_search",
    "tokenize",
    "GameDataStore",
    "Fo4DataStore",  # backward compat
    "RecordLoader",
]
