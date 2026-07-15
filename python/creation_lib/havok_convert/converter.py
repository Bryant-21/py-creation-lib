"""HavokConverter — thin wrapper over creation_lib._native.havok_native conversion."""
from __future__ import annotations

from pathlib import Path
from typing import Callable

from .native_runtime import (
    havok_convert_bytes_native,
    havok_convert_file_native,
    havok_convert_batch_native,
)


class HavokConverter:
    """Convert HKX files between Havok versions via the native Rust backend."""

    def __init__(self, extracted_dir: Path | None = None):
        # extracted_dir retained for backward-compatible callers; unused because
        # the Rust backend has the FO4 template baked in.
        self.extracted_dir = extracted_dir

    def convert_bytes(self, data: bytes, target_version: int) -> bytes:
        """Convert an HKX blob in memory."""
        return havok_convert_bytes_native(data, target_version)

    def convert_file(self, src_path: str, dst_path: str, target_version: int) -> None:
        """Convert an HKX file on disk."""
        havok_convert_file_native(src_path, dst_path, target_version)

    def convert_batch(
        self,
        src_dir: str,
        dst_dir: str,
        target_version: int,
        preserve_structure: bool = True,
        on_progress: Callable[[int, int, str], None] | None = None,
    ) -> dict:
        """Convert all HKX files in a directory.

        ``on_progress`` is accepted for API compatibility but is not called
        (the Rust backend does not surface per-file callbacks).
        """
        return havok_convert_batch_native(src_dir, dst_dir, target_version)
