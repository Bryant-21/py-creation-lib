"""NLLB-200 translation backend for Bethesda mod YAML string fields.

Walks the canonical authoring-dir layout (``yaml/plugin.yaml`` +
``yaml/records/<SIG>/*.yaml``), allocates string-table IDs for every literal
string sitting in a localized field (``Name``, ``Description``, …),
rewrites the field to ``{TargetLanguage: English, raw_hex: "..."}``,
writes the per-language ``Strings/<Plugin>_<lang>.STRINGS`` sidecars, and
sets the ``Localized`` header flag (``0x80``) in ``plugin.yaml``.
"""
from __future__ import annotations

import logging
import os
from collections.abc import Mapping
from typing import Any, Callable

from ruamel.yaml import YAML
from ruamel.yaml.comments import CommentedMap, CommentedSeq

from creation_lib.esp.strings import language_code, load_all_string_tables

_log = logging.getLogger(__name__)

# Bit set on the TES4 record header to mark a plugin as localized.
# Mirrors HEADER_FLAG_DEFINITIONS in py_creation_lib/native/esp/src/plugin_runtime.rs.
_HEADER_FLAG_LOCALIZED = 0x80

# ── Language config ──────────────────────────────────────────────────────────

# Output order matches the historical translation output.
LANGUAGE_ORDER = [
    "Chinese", "German", "English", "Spanish", "Spanish_Mexico",
    "French", "Italian", "Japanese", "Polish", "Portuguese_Brazil", "Russian",
]

NLLB_LANG_MAP: dict[str, str] = {
    "Chinese":           "zho_Hans",
    "German":            "deu_Latn",
    "English":           "eng_Latn",
    "Spanish":           "spa_Latn",
    "Spanish_Mexico":    "spa_Latn",   # same code; produces identical text
    "French":            "fra_Latn",
    "Italian":           "ita_Latn",
    "Japanese":          "jpn_Jpan",
    "Polish":            "pol_Latn",
    "Portuguese_Brazil": "por_Latn",
    "Russian":           "rus_Cyrl",
}

# Authoring-name → 4-letter signature for fields that resolve through the
# localized string table. Keep in sync with the schema's localized signature
# set (``LOCALIZED_SIGNATURE_TABLE_TYPES`` in ``py_creation_lib/python/creation_lib/esp/strings.py``).
_LOCALIZED_FIELD_SIGNATURES: dict[str, str] = {
    "Name":        "FULL",
    "Description": "DESC",
    "ShortName":   "SHRT",
    "FULL":        "FULL",
    "DESC":        "DESC",
    "SHRT":        "SHRT",
    "NNAM":        "NNAM",
    "NAM1":        "NAM1",
    "RNAM":        "RNAM",
    "ITXT":        "ITXT",
}

# ── Model cache (lazy-loaded) ────────────────────────────────────────────────

_tokenizer = None
_model = None


def _load_model(progress_cb: Callable[[str], None] | None = None) -> None:
    global _tokenizer, _model
    if _tokenizer is not None:
        return
    from transformers import AutoModelForSeq2SeqLM, NllbTokenizer
    _cb = progress_cb or _log.info
    _cb("Loading translation model (facebook/nllb-200-distilled-600M)...")
    _cb("First run will download ~600MB — this may take a few minutes.")
    _tokenizer = NllbTokenizer.from_pretrained("facebook/nllb-200-distilled-600M")
    _model = AutoModelForSeq2SeqLM.from_pretrained("facebook/nllb-200-distilled-600M")
    _cb("Translation model ready.")


def _translate_string(text: str, src_lang: str, tgt_lang: str) -> str:
    """Translate a single string. Separated for easy mocking in tests."""
    import torch
    tokenizer = _tokenizer
    model = _model
    tokenizer.src_lang = src_lang
    inputs = tokenizer(text, return_tensors="pt", padding=True, truncation=True, max_length=512)
    tgt_lang_id = tokenizer.convert_tokens_to_ids(tgt_lang)
    with torch.no_grad():
        output = model.generate(
            **inputs,
            forced_bos_token_id=tgt_lang_id,
            max_new_tokens=256,
        )
    return tokenizer.decode(output[0], skip_special_tokens=True)


