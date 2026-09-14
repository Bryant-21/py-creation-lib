"""records.content is zstd-compressed on write; readers decompress transparently."""

import os
import sqlite3
import tempfile
import textwrap


def _make_yaml(tmpdir, plugin, sig, editor_id, form_id, body):
    type_dir = os.path.join(tmpdir, plugin, "records", sig)
    os.makedirs(type_dir, exist_ok=True)
    path = os.path.join(type_dir, f"{editor_id} - {form_id}_{plugin}.yaml")
    header = textwrap.dedent(f"""\
        form_id: "{form_id}"
        version_control: 1
        form_version: 131
        eid: {editor_id}
        fields:
        """)
    with open(path, "w", encoding="utf-8") as f:
        f.write(header + body)
    return path


def test_content_column_is_compressed_blob():
    """The records.content column stores zstd frames, not plain text."""
    with tempfile.TemporaryDirectory() as tmpdir:
        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "PipePistol", "004822",
                   "- ObjectBounds: {}\n- FULL: Pipe Pistol\n- DESC: A pistol\n")

        from creation_lib.preprocessor.records import build_db

        db_path = os.path.join(tmpdir, "fo4_records.db")
        build_db(tmpdir, db_path)

        conn = sqlite3.connect(db_path)
        try:
            row = conn.execute(
                "SELECT typeof(content), content FROM records WHERE form_key = ?",
                ("004822:Fallout4.esm",),
            ).fetchone()
        finally:
            conn.close()

        assert row is not None, "PipePistol was not indexed"
        assert row[0] == "blob", f"expected BLOB, got typeof={row[0]!r}"
        # zstd frame magic: 0x28 0xB5 0x2F 0xFD (little-endian)
        assert row[1][:4] == b"\x28\xb5\x2f\xfd", \
            f"content does not start with zstd magic: {row[1][:8]!r}"


def test_load_full_yaml_returns_decompressed_text_when_yaml_path_absent():
    """If yaml_path doesn't resolve, load_full_yaml falls back to records.content
    and must decompress the BLOB."""
    with tempfile.TemporaryDirectory() as tmpdir:
        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "TestWeap", "004823",
                   "- ObjectBounds: {}\n- FULL: Test Weapon\n")

        from creation_lib.preprocessor.records import build_db
        from creation_lib.db.record_loader import RecordLoader

        db_path = os.path.join(tmpdir, "fo4_records.db")
        build_db(tmpdir, db_path)

        # Point yaml_path at something that doesn't exist so load_full_yaml
        # falls back to the DB content blob.
        conn = sqlite3.connect(db_path)
        conn.execute("UPDATE records SET yaml_path = ? WHERE form_key = ?",
                     ("/nonexistent/path.yaml", "004823:Fallout4.esm"))
        conn.commit()
        conn.close()

        loader = RecordLoader(db_path)
        try:
            text = loader.load_full_yaml("004823:Fallout4.esm")
        finally:
            loader.close()

        assert text is not None, "load_full_yaml returned None"
        assert isinstance(text, str), f"expected str, got {type(text).__name__}"
        assert "TestWeap" in text or "Test Weapon" in text, \
            f"decompressed text missing fixture markers: {text[:200]!r}"


def test_fts_search_still_finds_content_terms():
    """records_fts must still match terms from the (compressed) content column."""
    with tempfile.TemporaryDirectory() as tmpdir:
        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "BigIron", "004824",
                   "- ObjectBounds: {}\n- FULL: Big Iron Revolver\n")

        from creation_lib.preprocessor.records import build_db

        db_path = os.path.join(tmpdir, "fo4_records.db")
        build_db(tmpdir, db_path)

        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        try:
            rows = conn.execute(
                "SELECT r.editor_id FROM records_fts f "
                "JOIN records r ON r.rowid = f.rowid "
                "WHERE records_fts MATCH ?",
                ("Revolver",),
            ).fetchall()
        finally:
            conn.close()

        eids = [r["editor_id"] for r in rows]
        assert "BigIron" in eids, \
            f"FTS lost 'Revolver' (a content-only term); got eids={eids}"
