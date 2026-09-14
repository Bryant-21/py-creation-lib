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
    result = build_localized_field({"English": "Barrel", "German": "Lauf"})
    assert result["TargetLanguage"] == "English"
    rows = {row["Language"]: row["String"] for row in result["Values"]}
    assert rows["English"] == "Barrel"
    assert rows["German"] == "Lauf"


def test_build_localized_field_carries_no_raw_hex():
    """The text stays in the record. raw_hex moved it into a Strings/ sidecar
    that never merged with the table the packer ships, so the id resolved to
    nothing in game and every translation was lost on override."""
    from creation_lib.mod.translation import build_localized_field
    result = build_localized_field({"English": "Barrel"})
    assert "raw_hex" not in result


def test_build_localized_field_orders_rows_by_language_order():
    from creation_lib.mod.translation import LANGUAGE_ORDER, build_localized_field
    result = build_localized_field({lang: lang for lang in LANGUAGE_ORDER})
    assert [row["Language"] for row in result["Values"]] == LANGUAGE_ORDER


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

    # The text stays in the record, one row per language, English verbatim.
    rows = {row["Language"]: row["String"] for row in name_entry["Name"]["Values"]}
    assert rows["English"] == "Test Barrel"
    assert rows["German"] == "[deu_Latn]Test Barrel"
    assert "raw_hex" not in name_entry["Name"]

    # plugin.yaml has the Localized bit set.
    with open(yaml_dir / "plugin.yaml", encoding="utf-8") as f:
        plugin_doc = yaml.load(f)
    assert int(plugin_doc["header"]["flags"]) & 0x80

    # No sidecar tables. Ids and tables are the builder's job -- writing them
    # here too produced a second set under Strings/ that the packer collided
    # with and the game never loaded.
    assert not (yaml_dir / "Strings").exists()


# ── restore_localized_values ──────────────────────────────────────────────────

def _write_raw_hex_record(yaml_dir, string_id: int) -> "object":
    raw = string_id.to_bytes(4, "little").hex().upper()
    path = yaml_dir / "records" / "OMOD" / "B21_Test_mod_Barrel - 000800_B21_Test.esl.yaml"
    path.write_text(textwrap.dedent(f"""\
        form_id: "000800"
        eid: B21_Test_mod_Barrel
        fields:
        - Name:
            TargetLanguage: English
            raw_hex: "{raw}"
        - MODL: barrel.nif
    """), encoding="utf-8")
    return path


def test_restore_localized_values_rewrites_raw_hex(tmp_path):
    """The text is read back out of the shipped tables and put in the record."""
    from creation_lib.mod.translation import restore_localized_values

    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    record = _write_raw_hex_record(yaml_dir, 7)
    (mod_dir / "data" / "Strings").mkdir(parents=True)

    # load_all_string_tables keys by language code, not display name.
    tables = {"en": {7: "Long Barrel"}, "de": {7: "Langer Lauf"}}
    with patch("creation_lib.mod.translation.load_all_string_tables") as mock_load:
        mock_load.return_value = (tables, {})
        result = restore_localized_values(str(mod_dir))

    assert result["fields"] == 1
    assert result["records"] == 1
    assert result["unresolved"] == 0

    yaml = YAML()
    with open(record, encoding="utf-8") as f:
        doc = yaml.load(f)
    name = next(e for e in doc["fields"] if "Name" in e)["Name"]
    assert "raw_hex" not in name
    rows = {row["Language"]: row["String"] for row in name["Values"]}
    assert rows == {"English": "Long Barrel", "German": "Langer Lauf"}


def test_restore_localized_values_leaves_unresolved_ids_alone(tmp_path):
    """An id absent from every table must not blank the field -- a partial table
    would otherwise erase text that is still recoverable from a better one."""
    from creation_lib.mod.translation import restore_localized_values

    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    record = _write_raw_hex_record(yaml_dir, 999)
    (mod_dir / "data" / "Strings").mkdir(parents=True)

    with patch("creation_lib.mod.translation.load_all_string_tables") as mock_load:
        mock_load.return_value = ({"en": {7: "Long Barrel"}}, {})
        result = restore_localized_values(str(mod_dir))

    assert result["fields"] == 0
    assert result["unresolved"] == 1

    yaml = YAML()
    with open(record, encoding="utf-8") as f:
        doc = yaml.load(f)
    name = next(e for e in doc["fields"] if "Name" in e)["Name"]
    assert name["raw_hex"] == "E7030000"


def test_restore_localized_values_reaches_nested_fields(tmp_path):
    """A CELL's map-marker Name sits inside MapMarkers, two levels below
    ``fields``. A top-level-only pass leaves it as an opaque id."""
    from creation_lib.mod.translation import restore_localized_values

    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    (mod_dir / "data" / "Strings").mkdir(parents=True)
    record = yaml_dir / "records" / "OMOD" / "cell - 000801_B21_Test.esl.yaml"
    record.write_text(textwrap.dedent("""\
        form_id: "000801"
        fields:
        - XCLL:
          - MapMarkers:
            - MapMarkerData: true
              Name:
                TargetLanguage: English
                raw_hex: "07000000"
    """), encoding="utf-8")

    with patch("creation_lib.mod.translation.load_all_string_tables") as mock_load:
        mock_load.return_value = ({"en": {7: "My C.A.M.P."}}, {})
        result = restore_localized_values(str(mod_dir))

    assert result["fields"] == 1

    yaml = YAML()
    with open(record, encoding="utf-8") as f:
        doc = yaml.load(f)
    name = doc["fields"][0]["XCLL"][0]["MapMarkers"][0]["Name"]
    assert name["Values"][0]["String"] == "My C.A.M.P."


def test_restore_localized_values_ignores_unparsed_subrecord_blobs(tmp_path):
    """COBJ's CTDA is 32 bytes of raw condition data. It carries raw_hex but no
    TargetLanguage, and its first four bytes must not be read as a string id."""
    from creation_lib.mod.translation import restore_localized_values

    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    (mod_dir / "data" / "Strings").mkdir(parents=True)
    blob = "070000000000803F4A000000E7C73F07000000000000000000000000FFFFFFFF"
    record = yaml_dir / "records" / "OMOD" / "cobj - 000802_B21_Test.esl.yaml"
    record.write_text(textwrap.dedent(f"""\
        form_id: "000802"
        fields:
        - CTDA:
            raw_hex: "{blob}"
    """), encoding="utf-8")

    with patch("creation_lib.mod.translation.load_all_string_tables") as mock_load:
        mock_load.return_value = ({"en": {7: "Long Barrel"}}, {})
        result = restore_localized_values(str(mod_dir))

    assert result["fields"] == 0
    assert result["unresolved"] == 0

    yaml = YAML()
    with open(record, encoding="utf-8") as f:
        doc = yaml.load(f)
    assert doc["fields"][0]["CTDA"]["raw_hex"] == blob


def test_restore_localized_values_reports_missing_tables(tmp_path):
    from creation_lib.mod.translation import restore_localized_values

    mod_dir, yaml_dir = _scaffold_mod(tmp_path)
    _write_raw_hex_record(yaml_dir, 7)

    result = restore_localized_values(str(mod_dir))
    assert result["fields"] == 0
    assert result["errors"]


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
