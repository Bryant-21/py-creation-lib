"""Schema manifest models derived from observed plugin corpora."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from creation_lib.esp.model import Record


@dataclass(slots=True)
class SubrecordObservation:
    signature: str
    name: str | None = None
    count: int = 0
    record_count: int = 0
    min_size: int | None = None
    max_size: int = 0
    total_size: int = 0
    repeatable: bool = False
    known_formid: bool = False
    xedit_name: str | None = None
    xedit_builder: str | None = None
    xedit_repeatable: bool = False
    xedit_required: bool = False
    xedit_union: bool = False
    xedit_struct: bool = False
    xedit_formid: bool = False
    xedit_value_domain_kind: str | None = None
    xedit_value_domain_name: str | None = None

    def observe(self, sizes: list[int]) -> None:
        if not sizes:
            return
        self.count += len(sizes)
        self.record_count += 1
        self.total_size += sum(sizes)
        self.repeatable = self.repeatable or len(sizes) > 1
        local_min = min(sizes)
        local_max = max(sizes)
        self.min_size = local_min if self.min_size is None else min(self.min_size, local_min)
        self.max_size = max(self.max_size, local_max)

    def apply_xedit_member(self, member: dict[str, Any]) -> None:
        self.xedit_name = member.get("name") or self.xedit_name
        self.xedit_builder = member.get("builder") or self.xedit_builder
        self.xedit_repeatable = self.xedit_repeatable or bool(member.get("repeatable", False))
        self.xedit_required = self.xedit_required or bool(member.get("required", False))
        self.xedit_union = self.xedit_union or bool(member.get("union", False))
        self.xedit_struct = self.xedit_struct or bool(member.get("struct", False))
        self.xedit_formid = self.xedit_formid or bool(member.get("formid", False))
        self.xedit_value_domain_kind = member.get("value_domain_kind") or self.xedit_value_domain_kind
        self.xedit_value_domain_name = member.get("value_domain_name") or self.xedit_value_domain_name
        if self.xedit_name and not self.name:
            self.name = self.xedit_name
        if self.xedit_repeatable:
            self.repeatable = True
        if self.xedit_formid or (self.xedit_builder and self.xedit_builder.startswith("wbFormID")):
            self.xedit_formid = True
            self.known_formid = True
        elif self.known_formid:
            self.xedit_formid = True

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "count": self.count,
            "record_count": self.record_count,
            "min_size": self.min_size,
            "max_size": self.max_size,
            "avg_size": (self.total_size / self.count) if self.count else 0,
            "repeatable": self.repeatable,
            "known_formid": self.known_formid,
            "xedit_name": self.xedit_name,
            "xedit_builder": self.xedit_builder,
            "xedit_repeatable": self.xedit_repeatable,
            "xedit_required": self.xedit_required,
            "xedit_union": self.xedit_union,
            "xedit_struct": self.xedit_struct,
            "xedit_formid": self.xedit_formid,
            "xedit_value_domain_kind": self.xedit_value_domain_kind,
            "xedit_value_domain_name": self.xedit_value_domain_name,
        }

    @classmethod
    def from_dict(cls, signature: str, data: dict[str, Any]) -> "SubrecordObservation":
        avg_size = float(data.get("avg_size", 0))
        count = int(data.get("count", 0))
        return cls(
            signature=signature,
            name=data.get("name"),
            count=count,
            record_count=int(data.get("record_count", 0)),
            min_size=data.get("min_size"),
            max_size=int(data.get("max_size", 0)),
            total_size=int(round(avg_size * count)),
            repeatable=bool(data.get("repeatable", False)),
            known_formid=bool(data.get("known_formid", False)),
            xedit_name=data.get("xedit_name"),
            xedit_builder=data.get("xedit_builder"),
            xedit_repeatable=bool(data.get("xedit_repeatable", False)),
            xedit_required=bool(data.get("xedit_required", False)),
            xedit_union=bool(data.get("xedit_union", False)),
            xedit_struct=bool(data.get("xedit_struct", False)),
            xedit_formid=bool(data.get("xedit_formid", False)),
            xedit_value_domain_kind=data.get("xedit_value_domain_kind"),
            xedit_value_domain_name=data.get("xedit_value_domain_name"),
        )


@dataclass(slots=True)
class RecordMember:
    signature: str
    name: str | None = None
    builder: str | None = None
    repeatable: bool = False
    required: bool = False
    union: bool = False
    struct: bool = False
    formid: bool = False
    value_domain_kind: str | None = None
    value_domain_name: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "signature": self.signature,
            "name": self.name,
            "builder": self.builder,
            "repeatable": self.repeatable,
            "required": self.required,
            "union": self.union,
            "struct": self.struct,
            "formid": self.formid,
            "value_domain_kind": self.value_domain_kind,
            "value_domain_name": self.value_domain_name,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "RecordMember":
        return cls(
            signature=str(data["signature"]),
            name=data.get("name"),
            builder=data.get("builder"),
            repeatable=bool(data.get("repeatable", False)),
            required=bool(data.get("required", False)),
            union=bool(data.get("union", False)),
            struct=bool(data.get("struct", False)),
            formid=bool(data.get("formid", False)),
            value_domain_kind=data.get("value_domain_kind"),
            value_domain_name=data.get("value_domain_name"),
        )


@dataclass(slots=True)
class XEditRecordSchema:
    kind: str | None = None
    members: list[RecordMember] = field(default_factory=list)

    def subrecord_signatures(self) -> list[str]:
        return [member.signature for member in self.members]

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "members": [member.to_dict() for member in self.members],
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "XEditRecordSchema":
        return cls(
            kind=data.get("kind"),
            members=[RecordMember.from_dict(item) for item in data.get("members", [])],
        )


def _default_coverage() -> dict[str, int]:
    return {"expected": 0, "observed": 0, "matched": 0, "missing": 0, "unexpected": 0}


def _recompute_coverage(record: "RecordObservation") -> dict[str, int]:
    expected = set(record.xedit_order)
    observed = set(record.subrecords)
    matched = expected & observed
    return {
        "expected": len(expected),
        "observed": len(observed),
        "matched": len(matched),
        "missing": len(expected - observed),
        "unexpected": len(observed - expected),
    }


def _recompute_order_overlap(record: "RecordObservation") -> float | None:
    if not record.xedit_order or not record.examples:
        return None
    best = 0.0
    positions = {signature: index for index, signature in enumerate(record.xedit_order)}
    for example in record.examples:
        shared = [signature for signature in example if signature in positions]
        if not shared:
            continue
        if len(shared) == 1:
            best = max(best, 1.0)
            continue
        ordered_pairs = sum(1 for left, right in zip(shared, shared[1:]) if positions[left] <= positions[right])
        best = max(best, ordered_pairs / (len(shared) - 1))
    return best


def _recompute_is_complete(record: "RecordObservation") -> bool:
    return bool(record.xedit_order) and record.coverage["missing"] == 0


@dataclass(slots=True)
class RecordObservation:
    signature: str
    name: str | None = None
    count: int = 0
    compressed_count: int = 0
    min_size: int | None = None
    max_size: int = 0
    total_size: int = 0
    form_versions: dict[str, int] = field(default_factory=dict)
    group_types: dict[str, int] = field(default_factory=dict)
    subrecords: dict[str, SubrecordObservation] = field(default_factory=dict)
    order_edges: dict[str, int] = field(default_factory=dict)
    examples: list[list[str]] = field(default_factory=list)
    xedit: XEditRecordSchema = field(default_factory=XEditRecordSchema)
    xedit_order: list[str] = field(default_factory=list)
    coverage: dict[str, int] = field(default_factory=_default_coverage)
    order_overlap: float | None = None
    is_ref_record: bool = False
    is_complete: bool = False
    corpus_order_hint: list[str] = field(default_factory=list)

    def observe(self, record: Record, *, group_type: int | None = None) -> None:
        sizes = [subrecord.size for subrecord in record.subrecords]
        payload_size = sum(sizes)
        self.count += 1
        self.total_size += payload_size
        self.min_size = payload_size if self.min_size is None else min(self.min_size, payload_size)
        self.max_size = max(self.max_size, payload_size)
        if record.compressed:
            self.compressed_count += 1
        if record.form_version is not None:
            key = str(record.form_version)
            self.form_versions[key] = self.form_versions.get(key, 0) + 1
        if group_type is not None:
            key = str(group_type)
            self.group_types[key] = self.group_types.get(key, 0) + 1

        occurrences: dict[str, list[int]] = {}
        sequence: list[str] = []
        for subrecord in record.subrecords:
            sequence.append(subrecord.signature)
            occurrences.setdefault(subrecord.signature, []).append(subrecord.size)
        for signature, observed_sizes in occurrences.items():
            self.subrecords.setdefault(signature, SubrecordObservation(signature)).observe(observed_sizes)
        for left, right in zip(sequence, sequence[1:]):
            edge = f"{left}>{right}"
            self.order_edges[edge] = self.order_edges.get(edge, 0) + 1
        if sequence and sequence not in self.examples and len(self.examples) < 5:
            self.examples.append(sequence)
        if sequence and not self.corpus_order_hint:
            self.corpus_order_hint = list(sequence)
        self.refresh_derived()

    def apply_xedit_schema(self, schema: XEditRecordSchema) -> None:
        if schema.kind:
            self.xedit.kind = schema.kind
            if schema.kind == "wbRefRecord":
                self.is_ref_record = True
        existing: dict[str, RecordMember] = {member.signature: member for member in self.xedit.members}
        for member in schema.members:
            current = existing.get(member.signature)
            if current is None:
                current = RecordMember(
                    signature=member.signature,
                    name=member.name,
                    builder=member.builder,
                    repeatable=member.repeatable,
                    required=member.required,
                    union=member.union,
                    struct=member.struct,
                    formid=member.formid,
                    value_domain_kind=member.value_domain_kind,
                    value_domain_name=member.value_domain_name,
                )
                self.xedit.members.append(current)
                existing[member.signature] = current
            else:
                current.name = current.name or member.name
                current.builder = current.builder or member.builder
                current.repeatable = current.repeatable or member.repeatable
                current.required = current.required or member.required
                current.union = current.union or member.union
                current.struct = current.struct or member.struct
                current.formid = current.formid or member.formid
                current.value_domain_kind = current.value_domain_kind or member.value_domain_kind
                current.value_domain_name = current.value_domain_name or member.value_domain_name
            self.subrecords.setdefault(member.signature, SubrecordObservation(signature=member.signature)).apply_xedit_member(
                current.to_dict()
            )
        self.xedit_order = self.xedit.subrecord_signatures()
        self.refresh_derived()

    def refresh_derived(self) -> None:
        self.coverage = _recompute_coverage(self)
        self.order_overlap = _recompute_order_overlap(self)
        self.is_complete = _recompute_is_complete(self)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "count": self.count,
            "compressed_count": self.compressed_count,
            "min_size": self.min_size,
            "max_size": self.max_size,
            "avg_size": (self.total_size / self.count) if self.count else 0,
            "form_versions": dict(sorted(self.form_versions.items())),
            "group_types": dict(sorted(self.group_types.items())),
            "subrecords": {
                signature: observation.to_dict()
                for signature, observation in sorted(self.subrecords.items())
            },
            "order_edges": dict(sorted(self.order_edges.items())),
            "examples": self.examples,
            "xedit": self.xedit.to_dict(),
            "xedit_order": list(self.xedit_order),
            "coverage": dict(self.coverage),
            "order_overlap": self.order_overlap,
            "is_ref_record": self.is_ref_record,
            "is_complete": self.is_complete,
            "corpus_order_hint": list(self.corpus_order_hint),
        }

    @classmethod
    def from_dict(cls, signature: str, data: dict[str, Any]) -> "RecordObservation":
        avg_size = float(data.get("avg_size", 0))
        count = int(data.get("count", 0))
        observation = cls(
            signature=signature,
            name=data.get("name"),
            count=count,
            compressed_count=int(data.get("compressed_count", 0)),
            min_size=data.get("min_size"),
            max_size=int(data.get("max_size", 0)),
            total_size=int(round(avg_size * count)),
            form_versions={str(key): int(value) for key, value in data.get("form_versions", {}).items()},
            group_types={str(key): int(value) for key, value in data.get("group_types", {}).items()},
            subrecords={
                subrecord_signature: SubrecordObservation.from_dict(subrecord_signature, subrecord_data)
                for subrecord_signature, subrecord_data in data.get("subrecords", {}).items()
            },
            order_edges={str(key): int(value) for key, value in data.get("order_edges", {}).items()},
            examples=[list(example) for example in data.get("examples", [])],
            xedit=XEditRecordSchema.from_dict(data.get("xedit", {})),
            xedit_order=list(data.get("xedit_order", [])),
            coverage={str(key): int(value) for key, value in data.get("coverage", _default_coverage()).items()},
            order_overlap=data.get("order_overlap"),
            is_ref_record=bool(data.get("is_ref_record", False)),
            is_complete=bool(data.get("is_complete", False)),
            corpus_order_hint=list(data.get("corpus_order_hint", [])),
        )
        observation.refresh_derived()
        return observation


@dataclass(slots=True)
class SchemaManifest:
    game: str
    generated_at: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    plugins: list[str] = field(default_factory=list)
    records: dict[str, RecordObservation] = field(default_factory=dict)
    group_types: dict[str, int] = field(default_factory=dict)
    unknown_signatures: dict[str, list[str]] = field(default_factory=lambda: {"records": [], "subrecords": []})
    xedit_hints: dict[str, Any] = field(default_factory=dict)
    manual_overrides: dict[str, Any] = field(default_factory=dict)
    notes: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "game": self.game,
            "generated_at": self.generated_at,
            "plugins": list(self.plugins),
            "records": {
                signature: observation.to_dict()
                for signature, observation in sorted(self.records.items())
            },
            "group_types": dict(sorted(self.group_types.items())),
            "unknown_signatures": {
                key: sorted(values)
                for key, values in self.unknown_signatures.items()
            },
            "xedit_hints": self.xedit_hints,
            "manual_overrides": self.manual_overrides,
            "notes": list(self.notes),
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "SchemaManifest":
        return cls(
            game=str(data["game"]),
            generated_at=str(data.get("generated_at", "")),
            plugins=list(data.get("plugins", [])),
            records={
                signature: RecordObservation.from_dict(signature, observation)
                for signature, observation in data.get("records", {}).items()
            },
            group_types={str(key): int(value) for key, value in data.get("group_types", {}).items()},
            unknown_signatures={
                str(key): list(values)
                for key, values in data.get("unknown_signatures", {}).items()
            },
            xedit_hints=dict(data.get("xedit_hints", {})),
            manual_overrides=dict(data.get("manual_overrides", {})),
            notes=list(data.get("notes", [])),
        )

    def save(self, path: str | Path) -> Path:
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")
        return target

    @classmethod
    def load(cls, path: str | Path) -> "SchemaManifest":
        return cls.from_dict(json.loads(Path(path).read_text(encoding="utf-8")))
