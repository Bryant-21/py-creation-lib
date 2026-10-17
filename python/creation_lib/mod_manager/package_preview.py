"""Bounded inert previews from an already inspected installer package."""
from dataclasses import dataclass

from .backend import call


@dataclass(frozen=True)
class Preview:
    source: str
    bytes: int
    kind: str
    text: str = ""
    pixels: object = None
    truncated: bool = False


def read(package, source, cancel=None):
    return call("package_preview", package, source, cancel)
