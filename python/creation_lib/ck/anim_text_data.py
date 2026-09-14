from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


@dataclass(frozen=True)
class CreatureAnimTextTarget:
    graph_path: str
    clip_name: str
    annotation: str


@dataclass(frozen=True)
class CreatureAttackEventReceipt:
    event: str
    race_form_key: str
    atkd_index: int
    source: str
    targets: tuple[CreatureAnimTextTarget, ...]


@dataclass(frozen=True)
class CreatureAnimTextFamilyReceipt:
    family_id: str
    status: str
    race_form_keys: tuple[str, ...]
    graph_paths: tuple[str, ...]
    emitted_files: tuple[str, ...]
    attack_events: tuple[CreatureAttackEventReceipt, ...]


@dataclass(frozen=True)
class CreatureAnimTextClosureReceipt:
    version: int
    policy_id: str
    written: int
    families: tuple[CreatureAnimTextFamilyReceipt, ...]

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": self.version,
            "policy_id": self.policy_id,
            "written": self.written,
            "families": [
                {
                    "family_id": family.family_id,
                    "status": family.status,
                    "race_form_keys": list(family.race_form_keys),
                    "graph_paths": list(family.graph_paths),
                    "emitted_files": list(family.emitted_files),
                    "attack_events": [
                        {
                            "event": event.event,
                            "race_form_key": event.race_form_key,
                            "atkd_index": event.atkd_index,
                            "source": event.source,
                            "targets": [
                                {
                                    "graph_path": target.graph_path,
                                    "clip_name": target.clip_name,
                                    "annotation": target.annotation,
                                }
                                for target in event.targets
                            ],
                        }
                        for event in family.attack_events
                    ],
                }
                for family in self.families
            ],
        }


def _nonempty_strings(value: Any, field: str) -> tuple[str, ...]:
    if not isinstance(value, (list, tuple)):
        raise RuntimeError(f"creature AnimText receipt {field} must be a list")
    strings = tuple(str(item).strip() for item in value)
    if not strings or any(not item for item in strings):
        raise RuntimeError(f"creature AnimText receipt {field} must not be empty")
    if len({item.casefold() for item in strings}) != len(strings):
        raise RuntimeError(f"creature AnimText receipt {field} contains duplicates")
    return strings


