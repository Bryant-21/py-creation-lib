"""FO4 dialogue export with voice-type resolution (replaces FallTalk's xEdit export script)."""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

import creation_lib.esp.native_runtime as native_runtime
from creation_lib.esp.dialogue_export import PLAYER_VOICE_TYPES, export_dialogue_lines

MASTER = "Fallout4.esm"
MOD = "Mod.esp"

MALE_BOSTON = 0x000801
FEMALE_BOSTON = 0x000802
ROBOT_VOICE = 0x000803
MALE_ROUGH = 0x000804
BOB = 0x000810
ALICE = 0x000811
GUARD = 0x000812
GUARD_TEMPLATED = 0x000813
MALE_AND_FEMALE_LIST = 0x000820
ALICE_FACTION = 0x000830
ALICE_CLASS = 0x000840
GUARD_LIST = 0x000850
ROBOT_ACTIVATOR = 0x000860
MASTER_TOPIC = 0x000870
MASTER_INFO = 0x000871
TEST_CELL = 0x000900
BOB_REF = 0x000901
DEFAULT_NPC_VOICE_TYPES = 0x02D563

QUEST = 0x01000800
TOPIC = 0x01000801
INFO = 0x01000802
SECOND_INFO = 0x01000803
GREETING_TOPIC = 0x01000805
GREETING_INFO = 0x01000806
SCENE = 0x01000810
OTHER_QUEST = 0x01000820

GET_IS_CLASS = 68
GET_IN_FACTION = 71
GET_IS_ID = 72
GET_IS_VOICE_TYPE = 426
GET_IS_ALIAS_REF = 566

DIALOGUE_ACTION = 0
PLAYER_DIALOGUE_ACTION = 3
NPC_RESPONSE_DIALOGUE_ACTION = 5


def _sub(signature: str, data: bytes = b"") -> dict[str, str]:
    return {"signature": signature, "data_hex": data.hex().upper()}


def _z(text: str) -> bytes:
    return text.encode("cp1252") + b"\x00"


def _fid(form_id: int) -> bytes:
    return struct.pack("<I", form_id)


def _edid(text: str) -> dict[str, str]:
    return _sub("EDID", _z(text))


def _rec(signature: str, form_id: int, *subrecords: dict[str, str]) -> dict[str, object]:
    return {"signature": signature, "form_id": f"{form_id:08X}", "subrecords": list(subrecords)}


def _top(signature: str, *children: dict[str, object]) -> dict[str, object]:
    return {"type": "group", "label_text": signature, "group_type": 0, "children": list(children)}


def _children(parent_form_id: int, group_type: int, *children: dict[str, object]) -> dict[str, object]:
    return {
        "type": "group",
        "label_hex": _fid(parent_form_id).hex().upper(),
        "group_type": group_type,
        "children": list(children),
    }


def _write(path: Path, masters: list[str], items: list[dict[str, object]]) -> None:
    payload = {
        "plugin": path.name,
        "game": "fo4",
        "header": {
            "version": 1.0,
            "masters": masters,
            "master_sizes": [0] * len(masters),
            "next_object_id": "000900",
        },
        "items": items,
    }
    handle = native_runtime.plugin_handle_import_text(json.dumps(payload), "json", "fo4")
    try:
        native_runtime.plugin_handle_call(handle, "save", str(path))
    finally:
        native_runtime.plugin_handle_close(handle)


def _acbs(template_flags: int = 0) -> dict[str, str]:
    return _sub("ACBS", struct.pack("<IhHHHhHHBB", 0, 0, 1, 0, 0, 0, template_flags, 0, 0, 0))


def _npc(form_id: int, editor_id: str, *subrecords: dict[str, str]) -> dict[str, object]:
    return _rec("NPC_", form_id, _edid(editor_id), *subrecords)


