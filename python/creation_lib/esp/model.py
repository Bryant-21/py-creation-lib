"""Python-owned model objects for Bethesda plugins."""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from typing import Any, Iterable

LOCAL_FORM_INDEX = 0xFF
COMPRESSED_RECORD_FLAG = 1 << 18
TES4_FLAG_MASTER = 0x00000001
TES4_FLAG_LOCALIZED = 0x00000080
TES4_FLAG_LIGHT = 0x00000200


def _bytes(value: Any | None) -> bytes:
    if value is None:
        return b""
    if isinstance(value, bytes):
        return value
    if isinstance(value, bytearray):
        return bytes(value)
    return bytes(value)


def _decode_text(data: bytes | bytearray, encoding: str = "cp1252") -> str:
    payload = bytes(data)
    if payload.endswith(b"\x00"):
        payload = payload[:-1]
    return payload.decode(encoding, errors="replace")


def _encode_text(value: str, encoding: str = "cp1252", *, null_terminated: bool = True) -> bytearray:
    payload = bytearray(value.encode(encoding, errors="replace"))
    if null_terminated:
        payload.append(0)
    return payload


@dataclass
class FormRef:
    plugin_name: str | None
    object_id: int
    raw: int | None = None
    missing_index: int | None = None

    def __post_init__(self) -> None:
        self.object_id = int(self.object_id) & 0x00FF_FFFF
        if self.raw is not None:
            self.raw = int(self.raw) & 0xFFFF_FFFF
        if self.missing_index is not None:
            self.missing_index = int(self.missing_index) & 0xFF

    @classmethod
    def null(cls) -> "FormRef":
        return cls(None, 0, raw=0)

    @classmethod
    def local(cls, object_id: int) -> "FormRef":
        object_id = int(object_id) & 0x00FF_FFFF
        return cls(None, object_id, raw=(LOCAL_FORM_INDEX << 24) | object_id)

    @classmethod
    def from_raw(cls, raw: int, *, plugin_name: str, masters: Iterable[str]) -> "FormRef":
        raw = int(raw) & 0xFFFFFFFF
        if raw == 0:
            return cls.null()
        index = (raw >> 24) & 0xFF
        object_id = raw & 0x00FF_FFFF
        if index == LOCAL_FORM_INDEX:
            return cls(None, object_id, raw=raw)
        mapping = list(masters) + [plugin_name]
        if index < len(mapping):
            return cls(mapping[index], object_id, raw=raw)
        return cls(None, object_id, raw=raw, missing_index=index)

    def to_raw(
        self,
        *,
        target_plugin_name: str,
        masters: list[str],
        add_missing: bool = False,
    ) -> int:
        if self.object_id == 0:
            return 0
        object_id = self.object_id & 0x00FF_FFFF
        if self.plugin_name is None:
            return (LOCAL_FORM_INDEX << 24) | object_id
        if self.plugin_name.lower() == target_plugin_name.lower():
            return ((len(masters) & 0xFF) << 24) | object_id
        for index, master in enumerate(masters):
            if master.lower() == self.plugin_name.lower():
                return ((index & 0xFF) << 24) | object_id
        if not add_missing:
            raise KeyError(f"Missing master for FormRef: {self.plugin_name}")
        masters.append(self.plugin_name)
        return (((len(masters) - 1) & 0xFF) << 24) | object_id


@dataclass
class Subrecord:
    signature: str
    data: bytearray | bytes | None = None
    semantic_type: str | None = None

    def __post_init__(self) -> None:
        if len(self.signature) != 4:
            raise ValueError(f"Subrecord signature must be 4 chars: {self.signature!r}")
        self.data = bytearray(_bytes(self.data))

    def clone(self) -> "Subrecord":
        return Subrecord(self.signature, bytearray(self.data), self.semantic_type)

    @property
    def size(self) -> int:
        return len(self.data)

    def get_uint32(self, offset: int = 0) -> int:
        if offset + 4 > len(self.data):
            raise ValueError("subrecord payload too small for uint32 read")
        return struct.unpack_from("<I", self.data, offset)[0]

    def set_uint32(self, offset: int, value: int) -> None:
        if len(self.data) < offset + 4:
            self.data.extend(b"\x00" * (offset + 4 - len(self.data)))
        struct.pack_into("<I", self.data, offset, int(value) & 0xFFFFFFFF)

    def get_float(self, offset: int = 0) -> float:
        if offset + 4 > len(self.data):
            raise ValueError("subrecord payload too small for float read")
        return struct.unpack_from("<f", self.data, offset)[0]

    def set_float(self, offset: int, value: float) -> None:
        if len(self.data) < offset + 4:
            self.data.extend(b"\x00" * (offset + 4 - len(self.data)))
        struct.pack_into("<f", self.data, offset, float(value))

    def get_string(self, encoding: str = "cp1252") -> str:
        return _decode_text(self.data, encoding)

    def set_string(self, value: str, encoding: str = "cp1252", *, null_terminated: bool = True) -> None:
        self.data = _encode_text(value, encoding, null_terminated=null_terminated)

    def set_form_ref(self, value: FormRef | int) -> None:
        self.semantic_type = "formid"
        raw = value.raw if isinstance(value, FormRef) else int(value)
        self.data = bytearray(struct.pack("<I", int(raw or 0) & 0xFFFFFFFF))

    def set_form_ref_array(self, values: Iterable[FormRef | int]) -> None:
        self.semantic_type = "formid_array"
        payload = bytearray()
        for value in values:
            raw = value.raw if isinstance(value, FormRef) else int(value)
            payload.extend(struct.pack("<I", int(raw or 0) & 0xFFFFFFFF))
        self.data = payload


