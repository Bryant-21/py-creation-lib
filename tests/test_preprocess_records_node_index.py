"""End-to-end test that ADDN records get node_index populated by the native indexer."""

import os
import sqlite3
import tempfile
import textwrap


def _make_yaml(tmpdir, plugin, sig, editor_id, form_id, body: str) -> str:
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
        f.write(header + body)
    return path


def test_addn_node_index_extracted():
    """ADDN records should have node_index populated; non-ADDN should be NULL."""
    with tempfile.TemporaryDirectory() as tmpdir:
        _make_yaml(tmpdir, "Fallout4.esm", "ADDN", "MPSFireMed01", "01F23F",
                   "- ObjectBounds: {}\n- Index: 5\n")
        _make_yaml(tmpdir, "Fallout4.esm", "ADDN", "MPSMuzzleFlash", "03ADFA",
                   "- ObjectBounds: {}\n- Index: 27\n")
        _make_yaml(tmpdir, "Fallout4.esm", "WEAP", "PipePistol", "004822",
                   "- ObjectBounds: {}\n")

        from creation_lib.preprocessor.records import build_db

        db_path = os.path.join(tmpdir, "fo4_records.db")
        build_db(tmpdir, db_path)

        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        try:
            row = conn.execute(
                "SELECT node_index FROM records WHERE form_key = ?",
                ("01F23F:Fallout4.esm",),
            ).fetchone()
            assert row is not None
            assert row["node_index"] == 5

            row2 = conn.execute(
                "SELECT node_index FROM records WHERE form_key = ?",
                ("03ADFA:Fallout4.esm",),
            ).fetchone()
            assert row2 is not None
            assert row2["node_index"] == 27

            row3 = conn.execute(
                "SELECT node_index FROM records WHERE form_key = ?",
                ("004822:Fallout4.esm",),
            ).fetchone()
            assert row3 is not None
            assert row3["node_index"] is None

            indexes = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_records_node_index'"
            ).fetchone()
            assert indexes is not None
        finally:
            conn.close()


def test_addn_lookup_by_node_index():
    """Records can be queried by node_index after the build."""
    with tempfile.TemporaryDirectory() as tmpdir:
        _make_yaml(tmpdir, "Fallout4.esm", "ADDN", "MPSFireMed01", "01F23F",
                   "- Index: 5\n")

        from creation_lib.preprocessor.records import build_db

        db_path = os.path.join(tmpdir, "fo4_records.db")
        build_db(tmpdir, db_path)

        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        try:
            row = conn.execute(
                "SELECT * FROM records WHERE record_type = 'ADDN' AND node_index = ?",
                (5,),
            ).fetchone()
            assert row is not None
            assert row["editor_id"] == "MPSFireMed01"
        finally:
            conn.close()
