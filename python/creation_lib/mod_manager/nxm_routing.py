"""Frozen library/profile/game routing, independent of account credentials."""
from .backend import call


def context(manager):
    return call("nexus_destination", manager)


def pin(value):
    return call("nexus_destination_pin", value)


def validate(manager, expected):
    return call("nexus_destination_validate", manager, expected)


def receive(workspace, request):
    return call("nexus_receive", workspace, request)