@dataclass
class Record:
    signature: str
    form_id: int
    flags: int = 0
    version_control: int = 0
    form_version: int | None = None
    version2: int | None = None
    subrecords: list[Subrecord] = field(default_factory=list)
    raw_payload: bytes | None = None
    parse_error: str | None = None

    def __post_init__(self) -> None:
        if len(self.signature) != 4:
            raise ValueError(f"Record signature must be 4 chars: {self.signature!r}")
        self.form_id = int(self.form_id) & 0xFFFFFFFF
        self.flags = int(self.flags) & 0xFFFFFFFF
        self.version_control = int(self.version_control) & 0xFFFFFFFF
        self.subrecords = [
            item if isinstance(item, Subrecord) else Subrecord(**item)
            for item in (self.subrecords or [])
        ]
        if self.raw_payload is not None:
            self.raw_payload = bytes(self.raw_payload)

    def clone(self) -> "Record":
        return Record(
            signature=self.signature,
            form_id=self.form_id,
            flags=self.flags,
            version_control=self.version_control,
            form_version=self.form_version,
            version2=self.version2,
            subrecords=[sub.clone() for sub in self.subrecords],
            raw_payload=self.raw_payload,
            parse_error=self.parse_error,
        )

    @property
    def compressed(self) -> bool:
        return (self.flags & COMPRESSED_RECORD_FLAG) != 0

    @compressed.setter
    def compressed(self, value: bool) -> None:
        if value:
            self.flags |= COMPRESSED_RECORD_FLAG
        else:
            self.flags &= ~COMPRESSED_RECORD_FLAG

    @property
    def object_id(self) -> int:
        return self.form_id & 0x00FF_FFFF

    def get_subrecord(self, signature: str, occurrence: int = 0) -> Subrecord | None:
        if occurrence < 0:
            return None
        count = 0
        for subrecord in self.subrecords:
            if subrecord.signature != signature:
                continue
            if count == occurrence:
                return subrecord
            count += 1
        return None

    def get_subrecords(self, signature: str) -> list[Subrecord]:
        return [subrecord for subrecord in self.subrecords if subrecord.signature == signature]

    def add_subrecord(
        self,
        signature: str,
        data: bytes | bytearray | None = None,
        *,
        semantic_type: str | None = None,
    ) -> Subrecord:
        subrecord = Subrecord(signature, data, semantic_type)
        self.subrecords.append(subrecord)
        return subrecord

    def remove_subrecord(self, signature: str, occurrence: int = 0) -> bool:
        if occurrence < 0:
            return False
        count = 0
        for index, subrecord in enumerate(self.subrecords):
            if subrecord.signature != signature:
                continue
            if count == occurrence:
                del self.subrecords[index]
                return True
            count += 1
        return False

    def has_subrecord(self, signature: str) -> bool:
        return any(subrecord.signature == signature for subrecord in self.subrecords)

    def resolve_text(
        self,
        signature: str,
        *,
        plugin: Any | None = None,
        localized: bool | None = None,
    ) -> str | None:
        subrecord = self.get_subrecord(signature)
        if subrecord is None:
            return None
        use_localized = localized
        if use_localized is None:
            use_localized = bool(plugin and plugin.header.is_localized)
        if use_localized and len(subrecord.data) == 4 and plugin is not None:
            return plugin.resolve_string(subrecord.get_uint32())
        return subrecord.get_string()

    def get_name(self, *, plugin: Any | None = None, localized: bool | None = None) -> str | None:
        for signature in ("FULL", "RNAM"):
            value = self.resolve_text(signature, plugin=plugin, localized=localized)
            if value:
                return value
        return None

    def get_description(self, *, plugin: Any | None = None, localized: bool | None = None) -> str | None:
        for signature in ("DESC", "ITXT", "SHRT"):
            value = self.resolve_text(signature, plugin=plugin, localized=localized)
            if value:
                return value
        return None

    @property
    def editor_id(self) -> str | None:
        subrecord = self.get_subrecord("EDID")
        return subrecord.get_string() if subrecord is not None else None

    @editor_id.setter
    def editor_id(self, value: str) -> None:
        subrecord = self.get_subrecord("EDID") or self.add_subrecord("EDID")
        subrecord.set_string(value)

    @property
    def full_name(self) -> str | None:
        for signature in ("FULL", "RNAM"):
            subrecord = self.get_subrecord(signature)
            if subrecord is not None:
                return subrecord.get_string()
        return None

    @full_name.setter
    def full_name(self, value: str) -> None:
        subrecord = self.get_subrecord("FULL") or self.add_subrecord("FULL")
        subrecord.set_string(value)


