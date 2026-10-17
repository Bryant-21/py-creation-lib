from dataclasses import dataclass
from .backend import call


@dataclass(frozen=True)
class Badge:
    text: str
    icon: str
    description: str
    role: str = "muted"



def file_states(index):
    return call("status_file_states", index)


def conflict_state(row, index, states):
    return call("status_conflict", row, index, states)


def badges(row, index, states, updates, nexus_states=None, scan_state=""):
    return call("status_badges", row, index, states, updates, nexus_states, scan_state)
