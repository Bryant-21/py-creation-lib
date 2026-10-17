"""Explicit read-only folder commands; metadata is checked again at invocation."""
from .backend import call



def reveal_mod(library, profile_id, mod_id):
    return call("folder_reveal", library, profile_id, mod_id)


def reveal_library(library):
    return call("folder_reveal", library, None, None)
