"""Build external_mods.db from third-party external mods (YAML records, scripts, READMEs).

Three table groups in one DB: ext_records, ext_scripts, ext_readmes.
"""

import json
import os
import re
import sys
import yaml

from creation_lib.db.native_runtime import BulkInserter
from creation_lib.db.tokenizer import tokenize

_FORMKEY_RE = re.compile(r"[0-9A-Fa-f]{6}:[A-Za-z0-9_]+\.es[mp]")

MAX_CONTENT_LEN = 2000
MAX_SCRIPT_LEN = 4000
MAX_README_LEN = 8000

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

EXT_REC_COLS = [
    "form_key", "editor_id", "editor_id_tokens", "record_type", "name", "name_tokens",
    "mod_name", "keywords", "yaml_path", "content",
]
EXT_SCR_COLS = [
    "script_id", "script_name", "script_name_tokens", "filename", "extends", "mod_name",
    "properties", "functions", "events", "content", "script_path",
]
EXT_README_COLS = ["mod_name", "title", "title_tokens", "content", "readme_path"]
REF_COLS = ["referencing_form_key", "referenced_form_key"]

BULK_SCHEMA = json.dumps({
    "tables": [
        {"name": "ext_records", "pk": "form_key", "on_conflict": "REPLACE", "columns": EXT_REC_COLS},
        {"name": "ext_scripts", "pk": "script_id", "on_conflict": "REPLACE", "columns": EXT_SCR_COLS},
        {"name": "ext_readmes", "pk": "mod_name", "on_conflict": "REPLACE", "columns": EXT_README_COLS},
        {"name": "record_refs", "columns": REF_COLS},
    ],
})

CREATE_TABLES_DDL = """
CREATE TABLE IF NOT EXISTS ext_records (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    editor_id_tokens TEXT,
    record_type TEXT,
    name TEXT,
    name_tokens TEXT,
    mod_name TEXT,
    keywords TEXT,
    yaml_path TEXT,
    content TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS ext_records_fts USING fts5(
    editor_id_tokens, name_tokens, keywords, content,
    content='ext_records', content_rowid='rowid'
);
CREATE TABLE IF NOT EXISTS ext_scripts (
    script_id TEXT PRIMARY KEY,
    script_name TEXT,
    script_name_tokens TEXT,
    filename TEXT,
    extends TEXT,
    mod_name TEXT,
    properties TEXT,
    functions TEXT,
    events TEXT,
    content TEXT,
    script_path TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS ext_scripts_fts USING fts5(
    script_name_tokens, functions, events, content,
    content='ext_scripts', content_rowid='rowid'
);
CREATE TABLE IF NOT EXISTS ext_readmes (
    mod_name TEXT PRIMARY KEY,
    title TEXT,
    title_tokens TEXT,
    content TEXT,
    readme_path TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS ext_readmes_fts USING fts5(
    title_tokens, content,
    content='ext_readmes', content_rowid='rowid'
);
CREATE TABLE IF NOT EXISTS record_refs (
    referencing_form_key TEXT,
    referenced_form_key TEXT
);
"""

CREATE_INDEXES_SQL = """
CREATE INDEX IF NOT EXISTS idx_ext_records_mod ON ext_records(mod_name);
CREATE INDEX IF NOT EXISTS idx_ext_records_type ON ext_records(record_type);
CREATE INDEX IF NOT EXISTS idx_ext_scripts_mod ON ext_scripts(mod_name);
CREATE INDEX IF NOT EXISTS idx_ext_scripts_extends ON ext_scripts(extends);
CREATE INDEX IF NOT EXISTS idx_extref_referenced ON record_refs(referenced_form_key);
CREATE INDEX IF NOT EXISTS idx_extref_referencing ON record_refs(referencing_form_key);
"""


def parse_filename(filename):
    stem = filename[:-5]
    parts = stem.split(" - ", 1)
    if len(parts) != 2:
        return None, None
    editor_id = parts[0].strip()
    fk_raw = parts[1].strip()
    underscore_idx = fk_raw.find("_")
    if underscore_idx > 0:
        form_key = fk_raw[:underscore_idx] + ":" + fk_raw[underscore_idx + 1:]
    else:
        form_key = fk_raw
    return editor_id, form_key


