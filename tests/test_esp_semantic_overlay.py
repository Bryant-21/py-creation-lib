from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.esp.schema import (
    SemanticOverlay,
    apply_semantic_overlay,
    build_semantic_overlays,
    collect_overlay_changes,
    get_schema,
    install_semantic_overlay,
    load_installed_overlay,
)


def _evidence_payload(
    *,
    label: str = "Relaunch Interval",
    field_name: str = "relaunch_interval",
    confidence: str = "high",
    provenance: str = "editor_diff_verified",
) -> dict:
    return {
        "artifact_version": 1,
        "provenance": provenance,
        "game": "fo4",
        "record_signature": "PROJ",
        "subrecord_signature": "DNAM",
        "record_selector": {
            "editor_id": "ProbeProjectile",
            "form_id": None,
        },
        "control": {
            "target_key": "projectile",
            "label": label,
            "dialog_id": 3262,
            "dialog_caption": "Projectile",
            "dialog_start_line": 8174,
            "matched_target_labels": [label],
            "candidate_controls": [{"label": label, "control_id": "-1"}],
        },
        "mapping_candidates": [
            {
                "field_name": field_name,
                "primitive_kind": "float32",
                "confidence": confidence,
                "field_span_start": 76,
                "field_span_end": 79,
                "byte_width": 4,
            }
        ],
        "confidence": confidence,
        "notes": ["verified in CK"],
    }


def _field_label(schema, *, record_sig: str, subrecord_sig: str, field_name: str) -> str | None:
    record = schema.records[record_sig]
    subrecord = next(spec for spec in record.subrecords if spec.sig == subrecord_sig)
    field = next(field for field in subrecord.fields or () if field.name == field_name)
    return field.authoring_label


def test_build_semantic_overlays_promotes_verified_evidence(tmp_path: Path) -> None:
    evidence_path = tmp_path / "evidence.json"
    evidence_path.write_text("{}", encoding="utf-8")

    overlays = build_semantic_overlays([_evidence_payload()], evidence_paths=[evidence_path])

    overlay = overlays["fo4"]
    field_overlay = overlay.records["PROJ"].subrecords["DNAM"].fields["relaunch_interval"]

    assert field_overlay.authoring_label == "Relaunch Interval"
    assert field_overlay.dialog_caption == "Projectile"
    assert field_overlay.evidence_paths == (str(evidence_path),)

    output_path = tmp_path / "fo4.semantic_overlay.json"
    overlay.save(output_path)
    loaded = SemanticOverlay.load(output_path)

    assert loaded.to_dict() == overlay.to_dict()


def test_build_semantic_overlays_skips_unqualified_evidence() -> None:
    overlays = build_semantic_overlays(
        [_evidence_payload(confidence="medium"), _evidence_payload(provenance="editor_label_only")],
    )

    assert overlays == {}


def test_build_semantic_overlays_raises_on_conflicting_labels() -> None:
    with pytest.raises(ValueError, match="Conflicting semantic labels"):
        build_semantic_overlays(
            [
                _evidence_payload(label="Relaunch Interval"),
                _evidence_payload(label="Launch Delay"),
            ]
        )


def test_apply_semantic_overlay_promotes_verified_authoring_label() -> None:
    base_schema = get_schema("fo4")
    assert _field_label(base_schema, record_sig="PROJ", subrecord_sig="DNAM", field_name="relaunch_interval") == (
        "Relaunch Interval"
    )

    overlay = build_semantic_overlays([_evidence_payload(label="Verified Relaunch Interval")])["fo4"]
    updated_schema = apply_semantic_overlay(base_schema, overlay)

    assert _field_label(updated_schema, record_sig="PROJ", subrecord_sig="DNAM", field_name="relaunch_interval") == (
        "Verified Relaunch Interval"
    )
    assert _field_label(base_schema, record_sig="PROJ", subrecord_sig="DNAM", field_name="relaunch_interval") == (
        "Relaunch Interval"
    )


def test_apply_semantic_overlay_rejects_game_mismatch() -> None:
    base_schema = get_schema("fo4")
    overlay = SemanticOverlay(game="skyrimse")

    with pytest.raises(ValueError, match="does not match schema game"):
        apply_semantic_overlay(base_schema, overlay)


def test_collect_overlay_changes_reports_label_promotions() -> None:
    base_schema = get_schema("fo4")
    overlay = build_semantic_overlays([_evidence_payload(label="Verified Relaunch Interval")])["fo4"]

    changes = collect_overlay_changes(base_schema, overlay)

    assert changes == [
        {
            "record_signature": "PROJ",
            "subrecord_signature": "DNAM",
            "field_name": "relaunch_interval",
            "before_label": "Relaunch Interval",
            "after_label": "Verified Relaunch Interval",
            "provenance": "editor_diff_verified",
            "confidence": "high",
        }
    ]


def test_install_and_load_installed_overlay(tmp_path: Path) -> None:
    overlay = build_semantic_overlays([_evidence_payload()])["fo4"]

    installed_path = install_semantic_overlay(overlay, overlay_dir=tmp_path)
    loaded = load_installed_overlay("fo4", overlay_dir=tmp_path)

    assert installed_path == tmp_path / "fo4.semantic_overlay.json"
    assert loaded is not None
    assert loaded.to_dict() == overlay.to_dict()


def test_get_schema_applies_installed_overlay_when_present(tmp_path: Path) -> None:
    overlay = build_semantic_overlays([_evidence_payload()])["fo4"]
    install_semantic_overlay(overlay, overlay_dir=tmp_path)

    updated_schema = get_schema("fo4", overlay_dir=tmp_path)
    neutral_schema = get_schema("fo4", apply_overlays=False, overlay_dir=tmp_path)

    assert _field_label(updated_schema, record_sig="PROJ", subrecord_sig="DNAM", field_name="relaunch_interval") == (
        "Relaunch Interval"
    )
    assert _field_label(neutral_schema, record_sig="PROJ", subrecord_sig="DNAM", field_name="relaunch_interval") == (
        "Relaunch Interval"
    )
