"""Embedding utilities for semantic search.

Uses a Rust-side Model2Vec embedder (via ``creation_lib.db.native_runtime.Embedder``)
so no Python-level ML dependencies (torch / sentence-transformers) are
pulled in. The SQLite/sqlite-vec storage side was already Rust.

Default model is ``minishlab/potion-base-32M`` — a 512-dim static
distillation that HuggingFace-Hub-downloads on first use.
"""

from __future__ import annotations

from .native_runtime import (
    Embedder,
    vec_bulk_insert,
    vec_create_table,
    vec_drop_table,
    vec_knn_search,
)

MODEL_NAME = "minishlab/potion-base-32M"

_embedder: Embedder | None = None
_embedder_source: str | None = None


def get_embedder(model: str | None = None) -> Embedder:
    """Lazy-load (and cache) the Rust embedder. Passing a different model
    name transparently swaps the cached instance."""
    global _embedder, _embedder_source
    target = model or MODEL_NAME
    if _embedder is None or _embedder_source != target:
        if _embedder is not None:
            _embedder.close()
        _embedder = Embedder(target)
        _embedder_source = target
    return _embedder


def embed_texts(
    texts: list[str],
    model: str | None = None,
    # Kept for backward compatibility; the Rust embedder batches internally.
    batch_size: int = 256,  # noqa: ARG001
    show_progress: bool = False,  # noqa: ARG001
) -> bytes:
    """Embed a list of texts. Returns ``len(texts) * dim * 4`` bytes of
    contiguous little-endian float32 data (matches what ``vec_bulk_insert``
    expects)."""
    return get_embedder(model).embed(list(texts))


def build_vec_index(
    texts: list[str],
    doc_ids: list[str],
    db_path: str,
    table_name: str = "embeddings",
    batch_size: int = 256,  # noqa: ARG001
    chunk_size: int = 5000,
    model: str | None = None,
) -> None:
    """Build a sqlite-vec index in an existing SQLite database.

    Streams embeddings in chunks; each chunk is bulk-inserted through the
    Rust extension in a single transaction.
    """
    assert len(texts) == len(doc_ids), "texts and doc_ids must have same length"

    # Deduplicate by doc_id (keep first occurrence).
    seen: set[str] = set()
    unique_texts: list[str] = []
    unique_ids: list[str] = []
    for text, doc_id in zip(texts, doc_ids):
        if doc_id not in seen:
            seen.add(doc_id)
            unique_texts.append(text)
            unique_ids.append(doc_id)
    if len(unique_ids) < len(doc_ids):
        print(f"  Deduplicated: {len(doc_ids)} -> {len(unique_ids)} documents")
    texts, doc_ids = unique_texts, unique_ids

    embedder = get_embedder(model)
    dim = embedder.dim

    vec_create_table(db_path, table_name, dim)

    total = len(texts)
    model_label = model or MODEL_NAME
    print(f"  Embedding {total} documents with {model_label} (streaming, chunk={chunk_size})...")

    inserted = 0
    for start in range(0, total, chunk_size):
        end = min(start + chunk_size, total)
        chunk_texts = texts[start:end]
        chunk_ids = doc_ids[start:end]

        payload = embedder.embed(chunk_texts)
        vec_bulk_insert(db_path, table_name, chunk_ids, payload, dim)

        inserted += len(chunk_ids)
        print(f"    {inserted}/{total} embedded", end="\r", flush=True)

    print(f"  sqlite-vec index saved: {total} vectors, dim={dim}    ")
    print(f"    {db_path} (table: {table_name})")


class VecSearcher:
    """Load a sqlite-vec index and perform semantic search.

    Returns list of ``(doc_id, similarity)`` tuples where
    ``similarity = 1 - distance``.
    """

    def __init__(
        self,
        db_path: str,
        table_name: str = "embeddings",
        model: str | None = None,
    ):
        self.db_path = db_path
        self.table_name = table_name
        self.model = model

    def search(self, query: str, k: int = 10) -> list[tuple[str, float]]:
        payload = get_embedder(self.model).embed([query])
        return vec_knn_search(self.db_path, self.table_name, payload, k)


def drop_vec_index(db_path: str, table_name: str = "embeddings") -> None:
    vec_drop_table(db_path, table_name)


__all__ = [
    "MODEL_NAME",
    "get_embedder",
    "embed_texts",
    "build_vec_index",
    "VecSearcher",
    "drop_vec_index",
]
