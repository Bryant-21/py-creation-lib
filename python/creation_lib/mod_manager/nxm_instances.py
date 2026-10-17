from dataclasses import dataclass, field
from .backend import call


EMPTY = {"library_id": "", "library_root": "", "profile_id": "", "profile_name": "", "target_id": "", "target_name": "", "target_root": "", "game": "", "ready": False}
_pending_startup = None


@dataclass(frozen=True)
class Destination:
    instance_id: str
    address: str
    process: dict
    context: dict

    @property
    def label(self):
        return call("nxm_destination_label", self)


@dataclass(frozen=True)
class Request:
    id: str
    link: object = field(repr=False)
    destination: Destination | None = None
    uncertain: bool = False

    @property
    def identity(self):
        return self.link.identity

    @property
    def key(self):
        return self.link.key


def discover(directory, game=""):
    return call("nxm_discover", directory, game)


class Inbox:
    def __init__(self, directory, *, family=None, context=None):
        call("nxm_inbox_initialize", self, directory, family, context)

    @property
    def destination(self):
        return call("nxm_inbox_destination", self)

    def publish(self, context, *, force=False):
        return call("nxm_inbox_publish", self, context, force)

    def mark(self, request, state):
        return call("nxm_inbox_mark", self, request, state)

    def _dispatch(self, data):
        return call("nxm_inbox_dispatch", self, data)

    def _run(self):
        return call("nxm_inbox_run", self)

    def close(self):
        return call("nxm_inbox_close", self)


def send(destination, request, *, status_only=False):
    return call("nxm_send", destination, request, status_only)


def forward_link(directory, raw):
    return call("nxm_forward_link", directory, raw)


def take_startup(link):
    return call("nxm_take_startup", link)
