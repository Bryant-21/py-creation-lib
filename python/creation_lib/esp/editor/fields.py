"""Schema-driven field model for the ESP editor UI.

Decodes subrecord bytes into typed `Field` objects the UI can render with
appropriate widgets. Falls back to a raw hex view for unknown signatures
or codec failures.

The decoder consumes the rich auto-generated schema (`py_creation_lib/python/creation_lib/esp/schema/games/<game>`):

- ``codec='struct:i,B,B,B,B'``  → labelled per-member struct
- ``codec='array_struct:I,I'``  → list of labelled element rows
- ``codec='formid_array'``       → list of FormID rows
- ``codec='zstring'/'lstring'/'lenstring16'/'fixed_string:N'`` → text widgets
- ``codec='formid'``              → FORMID widget
- ``enum_ref`` / ``formlink_target`` on Subrecord OR FieldSpec → resolved at decode time

Each Field carries its own ``components`` (for STRUCT) or ``rows`` (for ARRAY) so
the renderer never falls back to "[0] [1] [2]" generic widgets when the schema
provides structured field names.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from enum import Enum
from typing import Any

from creation_lib.esp.schema import get_schema
from creation_lib.esp.schema.base import EnumDef, FieldSpec, SubrecordSpec


class FieldKind(Enum):
    BYTES = "bytes"
    STRING = "string"
    LSTRING = "lstring"  # 4-byte string id (when plugin is localized)
    INT = "int"
    FLOAT = "float"
    FORMID = "formid"
    ENUM = "enum"
    FLAGSET = "flagset"
    STRUCT = "struct"
    ARRAY = "array"
    EMPTY = "empty"


@dataclass
class Field:
    name: str
    signature: str
    kind: FieldKind
    value: Any
    raw: bytes
    spec: SubrecordSpec | None = None
    enum_options: list[tuple[int, str]] = field(default_factory=list)
    flag_options: list[tuple[int, str]] = field(default_factory=list)
    notes: str = ""
    struct_layout: str | None = None  # Python struct format string

    # Resolved schema metadata.
    enum_def: EnumDef | None = None
    formlink_target: str | None = None

    # Hierarchical decomposition.
    components: list["Field"] = field(default_factory=list)
    rows: list[list["Field"]] = field(default_factory=list)
    element_field_specs: tuple[FieldSpec, ...] = ()
    element_layout: str | None = None  # element struct format for ARRAY


# ---------------------------------------------------------------------------
# Type tables
# ---------------------------------------------------------------------------

# FieldSpec.kind / SubrecordSpec.codec → (FieldKind, Python struct fmt char) for
# scalar numeric types. Both forms appear in the generated schemas.
_NUMERIC_KIND: dict[str, tuple[FieldKind, str]] = {
    "int8":   (FieldKind.INT, "b"),
    "i8":     (FieldKind.INT, "b"),
    "uint8":  (FieldKind.INT, "B"),
    "u8":     (FieldKind.INT, "B"),
    "int16":  (FieldKind.INT, "h"),
    "i16":    (FieldKind.INT, "h"),
    "uint16": (FieldKind.INT, "H"),
    "u16":    (FieldKind.INT, "H"),
    "int32":  (FieldKind.INT, "i"),
    "i32":    (FieldKind.INT, "i"),
    "uint32": (FieldKind.INT, "I"),
    "u32":    (FieldKind.INT, "I"),
    "int64":  (FieldKind.INT, "q"),
    "i64":    (FieldKind.INT, "q"),
    "uint64": (FieldKind.INT, "Q"),
    "u64":    (FieldKind.INT, "Q"),
    "float32":(FieldKind.FLOAT, "f"),
    "f32":    (FieldKind.FLOAT, "f"),
    "float":  (FieldKind.FLOAT, "f"),
    "float64":(FieldKind.FLOAT, "d"),
    "f64":    (FieldKind.FLOAT, "d"),
    "formid": (FieldKind.FORMID, "I"),
    "form_id":(FieldKind.FORMID, "I"),
}


def _decode_simple(raw: bytes, fmt: str) -> int | float | None:
    size = struct.calcsize(fmt)
    if len(raw) < size:
        return None
    try:
        return struct.unpack_from(fmt, raw)[0]
    except struct.error:
        return None


def _decode_cstring(raw: bytes, encoding: str = "cp1252") -> str:
    payload = bytes(raw)
    if payload.endswith(b"\x00"):
        payload = payload[:-1]
    return payload.decode(encoding, errors="replace")


# ---------------------------------------------------------------------------
# Codec parsing
# ---------------------------------------------------------------------------

def _parse_struct_codec(codec: str) -> str | None:
    """``struct:i,B,B,B,B`` → ``<iBBBB``. Returns None if codec isn't a struct."""
    if not codec or not codec.startswith("struct:"):
        return None
    chars = codec[len("struct:"):].replace(",", "").replace(" ", "")
    if not chars:
        return None
    return "<" + chars


