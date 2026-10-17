"""Frozen explicit account request review; this module never performs network writes."""
from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Review:
    mod_id: str
    action: str
    name: str
    game: str
    nexus_mod_id: int
    file_id: int
    version: str
    account_name: str
    context: tuple


def review(manager, mod_id, action):
    return call("nexus_review", manager, mod_id, action)


def validate(manager, plan):
    return call("nexus_review_validate", manager, plan)
