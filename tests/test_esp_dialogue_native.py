"""Rust-native dialogue extraction replaces creation_lib.esp.dialogue."""

from __future__ import annotations

import json

from creation_lib.esp.native_runtime import (
    plugin_handle_extract_dialogue_text,
    plugin_handle_import_text,
)


def test_extract_dialogue_text_exports_dial_info_tree() -> None:
    handle = plugin_handle_import_text(
        json.dumps(
            {
                "plugin": "Dialogue.esp",
                "game": "fo4",
                "header": {"version": 1.0, "next_object_id": "000803"},
                "items": [
                    {
                        "type": "group",
                        "label_text": "DIAL",
                        "group_type": 0,
                        "children": [
                            {
                                "signature": "DIAL",
                                "form_id": "000801",
                                "subrecords": [
                                    {"signature": "EDID", "data_hex": "546F70696300"},
                                    {"signature": "FULL", "data_hex": "4772656574696E6700"},
                                ],
                            },
                            {
                                "type": "group",
                                "label_hex": "01080000",
                                "group_type": 7,
                                "children": [
                                    {
                                        "signature": "INFO",
                                        "form_id": "000802",
                                        "subrecords": [
                                            {"signature": "EDID", "data_hex": "526573706F6E736500"},
                                            {"signature": "RNAM", "data_hex": "50726F6D707400"},
                                            {"signature": "NAM1", "data_hex": "48656C6C6F00"},
                                        ],
                                    }
                                ],
                            },
                        ],
                    }
                ],
            }
        ),
        "json",
        "fo4",
    )

    topics = json.loads(plugin_handle_extract_dialogue_text(handle, "json"))

    assert len(topics) == 1
    topic = topics[0]
    assert topic["plugin"] == "Dialogue.esp"
    assert topic["dial_form_id"] == "00000801"
    assert topic["editor_id"] == "Topic"
    assert topic["topic"]["text"] == "Greeting"
    assert topic["response_count"] == 1
    info = topic["infos"][0]
    assert info["form_id"] == "00000802"
    assert info["editor_id"] == "Response"
    assert info["prompt"]["text"] == "Prompt"
    assert info["responses"][0]["text"] == "Hello"
