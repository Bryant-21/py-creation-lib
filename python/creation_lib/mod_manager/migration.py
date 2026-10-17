from dataclasses import asdict, dataclass
import json
import shutil

from .backend import call
from .paths import Cancelled, ManagerError


MAX_BATCH_ENTRIES = 1_000_000


@dataclass(frozen=True)
class SourceReview:
    mod_id: str
    name: str
    source: str
    token: str
    manifest: dict
    directories: tuple[str, ...]
    profiles: tuple[str, ...]
    bytes: int
    allocated: int
    required: int


@dataclass(frozen=True)
class MigrationReview:
    sources: tuple[SourceReview, ...]
    bytes: int
    required: int
    free: int


class MigrationInterrupted(ManagerError):
    def __init__(self, completed, total, reason):
        self.completed = tuple(completed)
        super().__init__(f"Copy stopped after {len(completed)} / {total} sources. Completed copies remain owned. {reason}")


class MigrationCancelled(Cancelled):
    def __init__(self, completed, total):
        self.completed = tuple(completed)
        super().__init__(f"Copy cancelled after {len(completed)} / {total} sources. Completed copies remain owned; the current uncommitted copy was rolled back.")


class SourceMigration:
    def __init__(self, library):
        self.library = library

    def review(self, mod_ids, cancel=None, progress=None):
        result = json.loads(call("migration_review", self.library, list(mod_ids), MAX_BATCH_ENTRIES,
                                 shutil.disk_usage, cancel, progress))
        result["sources"] = tuple(SourceReview(**{**source,
            "directories": tuple(source["directories"]), "profiles": tuple(source["profiles"])})
            for source in result["sources"])
        return MigrationReview(**result)

    def copy(self, review, cancel=None, progress=None):
        return tuple(call("migration_copy", self.library, json.dumps(asdict(review)), shutil.disk_usage, cancel, progress))
