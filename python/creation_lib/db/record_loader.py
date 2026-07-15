"""Load game records from SQLite FTS5 indexes."""
from __future__ import annotations

import os
from collections.abc import Sequence

import zstandard as _zstd

from creation_lib.db.native_runtime import Database
from creation_lib.esp.record_types import record_type_signature

_ZSTD_DECOMPRESSOR = _zstd.ZstdDecompressor()
_ZSTD_MAGIC = b"\x28\xb5\x2f\xfd"


def _decode_record_content(value):
    """Decode a value from `records.content` to text.

    `records.content` is stored as a zstd-compressed BLOB. Older DBs still hold
    plain TEXT — pass through if a `str` arrives, and fall back to UTF-8 decode
    if a BLOB lacks the zstd magic (defensive for partially-migrated files).
    """
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if isinstance(value, (bytes, bytearray, memoryview)):
        b = bytes(value)
        if not b:
            return ""
        if b[:4] == _ZSTD_MAGIC:
            # streaming decompressor — Rust's zstd::encode_all omits the content
            # size in the frame header, so the simple .decompress(b) path errs
            # with "could not determine content size in frame header".
            return _ZSTD_DECOMPRESSOR.decompressobj().decompress(b).decode("utf-8")
        return b.decode("utf-8", errors="replace")
    return value