def _parse_array_struct_codec(codec: str) -> str | None:
    """``array_struct:I,I`` → element layout ``<II``."""
    if not codec or not codec.startswith("array_struct:"):
        return None
    chars = codec[len("array_struct:"):].replace(",", "").replace(" ", "")
    if not chars:
        return None
    return "<" + chars


def _parse_fixed_string_codec(codec: str) -> int | None:
    """``fixed_string:4`` → 4."""
    if not codec or not codec.startswith("fixed_string:"):
        return None
    try:
        return int(codec[len("fixed_string:"):])
    except ValueError:
        return None


def _layout_from_fieldspecs(specs: tuple[FieldSpec, ...]) -> str | None:
    """Build a ``<...`` Python struct format string from a list of FieldSpecs.

    Used as a fallback when the codec doesn't carry a layout but the spec.fields
    list is fully numeric. Returns None if any field isn't a fixed-size scalar.
    """
    parts: list[str] = []
    for fs in specs:
        info = _NUMERIC_KIND.get(fs.kind)
        if info is None:
            return None
        parts.append(info[1])
    if not parts:
        return None
    return "<" + "".join(parts)


# ---------------------------------------------------------------------------
# BYTES / STRUCT / ARRAY codecs (legacy helpers — kept for tests + back-compat)
# ---------------------------------------------------------------------------

def decode_bytes(raw: bytes) -> bytes:
    """Passthrough — returns raw bytes unchanged."""
    return bytes(raw)


def encode_bytes(value: bytes | bytearray | str) -> bytes:
    """Accept raw bytes, bytearray, or a hex string like '0011AABB'."""
    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    if isinstance(value, str):
        cleaned = value.replace(" ", "").replace(":", "")
        try:
            return bytes.fromhex(cleaned)
        except ValueError:
            return value.encode("cp1252", errors="replace")
    return bytes(value)


def decode_struct(raw: bytes, layout: str | None) -> tuple | bytes:
    if not layout:
        return decode_bytes(raw)
    try:
        size = struct.calcsize(layout)
    except struct.error:
        return decode_bytes(raw)
    if len(raw) < size:
        return decode_bytes(raw)
    try:
        return struct.unpack_from(layout, raw)
    except struct.error:
        return decode_bytes(raw)


def encode_struct(values: tuple | list | bytes | bytearray | str, layout: str | None) -> bytes:
    if not layout:
        return encode_bytes(values)  # type: ignore[arg-type]
    if isinstance(values, (bytes, bytearray, str)):
        return encode_bytes(values)  # type: ignore[arg-type]
    try:
        return struct.pack(layout, *values)
    except (struct.error, TypeError):
        return b""


def decode_array(raw: bytes, element_layout: str | None) -> list | bytes:
    if not element_layout:
        return decode_bytes(raw)
    try:
        element_size = struct.calcsize(element_layout)
    except struct.error:
        return decode_bytes(raw)
    if element_size == 0 or len(raw) == 0:
        return decode_bytes(raw)

    data = bytes(raw)
    count = len(data) // element_size
    results = []
    for i in range(count):
        chunk = data[i * element_size: (i + 1) * element_size]
        try:
            unpacked = struct.unpack(element_layout, chunk)
            results.append(unpacked[0] if len(unpacked) == 1 else unpacked)
        except struct.error:
            results.append(chunk)
    return results


