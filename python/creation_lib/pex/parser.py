"""Native PEX binary parser facade.

Reads compiled Papyrus .pex files into typed dataclasses through
``creation_lib._native.papyrus_core``.
"""
from __future__ import annotations

from pathlib import Path

from creation_lib.pex import native_runtime
from creation_lib.pex.types import PexFile


def parse_pex_bytes(data: bytes) -> PexFile:
    """Parse PEX binary data into a PexFile structure."""
    return native_runtime.parse_pex_bytes_native(data)


def parse_pex(path: Path) -> PexFile:
    """Parse a .pex file from disk."""
    return native_runtime.parse_pex_file_native(path)