def _creature_anim_text_receipt(
    raw: Any,
    *,
    policy_id: str,
    expected_family_ids: tuple[str, ...],
    expected_attacks: Mapping[str, Mapping[str, frozenset[str]]],
    expected_graphs: Mapping[str, frozenset[str]],
) -> CreatureAnimTextClosureReceipt:
    if isinstance(raw, (str, bytes, bytearray)):
        raw = json.loads(raw)
    if not isinstance(raw, Mapping):
        raise RuntimeError("creature AnimText native receipt must be an object")
    if raw.get("version") != 1:
        raise RuntimeError("unsupported creature AnimText receipt version")
    if str(raw.get("policy_id") or "") != policy_id:
        raise RuntimeError(
            "creature AnimText receipt policy does not match its contract"
        )

    raw_families = raw.get("families")
    if not isinstance(raw_families, list):
        raise RuntimeError("creature AnimText receipt families must be a list")
    families: list[CreatureAnimTextFamilyReceipt] = []
    emitted_file_keys: set[str] = set()
    for raw_family in raw_families:
        if not isinstance(raw_family, Mapping):
            raise RuntimeError("creature AnimText family receipt must be an object")
        family_id = str(raw_family.get("family_id") or "").strip()
        if not family_id:
            raise RuntimeError("creature AnimText family receipt is missing family_id")
        if str(raw_family.get("status") or "") != "passed":
            raise RuntimeError(f"creature AnimText family {family_id} did not pass")
        race_form_keys = _nonempty_strings(
            raw_family.get("race_form_keys"), f"{family_id}.race_form_keys"
        )
        expected_races = expected_attacks.get(family_id.casefold())
        if expected_races is None or {
            value.casefold() for value in race_form_keys
        } != set(expected_races):
            raise RuntimeError(
                f"creature AnimText family {family_id} RACE receipt does not match "
                "the committed record batch"
            )
        has_expected_attacks = any(expected_races.values())
        graph_paths = _nonempty_strings(
            raw_family.get("graph_paths"), f"{family_id}.graph_paths"
        )
        if {path.casefold() for path in graph_paths} != expected_graphs.get(
            family_id.casefold(), frozenset()
        ):
            raise RuntimeError(
                f"creature AnimText family {family_id} graphs do not match the "
                "staged corpus plan"
            )
        emitted_files = _nonempty_strings(
            raw_family.get("emitted_files"), f"{family_id}.emitted_files"
        )
        emitted_buckets: set[str] = set()
        for emitted_file in emitted_files:
            normalized_file = emitted_file.replace("\\", "/")
            parts = normalized_file.split("/")
            if (
                parts[0].casefold() != "animtextdata"
                or any(part in {"", ".", ".."} for part in parts)
                or not normalized_file.casefold().endswith(".txt")
            ):
                raise RuntimeError(
                    f"creature AnimText family {family_id} emitted an invalid target path"
                )
            if len(parts) > 2:
                emitted_buckets.add(parts[1].casefold())
            emitted_file_keys.add(normalized_file.casefold())
        if "animationfiledata" not in emitted_buckets or (
            has_expected_attacks and "animeventinfo" not in emitted_buckets
        ):
            raise RuntimeError(
                f"creature AnimText family {family_id} is missing required graph or Actor Actions data"
            )
        raw_events = raw_family.get("attack_events")
        if not isinstance(raw_events, list):
            raise RuntimeError(
                f"creature AnimText family {family_id} attack events must be a list"
            )
        race_keys = {value.casefold() for value in race_form_keys}
        family_graphs = {value.casefold() for value in graph_paths}
        attack_events: list[CreatureAttackEventReceipt] = []
        resolved_attacks: set[tuple[str, str]] = set()
        for raw_event in raw_events:
            if not isinstance(raw_event, Mapping):
                raise RuntimeError(
                    f"creature AnimText family {family_id} attack receipt must be an object"
                )
            event = str(raw_event.get("event") or "").strip()
            race_form_key = str(raw_event.get("race_form_key") or "").strip()
            atkd_index = raw_event.get("atkd_index")
            source = str(raw_event.get("source") or "")
            if not event or race_form_key.casefold() not in race_keys:
                raise RuntimeError(
                    f"creature AnimText family {family_id} has an unowned attack event"
                )
            if (
                isinstance(atkd_index, bool)
                or not isinstance(atkd_index, int)
                or atkd_index < 0
            ):
                raise RuntimeError(
                    f"creature AnimText family {family_id} attack {event} has invalid ATKD index"
                )
            if source != "emitted_race_atkd_atke":
                raise RuntimeError(
                    f"creature AnimText family {family_id} attack {event} did not come from emitted RACE ATKD/ATKE"
                )
            attack_identity = (race_form_key.casefold(), event.casefold())
            if (
                event.casefold()
                not in expected_races.get(race_form_key.casefold(), frozenset())
                or attack_identity in resolved_attacks
            ):
                raise RuntimeError(
                    f"creature AnimText family {family_id} attack {event} does not "
                    "match its committed RACE receipt"
                )
            resolved_attacks.add(attack_identity)
            raw_targets = raw_event.get("targets")
            if not isinstance(raw_targets, list) or not raw_targets:
                raise RuntimeError(
                    f"creature AnimText family {family_id} attack {event} has no graph target"
                )
            targets: list[CreatureAnimTextTarget] = []
            for raw_target in raw_targets:
                if not isinstance(raw_target, Mapping):
                    raise RuntimeError(
                        f"creature AnimText family {family_id} attack {event} target must be an object"
                    )
                graph_path = str(raw_target.get("graph_path") or "").strip()
                clip_name = str(raw_target.get("clip_name") or "").strip()
                annotation = str(raw_target.get("annotation") or "").strip()
                if graph_path.casefold() not in family_graphs or not (
                    clip_name or annotation
                ):
                    raise RuntimeError(
                        f"creature AnimText family {family_id} attack {event} has an unresolved clip/annotation"
                    )
                targets.append(
                    CreatureAnimTextTarget(
                        graph_path=graph_path,
                        clip_name=clip_name,
                        annotation=annotation,
                    )
                )
            attack_events.append(
                CreatureAttackEventReceipt(
                    event=event,
                    race_form_key=race_form_key,
                    atkd_index=atkd_index,
                    source=source,
                    targets=tuple(targets),
                )
            )
        expected_attack_identities = {
            (race_key, event)
            for race_key, events in expected_races.items()
            for event in events
        }
        if resolved_attacks != expected_attack_identities:
            raise RuntimeError(
                f"creature AnimText family {family_id} did not resolve every "
                "committed RACE attack event"
            )
        families.append(
            CreatureAnimTextFamilyReceipt(
                family_id=family_id,
                status="passed",
                race_form_keys=race_form_keys,
                graph_paths=graph_paths,
                emitted_files=emitted_files,
                attack_events=tuple(attack_events),
            )
        )

    actual_family_ids = tuple(family.family_id for family in families)
    if len({family.casefold() for family in actual_family_ids}) != len(
        actual_family_ids
    ):
        raise RuntimeError("creature AnimText receipt contains duplicate families")
    if {family.casefold() for family in actual_family_ids} != {
        family.casefold() for family in expected_family_ids
    }:
        raise RuntimeError(
            "creature AnimText receipt does not account for every published family"
        )
    written = raw.get("written")
    if (
        isinstance(written, bool)
        or not isinstance(written, int)
        or written < 1
        or written != len(emitted_file_keys)
    ):
        raise RuntimeError(
            "creature AnimText receipt file accounting does not reconcile"
        )
    return CreatureAnimTextClosureReceipt(
        version=1,
        policy_id=policy_id,
        written=written,
        families=tuple(families),
    )


