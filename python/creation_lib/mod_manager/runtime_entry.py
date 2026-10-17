"""Frozen helper dispatch before any GUI or workspace import."""
from .backend import call
FLAG = "--modbox21-runtime-helper"


def dispatch(arguments):
    return call("runtime_dispatch", arguments)
