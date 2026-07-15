"""FieldKind — declares how a subrecord's bytes are handled at runtime."""
from __future__ import annotations
from enum import Enum


class FieldKind(Enum):
    """How a subrecord spec tells the runtime to treat its bytes.

    RAW:                       store bytes as-is; writer emits unchanged.
    PARSED:                    codec decodes to structured data; byte-exact on re-encode.
    PARSED_WITH_RAW_FALLBACK:  try codec; on failure keep raw bytes, log WARNING.
    CUSTOM_CODEC:              delegate parse/write to a named external Rust module
                               (e.g. ``esp_authoring_core::nvnm``). Until the codec
                               is wired through, bytes are preserved raw; the
                               runtime treats it as raw_only.
    """

    RAW = "raw"
    PARSED = "parsed"
    PARSED_WITH_RAW_FALLBACK = "parsed_with_raw_fallback"
    CUSTOM_CODEC = "custom_codec"
