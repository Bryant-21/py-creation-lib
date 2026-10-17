"""Frozen archive/destination review; importing downloads never copies their payloads."""
from dataclasses import dataclass
from functools import cached_property

from .backend import call


@dataclass(frozen=True)
class Candidate:
    id: str
    name: str
    source: str
    version: str
    file_id: int
    eligible: bool
    published: bool
    reason: str
    overlay_eligible: bool = False


@dataclass(frozen=True)
class DownloadReview:
    library_id: str
    profile_id: str
    target_id: str
    path: str
    signature: tuple
    imported: bool
    record_token: str
    context_token: str
    nexus: tuple
    candidates: tuple[Candidate, ...]

    def metadata(self):
        return dict(self.nexus)

    @cached_property
    def by_id(self):
        return {item.id: item for item in self.candidates}


def capture(library, profile_id, path, imported=False, states=None, cancel=None):
    return call("download_review_capture", library, profile_id, path, imported, states, cancel)


def validate(library, profile_id, review, cancel=None):
    return call("download_review_validate", library, profile_id, review, cancel)


def destination(review, mod_id, *, overlay=False):
    return call("download_review_destination", review, mod_id, overlay)