def encode_array(values: list | bytes | bytearray | str, element_layout: str | None) -> bytes:
    if not element_layout:
        return encode_bytes(values)  # type: ignore[arg-type]
    if isinstance(values, (bytes, bytearray, str)):
        return encode_bytes(values)  # type: ignore[arg-type]
    out = bytearray()
    for item in values:
        try:
            if isinstance(item, (list, tuple)):
                out += struct.pack(element_layout, *item)
            else:
                out += struct.pack(element_layout, item)
        except (struct.error, TypeError):
            pass
    return bytes(out)


# ---------------------------------------------------------------------------
# Component / row builders
# ---------------------------------------------------------------------------

def _enum_for(ref: str | None, schema_enums: dict[str, EnumDef]) -> EnumDef | None:
    if not ref:
        return None
    return schema_enums.get(ref)


def _enum_options(enum_def: EnumDef) -> list[tuple[int, str]]:
    """(value, label) pairs for ENUM combo boxes."""
    if enum_def.labels:
        return list(enum_def.labels)
    return list(enum_def.values)


def _build_component(
    fs: FieldSpec,
    value: int | float,
    *,
    schema_enums: dict[str, EnumDef],
) -> Field:
    """Build a single labelled sub-Field for a struct member or array column."""
    label = fs.authoring_label or fs.name
    enum_def = _enum_for(fs.enum_ref, schema_enums)

    formlink = fs.formlink_target
    if formlink or fs.kind in ("formid", "form_id"):
        return Field(
            name=label,
            signature=fs.name,
            kind=FieldKind.FORMID,
            value=int(value or 0),
            raw=b"",
            formlink_target=formlink,
            notes=fs.notes or "",
        )

    if enum_def is not None:
        if (enum_def.storage_kind or "").lower() == "flags":
            return Field(
                name=label,
                signature=fs.name,
                kind=FieldKind.FLAGSET,
                value=int(value or 0),
                raw=b"",
                enum_def=enum_def,
                flag_options=_enum_options(enum_def),
                notes=fs.notes or "",
            )
        return Field(
            name=label,
            signature=fs.name,
            kind=FieldKind.ENUM,
            value=int(value or 0),
            raw=b"",
            enum_def=enum_def,
            enum_options=_enum_options(enum_def),
            notes=fs.notes or "",
        )

    info = _NUMERIC_KIND.get(fs.kind)
    if info is not None:
        kind, _fmt = info
        cast = int if kind in (FieldKind.INT, FieldKind.FORMID) else float
        return Field(
            name=label,
            signature=fs.name,
            kind=kind,
            value=cast(value or 0),
            raw=b"",
            notes=fs.notes or "",
        )

    # Fallback — show whatever we have as bytes.
    if isinstance(value, (bytes, bytearray)):
        raw = bytes(value)
    else:
        raw = b""
    return Field(
        name=label,
        signature=fs.name,
        kind=FieldKind.BYTES,
        value=raw,
        raw=raw,
        notes=fs.notes or "",
    )


def _build_struct_field(
    *,
    name: str,
    signature: str,
    raw: bytes,
    spec: SubrecordSpec,
    layout: str,
    schema_enums: dict[str, EnumDef],
    notes: str,
) -> Field:
    field_specs = spec.fields or ()
    try:
        values = struct.unpack_from(layout, raw)
    except struct.error:
        return Field(name, signature, FieldKind.BYTES, bytes(raw), bytes(raw), spec, notes=notes)

    components: list[Field] = []
    for fs, val in zip(field_specs, values):
        components.append(_build_component(fs, val, schema_enums=schema_enums))
    # If the spec didn't supply enough FieldSpecs, fill in placeholders so the UI
    # at least shows the raw value.
    if len(field_specs) < len(values):
        for i in range(len(field_specs), len(values)):
            components.append(
                Field(
                    name=f"value[{i}]",
                    signature=f"_{i}",
                    kind=FieldKind.INT if isinstance(values[i], int) else FieldKind.FLOAT,
                    value=values[i],
                    raw=b"",
                )
            )
    return Field(
        name=name,
        signature=signature,
        kind=FieldKind.STRUCT,
        value=tuple(values),
        raw=bytes(raw),
        spec=spec,
        notes=notes,
        struct_layout=layout,
        components=components,
    )


