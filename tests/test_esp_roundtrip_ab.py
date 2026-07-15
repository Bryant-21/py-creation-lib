"""A/B roundtrip test for native ESP authoring.

For each selected plugin:
  1. Read original file size and record census
  2. Export round1 dataset in each requested native format
  3. Rebuild plugin from round1 dataset
  4. Export round2 dataset from rebuilt plugin
  5. Compare round1 vs round2 outputs, record drift, timings, and rebuilt size deltas

Usage:
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --game fo4
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --game fnv --esm OldWorldBlues.esm
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --game starfield --esm Starfield.esm
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --no-second-export
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --format both
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --game fo3 --jobs 12 --format yaml --skip-second-export
    uv run --reinstall-package modbox21-native tests/test_esp_roundtrip_ab.py --report data/esp_roundtrip_ab/custom_report.json
    # Every run writes JSON plus a sibling Markdown summary report under data/esp_roundtrip_ab by default.
    # When --game is supplied, default report and --clear scope are game-local.
"""

from __future__ import annotations

import argparse
import gc
import json
import os
import shutil
import struct
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent
if str(PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(PROJECT_ROOT))

from creation_lib.esp import Plugin, build_authoring_dir, export_authoring_dir
from creation_lib.esp.native_runtime import export_authoring_dir_native, load_native_module
from creation_lib.esp.schema import get_official_allowlist

