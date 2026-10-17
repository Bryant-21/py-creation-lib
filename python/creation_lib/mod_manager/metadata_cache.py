"""Small UI metadata caches retain recent explicit reads, never payload copies."""
from .backend import call


def remember(cache, key, value, capacity):
    return call("metadata_remember", cache, key, value, capacity)
