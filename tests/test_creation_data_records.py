from pathlib import Path


def test_lookup_editor_id_reads_fo4_records_db():
    from creation_lib.creation_data.records import lookup_editor_id

    db_dir = Path(__file__).resolve().parents[2] / "data"
    rows = lookup_editor_id(
        "DLC04WorkshopWorkbench",
        game="fo4",
        db_dir=str(db_dir),
    )

    assert rows[0]["form_key"] == "047DFA:DLCNukaWorld.esm"
    assert rows[0]["editor_id"] == "DLC04WorkshopWorkbench"