def _record_commit_attack_contract(
    record_commit_ledger_path: str | Path,
    expected_family_ids: tuple[str, ...],
) -> Mapping[str, Mapping[str, frozenset[str]]]:
    path = Path(record_commit_ledger_path)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(
            f"could not read creature record-commit receipt {path}: {exc}"
        ) from exc
    if not isinstance(payload, Mapping):
        raise RuntimeError("creature record-commit receipt must be an object")
    if "receipt" in payload:
        if payload.get("version") != 2:
            raise RuntimeError(
                "creature record-commit ledger has an unsupported version"
            )
        receipt = payload.get("receipt")
        if not isinstance(receipt, Mapping):
            raise RuntimeError("creature record-commit ledger receipt must be an object")
        payload = receipt
    raw_families = payload.get("families")
    if not isinstance(raw_families, list):
        raise RuntimeError("creature record-commit receipt families must be a list")
    expected = {family.casefold() for family in expected_family_ids}
    attacks: dict[str, dict[str, frozenset[str]]] = {}
    for raw_family in raw_families:
        if not isinstance(raw_family, Mapping):
            raise RuntimeError("creature record-commit family must be an object")
        family_id = str(raw_family.get("family_id") or "").strip().casefold()
        if not family_id or family_id not in expected or family_id in attacks:
            raise RuntimeError(
                "creature record-commit families do not match the AnimText contract"
            )
        raw_races = raw_family.get("races")
        if not isinstance(raw_races, list) or not raw_races:
            raise RuntimeError(
                f"creature record-commit family {family_id} has no RACE receipt"
            )
        races: dict[str, frozenset[str]] = {}
        for raw_race in raw_races:
            if not isinstance(raw_race, Mapping):
                raise RuntimeError("creature record-commit RACE must be an object")
            form_key = str(raw_race.get("form_key") or "").strip().casefold()
            raw_events = raw_race.get("attack_events")
            if not form_key or form_key in races or not isinstance(raw_events, list):
                raise RuntimeError("creature record-commit RACE receipt is invalid")
            events = frozenset(str(event).strip().casefold() for event in raw_events)
            if len(events) != len(raw_events) or "" in events:
                raise RuntimeError(
                    "creature record-commit RACE attack receipt is incomplete"
                )
            races[form_key] = events
        attacks[family_id] = races
    if set(attacks) != expected:
        raise RuntimeError(
            "creature record-commit receipt does not account for every family"
        )
    return attacks


def _corpus_plan_graph_contract(
    corpus_plan_path: str | Path,
    expected_family_ids: tuple[str, ...],
) -> Mapping[str, frozenset[str]]:
    path = Path(corpus_plan_path)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise RuntimeError(
            f"could not read creature corpus plan {path}: {exc}"
        ) from exc
    if not isinstance(payload, Mapping) or not isinstance(payload.get("jobs"), list):
        raise RuntimeError("creature corpus plan jobs must be a list")
    expected = {family.casefold() for family in expected_family_ids}
    graphs: dict[str, set[str]] = {family: set() for family in expected}
    for raw_job in payload["jobs"]:
        if not isinstance(raw_job, Mapping):
            raise RuntimeError("creature corpus job must be an object")
        family_id = str(raw_job.get("family_id") or "").strip().casefold()
        if family_id not in graphs:
            raise RuntimeError(
                "creature corpus plan families do not match the AnimText contract"
            )
        graph_paths = raw_job.get("graph_paths")
        if graph_paths is None:
            artifacts = raw_job.get("artifacts")
            if not isinstance(artifacts, list):
                raise RuntimeError("creature corpus job artifacts must be a list")
            graph_paths = [
                artifact.get("target_path")
                for artifact in artifacts
                if isinstance(artifact, Mapping)
                and artifact.get("kind")
                in {"character_hkx", "root_behavior_hkx", "core_behavior_hkx"}
            ]
        if not isinstance(graph_paths, list):
            raise RuntimeError("creature corpus job graph_paths must be a list")
        for graph_path in graph_paths:
            target_path = str(graph_path or "").strip()
            if not target_path or not target_path.casefold().endswith(".hkx"):
                raise RuntimeError("creature corpus graph artifact is invalid")
            graphs[family_id].add(target_path.casefold())
    if any(not paths for paths in graphs.values()):
        raise RuntimeError(
            "creature corpus plan has no staged graph for an AnimText family"
        )
    return {family: frozenset(paths) for family, paths in graphs.items()}


