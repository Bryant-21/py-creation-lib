from dataclasses import dataclass
from .backend import call


def remap(plan, changes, cancel=None, *, allow_empty=False):
    return call("installation_remap", plan, changes, cancel, allow_empty)


@dataclass(frozen=True)
class Source:
    path: str


def source_tree(package, cancel=None):
    return call("installation_source_tree", package, cancel)


def include_sources(package, plan, additions, cancel=None):
    return call("installation_include_sources", package, plan, additions, cancel)


class CombinedPackage:
    def __init__(self, existing, incoming, plan):
        call("installation_combined_init", self, existing, incoming, plan)

    def open(self, source):
        return call("installation_combined_open", self, source)


def merge(library, review, incoming, plan, cancel=None, progress=None, *, nexus=None, validate_before_commit=None):
    return call("installation_merge", library, review, incoming, plan, cancel, progress, nexus, validate_before_commit, False)


def merge_preview(library, review, incoming, plan, cancel=None):
    return call("installation_merge", library, review, incoming, plan, cancel, None, None, None, True)