# ── raw_hex helpers ──────────────────────────────────────────────────────────

def _decode_raw_hex(raw_hex: str) -> int | None:
    """Decode an 8-char little-endian hex string ID. Mirrors the records
    preprocessor (``py_creation_lib/python/creation_lib/preprocessor/records.py:_decode_raw_hex_string_id``).
    """
    if not isinstance(raw_hex, str):
        return None
    cleaned = raw_hex.strip()
    if len(cleaned) < 8:
        return None
    try:
        return int.from_bytes(bytes.fromhex(cleaned[:8]), "little")
    except ValueError:
        return None


# ── Field detection ──────────────────────────────────────────────────────────

def is_translatable_field(key: Any, value: Any) -> bool:
    """Return True if a ``fields:`` entry is a literal localized string that
    should be promoted to a ``raw_hex`` string-table entry.

    A field is translatable iff:
      * its key is a known localized field name (``Name``, ``Description``,
        ``ShortName``, …);
      * its value is a non-empty string (already-localized values are dicts).
    """
    if key not in _LOCALIZED_FIELD_SIGNATURES:
        return False
    return isinstance(value, str) and bool(value.strip())


def find_translatable_paths(doc: Any) -> dict[str, str]:
    """Return ``{field_name: english_text}`` for every literal localized field
    in a record YAML document. Used by tests and progress reporting.
    """
    results: dict[str, str] = {}
    if not isinstance(doc, dict):
        return results
    fields = doc.get("fields")
    if not isinstance(fields, list):
        return results
    for entry in fields:
        if not isinstance(entry, dict) or len(entry) != 1:
            continue
        for key, value in entry.items():
            if is_translatable_field(key, value):
                results[str(key)] = value
    return results


def build_localized_field(translations: Mapping[str, str]) -> CommentedMap:
    """Build the ``{TargetLanguage, Values}`` dict that replaces a literal
    localized string, carrying the text for every language in the record.

    This is the shape the ESP exporter produces and the builder consumes
    (``data/fo4_esm_yaml`` WEAP alone has 226 ``Values`` blocks and no
    ``raw_hex``). A ``{TargetLanguage, raw_hex}`` id with the text in a
    ``Strings/`` sidecar loses every translation on override, makes the
    authoring dir unreadable, and shows LOOKUP FAILED in game when the id
    reaches no loaded table.

    Allocating ids and writing tables is the builder's job
    (``EspPlugin.save_localized_strings``); doing it here too would create a
    second, non-merging table set.
    """
    cm = CommentedMap()
    cm["TargetLanguage"] = "English"
    rows = []
    for language in LANGUAGE_ORDER:
        text = translations.get(language)
        if text is None:
            continue
        row = CommentedMap()
        row["Language"] = language
        row["String"] = text
        rows.append(row)
    cm["Values"] = rows
    return cm


# ── plugin.yaml helpers ──────────────────────────────────────────────────────

def _read_plugin_manifest(yaml_dir: str) -> tuple[str, dict] | None:
    """Read ``plugin.yaml``. Returns (plugin_name, parsed_doc) or None."""
    plugin_yaml = os.path.join(yaml_dir, "plugin.yaml")
    if not os.path.isfile(plugin_yaml):
        return None
    yaml = YAML()
    with open(plugin_yaml, encoding="utf-8") as f:
        doc = yaml.load(f)
    if not isinstance(doc, dict):
        return None
    plugin_name = str(doc.get("plugin") or "")
    if not plugin_name:
        return None
    return plugin_name, doc