GAMES = [
    {
        "id": "fo4",
        "display": "Fallout 4",
        "dir_var": "FO4_DIR",
        "esms": [
            "Fallout4.esm",
            "DLCCoast.esm",
            "DLCNukaWorld.esm",
            "DLCRobot.esm",
            "DLCworkshop01.esm",
            "DLCworkshop02.esm",
            "DLCworkshop03.esm",
        ],
    },
    {
        "id": "fo3",
        "display": "Fallout 3",
        "dir_var": "FO3_DIR",
        "esms": [
            "Fallout3.esm",
            "Anchorage.esm",
            "ThePitt.esm",
            "BrokenSteel.esm",
            "PointLookout.esm",
            "Zeta.esm",
        ],
    },
    {
        "id": "fnv",
        "display": "Fallout New Vegas",
        "dir_var": "FONV_DIR",
        "esms": [
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
    },
    {
        "id": "skyrimse",
        "display": "Skyrim Special Edition",
        "dir_var": "SKYRIMSE_DIR",
        "esms": [
            "Skyrim.esm",
            "Update.esm",
        ],
    },
    {
        "id": "fo76",
        "display": "Fallout 76",
        "dir_var": "FO76_DIR",
        "esms": [
            "SeventySix.esm",
            "NW.esm",
        ],
    },
    {
        "id": "starfield",
        "display": "Starfield",
        "dir_var": "STARFIELD_DIR",
        "esms": get_official_allowlist("starfield"),
    },
]

AB_PIPELINE_ORDER = ("native_json", "native_yaml")
PIPELINE_LABELS = {
    "native_json": "Native JSON",
    "native_yaml": "Native YAML",
}
STARFIELD_NATIVE_JOBS_CAP = 12


def load_env() -> dict[str, str]:
    env: dict[str, str] = {}
    env_file = PROJECT_ROOT / ".env"
    for line in env_file.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        key = key.strip()
        val = val.strip().strip('"').strip("'")
        if "$" in val:
            continue
        env[key] = val
    return env


def resolve_project_path(raw_path: str) -> Path:
    candidate = Path(raw_path.replace("/", "\\"))
    if candidate.is_absolute():
        return candidate
    return PROJECT_ROOT / str(candidate).lstrip("\\")


def fmt_bytes(n: int | None) -> str:
    if n is None:
        return "n/a"
    if abs(n) < 1024:
        return f"{n} B"
    if abs(n) < 1024**2:
        return f"{n / 1024:.1f} KB"
    return f"{n / 1024**2:.2f} MB"


def fmt_seconds(value: float | None) -> str:
    if value is None:
        return "n/a"
    return f"{value:.2f}s"


def fmt_ratio(value: float | None) -> str:
    if value is None:
        return "n/a"
    return f"{value:.2f}x"


def _safe_div(numerator: float | None, denominator: float | None) -> float | None:
    if numerator is None or denominator is None or denominator == 0:
        return None
    return numerator / denominator


def describe_speed(lhs_seconds: float | None, rhs_seconds: float | None, *, lhs_label: str, rhs_label: str) -> str:
    if lhs_seconds is None or rhs_seconds is None or lhs_seconds <= 0 or rhs_seconds <= 0:
        return "n/a"
    ratio = lhs_seconds / rhs_seconds
    if 0.95 <= ratio <= 1.05:
        return "about the same"
    if ratio < 1.0:
        return f"{lhs_label} faster ({rhs_seconds / lhs_seconds:.2f}x)"
    return f"{lhs_label} slower ({ratio:.2f}x)"


def resolve_native_jobs(game_id: str, requested_jobs: int | None) -> int | None:
    if game_id != "starfield":
        return requested_jobs
    if requested_jobs is None:
        return STARFIELD_NATIVE_JOBS_CAP
    return max(1, min(int(requested_jobs), STARFIELD_NATIVE_JOBS_CAP))


def collect_files(directory: Path) -> dict[str, int]:
    return {
        str(path.relative_to(directory)): path.stat().st_size
        for path in directory.rglob("*")
        if path.is_file()
    }


def collect_dir_stats(directory: Path) -> dict[str, Any]:
    if not directory.exists():
        return {
            "exists": False,
            "files": 0,
            "total_bytes": 0,
        }
    files = collect_files(directory)
    return {
        "exists": True,
        "files": len(files),
        "total_bytes": sum(files.values()),
    }


def compare_output_dirs(round1: Path, round2: Path) -> dict[str, Any]:
    files1 = collect_files(round1)
    files2 = collect_files(round2)

    only_in_round1 = sorted(set(files1) - set(files2))
    only_in_round2 = sorted(set(files2) - set(files1))
    common = sorted(set(files1) & set(files2))

    size_changes: list[tuple[str, int, int]] = []
    content_diffs: list[str] = []
    for rel_path in common:
        size1, size2 = files1[rel_path], files2[rel_path]
        if size1 != size2:
            size_changes.append((rel_path, size1, size2))
        if size1 != size2 or (round1 / rel_path).suffix.lower() in {".yaml", ".json"}:
            if (round1 / rel_path).read_bytes() != (round2 / rel_path).read_bytes():
                content_diffs.append(rel_path)

    return {
        "round1_files": len(files1),
        "round2_files": len(files2),
        "round1_total_bytes": sum(files1.values()),
        "round2_total_bytes": sum(files2.values()),
        "only_in_round1": only_in_round1,
        "only_in_round2": only_in_round2,
        "size_changes": size_changes,
        "content_diffs": content_diffs,
        "identical": not (only_in_round1 or only_in_round2 or content_diffs),
    }


def _detect_record_header_size(raw: bytes) -> int:
    if len(raw) < 28:
        return 24
    tes4_data_size = struct.unpack_from("<I", raw, 4)[0]
    pos24 = 24 + tes4_data_size
    pos20 = 20 + tes4_data_size
    if pos24 + 4 <= len(raw) and raw[pos24:pos24 + 4] == b"GRUP":
        return 24
    if pos20 + 4 <= len(raw) and raw[pos20:pos20 + 4] == b"GRUP":
        return 20
    return 24


def read_plugin_header(plugin_path: Path) -> dict[str, Any] | None:
    try:
        raw = plugin_path.read_bytes()[:4096]
    except Exception:
        return None
    if len(raw) < 20 or raw[:4] not in (b"TES4", b"TES3"):
        return None

    header_size = _detect_record_header_size(raw)
    data_size = struct.unpack_from("<I", raw, 4)[0]
    record_end = header_size + data_size
    offset = header_size
    while offset + 6 <= min(record_end, len(raw)):
        sub_sig = raw[offset:offset + 4]
        sub_size = struct.unpack_from("<H", raw, offset + 4)[0]
        data_start = offset + 6
        if sub_sig == b"HEDR" and sub_size >= 12 and data_start + sub_size <= len(raw):
            version = struct.unpack_from("<f", raw, data_start)[0]
            num_records = struct.unpack_from("<I", raw, data_start + 4)[0]
            next_form_id = struct.unpack_from("<I", raw, data_start + 8)[0]
            return {
                "header_size": header_size,
                "num_records": num_records,
                "next_form_id": next_form_id,
                "version": round(version, 3),
            }
        offset = data_start + sub_size
    return {
        "header_size": header_size,
        "num_records": None,
        "next_form_id": None,
        "version": None,
    }


def enumerate_plugin_records(plugin_path: Path) -> tuple[dict[int, str], int]:
    try:
        raw = plugin_path.read_bytes()
    except Exception:
        return {}, 0

    if len(raw) < 20 or raw[:4] not in (b"TES4", b"TES3"):
        return {}, 0

    header_size = _detect_record_header_size(raw)
    raw_len = len(raw)
    records: dict[int, str] = {}

    def parse(offset: int, end: int) -> None:
        while offset + header_size <= end:
            if offset + 8 > raw_len:
                break
            signature = raw[offset:offset + 4]
            size = struct.unpack_from("<I", raw, offset + 4)[0]
            if signature == b"GRUP":
                group_end = offset + size
                if group_end > raw_len or size < header_size:
                    break
                parse(offset + header_size, group_end)
                offset = group_end
                continue

            if offset + 16 <= raw_len:
                form_id = struct.unpack_from("<I", raw, offset + 12)[0]
                if form_id != 0:
                    records[form_id] = signature.decode("ascii", errors="replace")
            next_offset = offset + header_size + size
            if next_offset <= offset:
                break
            offset = next_offset

    parse(0, raw_len)
    # Free the 1.39 GB source buffer before returning. CPython's refcount drops
    # it immediately, instead of waiting for the eventual GC pass after callers
    # have already started parsing the same plugin natively.
    del raw
    return records, header_size


def compare_plugin_records(original: dict[int, str], roundtrip: dict[int, str]) -> dict[str, Any]:
    original_ids = set(original)
    roundtrip_ids = set(roundtrip)

    missing_ids = original_ids - roundtrip_ids
    extra_ids = roundtrip_ids - original_ids
    type_changed = {form_id for form_id in original_ids & roundtrip_ids if original[form_id] != roundtrip[form_id]}

    def group_by_type(form_ids: set[int], source: dict[int, str]) -> dict[str, list[str]]:
        grouped: dict[str, list[str]] = {}
        for form_id in sorted(form_ids):
            grouped.setdefault(source.get(form_id, "????"), []).append(f"{form_id:08X}")
        return dict(sorted(grouped.items()))

    return {
        "total_original": len(original),
        "total_roundtrip": len(roundtrip),
        "missing_count": len(missing_ids),
        "extra_count": len(extra_ids),
        "type_changed_count": len(type_changed),
        "missing": group_by_type(missing_ids, original),
        "extra": group_by_type(extra_ids, roundtrip),
        "type_changed": [
            f"{form_id:08X}: {original[form_id]} -> {roundtrip[form_id]}"
            for form_id in sorted(type_changed)
        ],
        "identical": not (missing_ids or extra_ids or type_changed),
    }


def summarize_record_compare(compare: dict[str, Any] | None) -> str:
    if not compare:
        return "n/a"
    if compare["identical"]:
        return f"identical ({compare['total_original']} FormIDs)"
    return (
        f"{compare['missing_count']} missing, "
        f"{compare['extra_count']} extra, "
        f"{compare['type_changed_count']} type-changed"
    )


def _sample_grouped_records(grouped: dict[str, list[str]], *, max_types: int = 3, max_ids_per_type: int = 4) -> str:
    if not grouped:
        return "none"
    parts: list[str] = []
    for index, (record_type, form_ids) in enumerate(grouped.items()):
        if index >= max_types:
            parts.append(f"... +{len(grouped) - max_types} more types")
            break
        shown = ", ".join(form_ids[:max_ids_per_type])
        if len(form_ids) > max_ids_per_type:
            shown += f", +{len(form_ids) - max_ids_per_type} more"
        parts.append(f"{record_type} [{shown}]")
    return "; ".join(parts)


def _sample_changed_records(entries: list[str], *, max_entries: int = 6) -> str:
    if not entries:
        return "none"
    shown = entries[:max_entries]
    if len(entries) > max_entries:
        shown.append(f"... +{len(entries) - max_entries} more")
    return "; ".join(shown)


def build_record_compare_summary(compare: dict[str, Any] | None) -> dict[str, Any]:
    if not compare:
        return {
            "summary": "n/a",
            "missing_sample": "none",
            "extra_sample": "none",
            "type_changed_sample": "none",
        }
    return {
        "summary": summarize_record_compare(compare),
        "missing_sample": _sample_grouped_records(compare["missing"]),
        "extra_sample": _sample_grouped_records(compare["extra"]),
        "type_changed_sample": _sample_changed_records(compare["type_changed"]),
    }


def build_output_compare_summary(compare: dict[str, Any] | None) -> dict[str, Any]:
    if not compare:
        return {
            "summary": "n/a",
            "round1_bytes": None,
            "round2_bytes": None,
            "only_in_round1_sample": "none",
            "only_in_round2_sample": "none",
            "content_diff_sample": "none",
        }
    return {
        "summary": (
            "identical"
            if compare["identical"]
            else (
                f"{len(compare['only_in_round1'])} only-in-round1, "
                f"{len(compare['only_in_round2'])} only-in-round2, "
                f"{len(compare['content_diffs'])} content-diff"
            )
        ),
        "round1_bytes": compare.get("round1_total_bytes"),
        "round2_bytes": compare.get("round2_total_bytes"),
        "only_in_round1_sample": "; ".join(compare["only_in_round1"][:5]) if compare["only_in_round1"] else "none",
        "only_in_round2_sample": "; ".join(compare["only_in_round2"][:5]) if compare["only_in_round2"] else "none",
        "content_diff_sample": "; ".join(compare["content_diffs"][:5]) if compare["content_diffs"] else "none",
    }


def available_pipeline_names(results: list[dict[str, Any]]) -> list[str]:
    names: list[str] = []
    for pipeline_name in AB_PIPELINE_ORDER:
        if any(pipeline_name in case.get("pipelines", {}) for case in results):
            names.append(pipeline_name)
    return names


def run_native_roundtrip(
    *,
    game_id: str,
    plugin_path: Path,
    work_dir: Path,
    original_info: dict[str, Any],
    pipeline_name: str,
    authoring_format: str,
    jobs: int | None,
    do_second_export: bool = True,
) -> dict[str, Any]:
    authoring_round1_dir = work_dir / f"{authoring_format}_round1"
    authoring_round2_dir = work_dir / f"{authoring_format}_round2"
    roundtrip_path = work_dir / plugin_path.name

    shutil.rmtree(work_dir, ignore_errors=True)
    work_dir.mkdir(parents=True, exist_ok=True)

    result: dict[str, Any] = {
        "pipeline": pipeline_name,
        "ok": False,
        "output_dir": str(authoring_round1_dir),
        "round1_output_dir": str(authoring_round1_dir),
        "round2_output_dir": str(authoring_round2_dir),
        "output_stats": None,
        "round2_output_stats": None,
        "output_compare": None,
        "output_compare_summary": None,
        "second_export_enabled": do_second_export,
        "roundtrip_path": str(roundtrip_path),
        "roundtrip_size": None,
        "size_delta": None,
        "size_delta_pct": None,
        "roundtrip_header": None,
        "roundtrip_record_count": None,
        "roundtrip_actual_count": None,
        "record_compare": None,
        "record_compare_summary": None,
        "timings": {
            "load_seconds": None,
            "export_write_seconds": None,
            "export_seconds": None,
            "import_plugin_seconds": None,
            "import_file_seconds": None,
            "round2_load_seconds": None,
            "export_round2_seconds": None,
            "pipeline_total_seconds": None,
        },
        "steps": {},
    }

    t_export_start = time.perf_counter()
    try:
        t_write_start = time.perf_counter()
        export_authoring_dir_native(
            str(plugin_path),
            str(authoring_round1_dir),
            game=game_id,
            format=authoring_format,
            jobs=jobs,
        )
        write_seconds = time.perf_counter() - t_write_start
        result["timings"]["load_seconds"] = 0.0
        result["timings"]["export_write_seconds"] = write_seconds
        result["timings"]["export_seconds"] = time.perf_counter() - t_export_start
        result["steps"]["load"] = {"ok": True, "output": "skipped; direct native file export"}
        result["steps"]["export"] = {"ok": True, "output": ""}
    except Exception as exc:
        result["steps"]["load"] = {"ok": False, "output": f"{type(exc).__name__}: {exc}"}
        return result

    result["output_stats"] = collect_dir_stats(authoring_round1_dir)

    # Streaming build: walks the authoring dir once, encodes records to bytes,
    # writes to .esp, drops parsed data immediately. Bounded peak memory
    # regardless of plugin size.
    try:
        roundtrip_path.unlink(missing_ok=True)
        t_build_start = time.perf_counter()
        build_authoring_dir(
            authoring_round1_dir,
            roundtrip_path,
            game=game_id,
            jobs=jobs,
        )
        build_seconds = time.perf_counter() - t_build_start
        result["timings"]["import_plugin_seconds"] = 0.0
        result["timings"]["import_file_seconds"] = build_seconds
        result["steps"]["import_plugin"] = {"ok": True, "output": "streaming build (no separate import step)"}
        result["steps"]["import_file"] = {"ok": True, "output": ""}
    except Exception as exc:
        result["steps"]["import_file"] = {"ok": False, "output": f"{type(exc).__name__}: {exc}"}
        return result

    roundtrip_header = read_plugin_header(roundtrip_path)
    roundtrip_records, _ = enumerate_plugin_records(roundtrip_path)
    roundtrip_size = roundtrip_path.stat().st_size
    size_delta = roundtrip_size - int(original_info["original_size"])
    result["roundtrip_path"] = str(roundtrip_path)
    result["roundtrip_size"] = roundtrip_size
    result["size_delta"] = size_delta
    result["size_delta_pct"] = (size_delta / original_info["original_size"] * 100) if original_info["original_size"] else None
    result["roundtrip_header"] = roundtrip_header
    result["roundtrip_record_count"] = roundtrip_header["num_records"] if roundtrip_header else None
    result["roundtrip_actual_count"] = len(roundtrip_records)
    result["record_compare"] = compare_plugin_records(original_info["records"], roundtrip_records)
    result["record_compare_summary"] = build_record_compare_summary(result["record_compare"])

    if not do_second_export:
        result["timings"]["pipeline_total_seconds"] = (
            float(result["timings"]["export_seconds"] or 0.0)
            + float(result["timings"]["import_file_seconds"] or 0.0)
        )
        result["steps"]["export_round2"] = {"ok": True, "output": "skipped"}
        result["ok"] = True
        return result

    rebuilt_plugin: Any | None = None
    try:
        t_round2_load_start = time.perf_counter()
        rebuilt_plugin = Plugin.load(roundtrip_path, game=game_id, backend="native")
        result["timings"]["round2_load_seconds"] = time.perf_counter() - t_round2_load_start
        t_round2_export_start = time.perf_counter()
        export_authoring_dir(
            rebuilt_plugin,
            authoring_round2_dir,
            format=authoring_format,
            backend="native",
            jobs=jobs,
        )
        export_round2_seconds = time.perf_counter() - t_round2_export_start
        result["timings"]["export_round2_seconds"] = export_round2_seconds
        result["timings"]["pipeline_total_seconds"] = (
            float(result["timings"]["export_seconds"] or 0.0)
            + float(result["timings"]["import_file_seconds"] or 0.0)
            + export_round2_seconds
        )
        result["steps"]["export_round2"] = {"ok": True, "output": ""}
    except Exception as exc:
        result["steps"]["export_round2"] = {"ok": False, "output": f"{type(exc).__name__}: {exc}"}
        return result
    finally:
        if rebuilt_plugin is not None:
            rebuilt_plugin.close()
        del rebuilt_plugin
        gc.collect()

    result["round2_output_stats"] = collect_dir_stats(authoring_round2_dir)
    result["output_compare"] = compare_output_dirs(authoring_round1_dir, authoring_round2_dir)
    result["output_compare_summary"] = build_output_compare_summary(result["output_compare"])
    result["ok"] = True
    return result


def summarize_pipeline(result: dict[str, Any]) -> str:
    if not result["ok"]:
        failed = [name for name, step in result["steps"].items() if not step["ok"]]
        return f"FAIL ({', '.join(failed)})"
    output_stats = result.get("output_stats") or {}
    export_round2 = "skipped" if not result.get("second_export_enabled", True) else fmt_seconds(result["timings"].get("export_round2_seconds"))
    return (
        f"export {fmt_seconds(result['timings'].get('export_seconds'))} | "
        f"import-file {fmt_seconds(result['timings'].get('import_file_seconds'))} | "
        f"export2 {export_round2} | "
        f"total {fmt_seconds(result['timings'].get('pipeline_total_seconds'))} | "
        f"{output_stats.get('files', 0)} files | "
        f"{fmt_bytes(output_stats.get('total_bytes'))} | "
        f"records {summarize_record_compare(result.get('record_compare'))}"
    )


def print_pipeline_details(label: str, result: dict[str, Any]) -> None:
    print(f"    {label}")
    print(f"      stats: {summarize_pipeline(result)}")
    print(f"      output: {result.get('output_dir')}")
    if result.get("output_path"):
        print(f"      output file: {result.get('output_path')}")
    if not result["ok"]:
        for step_name, step in result["steps"].items():
            if step["ok"]:
                continue
            tail = (step["output"] or "").strip()
            if len(tail) > 500:
                tail = tail[-500:]
            print(f"      {step_name}: {tail or '(no output)'}")
        return

    roundtrip_size = result.get("roundtrip_size")
    size_delta = result.get("size_delta")
    size_delta_pct = result.get("size_delta_pct")
    size_note = (
        f"{fmt_bytes(roundtrip_size)} "
        f"({size_delta:+,} B, {size_delta_pct:+.2f}%)"
        if roundtrip_size is not None and size_delta is not None and size_delta_pct is not None
        else "n/a"
    )
    print(f"      rebuilt: {result.get('roundtrip_path')}")
    print(f"      rebuilt size: {size_note}")
    compare_summary = result.get("record_compare_summary") or build_record_compare_summary(result.get("record_compare"))
    if compare_summary["summary"] != "n/a":
        print(f"      drift: {compare_summary['summary']}")
        if compare_summary["missing_sample"] != "none":
            print(f"      missing: {compare_summary['missing_sample']}")
        if compare_summary["extra_sample"] != "none":
            print(f"      extra: {compare_summary['extra_sample']}")
        if compare_summary["type_changed_sample"] != "none":
            print(f"      changed: {compare_summary['type_changed_sample']}")
    output_compare_summary = result.get("output_compare_summary") or build_output_compare_summary(result.get("output_compare"))
    if output_compare_summary["summary"] != "n/a":
        print(
            "      roundtrip-output: "
            f"{output_compare_summary['summary']} "
            f"({fmt_bytes(output_compare_summary['round1_bytes'])} -> {fmt_bytes(output_compare_summary['round2_bytes'])})"
        )
        if output_compare_summary["only_in_round1_sample"] != "none":
            print(f"      only round1: {output_compare_summary['only_in_round1_sample']}")
        if output_compare_summary["only_in_round2_sample"] != "none":
            print(f"      only round2: {output_compare_summary['only_in_round2_sample']}")
        if output_compare_summary["content_diff_sample"] != "none":
            print(f"      output diffs: {output_compare_summary['content_diff_sample']}")
    if result.get("pipeline") in {"native_yaml", "native_json"}:
        print(
            "      native detail: "
            f"load={fmt_seconds(result['timings'].get('load_seconds'))} "
            f"export-write={fmt_seconds(result['timings'].get('export_write_seconds'))} "
            f"import-plugin={fmt_seconds(result['timings'].get('import_plugin_seconds'))} "
            f"round2-load={fmt_seconds(result['timings'].get('round2_load_seconds'))}"
        )


def build_case_comparison(case_result: dict[str, Any]) -> dict[str, Any]:
    native_yaml = case_result["pipelines"].get("native_yaml")
    native_json = case_result["pipelines"].get("native_json")
    return {
        "export_speed": describe_speed(
            native_json["timings"].get("export_seconds") if native_json else None,
            native_yaml["timings"].get("export_seconds") if native_yaml else None,
            lhs_label="Native JSON",
            rhs_label="Native YAML",
        ),
        "import_file_speed": describe_speed(
            native_json["timings"].get("import_file_seconds") if native_json else None,
            native_yaml["timings"].get("import_file_seconds") if native_yaml else None,
            lhs_label="Native JSON",
            rhs_label="Native YAML",
        ),
        "pipeline_total_speed": describe_speed(
            native_json["timings"].get("pipeline_total_seconds") if native_json else None,
            native_yaml["timings"].get("pipeline_total_seconds") if native_yaml else None,
            lhs_label="Native JSON",
            rhs_label="Native YAML",
        ),
        "export_ratio_json_over_yaml": _safe_div(
            native_json["timings"].get("export_seconds") if native_json else None,
            native_yaml["timings"].get("export_seconds") if native_yaml else None,
        ),
        "import_file_ratio_json_over_yaml": _safe_div(
            native_json["timings"].get("import_file_seconds") if native_json else None,
            native_yaml["timings"].get("import_file_seconds") if native_yaml else None,
        ),
        "pipeline_total_ratio_json_over_yaml": _safe_div(
            native_json["timings"].get("pipeline_total_seconds") if native_json else None,
            native_yaml["timings"].get("pipeline_total_seconds") if native_yaml else None,
        ),
        "record_drift": {
            "native_yaml": build_record_compare_summary(native_yaml.get("record_compare") if native_yaml else None),
            "native_json": build_record_compare_summary(native_json.get("record_compare") if native_json else None),
        },
    }


def build_drift_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    summary: dict[str, Any] = {
        pipeline_name: {
            "cases_with_drift": 0,
            "total_missing": 0,
            "total_extra": 0,
            "total_type_changed": 0,
            "affected_cases": [],
        }
        for pipeline_name in AB_PIPELINE_ORDER
    }
    for case in results:
        for pipeline_name in AB_PIPELINE_ORDER:
            pipeline = case["pipelines"].get(pipeline_name)
            if not pipeline or not pipeline.get("ok"):
                continue
            compare = pipeline.get("record_compare")
            if not compare or compare["identical"]:
                continue
            entry = summary[pipeline_name]
            entry["cases_with_drift"] += 1
            entry["total_missing"] += int(compare["missing_count"])
            entry["total_extra"] += int(compare["extra_count"])
            entry["total_type_changed"] += int(compare["type_changed_count"])
            entry["affected_cases"].append(
                {
                    "game": case["game"],
                    "plugin": case["plugin"],
                    "plugin_path": case["plugin_path"],
                    "summary": build_record_compare_summary(compare),
                }
            )
    return summary


def build_report_summary(results: list[dict[str, Any]]) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "cases": len(results),
        "native_yaml_ok": 0,
        "native_yaml_fail": 0,
        "native_json_ok": 0,
        "native_json_fail": 0,
        "native_yaml_total_export_seconds": 0.0,
        "native_yaml_total_import_file_seconds": 0.0,
        "native_yaml_total_pipeline_seconds": 0.0,
        "native_json_total_export_seconds": 0.0,
        "native_json_total_import_file_seconds": 0.0,
        "native_json_total_pipeline_seconds": 0.0,
    }
    for case in results:
        native_yaml = case["pipelines"].get("native_yaml")
        native_json = case["pipelines"].get("native_json")
        if native_yaml:
            if native_yaml["ok"]:
                summary["native_yaml_ok"] += 1
                summary["native_yaml_total_export_seconds"] += float(native_yaml["timings"].get("export_seconds") or 0.0)
                summary["native_yaml_total_import_file_seconds"] += float(native_yaml["timings"].get("import_file_seconds") or 0.0)
                summary["native_yaml_total_pipeline_seconds"] += float(native_yaml["timings"].get("pipeline_total_seconds") or 0.0)
            else:
                summary["native_yaml_fail"] += 1
        if native_json:
            if native_json["ok"]:
                summary["native_json_ok"] += 1
                summary["native_json_total_export_seconds"] += float(native_json["timings"].get("export_seconds") or 0.0)
                summary["native_json_total_import_file_seconds"] += float(native_json["timings"].get("import_file_seconds") or 0.0)
                summary["native_json_total_pipeline_seconds"] += float(native_json["timings"].get("pipeline_total_seconds") or 0.0)
            else:
                summary["native_json_fail"] += 1
    summary["aggregate_export_speed"] = describe_speed(
        summary["native_json_total_export_seconds"] if summary["native_json_ok"] else None,
        summary["native_yaml_total_export_seconds"] if summary["native_yaml_ok"] else None,
        lhs_label="Native JSON",
        rhs_label="Native YAML",
    )
    summary["aggregate_import_file_speed"] = describe_speed(
        summary["native_json_total_import_file_seconds"] if summary["native_json_ok"] else None,
        summary["native_yaml_total_import_file_seconds"] if summary["native_yaml_ok"] else None,
        lhs_label="Native JSON",
        rhs_label="Native YAML",
    )
    summary["aggregate_pipeline_speed"] = describe_speed(
        summary["native_json_total_pipeline_seconds"] if summary["native_json_ok"] else None,
        summary["native_yaml_total_pipeline_seconds"] if summary["native_yaml_ok"] else None,
        lhs_label="Native JSON",
        rhs_label="Native YAML",
    )
    return summary


