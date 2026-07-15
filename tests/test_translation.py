"""Tests for py_creation_lib/python/creation_lib/mod/translation.py — model is mocked throughout.

Covers the canonical YAML shape (``yaml/plugin.yaml`` +
``yaml/records/<SIG>/*.yaml``). Localized fields land in
``Strings/<Plugin>_<lang>.STRINGS`` sidecars; the source YAML carries
``{TargetLanguage, raw_hex}`` after rewriting.
"""
from __future__ import annotations

import textwrap
from io import StringIO
from unittest.mock import patch

from ruamel.yaml import YAML


# ── helpers ─────────────────────────────────────────────────────────────────

def _make_yaml(content: str) -> dict:
    yaml = YAML()
    return yaml.load(StringIO(textwrap.dedent(content)))


def _scaffold_mod(tmp_path, plugin_name: str = "B21_Test.esl") -> tuple:
    """Scaffold a minimal canonical authoring dir. Returns (mod_dir, yaml_dir)."""
    mod_dir = tmp_path / "B21_Test"
    yaml_dir = mod_dir / "yaml"
    (yaml_dir / "records" / "OMOD").mkdir(parents=True)

    (yaml_dir / "plugin.yaml").write_text(textwrap.dedent(f"""\
        format_version: 1
        plugin: {plugin_name}
        game: fo4
        header_size: 24
        header:
          version: 1.0
          num_records: 0
          next_object_id: "000800"
          author: B21
          flags: 512
          masters:
          - Fallout4.esm
          master_sizes:
          - 0
          overridden_forms: []
    """), encoding="utf-8")

    return mod_dir, yaml_dir


# ── is_translatable_field ────────────────────────────────────────────────────

def test_translatable_field_detected():
    from creation_lib.mod.translation import is_translatable_field
    assert is_translatable_field("Name", "Long Barrel") is True


def test_already_localized_dict_not_translatable():
    from creation_lib.mod.translation import is_translatable_field
    assert is_translatable_field(
        "Name", {"TargetLanguage": "English", "raw_hex": "01000000"}
    ) is False


def test_unknown_field_key_not_translatable():
    from creation_lib.mod.translation import is_translatable_field
    assert is_translatable_field("MODL", "barrel.nif") is False


def test_empty_string_not_translatable():
    from creation_lib.mod.translation import is_translatable_field
    assert is_translatable_field("Name", "") is False
    assert is_translatable_field("Name", "   ") is False


def test_non_string_value_not_translatable():
    from creation_lib.mod.translation import is_translatable_field
    assert is_translatable_field("Name", 25) is False


# ── build_localized_field ─────────────────────────────────────────────────────

def test_build_localized_field_structure():
    from creation_lib.mod.translation import build_localized_field
    result = build_localized_field(1)
    assert result["TargetLanguage"] == "English"
    # Little-endian u32 for ID 1.
    assert result["raw_hex"] == "01000000"


def test_build_localized_field_higher_id():
    from creation_lib.mod.translation import build_localized_field
    # 0x123 = 291 → little-endian "23010000"
    result = build_localized_field(0x123)
    assert result["raw_hex"] == "23010000"


# ── find_translatable_paths ────────────────────────────────────────────────────

def test_find_fields_in_record_doc():
    from creation_lib.mod.translation import find_translatable_paths
    doc = _make_yaml("""
        form_id: "000800"
        eid: B21_Test_mod_Barrel
        fields:
        - Name: Long Barrel
        - Description: Superior range.
        - MODL: barrel.nif
    """)
    paths = find_translatable_paths(doc)
    assert len(paths) == 2
    assert paths["Name"] == "Long Barrel"
    assert paths["Description"] == "Superior range."


def test_skip_already_localized():
    from creation_lib.mod.translation import find_translatable_paths
    doc = _make_yaml("""
        form_id: "000800"
        fields:
        - Name:
            TargetLanguage: English
            raw_hex: "01000000"
        - Description: New text
    """)
    paths = find_translatable_paths(doc)
    assert len(paths) == 1
    assert "Description" in paths


# ── set_localized_flag ────────────────────────────────────────────────────────

