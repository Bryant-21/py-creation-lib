"""Build nifs.db from Fallout 4 NIF mesh files.

Scans extracted FO4 meshes and external mods for .nif files,
loads each with NifFile.load() to extract metadata (block types,
behaviors, textures, materials, sequences, particles), and indexes
everything into a SQLite FTS5 database.

Phase 1: Fast rglob to collect all .nif paths (seconds).
Phase 2: ProcessPoolExecutor with all CPUs for parallel extraction.
Phase 3: Batch write to SQLite via the Rust db_native backend
         — single wrapping transaction, columnar FFI, GIL released
         during the actual INSERTs.

Run:
  cd <your-checkout> && uv run python py_creation_lib/python/creation_lib/preprocessor/nifs.py
  cd <your-checkout> && uv run python py_creation_lib/python/creation_lib/preprocessor/nifs.py --mod SomeExternalMod
  cd <your-checkout> && uv run python py_creation_lib/python/creation_lib/preprocessor/nifs.py --embeddings
  cd <your-checkout> && uv run python py_creation_lib/python/creation_lib/preprocessor/nifs.py --workers 4
"""

import json
import os
import re
import sys
import time
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

from creation_lib.db.native_runtime import BulkInserter

from creation_lib.core.game_profiles import GAME_PROFILES  # noqa: E402

CATEGORY_PATTERNS = [
    (re.compile(r"(?:^|[\\/])Weapons[\\/]", re.I), "Weapon"),
    (re.compile(r"(?:^|[\\/])(?:Armor|Clothes)[\\/]", re.I), "Armor"),
    (re.compile(r"(?:^|[\\/])Actors[\\/]", re.I), "Actor"),
    (re.compile(r"(?:^|[\\/])Architecture[\\/]", re.I), "Architecture"),
    (re.compile(r"(?:^|[\\/])Effects[\\/]", re.I), "Effect"),
    (re.compile(r"(?:^|[\\/])Furniture[\\/]", re.I), "Furniture"),
    (re.compile(r"(?:^|[\\/])SetDressing[\\/]", re.I), "SetDressing"),
    (re.compile(r"(?:^|[\\/])(?:Interface|Pipboy)[\\/]", re.I), "Interface"),
    (re.compile(r"(?:^|[\\/])(?:AnimObjects|AnimTextData)[\\/]", re.I), "AnimObject"),
    (re.compile(r"(?:^|[\\/])Markers[\\/]", re.I), "Misc"),
]

# ---- Schema ---------------------------------------------------------------