def _report_markdown_path(report_path: Path) -> Path:
    if report_path.suffix.lower() == ".md":
        return report_path.with_name(f"{report_path.stem}.summary.md")
    return report_path.with_suffix(".md")


def resolve_work_root() -> Path:
    return PROJECT_ROOT / "data" / "esp_roundtrip_ab"


def resolve_clear_root(work_root: Path, game_id: str | None) -> Path:
    if game_id:
        return work_root / game_id
    return work_root


def resolve_report_path(work_root: Path, report_arg: str | None, game_id: str | None) -> Path:
    if report_arg:
        report_path = Path(report_arg)
    elif game_id:
        report_path = work_root / game_id / "report.json"
    else:
        report_path = work_root / "report.json"

    if not report_path.is_absolute():
        report_path = PROJECT_ROOT / report_path
    return report_path


def write_markdown_report(
    report_path: Path,
    *,
    payload: dict[str, Any],
) -> None:
    lines: list[str] = []
    summary = payload["summary"]
    drift_summary = payload["drift_summary"]
    second_export_label = "disabled" if not payload.get("second_export_enabled", True) else "enabled"

    lines.append("# ESP Roundtrip A/B Report")
    lines.append("")
    lines.append(f"- Generated: `{payload['generated_at']}`")
    lines.append(f"- Native format selection: `{payload['format']}`")
    lines.append(f"- Second export: `{second_export_label}`")
    lines.append(f"- Report scope: `{payload.get('report_scope', 'all')}`")
    lines.append(f"- Work root: `{payload['work_root']}`")
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    lines.append(
        f"- Cases: `{summary['cases']}` | skipped: `{payload['skipped']}` | "
        f"native_yaml ok/fail: `{summary['native_yaml_ok']}/{summary['native_yaml_fail']}` | "
        f"native_json ok/fail: `{summary['native_json_ok']}/{summary['native_json_fail']}`"
    )
    lines.append(
        f"- Aggregate speed: export `{summary['aggregate_export_speed']}`, "
        f"import-file `{summary['aggregate_import_file_speed']}`, total `{summary['aggregate_pipeline_speed']}`"
    )
    lines.append("")
    lines.append("## Drift Summary")
    lines.append("")
    for pipeline_name in available_pipeline_names(payload["results"]):
        entry = drift_summary[pipeline_name]
        lines.append(
            f"- `{pipeline_name}`: cases with drift `{entry['cases_with_drift']}`, "
            f"missing `{entry['total_missing']}`, extra `{entry['total_extra']}`, "
            f"type-changed `{entry['total_type_changed']}`"
        )
    lines.append("")
    lines.append("## Cases")
    lines.append("")

    for case in payload["results"]:
        lines.append(f"### {case['game']} / {case['plugin']}")
        lines.append("")
        lines.append(f"- Source file: `{case['plugin_path']}`")
        lines.append(
            f"- Original size: `{fmt_bytes(case['original']['size'])}` | "
            f"HEDR count: `{case['original']['record_count']}` | "
            f"actual count: `{case['original']['actual_count']}`"
        )
        comparison = case.get("comparison", {})
        if comparison:
            lines.append(
                f"- Speed comparison: export `{comparison.get('export_speed', 'n/a')}`, "
                f"import-file `{comparison.get('import_file_speed', 'n/a')}`, "
                f"total `{comparison.get('pipeline_total_speed', 'n/a')}`"
            )
            lines.append(
                f"- Ratios native_json/native_yaml: export `{fmt_ratio(comparison.get('export_ratio_json_over_yaml'))}`, "
                f"import-file `{fmt_ratio(comparison.get('import_file_ratio_json_over_yaml'))}`, "
                f"total `{fmt_ratio(comparison.get('pipeline_total_ratio_json_over_yaml'))}`"
            )
        lines.append("")

        for pipeline_name in available_pipeline_names([case]):
            pipeline = case["pipelines"].get(pipeline_name)
            if not pipeline:
                continue
            export_round2_value = "skipped" if not pipeline.get("second_export_enabled", True) else fmt_seconds(pipeline["timings"].get("export_round2_seconds"))
            lines.append(f"#### {PIPELINE_LABELS.get(pipeline_name, pipeline_name)}")
            lines.append("")
            lines.append(f"- Status: `{'ok' if pipeline['ok'] else 'fail'}`")
            lines.append(f"- Output dir: `{pipeline['output_dir']}`")
            if pipeline.get("output_path"):
                lines.append(f"- Output file: `{pipeline['output_path']}`")
            lines.append(f"- Rebuilt file: `{pipeline['roundtrip_path']}`")
            lines.append(
                f"- Timings: export `{fmt_seconds(pipeline['timings'].get('export_seconds'))}`, "
                f"import-file `{fmt_seconds(pipeline['timings'].get('import_file_seconds'))}`, "
                f"export2 `{export_round2_value}`, "
                f"total `{fmt_seconds(pipeline['timings'].get('pipeline_total_seconds'))}`"
            )
            if pipeline_name in {"native_yaml", "native_json"}:
                lines.append(
                    f"- Native detail: load `{fmt_seconds(pipeline['timings'].get('load_seconds'))}`, "
                    f"export-write `{fmt_seconds(pipeline['timings'].get('export_write_seconds'))}`, "
                    f"import-plugin `{fmt_seconds(pipeline['timings'].get('import_plugin_seconds'))}`, "
                    f"round2-load `{fmt_seconds(pipeline['timings'].get('round2_load_seconds'))}`"
                )
            output_stats = pipeline.get("output_stats") or {}
            lines.append(
                f"- Output footprint: `{output_stats.get('files', 0)}` files, "
                f"`{fmt_bytes(output_stats.get('total_bytes'))}`"
            )
            size_delta = pipeline.get("size_delta")
            size_delta_pct = pipeline.get("size_delta_pct")
            if size_delta is not None and size_delta_pct is not None:
                lines.append(
                    f"- Rebuilt size: `{fmt_bytes(pipeline.get('roundtrip_size'))}` "
                    f"(`{size_delta:+,} B`, `{size_delta_pct:+.2f}%`)"
                )
            compare_summary = pipeline.get("record_compare_summary") or build_record_compare_summary(pipeline.get("record_compare"))
            lines.append(f"- Drift: `{compare_summary['summary']}`")
            if compare_summary["missing_sample"] != "none":
                lines.append(f"- Missing sample: {compare_summary['missing_sample']}")
            if compare_summary["extra_sample"] != "none":
                lines.append(f"- Extra sample: {compare_summary['extra_sample']}")
            if compare_summary["type_changed_sample"] != "none":
                lines.append(f"- Changed sample: {compare_summary['type_changed_sample']}")
            output_compare_summary = pipeline.get("output_compare_summary") or build_output_compare_summary(pipeline.get("output_compare"))
            if output_compare_summary["summary"] != "n/a":
                lines.append(
                    f"- Roundtrip output compare: `{output_compare_summary['summary']}` "
                    f"(`{fmt_bytes(output_compare_summary['round1_bytes'])}` -> `{fmt_bytes(output_compare_summary['round2_bytes'])}`)"
                )
                if output_compare_summary["only_in_round1_sample"] != "none":
                    lines.append(f"- Round1-only sample: {output_compare_summary['only_in_round1_sample']}")
                if output_compare_summary["only_in_round2_sample"] != "none":
                    lines.append(f"- Round2-only sample: {output_compare_summary['only_in_round2_sample']}")
                if output_compare_summary["content_diff_sample"] != "none":
                    lines.append(f"- Output diff sample: {output_compare_summary['content_diff_sample']}")
            for step_name, step in pipeline.get("steps", {}).items():
                if step["ok"]:
                    continue
                tail = (step.get("output") or "").strip()
                if len(tail) > 800:
                    tail = tail[-800:]
                lines.append(f"- Failed step `{step_name}`: `{tail or '(no output)'}`")
            lines.append("")

    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Native ESP authoring roundtrip A/B test")
    parser.add_argument("--game", choices=[game["id"] for game in GAMES], help="Only test this game")
    parser.add_argument("--esm", help="Only test this plugin filename")
    parser.add_argument(
        "--format",
        choices=["json", "yaml", "both"],
        default="both",
        help="Native authoring-dir format selection (default: both).",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=None,
        help="Native parallel job count override.",
    )
    parser.add_argument("--keep", action="store_true", help="Deprecated; outputs are now kept by default")
    parser.add_argument(
        "--clear",
        action="store_true",
        help="Clear old benchmark output before running. With --game, only clears that game's output.",
    )
    parser.add_argument(
        "--report",
        type=str,
        default=None,
        help=(
            "Optional JSON report path override. Defaults to data/esp_roundtrip_ab/report.json "
            "for all games, or data/esp_roundtrip_ab/<game>/report.json with --game."
        ),
    )
    parser.add_argument(
        "--trace-writes",
        action="store_true",
        help="Keep esp-native per-file write tracing enabled for debugging",
    )
    parser.add_argument(
        "--no-second-export",
        "--skip-second-export",
        action="store_true",
        help="Skip the round2 export and output comparison",
    )
    args = parser.parse_args()

    if not args.trace_writes:
        os.environ.pop("ESP_AUTHORING_TRACE_WRITES", None)
        os.environ.pop("ESP_AUTHORING_TRACE_WRITES_MIN_MS", None)

    try:
        _native_module = load_native_module()
    except Exception as exc:
        print(f"ERROR: modbox21-native / esp_authoring_core is not available: {exc}")
        return 1
    if _native_module is None:
        print("ERROR: modbox21-native / esp_authoring_core is not available")
        return 1

    env = load_env()

    selected_games = GAMES if not args.game else [game for game in GAMES if game["id"] == args.game]
    work_root = resolve_work_root()
    clear_root = resolve_clear_root(work_root, args.game)
    if args.clear and clear_root.exists():
        shutil.rmtree(clear_root, ignore_errors=True)
        print(f"Cleared old benchmark output: {clear_root}")

    results: list[dict[str, Any]] = []
    failures = 0
    skipped = 0

    for game in selected_games:
        dir_str = env.get(game["dir_var"], "").strip()
        print(f"\n--- {game['display']} ({game['id']}) ---")
        if not dir_str:
            print(f"  Skipped: {game['dir_var']} not set in .env")
            skipped += len(game["esms"])
            continue

        data_dir = Path(dir_str.replace("/", "\\")) / "Data"
        if not data_dir.exists():
            print(f"  Skipped: Data dir not found: {data_dir}")
            skipped += len(game["esms"])
            continue

        effective_jobs = resolve_native_jobs(game["id"], args.jobs)
        requested_jobs_label = "auto" if args.jobs is None else str(args.jobs)
        cap_label = f" (Starfield cap {STARFIELD_NATIVE_JOBS_CAP})" if game["id"] == "starfield" else ""
        print(f"  Native jobs: requested {requested_jobs_label}, effective {effective_jobs}{cap_label}")

        esms = list(game["esms"])
        if args.esm:
            esms = [esm for esm in esms if esm.lower() == args.esm.lower()]
            if not esms:
                print(f"  Skipped: {args.esm} not in configured list")
                continue

        for esm_name in esms:
            plugin_path = data_dir / esm_name
            if not plugin_path.exists():
                print(f"  {esm_name}: skipped (not found)")
                skipped += 1
                continue

            original_header = read_plugin_header(plugin_path)
            original_records, _ = enumerate_plugin_records(plugin_path)
            original_info = {
                "original_size": plugin_path.stat().st_size,
                "original_header": original_header,
                "original_record_count": original_header["num_records"] if original_header else None,
                "original_actual_count": len(original_records),
                "records": original_records,
            }

            print(f"  {esm_name}:")
            print(f"    source: {plugin_path}")
            print(
                "    original: "
                f"size={fmt_bytes(original_info['original_size'])} "
                f"HEDR={original_info['original_record_count'] if original_info['original_record_count'] is not None else 'n/a'} "
                f"actual={original_info['original_actual_count']}"
            )

            case_work_dir = work_root / game["id"] / plugin_path.stem
            case_result: dict[str, Any] = {
                "game": game["id"],
                "plugin": esm_name,
                "plugin_path": str(plugin_path),
                "original": {
                    "size": original_info["original_size"],
                    "header": original_header,
                    "record_count": original_info["original_record_count"],
                    "actual_count": original_info["original_actual_count"],
                },
                "pipelines": {},
                "comparison": {},
            }

            native_yaml_result: dict[str, Any] | None = None
            native_json_result: dict[str, Any] | None = None

            if args.format in {"yaml", "both"}:
                native_yaml_result = run_native_roundtrip(
                    game_id=game["id"],
                    plugin_path=plugin_path,
                    work_dir=case_work_dir / "native_yaml",
                    original_info=original_info,
                    pipeline_name="native_yaml",
                    authoring_format="yaml",
                    jobs=effective_jobs,
                    do_second_export=not args.no_second_export,
                )
                case_result["pipelines"]["native_yaml"] = native_yaml_result
                print_pipeline_details("Native YAML", native_yaml_result)
            if args.format in {"json", "both"}:
                native_json_result = run_native_roundtrip(
                    game_id=game["id"],
                    plugin_path=plugin_path,
                    work_dir=case_work_dir / "native_json",
                    original_info=original_info,
                    pipeline_name="native_json",
                    authoring_format="json",
                    jobs=effective_jobs,
                    do_second_export=not args.no_second_export,
                )
                case_result["pipelines"]["native_json"] = native_json_result
                print_pipeline_details("Native JSON", native_json_result)

            case_result["comparison"] = build_case_comparison(case_result)
            if native_yaml_result and native_json_result:
                print(
                    "    compare: "
                    f"export={case_result['comparison']['export_speed']}; "
                    f"import-file={case_result['comparison']['import_file_speed']}; "
                    f"total={case_result['comparison']['pipeline_total_speed']}"
                )

            results.append(case_result)

            case_failed = any(not pipeline["ok"] for pipeline in case_result["pipelines"].values())
            if case_failed:
                failures += 1
                print(f"    kept: {case_work_dir}")

    summary = build_report_summary(results)
    drift_summary = build_drift_summary(results)

    print(f"\n{'=' * 72}")
    summary_line = (
        "Summary: "
        f"cases={summary['cases']} "
        f"skipped={skipped} "
        f"native_yaml_ok={summary['native_yaml_ok']} "
        f"native_yaml_fail={summary['native_yaml_fail']} "
        f"native_json_ok={summary['native_json_ok']} "
        f"native_json_fail={summary['native_json_fail']}"
    )
    print(summary_line)
    print(
        "Aggregate speed: "
        f"export={summary['aggregate_export_speed']}; "
        f"import-file={summary['aggregate_import_file_speed']}; "
        f"total={summary['aggregate_pipeline_speed']}"
    )
    for pipeline_name in available_pipeline_names(results):
        entry = drift_summary[pipeline_name]
        if entry["cases_with_drift"] == 0:
            continue
        print(
            f"Drift summary ({pipeline_name}): "
            f"cases={entry['cases_with_drift']} "
            f"missing={entry['total_missing']} "
            f"extra={entry['total_extra']} "
            f"changed={entry['total_type_changed']}"
        )

    report_path = resolve_report_path(work_root, args.report, args.game)
    markdown_report_path = _report_markdown_path(report_path)
    payload = {
        "report_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "work_root": str(work_root),
        "report_scope": args.game or "all",
        "pipeline": "native",
        "format": args.format,
        "second_export_enabled": not args.no_second_export,
        "skipped": skipped,
        "results": results,
        "summary": summary,
        "drift_summary": drift_summary,
    }
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    write_markdown_report(markdown_report_path, payload=payload)
    print(f"Report JSON: {report_path}")
    print(f"Report Markdown: {markdown_report_path}")

    return 0 if failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
