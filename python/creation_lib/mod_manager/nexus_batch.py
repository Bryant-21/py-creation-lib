"""Read-only bounded metadata batches. Unknown endpoints never become negative facts."""
from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Batch:
    states: dict
    versions: dict
    issues: tuple[str, ...]
    retry_after: int = 60


def read(client, identities, cancel=None, progress=lambda *_: None, *, installed_files=None):
    return call("nexus_batch_read", client, identities, cancel, progress, installed_files)