NIFS_COLS = [
    "id", "name", "filename", "path", "category", "source", "source_path",
    "root_type", "block_count", "has_particles", "has_behavior",
    "has_controllers", "content",
]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "nifs", "pk": "id", "on_conflict": "REPLACE", "columns": NIFS_COLS},
        {"name": "nif_behavior_refs", "columns": ["nif_id", "behavior_path"]},
        {"name": "nif_textures", "columns": ["nif_id", "texture_path"]},
        {"name": "nif_materials", "columns": ["nif_id", "material_path"]},
        {"name": "nif_sequences", "columns": ["nif_id", "sequence_name"]},
        {"name": "nif_block_types", "columns": ["nif_id", "type_name", "count"]},
        {"name": "nif_material_textures", "on_conflict": "IGNORE",
         "columns": ["material_path", "material_type", "texture_slot", "texture_path"]},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS nifs (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    filename        TEXT NOT NULL,
    path            TEXT NOT NULL,
    category        TEXT NOT NULL,
    source          TEXT NOT NULL,
    source_path     TEXT NOT NULL,
    root_type       TEXT DEFAULT '',
    block_count     INTEGER DEFAULT 0,
    has_particles   INTEGER DEFAULT 0,
    has_behavior    INTEGER DEFAULT 0,
    has_controllers INTEGER DEFAULT 0,
    content         TEXT DEFAULT ''
);
CREATE VIRTUAL TABLE IF NOT EXISTS nifs_fts USING fts5(
    name, path, category, content,
    content=nifs, content_rowid=rowid
);
CREATE TABLE IF NOT EXISTS nif_behavior_refs (
    nif_id        TEXT NOT NULL REFERENCES nifs(id),
    behavior_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS nif_textures (
    nif_id       TEXT NOT NULL REFERENCES nifs(id),
    texture_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS nif_materials (
    nif_id        TEXT NOT NULL REFERENCES nifs(id),
    material_path TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS nif_sequences (
    nif_id        TEXT NOT NULL REFERENCES nifs(id),
    sequence_name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS nif_block_types (
    nif_id    TEXT NOT NULL REFERENCES nifs(id),
    type_name TEXT NOT NULL,
    count     INTEGER DEFAULT 1
);
CREATE TABLE IF NOT EXISTS nif_material_textures (
    material_path TEXT NOT NULL,
    material_type TEXT NOT NULL,
    texture_slot  TEXT NOT NULL,
    texture_path  TEXT NOT NULL,
    PRIMARY KEY (material_path, texture_slot)
);
"""

CREATE_INDEXES_SQL = """
CREATE INDEX IF NOT EXISTS idx_nifs_category ON nifs(category);
CREATE INDEX IF NOT EXISTS idx_nifs_source ON nifs(source);
CREATE INDEX IF NOT EXISTS idx_nifs_has_particles ON nifs(has_particles);
CREATE INDEX IF NOT EXISTS idx_nifs_has_behavior ON nifs(has_behavior);
CREATE INDEX IF NOT EXISTS idx_nif_behavior_refs_path ON nif_behavior_refs(behavior_path);
CREATE INDEX IF NOT EXISTS idx_nif_behavior_refs_nid ON nif_behavior_refs(nif_id);
CREATE INDEX IF NOT EXISTS idx_nif_textures_path ON nif_textures(texture_path);
CREATE INDEX IF NOT EXISTS idx_nif_textures_nid ON nif_textures(nif_id);
CREATE INDEX IF NOT EXISTS idx_nif_materials_path ON nif_materials(material_path);
CREATE INDEX IF NOT EXISTS idx_nif_materials_nid ON nif_materials(nif_id);
CREATE INDEX IF NOT EXISTS idx_nif_sequences_name ON nif_sequences(sequence_name);
CREATE INDEX IF NOT EXISTS idx_nif_sequences_nid ON nif_sequences(nif_id);
CREATE INDEX IF NOT EXISTS idx_nif_block_types_name ON nif_block_types(type_name);
CREATE INDEX IF NOT EXISTS idx_nif_block_types_nid ON nif_block_types(nif_id);
CREATE INDEX IF NOT EXISTS idx_nif_mat_tex_material ON nif_material_textures(material_path);
CREATE INDEX IF NOT EXISTS idx_nif_mat_tex_texture ON nif_material_textures(texture_path);
"""

# ---- Helpers --------------------------------------------------------------


def classify_category(rel_path: str) -> str:
    rel_path = rel_path.replace("\\", "/")
    for pattern, category in CATEGORY_PATTERNS:
        if pattern.search(rel_path):
            return category
    return "Misc"


def _find_meshes_dir(fo4_root: Path) -> Path | None:
    for candidate in (fo4_root / "Data" / "Meshes", fo4_root / "Meshes"):
        if candidate.is_dir():
            return candidate
    return None


SKIP_DIRS = {"facegendata", "precombined", "lod", "loadscreenart", "cameras"}


def collect_nif_paths(scan_dir: Path) -> list[tuple[str, str]]:
    results = []
    for root, dirs, files in os.walk(str(scan_dir)):
        dirs[:] = [d for d in dirs if d.lower() not in SKIP_DIRS]
        for fname in files:
            if fname.lower().endswith(".nif"):
                p = Path(root) / fname
                results.append((str(p), str(p.relative_to(scan_dir))))
    return results


def collect_external_mod_nifs(mod_dir: Path) -> tuple[list[tuple[str, str]], str]:
    scan_dir = mod_dir
    for candidate in ("Meshes", "meshes"):
        d = mod_dir / candidate
        if d.is_dir():
            scan_dir = d
            break
    return collect_nif_paths(scan_dir), str(scan_dir)


# ---- Parallel NIF extraction (worker) ------------------------------------

MAX_NIF_SIZE = 5 * 1024 * 1024

_INLINE_SHADER_TEXTURE_FIELDS = (
    "Source Texture",
    "Greyscale Texture",
    "Env Map Texture",
    "Normal Texture",
    "Env Mask Texture",
    "Reflectance Texture",
    "Lighting Texture",
    "Emit Gradient Texture",
)


def _extract_inline_shader_textures(block) -> list[str]:
    texs: list[str] = []
    for field_name in _INLINE_SHADER_TEXTURE_FIELDS:
        val = block.get_field(field_name)
        if isinstance(val, str) and val.strip():
            texs.append(val.strip())
    data = block.get_field("Shader Property Data")
    if isinstance(data, dict):
        for field_name in _INLINE_SHADER_TEXTURE_FIELDS:
            val = data.get(field_name)
            if isinstance(val, str) and val.strip():
                texs.append(val.strip())
    return texs


def _extract_worker(abs_path: str) -> tuple[dict | None, float]:
    from creation_lib.nif.nif_file import NifFile

    t0 = time.time()
    try:
        if os.path.getsize(abs_path) > MAX_NIF_SIZE:
            return None, time.time() - t0
        nif = NifFile.load(abs_path)
    except Exception as e:
        return {"error": f"{type(e).__name__}: {e}", "path": abs_path}, time.time() - t0

    result = {
        "block_count": len(nif.blocks),
        "root_type": nif.blocks[0].type_name if nif.blocks else "",
        "block_types": {},
        "behavior_refs": [],
        "textures": [],
        "materials": [],
        "sequences": [],
        "has_particles": False,
        "has_behavior": False,
        "has_controllers": False,
    }

    for block in nif.blocks:
        result["block_types"][block.type_name] = (
            result["block_types"].get(block.type_name, 0) + 1
        )
        if "Particle" in block.type_name or "PSys" in block.type_name:
            result["has_particles"] = True
        if block.type_name == "BSBehaviorGraphExtraData":
            bpath = block.get_field("Behaviour Graph File") or ""
            if bpath:
                result["behavior_refs"].append(bpath)
                result["has_behavior"] = True
        if block.type_name in ("BSLightingShaderProperty", "BSEffectShaderProperty"):
            mat = block.get_field("Name") or ""
            has_material_file = bool(
                mat and (mat.lower().endswith(".bgsm") or mat.lower().endswith(".bgem"))
            )
            if has_material_file:
                result["materials"].append(mat)
            else:
                for tex in _extract_inline_shader_textures(block):
                    result["textures"].append(tex)
        if block.type_name == "BSShaderTextureSet":
            textures = block.get_field("Textures")
            if isinstance(textures, list):
                for t in textures:
                    if isinstance(t, str) and t.strip():
                        result["textures"].append(t.strip())
        if block.type_name == "NiControllerSequence":
            seq_name = block.get_field("Name") or ""
            if seq_name:
                result["sequences"].append(seq_name)
        if block.type_name == "NiControllerManager":
            result["has_controllers"] = True

    result["textures"] = list(dict.fromkeys(result["textures"]))
    result["materials"] = list(dict.fromkeys(result["materials"]))
    result["sequences"] = list(dict.fromkeys(result["sequences"]))

    return result, time.time() - t0


# ---- FTS content ----------------------------------------------------------


def build_fts_content(name: str, rel_path: str, metadata: dict) -> str:
    parts = [name]
    for seg in rel_path.replace("\\", "/").split("/"):
        stem = seg.rsplit(".", 1)[0] if "." in seg else seg
        if stem.lower() not in {"meshes"} and stem != name:
            parts.append(stem)
    if metadata["block_types"]:
        parts.append("blocks: " + " ".join(sorted(metadata["block_types"])))
    for bref in metadata["behavior_refs"]:
        parts.append(bref)
    for tex in metadata["textures"][:20]:
        parts.append(tex)
    for mat in metadata["materials"][:20]:
        parts.append(mat)
    for seq in metadata["sequences"][:20]:
        parts.append(seq)
    if metadata["has_particles"]:
        parts.append("particle particles")
    if metadata["has_behavior"]:
        parts.append("behavior animated")
    if metadata["has_controllers"]:
        parts.append("controller animation")
    return " ".join(parts)


# ---- Columnar buffer ------------------------------------------------------


def new_columnar_buffer():
    return {
        "nifs": {c: [] for c in NIFS_COLS},
        "nif_behavior_refs": {"nif_id": [], "behavior_path": []},
        "nif_textures": {"nif_id": [], "texture_path": []},
        "nif_materials": {"nif_id": [], "material_path": []},
        "nif_sequences": {"nif_id": [], "sequence_name": []},
        "nif_block_types": {"nif_id": [], "type_name": [], "count": []},
    }


def _buffer_row_count(buf: dict) -> int:
    """Sum of main-table row counts — used for flush-trigger heuristics."""
    return len(buf["nifs"]["id"])


def flush_buffer(bulk: BulkInserter, buf: dict) -> None:
    for table, cols in buf.items():
        if not cols:
            continue
        n = len(next(iter(cols.values())))
        if n == 0:
            continue
        bulk.add_chunk(table, cols)
        for lst in cols.values():
            lst.clear()


def append_nif(
    buf: dict,
    nif_id: str,
    name: str,
    filename: str,
    rel_path: str,
    category: str,
    source: str,
    source_path: str,
    metadata: dict,
) -> None:
    content = build_fts_content(name, rel_path, metadata)
    nifs = buf["nifs"]
    nifs["id"].append(nif_id)
    nifs["name"].append(name)
    nifs["filename"].append(filename)
    nifs["path"].append(rel_path.replace("\\", "/"))
    nifs["category"].append(category)
    nifs["source"].append(source)
    nifs["source_path"].append(source_path)
    nifs["root_type"].append(metadata["root_type"])
    nifs["block_count"].append(int(metadata["block_count"]))
    nifs["has_particles"].append(1 if metadata["has_particles"] else 0)
    nifs["has_behavior"].append(1 if metadata["has_behavior"] else 0)
    nifs["has_controllers"].append(1 if metadata["has_controllers"] else 0)
    nifs["content"].append(content)

    for bref in metadata["behavior_refs"]:
        buf["nif_behavior_refs"]["nif_id"].append(nif_id)
        buf["nif_behavior_refs"]["behavior_path"].append(bref)
    for tex in metadata["textures"]:
        buf["nif_textures"]["nif_id"].append(nif_id)
        buf["nif_textures"]["texture_path"].append(tex)
    for mat in metadata["materials"]:
        buf["nif_materials"]["nif_id"].append(nif_id)
        buf["nif_materials"]["material_path"].append(mat)
    for seq in metadata["sequences"]:
        buf["nif_sequences"]["nif_id"].append(nif_id)
        buf["nif_sequences"]["sequence_name"].append(seq)
    for type_name, count in metadata["block_types"].items():
        buf["nif_block_types"]["nif_id"].append(nif_id)
        buf["nif_block_types"]["type_name"].append(type_name)
        buf["nif_block_types"]["count"].append(int(count))


# ---- Incremental source delete -------------------------------------------


def delete_source_data(bulk: BulkInserter, source: str) -> None:
    """Remove all rows for a given `source` value, cascading through children."""
    child_tables = [
        "nif_behavior_refs",
        "nif_textures",
        "nif_materials",
        "nif_sequences",
        "nif_block_types",
    ]
    for tbl in child_tables:
        bulk.execute_params(
            f"DELETE FROM {tbl} WHERE nif_id IN (SELECT id FROM nifs WHERE source = ?)",
            [source],
        )
    bulk.execute_params("DELETE FROM nifs WHERE source = ?", [source])


# ---- Batch processing ----------------------------------------------------


def process_nif_batch(
    bulk: BulkInserter,
    paths,
    source,
    source_path,
    category_counts,
    source_counts,
    num_workers,
    label,
    timing_log=None,
):
    native_index_nifs = getattr(bulk, "index_nifs", None)
    if callable(native_index_nifs):
        t0 = time.time()
        timing_log_path = (
            str(timing_log)
            if isinstance(timing_log, (str, Path))
            else getattr(timing_log, "name", None)
        )
        summary = native_index_nifs(
            list(paths),
            source,
            source_path,
            max_size=MAX_NIF_SIZE,
            workers=num_workers,
            timing_log_path=timing_log_path,
        )
        indexed = int(summary.get("indexed", 0))
        errors = int(summary.get("errors", 0)) + int(summary.get("skipped", 0))
        for category, count in dict(summary.get("category_counts") or {}).items():
            category_counts[category] = category_counts.get(category, 0) + int(count)
        source_counts[source] = source_counts.get(source, 0) + indexed
        elapsed = time.time() - t0
        rate = len(paths) / elapsed if elapsed > 0 else 0
        print(
            f"    [{label}] Native index: {indexed:,} indexed, {errors:,} errors/skipped "
            f"in {elapsed:.1f}s ({rate:.0f}/s)",
            flush=True,
        )
        return indexed, errors

    total_indexed = 0
    total_errors = 0
    total_count = len(paths)
    t0 = time.time()
    buf = new_columnar_buffer()
    close_timing_log = False
    if isinstance(timing_log, (str, Path)):
        timing_log = open(str(timing_log), "a", encoding="utf-8")
        close_timing_log = True

    with ProcessPoolExecutor(max_workers=num_workers) as pool:
        future_map = {
            pool.submit(_extract_worker, abs_path): rel_path
            for abs_path, rel_path in paths
        }

        for done, future in enumerate(as_completed(future_map), start=1):
            rel_path = future_map[future]
            metadata, elapsed = future.result()

            is_error = metadata is None or "error" in (
                metadata if isinstance(metadata, dict) else {}
            )
            if timing_log is not None:
                status = "ok" if not is_error else "ERR"
                err_msg = ""
                if is_error and isinstance(metadata, dict) and "error" in metadata:
                    err_msg = f"\t{metadata['error']}"
                timing_log.write(f"{elapsed:.3f}s\t{status}\t{rel_path}{err_msg}\n")

            if is_error:
                total_errors += 1
            else:
                name = Path(rel_path).stem
                filename = Path(rel_path).name
                category = classify_category(rel_path)
                nif_id = f"{source}/{rel_path.replace(chr(92), '/')}"
                append_nif(
                    buf, nif_id, name, filename, rel_path, category,
                    source, source_path, metadata,
                )
                category_counts[category] = category_counts.get(category, 0) + 1
                source_counts[source] = source_counts.get(source, 0) + 1
                total_indexed += 1

            if done % 2000 == 0 or done == total_count:
                flush_buffer(bulk, buf)
                elapsed = time.time() - t0
                rate = done / elapsed if elapsed > 0 else 0
                pct = done * 100 // total_count
                eta = (total_count - done) / rate if rate > 0 else 0
                print(
                    f"    [{label}] {done:,}/{total_count:,} ({pct}%) "
                    f"| {total_indexed:,} ok, {total_errors:,} err "
                    f"| {rate:.0f}/s, ETA {eta:.0f}s",
                    flush=True,
                )

    flush_buffer(bulk, buf)
    if close_timing_log:
        timing_log.close()
    elapsed = time.time() - t0
    print(
        f"    [{label}] Done: {total_indexed:,} indexed, {total_errors:,} errors in {elapsed:.1f}s",
        flush=True,
    )
    return total_indexed, total_errors


# ---- Material textures ---------------------------------------------------


def _index_material_textures(bulk: BulkInserter, meshes_dir: Path) -> None:
    from creation_lib.material_tools.extract_textures import parse_material_textures

    rows = bulk.query_all("SELECT DISTINCT material_path FROM nif_materials")
    mat_paths = [r[0] for r in rows]
    print(f"  Parsing {len(mat_paths)} unique material files...", flush=True)

    data_dir = meshes_dir.parent
    parsed_count = 0
    missing_count = 0

    mt_buf = {"material_path": [], "material_type": [], "texture_slot": [], "texture_path": []}

    for mat_rel in mat_paths:
        mat_normalized = mat_rel.replace("\\", "/").strip()
        abs_path = data_dir / mat_normalized
        if not abs_path.exists():
            alt = data_dir / mat_normalized.lower()
            if alt.exists():
                abs_path = alt
            else:
                missing_count += 1
                continue

        result = parse_material_textures(str(abs_path))
        if result is None:
            continue

        mat_key = mat_normalized.lower()
        for slot, tex_path in result["textures"].items():
            mt_buf["material_path"].append(mat_key)
            mt_buf["material_type"].append(result["type"])
            mt_buf["texture_slot"].append(slot)
            mt_buf["texture_path"].append(tex_path)
        parsed_count += 1

    if mt_buf["material_path"]:
        bulk.add_chunk("nif_material_textures", mt_buf)
    print(f"  Parsed: {parsed_count}, Missing: {missing_count}", flush=True)


# ---- Main build ----------------------------------------------------------


def build_db(
    env,
    mod_name_filter=None,
    build_embeddings=False,
    num_workers=None,
    db_path=None,
    external_mods_dir=None,
    game="fo4",
):
    if db_path is None:
        raise ValueError("db_path is required")
    db_path = Path(db_path)
    external_mods_dir = Path(external_mods_dir) if external_mods_dir is not None else None
    os.makedirs(str(db_path.parent), exist_ok=True)

    if num_workers is None:
        num_workers = max(1, (os.cpu_count() or 4) // 2)

    fresh = not mod_name_filter  # full rebuild drops the DB

    total = 0
    errors = 0
    category_counts: dict[str, int] = {}
    source_counts: dict[str, int] = {}
    meshes_dir: Path | None = None

    timing_log_path = db_path.parent / "nifs_timing.log"
    timing_log_path.write_text("elapsed\tstatus\tpath\n", encoding="utf-8")
    print(f"  Timing log: {timing_log_path}", flush=True)

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=fresh) as bulk:
        bulk.execute(CREATE_TABLES_DDL)

        if mod_name_filter:
            delete_source_data(bulk, f"ext:{mod_name_filter}")

        # Base game
        if not mod_name_filter:
            extracted_env_key = GAME_PROFILES[game].env_var_name
            game_dir = env.get(extracted_env_key, "")
            if game_dir:
                meshes_dir = _find_meshes_dir(Path(game_dir))
                if meshes_dir:
                    source_path = str(meshes_dir)
                    print(
                        f"\n[{game.upper()}] Collecting NIF paths from {meshes_dir}...",
                        flush=True,
                    )
                    t0 = time.time()
                    paths = collect_nif_paths(meshes_dir)
                    print(
                        f"    Found {len(paths):,} NIFs in {time.time() - t0:.1f}s",
                        flush=True,
                    )
                    print(
                        f"    Extracting with {num_workers} worker processes...",
                        flush=True,
                    )
                    indexed, errs = process_nif_batch(
                        bulk, paths, game, source_path,
                        category_counts, source_counts, num_workers,
                        game.upper(), timing_log_path,
                    )
                    total += indexed
                    errors += errs
                else:
                    print(
                        f"  ERROR: Could not find Meshes directory under {extracted_env_key}={game_dir}"
                    )
                    print("  Looked for:")
                    print(f"    {Path(game_dir) / 'Data' / 'Meshes'}")
                    print(f"    {Path(game_dir) / 'Meshes'}")
                    print(
                        f"  Make sure {extracted_env_key} points to your extracted game root"
                    )
                    print("  (the folder containing Data/Meshes/ or Meshes/).")
                    sys.exit(1)
            else:
                print(f"  WARNING: {extracted_env_key} not supplied")

        # External mods
        if external_mods_dir is not None and external_mods_dir.is_dir():
            for mod_name in sorted(os.listdir(str(external_mods_dir))):
                mod_dir = external_mods_dir / mod_name
                if not mod_dir.is_dir():
                    continue
                if mod_name_filter and mod_name != mod_name_filter:
                    continue
                game_file = mod_dir / ".game"
                mod_game = (
                    game_file.read_text(encoding="utf-8").strip()
                    if game_file.exists()
                    else "fo4"
                )
                if mod_game != game:
                    continue
                source = f"ext:{mod_name}"
                paths, scan_dir = collect_external_mod_nifs(mod_dir)
                if not paths:
                    continue
                print(f"\n[ext:{mod_name}] Found {len(paths)} NIF files", flush=True)
                indexed, errs = process_nif_batch(
                    bulk, paths, source, scan_dir,
                    category_counts, source_counts, num_workers,
                    f"ext:{mod_name}", timing_log_path,
                )
                total += indexed
                errors += errs

        if not mod_name_filter:
            print("\n  Building indexes...", flush=True)
            bulk.create_indexes(CREATE_INDEXES_SQL)
        print("\n  Rebuilding FTS index...", flush=True)
        bulk.rebuild_fts("nifs_fts")

        if meshes_dir:
            _index_material_textures(bulk, meshes_dir)

    # `with` has committed; run embeddings on the finalized DB.
    if build_embeddings and total > 0:
        print("\nBuilding embeddings...", flush=True)
        _build_embeddings(str(db_path))

    return total, errors, category_counts, source_counts


def _build_embeddings(db_path: str) -> None:
    """Collect nif rows via a fresh read connection, then build the vec index."""
    from creation_lib.db.embeddings import build_vec_index
    from creation_lib.db.native_runtime import Database

    texts: list[str] = []
    ids: list[str] = []
    # Use a short-lived read connection via BulkInserter.query_all under a no-op txn.
    # We can't easily open a read handle here since BulkInserter already closed; use a
    # fresh rw BulkInserter only to read (cheap — no INSERTs, single SELECT).
    with BulkInserter(db_path, BULK_SCHEMA, fresh=False) as bulk:
        rows = bulk.query_all("SELECT id, name, category, content FROM nifs ORDER BY rowid")
    for r in rows:
        texts.append(f"{r[1]} {r[2]} {r[3]}".strip())
        ids.append(r[0])
    if texts:
        print(f"  nifs: {len(texts)} documents")
        build_vec_index(texts, ids, db_path, table_name="nifs_embeddings")


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build NIF mesh database")
    parser.add_argument(
        "--game", default="fo4", help="Game identifier (fo4, fo76, skyrimse, starfield)"
    )
    parser.add_argument("--mod", default=None, help="Only index this external mod name")
    parser.add_argument(
        "--workers",
        type=int,
        default=max(1, (os.cpu_count() or 4) // 2),
        help="Parallel worker count",
    )
    parser.add_argument("--embeddings", action="store_true", help="Build embeddings index")
    parser.add_argument("--extracted-dir", default=None)
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--external-mods-dir", default=None)
    args = parser.parse_args()

    game = args.game
    if game not in GAME_PROFILES:
        print(
            f"ERROR: Invalid --game '{game}'. Valid: {', '.join(sorted(GAME_PROFILES))}"
        )
        sys.exit(1)

    env: dict[str, str] = {}
    extracted_env_key = GAME_PROFILES[game].env_var_name
    if args.extracted_dir:
        env[extracted_env_key] = args.extracted_dir

    db_path = Path(args.db_path)

    mod_name_filter = args.mod
    build_embeddings = args.embeddings
    num_workers = args.workers

    mode = f"incremental ({mod_name_filter})" if mod_name_filter else "full rebuild"
    print(f"Building NIFs database [{game}] [{mode}]")
    print(f"  Database: {db_path}")
    print(f"  Workers: {num_workers} (of {os.cpu_count()} CPUs)")
    if env.get(extracted_env_key):
        print(f"  {game.upper()} source: {env[extracted_env_key]}")
    if build_embeddings:
        print("  Embeddings: enabled")
    print(flush=True)

    if not mod_name_filter and not env.get(extracted_env_key):
        print(f"ERROR: {extracted_env_key} is not set.")
        print(f"  Pass --extracted-dir.")
        sys.exit(1)

    total, errors, category_counts, source_counts = build_db(
        env,
        mod_name_filter,
        build_embeddings,
        num_workers,
        db_path,
        args.external_mods_dir,
        game,
    )

    print("\nDatabase built successfully!")
    print(f"  Total NIFs: {total:,}")
    if errors:
        print(f"  Errors (skipped): {errors:,}")

    if source_counts:
        print("\n  NIFs by source:")
        for src in sorted(source_counts, key=source_counts.get, reverse=True):
            print(f"    {src}: {source_counts[src]:,}")

    if category_counts:
        print("\n  NIFs by category:")
        for cat in sorted(category_counts, key=category_counts.get, reverse=True):
            print(f"    {cat}: {category_counts[cat]:,}")


if __name__ == "__main__":
    main()
