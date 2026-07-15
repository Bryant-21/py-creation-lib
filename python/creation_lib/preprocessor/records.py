"""Build records.db from ESP authoring-dir YAML.

The heavy lifting (YAML parse, field extraction, ref walking, tokenization,
SQLite ingest) runs in `db_native::records_indexer`. This module is a thin
Python entrypoint: parse CLI args, prep the database, delegate, finalize FTS.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.db.native_runtime import BulkInserter

# Game ID → YAML source subdirectory name under data/.
GAME_ESM_YAML_DIR = {
    "fo4": "fo4_esm_yaml",
    "skyrimse": "skyrimse_esm_yaml",
    "starfield": "starfield_esm_yaml",
    "fo76": "fo76_esm_yaml",
    "fo3": "fo3_esm_yaml",
    "fnv": "fnv_esm_yaml",
}

RECORD_COLS = [
    "form_key", "editor_id", "editor_id_tokens", "record_type", "name",
    "name_tokens", "source", "keywords", "yaml_path", "content", "node_index",
]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "records", "pk": "form_key", "on_conflict": "REPLACE", "columns": RECORD_COLS},
        {"name": "record_refs",
         "columns": ["referencing_form_key", "referenced_form_key"]},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS records (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    editor_id_tokens TEXT,
    record_type TEXT,
    name TEXT,
    name_tokens TEXT,
    source TEXT,
    keywords TEXT,
    yaml_path TEXT,
    content BLOB,
    node_index INTEGER
);
CREATE VIRTUAL TABLE IF NOT EXISTS records_fts USING fts5(
    editor_id_tokens, name_tokens, keywords, content,
    content='records', content_rowid='rowid'
);
CREATE TABLE IF NOT EXISTS record_refs (
    referencing_form_key TEXT,
    referenced_form_key TEXT
);
"""

CREATE_INDEXES_SQL = """
CREATE INDEX IF NOT EXISTS idx_records_type ON records(record_type);
CREATE INDEX IF NOT EXISTS idx_records_source ON records(source);
CREATE INDEX IF NOT EXISTS idx_records_editor_id ON records(editor_id);
CREATE INDEX IF NOT EXISTS idx_records_node_index ON records(node_index) WHERE node_index IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recref_referenced ON record_refs(referenced_form_key);
CREATE INDEX IF NOT EXISTS idx_recref_referencing ON record_refs(referencing_form_key);
"""


def parse_filename(filename: str) -> tuple[str | None, str | None]:
    """Parse 'EditorID - FormID_Plugin.yaml' → (editor_id, form_key).

    Mirrors the same routine in `db_native::records_indexer::parse_filename`;
    kept on the Python side because `py_creation_lib/python/creation_lib/nif/previs_merge.py` calls it
    against arbitrary filenames outside the indexing path.
    """
    if not filename.endswith(".yaml"):
        return None, None
    stem = filename[:-5]
    parts = stem.split(" - ", 1)
    if len(parts) != 2:
        return None, None
    editor_id = parts[0].strip()
    fk_raw = parts[1].strip()
    underscore_idx = fk_raw.find("_")
    if underscore_idx > 0:
        form_key = fk_raw[:underscore_idx] + ":" + fk_raw[underscore_idx + 1:]
    else:
        form_key = fk_raw
    return editor_id, form_key


def build_db(esm_yaml_dir, db_path, build_embeddings=False):
    """Full rebuild: drop the DB, re-create schema, index every record."""
    db_path = Path(db_path)
    os.makedirs(str(db_path.parent), exist_ok=True)

    print(f"  Indexing records from {esm_yaml_dir} ...")

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        summary = bulk.index_records(str(esm_yaml_dir))
        print("  Building indexes...")
        bulk.create_indexes(CREATE_INDEXES_SQL)
        print("  Rebuilding FTS index...")
        # records.content is zstd-compressed, so the standard FTS5 'rebuild'
        # command would tokenize raw zstd bytes as empty strings — losing
        # content search. Use the records-specific rebuild that decompresses
        # each row before insert.
        bulk.rebuild_records_fts()

    total = summary["indexed"]
    type_counts = summary["type_counts"]
    source_counts = summary["source_counts"]
    elapsed = summary["elapsed_seconds"]
    print(
        f"  Indexed {total:,} records, "
        f"{summary['refs']:,} cross-refs in {elapsed:.2f}s"
    )

    if build_embeddings and total:
        _build_embeddings(db_path)

    return total, type_counts, source_counts


