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
import re
from typing import Any, Callable

from ruamel.yaml import YAML
from ruamel.yaml.comments import CommentedMap, CommentedSeq

from creation_lib.esp.strings import (
    STRING_TABLE_EXTENSIONS,
    language_code,
    load_string_tables,
    localized_table_type_for_signature,
    write_string_table,
)

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

def _encode_raw_hex(string_id: int) -> str:
    """Encode a string-table ID as 8-char little-endian uppercase hex."""
    return string_id.to_bytes(4, "little").hex().upper()


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


def build_localized_field(string_id: int) -> CommentedMap:
    """Build the ``{TargetLanguage, raw_hex}`` dict that replaces a literal
    localized string. ``TargetLanguage`` is always English — the actual
    translations land in the per-language ``.STRINGS`` sidecars.
    """
    cm = CommentedMap()
    cm["TargetLanguage"] = "English"
    cm["raw_hex"] = _encode_raw_hex(string_id)
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


def _next_string_id(strings: dict[int, str]) -> int:
    """Return the next free string-table ID. IDs start at 1 — ID 0 is
    reserved by the Bethesda format as "no string".
    """
    if not strings:
        return 1
    return max(strings) + 1


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

def _rewrite_record_fields(
    doc: Any,
    *,
    strings: dict[int, str],
    next_id: list[int],
) -> tuple[int, list[tuple[int, str, str]]]:
    """Rewrite literal localized fields in a record doc.

    Returns ``(rewrites, allocated)`` where ``allocated`` is a list of
    ``(string_id, signature, english_text)`` tuples for every newly-allocated
    string. ``next_id`` is a 1-element list used as a mutable counter so the
    caller can keep allocating across files.
    """
    if not isinstance(doc, dict):
        return 0, []
    fields = doc.get("fields")
    if not isinstance(fields, list):
        return 0, []

    allocated: list[tuple[int, str, str]] = []
    rewrites = 0

    for entry in fields:
        if not isinstance(entry, dict) or len(entry) != 1:
            continue
        key = next(iter(entry))
        value = entry[key]
        if not is_translatable_field(key, value):
            continue
        signature = _LOCALIZED_FIELD_SIGNATURES[key]
        sid = next_id[0]
        next_id[0] += 1
        strings[sid] = value
        allocated.append((sid, signature, value))
        entry[key] = build_localized_field(sid)
        rewrites += 1

    return rewrites, allocated


# ── Strings/ sidecar writers ─────────────────────────────────────────────────

def _strings_path(yaml_dir: str, plugin_name: str, lang: str, table_type: str) -> str:
    """Path of a STRINGS sidecar. ``plugin_name`` is the full filename with
    extension (``B21_MyMod.esl``); the sidecar uses the stem.
    """
    stem = re.sub(r"\.es[plm]$", "", plugin_name, flags=re.IGNORECASE)
    ext = STRING_TABLE_EXTENSIONS[table_type]
    return os.path.join(yaml_dir, "Strings", f"{stem}_{lang}{ext}")


def _load_existing_table(
    yaml_dir: str,
    plugin_name: str,
    lang_display: str,
    table_type: str,
) -> dict[int, str]:
    """Best-effort load of an existing per-language STRINGS sidecar."""
    strings_dir = os.path.join(yaml_dir, "Strings")
    if not os.path.isdir(strings_dir):
        return {}
    try:
        tables = load_string_tables(plugin_name, strings_dir=strings_dir, language=lang_display)
    except Exception:
        return {}
    if not tables:
        return {}
    return {int(k): str(v) for k, v in tables.items()}


def _write_language_table(
    yaml_dir: str,
    plugin_name: str,
    lang_display: str,
    table_type: str,
    table: dict[int, str],
) -> None:
    """Write a per-language STRINGS sidecar."""
    lang = language_code(lang_display)
    out_path = _strings_path(yaml_dir, plugin_name, lang, table_type)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    write_string_table(out_path, table, table_type=table_type)


