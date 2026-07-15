from __future__ import annotations

from pathlib import Path

from creation_lib.esp.validate import validate_authoring


def _write_plugin_yaml(yaml_dir: Path, plugin: str, masters: list[str] | None = None) -> None:
    master_lines = "\n".join(f"    - {master}" for master in (masters or []))
    yaml_dir.mkdir(parents=True, exist_ok=True)
    (yaml_dir / "plugin.yaml").write_text(
        f"plugin: {plugin}\nheader:\n  masters:\n{master_lines or '    []'}\n",
        encoding="utf-8",
    )


def test_validate_authoring_finds_canonical_and_legacy_refs(tmp_path: Path):
    yaml_dir = tmp_path / "yaml"
    _write_plugin_yaml(yaml_dir, "B21_Test.esp", ["Fallout4.esm"])

    record_dir = yaml_dir / "records" / "WEAP"
    record_dir.mkdir(parents=True)
    (record_dir / "TestGun - 000800_B21_Test.esp.yaml").write_text(
        """
form_id: "000800"
fields:
  - Internal:
      reference:
        plugin: B21_Test.esp
        object_id: "000800"
  - External:
      reference:
        plugin: Fallout4.esm
        object_id: "248AB9"
  - LegacyNote: "017E69:Fallout4.esm"
""".lstrip(),
        encoding="utf-8",
    )

    errors, checked = validate_authoring(yaml_dir)

    # internal ref to 000800 resolves (same record defines it), no error
    assert errors == []
    # 3 refs scanned: internal, canonical external, legacy external
    assert checked == 3


def test_validate_authoring_accepts_projected_recorddata_definitions(tmp_path: Path):
    yaml_dir = tmp_path / "yaml"
    _write_plugin_yaml(yaml_dir, "B21_Test.esp", [])
    cell_dir = (
        yaml_dir
        / "records"
        / "WRLD"
        / "World - 000800_B21_Test.esp"
        / "0, 0"
        / "0, 0"
        / "0, 0"
    )
    cell_dir.mkdir(parents=True)
    (cell_dir / "RecordData.yaml").write_text(
        """
signature: CELL
form_id: "000801:B21_Test.esp"
subrecords:
  - signature: EDID
    data_hex: "00"
Landscape:
  signature: LAND
  form_id: "000802:B21_Test.esp"
  subrecords:
    - signature: DATA
      data_hex: "00000000"
metadata:
  form_id: "000999:B21_Test.esp"
""".lstrip(),
        encoding="utf-8",
    )

    errors, checked = validate_authoring(yaml_dir)

    assert checked == 0  # no <reference: ...> blocks, just projected form_ids
    assert errors == []
