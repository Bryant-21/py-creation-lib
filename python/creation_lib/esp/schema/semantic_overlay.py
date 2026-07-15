"""Semantic overlay artifacts derived from verified editor evidence."""

from __future__ import annotations

import json
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any

from .base import FieldSpec, GameSchema, RecordSpec, SubrecordSpec

ARTIFACT_VERSION = 1

PROVENANCE_RANK: dict[str, int] = {
    "editor_label_only": 0,
    "editor_diff_verified_partial": 1,
    "binary_inferred": 1,
    "editor_diff_verified": 2,
    "runtime_verified": 3,
}

CONFIDENCE_RANK: dict[str, int] = {
    "low": 0,
    "medium": 1,
    "high": 2,
}


def _rank(value: str, ranking: dict[str, int]) -> int:
    return ranking.get(value, -1)


def _qualifies(*, provenance: str, confidence: str, min_provenance: str, min_confidence: str) -> bool:
    return _rank(provenance, PROVENANCE_RANK) >= _rank(min_provenance, PROVENANCE_RANK) and _rank(
        confidence, CONFIDENCE_RANK
    ) >= _rank(min_confidence, CONFIDENCE_RANK)


@dataclass(slots=True)
class FieldSemanticOverlay:
    field_name: str
    authoring_label: str
    provenance: str
    confidence: str
    control_label: str | None = None
    primitive_kind: str | None = None
    dialog_id: int | None = None
    dialog_caption: str | None = None
    evidence_paths: tuple[str, ...] = ()
    notes: tuple[str, ...] = ()

    def to_dict(self) -> dict[str, Any]:
        return {
            "authoring_label": self.authoring_label,
            "provenance": self.provenance,
            "confidence": self.confidence,
            "control_label": self.control_label,
            "primitive_kind": self.primitive_kind,
            "dialog_id": self.dialog_id,
            "dialog_caption": self.dialog_caption,
            "evidence_paths": list(self.evidence_paths),
            "notes": list(self.notes),
        }

    @classmethod
    def from_dict(cls, field_name: str, payload: dict[str, Any]) -> "FieldSemanticOverlay":
        return cls(
            field_name=field_name,
            authoring_label=str(payload["authoring_label"]),
            provenance=str(payload.get("provenance", "editor_label_only")),
            confidence=str(payload.get("confidence", "low")),
            control_label=payload.get("control_label"),
            primitive_kind=payload.get("primitive_kind"),
            dialog_id=payload.get("dialog_id"),
            dialog_caption=payload.get("dialog_caption"),
            evidence_paths=tuple(str(item) for item in payload.get("evidence_paths", [])),
            notes=tuple(str(item) for item in payload.get("notes", [])),
        )

    def merge(self, other: "FieldSemanticOverlay") -> "FieldSemanticOverlay":
        if self.field_name != other.field_name:
            raise ValueError(f"Cannot merge distinct fields: {self.field_name!r} vs {other.field_name!r}")
        if self.authoring_label != other.authoring_label:
            raise ValueError(
                f"Conflicting semantic labels for {self.field_name}: {self.authoring_label!r} vs {other.authoring_label!r}"
            )
        provenance = self.provenance
        if _rank(other.provenance, PROVENANCE_RANK) > _rank(self.provenance, PROVENANCE_RANK):
            provenance = other.provenance
        confidence = self.confidence
        if _rank(other.confidence, CONFIDENCE_RANK) > _rank(self.confidence, CONFIDENCE_RANK):
            confidence = other.confidence
        return FieldSemanticOverlay(
            field_name=self.field_name,
            authoring_label=self.authoring_label,
            provenance=provenance,
            confidence=confidence,
            control_label=self.control_label or other.control_label,
            primitive_kind=self.primitive_kind or other.primitive_kind,
            dialog_id=self.dialog_id if self.dialog_id is not None else other.dialog_id,
            dialog_caption=self.dialog_caption or other.dialog_caption,
            evidence_paths=tuple(dict.fromkeys(self.evidence_paths + other.evidence_paths)),
            notes=tuple(dict.fromkeys(self.notes + other.notes)),
        )


@dataclass(slots=True)
class SubrecordSemanticOverlay:
    signature: str
    fields: dict[str, FieldSemanticOverlay] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "fields": {
                field_name: overlay.to_dict()
                for field_name, overlay in sorted(self.fields.items())
            }
        }

    @classmethod
    def from_dict(cls, signature: str, payload: dict[str, Any]) -> "SubrecordSemanticOverlay":
        return cls(
            signature=signature,
            fields={
                field_name: FieldSemanticOverlay.from_dict(field_name, field_payload)
                for field_name, field_payload in payload.get("fields", {}).items()
            },
        )