def _build_array_field(
    *,
    name: str,
    signature: str,
    raw: bytes,
    spec: SubrecordSpec,
    element_layout: str,
    schema_enums: dict[str, EnumDef],
    notes: str,
) -> Field:
    field_specs = spec.fields or ()
    try:
        element_size = struct.calcsize(element_layout)
    except struct.error:
        return Field(name, signature, FieldKind.BYTES, bytes(raw), bytes(raw), spec, notes=notes)
    if element_size <= 0:
        return Field(name, signature, FieldKind.BYTES, bytes(raw), bytes(raw), spec, notes=notes)

    rows: list[list[Field]] = []
    data = bytes(raw)
    count = len(data) // element_size
    for i in range(count):
        chunk = data[i * element_size: (i + 1) * element_size]
        try:
            values = struct.unpack(element_layout, chunk)
        except struct.error:
            break
        row: list[Field] = []
        for fs, val in zip(field_specs, values):
            row.append(_build_component(fs, val, schema_enums=schema_enums))
        # Backfill if FieldSpec list is shorter than tuple width.
        if len(field_specs) < len(values):
            for j in range(len(field_specs), len(values)):
                row.append(
                    Field(
                        name=f"value[{j}]",
                        signature=f"_{j}",
                        kind=FieldKind.INT if isinstance(values[j], int) else FieldKind.FLOAT,
                        value=values[j],
                        raw=b"",
                    )
                )
        rows.append(row)
    return Field(
        name=name,
        signature=signature,
        kind=FieldKind.ARRAY,
        value=rows,
        raw=bytes(raw),
        spec=spec,
        notes=notes,
        struct_layout=element_layout,
        element_layout=element_layout,
        element_field_specs=tuple(field_specs),
        rows=rows,
    )


# ---------------------------------------------------------------------------
# Top-level builder
# ---------------------------------------------------------------------------