class RecordLoader:
    """Load and search game records from a SQLite records database."""

    def __init__(self, db_path: str):
        if not os.path.isfile(db_path):
            raise FileNotFoundError(f"Records DB not found: {db_path}")
        self._db_path = db_path
        self._conn: Database | None = None
        self._editor_id_type_index: dict[str, dict[str, list[dict]]] = {}
        self._editor_id_index: dict[str, list[dict]] | None = None
        self._form_key_index: dict[str, dict] | None = None

    def _connect(self) -> Database:
        if self._conn is None:
            self._conn = Database.open(self._db_path, "ro")
        return self._conn

    def close(self) -> None:
        conn = self._conn
        if conn is not None:
            conn.close()
            self._conn = None

    def load_by_form_key(self, form_key: str) -> dict | None:
        """Load a single record by FormKey. Returns None if not found.

        Hot path is backed by an in-memory ``_fk_index`` of lightweight
        metadata (form_key, editor_id, record_type, name, source). Eliminates
        the per-FK SQLite query that dominated ``_fix_invalid_target_formkeys``
        on large plugins (54k unbatched lookups in FO76→FO4 iter 1).

        If the caller needs the heavier ``yaml_path`` or ``content`` columns,
        we fall through to a one-off SQLite query — that path is rare and
        not worth bloating the in-memory index with multi-GB content fields.
        """
        meta = self._fk_index().get(form_key)
        if meta is None:
            return None
        return dict(meta)

    def _fk_index(self) -> dict[str, dict]:
        index = self._form_key_index
        if index is not None:
            return index
        # Cache lightweight columns + yaml_path (a path string, small).
        # The omitted ``content`` field is the multi-GB driver and is only
        # consulted by the rarely-used ``load_full_yaml``.
        rows = self._connect().query_all(
            "SELECT form_key, editor_id, record_type, name, source, yaml_path FROM records"
        )
        built: dict[str, dict] = {}
        for record in rows:
            fk = record.get("form_key")
            if fk:
                built[fk] = record
        self._form_key_index = built
        return built

    def load_full_yaml(self, form_key: str) -> str | None:
        """Load the full YAML content for a record from its yaml_path on disk."""
        # ``load_by_form_key`` now returns a lightweight metadata-only record;
        # the heavier ``yaml_path``/``content`` fields are fetched on demand.
        row = self._connect().query_one(
            "SELECT yaml_path, content FROM records WHERE form_key = ?",
            [form_key],
        )
        if not row:
            return None
        yaml_path = row.get("yaml_path", "") or ""
        if yaml_path and os.path.isfile(yaml_path):
            with open(yaml_path, encoding="utf-8", errors="replace") as f:
                return f.read()
        return _decode_record_content(row.get("content"))

    def search_by_editor_id(self, editor_id: str) -> list[dict]:
        """Case-insensitive exact match on EditorID.

        Backed by a lazy in-memory index built on first call. Eliminates the
        per-record SQLite query that dominated `phase_translate` on large
        plugins (75s for 5000 records under FO76→FO4 conversion). The
        full-table snapshot is bounded by the records DB size (~200k rows).
        """
        return list(self._eid_index().get(editor_id.lower(), []))

    def _eid_index(self) -> dict[str, list[dict]]:
        index = self._editor_id_index
        if index is not None:
            return index
        rows = self._connect().query_all(
            "SELECT form_key, editor_id, record_type, name, source FROM records"
        )
        built: dict[str, list[dict]] = {}
        for record in rows:
            key = (record.get("editor_id") or "").lower()
            if not key:
                continue
            built.setdefault(key, []).append(record)
        self._editor_id_index = built
        return built

    def preload_indices(self) -> None:
        """Force the editor_id and form_key indices to be built. Call before
        a hot loop to front-load the one-time DB scans outside the timed
        work.
        """
        self._eid_index()
        self._fk_index()

    def search_by_editor_id_and_type(self, editor_id: str, record_type: str) -> list[dict]:
        """Case-insensitive EditorID match filtered by record type or signature."""
        for candidate in self._record_type_lookup_candidates(record_type):
            results = self._search_by_editor_id_and_exact_type(editor_id, candidate)
            if results:
                return results

        expected_sig = record_type_signature(record_type)
        if not expected_sig:
            return []

        return [
            record for record in self.search_by_editor_id(editor_id)
            if record_type_signature(record.get("record_type", "")) == expected_sig
        ]

    @staticmethod
    def _record_type_lookup_candidates(record_type: str) -> list[str]:
        candidates: list[str] = []
        for candidate in (record_type, record_type_signature(record_type)):
            if candidate and candidate not in candidates:
                candidates.append(candidate)
        return candidates

    def _search_by_editor_id_and_exact_type(
        self,
        editor_id: str,
        record_type: str,
    ) -> list[dict]:
        index = self._editor_id_type_index.get(record_type)
        if index is None:
            rows = self._connect().query_all(
                "SELECT form_key, editor_id, record_type, name, source "
                "FROM records WHERE record_type = ?",
                [record_type],
            )

            index = {}
            for record in rows:
                key = (record.get("editor_id") or "").lower()
                index.setdefault(key, []).append(record)
            self._editor_id_type_index[record_type] = index

        return list(index.get(editor_id.lower(), []))

    def list_record_types(self) -> list[str]:
        """Return distinct record types in the database, sorted."""
        rows = self._connect().query_all(
            "SELECT DISTINCT record_type FROM records ORDER BY record_type"
        )
        return [r["record_type"] for r in rows if r.get("record_type")]

    def list_by_type(self, record_type: str, limit: int = 500) -> list[dict]:
        """List records of a given type, ordered by editor_id."""
        return self._connect().query_all(
            "SELECT form_key, editor_id, record_type, name, source "
            "FROM records WHERE record_type = ? ORDER BY editor_id LIMIT ?",
            [record_type, limit],
        )

    def list_by_types(self, record_types: Sequence[str], limit: int = 500) -> list[dict]:
        """List records for multiple record types, ordered by editor_id."""
        selected = [record_type for record_type in record_types if record_type]
        if not selected:
            return []

        placeholders = ", ".join("?" for _ in selected)
        return self._connect().query_all(
            "SELECT form_key, editor_id, record_type, name, source "
            f"FROM records WHERE record_type IN ({placeholders}) "
            "ORDER BY editor_id LIMIT ?",
            [*selected, limit],
        )

    def search(
        self,
        query: str,
        record_type: str = "",
        *,
        record_types: Sequence[str] | None = None,
        limit: int = 50,
    ) -> list[dict]:
        """Search records by editor_id and name. Uses LIKE for partial matching."""
        if len(query.strip()) < 2:
            return []
        return self._like_search(query.strip(), record_type, record_types, limit)

    def _like_search(
        self,
        query: str,
        record_type: str,
        record_types: Sequence[str] | None,
        limit: int,
    ) -> list[dict]:
        """LIKE-based search -- supports partial matching at any position."""
        like = f"%{query}%"
        selected = [value for value in (record_types or []) if value]
        conn = self._connect()
        if selected:
            placeholders = ", ".join("?" for _ in selected)
            return conn.query_all(
                "SELECT form_key, editor_id, record_type, name, source "
                "FROM records WHERE (editor_id LIKE ? OR name LIKE ?) "
                f"AND record_type IN ({placeholders}) "
                "ORDER BY editor_id LIMIT ?",
                [like, like, *selected, limit],
            )
        if record_type:
            return conn.query_all(
                "SELECT form_key, editor_id, record_type, name, source "
                "FROM records WHERE (editor_id LIKE ? OR name LIKE ?) "
                "AND record_type = ? ORDER BY editor_id LIMIT ?",
                [like, like, record_type, limit],
            )
        return conn.query_all(
            "SELECT form_key, editor_id, record_type, name, source "
            "FROM records WHERE editor_id LIKE ? OR name LIKE ? "
            "ORDER BY editor_id LIMIT ?",
            [like, like, limit],
        )

    def get_forward_refs(self, form_key: str) -> list[str]:
        """Get all FormKeys that this record references via record_refs table."""
        try:
            rows = self._connect().query_all(
                "SELECT referenced_form_key FROM record_refs "
                "WHERE referencing_form_key = ?",
                [form_key],
            )
        except Exception:
            return []
        return [r["referenced_form_key"] for r in rows if r.get("referenced_form_key")]
