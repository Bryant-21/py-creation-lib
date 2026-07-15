from __future__ import annotations

from pathlib import Path

from creation_lib.esp.validate import validate_authoring


def _write_plugin_yaml(yaml_dir: Path, plugin: str, masters: list[str]) -> None:
    master_lines = "\n".join(f"    - {master}" for master in masters)
    yaml_dir.mkdir(parents=True, exist_ok=True)
    (yaml_dir / "plugin.yaml").write_text(
        f"plugin: {plugin}\nheader:\n  masters:\n{master_lines or '    []'}\n",
        encoding="utf-8",
    )


def test_projected_recorddata_form_ids_treated_as_internal(tmp_path: Path):
    """Projected RecordData inside the mod registers its form_id as defined."""
    yaml_dir = tmp_path / "yaml"
    _write_plugin_yaml(yaml_dir, "B21_AppalachiaTestWorld.esp", [])
    cell_dir = (
        yaml_dir
        / "records"
        / "WRLD"
        / "B21_AppalachiaTestWorld - 000800_B21_AppalachiaTestWorld.esp"
        / "0, 0"
        / "0, 0"
        / "0, 0"
    )
    cell_dir.mkdir(parents=True)
    (cell_dir / "RecordData.yaml").write_text(
        """
signature: CELL
form_id: "000801:B21_AppalachiaTestWorld.esp"
subrecords:
  - signature: EDID
    data_hex: "00"
Landscape:
  signature: LAND
  form_id: "000802:B21_AppalachiaTestWorld.esp"
  subrecords:
    - signature: DATA
      data_hex: "00000000"
fields:
  - SelfRef:
      reference:
        plugin: B21_AppalachiaTestWorld.esp
        object_id: "000802"
""".lstrip(),
        encoding="utf-8",
    )

    errors, _ = validate_authoring(yaml_dir)

    # 000802 is defined via projected Landscape mapping → SelfRef resolves OK
    internal_misses = [e for e in errors if "internal ref not found" in e["reason"]]
    assert internal_misses == []


def test_external_projected_form_ids_not_treated_as_internal(tmp_path: Path):
    """Projected RecordData targeting an external plugin must not count as a
    local definition — refs to other internal ids that *would* match if it did
    should still report as missing."""
    yaml_dir = tmp_path / "yaml"
    _write_plugin_yaml(yaml_dir, "B21_Test.esp", ["Other.esp"])

    record_dir = yaml_dir / "records" / "WRLD" / "ExternalProjected"
    record_dir.mkdir(parents=True)
    (record_dir / "RecordData.yaml").write_text(
        """
signature: CELL
form_id: "000801:Other.esp"
fields:
  - SelfRef:
      reference:
        plugin: B21_Test.esp
        object_id: "000801"
""".lstrip(),
        encoding="utf-8",
    )

    errors, _ = validate_authoring(yaml_dir)

    internal_misses = [e for e in errors if "internal ref not found" in e["reason"]]
    assert len(internal_misses) >= 1
    assert any("000801:B21_Test.esp" in e["formkey"] for e in internal_misses)