def set_localized_flag(yaml_dir: str) -> None:
    """Set the ``Localized`` flag (bit 0x80) on ``header.flags`` in
    ``plugin.yaml``. Idempotent — the flag is OR'd in.
    """
    plugin_yaml = os.path.join(yaml_dir, "plugin.yaml")
    if not os.path.isfile(plugin_yaml):
        return
    yaml = YAML()
    with open(plugin_yaml, encoding="utf-8") as f:
        doc = yaml.load(f)
    if not isinstance(doc, dict):
        return
    header = doc.get("header")
    if not isinstance(header, dict):
        return
    flags = header.get("flags", 0)
    if isinstance(flags, str):
        try:
            flags_int = int(flags, 16) if not flags.isdigit() else int(flags)
        except ValueError:
            flags_int = 0
    elif isinstance(flags, int):
        flags_int = flags
    else:
        flags_int = 0
    new_flags = flags_int | _HEADER_FLAG_LOCALIZED
    if new_flags == flags_int:
        return
    header["flags"] = new_flags
    with open(plugin_yaml, "w", encoding="utf-8") as f:
        yaml.dump(doc, f)


# ── YAML field rewriting ─────────────────────────────────────────────────────

def translate_to_all_languages(english: str) -> dict[str, str]:
    """English text to one entry per language in ``LANGUAGE_ORDER``.

    A language whose translation fails falls back to the English text rather
    than being dropped, so every record carries a full set of rows.
    """
    english_code = NLLB_LANG_MAP["English"]
    out: dict[str, str] = {}
    for language in LANGUAGE_ORDER:
        code = NLLB_LANG_MAP[language]
        if code == english_code:
            out[language] = english
            continue
        try:
            out[language] = _translate_string(english, english_code, code)
        except Exception:  # noqa: BLE001 - a failed language must not lose the row
            out[language] = english
    return out


def _rewrite_record_fields(
    doc: Any,
    *,
    translate: Callable[[str], dict[str, str]] | None = None,
) -> int:
    """Rewrite literal localized fields in a record doc to ``Values`` blocks.

    Returns the number of fields rewritten. No string ids are allocated here:
    the text stays in the record and the builder assigns ids and emits the
    tables when it writes the plugin.
    """
    if not isinstance(doc, dict):
        return 0
    fields = doc.get("fields")
    if not isinstance(fields, list):
        return 0

    translate_fn = translate or translate_to_all_languages
    rewrites = 0

    for entry in fields:
        if not isinstance(entry, dict) or len(entry) != 1:
            continue
        key = next(iter(entry))
        value = entry[key]
        if not is_translatable_field(key, value):
            continue
        entry[key] = build_localized_field(translate_fn(value))
        rewrites += 1

    return rewrites


# ── raw_hex migration ────────────────────────────────────────────────────────

def _default_strings_dirs(mod_dir: str) -> list[str]:
    """Where a mod's shipped string tables live, most authoritative first."""
    return [
        os.path.join(mod_dir, "data", "Strings"),
        os.path.join(mod_dir, "yaml", "Strings"),
        os.path.join(mod_dir, "Strings"),
    ]


def _is_promoted_string(value: Any) -> bool:
    """Whether a dict is a localized field that was promoted to a string id.

    Both keys are required. An unparsed subrecord blob carries ``raw_hex``
    alone -- COBJ's ``CTDA`` is 32 bytes of condition data whose first four
    would otherwise read as a string id.
    """
    return isinstance(value, dict) and "raw_hex" in value and "TargetLanguage" in value


def _restore_field(
    entry: dict,
    key: Any,
    tables: Mapping[str, Mapping[int, str]],
) -> bool:
    """Replace one ``{TargetLanguage, raw_hex}`` field with its ``Values`` block.

    Returns False -- leaving the field untouched -- when the id resolves to
    nothing, so an unreadable table degrades to "nothing changed" instead of
    silently blanking the record's text.
    """
    value = entry.get(key)
    if not _is_promoted_string(value):
        return False
    string_id = _decode_raw_hex(value.get("raw_hex"))
    if not string_id:  # id 0 is the format's "no string" - not a lookup failure
        return False

    # load_all_string_tables keys by language code ("de"), the record by
    # display name ("German").
    translations = {
        language: text
        for language in LANGUAGE_ORDER
        if (text := tables.get(language_code(language), {}).get(string_id)) is not None
    }
    if not translations:
        return False

    entry[key] = build_localized_field(translations)
    return True