def extract_yaml_fields(yaml_path):
    try:
        with open(yaml_path, encoding="utf-8", errors="replace") as f:
            raw = f.read()
    except Exception:
        return "", "", "", []

    display_name = ""
    keywords_str = ""
    try:
        data = yaml.safe_load(raw)
        if isinstance(data, dict):
            name_val = data.get("Name")
            if isinstance(name_val, str):
                display_name = name_val
            full_val = data.get("FULL")
            if isinstance(full_val, str) and not display_name:
                display_name = full_val
            kw_list = data.get("Keywords")
            if isinstance(kw_list, list):
                keywords_str = " ".join(str(k) for k in kw_list)
    except Exception:
        pass

    content = raw[:MAX_CONTENT_LEN]
    refs = _FORMKEY_RE.findall(raw)
    return display_name, keywords_str, content, refs


def scan_mod_yaml(mod_name, mod_dir):
    yaml_dir = os.path.join(mod_dir, "yaml")
    if not os.path.isdir(yaml_dir):
        return
    for record_type in sorted(os.listdir(yaml_dir)):
        type_dir = os.path.join(yaml_dir, record_type)
        if not os.path.isdir(type_dir):
            continue
        if record_type in ("yaml",):
            continue
        for fname in os.listdir(type_dir):
            full_path = os.path.join(type_dir, fname)
            if fname.endswith(".yaml"):
                if fname in ("RecordData.yaml", "spriggit-meta.json"):
                    continue
                editor_id, form_key = parse_filename(fname)
                if not editor_id or not form_key:
                    continue
                yaml_path = full_path
            elif os.path.isdir(full_path):
                legacy_record_data: str = os.path.join(full_path, "RecordData.yaml")
                if not os.path.isfile(legacy_record_data):
                    continue
                editor_id, form_key = parse_filename(fname + ".yaml")
                if not editor_id or not form_key:
                    continue
                yaml_path = legacy_record_data
            else:
                continue
            prefixed_key = f"{mod_name}/{form_key}"
            yield yaml_path, editor_id, prefixed_key, record_type


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
    content = raw[:MAX_SCRIPT_LEN]
    return script_name, extends, properties, functions, events, content


def scan_mod_scripts(mod_name, mod_dir):
    scripts_dir = os.path.join(mod_dir, "scripts")
    if not os.path.isdir(scripts_dir):
        return
    for root, dirs, files in os.walk(scripts_dir):
        for fname in files:
            if not fname.lower().endswith(".psc"):
                continue
            filepath = os.path.join(root, fname)
            result = parse_script(filepath)
            if result is None:
                continue
            script_name, extends, properties, functions, events, content = result
            script_id = f"{mod_name}/{script_name}"
            yield (
                script_id, script_name, fname, extends, properties, functions,
                events, content, filepath,
            )


def scan_mod_readme(mod_name, mod_dir):
    readme_path = os.path.join(mod_dir, "README.md")
    if not os.path.isfile(readme_path):
        return None
    try:
        with open(readme_path, encoding="utf-8", errors="replace") as f:
            raw = f.read()
    except Exception:
        return None
    title = mod_name
    for line in raw.split("\n"):
        line = line.strip()
        if line.startswith("# "):
            title = line[2:].strip()
            break
    content = raw[:MAX_README_LEN]
    return title, content, readme_path


def get_mod_dirs(reference_dir, mod_name_filter=None):
    if not os.path.isdir(reference_dir):
        print(f"WARNING: External mods directory not found: {reference_dir}")
        return
    for name in sorted(os.listdir(reference_dir)):
        mod_dir = os.path.join(reference_dir, name)
        if not os.path.isdir(mod_dir):
            continue
        if mod_name_filter and name != mod_name_filter:
            continue
        yield name, mod_dir