def _build_field(
    signature: str,
    raw_bytes: bytes,
    spec: SubrecordSpec | None,
    *,
    is_localized: bool,
    schema_enums: dict[str, EnumDef] | None = None,
) -> Field:
    schema_enums = schema_enums or {}
    name = (spec.display_label if spec else None) or signature
    notes = (spec.notes if spec else "") or ""
    raw = bytes(raw_bytes)

    if spec is None:
        return Field(name, signature, FieldKind.BYTES, raw, raw, spec, notes=notes)

    # Preserve case in `codec` — struct format chars (`I` vs `i`, `H` vs `h`)
    # are case-sensitive. Use `codec_lc` only for prefix/keyword matching.
    codec = spec.codec or ""
    codec_lc = codec.lower()

    # Resolve enum/flag/formlink at the subrecord level (used for single-value codecs).
    sub_enum = _enum_for(spec.enum_ref, schema_enums)
    sub_formlink = spec.formlink_target

    # FormID-typed subrecords (single u32 form id).
    if sub_formlink and codec_lc in ("formid", "form_id", "uint32", "u32", ""):
        value = _decode_simple(raw, "<I")
        return Field(
            name, signature, FieldKind.FORMID, value, raw, spec,
            formlink_target=sub_formlink, notes=notes,
        )

    # `empty` — zero-length marker subrecord.
    if codec_lc == "empty" or len(raw) == 0:
        return Field(name, signature, FieldKind.EMPTY, b"", raw, spec, notes=notes)

    # Single-scalar numeric codecs (uint32/int16/float32/...).
    if codec_lc in _NUMERIC_KIND:
        kind, fmt_char = _NUMERIC_KIND[codec_lc]
        value = _decode_simple(raw, "<" + fmt_char)
        if sub_enum is not None:
            if (sub_enum.storage_kind or "").lower() == "flags":
                return Field(
                    name, signature, FieldKind.FLAGSET, int(value or 0), raw, spec,
                    enum_def=sub_enum, flag_options=_enum_options(sub_enum), notes=notes,
                )
            return Field(
                name, signature, FieldKind.ENUM, int(value or 0), raw, spec,
                enum_def=sub_enum, enum_options=_enum_options(sub_enum), notes=notes,
            )
        # Legacy spec.enum/spec.flags fallback (older hand-written specs).
        if spec.enum:
            return Field(
                name, signature, FieldKind.ENUM, int(value or 0), raw, spec,
                enum_options=list(spec.enum), notes=notes,
            )
        if spec.flags:
            return Field(
                name, signature, FieldKind.FLAGSET, int(value or 0), raw, spec,
                flag_options=list(spec.flags), notes=notes,
            )
        return Field(name, signature, kind, value, raw, spec, notes=notes)

    # zstring / cstring / fixed-width strings.
    if codec_lc in ("cstring", "zstring", "string"):
        return Field(
            name, signature, FieldKind.STRING,
            _decode_cstring(raw), raw, spec, notes=notes,
        )

    fixed_len = _parse_fixed_string_codec(codec_lc)
    if fixed_len is not None:
        text = raw[:fixed_len].rstrip(b"\x00").decode("cp1252", errors="replace")
        return Field(name, signature, FieldKind.STRING, text, raw, spec, notes=notes)

    if codec_lc == "lenstring16":
        text = ""
        if len(raw) >= 2:
            length = int.from_bytes(raw[:2], "little")
            text = raw[2:2 + length].decode("cp1252", errors="replace")
        return Field(name, signature, FieldKind.STRING, text, raw, spec, notes=notes)

    # lstring — 4-byte string id when localized, otherwise zstring.
    if codec_lc == "lstring":
        if is_localized and len(raw) >= 4:
            return Field(
                name, signature, FieldKind.LSTRING,
                _decode_simple(raw, "<I"), raw, spec, notes=notes,
            )
        return Field(
            name, signature, FieldKind.STRING,
            _decode_cstring(raw), raw, spec, notes=notes,
        )

    # struct:... — fixed-width struct of named members.
    struct_layout = _parse_struct_codec(codec)
    if (
        struct_layout is None
        and not codec_lc.startswith(("array_struct:", "row_array", "formid_array"))
        and spec.array is None
        and (spec.fields and len(spec.fields) > 1)
    ):
        # Fall back to fieldspecs when codec is missing but the spec describes a struct.
        struct_layout = _layout_from_fieldspecs(spec.fields)
    if struct_layout is not None and len(raw) >= struct.calcsize(struct_layout):
        return _build_struct_field(
            name=name,
            signature=signature,
            raw=raw,
            spec=spec,
            layout=struct_layout,
            schema_enums=schema_enums,
            notes=notes,
        )

    # array_struct:... — flat array of fixed-size element rows.
    elem_layout = _parse_array_struct_codec(codec)
    if elem_layout is None and codec_lc == "formid_array":
        elem_layout = "<I"
    if elem_layout is None and codec_lc == "row_array" and spec.fields:
        elem_layout = _layout_from_fieldspecs(spec.fields)
    if elem_layout is None and spec.array is not None:
        # Older specs put the layout in spec.array.element_codec.
        ec = spec.array.element_codec
        if ec:
            chars = ec.replace(",", "").replace(" ", "")
            if chars:
                elem_layout = "<" + chars

    if elem_layout is not None:
        return _build_array_field(
            name=name,
            signature=signature,
            raw=raw,
            spec=spec,
            element_layout=elem_layout,
            schema_enums=schema_enums,
            notes=notes,
        )

    # Generic fallback: hex view.
    return Field(name, signature, FieldKind.BYTES, raw, raw, spec, notes=notes)


def make_array_row(field_obj: Field, *, game: str) -> list[Field]:
    """Build a fresh default-zero row for an ARRAY field. Used by the UI when the
    user clicks "Add" on an array widget."""
    schema = get_schema(game)
    return [
        _build_component(fs, 0, schema_enums=schema.enums)
        for fs in (field_obj.element_field_specs or ())
    ]


def clone_array_row(row: list[Field]) -> list[Field]:
    """Deep-copy an array row so the duplicated entry is independent of the source."""
    import copy as _copy
    return [_copy.deepcopy(c) for c in row]