def _restore_node(node: Any, tables: Mapping[str, Mapping[int, str]]) -> tuple[int, int]:
    """Restore every promoted string under ``node``. Returns (restored, unresolved).

    Walks the whole document, not just top-level ``fields`` entries: a CELL's
    map-marker ``Name`` sits two levels down inside ``MapMarkers``, and a
    top-level-only pass silently leaves those behind.
    """
    restored = unresolved = 0
    if isinstance(node, dict):
        for key, value in list(node.items()):
            if _is_promoted_string(value):
                if _restore_field(node, key, tables):
                    restored += 1
                else:
                    unresolved += 1
            else:
                sub_r, sub_u = _restore_node(value, tables)
                restored += sub_r
                unresolved += sub_u
    elif isinstance(node, list):
        for item in node:
            sub_r, sub_u = _restore_node(item, tables)
            restored += sub_r
            unresolved += sub_u
    return restored, unresolved


def restore_localized_values(
    mod_dir: str,
    strings_dir: str | None = None,
    progress_cb: Callable[[str], None] | None = None,
) -> dict:
    """Migrate ``{TargetLanguage, raw_hex}`` fields back to ``Values`` blocks.

    Repairs authoring dirs whose localized text was moved out of the record
    into a sidecar. Reads the text back from the mod's shipped string tables,
    so it only recovers what actually shipped: a field whose id is missing
    from every table is reported under ``unresolved`` and left alone.

    Args:
        mod_dir: Mod directory (e.g. ``mods/B21_MyMod``).
        strings_dir: Table directory. Defaults to the first of ``data/Strings``,
            ``yaml/Strings``, ``Strings`` that exists.

    Returns:
        ``{records, fields, unresolved, languages, strings_dir, errors}``.
    """
    def report(msg: str) -> None:
        _log.info(msg)
        if progress_cb:
            progress_cb(msg)

    yaml_dir = os.path.join(mod_dir, "yaml")
    manifest = _read_plugin_manifest(yaml_dir)
    if manifest is None:
        return {"records": 0, "fields": 0, "unresolved": 0, "languages": 0,
                "strings_dir": None, "errors": ["no readable plugin.yaml"]}
    plugin_name, _ = manifest

    candidates = [strings_dir] if strings_dir else _default_strings_dirs(mod_dir)
    source = next((d for d in candidates if d and os.path.isdir(d)), None)
    if source is None:
        return {"records": 0, "fields": 0, "unresolved": 0, "languages": 0,
                "strings_dir": None, "errors": [f"no string tables found for {plugin_name}"]}

    try:
        tables, _table_types = load_all_string_tables(plugin_name, strings_dir=source)
    except Exception as exc:  # noqa: BLE001 - surfaced in the result, not raised
        return {"records": 0, "fields": 0, "unresolved": 0, "languages": 0,
                "strings_dir": source, "errors": [f"could not read tables: {exc}"]}

    languages = [lang for lang in LANGUAGE_ORDER if tables.get(language_code(lang))]
    report(f"loaded {len(languages)} language table(s) from {source}")

    records_dir = os.path.join(yaml_dir, "records")
    yaml = YAML()
    yaml.preserve_quotes = True
    records = fields = unresolved = 0
    errors: list[str] = []

    for root, _dirs, names in os.walk(records_dir):
        for name in sorted(names):
            if not name.endswith((".yaml", ".yml")):
                continue
            path = os.path.join(root, name)
            try:
                with open(path, encoding="utf-8") as f:
                    doc = yaml.load(f)
            except Exception as exc:  # noqa: BLE001
                errors.append(f"{name}: {exc}")
                continue
            if not isinstance(doc, dict):
                continue

            changed, missed = _restore_node(doc, tables)
            unresolved += missed
            if not changed:
                continue
            try:
                with open(path, "w", encoding="utf-8") as f:
                    yaml.dump(doc, f)
            except Exception as exc:  # noqa: BLE001
                errors.append(f"{name}: {exc}")
                continue
            records += 1
            fields += changed

    report(f"restored {fields} field(s) across {records} record(s); {unresolved} unresolved")
    return {
        "records": records,
        "fields": fields,
        "unresolved": unresolved,
        "languages": len(languages),
        "strings_dir": source,
        "errors": errors,
    }