def _form_list(form_id: int, editor_id: str, *entries: int) -> dict[str, object]:
    return _rec("FLST", form_id, _edid(editor_id), *(_sub("LNAM", _fid(entry)) for entry in entries))


def _responses(*texts: str) -> list[dict[str, str]]:
    subrecords = []
    for number, text in enumerate(texts, start=1):
        subrecords.append(_sub("TRDA", struct.pack("<IBIBHii", 0, number, 0, 0, 0, -1, -1)))
        subrecords.append(_sub("NAM1", _z(text)))
    return subrecords


def _info(form_id: int, *subrecords: dict[str, str], texts: tuple[str, ...] = ("Hello there.",)) -> dict[str, object]:
    return _rec("INFO", form_id, *_responses(*texts), *subrecords)


def _ctda(
    function: int,
    param1: int,
    *,
    value: float = 1.0,
    or_next: bool = False,
    run_on: int = 0,
) -> dict[str, str]:
    return _sub(
        "CTDA",
        struct.pack("<BBBBfHBBIIIIi", int(or_next), 0, 0, 0, value, function, 0, 0, param1, 0, run_on, 0, -1),
    )


def _topic(
    form_id: int,
    *infos: dict[str, object],
    quest: int | None = QUEST,
    subtype: bytes = b"CUST",
) -> list[dict[str, object]]:
    subrecords = [_edid(f"Topic{form_id:X}"), _sub("FULL", _z("Topic Text"))]
    if quest is not None:
        subrecords.append(_sub("QNAM", _fid(quest)))
    subrecords += [_sub("DATA", bytes([0, 7, 0, 0])), _sub("SNAM", subtype)]
    return [_rec("DIAL", form_id, *subrecords), _children(form_id, 7, *infos)]


def _alias(alias_id: int, *subrecords: dict[str, str]) -> list[dict[str, str]]:
    return [
        _sub("ALST", _fid(alias_id)),
        _sub("ALID", _z(f"Alias{alias_id}")),
        _sub("FNAM", _fid(0)),
        *subrecords,
        _sub("ALED"),
    ]


def _quest(
    form_id: int = QUEST,
    *aliases: list[dict[str, str]],
    editor_id: str = "ModQuest",
    dialogue_conditions: tuple[dict[str, str], ...] = (),
    event_conditions: tuple[dict[str, str], ...] = (),
) -> dict[str, object]:
    return _rec(
        "QUST",
        form_id,
        _edid(editor_id),
        *dialogue_conditions,
        _sub("NEXT"),
        *event_conditions,
        *(sub for alias in aliases for sub in alias),
    )


def _scene(*actions: tuple[int, int, list[dict[str, str]]], quest: int = QUEST) -> dict[str, object]:
    subrecords = [_edid("ModScene")]
    for index, (action_type, alias_id, topic_subrecords) in enumerate(actions, start=1):
        subrecords += [
            _sub("ANAM", struct.pack("<H", action_type)),
            _sub("NAM0", b"\x00"),
            _sub("ALID", struct.pack("<i", alias_id)),
            _sub("INAM", _fid(index)),
            *topic_subrecords,
            _sub("ANAM"),
        ]
    subrecords.append(_sub("PNAM", _fid(quest)))
    return _rec("SCEN", SCENE, *subrecords)


