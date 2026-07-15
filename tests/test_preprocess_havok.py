"""Integration tests for py_creation_lib/python/creation_lib/preprocessor/havok.py — Havok asset database builder."""

import os
import sqlite3
import tempfile
from pathlib import Path

import pytest


def _make_behavior_project(base: Path, name: str):
    """Create a minimal behavior project directory structure."""
    proj_dir = base / name
    (proj_dir / "Characters").mkdir(parents=True)
    (proj_dir / "Behaviors").mkdir(parents=True)

    # Project file
    (proj_dir / f"{name}.xml").write_text(f"""\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbProjectStringData" signature="0x76ad60a">
      <hkparam name="characterFilenames" numelements="1">
        <hkcstring>Characters\\Character.hkx</hkcstring>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>""")

    # Character file
    (proj_dir / "Characters" / "Character.xml").write_text("""\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbCharacterStringData" signature="0x655b42bc">
      <hkparam name="rigName">Behaviors\\Behavior.hkx</hkparam>
      <hkparam name="behaviorFilename">Behaviors\\Behavior.hkx</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>""")

    # Behavior file
    (proj_dir / "Behaviors" / "Behavior.xml").write_text("""\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbBehaviorGraphStringData" signature="0xc713064e">
      <hkparam name="eventNames" numelements="1">
        <hkcstring>TestEvent</hkcstring>
      </hkparam>
      <hkparam name="variableNames" numelements="0"></hkparam>
    </hkobject>
  </hksection>
</hkpackfile>""")


class TestPreprocessHavok:
    def test_creates_all_tables(self, tmp_path):
        from creation_lib.preprocessor.havok import create_db

        db_path = tmp_path / "test_havok.db"
        conn = create_db(db_path)
        cursor = conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
        tables = {row[0] for row in cursor.fetchall()}
        assert "havok_projects" in tables
        assert "havok_characters" in tables
        assert "havok_skeletons" in tables
        assert "havok_behaviors" in tables
        assert "havok_animations" in tables
        assert "havok_manifests" in tables
        assert "havok_manifest_files" in tables
        assert "havok_manifest_deps" in tables
        assert "havok_fts" in tables
        assert "behavior_events" in tables
        assert "behavior_variables" in tables
        assert "behavior_sequences" in tables
        assert "behavior_transitions" in tables
        conn.close()

    def test_indexes_synthetic_project(self, tmp_path):
        from creation_lib.preprocessor.havok import build_db

        meshes_dir = tmp_path / "Meshes"
        meshes_dir.mkdir()
        _make_behavior_project(meshes_dir / "UniqueBehaviors", "TestFX")

        db_path = tmp_path / "test_havok.db"
        stats = build_db(
            meshes_dir=meshes_dir,
            db_path=db_path,
            game="fo4",
            source="fo4",
        )

        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row

        # Verify project indexed
        projects = conn.execute("SELECT * FROM havok_projects").fetchall()
        assert len(projects) >= 1

        # Verify behavior indexed with events
        behaviors = conn.execute("SELECT * FROM havok_behaviors").fetchall()
        assert len(behaviors) >= 1
        events = conn.execute("SELECT * FROM behavior_events").fetchall()
        assert len(events) >= 1

        # Verify manifest created
        manifests = conn.execute("SELECT * FROM havok_manifests").fetchall()
        assert len(manifests) >= 1

        # Verify FTS works
        fts_results = conn.execute(
            "SELECT * FROM havok_fts WHERE havok_fts MATCH 'TestFX'"
        ).fetchall()
        assert len(fts_results) >= 1

        conn.close()