def decode_record(record, *, game: str, is_localized: bool = False) -> list[Field]:
    """Decode a `creation_lib.esp.model.Record`'s subrecords into UI-ready `Field`s."""
    schema = get_schema(game)
    record_spec = schema.records.get(record.signature)
    sub_specs: dict[str, SubrecordSpec] = {}
    if record_spec is not None:
        for sub in record_spec.subrecords:
            sub_specs[sub.sig] = sub

    fields: list[Field] = []
    for sub in record.subrecords:
        spec = sub_specs.get(sub.signature)
        raw = bytes(sub.data)
        fields.append(
            _build_field(
                sub.signature,
                raw,
                spec,
                is_localized=is_localized,
                schema_enums=schema.enums,
            )
        )
    return fields


# ---------------------------------------------------------------------------
# Re-encoding
# ---------------------------------------------------------------------------

def _component_value(comp: Field) -> Any:
    """Return a Python primitive suitable for ``struct.pack`` for a component Field."""
    if comp.kind in (FieldKind.INT, FieldKind.FORMID, FieldKind.ENUM, FieldKind.FLAGSET):
        try:
            return int(comp.value or 0)
        except (TypeError, ValueError):
            return 0
    if comp.kind == FieldKind.FLOAT:
        try:
            return float(comp.value or 0.0)
        except (TypeError, ValueError):
            return 0.0
    if comp.kind == FieldKind.BYTES:
        return comp.value or 0
    return comp.value


def _pack_components(layout: str | None, components: list[Field]) -> bytes | None:
    if not layout:
        return None
    try:
        return struct.pack(layout, *(_component_value(c) for c in components))
    except (struct.error, TypeError):
        return None


def encode_field(field_obj: Field, value: Any) -> bytes:
    """Re-encode an edited `Field` value back to subrecord bytes.

    For STRUCT/ARRAY fields the canonical source is the field's own components/rows
    — `value` is accepted as a tuple/list for back-compat (tests + scalar callers).
    """
    spec = field_obj.spec
    codec = (spec.codec.lower() if spec and spec.codec else "")

    # Scalars via SubrecordSpec.codec.
    if field_obj.kind in (FieldKind.INT, FieldKind.FORMID, FieldKind.ENUM, FieldKind.LSTRING):
        fmt = "<I"
        if codec in _NUMERIC_KIND:
            fmt = "<" + _NUMERIC_KIND[codec][1]
        elif field_obj.kind == FieldKind.FORMID:
            fmt = "<I"
        try:
            return struct.pack(fmt, int(value))
        except struct.error:
            return field_obj.raw

    if field_obj.kind == FieldKind.FLAGSET:
        fmt = "<I"
        if codec in _NUMERIC_KIND:
            fmt = "<" + _NUMERIC_KIND[codec][1]
        try:
            return struct.pack(fmt, int(value) & 0xFFFFFFFFFFFFFFFF)
        except struct.error:
            return field_obj.raw

    if field_obj.kind == FieldKind.FLOAT:
        try:
            return struct.pack("<f", float(value))
        except struct.error:
            return field_obj.raw

    if field_obj.kind == FieldKind.STRING:
        text = str(value)
        if codec == "lenstring16":
            payload = text.encode("cp1252", errors="replace")
            return len(payload).to_bytes(2, "little") + payload
        fixed_len = _parse_fixed_string_codec(codec)
        if fixed_len is not None:
            payload = text.encode("cp1252", errors="replace")[:fixed_len]
            return payload.ljust(fixed_len, b"\x00")
        # cstring / zstring / lstring (non-localized).
        return text.encode("cp1252", errors="replace") + b"\x00"

    if field_obj.kind == FieldKind.EMPTY:
        return b""

    if field_obj.kind == FieldKind.BYTES:
        return encode_bytes(value)

    if field_obj.kind == FieldKind.STRUCT:
        # Prefer hierarchical components when present (UI edits sub-fields).
        if field_obj.components:
            packed = _pack_components(field_obj.struct_layout, field_obj.components)
            if packed is not None:
                return packed
        return encode_struct(value, field_obj.struct_layout)

    if field_obj.kind == FieldKind.ARRAY:
        if field_obj.rows and field_obj.element_layout:
            out = bytearray()
            for row in field_obj.rows:
                packed = _pack_components(field_obj.element_layout, row)
                if packed is not None:
                    out += packed
            return bytes(out)
        return encode_array(value, field_obj.struct_layout)

    if isinstance(value, (bytes, bytearray)):
        return bytes(value)
    return field_obj.raw
