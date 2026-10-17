
                CREATE TABLE IF NOT EXISTS mods (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, source TEXT NOT NULL,
                    kind TEXT NOT NULL, version TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT '',
                    notes TEXT NOT NULL DEFAULT '', tags TEXT NOT NULL DEFAULT '');
                CREATE TABLE IF NOT EXISTS profiles (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, source TEXT NOT NULL DEFAULT '', files TEXT NOT NULL DEFAULT '{}');
                CREATE TABLE IF NOT EXISTS entries (
                    profile_id TEXT REFERENCES profiles(id), mod_id TEXT REFERENCES mods(id),
                    priority INTEGER NOT NULL, enabled INTEGER NOT NULL, group_name TEXT NOT NULL DEFAULT '',
                    PRIMARY KEY(profile_id, mod_id));
                CREATE INDEX IF NOT EXISTS entries_order ON entries(profile_id, priority);
                CREATE TABLE IF NOT EXISTS instances (id TEXT PRIMARY KEY, source TEXT NOT NULL, configuration TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS downloads (game TEXT, mod_id INTEGER, file_id INTEGER, path TEXT NOT NULL,
                    version TEXT NOT NULL, checksum TEXT NOT NULL, PRIMARY KEY(game,mod_id,file_id));
                CREATE TABLE IF NOT EXISTS download_jobs (id TEXT PRIMARY KEY, game TEXT NOT NULL, mod_id INTEGER NOT NULL,
                    file_id INTEGER NOT NULL, state TEXT NOT NULL, record TEXT NOT NULL DEFAULT '{}',
                    UNIQUE(game,mod_id,file_id));
                CREATE TABLE IF NOT EXISTS operations (id TEXT PRIMARY KEY, kind TEXT NOT NULL, mod_id TEXT NOT NULL,
                    snapshot TEXT NOT NULL, created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE TABLE IF NOT EXISTS origins (mod_id TEXT PRIMARY KEY, source TEXT NOT NULL,
                    identity TEXT NOT NULL, copied TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE TABLE IF NOT EXISTS mo2_migrations (id TEXT PRIMARY KEY, source TEXT NOT NULL,
                    state TEXT NOT NULL, report TEXT NOT NULL, updated TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE TABLE IF NOT EXISTS deleted_profiles (id TEXT PRIMARY KEY, name TEXT NOT NULL,
                    snapshot TEXT NOT NULL, removed TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE TABLE IF NOT EXISTS executables (id TEXT PRIMARY KEY, instance_id TEXT NOT NULL,
                    configuration TEXT NOT NULL, removed INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS plugin_metadata (path TEXT NOT NULL, game TEXT NOT NULL, size INTEGER NOT NULL,
                    modified INTEGER NOT NULL, metadata TEXT NOT NULL, PRIMARY KEY(path,game));
                CREATE TABLE IF NOT EXISTS archive_metadata (path TEXT PRIMARY KEY, size INTEGER NOT NULL,
                    modified INTEGER NOT NULL, metadata TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS profile_backups (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL,
                    snapshot TEXT NOT NULL, created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE INDEX IF NOT EXISTS profile_backups_profile ON profile_backups(profile_id);
                CREATE TABLE IF NOT EXISTS categories (name TEXT PRIMARY KEY COLLATE NOCASE, nexus_id INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS nexus_category_catalog (game TEXT PRIMARY KEY, data TEXT NOT NULL, fetched INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS nexus_update_schedule (scope TEXT PRIMARY KEY, state TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS nexus_update_limits (account INTEGER PRIMARY KEY, blocked REAL NOT NULL);
                CREATE TABLE IF NOT EXISTS nexus_category_mapping (game TEXT NOT NULL, category_id INTEGER NOT NULL,
                    local_name TEXT NOT NULL, PRIMARY KEY(game,category_id));
                CREATE TABLE IF NOT EXISTS organization_history (sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT UNIQUE NOT NULL,
                    profile_id TEXT NOT NULL, kind TEXT NOT NULL, snapshot TEXT NOT NULL, expected TEXT NOT NULL,
                    created TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
                CREATE TABLE IF NOT EXISTS reference_exclusions (profile_id TEXT NOT NULL, mod_id TEXT NOT NULL,
                    PRIMARY KEY(profile_id,mod_id));
                CREATE TABLE IF NOT EXISTS backup_prunes (id TEXT PRIMARY KEY, token TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS reference_overlays (
                    overlay_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,
                    profile_id TEXT NOT NULL, target_id TEXT NOT NULL, base_id TEXT NOT NULL, source TEXT NOT NULL);