# ── Public API ───────────────────────────────────────────────────────────────

def translate_mod(mod_dir: str, progress_cb: Callable[[str], None] | None = None) -> dict:
    """Translate every literal localized string in a mod's YAML to 11 languages.

    Walks ``mod_dir/yaml/records/<SIG>/*.yaml``, rewrites translatable fields
    whose value is a literal string into a ``{TargetLanguage, Values}`` block
    (see ``build_localized_field``), and sets the Localized flag on
    ``plugin.yaml`` when no file failed. ``progress_cb`` defaults to logging.

    Returns ``{"translated": int, "skipped": int, "errors": list[str]}``.
    """
    cb = progress_cb or _log.info
    yaml_dir = os.path.join(mod_dir, "yaml")
    if not os.path.isdir(yaml_dir):
        raise ValueError(f"No yaml/ directory found in {mod_dir}")

    manifest = _read_plugin_manifest(yaml_dir)
    if manifest is None:
        raise ValueError(f"No plugin.yaml (with a 'plugin:' field) found in {yaml_dir}")
    plugin_name, _ = manifest

    records_root = os.path.join(yaml_dir, "records")
    if not os.path.isdir(records_root):
        raise ValueError(f"No records/ directory found in {yaml_dir}")

    _load_model(cb)

    yaml = YAML()
    translated = 0
    skipped = 0
    errors: list[str] = []

    # Collect record YAMLs under records/<SIG>/*.yaml
    files: list[str] = []
    for sig_entry in sorted(os.listdir(records_root)):
        sig_dir = os.path.join(records_root, sig_entry)
        if not os.path.isdir(sig_dir):
            continue
        for name in sorted(os.listdir(sig_dir)):
            if name.endswith(".yaml"):
                files.append(os.path.join(sig_dir, name))

    cb(f"Scanning {len(files)} record YAML file(s)...")

    # No id allocation and no sidecar tables. The translated text is written
    # into the record as a Values block; the builder assigns ids and emits the
    # string tables when it writes the plugin. Allocating here too would create
    # a second Strings/ table set that never merges with the one the packer
    # ships, so the ids would resolve to nothing in game.
    for filepath in files:
        try:
            with open(filepath, encoding="utf-8") as f:
                doc = yaml.load(f)
            if not isinstance(doc, dict):
                skipped += 1
                continue

            count = _rewrite_record_fields(doc)
            if count == 0:
                skipped += 1
                continue

            cb(f"  Rewriting {os.path.basename(filepath)} ({count} field(s))...")

            with open(filepath, "w", encoding="utf-8") as f:
                yaml.dump(doc, f)

            translated += count

        except Exception as e:
            msg = f"Failed to process {os.path.basename(filepath)}: {e}"
            cb(f"  Warning: {msg}")
            errors.append(msg)
            skipped += 1
            continue

    if not errors:
        set_localized_flag(yaml_dir)
        cb("Set Localized flag in plugin.yaml.")

    cb(f"Translation complete — {translated} field(s) translated, {skipped} file(s) skipped, {len(errors)} error(s).")
    return {"translated": translated, "skipped": skipped, "errors": errors}


# ── Back-compat shim ─────────────────────────────────────────────────────────

# Older callers (tests, UI) imported ``CommentedSeq`` indirectly through this
# module. Keep the import re-exported for those.
__all__ = [
    "LANGUAGE_ORDER",
    "NLLB_LANG_MAP",
    "build_localized_field",
    "find_translatable_paths",
    "is_translatable_field",
    "set_localized_flag",
    "translate_mod",
]

# Silence unused-import warning — re-exported for callers that pulled the
# symbol from this module under the old shape.
_ = CommentedSeq
