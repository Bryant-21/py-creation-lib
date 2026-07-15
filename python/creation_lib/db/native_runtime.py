"""Single-point loader and Python wrappers for the Rust db_native submodule.

The native boundary is function-only. Stateful APIs use opaque native handle
IDs here, while Python classes preserve the public ergonomics expected by
callers in ``creation_lib.db`` and the preprocessors.
"""

from __future__ import annotations

from importlib import import_module
from typing import Any

_NATIVE_CAPABILITIES = (
    "database_open",
    "fts_search",
    "tokenize",
)
_NATIVE_MODULE: Any | None = None
_QUERY_PREFIXES = ("select", "with", "pragma", "explain", "values")


def _looks_like_db_native(module: Any | None) -> bool:
    if module is None:
        return False
    return any(callable(getattr(module, name, None)) for name in _NATIVE_CAPABILITIES)


def _load_native_module() -> Any:
    candidates = (
        "db_native",
        "db_native.db_native",
        "creation_lib._native.db_native",
    )
    for name in candidates:
        try:
            module = import_module(name)
        except ImportError:
            continue
        if _looks_like_db_native(module):
            return module

    try:
        umbrella = import_module("creation_lib._native")
    except ImportError as exc:
        raise RuntimeError("db_native is required for database operations") from exc
    module = getattr(umbrella, "db_native", None)
    if _looks_like_db_native(module):
        return module

    # The umbrella package may be a namespace package with the .pyd installed
    # as the private extension. Try importing the extension directly.
    try:
        pyd = import_module("creation_lib._native")
        module = getattr(pyd, "db_native", None)
        if _looks_like_db_native(module):
            return module
    except ImportError:
        pass

    raise RuntimeError("db_native is required for database operations")


def _native() -> Any:
    global _NATIVE_MODULE
    if _NATIVE_MODULE is None:
        _NATIVE_MODULE = _load_native_module()
    return _NATIVE_MODULE


def _looks_like_query(sql: str) -> bool:
    stripped = sql.lstrip().lower()
    return stripped.startswith(_QUERY_PREFIXES)


class Database:
    def __init__(self, handle: int):
        self._handle = handle

    @classmethod
    def open(cls, path: str, mode: str = "rw", load_vec: bool = False) -> "Database":
        return cls(_native().database_open(path, mode, load_vec))

    def path(self) -> str:
        return _native().database_path(self._handle)

    def is_write(self) -> bool:
        return _native().database_is_write(self._handle)

    def execute(
        self,
        sql: str,
        params: list[Any] | tuple[Any, ...] | None = None,
    ) -> "_QueryResult":
        if not params and not _looks_like_query(sql):
            _native().database_execute(self._handle, sql)
            return _QueryResult([])
        try:
            rows = _native().database_query_all(self._handle, sql, params)
        except Exception:
            _native().database_execute(self._handle, sql)
            rows = []
        return _QueryResult(rows)

    def execute_one(self, sql: str, params: list[Any] | None = None) -> int:
        return _native().database_execute_one(self._handle, sql, params)

    def query_all(
        self, sql: str, params: list[Any] | None = None
    ) -> list[dict[str, Any]]:
        return _native().database_query_all(self._handle, sql, params)

    def query_one(
        self, sql: str, params: list[Any] | None = None
    ) -> dict[str, Any] | None:
        rows = _native().database_query_all(self._handle, sql, params)
        return rows[0] if rows else None

    def close(self) -> None:
        handle = getattr(self, "_handle", None)
        if handle is not None:
            _native().database_close(handle)
            self._handle = None

    def __enter__(self) -> "Database":
        return self

    def __exit__(self, exc_type: Any, exc_value: Any, tb: Any) -> bool:
        self.close()
        return False

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


class _QueryResult:
    def __init__(self, rows: list[Any]):
        self._rows = list(rows)

    def fetchall(self) -> list[Any]:
        return list(self._rows)

    def fetchone(self) -> Any | None:
        return self._rows[0] if self._rows else None


