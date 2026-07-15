"""Build scripts.db from Papyrus .psc source files."""

import json
import os
import re
import sys
from pathlib import Path

from creation_lib.core.game_profiles import GAME_PROFILES
from creation_lib.db.native_runtime import BulkInserter
from creation_lib.db.tokenizer import tokenize

FRAGMENT_PREFIXES = ("TIF_", "QF_", "SF_", "TERM_", "PF_", "PRKF_")
MAX_CONTENT_LEN = 4000

RE_SCRIPTNAME = re.compile(r"^ScriptName\s+(\S+)", re.IGNORECASE | re.MULTILINE)
RE_EXTENDS = re.compile(
    r"^ScriptName\s+\S+\s+extends\s+(\S+)", re.IGNORECASE | re.MULTILINE
)
RE_PROPERTY = re.compile(
    r"^\s*(\w[\w\[\]]*)\s+Property\s+(\w+)", re.IGNORECASE | re.MULTILINE
)
RE_FUNCTION = re.compile(
    r"^\s*(?:(\w[\w\[\]]*)\s+)?Function\s+(\w+)\s*\(([^)]*)\)",
    re.IGNORECASE | re.MULTILINE,
)
RE_EVENT = re.compile(r"^\s*Event\s+(\w+)\s*\(([^)]*)\)", re.IGNORECASE | re.MULTILINE)

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


def get_script_roots(game: str, env: dict) -> list:
    if game == "starfield":
        roots = []
        sf_dir = env.get("STARFIELD_DIR", "")
        if sf_dir:
            roots.append(Path(sf_dir) / "Data" / "Scripts" / "Source")
        sf_cr = env.get("STARFIELD_CONTENT_RESOURCES_DIR", "")
        if sf_cr:
            roots.append(Path(sf_cr))
        if not roots:
            print("ERROR: no Starfield script roots supplied")
            sys.exit(1)
        return roots

    env_var = f"{game.upper()}_DIR"
    game_dir = env.get(env_var, "")
    if not game_dir:
        print(f"ERROR: {env_var} not supplied")
        sys.exit(1)
    return [Path(game_dir) / "Data" / "Scripts" / "Source"]


def categorize(source_dir, rel_path):
    basename = os.path.basename(rel_path)
    name_no_ext = os.path.splitext(basename)[0]
    if any(name_no_ext.startswith(p) for p in FRAGMENT_PREFIXES):
        return "fragment"
    if "Fragments" in rel_path:
        return "fragment"
    if source_dir.lower() == "base":
        return "api"
    return "dlc"


def parse_script(filepath):
    try:
        with open(filepath, encoding="utf-8", errors="replace") as f:
            raw = f.read()
    except Exception:
        return None

    m = RE_SCRIPTNAME.search(raw)
    script_name = m.group(1) if m else os.path.splitext(os.path.basename(filepath))[0]

    m = RE_EXTENDS.search(raw)
    extends = m.group(1) if m else ""

    props = []
    for m in RE_PROPERTY.finditer(raw):
        props.append(f"{m.group(1)} {m.group(2)}")
    properties = "; ".join(props)

    funcs = []
    for m in RE_FUNCTION.finditer(raw):
        ret_type = m.group(1) or "None"
        funcs.append(f"{ret_type} {m.group(2)}({m.group(3).strip()})")
    functions = "; ".join(funcs)

    evts = []
    for m in RE_EVENT.finditer(raw):
        evts.append(f"{m.group(1)}({m.group(2).strip()})")
    events = "; ".join(evts)

    content = raw[:MAX_CONTENT_LEN]
    return script_name, extends, properties, functions, events, content


