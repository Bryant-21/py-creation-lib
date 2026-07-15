"""Tests for GameDataStore multi-game domain resolution."""
import os
import sqlite3
import tempfile
from pathlib import Path

from creation_lib.db.store import GameDataStore


def _create_dummy_db(path: str, table: str = "records", fts_table: str = "records_fts"):
    """Create a minimal SQLite DB with a table and FTS index."""
    conn = sqlite3.connect(path)
    conn.execute(f"CREATE TABLE {table} (form_key TEXT PRIMARY KEY, content TEXT)")
    conn.execute(f"CREATE VIRTUAL TABLE {fts_table} USING fts5(content, content={table}, content_rowid=rowid)")
    conn.execute(f"INSERT INTO {table} VALUES ('test:Test.esm', 'hello world')")
    conn.commit()
    conn.close()


class TestGameDataStore:
    def setup_method(self):
        self.tmpdir = tempfile.mkdtemp()

    def test_wiki_domain_resolves(self):
        """The merged 'wiki' domain should resolve to {game}_wiki.db."""
        db_path = os.path.join(self.tmpdir, "fo4_wiki.db")
        _create_dummy_db(db_path, table="pages", fts_table="pages_fts")
        store = GameDataStore(db_dir=self.tmpdir, game="fo4")
        assert store.is_available("wiki")

    def test_papyrus_wiki_domain_removed(self):
        """Old papyrus_wiki domain should no longer be valid."""
        store = GameDataStore(db_dir=self.tmpdir, game="fo4")
        assert not store.is_available("papyrus_wiki")

    def test_ck_wiki_domain_removed(self):
        """Old ck_wiki domain should no longer be valid."""
        store = GameDataStore(db_dir=self.tmpdir, game="fo4")
        assert not store.is_available("ck_wiki")

    def test_all_domains_use_game_prefix(self):
        """Every domain should resolve to {game}_{domain}.db or {game}_external_mods.db."""
        store = GameDataStore(db_dir=self.tmpdir, game="skyrimse")
        for domain in ("records", "scripts", "wiki", "behaviors", "nifs"):
            path = store._db_path(domain)
            assert "skyrimse_" in path, f"Domain '{domain}' path missing game prefix: {path}"

    def test_ext_domains_share_external_mods_db(self):
        """ext_records and ext_scripts should both map to {game}_external_mods.db."""
        db_path = os.path.join(self.tmpdir, "fo4_external_mods.db")
        _create_dummy_db(db_path, table="ext_records", fts_table="ext_records_fts")
        store = GameDataStore(db_dir=self.tmpdir, game="fo4")
        path_records = store._db_path("ext_records")
        path_scripts = store._db_path("ext_scripts")
        assert path_records == path_scripts
        assert "fo4_external_mods.db" in path_records

    def test_different_games_different_dbs(self):
        """Same domain, different game should resolve to different DB files."""
        for game in ("fo4", "skyrimse"):
            _create_dummy_db(os.path.join(self.tmpdir, f"{game}_records.db"))
        store_fo4 = GameDataStore(db_dir=self.tmpdir, game="fo4")
        store_sse = GameDataStore(db_dir=self.tmpdir, game="skyrimse")
        assert "fo4_records.db" in store_fo4._db_path("records")
        assert "skyrimse_records.db" in store_sse._db_path("records")

    def test_fnv_wiki_uses_fo3_wiki_db(self):
        store = GameDataStore(db_dir=self.tmpdir, game="fnv")
        assert store._db_path("wiki").endswith("fo3_wiki.db")

    def test_unknown_domain_raises(self):
        """Unknown domain should raise ValueError."""
        import pytest
        store = GameDataStore(db_dir=self.tmpdir, game="fo4")
        with pytest.raises(ValueError):
            store._db_path("nonexistent")
