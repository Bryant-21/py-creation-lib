"""Localized string-table helpers.

Binary I/O (parse/write/discover) is implemented natively in Rust at
``py_creation_lib/native/esp/src/strings.rs``. This module keeps the pure-Python helpers
(language alias tables, signature → table-type mapping) and thin PyO3 shims
around the native functions so external callers keep the old API surface.
"""

from __future__ import annotations

from pathlib import Path
from typing import Mapping

from .native_runtime import _require_native_function

STRING_TABLE_TYPES = ("strings", "ilstrings", "dlstrings")
STRING_TABLE_EXTENSIONS = {
    "strings": ".STRINGS",
    "ilstrings": ".ILSTRINGS",
    "dlstrings": ".DLSTRINGS",
}
LANGUAGE_ALIASES = {
    "chinese": "cn",
    "cn": "cn",
    "chinesesimplified": "zhhans",
    "chinese_simplified": "zhhans",
    "zhhans": "zhhans",
    "chinesetraditional": "zhhant",
    "chinese_traditional": "zhhant",
    "zhhant": "zhhant",
    "german": "de",
    "de": "de",
    "english": "en",
    "en": "en",
    "spanish": "es",
    "es": "es",
    "spanish_mexico": "esmx",
    "esmx": "esmx",
    "french": "fr",
    "fr": "fr",
    "italian": "it",
    "it": "it",
    "japanese": "ja",
    "ja": "ja",
    "korean": "ko",
    "ko": "ko",
    "polish": "pl",
    "pl": "pl",
    "portuguese_brazil": "ptbr",
    "ptbr": "ptbr",
    "russian": "ru",
    "ru": "ru",
}
LANGUAGE_DISPLAY_NAMES = {
    "cn": "Chinese",
    "zhhans": "ChineseSimplified",
    "zhhant": "ChineseTraditional",
    "de": "German",
    "en": "English",
    "es": "Spanish",
    "esmx": "Spanish_Mexico",
    "fr": "French",
    "it": "Italian",
    "ja": "Japanese",
    "ko": "Korean",
    "pl": "Polish",
    "ptbr": "Portuguese_Brazil",
    "ru": "Russian",
}
LANGUAGE_DISPLAY_ORDER = (
    "Chinese",
    "ChineseSimplified",
    "ChineseTraditional",
    "German",
    "English",
    "Spanish",
    "Spanish_Mexico",
    "French",
    "Italian",
    "Japanese",
    "Korean",
    "Polish",
    "Portuguese_Brazil",
    "Russian",
)
LOCALIZED_SIGNATURE_TABLE_TYPES = {
    "DESC": "dlstrings",
    "ITXT": "dlstrings",
    "FULL": "strings",
    "NNAM": "strings",
    "SHRT": "strings",
    "NAM1": "ilstrings",
    # RNAM (INFO Prompt / FLOR Activate Text) is plain UI text, never voiced — STRINGS.
    "RNAM": "strings",
}

# Record-scoped overrides applied before the flat table above. Keep in lockstep
# with esp::io::table_type_for_localized_signature (the authoritative write path):
# LSCR.DESC is plain STRINGS in FO4, unlike the BOOK/SPEL/PERK long descriptions
# that use DLSTRINGS.
LOCALIZED_RECORD_SIGNATURE_TABLE_TYPES = {
    ("LSCR", "DESC"): "strings",
    ("TERM", "ITXT"): "strings",
    ("TERM", "RNAM"): "strings",
    ("MESG", "DESC"): "strings",
    ("MESG", "ITXT"): "strings",
    # CNAM defaults to STRINGS, but on these two it is long-form prose (book
    # text, quest log entry) and the write path files it under DLSTRINGS. Absent
    # here, a reader looks in the wrong table and the field comes back
    # unresolved.
    ("BOOK", "CNAM"): "dlstrings",
    ("QUST", "CNAM"): "dlstrings",
}


def _normalize_language(language: str | None) -> str | None:
    if not language:
        return None
    key = language.strip().lower().replace("-", "_").replace(" ", "_")
    return LANGUAGE_ALIASES.get(key, key)


def language_display_name(language: str | None) -> str:
    normalized = _normalize_language(language)
    if normalized is None:
        return "English"
    return LANGUAGE_DISPLAY_NAMES.get(normalized, normalized)


def language_code(language: str | None) -> str:
    normalized = _normalize_language(language)
    if normalized is None:
        return "en"
    return normalized


def localized_table_type_for_signature(
    signature: str, record_signature: str | None = None
) -> str:
    if record_signature is not None:
        scoped = LOCALIZED_RECORD_SIGNATURE_TABLE_TYPES.get((record_signature, signature))
        if scoped is not None:
            return scoped
    return LOCALIZED_SIGNATURE_TABLE_TYPES.get(signature, "strings")


def parse_string_table(path: str | Path) -> dict[int, str]:
    source = Path(path)
    native_fn = _require_native_function("parse_string_table_native")
    return {int(string_id): str(text) for string_id, text in native_fn(str(source))}


def load_string_tables(
    plugin_name: str,
    *,
    strings_dir: str | Path | None,
    language: str | None = None,
) -> dict[int, str]:
    native_fn = _require_native_function("load_string_tables_native")
    return {
        int(string_id): str(text)
        for string_id, text in native_fn(
            plugin_name,
            None if strings_dir is None else str(strings_dir),
            language,
        )
    }


def load_all_string_tables(
    plugin_name: str,
    *,
    strings_dir: str | Path | None,
) -> tuple[dict[str, dict[int, str]], dict[int, str]]:
    native_fn = _require_native_function("load_all_string_tables_native")
    value_rows, table_type_rows = native_fn(
        plugin_name,
        None if strings_dir is None else str(strings_dir),
    )
    values_by_language: dict[str, dict[int, str]] = {}
    for language, string_id, text in value_rows:
        values_by_language.setdefault(str(language), {})[int(string_id)] = str(text)
    return (
        values_by_language,
        {int(string_id): str(table_type) for string_id, table_type in table_type_rows},
    )


def write_string_table(path: str | Path, values: Mapping[int, str], *, table_type: str) -> Path:
    target = Path(path)
    native_fn = _require_native_function("write_string_table_native")
    coerced = [(int(key), str(value)) for key, value in values.items()]
    native_fn(str(target), coerced, table_type)
    return target