@dataclass(slots=True)
class RecordSemanticOverlay:
    signature: str
    subrecords: dict[str, SubrecordSemanticOverlay] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "subrecords": {
                subrecord_sig: overlay.to_dict()
                for subrecord_sig, overlay in sorted(self.subrecords.items())
            }
        }

    @classmethod
    def from_dict(cls, signature: str, payload: dict[str, Any]) -> "RecordSemanticOverlay":
        return cls(
            signature=signature,
            subrecords={
                subrecord_sig: SubrecordSemanticOverlay.from_dict(subrecord_sig, subrecord_payload)
                for subrecord_sig, subrecord_payload in payload.get("subrecords", {}).items()
            },
        )


@dataclass(slots=True)
class SemanticOverlay:
    game: str
    artifact_version: int = ARTIFACT_VERSION
    records: dict[str, RecordSemanticOverlay] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "artifact_version": self.artifact_version,
            "game": self.game,
            "records": {
                record_sig: overlay.to_dict()
                for record_sig, overlay in sorted(self.records.items())
            },
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SemanticOverlay":
        return cls(
            game=str(payload["game"]),
            artifact_version=int(payload.get("artifact_version", ARTIFACT_VERSION)),
            records={
                record_sig: RecordSemanticOverlay.from_dict(record_sig, record_payload)
                for record_sig, record_payload in payload.get("records", {}).items()
            },
        )

    def save(self, path: str | Path) -> Path:
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")
        return target

    @classmethod
    def load(cls, path: str | Path) -> "SemanticOverlay":
        return cls.from_dict(json.loads(Path(path).read_text(encoding="utf-8")))


def default_overlay_dir() -> Path:
    return Path(__file__).resolve().parent / "data"


def default_overlay_path(game: str, *, overlay_dir: str | Path | None = None) -> Path:
    base_dir = Path(overlay_dir) if overlay_dir is not None else default_overlay_dir()
    return base_dir / f"{game}.semantic_overlay.json"


def load_installed_overlay(game: str, *, overlay_dir: str | Path | None = None) -> "SemanticOverlay" | None:
    path = default_overlay_path(game, overlay_dir=overlay_dir)
    if not path.is_file():
        return None
    return SemanticOverlay.load(path)


def install_semantic_overlay(
    overlay: "SemanticOverlay",
    *,
    overlay_dir: str | Path | None = None,
) -> Path:
    return overlay.save(default_overlay_path(overlay.game, overlay_dir=overlay_dir))


def _single_promoted_candidate(
    evidence_payload: dict[str, Any],
    *,
    min_provenance: str,
    min_confidence: str,
) -> dict[str, Any] | None:
    provenance = str(evidence_payload.get("provenance", "editor_label_only"))
    candidates = evidence_payload.get("mapping_candidates", [])
    if not isinstance(candidates, list):
        return None
    qualified = [
        candidate
        for candidate in candidates
        if isinstance(candidate, dict)
        and isinstance(candidate.get("field_name"), str)
        and _qualifies(
            provenance=provenance,
            confidence=str(candidate.get("confidence", evidence_payload.get("confidence", "low"))),
            min_provenance=min_provenance,
            min_confidence=min_confidence,
        )
    ]
    unique = {
        str(candidate["field_name"]): candidate
        for candidate in qualified
    }
    if len(unique) != 1:
        return None
    return next(iter(unique.values()))


def build_semantic_overlays(
    evidence_payloads: list[dict[str, Any]],
    *,
    evidence_paths: list[str | Path | None] | None = None,
    min_provenance: str = "editor_diff_verified",
    min_confidence: str = "high",
) -> dict[str, SemanticOverlay]:
    if evidence_paths is None:
        evidence_path_items: list[str | Path | None] = [None] * len(evidence_payloads)
    else:
        evidence_path_items = evidence_paths
    if len(evidence_path_items) != len(evidence_payloads):
        raise ValueError("evidence_paths must match evidence_payloads length")

    overlays: dict[str, SemanticOverlay] = {}

    for evidence_payload, evidence_path in zip(evidence_payloads, evidence_path_items, strict=True):
        promoted = _single_promoted_candidate(
            evidence_payload,
            min_provenance=min_provenance,
            min_confidence=min_confidence,
        )
        if promoted is None:
            continue

        game = str(evidence_payload["game"])
        record_sig = str(evidence_payload["record_signature"])
        subrecord_sig = str(evidence_payload["subrecord_signature"])
        control = evidence_payload.get("control", {})
        if not isinstance(control, dict):
            control = {}
        notes = evidence_payload.get("notes", [])
        if not isinstance(notes, list):
            notes = []

        field_overlay = FieldSemanticOverlay(
            field_name=str(promoted["field_name"]),
            authoring_label=str(control.get("label") or promoted["field_name"]),
            provenance=str(evidence_payload.get("provenance", "editor_label_only")),
            confidence=str(promoted.get("confidence", evidence_payload.get("confidence", "low"))),
            control_label=str(control.get("label")) if control.get("label") is not None else None,
            primitive_kind=str(promoted.get("primitive_kind")) if promoted.get("primitive_kind") is not None else None,
            dialog_id=int(control["dialog_id"]) if control.get("dialog_id") is not None else None,
            dialog_caption=str(control["dialog_caption"]) if control.get("dialog_caption") is not None else None,
            evidence_paths=(str(Path(evidence_path)),) if evidence_path is not None else (),
            notes=tuple(str(item) for item in notes),
        )

        overlay = overlays.setdefault(game, SemanticOverlay(game=game))
        record_overlay = overlay.records.setdefault(record_sig, RecordSemanticOverlay(signature=record_sig))
        subrecord_overlay = record_overlay.subrecords.setdefault(
            subrecord_sig,
            SubrecordSemanticOverlay(signature=subrecord_sig),
        )
        existing = subrecord_overlay.fields.get(field_overlay.field_name)
        subrecord_overlay.fields[field_overlay.field_name] = (
            field_overlay if existing is None else existing.merge(field_overlay)
        )

    return overlays