def _write_master(data_dir: Path, *, include_default_voice_types: bool = True) -> None:
    form_lists = [_form_list(MALE_AND_FEMALE_LIST, "MaleAndFemale", MALE_BOSTON, FEMALE_BOSTON)]
    if include_default_voice_types:
        form_lists.append(_form_list(DEFAULT_NPC_VOICE_TYPES, "DefaultNPCVoiceTypes", MALE_ROUGH))
    _write(
        data_dir / MASTER,
        [],
        [
            _top(
                "VTYP",
                *(
                    _rec("VTYP", form_id, _edid(name))
                    for form_id, name in [
                        (MALE_BOSTON, "MaleBoston"),
                        (FEMALE_BOSTON, "FemaleBoston"),
                        (ROBOT_VOICE, "RobotVoice"),
                        (MALE_ROUGH, "MaleRough"),
                    ]
                ),
            ),
            _top("FACT", _rec("FACT", ALICE_FACTION, _edid("AliceFaction"))),
            _top("CLAS", _rec("CLAS", ALICE_CLASS, _edid("AliceClass"))),
            _top("TACT", _rec("TACT", ROBOT_ACTIVATOR, _edid("RobotActivator"), _sub("VNAM", _fid(ROBOT_VOICE)))),
            _top(
                "NPC_",
                _npc(BOB, "Bob", _acbs(), _sub("VTCK", _fid(MALE_BOSTON))),
                _npc(
                    ALICE,
                    "Alice",
                    _acbs(),
                    _sub("VTCK", _fid(FEMALE_BOSTON)),
                    _sub("SNAM", _fid(ALICE_FACTION) + b"\x00"),
                    _sub("CNAM", _fid(ALICE_CLASS)),
                ),
                _npc(GUARD, "Guard", _acbs(), _sub("VTCK", _fid(MALE_ROUGH))),
                _npc(GUARD_TEMPLATED, "GuardTemplated", _acbs(template_flags=1), _sub("TPLT", _fid(GUARD_LIST))),
            ),
            _top("LVLN", _rec("LVLN", GUARD_LIST, _edid("GuardList"), _sub("LVLO", struct.pack("<HBBIHBB", 1, 0, 0, GUARD, 1, 0, 0)))),
            _top("FLST", *form_lists),
            _top(
                "CELL",
                {
                    "type": "group",
                    "label_hex": "00000000",
                    "group_type": 2,
                    "children": [
                        {
                            "type": "group",
                            "label_hex": "00000000",
                            "group_type": 3,
                            "children": [
                                _rec("CELL", TEST_CELL, _edid("TestCell")),
                                _children(TEST_CELL, 6, _children(TEST_CELL, 8, _rec("ACHR", BOB_REF, _sub("NAME", _fid(BOB))))),
                            ],
                        }
                    ],
                },
            ),
            _top("DIAL", *_topic(MASTER_TOPIC, _info(MASTER_INFO, texts=("Master line.",)), quest=None)),
        ],
    )


def _export(
    tmp_path: Path,
    *,
    quests: list[dict[str, object]] | None = None,
    topics: list[list[dict[str, object]]] | None = None,
    scenes: list[dict[str, object]] | None = None,
    include_default_voice_types: bool = True,
):
    _write_master(tmp_path, include_default_voice_types=include_default_voice_types)
    items = [_top("QUST", *(quests if quests is not None else [_quest()]))]
    if topics:
        items.append(_top("DIAL", *(item for topic in topics for item in topic)))
    if scenes:
        items.append(_top("SCEN", *scenes))
    _write(tmp_path / MOD, [MASTER], items)
    return export_dialogue_lines(tmp_path / MOD, data_dir=tmp_path)


def _voices(lines) -> set[tuple[str, str]]:
    return {(line.voice_type, line.voice_source) for line in lines}


def test_speaker_voice_names_each_response_line(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        topics=[_topic(TOPIC, _info(INFO, _sub("ANAM", _fid(BOB)), texts=("Hello there.", "Nice day.")))],
    )

    assert [(line.voice_type, line.voice_source, line.response_number, line.response_text) for line in lines] == [
        ("MaleBoston", "speaker", 1, "Hello there."),
        ("MaleBoston", "speaker", 2, "Nice day."),
    ]
    first = lines[0]
    assert first.plugin == MOD
    assert first.quest == "ModQuest"
    assert first.topic_text == "Topic Text"
    assert first.info_form_id == "01000802"
    assert first.file_name == "00000802_1"
    assert first.voice_path == r"Sound\Voice\Mod.esp\MaleBoston\00000802_1.fuz"


