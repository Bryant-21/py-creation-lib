from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from .backend import NativeContext, call, invoke


@dataclass(frozen=True)
class Limits:
    files: int = 100_000
    total_bytes: int = 8 * 1024**3
    file_bytes: int = 2 * 1024**3
    ratio: int = 2000


# Already installed ordinary folders are inert payloads, including large BA2s.
# Archive installs and staged game exports retain their smaller default bounds.
OWNED_LIMITS = Limits(files=500_000, total_bytes=256 * 1024**3, file_bytes=64 * 1024**3)


def installer_metadata(name):
    return call("installer_metadata", name)


def validate_intake(path):
    return call("manager_validate_intake", path)


class Package:
    def __init__(self, path: str | Path, cancel=None, limits=Limits(), *, inspect_installer=True):
        call("package_initialize", self, path, cancel, limits, inspect_installer)

    def open(self, source: str):
        return NativeContext(call("package_open", self, source))

    @staticmethod
    def _signature(path):
        return call("package_signature", path)

    def read_config(self):
        return call("package_read_config", self)

    def validate_sources(self, sources, cancel=None):
        return invoke(self._index.validate_sources, list(sources), cancel)

    def plain_plan(self, root="", cancel=None, *, allow_empty=False):
        return invoke(self._index.plain_plan, root, cancel, allow_empty)

    def close(self):
        return call("package_close", self)