def _apply_field_overlays(
    fields: tuple[FieldSpec, ...] | None,
    overlays: dict[str, FieldSemanticOverlay],
    *,
    min_provenance: str,
    min_confidence: str,
) -> tuple[FieldSpec, ...] | None:
    if not fields:
        return fields
    updated = list(fields)
    by_name = {field.name: index for index, field in enumerate(fields)}
    changed = False
    for field_name, overlay in overlays.items():
        index = by_name.get(field_name)
        if index is None:
            continue
        if not _qualifies(
            provenance=overlay.provenance,
            confidence=overlay.confidence,
            min_provenance=min_provenance,
            min_confidence=min_confidence,
        ):
            continue
        current = updated[index]
        if current.authoring_label == overlay.authoring_label:
            continue
        updated[index] = replace(current, authoring_label=overlay.authoring_label)
        changed = True
    return tuple(updated) if changed else fields


def apply_semantic_overlay(
    schema: GameSchema,
    overlay: SemanticOverlay,
    *,
    min_provenance: str = "editor_diff_verified",
    min_confidence: str = "high",
) -> GameSchema:
    if schema.game != overlay.game:
        raise ValueError(f"Overlay game {overlay.game!r} does not match schema game {schema.game!r}")

    updated_records = dict(schema.records)
    records_changed = False

    for record_sig, record_overlay in overlay.records.items():
        record_spec = updated_records.get(record_sig)
        if record_spec is None:
            continue
        updated_subrecords: list[SubrecordSpec] = []
        subrecords_changed = False
        for subrecord_spec in record_spec.subrecords:
            subrecord_overlay = record_overlay.subrecords.get(subrecord_spec.sig)
            if subrecord_overlay is None:
                updated_subrecords.append(subrecord_spec)
                continue
            updated_fields = _apply_field_overlays(
                subrecord_spec.fields,
                subrecord_overlay.fields,
                min_provenance=min_provenance,
                min_confidence=min_confidence,
            )
            if updated_fields is subrecord_spec.fields:
                updated_subrecords.append(subrecord_spec)
                continue
            updated_subrecords.append(replace(subrecord_spec, fields=updated_fields))
            subrecords_changed = True
        if subrecords_changed:
            updated_records[record_sig] = replace(record_spec, subrecords=tuple(updated_subrecords))
            records_changed = True

    if not records_changed:
        return schema
    return GameSchema(
        game=schema.game,
        records=updated_records,
        header_version=schema.header_version,
        localized_support=schema.localized_support,
        enums=dict(schema.enums),
    )


def collect_overlay_changes(
    schema: GameSchema,
    overlay: SemanticOverlay,
    *,
    min_provenance: str = "editor_diff_verified",
    min_confidence: str = "high",
) -> list[dict[str, Any]]:
    if schema.game != overlay.game:
        raise ValueError(f"Overlay game {overlay.game!r} does not match schema game {schema.game!r}")

    changes: list[dict[str, Any]] = []
    for record_sig, record_overlay in sorted(overlay.records.items()):
        record_spec = schema.records.get(record_sig)
        if record_spec is None:
            continue
        for subrecord_spec in record_spec.subrecords:
            subrecord_overlay = record_overlay.subrecords.get(subrecord_spec.sig)
            if subrecord_overlay is None or not subrecord_spec.fields:
                continue
            fields_by_name = {field.name: field for field in subrecord_spec.fields}
            for field_name, field_overlay in sorted(subrecord_overlay.fields.items()):
                if not _qualifies(
                    provenance=field_overlay.provenance,
                    confidence=field_overlay.confidence,
                    min_provenance=min_provenance,
                    min_confidence=min_confidence,
                ):
                    continue
                field_spec = fields_by_name.get(field_name)
                if field_spec is None or field_spec.authoring_label == field_overlay.authoring_label:
                    continue
                changes.append(
                    {
                        "record_signature": record_sig,
                        "subrecord_signature": subrecord_spec.sig,
                        "field_name": field_name,
                        "before_label": field_spec.authoring_label,
                        "after_label": field_overlay.authoring_label,
                        "provenance": field_overlay.provenance,
                        "confidence": field_overlay.confidence,
                    }
                )
    return changes
