from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Candidate:
    kind: str
    provider: object
    archive: object = None

    @property
    def name(self):
        return self.provider.name + (" | " + self.archive.name if self.archive else " | Loose")


@dataclass(frozen=True)
class Entry:
    path: str
    loose: object
    archives: tuple
    winner: Candidate | None
    reason: str

    @property
    def candidates(self):
        return call("combined_candidates", self, "candidates")

    @property
    def active(self):
        return call("combined_candidates", self, "active")

    @property
    def providers(self):
        return call("combined_candidates", self, "providers")


@dataclass
class Index:
    profile_id: str
    files: tuple[Entry, ...]
    loose: object
    archives: object
    settings: dict
    issues: tuple[str, ...]
    tree: object = None



def resolve(path, loose, archives, settings, *, game="fo4"):
    return call("combined_resolve", path, loose, archives, settings, game)


def build(library, profile_id, cancel=None, progress=None, *, reader=None, resolver=None, loose=None):
    return call("combined_build", library, profile_id, cancel, progress, reader, resolver, loose)
