"""Reviewed bounded account/download batches; no optimistic remote account state."""
from dataclasses import dataclass, field

from .backend import call


@dataclass(frozen=True)
class Item:
    id: object
    name: str
    identity: tuple
    version: str = ""
    issue: str = ""
    link: object = field(default=None, repr=False)
    file_id: int = 0


@dataclass(frozen=True)
class Review:
    kind: str
    action: str
    account: str
    context: tuple
    destination: dict
    items: tuple[Item, ...]


@dataclass(frozen=True)
class Result:
    rows: tuple[tuple[object, str], ...]
    stopped: bool = False
    retry_after: int = 0


def account_review(manager, mod_ids, action):
    return call("nexus_bulk_account", manager, mod_ids, action)


def download_review(manager):
    return call("nexus_bulk_downloads", manager)


def validate(manager, plan, selected):
    return call("nexus_bulk_validate", manager, plan, selected)


def account_run(manager, plan, items, cancel=None, progress=lambda *_: None):
    return call("nexus_bulk_run", manager, plan, items, cancel, progress)


def queue_downloads(manager, plan, selected):
    return call("nexus_bulk_queue", manager, plan, selected)
