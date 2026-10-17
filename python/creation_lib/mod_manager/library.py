from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path

from .backend import call, NativeContext
from .mo2 import ImportPreview
from .package import Package


@dataclass(frozen=True)
class Profile:
    id: str
    name: str
    source: str
    files: dict[str, str]


@dataclass(frozen=True)
class ModRow:
    id: str
    name: str
    source: str
    kind: str
    version: str
    category: str
    notes: str
    tags: str
    priority: int
    enabled: bool
    group: str
    nexus_game: str = ""
    nexus_mod_id: int = 0
    nexus_file_id: int = 0
    categories: tuple[str, ...] = ()


@dataclass
class ExportPlan:
    profile_id: str
    profile_name: str
    files: dict[str, tuple[str, str, int, int]]
    conflicts: dict[str, list[str]]


class Library:
    MARKER = ".modbox21-library.json"

    def __init__(self, root: str | Path, *, create=False):
        call("library_initialize", self, root, create)

    def _db(self):
        return NativeContext(call("LibraryDatabase", self))

    def profiles(self):
        with self._db() as db:
            return [Profile(**row) for row in json.loads(call("library_records", db.native, "profiles"))]

    def rows(self, profile_id):
        with self._db() as db:
            return [ModRow(**{**row, "categories": tuple(row["categories"])}) for row in json.loads(call("library_rows", db.native, profile_id))]

    def unmanaged_names(self, profile_id):
        with self._db() as db:
            return call("library_unmanaged_names", db.native, profile_id)

    def profile_snapshot(self, profile_id, *, include_signature=False):
        with self._db() as db:
            profile, rows, configuration, *signature = json.loads(call("library_profile_snapshot", db.native, profile_id, include_signature))
            return (Profile(**profile), [ModRow(**{**row, "categories": tuple(row["categories"])}) for row in rows], configuration, *signature)

    def import_mo2(self, preview: ImportPreview, selected=None, cancel=None, progress=None, *, reconcile=False, checkpoint=None):
        from dataclasses import asdict
        with self._db() as db:
            imported, repaired = call("library_import_mo2", db.native, self.root, json.dumps(asdict(preview)),
                                      None if selected is None else list(selected), reconcile, cancel, progress, checkpoint)
        self.last_import_repaired = repaired
        return imported

    def migration_runs(self):
        with self._db() as db:
            return json.loads(call("library_records", db.native, "migrations"))

    def save_migration_run(self, source, state, report):
        with self._db() as db:
            call("library_save_migration", db.native, source, state, json.dumps(report))

    def create_profile(self, name, clone="", *, template="", cancel=None):
        return call("library_create_profile_guarded", self, name, clone, template, cancel)

    @staticmethod
    def _profile_create_checkpoint():
        """Disposable commit/cancel seam after logical and target metadata have been staged."""

    def deleted_profiles(self):
        with self._db() as db:
            return json.loads(call("library_records", db.native, "deleted_profiles"))

    def delete_profile(self, profile_id, cancel=None):
        with self._db() as db:
            return call("library_delete_profile", db.native, profile_id, cancel)

    def restore_profile(self, profile_id, cancel=None):
        with self._db() as db:
            return call("library_restore_profile", db.native, profile_id, cancel)

    def customize(self, profile_id, mod_id, *, enabled=None, group=None, tags=None, notes=None):
        with self._db() as db:
            call("library_customize", db.native, profile_id, mod_id, enabled, group, tags, notes)

    def reorder(self, profile_id, mod_id, priority):
        with self._db() as db:
            call("library_reorder", db.native, profile_id, mod_id, priority)

    def _copy_plan(self, sources, destination, cancel=None, progress=None, *, byte_progress=False):
        return json.loads(call("library_copy_plan", sources, destination, cancel, progress, byte_progress))

    def _stage(self):
        return Path(call("library_stage", self.root))

    def _discard(self, path):
        call("library_discard", self.root, path)

    def install(self, package: Package, plan: dict[str, str], name: str, profile_id: str, cancel=None, progress=None, *, enabled=True, nexus=None, validate_before_commit=None):
        return call("library_mutation", self, "install", (package, plan, name, profile_id, cancel, progress), dict(enabled=enabled, nexus=nexus, validate_before_commit=validate_before_commit))

    def review_owned(self, mod_id, cancel=None):
        return self.owned.review(mod_id, cancel)

    def reinstall(self, review, package, plan, cancel=None, progress=None, *, nexus=None, restore_directories=(), validate_before_commit=None):
        return call("library_mutation", self, "reinstall", (review, package, plan, cancel, progress), dict(nexus=nexus, restore_directories=restore_directories, validate_before_commit=validate_before_commit))

    def uninstall(self, review, cancel=None):
        return call("library_mutation", self, "uninstall", (review, cancel), {})

    def history(self):
        return self.owned.history()

    def review_restore(self, operation_id, cancel=None):
        return self.owned.review_restore(operation_id, cancel)

    def restore(self, operation_id, review=None, cancel=None, progress=None, *, backup_token=None):
        return call("library_mutation", self, "restore", (operation_id, review, cancel, progress), dict(backup_token=backup_token))

    def configurations(self):
        with self._db() as db:
            return json.loads(call("library_records", db.native, "configurations"))

    def executables(self, instance_id=""):
        from .launcher import Executable
        with self._db() as db:
            return [Executable(**row) for row in json.loads(call("executable_records", db.native, instance_id))]

    def save_executable(self, executable):
        from .launcher import Executable
        with self._db() as db:
            return Executable(**json.loads(call("executable_save", db.native, json.dumps(executable.record()))))

    def remove_executable(self, executable_id):
        with self._db() as db:
            call("executable_remove", db.native, executable_id)

    def origins(self):
        with self._db() as db:
            return {row["mod_id"]: row for row in json.loads(call("library_records", db.native, "origins"))}

    def review_migration(self, mod_ids, cancel=None, progress=None):
        from .migration import SourceMigration
        return SourceMigration(self).review(mod_ids, cancel, progress)

    def migrate(self, review, cancel=None, progress=None):
        return call("library_mutation", self, "migrate", (review, cancel, progress), {})

    def _runtime_write_guard(self):
        return call("library_runtime_write_guard", self)

    def downloads(self):
        with self._db() as db:
            return json.loads(call("library_records", db.native, "downloads"))

    def download_nexus(self, client, link, cancel=None, progress=None):
        return call("library_download_nexus", self, client, link, cancel, progress)

    def export_plan(self, profile_id, cancel=None):
        with self._db() as db:
            record = json.loads(call("library_export_plan", db.native, profile_id, cancel))
        record["files"] = {key: tuple(value) for key, value in record["files"].items()}
        return ExportPlan(**record)

    def export(self, plan: ExportPlan, cancel=None, progress=None):
        return Path(call("library_export", self.root, json.dumps(list(plan.files.values())), cancel, progress))
