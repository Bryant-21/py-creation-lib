"""Bounded read-only source observation; persisted notices never import fields."""
from dataclasses import dataclass
from .backend import call

FILE_LIMIT = 1024**2
BATCH_BYTES = 4 * 1024**2
BATCH_FILES = 128
NOTICE_LIMIT = 200
TARGET_LIMIT = 100_000


@dataclass(frozen=True)
class Target:
    owner_id: str
    kind: str
    name: str
    path: str
    source: str


@dataclass(frozen=True)
class Notice:
    owner_id: str
    kind: str
    name: str
    path: str
    change: str
    observed: str
    issue: str = ""


@dataclass(frozen=True)
class Index:
    targets: tuple
    cursor: int
    visited: int
    notices: tuple
    count: int
    changed_paths: tuple


def targets(library):
    return call("observer_targets", library)


def fingerprint(target, cancel=None):
    return call("observer_fingerprint", target, cancel)


def scan(library, watched=None, cursor=0, visited=0, cancel=None, selected=""):
    return call("observer_scan", library, watched, cursor, visited, cancel, selected, BATCH_FILES)


def dismiss(library, notice):
    return call("observer_dismiss", library, notice)


class Observer:
    def __init__(self):
        call("observer_initialize", self)

    def invalidate(self):
        return call("observer_invalidate", self)

    def poll(self, manager):
        return call("observer_poll", self, manager)

    def close(self):
        return call("observer_close", self)
