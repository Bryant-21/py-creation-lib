"""Observed-schema extraction from official base plugin corpora."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Iterable

from creation_lib.esp.native_runtime import (
    plugin_handle_call,
    plugin_handle_close,
    plugin_handle_load,
)
from creation_lib.esp.schema.manifest import (
    RecordMember,
    RecordObservation,
    SchemaManifest,
    SubrecordObservation,
    XEditRecordSchema,
)



OFFICIAL_PLUGIN_ALLOWLISTS: dict[str, list[str]] = {
    "oblivion": [
        "Oblivion.esm",
        "DLCShiveringIsles.esp",
        "Knights.esp",
        "DLCBattlehornCastle.esp",
        "DLCFrostcrag.esp",
        "DLCHorseArmor.esp",
        "DLCMehrunesRazor.esp",
        "DLCOrrery.esp",
        "DLCSpellTomes.esp",
        "DLCThievesDen.esp",
        "DLCVileLair.esp",
    ],
    "fo4": [
        "Fallout4.esm",
        "DLCCoast.esm",
        "DLCNukaWorld.esm",
        "DLCRobot.esm",
        "DLCworkshop01.esm",
        "DLCworkshop02.esm",
        "DLCworkshop03.esm",
    ],
    "fo3": [
        "Fallout3.esm",
        "Anchorage.esm",
        "ThePitt.esm",
        "BrokenSteel.esm",
        "PointLookout.esm",
        "Zeta.esm",
    ],
    "fnv": [
        "FalloutNV.esm",
        "CaravanPack.esm",
        "ClassicPack.esm",
        "DeadMoney.esm",
        "GunRunnersArsenal.esm",
        "HonestHearts.esm",
        "LonesomeRoad.esm",
        "MercenaryPack.esm",
        "OldWorldBlues.esm",
        "TribalPack.esm",
    ],
    "skyrimse": [
        "Skyrim.esm",
        "Update.esm",
        "Dawnguard.esm",
        "HearthFires.esm",
        "Dragonborn.esm",
        "_ResourcePack.esl",
    ],
    "fo76": [
        "SeventySix.esm",
        "NW.esm",
    ],
    "starfield": [
        "Starfield.esm",
        "OldMars.esm",
        "BlueprintShips-Starfield.esm",
        "SFBGS003.esm",
        "SFBGS004.esm",
        "SFBGS006.esm",
        "SFBGS007.esm",
        "SFBGS008.esm",
        "SFBGS00D.esm",
        "SFBGS047.esm",
    ],
}

_GAME_ALIASES = {
    "tes4": "oblivion",
}


def get_official_allowlist(game: str) -> list[str]:
    canonical = _GAME_ALIASES.get(game.strip().lower(), game.strip().lower())
    return list(OFFICIAL_PLUGIN_ALLOWLISTS.get(canonical, []))


def iter_official_plugin_paths(
    game: str,
    data_dir: str | Path,
    *,
    allowlist: Iterable[str] | None = None,
) -> list[Path]:
    base_dir = Path(data_dir)
    names = list(allowlist or get_official_allowlist(game))
    return [base_dir / name for name in names if (base_dir / name).exists()]


def _children(payload: dict[str, Any]) -> list[Any]:
    return list(payload.get("children") or payload.get("items") or [])


def _payload_int(value: Any) -> int:
    if isinstance(value, int):
        return value
    text = str(value)
    return int(text, 16) if any(c in text for c in "ABCDEFabcdef") else int(text)


def _subrecord_size(payload: dict[str, Any]) -> int:
    if "size" in payload:
        return int(payload["size"])
    if "data_hex" in payload:
        return len(bytes.fromhex(str(payload["data_hex"])))
    data = payload.get("data")
    return len(data) if isinstance(data, (bytes, bytearray)) else 0


def _observe_record_payload(
    manifest: SchemaManifest,
    payload: dict[str, Any],
    *,
    group_type: int | None = None,
) -> None:
    signature = str(payload.get("signature", ""))
    if not signature:
        return
    observation = manifest.records.setdefault(signature, RecordObservation(signature))
    subrecords = [item for item in payload.get("subrecords", []) or [] if isinstance(item, dict)]
    sizes = [_subrecord_size(subrecord) for subrecord in subrecords]
    payload_size = sum(sizes)
    observation.count += 1
    observation.total_size += payload_size
    observation.min_size = payload_size if observation.min_size is None else min(observation.min_size, payload_size)
    observation.max_size = max(observation.max_size, payload_size)
    try:
        if _payload_int(payload.get("flags", 0)) & 0x00040000:
            observation.compressed_count += 1
    except ValueError:
        pass
    form_version = payload.get("form_version")
    if form_version is not None:
        key = str(form_version)
        observation.form_versions[key] = observation.form_versions.get(key, 0) + 1
    if group_type is not None:
        key = str(group_type)
        observation.group_types[key] = observation.group_types.get(key, 0) + 1

    occurrences: dict[str, list[int]] = {}
    sequence: list[str] = []
    for subrecord, size in zip(subrecords, sizes):
        sub_sig = str(subrecord.get("signature", ""))
        if not sub_sig:
            continue
        sequence.append(sub_sig)
        occurrences.setdefault(sub_sig, []).append(size)
    for sub_sig, observed_sizes in occurrences.items():
        observation.subrecords.setdefault(sub_sig, SubrecordObservation(sub_sig)).observe(observed_sizes)
    for left, right in zip(sequence, sequence[1:]):
        edge = f"{left}>{right}"
        observation.order_edges[edge] = observation.order_edges.get(edge, 0) + 1
    if sequence and sequence not in observation.examples and len(observation.examples) < 5:
        observation.examples.append(sequence)
    if sequence and not observation.corpus_order_hint:
        observation.corpus_order_hint = list(sequence)
    observation.refresh_derived()


def _observe_payload_item(
    manifest: SchemaManifest,
    payload: dict[str, Any],
    *,
    group_type: int | None = None,
) -> None:
    children = _children(payload)
    if children:
        next_group_type = payload.get("group_type", group_type)
        if next_group_type is not None:
            key = str(next_group_type)
            manifest.group_types[key] = manifest.group_types.get(key, 0) + 1
        for child in children:
            if isinstance(child, dict):
                _observe_payload_item(manifest, child, group_type=next_group_type)
        return
    _observe_record_payload(manifest, payload, group_type=group_type)


def _observe_plugin_text(manifest: SchemaManifest, handle: int) -> None:
    payload = json.loads(plugin_handle_call(handle, "export_plugin_text", "lossless", "json"))
    for item in _children(payload):
        if isinstance(item, dict):
            _observe_payload_item(manifest, item)


def _record_schema_from_hints(record_signature: str, xedit_hints: dict[str, Any]) -> XEditRecordSchema:
    return XEditRecordSchema(
        kind=xedit_hints.get("record_kinds", {}).get(record_signature),
        members=[RecordMember.from_dict(item) for item in xedit_hints.get("record_members", {}).get(record_signature, [])],
    )


def _apply_layers(
    manifest: SchemaManifest,
    *,
    xedit_hints: dict[str, Any] | None = None,
    manual_overrides: dict[str, Any] | None = None,
) -> None:
    xedit_hints = xedit_hints or {}
    record_names = dict(xedit_hints.get("record_names", {}))
    subrecord_names = dict(xedit_hints.get("subrecord_names", {}))
    formid_subrecords = set(xedit_hints.get("formid_subrecords", []))

    for record_signature, record_observation in manifest.records.items():
        record_observation.name = record_names.get(record_signature, record_observation.name)
        record_observation.apply_xedit_schema(_record_schema_from_hints(record_signature, xedit_hints))
        for subrecord_signature, subrecord_observation in record_observation.subrecords.items():
            subrecord_observation.name = subrecord_names.get(subrecord_signature, subrecord_observation.name)
            if subrecord_signature in formid_subrecords:
                subrecord_observation.known_formid = True
                subrecord_observation.xedit_formid = True
        record_observation.refresh_derived()

    if manual_overrides:
        manifest.manual_overrides = manual_overrides
        for signature, payload in manual_overrides.get("records", {}).items():
            observation = manifest.records.get(signature)
            if observation is None:
                continue
            for key, value in payload.items():
                if hasattr(observation, key):
                    setattr(observation, key, value)
            observation.refresh_derived()
        for signature, payload in manual_overrides.get("subrecords", {}).items():
            for observation in manifest.records.values():
                subrecord = observation.subrecords.get(signature)
                if subrecord is None:
                    continue
                for key, value in payload.items():
                    if hasattr(subrecord, key):
                        setattr(subrecord, key, value)
                observation.refresh_derived()

    observed_subrecords = {
        subrecord_signature
        for observation in manifest.records.values()
        for subrecord_signature in observation.subrecords
    }
    manifest.unknown_signatures["records"] = sorted(
        signature for signature in manifest.records if signature not in record_names
    )
    manifest.unknown_signatures["subrecords"] = sorted(
        signature for signature in observed_subrecords if signature not in subrecord_names
    )
    manifest.xedit_hints = xedit_hints


def build_corpus_manifest(
    game: str,
    data_dir: str | Path,
    *,
    allowlist: Iterable[str] | None = None,
    xedit_hints: dict[str, Any] | None = None,
    manual_overrides: dict[str, Any] | None = None,
) -> SchemaManifest:
    manifest = SchemaManifest(game=game)
    plugin_paths = iter_official_plugin_paths(game, data_dir, allowlist=allowlist)
    if not plugin_paths:
        manifest.notes.append(f"No official plugins found under {Path(data_dir)}")
    for plugin_path in plugin_paths:
        manifest.plugins.append(plugin_path.name)
        handle: int | None = None
        try:
            handle = plugin_handle_load(str(plugin_path), game=game)
            _observe_plugin_text(manifest, int(handle))
        except Exception as exc:
            manifest.notes.append(f"Failed to parse {plugin_path.name}: {exc}")
        finally:
            if handle is not None:
                try:
                    plugin_handle_close(handle)
                except Exception:
                    pass
    _apply_layers(manifest, xedit_hints=xedit_hints, manual_overrides=manual_overrides)
    return manifest


def build_layered_manifest(
    game: str,
    data_dir: str | Path,
    *,
    allowlist: Iterable[str] | None = None,
    manual_overrides: dict[str, Any] | None = None,
) -> SchemaManifest:
    return build_corpus_manifest(
        game,
        data_dir,
        allowlist=allowlist,
        xedit_hints={},
        manual_overrides=manual_overrides,
    )


def schema_coverage_summary(manifest: SchemaManifest) -> dict[str, Any]:
    total_records = len(manifest.records)
    records_with_xedit = sum(1 for record in manifest.records.values() if record.xedit_order)
    complete_records = sum(1 for record in manifest.records.values() if record.is_complete)
    ref_records = sorted(signature for signature, record in manifest.records.items() if record.is_ref_record)
    union_records = sorted(
        signature
        for signature, record in manifest.records.items()
        if any(member.union for member in record.xedit.members)
    )
    raw_only_subrecords = sorted(
        f"{record.signature}.{subrecord.signature}"
        for record in manifest.records.values()
        for subrecord in record.subrecords.values()
        if not subrecord.xedit_builder
    )
    return {
        "game": manifest.game,
        "record_count": total_records,
        "records_with_xedit": records_with_xedit,
        "complete_records": complete_records,
        "record_coverage_pct": (records_with_xedit / total_records * 100.0) if total_records else 0.0,
        "complete_record_pct": (complete_records / total_records * 100.0) if total_records else 0.0,
        "ref_records": ref_records,
        "union_records": union_records,
        "raw_only_subrecords": raw_only_subrecords,
        "missing_record_signatures": list(manifest.unknown_signatures.get("records", [])),
        "missing_subrecord_signatures": list(manifest.unknown_signatures.get("subrecords", [])),
    }


def record_coverage_rows(manifest: SchemaManifest) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for signature, record in sorted(manifest.records.items()):
        rows.append(
            {
                "signature": signature,
                "name": record.name,
                "count": record.count,
                "kind": record.xedit.kind,
                "expected_subrecords": record.coverage["expected"],
                "observed_subrecords": record.coverage["observed"],
                "matched_subrecords": record.coverage["matched"],
                "missing_subrecords": record.coverage["missing"],
                "unexpected_subrecords": record.coverage["unexpected"],
                "order_overlap": record.order_overlap,
                "is_complete": record.is_complete,
                "is_ref_record": record.is_ref_record,
            }
        )
    return rows
