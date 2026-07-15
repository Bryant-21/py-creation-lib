"""Havok asset database builder — unified pipeline for all Havok file types.

Pipeline:
  1. Discovery — walk Meshes/ tree, classify files
  2. Caching — unpack HKX → XML for non-XML entries
  3. Parsing — dispatch each entry to its parser (animations/skeletons
               streamed to the BulkInserter as they parse; behaviors /
               projects / characters held for FK resolution)
  4. Manifests — group files into conversion-ready bundles
  5. DB creation — create schema (via the BulkInserter's own connection)
  6. Indexing + FTS — populate remaining tables and rebuild FTS
"""

from __future__ import annotations

import json
import multiprocessing
import os
import subprocess
import sys
import time
from pathlib import Path

from creation_lib.db.native_runtime import BulkInserter
from creation_lib.havok.discovery import FileEntry, discover_havok_files
from creation_lib.havok.manifest import build_manifests
from creation_lib.havok.parsers.animation import parse_animation
from creation_lib.havok.parsers.behavior import BehaviorData, parse_behavior
from creation_lib.havok.parsers.character import parse_character
from creation_lib.havok.parsers.project import parse_project
from creation_lib.havok.parsers.skeleton import parse_skeleton
from creation_lib._native.havok_native import detect_format
from creation_lib.starfield_anim.af_reader import parse_af
from creation_lib.starfield_anim.agx_reader import parse_agx
from creation_lib.starfield_anim.rig_reader import parse_rig

CODE_ROOT = Path(__file__).resolve().parents[2]

NON_USABLE_BEHAVIORS = {
    "WeaponBehavior.xml",
    "WeaponFurnitureBehavior.xml",
    "WorkbenchFurnitureBehavior.xml",
}

_DEFAULT_WORKERS = min(8, max(1, int((os.cpu_count() or 4) * 2 / 3)))

# ---------------------------------------------------------------------------
# Schema
# ---------------------------------------------------------------------------

PROJ_COLS = ["id", "name", "category", "source", "source_path", "havok_version"]
CHAR_COLS = [
    "id", "project_id", "source", "skeleton_path", "skeleton_id",
    "behavior_path", "rig_name", "source_path",
]
SKEL_COLS = [
    "id", "name", "source_path", "source", "bone_count", "bone_names",
    "parent_indices", "reference_pose", "float_count", "partition_names",
]
BEH_COLS = [
    "id", "name", "filename", "category", "source", "source_path",
    "project_id", "character_id", "graph_path", "node_count", "usable", "content",
]
BEH_EV_COLS = ["behavior_id", "event_name"]
BEH_VAR_COLS = ["behavior_id", "variable_name", "variable_type"]
BEH_SEQ_COLS = ["behavior_id", "sequence_name"]
BEH_TRANS_COLS = ["behavior_id", "transition_name", "duration"]
ANIM_COLS = [
    "id", "name", "source_path", "source", "actor", "category", "subcategory",
    "compression_type", "bone_count", "duration", "frame_count",
    "annotation_tracks", "frame0_transforms",
]
MAN_COLS = ["id", "name", "manifest_type", "source", "project_id", "file_count", "total_size"]
MAN_FILE_COLS = ["manifest_id", "file_path", "file_type", "file_size", "role", "ref_type"]
MAN_DEP_COLS = ["manifest_id", "depends_on", "dep_type"]
FTS_COLS = ["name", "id", "entity_type", "category", "content"]


BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "havok_projects", "pk": "id", "on_conflict": "REPLACE", "columns": PROJ_COLS},
        {"name": "havok_characters", "pk": "id", "on_conflict": "REPLACE", "columns": CHAR_COLS},
        {"name": "havok_skeletons", "pk": "id", "on_conflict": "REPLACE", "columns": SKEL_COLS},
        {"name": "havok_behaviors", "pk": "id", "on_conflict": "REPLACE", "columns": BEH_COLS},
        {"name": "behavior_events", "columns": BEH_EV_COLS},
        {"name": "behavior_variables", "columns": BEH_VAR_COLS},
        {"name": "behavior_sequences", "columns": BEH_SEQ_COLS},
        {"name": "behavior_transitions", "columns": BEH_TRANS_COLS},
        {"name": "havok_animations", "pk": "id", "on_conflict": "REPLACE", "columns": ANIM_COLS},
        {"name": "havok_manifests", "pk": "id", "on_conflict": "REPLACE", "columns": MAN_COLS},
        {"name": "havok_manifest_files", "on_conflict": "REPLACE", "columns": MAN_FILE_COLS},
        {"name": "havok_manifest_deps", "on_conflict": "REPLACE", "columns": MAN_DEP_COLS},
        {"name": "havok_fts_stage", "columns": FTS_COLS},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS havok_projects (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL,
    source TEXT NOT NULL, source_path TEXT NOT NULL, havok_version TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS havok_characters (
    id TEXT PRIMARY KEY, project_id TEXT, source TEXT NOT NULL,
    skeleton_path TEXT DEFAULT '', skeleton_id TEXT DEFAULT '',
    behavior_path TEXT DEFAULT '', rig_name TEXT DEFAULT '',
    source_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS havok_skeletons (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, source_path TEXT NOT NULL,
    source TEXT NOT NULL, bone_count INTEGER DEFAULT 0,
    bone_names TEXT DEFAULT '[]', parent_indices TEXT DEFAULT '[]',
    reference_pose TEXT DEFAULT '[]', float_count INTEGER DEFAULT 0,
    partition_names TEXT DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS havok_behaviors (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, filename TEXT NOT NULL,
    category TEXT NOT NULL, source TEXT NOT NULL, source_path TEXT NOT NULL,
    project_id TEXT, character_id TEXT,
    graph_path TEXT DEFAULT '', node_count INTEGER DEFAULT 0,
    usable INTEGER DEFAULT 1, content TEXT DEFAULT ''
);
CREATE TABLE IF NOT EXISTS behavior_events (
    behavior_id TEXT NOT NULL, event_name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS behavior_variables (
    behavior_id TEXT NOT NULL, variable_name TEXT NOT NULL,
    variable_type TEXT DEFAULT 'VARIABLE_TYPE_REAL'
);
CREATE TABLE IF NOT EXISTS behavior_sequences (
    behavior_id TEXT NOT NULL, sequence_name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS behavior_transitions (
    behavior_id TEXT NOT NULL, transition_name TEXT NOT NULL, duration TEXT DEFAULT '0'
);
CREATE TABLE IF NOT EXISTS havok_animations (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, source_path TEXT NOT NULL,
    source TEXT NOT NULL, actor TEXT DEFAULT '', category TEXT DEFAULT '',
    subcategory TEXT DEFAULT '', compression_type TEXT DEFAULT 'unknown',
    bone_count INTEGER DEFAULT 0, duration REAL DEFAULT 0.0,
    frame_count INTEGER DEFAULT 0, annotation_tracks TEXT DEFAULT '[]',
    frame0_transforms BLOB
);
CREATE TABLE IF NOT EXISTS havok_manifests (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, manifest_type TEXT NOT NULL,
    source TEXT NOT NULL, project_id TEXT DEFAULT '',
    file_count INTEGER DEFAULT 0, total_size INTEGER DEFAULT 0
);
CREATE TABLE IF NOT EXISTS havok_manifest_files (
    manifest_id TEXT NOT NULL, file_path TEXT NOT NULL,
    file_type TEXT DEFAULT '', file_size INTEGER DEFAULT 0,
    role TEXT DEFAULT '', ref_type TEXT DEFAULT 'owned',
    PRIMARY KEY (manifest_id, file_path)
);
CREATE TABLE IF NOT EXISTS havok_manifest_deps (
    manifest_id TEXT NOT NULL, depends_on TEXT NOT NULL,
    dep_type TEXT DEFAULT '',
    PRIMARY KEY (manifest_id, depends_on)
);
/* Staging table for FTS rows; we copy into havok_fts (FTS5) once at the end. */
CREATE TABLE IF NOT EXISTS havok_fts_stage (
    name TEXT, id TEXT, entity_type TEXT, category TEXT, content TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS havok_fts USING fts5(
    name, id, entity_type, category, content
);
"""

CREATE_INDEXES_SQL = """
CREATE INDEX IF NOT EXISTS idx_havok_projects_category ON havok_projects(category);
CREATE INDEX IF NOT EXISTS idx_havok_projects_source ON havok_projects(source);
CREATE INDEX IF NOT EXISTS idx_havok_behaviors_category ON havok_behaviors(category);
CREATE INDEX IF NOT EXISTS idx_havok_behaviors_source ON havok_behaviors(source);
CREATE INDEX IF NOT EXISTS idx_havok_animations_source ON havok_animations(source);
CREATE INDEX IF NOT EXISTS idx_havok_animations_actor ON havok_animations(actor);
CREATE INDEX IF NOT EXISTS idx_behavior_events_name ON behavior_events(event_name);
CREATE INDEX IF NOT EXISTS idx_behavior_events_bid ON behavior_events(behavior_id);
CREATE INDEX IF NOT EXISTS idx_behavior_variables_name ON behavior_variables(variable_name);
CREATE INDEX IF NOT EXISTS idx_behavior_variables_bid ON behavior_variables(behavior_id);
CREATE INDEX IF NOT EXISTS idx_behavior_sequences_bid ON behavior_sequences(behavior_id);
CREATE INDEX IF NOT EXISTS idx_behavior_sequences_name ON behavior_sequences(sequence_name);
CREATE INDEX IF NOT EXISTS idx_behavior_transitions_bid ON behavior_transitions(behavior_id);
CREATE INDEX IF NOT EXISTS idx_behavior_transitions_name ON behavior_transitions(transition_name);
"""

# ---------------------------------------------------------------------------
# Progress helpers
# ---------------------------------------------------------------------------


def _fmt_time(seconds: float) -> str:
    s = int(seconds)
    if s < 3600:
        return f"{s // 60:02d}:{s % 60:02d}"
    return f"{s // 3600}:{(s % 3600) // 60:02d}:{s % 60:02d}"


def _print_progress(label: str, done: int, total: int, start_time: float) -> None:
    elapsed = time.monotonic() - start_time
    pct = done / total if total else 1.0
    eta_str = _fmt_time((elapsed / pct) - elapsed) if pct > 0 else "?"
    bar_len = 30
    filled = int(bar_len * pct)
    bar = "=" * filled + "-" * (bar_len - filled)
    print(
        f"\r  {label}: [{bar}] {done}/{total} ({pct:.0%})"
        f"  elapsed={_fmt_time(elapsed)}  eta={eta_str}   ",
        end="",
        flush=True,
    )


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def build_display_name(rel_path: str) -> str:
    rel_path = rel_path.replace("\\", "/")
    for ext in (".xml", ".hkx", ".hkt", ".nif"):
        if rel_path.lower().endswith(ext):
            rel_path = rel_path[: -len(ext)]
            break
    parts = rel_path.split("/")
    filename = parts[-1] if parts else rel_path
    generic_names = {"behavior", "behaviorgraph", "rootbehavior", "character", "skeleton", "project"}
    if len(parts) >= 2 and filename.lower() in generic_names:
        parent = parts[-2]
        skip_parents = {"behaviors", "characters", "characterassets", "animations"}
        if parent.lower() not in skip_parents:
            return f"{parent} - {filename}"
        if len(parts) >= 3:
            return f"{parts[-3]} - {filename}"
    return filename


def is_non_usable(rel_path: str) -> bool:
    filename = Path(rel_path).name
    rel_normalized = rel_path.replace("\\", "/")
    return (
        "Actors/Character/Behaviors/" in rel_normalized
        and filename in NON_USABLE_BEHAVIORS
    )


def build_fts_content(
    name: str, parsed: BehaviorData, rel_path: str = "", graph_path: str = ""
) -> str:
    parts = [name]
    if rel_path:
        path_parts = rel_path.replace("\\", "/").split("/")
        skip = {"behaviors", "behaviour", "xml", "hkx", "characters", "characterassets", "animations"}
        for seg in path_parts:
            stem = seg.rsplit(".", 1)[0] if "." in seg else seg
            if stem.lower() not in skip and stem.lower() != name.lower():
                parts.append(stem)
    if graph_path:
        gp_parts = graph_path.replace("\\", "/").split("/")
        skip = {"behaviors", "behaviour", "xml", "hkx"}
        for seg in gp_parts:
            stem = seg.rsplit(".", 1)[0] if "." in seg else seg
            if stem.lower() not in skip and stem.lower() != name.lower():
                if stem not in parts:
                    parts.append(stem)
        hkx_filename = gp_parts[-1] if gp_parts else ""
        if hkx_filename and hkx_filename not in parts:
            parts.append(hkx_filename)
    if parsed.events:
        parts.append("events: " + " ".join(parsed.events[:50]))
    if parsed.variables:
        parts.append("vars: " + " ".join(v[0] for v in parsed.variables[:50]))
    if parsed.node_classes:
        parts.append("nodes: " + " ".join(parsed.node_classes[:30]))
    return " ".join(parts)


def _resolve_graph_path(rel_path: str, projects: dict) -> str:
    rel_normalized = rel_path.replace("\\", "/")
    rel_dir = os.path.dirname(rel_normalized)
    while rel_dir:
        for proj_rel in projects:
            proj_dir = os.path.dirname(proj_rel.replace("\\", "/"))
            if rel_dir.startswith(proj_dir):
                return rel_normalized.replace(".xml", ".hkx")
        parent = os.path.dirname(rel_dir)
        if parent == rel_dir:
            break
        rel_dir = parent
    return rel_normalized.replace(".xml", ".hkx")


def _infer_actor(rel_path: str) -> str:
    parts = rel_path.replace("\\", "/").split("/")
    for i, p in enumerate(parts):
        if p.lower() == "actors" and i + 1 < len(parts):
            return parts[i + 1]
    return ""


def _infer_subcategory(rel_path: str) -> str:
    parts = rel_path.replace("\\", "/").split("/")
    for i, p in enumerate(parts):
        if p.lower() == "animations" and i + 1 < len(parts):
            if i + 2 < len(parts):
                return parts[i + 1]
    return ""


# ---------------------------------------------------------------------------
# Columnar buffer helpers
# ---------------------------------------------------------------------------


def _empty_buf():
    return {
        "havok_projects": {c: [] for c in PROJ_COLS},
        "havok_characters": {c: [] for c in CHAR_COLS},
        "havok_skeletons": {c: [] for c in SKEL_COLS},
        "havok_behaviors": {c: [] for c in BEH_COLS},
        "behavior_events": {c: [] for c in BEH_EV_COLS},
        "behavior_variables": {c: [] for c in BEH_VAR_COLS},
        "behavior_sequences": {c: [] for c in BEH_SEQ_COLS},
        "behavior_transitions": {c: [] for c in BEH_TRANS_COLS},
        "havok_animations": {c: [] for c in ANIM_COLS},
        "havok_manifests": {c: [] for c in MAN_COLS},
        "havok_manifest_files": {c: [] for c in MAN_FILE_COLS},
        "havok_manifest_deps": {c: [] for c in MAN_DEP_COLS},
        "havok_fts_stage": {c: [] for c in FTS_COLS},
    }


def _flush(bulk, buf, tables=None):
    for table, cols in buf.items():
        if tables is not None and table not in tables:
            continue
        n = len(next(iter(cols.values())))
        if n > 0:
            bulk.add_chunk(table, cols)
            for lst in cols.values():
                lst.clear()


def _flush_if_big(bulk, buf, table, threshold):
    cols = buf[table]
    if len(next(iter(cols.values()))) >= threshold:
        bulk.add_chunk(table, cols)
        for lst in cols.values():
            lst.clear()


# ---------------------------------------------------------------------------
# Indexing helpers — append to columnar buffers
# ---------------------------------------------------------------------------


def _index_project(buf, proj_id, name, category, source, source_path, havok_version=""):
    b = buf["havok_projects"]
    b["id"].append(proj_id)
    b["name"].append(name)
    b["category"].append(category)
    b["source"].append(source)
    b["source_path"].append(source_path)
    b["havok_version"].append(havok_version)


def _index_character(buf, char_id, project_id, source, skeleton_path, skeleton_id,
                     behavior_path, rig_name, source_path):
    b = buf["havok_characters"]
    b["id"].append(char_id)
    b["project_id"].append(project_id)
    b["source"].append(source)
    b["skeleton_path"].append(skeleton_path)
    b["skeleton_id"].append(skeleton_id)
    b["behavior_path"].append(behavior_path)
    b["rig_name"].append(rig_name)
    b["source_path"].append(source_path)


def _index_skeleton(buf, skel_id, name, source_path, source, skel_data):
    b = buf["havok_skeletons"]
    b["id"].append(skel_id)
    b["name"].append(name)
    b["source_path"].append(source_path)
    b["source"].append(source)
    b["bone_count"].append(skel_data.bone_count)
    b["bone_names"].append(json.dumps(skel_data.bone_names))
    b["parent_indices"].append(json.dumps(skel_data.parent_indices))
    b["reference_pose"].append(json.dumps(skel_data.reference_pose))
    b["float_count"].append(skel_data.float_count)
    b["partition_names"].append(json.dumps(skel_data.partition_names))


def _index_behavior(buf, beh_id, name, filename, category, source, source_path,
                    project_id, character_id, graph_path, parsed, usable):
    content = build_fts_content(name, parsed, source_path, graph_path)
    b = buf["havok_behaviors"]
    b["id"].append(beh_id)
    b["name"].append(name)
    b["filename"].append(filename)
    b["category"].append(category)
    b["source"].append(source)
    b["source_path"].append(source_path)
    b["project_id"].append(project_id)
    b["character_id"].append(character_id)
    b["graph_path"].append(graph_path)
    b["node_count"].append(parsed.node_count)
    b["usable"].append(usable)
    b["content"].append(content)

    for ev in parsed.events:
        buf["behavior_events"]["behavior_id"].append(beh_id)
        buf["behavior_events"]["event_name"].append(ev)
    for vname, vtype in parsed.variables:
        buf["behavior_variables"]["behavior_id"].append(beh_id)
        buf["behavior_variables"]["variable_name"].append(vname)
        buf["behavior_variables"]["variable_type"].append(vtype)
    for seq in parsed.sequences:
        buf["behavior_sequences"]["behavior_id"].append(beh_id)
        buf["behavior_sequences"]["sequence_name"].append(seq)
    for tname, tdur in parsed.transitions:
        buf["behavior_transitions"]["behavior_id"].append(beh_id)
        buf["behavior_transitions"]["transition_name"].append(tname)
        buf["behavior_transitions"]["duration"].append(tdur)


def _index_animation(buf, anim_id, name, source_path, source, actor, category, subcategory, anim_data):
    b = buf["havok_animations"]
    b["id"].append(anim_id)
    b["name"].append(name)
    b["source_path"].append(source_path)
    b["source"].append(source)
    b["actor"].append(actor)
    b["category"].append(category)
    b["subcategory"].append(subcategory)
    b["compression_type"].append(anim_data.compression_type)
    b["bone_count"].append(anim_data.bone_count)
    b["duration"].append(float(anim_data.duration))
    b["frame_count"].append(anim_data.frame_count)
    b["annotation_tracks"].append(json.dumps(anim_data.annotation_tracks))
    # frame0_transforms is a bytes/BLOB (or None)
    b["frame0_transforms"].append(anim_data.frame0_transforms)


def _index_manifest(buf, manifest):
    m = buf["havok_manifests"]
    m["id"].append(manifest.id)
    m["name"].append(manifest.name)
    m["manifest_type"].append(manifest.manifest_type)
    m["source"].append(manifest.source)
    m["project_id"].append(manifest.project_id)
    m["file_count"].append(manifest.file_count)
    m["total_size"].append(manifest.total_size)
    for f in manifest.files:
        mf = buf["havok_manifest_files"]
        mf["manifest_id"].append(manifest.id)
        mf["file_path"].append(f.file_path)
        mf["file_type"].append(f.file_type)
        mf["file_size"].append(f.file_size)
        mf["role"].append(f.role)
        mf["ref_type"].append(f.ref_type)
    for d in manifest.dependencies:
        md = buf["havok_manifest_deps"]
        md["manifest_id"].append(manifest.id)
        md["depends_on"].append(d.depends_on)
        md["dep_type"].append(d.dep_type)


def _index_fts(buf, entity_id, name, entity_type, category, content_text):
    f = buf["havok_fts_stage"]
    f["name"].append(name)
    f["id"].append(entity_id)
    f["entity_type"].append(entity_type)
    f["category"].append(category)
    f["content"].append(content_text)


# ---------------------------------------------------------------------------
# Caching
# ---------------------------------------------------------------------------


def _cache_worker(args):
    rel_path, abs_path, cached_xml_str = args
    cached_xml = Path(cached_xml_str)
    try:
        cached_xml.parent.mkdir(parents=True, exist_ok=True)
        result = subprocess.run(
            [
                sys.executable, "-c",
                f"import sys; sys.path.insert(0, {str(CODE_ROOT)!r});"
                f"from pathlib import Path;"
                f"from creation_lib._native.havok_native import unpack_hkx_to_xml;"
                f"xml = unpack_hkx_to_xml({abs_path!r});"
                f"Path({cached_xml_str!r}).write_text(xml, encoding='utf-8')",
            ],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60,
        )
        if cached_xml.exists():
            return (rel_path, cached_xml_str, None)
        return (rel_path, None, f"exit code {result.returncode}")
    except subprocess.TimeoutExpired:
        return (rel_path, None, "timeout (60s)")
    except Exception as e:
        return (rel_path, None, str(e))


def cache_havok_files(entries, cache_dir: Path, num_workers=None, mem_check=None):
    xml_map: dict[str, str] = {}
    to_unpack: list[tuple[str, str, str]] = []

    skipped_binary_tagfile = 0
    for entry in entries:
        if entry.file_type not in ("xml", "hkx", "hkt"):
            continue
        if entry.is_xml:
            xml_map[entry.rel_path] = str(entry.abs_path)
            continue

        cached_xml = cache_dir / entry.rel_path.replace(".hkx", ".xml").replace(".hkt", ".xml")
        if cached_xml.exists():
            xml_map[entry.rel_path] = str(cached_xml)
            continue

        try:
            with open(entry.abs_path, "rb") as _fh:
                _header = _fh.read(32)
            _fmt = detect_format(_header)
        except OSError:
            _fmt = None

        if _fmt and _fmt[0] == "binary_tagfile":
            skipped_binary_tagfile += 1
            continue

        to_unpack.append((entry.rel_path, str(entry.abs_path), str(cached_xml)))

    if skipped_binary_tagfile:
        print(f"      Skipped {skipped_binary_tagfile} binary tagfile files")

    if not to_unpack:
        return xml_map

    if num_workers is None:
        num_workers = _DEFAULT_WORKERS
    total = len(to_unpack)
    print(f"  Unpacking {total} HKX files with {min(num_workers, 4)} workers...")
    start_t = time.monotonic()
    done = 0
    unpack_errors = 0
    _print_progress("Unpack HKX", done, total, start_t)

    def _handle_unpack(rel_path, xml_path, error):
        nonlocal unpack_errors
        if xml_path:
            xml_map[rel_path] = xml_path
        elif error:
            unpack_errors += 1
            print(f"\n  ERROR unpacking {rel_path}: {error}", flush=True)

    from concurrent.futures import ThreadPoolExecutor, as_completed

    unpack_workers = min(num_workers or _DEFAULT_WORKERS, 8)
    if unpack_workers > 1:
        with ThreadPoolExecutor(max_workers=unpack_workers) as pool:
            CHUNK = 500
            for chunk_start in range(0, total, CHUNK):
                chunk = to_unpack[chunk_start: chunk_start + CHUNK]
                futures = {pool.submit(_cache_worker, a): a[0] for a in chunk}
                for fut in as_completed(futures):
                    _handle_unpack(*fut.result())
                    done += 1
                    if done % max(1, total // 200) == 0 or done == total:
                        _print_progress("Unpack HKX", done, total, start_t)
                del futures
                if mem_check:
                    mem_check(f"unpack ({done}/{total})")
    else:
        for args in to_unpack:
            _handle_unpack(*_cache_worker(args))
            done += 1
            if done % max(1, total // 200) == 0 or done == total:
                _print_progress("Unpack HKX", done, total, start_t)
            if mem_check and done % 100 == 0:
                mem_check(f"unpack ({done}/{total})")

    suffix = f"  ({unpack_errors} errors)" if unpack_errors else ""
    print(f"\n  Done in {_fmt_time(time.monotonic() - start_t)}{suffix}")
    return xml_map


# ---------------------------------------------------------------------------
# FK resolution
# ---------------------------------------------------------------------------


def _resolve_fk_links(projects, characters, behaviors):
    char_by_path = {}
    for rel_path, cdata in characters.items():
        char_id = cdata.get("id", "")
        proj_id = cdata.get("project_id", "")
        beh_file = cdata["data"].behavior_filename if cdata["data"] else ""
        char_by_path[rel_path] = (char_id, proj_id, beh_file)

    beh_by_path = {}
    for rel_path, bdata in behaviors.items():
        beh_by_path[rel_path.replace("\\", "/").lower()] = bdata

    for proj_rel, pdata in projects.items():
        proj_id = pdata.get("id", "")
        proj_dir = os.path.dirname(proj_rel)
        for char_filename in pdata["data"].character_filenames:
            char_rel = os.path.join(proj_dir, char_filename).replace("\\", "/")
            for cr in [char_rel, char_rel.replace(".hkx", ".xml")]:
                if cr in char_by_path:
                    char_id, _, beh_filename = char_by_path[cr]
                    characters[cr]["project_id"] = proj_id
                    if beh_filename:
                        beh_rel = os.path.join(os.path.dirname(cr), beh_filename)
                        beh_key = beh_rel.replace("\\", "/").lower()
                        for bk in [beh_key, beh_key.replace(".hkx", ".xml")]:
                            if bk in beh_by_path:
                                beh_by_path[bk]["project_id"] = proj_id
                                beh_by_path[bk]["character_id"] = char_id
                                break
                    break


# ---------------------------------------------------------------------------
# Parse worker
# ---------------------------------------------------------------------------


def _parse_worker(args):
    rel_path, src_path, role, category, entity_id = args
    src = Path(src_path)
    try:
        ext = src.suffix.lower()
        if ext == ".rig":
            data = parse_rig(src)
        elif ext == ".af":
            data = parse_af(src)
        elif ext == ".agx":
            data = parse_agx(src)
        elif role == "project":
            data = parse_project(src)
        elif role == "character":
            data = parse_character(src)
        elif role == "skeleton":
            data = parse_skeleton(src)
        elif role == "behavior":
            data = parse_behavior(src)
        elif role == "animation":
            data = parse_animation(src)
        else:
            data = None
        return (rel_path, role, data, entity_id, category, None)
    except Exception as e:
        return (rel_path, role, None, entity_id, category, str(e))


# ---------------------------------------------------------------------------
# Build pipeline
# ---------------------------------------------------------------------------


def build_db(
    meshes_dir,
    db_path,
    game="fo4",
    source="fo4",
    cache_dir=None,
    build_embeddings=False,
    num_workers=None,
):
    meshes_dir = Path(meshes_dir)
    db_path = Path(db_path)
    pipeline_start = time.monotonic()

    if num_workers is None:
        num_workers = _DEFAULT_WORKERS

    import psutil
    _proc = psutil.Process()
    _MEM_LIMIT_MB = 10 * 1024

    def _mem_mb():
        total = _proc.memory_info().rss
        try:
            for child in _proc.children(recursive=True):
                try:
                    total += child.memory_info().rss
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
        return total / 1024 / 1024

    def _check_mem(phase: str):
        mb = _mem_mb()
        if mb > _MEM_LIMIT_MB:
            for child in _proc.children(recursive=True):
                try:
                    child.kill()
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
            print(f"\n  ABORT: memory {mb:.0f} MB exceeds {_MEM_LIMIT_MB} MB limit during {phase}")
            sys.exit(1)
        return mb

    # Phase 1: Discovery
    t0 = time.monotonic()
    entries = discover_havok_files(meshes_dir)
    print(f"[1/6] Discovery: {len(entries)} files  ({_fmt_time(time.monotonic() - t0)})  [{_check_mem('discovery'):.0f} MB]")

    # Phase 2: Cache HKX -> XML
    t0 = time.monotonic()
    print("[2/6] Caching HKX -> XML...")
    if cache_dir is None:
        cache_dir = db_path.parent / f"xml_{game}_cache"
    cache_dir = Path(cache_dir)
    cache_dir.mkdir(parents=True, exist_ok=True)
    xml_map = cache_havok_files(entries, cache_dir, num_workers=num_workers, mem_check=_check_mem)
    print(f"      {len(xml_map)} XML paths ready  ({_fmt_time(time.monotonic() - t0)})  [{_check_mem('caching'):.0f} MB]")

    # Build parse task list
    parse_tasks = []
    seen_entity_ids: set[str] = set()
    for entry in entries:
        if entry.role == "asset":
            continue
        if entry.file_type in ("af", "rig", "agx"):
            src = str(entry.abs_path)
        else:
            xml_path = xml_map.get(entry.rel_path)
            if xml_path is None and not entry.is_xml:
                continue
            src = xml_path if xml_path else str(entry.abs_path)
        entity_id = f"{source}/{os.path.splitext(entry.rel_path)[0]}"
        if entity_id in seen_entity_ids:
            continue
        seen_entity_ids.add(entity_id)
        parse_tasks.append((entry.rel_path, src, entry.role, entry.category, entity_id))

    os.makedirs(str(db_path.parent), exist_ok=True)

    projects = {}
    characters = {}
    behaviors = {}
    parse_errors = 0
    anim_count = 0
    skel_count = 0

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        buf = _empty_buf()

        t0 = time.monotonic()
        total_parse = len(parse_tasks)
        print(f"[3/6] Parsing {total_parse} files with {num_workers} workers...")
        done_parse = 0
        _print_progress("Parse", done_parse, total_parse, t0)

        def _collect_parsed(rel_path, role, data, entity_id, category, error):
            nonlocal parse_errors, anim_count, skel_count
            if error or data is None:
                parse_errors += 1
                print(f"\n  PARSE ERROR {rel_path}: {error}", flush=True)
                return
            if role == "animation":
                name = build_display_name(rel_path)
                actor = _infer_actor(rel_path)
                subcat = _infer_subcategory(rel_path)
                _index_animation(buf, entity_id, name, rel_path, source, actor, category, subcat, data)
                _index_fts(buf, entity_id, name, "animation", category, f"{actor} {category} {subcat}")
                anim_count += 1
                # Periodically flush the hot tables to cap buffer memory.
                _flush_if_big(bulk, buf, "havok_animations", 1000)
                _flush_if_big(bulk, buf, "havok_fts_stage", 2000)
            elif role == "skeleton":
                name = data.name or build_display_name(rel_path)
                _index_skeleton(buf, entity_id, name, rel_path, source, data)
                _index_fts(buf, entity_id, name, "skeleton", "",
                           " ".join(data.bone_names + data.partition_names))
                skel_count += 1
                _flush_if_big(bulk, buf, "havok_skeletons", 500)
            elif role == "project":
                projects[rel_path] = {
                    "id": entity_id, "data": data, "category": category,
                    "source_path": rel_path,
                }
            elif role == "character":
                characters[rel_path] = {
                    "id": entity_id, "data": data, "project_id": "",
                    "source_path": rel_path,
                }
            elif role == "behavior":
                behaviors[rel_path] = {
                    "id": entity_id, "data": data, "category": category,
                    "source_path": rel_path, "project_id": None, "character_id": None,
                }

        if num_workers > 1 and parse_tasks:
            with multiprocessing.Pool(num_workers, maxtasksperchild=10) as pool:
                for result in pool.imap_unordered(_parse_worker, parse_tasks, chunksize=4):
                    _collect_parsed(*result)
                    done_parse += 1
                    if done_parse % max(1, total_parse // 200) == 0 or done_parse == total_parse:
                        _print_progress("Parse", done_parse, total_parse, t0)
                    if done_parse % 500 == 0:
                        _check_mem(f"parse ({done_parse}/{total_parse})")
        else:
            for task in parse_tasks:
                _collect_parsed(*_parse_worker(task))
                done_parse += 1
                if done_parse % max(1, total_parse // 200) == 0 or done_parse == total_parse:
                    _print_progress("Parse", done_parse, total_parse, t0)
                if done_parse % 500 == 0:
                    _check_mem(f"parse ({done_parse}/{total_parse})")

        # Flush streamed tables before phase 3b
        _flush(bulk, buf, tables=("havok_animations", "havok_skeletons", "havok_fts_stage"))

        parse_err_str = f"  ({parse_errors} errors)" if parse_errors else ""
        print(f"\n      Done in {_fmt_time(time.monotonic() - t0)}{parse_err_str}  [{_check_mem('parse done'):.0f} MB]")
        print(f"     streamed {anim_count} animations, {skel_count} skeletons to DB")

        # Phase 3b: Resolve FK links
        t0 = time.monotonic()
        _resolve_fk_links(projects, characters, behaviors)
        print(f"[3b]   FK links resolved  ({_fmt_time(time.monotonic() - t0)})")

        # Phase 4: Manifests
        t0 = time.monotonic()
        char_data_map = {rp: d["data"] for rp, d in characters.items()}
        manifests = build_manifests(entries, char_data_map, source)
        print(f"[4/6] Manifests: {len(manifests)} bundles  ({_fmt_time(time.monotonic() - t0)})")

        # Phase 5: Index remaining entities
        t0 = time.monotonic()
        total_index = len(projects) + len(characters) + len(behaviors) + len(manifests)
        print(f"[5/6] Indexing {total_index} remaining entities...")
        done_index = 0
        _print_progress("Index", done_index, total_index, t0)

        def _tick():
            nonlocal done_index
            done_index += 1
            if done_index % max(1, total_index // 200) == 0 or done_index == total_index:
                _print_progress("Index", done_index, total_index, t0)

        for rp, p in projects.items():
            _index_project(buf, p["id"], build_display_name(rp), p["category"], source, p["source_path"])
            _index_fts(buf, p["id"], build_display_name(rp), "project", p["category"], rp.replace("/", " "))
            _tick()

        for rp, c in characters.items():
            cd = c["data"]
            _index_character(buf, c["id"], c.get("project_id", ""), source, "", "",
                             cd.behavior_filename, cd.rig_name, rp)
            _tick()

        for rp, b in behaviors.items():
            bd = b["data"]
            beh_id = b["id"]
            fname = os.path.basename(rp)
            usable = 0 if is_non_usable(rp) else 1
            graph_path = _resolve_graph_path(rp, projects)
            _index_behavior(buf, beh_id, build_display_name(rp), fname, b["category"],
                            source, rp, b.get("project_id", ""), b.get("character_id", ""),
                            graph_path, bd, usable)
            content = build_fts_content(build_display_name(rp), bd, rp, graph_path)
            _index_fts(buf, beh_id, build_display_name(rp), "behavior", b["category"], content)
            _tick()

        for m in manifests:
            _index_manifest(buf, m)
            _index_fts(buf, m.id, m.name, "manifest", m.manifest_type,
                       " ".join(f.file_path for f in m.files))
            _tick()

        print(f"\n      Done in {_fmt_time(time.monotonic() - t0)}")

        # Final flush + indexes + FTS
        _flush(bulk, buf)

        print("[idx]  Building indexes...", flush=True)
        bulk.create_indexes(CREATE_INDEXES_SQL)

        # Copy staging FTS rows into the FTS5 virtual table, then drop staging.
        print("[fts]  Populating FTS...", flush=True)
        bulk.execute(
            "INSERT INTO havok_fts (name, id, entity_type, category, content) "
            "SELECT name, id, entity_type, category, content FROM havok_fts_stage; "
            "DROP TABLE havok_fts_stage;"
        )

    if build_embeddings:
        t0 = time.monotonic()
        print("Embeddings: building...", flush=True)
        with BulkInserter(str(db_path), BULK_SCHEMA, fresh=False) as bulk:
            rows = bulk.query_all(
                "SELECT id, name, entity_type, category, content FROM havok_fts"
            )
        embed_texts = []
        embed_ids = []
        for r in rows:
            entity_id, name, entity_type, category, content = r
            text = f"{name} {entity_type} {category} {content}".strip()
            embed_texts.append(text)
            embed_ids.append(entity_id)
        if embed_texts:
            from creation_lib.db.embeddings import build_vec_index
            print(f"  {len(embed_texts)} documents")
            build_vec_index(embed_texts, embed_ids, str(db_path), table_name="havok_embeddings")
            print(f"  Embeddings done  ({_fmt_time(time.monotonic() - t0)})")

    total = len(projects) + len(characters) + skel_count + len(behaviors) + anim_count
    elapsed_total = time.monotonic() - pipeline_start
    print(f"\nDone. Indexed {total} entities + {len(manifests)} manifests in {_fmt_time(elapsed_total)}")
    return {
        "projects": len(projects),
        "characters": len(characters),
        "skeletons": skel_count,
        "behaviors": len(behaviors),
        "animations": anim_count,
        "manifests": len(manifests),
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build Havok asset database")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--stats", action="store_true")
    parser.add_argument("--extracted-dir", default=None)
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--cache-dir", default=None)
    parser.add_argument("--mod", default=None)
    parser.add_argument("--external-mods-dir", default=None)
    parser.add_argument("--embeddings", action="store_true")
    parser.add_argument("--workers", type=int, default=_DEFAULT_WORKERS)
    args = parser.parse_args()

    game = args.game
    extracted_key = f"{game.upper()}_EXTRACTED_DIR"

    meshes_dir = args.extracted_dir or ""
    meshes_is_root = False
    source = game
    if args.mod:
        if not args.external_mods_dir:
            print("ERROR: --mod requires --external-mods-dir.")
            exit(1)
        mod_dir = Path(args.external_mods_dir) / args.mod
        for candidate in (mod_dir / "Meshes", mod_dir / "meshes", mod_dir):
            if candidate.is_dir():
                meshes_dir = str(candidate)
                meshes_is_root = candidate.name.lower() == "meshes"
                source = f"ext:{args.mod}"
                break

    if not meshes_dir:
        print(f"ERROR: {extracted_key} not supplied. Pass --extracted-dir.")
        exit(1)

    meshes_path = Path(meshes_dir) if meshes_is_root else Path(meshes_dir) / "Meshes"
    if not meshes_path.is_dir():
        print(f"ERROR: Meshes directory not found: {meshes_path}")
        exit(1)

    db_path = Path(args.db_path)
    print(f"Building {db_path} from {meshes_path}...")
    print(f"  Workers: {args.workers} (of {os.cpu_count()} CPUs)")
    if args.embeddings:
        print("  Embeddings: enabled")
    stats = build_db(
        meshes_dir=meshes_path,
        db_path=db_path,
        game=game,
        source=source,
        cache_dir=args.cache_dir,
        build_embeddings=args.embeddings,
        num_workers=args.workers,
    )

    if args.stats:
        for k, v in stats.items():
            print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
