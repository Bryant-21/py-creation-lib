from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Review:
    library_id: str
    profile_id: str
    original: str
    affected: int
    expected: str


def review(library, profile, original, cancel=None):
    with library._db() as db:
        count, expected = call("group_review", db.native, profile, original, cancel)
        return Review(library.library_id, profile, original, count, expected)


def _checkpoint():
    pass


def apply(library, plan, replacement, cancel=None):
    library._runtime_write_guard()
    with library._db() as db:
        call("group_apply", db.native, plan.profile_id, plan.original, replacement, plan.expected,
             library.library_id, plan.library_id, cancel, _checkpoint)
        library._runtime_write_guard()