@dataclass
class Group:
    label: bytes | bytearray
    group_type: int
    tail: bytes | bytearray | None = None
    children: list["Group | Record"] = field(default_factory=list)

    def __post_init__(self) -> None:
        label = _bytes(self.label)[:4]
        self.label = label + (b"\x00" * (4 - len(label)))
        self.group_type = int(self.group_type)
        self.tail = _bytes(self.tail)
        self.children = list(self.children or [])

    def clone(self) -> "Group":
        return Group(
            self.label,
            self.group_type,
            tail=bytes(self.tail),
            children=[child.clone() for child in self.children],
        )

    @property
    def label_text(self) -> str:
        return bytes(self.label).rstrip(b"\x00").decode("ascii", errors="replace")

    def walk_records(self) -> list[Record]:
        out: list[Record] = []
        for child in self.children:
            if isinstance(child, Group):
                out.extend(child.walk_records())
            else:
                out.append(child)
        return out


@dataclass
class PluginHeader:
    version: float = 1.0
    num_records: int = 0
    next_object_id: int = 0x800
    author: str = ""
    description: str = ""
    masters: list[str] = field(default_factory=list)
    master_sizes: list[int] = field(default_factory=list)
    overridden_forms: list[int] = field(default_factory=list)
    flags: int = 0
    extra_subrecords: list[Subrecord] = field(default_factory=list)
    version_control: int = 0
    form_version: int | None = None
    version2: int | None = None
    hedr_raw: bytes | None = None
    _raw_subrecords: list[Subrecord] = field(default_factory=list)
    hedr_changed: bool = False

    def __post_init__(self) -> None:
        self.version = float(self.version)
        self.num_records = int(self.num_records)
        self.next_object_id = int(self.next_object_id) & 0x00FF_FFFF
        self.flags = int(self.flags) & 0xFFFFFFFF
        self.version_control = int(self.version_control) & 0xFFFFFFFF
        self.masters = [str(master) for master in (self.masters or [])]
        self.master_sizes = [int(size) for size in (self.master_sizes or [])]
        self.overridden_forms = [int(form_id) & 0xFFFFFFFF for form_id in (self.overridden_forms or [])]
        self.extra_subrecords = [
            item if isinstance(item, Subrecord) else Subrecord(**item)
            for item in (self.extra_subrecords or [])
        ]
        if self.hedr_raw is not None:
            self.hedr_raw = bytes(self.hedr_raw)
        self._raw_subrecords = [
            item if isinstance(item, Subrecord) else Subrecord(**item)
            for item in (self._raw_subrecords or [])
        ]

    @property
    def is_master(self) -> bool:
        return (self.flags & TES4_FLAG_MASTER) != 0

    @is_master.setter
    def is_master(self, value: bool) -> None:
        if value:
            self.flags |= TES4_FLAG_MASTER
        else:
            self.flags &= ~TES4_FLAG_MASTER

    @property
    def is_localized(self) -> bool:
        return (self.flags & TES4_FLAG_LOCALIZED) != 0

    @is_localized.setter
    def is_localized(self, value: bool) -> None:
        if value:
            self.flags |= TES4_FLAG_LOCALIZED
        else:
            self.flags &= ~TES4_FLAG_LOCALIZED

    @property
    def is_light(self) -> bool:
        return (self.flags & TES4_FLAG_LIGHT) != 0

    @is_light.setter
    def is_light(self, value: bool) -> None:
        if value:
            self.flags |= TES4_FLAG_LIGHT
        else:
            self.flags &= ~TES4_FLAG_LIGHT

    def set_hedr_raw(self, data: bytes | bytearray) -> None:
        self.hedr_raw = bytes(data)
        self.hedr_changed = False

    @property
    def hedr_bytes(self) -> bytes:
        if self.hedr_raw is not None:
            return self.hedr_raw
        return struct.pack("<fII", float(self.version), int(self.num_records), int(self.next_object_id))

    @hedr_bytes.setter
    def hedr_bytes(self, data: bytes | bytearray) -> None:
        self.hedr_raw = bytes(data)
        self.hedr_changed = True