# ── Public API ───────────────────────────────────────────────────────────────

def translate_mod(mod_dir: str, progress_cb: Callable[[str], None] | None = None) -> dict:
    """Translate every literal localized string in a mod's YAML to 11 languages.

    Walks ``mod_dir/yaml/records/<SIG>/*.yaml``, finds ``Name`` /
    ``Description`` / ``ShortName`` entries whose value is a literal string,
    allocates a string-table ID for each, rewrites the entry to
    ``{TargetLanguage: English, raw_hex: "..."}``, writes the per-language
    ``Strings/<Plugin>_<lang>.STRINGS`` sidecars (one per language in
    ``LANGUAGE_ORDER``), and sets the Localized flag on ``plugin.yaml``.

    Args:
        mod_dir: Path to the mod directory (e.g. ``mods/B21_MyMod``).
        progress_cb: Called with log messages. Defaults to logging.

    Returns:
        ``{"translated": int, "skipped": int, "errors": list[str]}``
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

    # Existing English STRINGS table is the seed for ID allocation.
    english_strings = _load_existing_table(yaml_dir, plugin_name, "English", "strings")
    english_dlstrings = _load_existing_table(yaml_dir, plugin_name, "English", "dlstrings")
    english_ilstrings = _load_existing_table(yaml_dir, plugin_name, "English", "ilstrings")

    # The full ID space spans all three table types — string IDs are unique
    # across the whole plugin, not per-table.
    seed = {**english_strings, **english_dlstrings, **english_ilstrings}
    next_id = [_next_string_id(seed)]

    # Per-table accumulator of (sid, english_text, signature) for translation pass.
    new_entries: list[tuple[int, str, str]] = []

    for filepath in files:
        try:
            with open(filepath, encoding="utf-8") as f:
                doc = yaml.load(f)
            if not isinstance(doc, dict):
                skipped += 1
                continue

            count, allocated = _rewrite_record_fields(
                doc, strings=seed, next_id=next_id,
            )
            if count == 0:
                skipped += 1
                continue

            cb(f"  Rewriting {os.path.basename(filepath)} ({count} field(s))...")

            with open(filepath, "w", encoding="utf-8") as f:
                yaml.dump(doc, f)

            translated += count
            new_entries.extend(allocated)

        except Exception as e:
            msg = f"Failed to process {os.path.basename(filepath)}: {e}"
            cb(f"  Warning: {msg}")
            errors.append(msg)
            skipped += 1
            continue

    if new_entries:
        # Bucket newly-allocated entries by table type.
        by_table: dict[str, list[tuple[int, str]]] = {
            "strings": [],
            "dlstrings": [],
            "ilstrings": [],
        }
        for sid, signature, text in new_entries:
            table_type = localized_table_type_for_signature(signature)
            by_table[table_type].append((sid, text))

        existing_by_table = {
            "strings": english_strings,
            "dlstrings": english_dlstrings,
            "ilstrings": english_ilstrings,
        }

        cb("Writing localized string tables...")
        for lang in LANGUAGE_ORDER:
            for table_type, additions in by_table.items():
                if not additions and not existing_by_table[table_type]:
                    continue
                # English passes the source text through; other languages get
                # NLLB output (or fall back to English if translation fails).
                table = dict(existing_by_table[table_type])
                if lang == "English":
                    for sid, text in additions:
                        table[sid] = text
                else:
                    nllb_code = NLLB_LANG_MAP[lang]
                    for sid, text in additions:
                        if nllb_code == NLLB_LANG_MAP["English"]:
                            table[sid] = text
                            continue
                        try:
                            table[sid] = _translate_string(text, "eng_Latn", nllb_code)
                        except Exception as e:
                            cb(f"    Warning: {lang} translation failed, using English: {e}")
                            table[sid] = text
                _write_language_table(yaml_dir, plugin_name, lang, table_type, table)

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