def test_player_topic_in_scene_is_spoken_by_player_voices(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        topics=[_topic(TOPIC, _info(INFO))],
        scenes=[_scene((PLAYER_DIALOGUE_ACTION, 0, [_sub("PTOP", _fid(TOPIC))]))],
    )

    assert _voices(lines) == {(voice, "player") for voice in PLAYER_VOICE_TYPES}


def test_npc_response_topic_uses_the_scene_alias_voice(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        quests=[_quest(QUEST, _alias(0, _sub("ALUA", _fid(BOB))))],
        topics=[_topic(TOPIC, _info(INFO))],
        scenes=[_scene((PLAYER_DIALOGUE_ACTION, 0, [_sub("NPOT", _fid(TOPIC))]))],
    )

    assert _voices(lines) == {("MaleBoston", "scene")}


def test_npc_response_dialogue_gives_player_slots_to_the_actor_alias(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        quests=[_quest(QUEST, _alias(0, _sub("ALUA", _fid(BOB))))],
        topics=[_topic(TOPIC, _info(INFO))],
        scenes=[_scene((NPC_RESPONSE_DIALOGUE_ACTION, 0, [_sub("PTOP", _fid(TOPIC))]))],
    )

    assert _voices(lines) == {("MaleBoston", "scene")}


def test_dialogue_action_uses_forced_reference_alias(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        quests=[_quest(QUEST, _alias(1, _sub("ALFR", _fid(BOB_REF))))],
        topics=[_topic(TOPIC, _info(INFO))],
        scenes=[_scene((DIALOGUE_ACTION, 1, [_sub("DATA", _fid(TOPIC))]))],
    )

    assert _voices(lines) == {("MaleBoston", "scene")}


def test_voice_type_condition_expands_form_lists(tmp_path: Path) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO, _ctda(GET_IS_VOICE_TYPE, MALE_AND_FEMALE_LIST)))])

    assert _voices(lines) == {("FemaleBoston", "condition"), ("MaleBoston", "condition")}


def test_or_group_is_intersected_with_the_next_condition(tmp_path: Path) -> None:
    info = _info(
        INFO,
        _ctda(GET_IS_VOICE_TYPE, MALE_BOSTON, or_next=True),
        _ctda(GET_IS_VOICE_TYPE, FEMALE_BOSTON),
        _ctda(GET_IS_ID, ALICE),
    )

    assert _voices(_export(tmp_path, topics=[_topic(TOPIC, info)])) == {("FemaleBoston", "condition")}


def test_negated_and_non_subject_conditions_do_not_name_the_speaker(tmp_path: Path) -> None:
    info = _info(
        INFO,
        _ctda(GET_IS_ID, BOB, value=0.0),
        _ctda(GET_IS_ID, GUARD, run_on=1),
        _ctda(GET_IS_VOICE_TYPE, FEMALE_BOSTON),
    )

    assert _voices(_export(tmp_path, topics=[_topic(TOPIC, info)])) == {("FemaleBoston", "condition")}


def test_alias_ref_condition_follows_external_alias(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        quests=[
            _quest(QUEST, _alias(2, _sub("ALEQ", _fid(OTHER_QUEST)), _sub("ALEA", struct.pack("<i", 0)))),
            _quest(OTHER_QUEST, _alias(0, _sub("ALUA", _fid(ALICE))), editor_id="OtherQuest"),
        ],
        topics=[_topic(TOPIC, _info(INFO, _ctda(GET_IS_ALIAS_REF, 2)))],
    )

    assert _voices(lines) == {("FemaleBoston", "condition")}


def test_alias_voice_type_list_limits_alias_conditions(tmp_path: Path) -> None:
    alias = _alias(3, _ctda(GET_IS_VOICE_TYPE, MALE_AND_FEMALE_LIST), _sub("VTCK", _fid(MALE_BOSTON)))
    lines = _export(
        tmp_path,
        quests=[_quest(QUEST, alias)],
        topics=[_topic(TOPIC, _info(INFO, _ctda(GET_IS_ALIAS_REF, 3)))],
    )

    assert _voices(lines) == {("MaleBoston", "condition")}