def _new_buffer():
    return {
        "ext_records": {c: [] for c in EXT_REC_COLS},
        "ext_scripts": {c: [] for c in EXT_SCR_COLS},
        "ext_readmes": {c: [] for c in EXT_README_COLS},
        "record_refs": {c: [] for c in REF_COLS},
    }


def _flush_table(bulk, cols):
    n = len(next(iter(cols.values())))
    if n > 0:
        # Ensure ext_readmes IGNORE duplicates if PK collides; default is REPLACE
        pass


def _flush_all(bulk, buf):
    for table, cols in buf.items():
        n = len(next(iter(cols.values())))
        if n > 0:
            bulk.add_chunk(table, cols)
            for lst in cols.values():
                lst.clear()


def build_db(reference_dir, mod_name_filter=None, build_embeddings=False, db_path=None):
    if db_path is None:
        raise ValueError("db_path is required")

    is_incremental = bool(mod_name_filter)
    rec_count = 0
    ref_count = 0
    script_count = 0
    readme_count = 0

    with BulkInserter(db_path, BULK_SCHEMA, fresh=not is_incremental) as bulk:
        bulk.execute(CREATE_TABLES_DDL)

        if is_incremental:
            bulk.execute_params(
                "DELETE FROM record_refs WHERE referencing_form_key IN "
                "(SELECT form_key FROM ext_records WHERE mod_name = ?)",
                [mod_name_filter],
            )
            bulk.execute_params("DELETE FROM ext_records WHERE mod_name = ?", [mod_name_filter])
            bulk.execute_params("DELETE FROM ext_scripts WHERE mod_name = ?", [mod_name_filter])
            bulk.execute_params("DELETE FROM ext_readmes WHERE mod_name = ?", [mod_name_filter])

        buf = _new_buffer()

        for mod_name, mod_dir in get_mod_dirs(reference_dir, mod_name_filter):
            print(f"  Records: scanning {mod_name}...")
            mod_rec_count = 0
            mod_ref_count = 0
            for yaml_path, editor_id, form_key, record_type in scan_mod_yaml(mod_name, mod_dir):
                display_name, keywords_str, content, refs = extract_yaml_fields(yaml_path)
                editor_id_tokens = tokenize(editor_id)
                name_tokens = tokenize(display_name) if display_name else ""

                r = buf["ext_records"]
                r["form_key"].append(form_key)
                r["editor_id"].append(editor_id)
                r["editor_id_tokens"].append(editor_id_tokens)
                r["record_type"].append(record_type)
                r["name"].append(display_name)
                r["name_tokens"].append(name_tokens)
                r["mod_name"].append(mod_name)
                r["keywords"].append(keywords_str)
                r["yaml_path"].append(yaml_path)
                r["content"].append(content)
                mod_rec_count += 1

                seen_refs = set()
                for ref in refs:
                    if ref != form_key and ref not in seen_refs:
                        seen_refs.add(ref)
                        buf["record_refs"]["referencing_form_key"].append(form_key)
                        buf["record_refs"]["referenced_form_key"].append(ref)
                        mod_ref_count += 1

                if len(r["form_key"]) >= 1000:
                    bulk.add_chunk("ext_records", r)
                    for lst in r.values():
                        lst.clear()
                if len(buf["record_refs"]["referencing_form_key"]) >= 5000:
                    bulk.add_chunk("record_refs", buf["record_refs"])
                    for lst in buf["record_refs"].values():
                        lst.clear()

            print(f"    {mod_rec_count} records, {mod_ref_count} cross-refs")
            rec_count += mod_rec_count
            ref_count += mod_ref_count

            print(f"  Scripts: scanning {mod_name}...")
            mod_scr_count = 0
            s = buf["ext_scripts"]
            for (
                script_id, script_name, filename, extends, properties, functions,
                events, content, filepath,
            ) in scan_mod_scripts(mod_name, mod_dir):
                script_name_tokens = tokenize(script_name)
                s["script_id"].append(script_id)
                s["script_name"].append(script_name)
                s["script_name_tokens"].append(script_name_tokens)
                s["filename"].append(filename)
                s["extends"].append(extends.lower() if extends else "")
                s["mod_name"].append(mod_name)
                s["properties"].append(properties)
                s["functions"].append(functions)
                s["events"].append(events)
                s["content"].append(content)
                s["script_path"].append(filepath)
                mod_scr_count += 1
                if len(s["script_id"]) >= 1000:
                    bulk.add_chunk("ext_scripts", s)
                    for lst in s.values():
                        lst.clear()

            print(f"    {mod_scr_count} scripts")
            script_count += mod_scr_count

            result = scan_mod_readme(mod_name, mod_dir)
            if result:
                title, content, readme_path = result
                title_tokens = tokenize(title)
                rm = buf["ext_readmes"]
                rm["mod_name"].append(mod_name)
                rm["title"].append(title)
                rm["title_tokens"].append(title_tokens)
                rm["content"].append(content)
                rm["readme_path"].append(readme_path)
                print(f"  README: {mod_name} ({title})")
                readme_count += 1

        _flush_all(bulk, buf)

        if not is_incremental:
            print("  Building indexes...")
            bulk.create_indexes(CREATE_INDEXES_SQL)

        print("  Rebuilding FTS indexes...")
        bulk.rebuild_fts("ext_records_fts")
        bulk.rebuild_fts("ext_scripts_fts")
        bulk.rebuild_fts("ext_readmes_fts")

    if build_embeddings:
        _build_embeddings(db_path)

    return rec_count, script_count, readme_count


