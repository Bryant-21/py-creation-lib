"""Common record specs shared across Bethesda games.

Per-game modules import from here and override / extend as needed.
"""
from __future__ import annotations

from ..base import EnumDef, RecordSpec
from .tes4 import TES4

COMMON_ENUMS: dict[str, EnumDef] = {}
COMMON_RECORDS: dict[str, RecordSpec] = {
    "TES4": TES4,
}

__all__ = ["COMMON_ENUMS", "COMMON_RECORDS", "TES4"]
