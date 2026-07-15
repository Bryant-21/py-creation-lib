from pathlib import Path


def test_get_function_reads_current_wiki_schema():
    from creation_lib.creation_data.scripts import get_function

    db_dir = Path(__file__).resolve().parents[2] / "data"
    rows = get_function("GetFormID", game="fo4", db_dir=str(db_dir))

    assert isinstance(rows, list)
    assert rows[0]["function_name"] == "GetFormID"
    assert rows[0]["parent_script"] == "Form"
    assert rows[0]["content_type"] == "wiki"
    assert rows[0]["syntax"].startswith("int Function GetFormID()")


def test_get_script_hierarchy_reads_extends_from_content():
    from creation_lib.creation_data.scripts import get_script_hierarchy

    db_dir = Path(__file__).resolve().parents[2] / "data"
    hierarchy = get_script_hierarchy("Form", game="fo4", db_dir=str(db_dir))

    assert hierarchy[0]["script_type"] == "Form"
    assert hierarchy[0]["extends"] == "ScriptObject"
