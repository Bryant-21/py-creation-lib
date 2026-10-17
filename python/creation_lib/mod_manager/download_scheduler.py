from dataclasses import dataclass
import threading

from .backend import call


@dataclass
class Transfer:
    identity: tuple
    future: object
    cancel: threading.Event
    identifier: str = ""
    progress: tuple = (None, "Queued")


class Scheduler:
    def __init__(self, library, slots=2, observer=None):
        call("scheduler_init", self, library, slots, observer)

    @property
    def active(self):
        return bool(self.transfers)

    def has(self, identity_or_id):
        return call("scheduler_has", self, identity_or_id)

    def add(self, client, link=None, identifier=""):
        return call("scheduler_add", self, client, link, identifier)

    def pause(self, identity_or_id=None):
        call("scheduler_pause", self, identity_or_id)

    def poll(self):
        return call("scheduler_poll", self)

    def close(self):
        call("scheduler_close", self)
