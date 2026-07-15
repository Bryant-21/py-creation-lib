"""Spellcheck YAML mod content using language_tool_python.

Checks dialogue text, descriptions, script notes, and other text fields
in mod YAML files for spelling and grammar errors. Uses a custom Bethesda
dictionary to suppress false positives on game-specific terms.
"""

from __future__ import annotations

import glob
import logging
import os
from dataclasses import dataclass, field
from pathlib import Path

import yaml

_log = logging.getLogger("creation_lib.spellcheck")

try:
    import language_tool_python
    HAS_LANGUAGE_TOOL = True
except ImportError:
    HAS_LANGUAGE_TOOL = False

_DICTIONARY_PATH = str(Path(__file__).parent / "spellcheck_dictionary.txt")

# Cached LanguageTool instance (starts Java server on first use)
_tool_instance: language_tool_python.LanguageTool | None = None


@dataclass
class SpellcheckIssue:
    """Single spellcheck finding."""
    file: str                          # YAML path relative to mod
    field: str                         # e.g. "Response Text", "Description"
    text: str                          # Full text that was checked
    offset: int                        # Error start position in text
    length: int                        # Error length
    message: str                       # Error description
    suggestions: list[str] = field(default_factory=list)
    rule_id: str = ""                  # LanguageTool rule ID
    context: str = ""                  # Surrounding text snippet

    @property
    def error_text(self) -> str:
        """The flagged portion of text."""
        return self.text[self.offset:self.offset + self.length]


# ---------------------------------------------------------------------------
# Dictionary management
# ---------------------------------------------------------------------------

def load_dictionary(path: str | None = None) -> set[str]:
    """Load custom dictionary terms from file. Returns set of lowercase terms."""
    path = path or _DICTIONARY_PATH
    terms: set[str] = set()
    try:
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    terms.add(line.lower())
    except FileNotFoundError:
        _log.warning("Dictionary file not found: %s", path)
    return terms


def add_to_dictionary(term: str, path: str | None = None) -> None:
    """Append a term to the custom dictionary file."""
    path = path or _DICTIONARY_PATH
    with open(path, "a", encoding="utf-8") as f:
        f.write(f"\n{term}")
    _log.info("Added to dictionary: %s", term)