def test_set_localized_flag_or_in(tmp_path):
    from creation_lib.mod.translation import set_localized_flag
    _, yaml_dir = _scaffold_mod(tmp_path)
    set_localized_flag(str(yaml_dir))

    yaml = YAML()
    with open(yaml_dir / "plugin.yaml", encoding="utf-8") as f:
        doc = yaml.load(f)
    flags = int(doc["header"]["flags"])
    # Light (0x200) was already set by the scaffold; Localized (0x80) is new.
    assert flags & 0x80
    assert flags & 0x200


def test_set_localized_flag_idempotent(tmp_path):
    from creation_lib.mod.translation import set_localized_flag
    _, yaml_dir = _scaffold_mod(tmp_path)
    set_localized_flag(str(yaml_dir))
    set_localized_flag(str(yaml_dir))

    yaml = YAML()
    with open(yaml_dir / "plugin.yaml", encoding="utf-8") as f:
        doc = yaml.load(f)
    flags = int(doc["header"]["flags"])
    # 0x200 (Light) | 0x80 (Localized) = 0x280
    assert flags == 0x280


# ── translate_mod (integration, model mocked) ─────────────────────────────────

def test_translate_mod_rewrites_record_yaml(tmp_path):
    """translate_mod replaces a literal Name with {TargetLanguage, raw_hex},
    sets the Localized flag, and writes per-language STRINGS sidecars."""
    mod_dir, yaml_dir = _scaffold_mod(tmp_path)

    omod_file = yaml_dir / "records" / "OMOD" / "B21_Test_mod_Barrel - 000800_B21_Test.esl.yaml"
    omod_file.write_text(textwrap.dedent("""\
        form_id: "000800"
        eid: B21_Test_mod_Barrel
        fields:
        - Name: Test Barrel
        - MODL: barrel.nif
    """), encoding="utf-8")

    with patch("creation_lib.mod.translation._translate_string") as mock_translate:
        mock_translate.side_effect = lambda text, src, tgt: f"[{tgt}]{text}"
        with patch("creation_lib.mod.translation._load_model"):
            from creation_lib.mod.translation import translate_mod
            result = translate_mod(str(mod_dir))

    assert result["translated"] == 1
    assert result["errors"] == []

    # Record YAML now carries the {TargetLanguage, raw_hex} dict.
    yaml = YAML()
    with open(omod_file, encoding="utf-8") as f:
        doc = yaml.load(f)
    name_entry = next(e for e in doc["fields"] if "Name" in e)
    assert name_entry["Name"]["TargetLanguage"] == "English"
    raw_hex = name_entry["Name"]["raw_hex"]
    assert isinstance(raw_hex, str) and len(raw_hex) == 8

    # plugin.yaml has the Localized bit set.
    with open(yaml_dir / "plugin.yaml", encoding="utf-8") as f:
        plugin_doc = yaml.load(f)
    assert int(plugin_doc["header"]["flags"]) & 0x80

    # Strings sidecars exist for at least English + one translated language.
    strings_dir = yaml_dir / "Strings"
    assert (strings_dir / "B21_Test_en.STRINGS").is_file()
    assert (strings_dir / "B21_Test_de.STRINGS").is_file()

    # English table contains the literal source text.
    from creation_lib.esp.strings import load_string_tables
    en_table = load_string_tables("B21_Test.esl", strings_dir=strings_dir, language="English")
    assert "Test Barrel" in en_table.values()


def test_translate_mod_skips_already_localized(tmp_path):
    """Records whose Name is already {TargetLanguage, raw_hex} should be skipped."""
    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    omod_file = yaml_dir / "records" / "OMOD" / "test - 000800_B21_Test.esl.yaml"
    omod_file.write_text(textwrap.dedent("""\
        form_id: "000800"
        fields:
        - Name:
            TargetLanguage: English
            raw_hex: "01000000"
    """), encoding="utf-8")

    with patch("creation_lib.mod.translation._translate_string") as mock_translate:
        mock_translate.return_value = "translated"
        with patch("creation_lib.mod.translation._load_model"):
            from creation_lib.mod.translation import translate_mod
            result = translate_mod(str(mod_dir))

    assert result["translated"] == 0
    assert result["skipped"] == 1