def scan_scripts(script_roots: list):
    for root in script_roots:
        root = Path(root)
        if not root.is_dir():
            print(f"  Skipping {root} (not found)")
            continue

        try:
            subdirs = sorted([d for d in root.iterdir() if d.is_dir()])
        except Exception:
            subdirs = []

        for subdir in subdirs:
            source_dir = subdir.name
            if source_dir.lower() == "user":
                continue

            print(f"  Scanning {source_dir}/...")
            count = 0
            skipped = 0

            for walk_root, dirs, files in os.walk(str(subdir)):
                for fname in files:
                    if not fname.lower().endswith(".psc"):
                        continue

                    rel_path = os.path.relpath(
                        os.path.join(walk_root, fname), str(subdir)
                    )
                    category = categorize(source_dir, rel_path)

                    if category == "fragment":
                        skipped += 1
                        continue

                    filepath = os.path.join(walk_root, fname)
                    result = parse_script(filepath)
                    if result is None:
                        continue

                    script_name, extends, properties, functions, events, content = result
                    script_id = f"{source_dir}/{script_name}"

                    count += 1
                    yield (
                        script_id, script_name, fname, extends, source_dir, category,
                        properties, functions, events, content, filepath,
                    )

            print(f"    Indexed {count} scripts, skipped {skipped} fragments")


def build_db(script_roots: list, db_path, build_embeddings=False):
    total = 0
    source_counts: dict[str, int] = {}
    category_counts: dict[str, int] = {}
    extends_counts: dict[str, int] = {}
    embed_texts: list[str] = []
    embed_doc_ids: list[str] = []

    db_path_p = Path(db_path)
    os.makedirs(str(db_path_p.parent), exist_ok=True)

    with BulkInserter(str(db_path_p), BULK_SCHEMA, fresh=True) as bulk:
        bulk.execute(CREATE_TABLES_DDL)
        buf = {c: [] for c in SCRIPT_COLS}

        for (
            script_id, script_name, filename, extends, source, category,
            properties, functions, events, content, filepath,
        ) in scan_scripts(script_roots):
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

            embed_texts.append(f"{script_name} {extends} {functions[:200]} {events[:100]}")
            embed_doc_ids.append(script_id)

            source_counts[source] = source_counts.get(source, 0) + 1
            category_counts[category] = category_counts.get(category, 0) + 1
            if extends:
                extends_counts[extends] = extends_counts.get(extends, 0) + 1
            total += 1

            if len(buf["script_id"]) >= 2000:
                bulk.add_chunk("scripts", buf)
                for lst in buf.values():
                    lst.clear()

        if buf["script_id"]:
            bulk.add_chunk("scripts", buf)

        bulk.create_indexes(CREATE_INDEXES_SQL)
        bulk.rebuild_fts("scripts_fts")

    if build_embeddings and embed_texts:
        from creation_lib.db.embeddings import build_vec_index
        print(f"\nBuilding sqlite-vec index ({len(embed_texts)} documents)...")
        build_vec_index(embed_texts, embed_doc_ids, str(db_path_p))

    return total, source_counts, category_counts, extends_counts


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build Papyrus scripts database")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--embeddings", action="store_true")
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--game-dir", default="")
    parser.add_argument("--content-resources-dir", default="")
    parser.add_argument("--script-root", action="append", default=[])
    args = parser.parse_args()
    build_embeddings = args.embeddings
    game = args.game

    if game not in GAME_PROFILES:
        print(f"ERROR: Invalid --game '{game}'. Valid: {', '.join(sorted(GAME_PROFILES))}")
        sys.exit(1)

    env: dict[str, str] = {}
    if args.game_dir:
        env[f"{game.upper()}_DIR"] = args.game_dir
    if args.content_resources_dir:
        env["STARFIELD_CONTENT_RESOURCES_DIR"] = args.content_resources_dir

    db_path = args.db_path
    script_roots = [Path(p) for p in args.script_root] if args.script_root else get_script_roots(game, env)

    print(f"Building Papyrus scripts database [{game}]")
    print(f"  Script roots: {[str(r) for r in script_roots]}")
    print(f"  Database output: {db_path}")
    print()

    total, source_counts, category_counts, extends_counts = build_db(
        script_roots, db_path, build_embeddings
    )

    print("\nDatabase built successfully!")
    print(f"  Total scripts: {total:,}")
    print("\n  By source:")
    for src in sorted(source_counts, key=source_counts.get, reverse=True):
        print(f"    {src}: {source_counts[src]:,}")
    print("\n  By category:")
    for cat in sorted(category_counts, key=category_counts.get, reverse=True):
        print(f"    {cat}: {category_counts[cat]:,}")
    print("\n  Top parent types (extends):")
    for ext in sorted(extends_counts, key=extends_counts.get, reverse=True)[:15]:
        print(f"    {ext}: {extends_counts[ext]:,}")


if __name__ == "__main__":
    main()