def remove_from_dictionary(term: str, path: str | None = None) -> None:
    """Remove a term from the custom dictionary file."""
    path = path or _DICTIONARY_PATH
    lines = []
    try:
        with open(path, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except FileNotFoundError:
        return
    with open(path, "w", encoding="utf-8") as f:
        for line in lines:
            if line.strip().lower() != term.lower():
                f.write(line)
    _log.info("Removed from dictionary: %s", term)


def get_dictionary_terms(path: str | None = None) -> list[str]:
    """Return sorted list of dictionary terms (original casing from file)."""
    path = path or _DICTIONARY_PATH
    terms = []
    try:
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#"):
                    terms.append(line)
    except FileNotFoundError:
        pass
    return sorted(terms, key=str.lower)


# ---------------------------------------------------------------------------
# Tool initialization
# ---------------------------------------------------------------------------

def _get_tool() -> language_tool_python.LanguageTool:
    """Get or create the cached LanguageTool instance."""
    global _tool_instance
    if _tool_instance is None:
        if not HAS_LANGUAGE_TOOL:
            raise ImportError(
                "language_tool_python not installed. Run: uv add language-tool-python"
            )
        _log.info("Starting LanguageTool server (first use may download ~200MB)...")
        _tool_instance = language_tool_python.LanguageTool("en-US")
        _log.info("LanguageTool ready.")
    return _tool_instance


def shutdown_tool() -> None:
    """Shut down the cached LanguageTool server."""
    global _tool_instance
    if _tool_instance is not None:
        _tool_instance.close()
        _tool_instance = None


# ---------------------------------------------------------------------------
# YAML text extraction
# ---------------------------------------------------------------------------

def _extract_checkable_fields(yaml_path: str) -> list[tuple[str, str]]:
    """Extract (field_name, text) pairs from a YAML file worth spellchecking.

    Looks for:
    - Responses[].Text — dialogue subtitles
    - Responses[].ScriptNotes — script notes
    - Responses[].Edits — editorial notes
    - FULL / Description fields — item/quest descriptions
    """
    fields: list[tuple[str, str]] = []
    try:
        with open(yaml_path, "r", encoding="utf-8") as f:
            data = yaml.safe_load(f)
    except Exception:
        return fields

    if not data or not isinstance(data, dict):
        return fields

    # Check top-level description fields
    for key in ("FULL", "Description", "FULL - Name"):
        val = data.get(key)
        if isinstance(val, str) and val.strip():
            fields.append((key, val.strip()))
        elif isinstance(val, dict):
            text = val.get("String", val.get("TargetLanguage", ""))
            if isinstance(text, str) and text.strip():
                fields.append((key, text.strip()))

    # Check Responses array (dialogue)
    responses = data.get("Responses", [])
    if isinstance(responses, list):
        for i, resp in enumerate(responses):
            if not isinstance(resp, dict):
                continue

            # Response text (subtitle)
            text_field = resp.get("Text", "")
            if isinstance(text_field, dict):
                text = text_field.get("String", text_field.get("TargetLanguage", ""))
            elif isinstance(text_field, str):
                text = text_field
            else:
                text = ""
            if text and text.strip():
                fields.append((f"Response Text [{i+1}]", text.strip()))

            # Script notes
            notes = resp.get("ScriptNotes", "")
            if isinstance(notes, str) and notes.strip():
                fields.append((f"Script Notes [{i+1}]", notes.strip()))

            # Edits
            edits = resp.get("Edits", "")
            if isinstance(edits, str) and edits.strip():
                fields.append((f"Edits [{i+1}]", edits.strip()))

    return fields


# ---------------------------------------------------------------------------
# Main check function
# ---------------------------------------------------------------------------

def check_mod_text(yaml_dir: str,
                   dictionary_path: str | None = None) -> list[SpellcheckIssue]:
    """Scan all YAML files in a mod for spelling/grammar issues.

    Args:
        yaml_dir: Path to the mod's yaml/ directory.
        dictionary_path: Path to custom dictionary file (default: built-in).

    Returns:
        List of SpellcheckIssue, sorted by file then offset.
    """
    tool = _get_tool()
    custom_terms = load_dictionary(dictionary_path)
    issues: list[SpellcheckIssue] = []

    # Find all YAML files
    yaml_files = glob.glob(os.path.join(yaml_dir, "**", "*.yaml"), recursive=True)
    _log.info("Spellchecking %d YAML files in %s", len(yaml_files), yaml_dir)

    for yaml_path in sorted(yaml_files):
        rel_path = os.path.relpath(yaml_path, yaml_dir)
        fields = _extract_checkable_fields(yaml_path)

        for field_name, text in fields:
            matches = tool.check(text)
            for match in matches:
                # Skip if the flagged word is in our custom dictionary
                error_text = text[match.offset:match.offset + match.errorLength]
                if error_text.lower() in custom_terms:
                    continue
                # Also check if any word in the error is a known term
                if any(w.lower() in custom_terms for w in error_text.split()):
                    continue

                issues.append(SpellcheckIssue(
                    file=rel_path,
                    field=field_name,
                    text=text,
                    offset=match.offset,
                    length=match.errorLength,
                    message=match.message,
                    suggestions=list(match.replacements[:5]),
                    rule_id=match.ruleId,
                    context=match.context,
                ))

    _log.info("Spellcheck complete: %d issues in %d files", len(issues), len(yaml_files))
    return issues


def apply_fix(yaml_dir: str, issue: SpellcheckIssue,
              replacement: str) -> bool:
    """Apply a spellcheck fix by replacing text in the source YAML file.

    Reads the YAML file, finds the field text, performs string replacement,
    and writes back. Returns True on success.
    """
    yaml_path = os.path.join(yaml_dir, issue.file)
    try:
        with open(yaml_path, "r", encoding="utf-8") as f:
            content = f.read()

        old_text = issue.text[issue.offset:issue.offset + issue.length]
        new_text = issue.text[:issue.offset] + replacement + issue.text[issue.offset + issue.length:]

        # Replace the old full text with the new corrected text in the file
        # We replace the exact field text to be safe
        if issue.text in content:
            content = content.replace(issue.text, new_text, 1)
        else:
            _log.warning("Could not find exact text in %s for fix", issue.file)
            return False

        with open(yaml_path, "w", encoding="utf-8") as f:
            f.write(content)

        _log.info("Fixed '%s' -> '%s' in %s", old_text, replacement, issue.file)
        return True

    except Exception as e:
        _log.error("Failed to apply fix in %s: %s", issue.file, e)
        return False