class BulkInserter:
    def __init__(
        self,
        db_path: str,
        schema_json: str,
        *,
        fresh: bool = False,
        load_vec: bool = False,
    ):
        self._handle = _native().bulk_inserter_new(db_path, schema_json, fresh=fresh, load_vec=load_vec)
        self._closed_rows_inserted: int | None = None

    def execute(self, sql: str) -> None:
        _native().bulk_execute(self._handle, sql)

    def execute_params(self, sql: str, params: list[Any] | None = None) -> int:
        return _native().bulk_execute_params(self._handle, sql, params)

    def add_chunk(self, table: str, columns: dict[str, list[Any]]) -> int:
        return _native().bulk_add_chunk(self._handle, table, columns)

    def index_nifs(
        self,
        tasks: list[tuple[str, str]],
        source: str,
        source_path: str,
        *,
        max_size: int,
        workers: int,
        timing_log_path: str | None = None,
    ) -> dict[str, Any]:
        fn = getattr(_native(), "bulk_index_nifs", None)
        if not callable(fn):
            raise RuntimeError("db_native.bulk_index_nifs is not available")
        return fn(
            self._handle,
            tasks,
            source,
            source_path,
            max_size,
            workers,
            timing_log_path,
        )

    def index_records(
        self,
        yaml_root: str,
        *,
        sources: list[str] | None = None,
        workers: int = 0,
    ) -> dict[str, Any]:
        fn = getattr(_native(), "bulk_index_records", None)
        if not callable(fn):
            raise RuntimeError("db_native.bulk_index_records is not available")
        return fn(self._handle, yaml_root, sources, workers)

    def add_rows(self, table: str, rows: list[dict[str, Any]]) -> int:
        return _native().bulk_add_rows(self._handle, table, rows)

    def query_all(self, sql: str, params: list[Any] | None = None) -> list[tuple[Any, ...]]:
        return _native().bulk_query_all(self._handle, sql, params)

    def rebuild_fts(self, fts_table: str) -> None:
        _native().bulk_rebuild_fts(self._handle, fts_table)

    def rebuild_records_fts(self) -> None:
        """Rebuild records_fts after decompressing each row's content BLOB.

        records.content is zstd-compressed, so the standard FTS5 rebuild that
        reads bytes from the content table would tokenize them as empty strings.
        This method does the decompression in Rust and inserts plain text into
        the FTS index.
        """
        _native().bulk_rebuild_records_fts(self._handle)

    def create_indexes(self, sql: str) -> None:
        _native().bulk_create_indexes(self._handle, sql)

    def commit(self) -> None:
        _native().bulk_commit(self._handle)

    def finalize(self) -> int:
        inserted = _native().bulk_finalize(self._handle)
        self._closed_rows_inserted = inserted
        return inserted

    def rollback(self) -> None:
        if self._closed_rows_inserted is None:
            self._closed_rows_inserted = _native().bulk_rows_inserted(self._handle)
        _native().bulk_rollback(self._handle)

    def rows_inserted(self) -> int:
        if self._handle is None:
            return int(self._closed_rows_inserted or 0)
        return _native().bulk_rows_inserted(self._handle)

    def close(self) -> None:
        handle = getattr(self, "_handle", None)
        if handle is not None:
            if self._closed_rows_inserted is None:
                try:
                    self._closed_rows_inserted = _native().bulk_rows_inserted(handle)
                except Exception:
                    pass
            _native().bulk_close(handle)
            self._handle = None

    def __enter__(self) -> "BulkInserter":
        return self

    def __exit__(self, exc_type: Any, exc_value: Any, tb: Any) -> bool:
        if exc_type is None:
            self.finalize()
        else:
            self.rollback()
        self.close()
        return False

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


class Embedder:
    """Rust-backed Model2Vec embedder. Produces float32 vectors as bytes."""

    def __init__(self, repo_or_path: str):
        self._handle = _native().embedder_new(repo_or_path)

    @property
    def dim(self) -> int:
        return _native().embedder_dim(self._handle)

    def embed(self, texts: list[str]) -> bytes:
        """Encode a batch of texts. Returns ``len(texts) * dim * 4`` bytes of
        contiguous little-endian float32 data."""
        return _native().embedder_embed(self._handle, list(texts))

    def close(self) -> None:
        handle = getattr(self, "_handle", None)
        if handle is not None:
            _native().embedder_close(handle)
            self._handle = None

    def __enter__(self) -> "Embedder":
        return self

    def __exit__(self, exc_type: Any, exc_value: Any, tb: Any) -> bool:
        self.close()
        return False

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


class DirectoryIndex:
    def __init__(self, root: str, *, cache_dir: str | None = None):
        self._handle = _native().dir_index_new(root, cache_dir=cache_dir)

    def resolve(self, rel_path: str) -> str | None:
        return _native().dir_index_resolve(self._handle, rel_path)

    def contains(self, rel_path: str) -> bool:
        return _native().dir_index_contains(self._handle, rel_path)

    @property
    def file_count(self) -> int:
        return _native().dir_index_file_count(self._handle)

    @property
    def _lookup(self) -> dict[str, str]:
        return _native().dir_index_lookup(self._handle)

    def close(self) -> None:
        handle = getattr(self, "_handle", None)
        if handle is not None:
            _native().dir_index_close(handle)
            self._handle = None

    def __enter__(self) -> "DirectoryIndex":
        return self

    def __exit__(self, exc_type: Any, exc_value: Any, tb: Any) -> bool:
        self.close()
        return False

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


def fts_search(*args: Any, **kwargs: Any) -> Any:
    return _native().fts_search(*args, **kwargs)


def exact_lookup(*args: Any, **kwargs: Any) -> Any:
    return _native().exact_lookup(*args, **kwargs)


def batch_lookup(*args: Any, **kwargs: Any) -> Any:
    return _native().batch_lookup(*args, **kwargs)


def count_by_column(*args: Any, **kwargs: Any) -> Any:
    return _native().count_by_column(*args, **kwargs)


def fallback_word_search(*args: Any, **kwargs: Any) -> Any:
    return _native().fallback_word_search(*args, **kwargs)


def fts5_escape(*args: Any, **kwargs: Any) -> Any:
    return _native().fts5_escape(*args, **kwargs)


def tokenize(*args: Any, **kwargs: Any) -> Any:
    return _native().tokenize(*args, **kwargs)


def vec_bulk_insert(*args: Any, **kwargs: Any) -> Any:
    return _native().vec_bulk_insert(*args, **kwargs)


def vec_knn_search(*args: Any, **kwargs: Any) -> Any:
    return _native().vec_knn_search(*args, **kwargs)


def vec_create_table(*args: Any, **kwargs: Any) -> Any:
    return _native().vec_create_table(*args, **kwargs)


def vec_drop_table(*args: Any, **kwargs: Any) -> Any:
    return _native().vec_drop_table(*args, **kwargs)


def clear_registry(*args: Any, **kwargs: Any) -> Any:
    return _native().clear_registry(*args, **kwargs)

__all__ = [
    "Database",
    "BulkInserter",
    "DirectoryIndex",
    "Embedder",
    "fts_search",
    "exact_lookup",
    "batch_lookup",
    "count_by_column",
    "fallback_word_search",
    "fts5_escape",
    "tokenize",
    "vec_bulk_insert",
    "vec_knn_search",
    "vec_create_table",
    "vec_drop_table",
    "clear_registry",
]