def test_quest_dialogue_conditions_voice_unconditioned_lines(tmp_path: Path) -> None:
    quest = _quest(
        dialogue_conditions=(_ctda(GET_IS_VOICE_TYPE, MALE_AND_FEMALE_LIST),),
        event_conditions=(_ctda(GET_IS_VOICE_TYPE, ROBOT_VOICE),),
    )
    lines = _export(tmp_path, quests=[quest], topics=[_topic(TOPIC, _info(INFO))])

    assert _voices(lines) == {("FemaleBoston", "condition"), ("MaleBoston", "condition")}


def test_quest_dialogue_conditions_narrow_info_conditions(tmp_path: Path) -> None:
    quest = _quest(dialogue_conditions=(_ctda(GET_IS_VOICE_TYPE, FEMALE_BOSTON),))
    lines = _export(
        tmp_path,
        quests=[quest],
        topics=[_topic(TOPIC, _info(INFO, _ctda(GET_IS_VOICE_TYPE, MALE_AND_FEMALE_LIST)))],
    )

    assert _voices(lines) == {("FemaleBoston", "condition")}


def test_templated_npc_inherits_voice_through_leveled_list(tmp_path: Path) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO, _sub("ANAM", _fid(GUARD_TEMPLATED))))])

    assert _voices(lines) == {("MaleRough", "speaker")}


def test_talking_activator_condition_uses_activator_voice(tmp_path: Path) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO, _ctda(GET_IS_ID, ROBOT_ACTIVATOR)))])

    assert _voices(lines) == {("RobotVoice", "condition")}


@pytest.mark.parametrize(("function", "form_id"), [(GET_IN_FACTION, ALICE_FACTION), (GET_IS_CLASS, ALICE_CLASS)])
def test_faction_and_class_conditions_collect_member_voices(tmp_path: Path, function: int, form_id: int) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO, _ctda(function, form_id)))])

    assert _voices(lines) == {("FemaleBoston", "condition")}


def test_previous_info_voice_is_used_when_unresolved(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        topics=[
            _topic(
                TOPIC,
                _info(INFO, _sub("ANAM", _fid(BOB))),
                _info(SECOND_INFO, _sub("PNAM", _fid(INFO))),
            )
        ],
    )

    assert _voices([line for line in lines if line.info_form_id == "01000803"]) == {("MaleBoston", "previous")}


def test_quest_greeting_voice_is_the_fallback(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        topics=[
            _topic(GREETING_TOPIC, _info(GREETING_INFO, _sub("ANAM", _fid(ALICE))), subtype=b"GREE"),
            _topic(TOPIC, _info(INFO)),
        ],
    )

    assert _voices([line for line in lines if line.info_form_id == "01000802"]) == {("FemaleBoston", "greeting")}


def test_default_npc_voice_types_are_the_last_resort(tmp_path: Path) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO))])

    assert _voices(lines) == {("MaleRough", "default")}


def test_unresolved_line_is_exported_without_voice_type(tmp_path: Path) -> None:
    lines = _export(tmp_path, topics=[_topic(TOPIC, _info(INFO))], include_default_voice_types=False)

    assert [(line.voice_type, line.voice_source, line.response_text) for line in lines] == [("", "", "Hello there.")]


def test_only_dialogue_added_by_the_plugin_is_exported(tmp_path: Path) -> None:
    lines = _export(
        tmp_path,
        topics=[
            _topic(MASTER_TOPIC, _info(MASTER_INFO, texts=("Edited master line.",)), quest=None),
            _topic(TOPIC, _info(INFO, _sub("ANAM", _fid(BOB)))),
        ],
    )

    assert {line.info_form_id for line in lines} == {"01000802"}
