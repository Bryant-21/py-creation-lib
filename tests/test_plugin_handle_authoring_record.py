"""Native handle record view contracts without authoring dict materialization."""

from __future__ import annotations

import json

import pytest

from creation_lib.esp import native_runtime


def _handle() -> int:
    return int(
        native_runtime.plugin_handle_import_text(
            json.dumps(
                {
                    "plugin": "AuthoringRecord.esp",
                    "game": "fo4",
                    "header": {"version": 1.0, "next_object_id": "000801"},
                    "items": [
                        {
                            "type": "group",
                            "label_text": "MISC",
                            "group_type": 0,
                            "children": [
                                {
                                    "signature": "MISC",
                                    "form_id": "000800",
                                    "subrecords": [
                                        {
                                            "signature": "EDID",
                                            "data_hex": "4E6174697665417574686F72696E675265636F726400",
                                        }
                                    ],
                                }
                            ],
                        }
                    ],
                }
            ),
            "json",
            "fo4",
        )
    )


def test_record_summary_returns_scalar_record_metadata() -> None:
    summary = native_runtime.plugin_handle_record_summary(_handle(), 0x000800)

    assert summary is not None
    assert summary.form_id == 0x000800
    assert summary.signature == "MISC"
    assert summary.editor_id == "NativeAuthoringRecord"


def test_export_record_text_returns_authoring_text_payload() -> None:
    exported = native_runtime.plugin_handle_call(_handle(), "export_record_text", 0x000800, "json")
    payload = json.loads(exported)

    assert payload["type"] == "record"
    assert payload["signature"] == "MISC"
    assert payload["form_id"] == "000800"
    assert payload["eid"] == "NativeAuthoringRecord"


def test_export_record_text_rejects_unknown_form_id() -> None:
    with pytest.raises(KeyError, match="unknown record form_id"):
        native_runtime.plugin_handle_call(_handle(), "export_record_text", 0x00FFFF, "json")


def test_authoring_dict_handle_apis_are_not_exported() -> None:
    module = native_runtime.load_native_module()
    assert not hasattr(module, "plugin_handle_record_as_authoring_dict")
    assert not hasattr(module, "plugin_handle_records_as_authoring_dicts_batch")
    assert not hasattr(module, "plugin_handle_add_authoring_record")