def _build_embeddings(db_path):
    from creation_lib.db.embeddings import build_vec_index

    with BulkInserter(db_path, BULK_SCHEMA, fresh=False) as bulk:
        rec_rows = bulk.query_all(
            "SELECT form_key, editor_id, name, record_type FROM ext_records ORDER BY rowid"
        )
        scr_rows = bulk.query_all(
            "SELECT script_id, script_name, extends, functions FROM ext_scripts ORDER BY rowid"
        )

    if rec_rows:
        texts = [f"{r[1]} {r[2] or ''} {r[3]}".strip() for r in rec_rows]
        ids = [r[0] for r in rec_rows]
        print(f"\n  ext_records: {len(texts)} documents")
        build_vec_index(texts, ids, db_path, table_name="ext_records_embeddings")

    if scr_rows:
        texts = [f"{r[1]} {r[2] or ''} {(r[3] or '')[:200]}".strip() for r in scr_rows]
        ids = [r[0] for r in scr_rows]
        print(f"\n  ext_scripts: {len(texts)} documents")
        build_vec_index(texts, ids, db_path, table_name="ext_scripts_embeddings")


def main():
    import argparse

    parser = argparse.ArgumentParser(description="Build external mods database")
    parser.add_argument("--game", default="fo4")
    parser.add_argument("--mod", default=None)
    parser.add_argument("--external-mods-dir", required=True)
    parser.add_argument("--db-path", required=True)
    parser.add_argument("--embeddings", action="store_true")
    args = parser.parse_args()
    mod_name_filter = args.mod
    game = args.game

    from creation_lib.core.game_profiles import GAME_PROFILES
    if game not in GAME_PROFILES:
        print(f"ERROR: Invalid --game '{game}'. Valid: {', '.join(sorted(GAME_PROFILES))}")
        sys.exit(1)

    build_embeddings = args.embeddings
    db_path = args.db_path
    reference_dir = args.external_mods_dir

    mode = f"incremental ({mod_name_filter})" if mod_name_filter else "full rebuild"
    print(f"Building external mods database [{game}] [{mode}]")
    print(f"  Source: {reference_dir}")
    print(f"  Database: {db_path}")
    if build_embeddings:
        print("  Embeddings: enabled")
    print()

    rec_count, script_count, readme_count = build_db(
        reference_dir, mod_name_filter, build_embeddings, db_path
    )
    print(f"\nDone! Indexed {rec_count} records, {script_count} scripts, {readme_count} READMEs")


if __name__ == "__main__":
    main()