def incremental_update(esm_yaml_dir, db_path, sources, delete_sources=None):
    """Re-index only `sources`, optionally deleting stale rows for `delete_sources` first."""
    db_path = Path(db_path)
    if not db_path.exists():
        print("Records DB not found — falling back to full build.")
        total, _, _ = build_db(str(esm_yaml_dir), str(db_path))
        return total

    sources = list(sources or [])
    delete_sources = list(delete_sources or [])

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=False) as bulk:
        bulk.execute(CREATE_TABLES_DDL)

        for src in delete_sources:
            bulk.execute_params(
                "DELETE FROM record_refs WHERE referencing_form_key IN "
                "(SELECT form_key FROM records WHERE source = ?)",
                [src],
            )
            bulk.execute_params("DELETE FROM records WHERE source = ?", [src])

        summary = bulk.index_records(str(esm_yaml_dir), sources=sources)
        print("  Rebuilding FTS index...")
        bulk.rebuild_fts("records_fts")

    total = summary["indexed"]
    print(f"  Incremental update complete: {total:,} records added/updated.")
    return total


def _build_embeddings(db_path: Path) -> None:
    """Build the sqlite-vec embedding index. Reads back from the DB so the
    embedding text matches what the indexer actually wrote, including resolved
    keyword EditorIDs.
    """
    print("  Building FormKey → EditorID lookup for keyword resolution...")
    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=False) as bulk:
        rows = bulk.query_all(
            "SELECT form_key, editor_id FROM records WHERE editor_id != ''"
        )
    fk_to_eid = {r[0]: r[1] for r in rows}
    print(f"    {len(fk_to_eid):,} FormKeys mapped")

    print("  Resolving keywords in embedding texts...")
    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=False) as bulk:
        rows = bulk.query_all(
            "SELECT form_key, editor_id, name, record_type, keywords FROM records ORDER BY rowid"
        )
    embed_texts: list[str] = []
    embed_doc_ids: list[str] = []
    for r in rows:
        form_key, editor_id, name, record_type, keywords_str = r
        resolved_kws = ""
        if keywords_str:
            kw_parts = [fk_to_eid[fk] for fk in keywords_str.split() if fk in fk_to_eid]
            resolved_kws = " ".join(kw_parts)
        embed_texts.append(
            f"{editor_id} {name or ''} {record_type} {resolved_kws}".strip()
        )
        embed_doc_ids.append(form_key)

    from creation_lib.db.embeddings import build_vec_index
    print(f"\nBuilding sqlite-vec index ({len(embed_texts)} documents)...")
    build_vec_index(embed_texts, embed_doc_ids, str(db_path))


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build records search database")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--embeddings", action="store_true")
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--esm-yaml-dir", default=None)
    parser.add_argument("--incremental", action="store_true")
    parser.add_argument("--sources", default="")
    parser.add_argument("--delete-sources", default="")
    args = parser.parse_args()

    game = args.game
    if game not in GAME_PROFILES:
        print(
            f"ERROR: Invalid --game '{game}'. Valid: {', '.join(sorted(GAME_PROFILES))}"
        )
        sys.exit(1)

    esm_yaml_subdir = GAME_ESM_YAML_DIR.get(game, f"{game}_esm_yaml")
    db_path = args.db_path
    esm_yaml_dir = args.esm_yaml_dir or str(Path(db_path).parent / esm_yaml_subdir)

    print(f"Building records database [{game}] from: {esm_yaml_dir}")
    print(f"Database output: {db_path}")
    print()

    if args.incremental:
        sources = [s.strip() for s in args.sources.split(",") if s.strip()]
        delete_sources = [s.strip() for s in args.delete_sources.split(",") if s.strip()]
        if not sources:
            print("ERROR: --incremental requires --sources.")
            sys.exit(1)
        print(f"  Mode: incremental — sources: {sources}")
        if delete_sources:
            print(f"  Deleting stale records for: {delete_sources}")
        incremental_update(esm_yaml_dir, db_path, sources, delete_sources)
    else:
        total, type_counts, source_counts = build_db(
            esm_yaml_dir, db_path, args.embeddings
        )
        print("\nDatabase built successfully!")
        print(f"  Total records: {total:,}")
        print("\n  Records by source:")
        for src in sorted(source_counts, key=source_counts.get, reverse=True):
            print(f"    {src}: {source_counts[src]:,}")
        print("\n  Top record types:")
        for rt in sorted(type_counts, key=type_counts.get, reverse=True)[:20]:
            print(f"    {rt}: {type_counts[rt]:,}")


if __name__ == "__main__":
    main()
