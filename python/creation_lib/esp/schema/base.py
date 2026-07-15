"""Core schema dataclasses.

FieldSpec:     one typed field within a structured subrecord.
SubrecordSpec: one subrecord signature within a record.
RecordSpec:    one record signature and its subrecord specs.
GameSchema:    a full per-game schema (record signature -> RecordSpec).

All are frozen. Per-game overrides are composed in schema/games/<game>.py.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .kinds import FieldKind


@dataclass(frozen=True)
class EnumDef:
    name: str
    values: tuple[tuple[int, str], ...] = ()
    labels: tuple[tuple[int, str], ...] = ()
    aliases: tuple[tuple[str, str], ...] = ()
    scope: str = "scoped"
    storage_kind: str = "enum"
    byte_width: int = 4
    # FO4 fallback value used when a source value is out of range and must be
    # clamped (e.g. KYWD.TNAM -> 0 "None"). None means no explicit default.
    default_value: int | None = None
    notes: str = ""

    def token_for_value(self, value: int) -> str | None:
        for candidate_value, token in self.values:
            if candidate_value == value:
                return token
        return None

    def label_for_value(self, value: int) -> str | None:
        for candidate_value, label in self.labels:
            if candidate_value == value:
                return label
        token = self.token_for_value(value)
        if token is not None:
            return token
        return None

    def resolve_token(self, token: str) -> str:
        normalized = token.strip()
        for legacy, canonical in self.aliases:
            if legacy == normalized:
                return canonical
        return normalized


@dataclass(frozen=True)
class ConditionSpec:
    field: str
    operator: str = "eq"
    value: Any = None
    values: tuple[Any, ...] = ()
    notes: str = ""


@dataclass(frozen=True)
class TargetMapEntry:
    selector: str
    value: Any
    target: str
    notes: str = ""


@dataclass(frozen=True)
class ArraySpec:
    layout: str
    element_kind: str | None = None
    element_codec: str | None = None
    count_field: str | None = None
    count_codec: str | None = None
    count_transform: str | None = None
    # Cross-subrecord count source: looks up `count_record_field` in the
    # record-level context populated by previously-decoded subrecords.
    # Used for xEdit `SetCountPath('..\\<SIG>\\<Field>')` patterns where the
    # array length is stored in a sibling subrecord (e.g. FSTS.DATA arrays
    # sized by FSTS.XCNT counts). Mutually exclusive with count_field /
    # count_codec / count_transform — when set, no count prefix is consumed
    # from the array's own subrecord bytes.
    count_record_field: str | None = None
    notes: str = ""


@dataclass(frozen=True)
class UnionVariantSpec:
    name: str
    codec: str | None = None
    fields: tuple["FieldSpec", ...] = ()
    conditions: tuple[ConditionSpec, ...] = ()
    notes: str = ""


@dataclass(frozen=True)
class FieldSpec:
    name: str
    kind: str
    enum_ref: str | None = None
    formlink_target: str | None = None
    # Full ordered set of allowed FK target signatures from xEdit's
    # wbFormIDCk(...) list. NULL is stripped out and recorded as null_allowed.
    # Empty means "no FK constraint emitted" (either not a reference, or the
    # target set was unparseable). formlink_target stays the single-target
    # convenience for authoring labels; formlink_targets is authoritative.
    formlink_targets: tuple[str, ...] = ()
    null_allowed: bool = False
    target_map: tuple[TargetMapEntry, ...] = ()
    presence_conditions: tuple[ConditionSpec, ...] = ()
    nested_fields: tuple["FieldSpec", ...] = ()
    union_selector: str | None = None
    union_variants: tuple[UnionVariantSpec, ...] = ()
    array: ArraySpec | None = None
    authoring_label: str | None = None
    default_value: Any = None
    notes: str = ""


@dataclass(frozen=True)
class SubrecordSpec:
    sig: str
    kind: FieldKind
    display_label: str | None = None
    codec: str | None = None
    fields: tuple[FieldSpec, ...] | None = None
    repeatable: bool = False
    required: bool = False
    localized: bool = False
    enum_ref: str | None = None
    formlink_target: str | None = None
    formlink_targets: tuple[str, ...] = ()
    null_allowed: bool = False
    enum: tuple[tuple[int, str], ...] | None = None
    flags: tuple[tuple[int, str], ...] | None = None
    struct_layout: tuple | None = None
    presence_conditions: tuple[ConditionSpec, ...] = ()
    union_selector: str | None = None
    union_variants: tuple[UnionVariantSpec, ...] = ()
    array: ArraySpec | None = None
    row_label: str | None = None
    authoring_layout: str | None = None
    authoring_key: str | None = None
    scope_id: str | None = None
    notes: str = ""


@dataclass(frozen=True)
class RecordFlagBit:
    bit: int
    name: str


@dataclass(frozen=True)
class RecordFlagsSpec:
    # OR of valid header-flag bit values, incl. the universal mask. A set bit
    # outside this mask is an unknown flag (unless permissive).
    valid_mask: int = 0
    # xEdit wbFlagsList(..., aUnknowns=True) — every bit is accepted, so no
    # unknown-bit error is ever raised for this record's header flags.
    permissive: bool = False
    # Named valid bits, for diagnostics/labels (not required for masking).
    bits: tuple[RecordFlagBit, ...] = ()


@dataclass(frozen=True)
class RecordSpec:
    sig: str
    subrecords: tuple[SubrecordSpec, ...]
    display_label: str | None = None
    order_hint: tuple[str, ...] = ()
    header_flags: int = 0
    record_flags: RecordFlagsSpec | None = None
    notes: str = ""


@dataclass(frozen=True)
class GameSchema:
    game: str
    records: dict[str, RecordSpec]
    header_version: float
    localized_support: bool
    enums: dict[str, EnumDef] = field(default_factory=dict)
