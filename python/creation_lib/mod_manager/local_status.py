from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class StatusIndex:
    loose: object
    states: dict
    signature: tuple
    unavailable_mod_ids: tuple
    issues: tuple
    scope: str
    partial: bool = False
    measured: bool = True



def summarize(loose, combined=None, *, archive_error=""):
    return call("local_status_summarize", loose, combined, archive_error)


def scan(library, profile_id, cancel, progress, publish, *, resolver=None):
    return call("local_status_scan", library, profile_id, cancel, progress, publish, resolver)


def matches(index, rows):
    return call("local_status_matches", index, rows)
