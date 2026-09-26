"""Tests for preprocess_records.incremental_update."""

import os
import shutil
import sqlite3
import tempfile
import textwrap


def _make_yaml(tmpdir, plugin, sig, editor_id, form_id, body: str = "") -> str:
    type_dir = os.path.join(tmpdir, plugin, "records", sig)
    os.makedirs(type_dir, exist_ok=True)
    fname = f"{editor_id} - {form_id}_{plugin}.yaml"
    path = os.path.join(type_dir, fname)
    header = textwrap.dedent(f"""\
        form_id: "{form_id}"
        version_control: 1
        form_version: 131
        eid: {editor_id}
        fields:
        """)
    with open(path, "w", encoding="utf-8") as f:
        f.write(header + (body or "- ObjectBounds: {}\n"))
    return path


def _form_keys(db_path: str) -> list[str]:
    conn = sqlite3.connect(db_path)
    try:
        return [r[0] for r in conn.execute("SELECT form_key FROM records").fetchall()]
    finally:
        conn.close()


def test_incremental_adds_replaces_and_stays_fts_queryable():
    """incremental_update adds records from a new source without touching
    existing ones, replaces a modified source's records after a delete, and
    the added records remain searchable via FTS."""
    with tempfile.TemporaryDirectory() as tmpdir:
        import creation_lib.preprocessor.records as pr

        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "OldWeapon", "000001")
        db_path = os.path.join(tmpdir, "test_records.db")
        pr.build_db(tmpdir, db_path)

        # Add a brand-new source.
        _make_yaml(tmpdir, "NewDLC.esm", "WEAP", "PlasmaCaster", "009999")
        pr.incremental_update(tmpdir, db_path, sources=["NewDLC.esm"])
        fks = _form_keys(db_path)
        assert "000001:Fallout4.esm" in fks
        assert "009999:NewDLC.esm" in fks

        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        try:
            rows = conn.execute(
                "SELECT r.form_key FROM records r JOIN records_fts f ON r.rowid = f.rowid "
                "WHERE records_fts MATCH 'PlasmaCaster'"
            ).fetchall()
        finally:
            conn.close()
        assert any(r["form_key"] == "009999:NewDLC.esm" for r in rows)

        # Replace the modified source: old record gone, new one present.
        shutil.rmtree(os.path.join(tmpdir, "Fallout4.esm"))
        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "NewWeapon", "000002")
        pr.incremental_update(
            tmpdir, db_path, sources=["Fallout4.esm"], delete_sources=["Fallout4.esm"],
        )
        fks = _form_keys(db_path)
        assert "000001:Fallout4.esm" not in fks
        assert "000002:Fallout4.esm" in fks


def test_incremental_fallback_when_no_db():
    """incremental_update falls back to a full build (and returns a count) if
    the DB doesn't exist yet."""
    with tempfile.TemporaryDirectory() as tmpdir:
        import creation_lib.preprocessor.records as pr

        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "PipePistol", "004822")
        db_path = os.path.join(tmpdir, "nonexistent_records.db")

        result = pr.incremental_update(tmpdir, db_path, sources=["Fallout4.esm"])

        assert result is not None and result > 0
        assert os.path.exists(db_path)
        conn = sqlite3.connect(db_path)
        try:
            row = conn.execute(
                "SELECT form_key FROM records WHERE form_key = ?",
                ("004822:Fallout4.esm",),
            ).fetchone()
        finally:
            conn.close()
        assert row is not None
