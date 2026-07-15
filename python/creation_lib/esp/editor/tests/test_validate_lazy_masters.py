from __future__ import annotations

from creation_lib.esp import native_runtime
from creation_lib.esp.editor import EditorSession, header_flags, validate
from creation_lib.esp.plugin import Plugin


def _issue_key(issue) -> tuple:
    return (
        issue.severity.value,
        issue.category.value,
        issue.plugin_name,
        issue.message,
        issue.form_id,
        issue.path,
        issue.signature,
    )


def test_validation_uses_lazy_master_and_preserves_itm_detection(tmp_path) -> None:
    master_path = tmp_path / "B21_Master.esm"
    master = Plugin.new(master_path.name, game="fo4", masters=[])
    record = master.new_record("MISC", form_id=0x00000800)
    record.add_subrecord("EDID", b"B21_Item\x00")
    master.add_record(record)
    header_flags.set_master(master._rust_handle, True)
    master.save(master_path)
    master.close()

    plugin_path = tmp_path / "B21_Override.esp"
    plugin = Plugin.new(plugin_path.name, game="fo4", masters=[master_path.name])
    override = plugin.new_record("MISC", form_id=0x00000800)
    override.add_subrecord("EDID", b"B21_Item\x00")
    plugin.add_record(override)
    plugin.save(plugin_path)
    plugin.close()

    eager_session = EditorSession(
        default_game="fo4",
        auto_scan_conflicts=False,
        master_search_paths=[tmp_path],
    )
    try:
        eager_loaded = eager_session.load(plugin_path, game="fo4")
        eager_report = [_issue_key(issue) for issue in validate(eager_session, handle=eager_loaded.handle)]
    finally:
        eager_session.close_all()

    session = EditorSession(
        default_game="fo4",
        auto_scan_conflicts=False,
        master_search_paths=[tmp_path],
        lazy_masters=True,
    )
    try:
        loaded = session.load(plugin_path, game="fo4")
        loaded_master = next(item for item in session.plugins if item.is_master)
        assert native_runtime.plugin_handle_record_form_ids(loaded_master.handle) == []

        report = validate(session, handle=loaded.handle)
        assert [_issue_key(issue) for issue in report] == eager_report
        assert any(issue.category.value == "itm" for issue in report)
    finally:
        session.close_all()
