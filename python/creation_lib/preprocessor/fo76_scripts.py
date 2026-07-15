"""Build fo76_scripts.db from compiled Papyrus .pex files in FO76_EXTRACTED_DIR.

Unlike other games, FO76 ships only compiled .pex scripts (no source .psc).
This script decompiles each .pex using creation_lib.pex, extracts structured metadata,
and indexes it into a SQLite FTS5 database with the same schema as scripts.db.
"""

import json
import os
import sys
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

from creation_lib.db.native_runtime import BulkInserter
from creation_lib.db.tokenizer import tokenize
from creation_lib.pex import decompile_pex, parse_pex

FRAGMENT_PREFIXES = ("TIF_", "QF_", "SF_", "TERM_", "PF_", "PRKF_")
MAX_CONTENT_LEN = 4000
DEFAULT_WORKERS = 8

SCRIPT_COLS = [
    "script_id", "script_name", "script_name_tokens", "filename", "extends",
    "source", "category", "properties", "functions", "events", "content", "script_path",
]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "scripts", "pk": "script_id", "on_conflict": "REPLACE", "columns": SCRIPT_COLS},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS scripts (
    script_id TEXT PRIMARY KEY,
    script_name TEXT,
    script_name_tokens TEXT,
    filename TEXT,
    extends TEXT,
    source TEXT,
    category TEXT,
    properties TEXT,
    functions TEXT,
    events TEXT,
    content TEXT,
    script_path TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS scripts_fts USING fts5(
    script_name_tokens, functions, events, content,
    content='scripts', content_rowid='rowid'
);
"""

CREATE_INDEXES_SQL = """
CREATE INDEX IF NOT EXISTS idx_scripts_source ON scripts(source);
CREATE INDEX IF NOT EXISTS idx_scripts_category ON scripts(category);
CREATE INDEX IF NOT EXISTS idx_scripts_extends ON scripts(extends);
CREATE INDEX IF NOT EXISTS idx_scripts_name ON scripts(script_name);
"""


def is_fragment(stem: str) -> bool:
    return any(stem.startswith(p) for p in FRAGMENT_PREFIXES)


def _extract_metadata(pex_path: Path):
    pex = parse_pex(pex_path)
    if not pex.objects:
        return None

    obj = pex.objects[0]
    script_name = obj.name
    extends = obj.parent or ""

    props = []
    for prop in obj.properties:
        props.append(f"{prop.type} {prop.name}")
    properties_str = "; ".join(props)

    _EVENT_LOWER = {
        "oninit", "onactivate", "onhit", "ondying", "ondeath", "onload",
        "onunload", "oncellattach", "oncelldetach", "onreset", "onopen",
        "onclose", "ontriggerenter", "ontriggerleave", "onequipped",
        "onunequipped", "oncontainerchanged", "onitemadded", "onitemremoved",
        "onread", "onsell", "onworkshopobjectplaced",
        "onworkshopobjectdestroyed", "onworkshopobjectmoved", "ontimer",
        "onplayerloadgame", "onbeginstate", "onendstate", "onaliaschanged",
        "onaliasreset", "onaliasshutdown", "oneffectstart", "oneffectfinish",
        "onmagiceffectapply", "onobjectequipped", "onobjectunequipped",
        "oncombatstatetchanged", "onlocationchange", "onpackagechange",
        "onpackagestart", "onpackageend",
    }

    funcs = []
    evts = []
    for state in obj.states:
        for fn in state.functions:
            param_str = ", ".join(f"{p.type} {p.name}" for p in fn.params)
            sig = f"{fn.return_type or 'None'} {fn.name}({param_str})"
            if fn.name.lower() in _EVENT_LOWER:
                evts.append(f"{fn.name}({param_str})")
            else:
                funcs.append(sig)

    return script_name, extends, properties_str, "; ".join(funcs), "; ".join(evts)


def decompile_worker(pex_path: Path):
    stem = pex_path.stem
    if is_fragment(stem):
        return None
    try:
        meta = _extract_metadata(pex_path)
        if meta is None:
            return None
        script_name, extends, properties, functions, events = meta
        try:
            source_text = decompile_pex(pex_path)
        except Exception:
            source_text = f"; Decompilation failed for {stem}\n"
        content = source_text[:MAX_CONTENT_LEN]
        return (
            script_name, extends, properties, functions, events, content, str(pex_path),
        )
    except Exception as exc:
        print(f"  [WARN] Failed to parse {pex_path.name}: {exc}")
        return None


def find_pex_files(scripts_dir: Path) -> list[Path]:
    pex_files = []
    for root, _dirs, files in os.walk(str(scripts_dir)):
        for fname in files:
            if fname.lower().endswith(".pex"):
                pex_files.append(Path(root) / fname)
    return pex_files


def build_db(scripts_dir: Path, db_path: Path, workers: int) -> tuple[int, int, int]:
    pex_files = find_pex_files(scripts_dir)
    total_found = len(pex_files)
    print(f"  Found {total_found:,} .pex files under {scripts_dir}")
    print(f"  Decompiling with {workers} workers...")

    os.makedirs(str(db_path.parent), exist_ok=True)

    indexed = 0
    skipped_fragment = 0
    skipped_error = 0

    with BulkInserter(str(db_path), BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        buf = {c: [] for c in SCRIPT_COLS}

        with ThreadPoolExecutor(max_workers=workers) as pool:
            futures = {pool.submit(decompile_worker, p): p for p in pex_files}
            done_count = 0

            for future in as_completed(futures):
                done_count += 1
                if done_count % 500 == 0 or done_count == total_found:
                    print(
                        f"    {done_count:,}/{total_found:,} processed "
                        f"({indexed:,} indexed)..."
                    )

                result = future.result()
                if result is None:
                    pex_path = futures[future]
                    if is_fragment(pex_path.stem):
                        skipped_fragment += 1
                    else:
                        skipped_error += 1
                    continue

                script_name, extends, properties, functions, events, content, filepath = result
                filename = Path(filepath).name
                source = "client"
                category = "api"
                script_id = f"client/{script_name}"
                script_name_tokens = tokenize(script_name)

                buf["script_id"].append(script_id)
                buf["script_name"].append(script_name)
                buf["script_name_tokens"].append(script_name_tokens)
                buf["filename"].append(filename)
                buf["extends"].append(extends.lower() if extends else "")
                buf["source"].append(source)
                buf["category"].append(category)
                buf["properties"].append(properties)
                buf["functions"].append(functions)
                buf["events"].append(events)
                buf["content"].append(content)
                buf["script_path"].append(filepath)
                indexed += 1

                if len(buf["script_id"]) >= 500:
                    bulk.add_chunk("scripts", buf)
                    for lst in buf.values():
                        lst.clear()

        if buf["script_id"]:
            bulk.add_chunk("scripts", buf)

        print("  Building indexes...")
        bulk.create_indexes(CREATE_INDEXES_SQL)
        print("  Building FTS index...")
        bulk.rebuild_fts("scripts_fts")

    return indexed, skipped_fragment, skipped_error


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build Fallout 76 Papyrus scripts database")
    parser.add_argument("--extracted-dir", default="")
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    args = parser.parse_args()

    workers = DEFAULT_WORKERS
    workers = args.workers

    fo76_extracted = args.extracted_dir
    if not fo76_extracted:
        print("ERROR: FO76_EXTRACTED_DIR not supplied. Pass --extracted-dir.")
        sys.exit(1)

    scripts_dir = Path(fo76_extracted) / "scripts"
    if not scripts_dir.is_dir():
        print(f"ERROR: Scripts directory not found: {scripts_dir}")
        sys.exit(1)

    db_path = Path(args.db_path)

    print("Building Fallout 76 Papyrus scripts database")
    print(f"  Source:   {scripts_dir}")
    print(f"  Output:   {db_path}")
    print(f"  Workers:  {workers}")
    print()

    indexed, skipped_frag, skipped_err = build_db(scripts_dir, db_path, workers)

    print()
    print("Database built successfully!")
    print(f"  Indexed:          {indexed:,}")
    print(f"  Skipped (frags):  {skipped_frag:,}")
    print(f"  Skipped (errors): {skipped_err:,}")


if __name__ == "__main__":
    main()
