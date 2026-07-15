"""Report-only collision validation pass over converted NIFs.

Walks NIFs under a meshes root, extracts each bhkPhysicsSystem/bhkRagdollSystem
blob, runs the native validator, and aggregates findings. NEVER raises on a bad
blob — a parse failure becomes a finding. NEVER fails the caller.

Parallelism: pass ``workers`` to fan NIF parsing + validation across worker
processes (mirrors ``scripts/dump_nif_collision_types.py``). ``workers=1`` runs
in-process (no pool) for tests and small roots.
"""

from __future__ import annotations

import json
import os
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import Any

from creation_lib.havok.native_runtime import validate_collision_blob_native
from creation_lib.nif.nif_file import NifFile

COLLISION_BLOCK_TYPES = {"bhkPhysicsSystem", "bhkRagdollSystem"}

_DEFAULT_INVARIANTS = (
    Path(__file__).resolve().parent / "resources" / "collision_invariants.json"
)

# Set once per worker process via the pool initializer so the (small) invariants
# JSON is not re-pickled for every one of tens of thousands of NIF jobs.
_WORKER_INVARIANTS: str = "{}"


def _blob_from_block(block: Any) -> bytes:
    binary = block.get_field("Binary Data") if hasattr(block, "get_field") else None
    raw = binary.get("Data") if isinstance(binary, dict) else None
    if isinstance(raw, (bytes, bytearray)):
        return bytes(raw)
    if isinstance(raw, list):
        return bytes(raw)
    return b""


def load_invariants_json(path: Path | None = None) -> str:
    target = path or _DEFAULT_INVARIANTS
    if target.is_file():
        return target.read_text(encoding="utf-8")
    return "{}"


def default_worker_count() -> int:
    return max(1, (os.cpu_count() or 2) // 2)


def summarize_violations(findings: list[dict[str, Any]]) -> dict[str, Any]:
    by_rule: Counter[str] = Counter()
    errors = 0
    warnings = 0
    for finding in findings:
        by_rule[str(finding.get("rule_id"))] += 1
        if finding.get("severity") == "error":
            errors += 1
        elif finding.get("severity") == "warning":
            warnings += 1
    return {"errors": errors, "warnings": warnings, "by_rule": dict(by_rule)}


def _worker_init(invariants_json: str) -> None:
    global _WORKER_INVARIANTS
    _WORKER_INVARIANTS = invariants_json


def _validate_one_nif(job: tuple[str, str]) -> tuple[list[dict[str, Any]], int]:
    """Validate one NIF. Returns (findings, blob_count). NEVER raises."""
    root_str, path_str = job
    findings: list[dict[str, Any]] = []
    blob_count = 0
    try:
        rel = Path(path_str).relative_to(root_str).as_posix()
    except ValueError:
        rel = path_str

    try:
        nif = NifFile.load(path_str)
    except Exception as exc:  # parse failure → finding, never raise
        return ([{
            "nif": rel, "rule_id": "nif_parse_error",
            "severity": "error", "message": str(exc),
        }], 0)

    for block_index, block in enumerate(nif.blocks):
        if getattr(block, "type_name", "") not in COLLISION_BLOCK_TYPES:
            continue
        blob = _blob_from_block(block)
        if not blob:
            continue
        blob_count += 1
        try:
            raw = validate_collision_blob_native(blob, _WORKER_INVARIANTS)
            for violation in json.loads(raw):
                violation["nif"] = rel
                violation["block_index"] = block_index
                findings.append(violation)
        except Exception as exc:  # validator/parse failure → finding
            findings.append({
                "nif": rel, "block_index": block_index,
                "rule_id": "validator_error", "severity": "error",
                "message": str(exc),
            })
    return (findings, blob_count)


def validate_collision_root(
    meshes_root: Path,
    invariants_json: str | None = None,
    report_dir: Path | None = None,
    workers: int | None = None,
) -> dict[str, Any]:
    invariants_json = invariants_json or load_invariants_json()
    root = Path(meshes_root)
    workers = default_worker_count() if workers is None else max(1, int(workers))

    jobs = [
        (str(root), str(path))
        for path in sorted(root.rglob("*.nif"))
        if path.is_file()
    ]
    nif_count = len(jobs)
    findings: list[dict[str, Any]] = []
    blob_count = 0

    if workers == 1 or nif_count <= 1:
        _worker_init(invariants_json)
        for job in jobs:
            job_findings, job_blobs = _validate_one_nif(job)
            findings.extend(job_findings)
            blob_count += job_blobs
    else:
        with ProcessPoolExecutor(
            max_workers=workers,
            initializer=_worker_init,
            initargs=(invariants_json,),
        ) as executor:
            for job_findings, job_blobs in executor.map(
                _validate_one_nif, jobs, chunksize=32
            ):
                findings.extend(job_findings)
                blob_count += job_blobs

    summary = summarize_violations(findings)
    summary.update({"nifs": nif_count, "blobs": blob_count, "workers": workers})

    if report_dir is not None:
        report_dir = Path(report_dir)
        report_dir.mkdir(parents=True, exist_ok=True)
        with (report_dir / "report.jsonl").open("w", encoding="utf-8") as handle:
            for finding in findings:
                handle.write(json.dumps(finding, sort_keys=True) + "\n")
        (report_dir / "summary.txt").write_text(
            json.dumps(summary, indent=2), encoding="utf-8"
        )

    return summary
