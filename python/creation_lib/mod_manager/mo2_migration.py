"""Persistent MO2 metadata import; source payloads remain in their original folders."""
from dataclasses import dataclass, field

from .library import Library
from .paths import Cancelled, ManagerError
from .backend import call


EXCLUSIONS = ("Unmanaged game-directory content", "Download archives, caches, credentials and MO2 plugins",
              "MO2 overwrite output (create a named source mod if needed)",
              "Profile saves, active INI/plugin configuration, VFS and game launch")


@dataclass
class MigrationResult:
    library: Library | None = None
    preview: object = None
    state: str = "discovering"
    new_profiles: int = 0
    original_profiles: int = 0
    referenced: list = field(default_factory=list)
    unavailable: list = field(default_factory=list)
    game_content: list = field(default_factory=list)
    retired_profiles: list = field(default_factory=list)
    issues: list = field(default_factory=list)
    auto_links: list = field(default_factory=list)
    downloads: int = 0

    def report(self):
        return {name: getattr(self, name) for name in ("state", "new_profiles", "original_profiles", "referenced",
                "unavailable", "game_content", "retired_profiles", "issues", "auto_links", "downloads")} | {
                    "exclusions": list(EXCLUSIONS), "payload_mode": "in-place references; no copying or hashing"}


class MigrationFailure(ManagerError):
    def __init__(self, result, cause):
        self.result = result
        super().__init__(f"Metadata migration {result.state}: {cause}. Retry is safe; MO2 profiles and mod folders remain unchanged.")


class MigrationStopped(Cancelled):
    def __init__(self, result):
        self.result = result
        super().__init__("Metadata migration cancelled. Uncommitted profile changes rolled back; MO2 source folders remain unchanged.")


def discover(path, destination, cancel=None, source_mappings=None):
    return call("mo2_migration_discover", path, destination, cancel, source_mappings)


def augment(preview, cancel=None):
    return call("mo2_migration_augment", preview, cancel)


def migrate(path, destination, validate, prepare, cancel, progress, source_mappings=None, checkpoint=None):
    return call("mo2_migrate", path, destination, validate, prepare, cancel, progress, source_mappings, checkpoint)
