"""End-to-end test: run preprocess_havok on Starfield extracted data and verify DB."""

import pytest
import sqlite3
import tempfile
from pathlib import Path


@pytest.fixture
def project_root():
    return Path(__file__).resolve().parents[4]


def test_e2e_starfield_pipeline(project_root):
    """Run pipeline on Starfield meshes and verify all 3 format types indexed."""
    meshes_dir = project_root / "extracted/starfield/meshes"
    if not meshes_dir.exists():
        pytest.skip("Starfield extracted data not available")

    from creation_lib.preprocessor.havok import build_db

    with tempfile.TemporaryDirectory() as tmpdir:
        db_path = Path(tmpdir) / "test_starfield_havok.db"
        cache_dir = Path(tmpdir) / "cache"

        build_db(
            meshes_dir=str(meshes_dir),
            db_path=db_path,
            game="starfield",
            source="starfield",
            cache_dir=str(cache_dir),
            num_workers=1,  # Single worker for test stability
        )

        conn = sqlite3.connect(str(db_path))

        # Verify skeletons (.rig files indexed)
        skel_count = conn.execute("SELECT COUNT(*) FROM havok_skeletons").fetchone()[0]
        assert skel_count > 0, "No skeletons indexed from .rig files"

        # Verify a known skeleton
        row = conn.execute(
            "SELECT bone_count, bone_names FROM havok_skeletons WHERE id LIKE '%human%skeleton'"
        ).fetchone()
        if row:  # May not be found if path differs
            assert row[0] > 80  # Human has ~97 bones

        # Verify animations (.af files indexed)
        anim_count = conn.execute("SELECT COUNT(*) FROM havok_animations").fetchone()[0]
        assert anim_count > 0, "No animations indexed from .af files"

        # Verify behaviors (.agx files indexed)
        beh_count = conn.execute("SELECT COUNT(*) FROM havok_behaviors").fetchone()[0]
        assert beh_count > 0, "No behaviors indexed from .agx files"

        # Verify FTS index has entries for all three types
        fts_types = conn.execute(
            "SELECT DISTINCT entity_type FROM havok_fts"
        ).fetchall()
        fts_type_set = {r[0] for r in fts_types}
        assert "skeleton" in fts_type_set, "No skeleton FTS entries"
        assert "animation" in fts_type_set, "No animation FTS entries"
        assert "behavior" in fts_type_set, "No behavior FTS entries"

        # Verify FTS search works
        results = conn.execute(
            "SELECT id, name FROM havok_fts WHERE havok_fts MATCH 'idle' LIMIT 5"
        ).fetchall()
        # Should find at least some idle animations
        assert len(results) > 0, "FTS search for 'idle' returned no results"

        conn.close()
