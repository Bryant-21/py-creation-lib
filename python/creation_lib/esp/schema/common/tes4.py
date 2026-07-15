"""TES4 plugin header record — shared across all games.

Per-game modules may override `header_version` and extend subrecords.
"""
from __future__ import annotations

from ..base import RecordSpec, SubrecordSpec
from ..kinds import FieldKind


TES4: RecordSpec = RecordSpec(
    sig="TES4",
    subrecords=(
        SubrecordSpec(sig="HEDR", kind=FieldKind.PARSED, codec="struct:f,i,i",
                      required=True, notes="version, num_records, next_object_id"),
        SubrecordSpec(sig="CNAM", kind=FieldKind.PARSED, codec="zstring",
                      notes="Author"),
        SubrecordSpec(sig="SNAM", kind=FieldKind.PARSED, codec="zstring",
                      notes="Description"),
        SubrecordSpec(sig="MAST", kind=FieldKind.PARSED, codec="zstring",
                      repeatable=True, notes="Master name; paired with DATA"),
        SubrecordSpec(sig="DATA", kind=FieldKind.PARSED, codec="int64",
                      repeatable=True, notes="Master size; paired with MAST"),
        SubrecordSpec(sig="ONAM", kind=FieldKind.PARSED, codec="formid",
                      repeatable=True, notes="Overridden form IDs"),
        SubrecordSpec(sig="INTV", kind=FieldKind.PARSED, codec="uint32",
                      notes="Internal version"),
        SubrecordSpec(sig="INCC", kind=FieldKind.PARSED, codec="uint32",
                      notes="Incremental counter"),
    ),
    order_hint=("HEDR", "CNAM", "SNAM", "MAST", "DATA", "ONAM", "INTV", "INCC"),
)