def generate_anim_text_data(
    plugin_path: str | Path,
    *,
    game: str,
    source_meshes_root: str | Path,
    output_meshes_root: str | Path,
    base_meshes_root: str | Path | None = None,
    base_plugin_paths: Sequence[str | Path] = (),
    mod_prefix: str | None = None,
    progress_callback=None,
    workers: int | None = None,
) -> int:
    from creation_lib._native import ck_native

    args = (
        str(plugin_path),
        game,
        str(source_meshes_root),
        str(output_meshes_root),
        str(base_meshes_root) if base_meshes_root is not None else None,
        [str(path) for path in base_plugin_paths],
        mod_prefix,
        progress_callback,
    )
    if workers is None:
        return ck_native.ck_generate_anim_text_data(*args)
    return ck_native.ck_generate_anim_text_data(*args, workers=workers)


def generate_creature_anim_text_closure(
    plugin_path: str | Path,
    *,
    game: str,
    source_meshes_root: str | Path,
    output_meshes_root: str | Path,
    corpus_plan_path: str | Path,
    execution_ledger_path: str | Path,
    record_commit_ledger_path: str | Path,
    expected_family_ids: Sequence[str],
    policy_id: str,
    base_meshes_root: str | Path | None = None,
    base_plugin_paths: Sequence[str | Path] = (),
    mod_prefix: str | None = None,
    progress_callback=None,
    workers: int | None = None,
) -> CreatureAnimTextClosureReceipt:
    from creation_lib._native import ck_native

    expected = tuple(str(family).strip() for family in expected_family_ids)
    if not expected or any(not family for family in expected):
        raise RuntimeError("creature AnimText contract requires published family IDs")
    if len({family.casefold() for family in expected}) != len(expected):
        raise RuntimeError("creature AnimText contract contains duplicate families")
    native_generate = getattr(ck_native, "ck_generate_creature_anim_text_closure", None)
    if not callable(native_generate):
        raise RuntimeError(
            "all_creatures_v1 requires the missing family-local CK native seam "
            "ck_generate_creature_anim_text_closure; it must decode emitted RACE "
            "ATKD/ATKE and resolve each attack through the emitted family graphs"
        )
    expected_attacks = _record_commit_attack_contract(
        record_commit_ledger_path, expected
    )
    expected_graphs = _corpus_plan_graph_contract(corpus_plan_path, expected)
    contract = {
        "version": 1,
        "policy_id": policy_id,
        "game": game,
        "plugin_path": str(plugin_path),
        "source_meshes_root": str(source_meshes_root),
        "output_meshes_root": str(output_meshes_root),
        "base_meshes_root": (
            str(base_meshes_root) if base_meshes_root is not None else None
        ),
        "base_plugin_paths": [str(path) for path in base_plugin_paths],
        "corpus_plan_path": str(corpus_plan_path),
        "execution_ledger_path": str(execution_ledger_path),
        "record_commit_ledger_path": str(record_commit_ledger_path),
        "expected_family_ids": list(expected),
        "event_source": "emitted_race_atkd_atke",
        "resolution_source": "emitted_family_graphs",
        "mod_prefix": mod_prefix,
    }
    args = (json.dumps(contract, sort_keys=True), progress_callback)
    raw = (
        native_generate(*args)
        if workers is None
        else native_generate(*args, workers=workers)
    )
    return _creature_anim_text_receipt(
        raw,
        policy_id=policy_id,
        expected_family_ids=expected,
        expected_attacks=expected_attacks,
        expected_graphs=expected_graphs,
    )
