"""Export a Fallout 4 plugin's dialogue with the voice types that speak each line.

Voice types are resolved the way the Creation Kit assigns voice files: player
scene topics, then the INFO speaker, the INFO and quest dialogue conditions,
the scene alias that says the topic, the previous INFO, the quest's greetings,
and finally the game's DefaultNPCVoiceTypes list.
"""

from __future__ import annotations

import json
import logging
import struct
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

from creation_lib.esp import native_runtime
from creation_lib.esp.plugin import Plugin

_log = logging.getLogger("creation_lib.esp.dialogue_export")

PLAYER_VOICE_TYPES = ("PlayerVoiceFemale01", "PlayerVoiceMale01")

_GAME = "fo4"
_BASE_MASTER = "fallout4.esm"
_PLAYER_OBJECT_IDS = {0x000007, 0x000014}
_DEFAULT_NPC_VOICE_TYPES = (_BASE_MASTER, 0x02D563)

_GET_IS_CLASS = 68
_GET_IN_FACTION = 71
_GET_IS_ID = 72
_GET_IS_VOICE_TYPE = 426
_GET_IS_ALIAS_REF = 566
_FORM_CONDITIONS = {_GET_IS_CLASS, _GET_IN_FACTION, _GET_IS_ID, _GET_IS_VOICE_TYPE}

_CTDA_OR = 0x01
_CTDA_USE_GLOBAL = 0x04
_CTDA_SWAP_SUBJECT_TARGET = 0x10
_TEMPLATE_USE_TRAITS = 0x0001

_PLAYER_TOPICS = {"PTOP", "NTOP", "NETO", "QTOP"}
_ACTOR_TOPICS = {"DATA", "NPOT", "NNGT", "NNUT", "NQUT"}
_QUEST_DIALOGUE_CONDITIONS_END = {"NEXT", "INDX", "QOBJ", "ANAM", "ALST", "ALLS"}
_NPC_RESPONSE_DIALOGUE = 5

FormKey = tuple[str, int]


@dataclass(slots=True)
class DialogueLine:
    plugin: str
    quest: str
    topic_text: str
    info_form_id: str
    response_number: int
    response_text: str
    voice_type: str
    voice_source: str

    @property
    def file_name(self) -> str:
        return f"{int(self.info_form_id, 16) & 0xFFFFFF:08X}_{self.response_number}"

    @property
    def voice_path(self) -> str:
        return f"Sound\\Voice\\{self.plugin}\\{self.voice_type}\\{self.file_name}.fuz"


def export_dialogue_lines(plugin_path: str | Path, *, data_dir: str | Path) -> list[DialogueLine]:
    """Return one line per INFO response and voice type for dialogue the plugin adds.

    Masters are loaded from ``data_dir``. INFOs that override a master's INFO
    are skipped because their voice files belong to that master.
    """
    order = _LoadOrder(Path(plugin_path), Path(data_dir))
    try:
        return _export(order)
    finally:
        order.close()


def _u32(data: bytes, offset: int = 0) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def _i32(data: bytes, offset: int = 0) -> int:
    return struct.unpack_from("<i", data, offset)[0]


@dataclass(slots=True)
class _LoadedPlugin:
    plugin: Plugin
    name: str
    masters: list[str]

    @property
    def handle(self) -> int:
        return self.plugin._rust_handle

    def key(self, raw: int) -> FormKey | None:
        if raw in (0, 0xFFFFFFFF):
            return None
        index = raw >> 24
        origin = self.masters[index] if index < len(self.masters) else self.name
        return origin.lower(), raw & 0xFFFFFF

    def raw(self, key: FormKey) -> int | None:
        origin, object_id = key
        lowered = [master.lower() for master in self.masters]
        if origin == self.name.lower():
            index = len(lowered)
        elif origin in lowered:
            index = lowered.index(origin)
        else:
            return None
        return (index << 24) | object_id


@dataclass(slots=True)
class _Record:
    key: FormKey
    signature: str
    subrecords: list[tuple[str, bytes]]
    plugin: _LoadedPlugin

    def first(self, signature: str) -> bytes | None:
        return next((data for sig, data in self.subrecords if sig == signature), None)

    def all(self, signature: str) -> list[bytes]:
        return [data for sig, data in self.subrecords if sig == signature]

    def form(self, signature: str) -> FormKey | None:
        data = self.first(signature)
        return self.plugin.key(_u32(data)) if data and len(data) >= 4 else None

    @property
    def editor_id(self) -> str:
        data = self.first("EDID") or b""
        return data.split(b"\x00", 1)[0].decode("cp1252", errors="replace")


class _LoadOrder:
    def __init__(self, plugin_path: Path, data_dir: Path) -> None:
        self.target = self._open(plugin_path, lazy=False)
        self.plugins: list[_LoadedPlugin] = []
        for master in self.target.masters:
            path = data_dir / master
            if not path.is_file():
                _log.warning("Master not found, its records will not resolve: %s", path)
                continue
            self.plugins.append(self._open(path, lazy=True))
        self.plugins.append(self.target)
        self._records: dict[FormKey, _Record | None] = {}
        self._form_ids: dict[str, set[int]] = {}

    @staticmethod
    def _open(path: Path, *, lazy: bool) -> _LoadedPlugin:
        plugin = Plugin.load(path, game=_GAME, lazy_index=lazy)
        return _LoadedPlugin(plugin, plugin.plugin_name, list(plugin.masters))

    def record(self, key: FormKey | None) -> _Record | None:
        if key is None:
            return None
        if key not in self._records:
            self._records[key] = self._find(key)
        return self._records[key]

    def _find(self, key: FormKey) -> _Record | None:
        for plugin in reversed(self.plugins):
            raw = plugin.raw(key)
            if raw is None or not self._contains(plugin, raw):
                continue
            context = native_runtime.plugin_handle_call(plugin.handle, "record_context_for_form_id", raw)
            if not context:
                continue
            subrecords = native_runtime.plugin_handle_record_subrecords(plugin.handle, raw) or []
            return _Record(key, context["record_signature"], [(sig, data) for sig, data, _ in subrecords], plugin)
        return None

    def _contains(self, plugin: _LoadedPlugin, raw: int) -> bool:
        # Native per-handle lookups match on object id alone, so 00000801 in a
        # plugin with masters would return that plugin's own 01000801.
        if not plugin.masters:
            return True
        if plugin.name not in self._form_ids:
            self._form_ids[plugin.name] = set(native_runtime.plugin_handle_record_form_ids(plugin.handle, None))
        return raw in self._form_ids[plugin.name]

    def records_of(self, plugin: _LoadedPlugin, signature: str) -> list[tuple[int, list[tuple[str, bytes]]]]:
        out = []
        for raw in native_runtime.plugin_handle_record_form_ids(plugin.handle, [signature]):
            subrecords = native_runtime.plugin_handle_record_subrecords(plugin.handle, raw) or []
            out.append((raw, [(sig, data) for sig, data, _ in subrecords]))
        return out

    def close(self) -> None:
        for plugin in self.plugins:
            plugin.plugin.close()


@dataclass(slots=True)
class _Condition:
    function: int
    param1: int
    target: FormKey | None
    applies: bool
    or_next: bool


def _conditions(record: _Record, subrecords: list[bytes] | None = None) -> list[_Condition]:
    out = []
    for data in record.all("CTDA") if subrecords is None else subrecords:
        if len(data) < 24:
            continue
        flags = data[0]
        operator = flags >> 5
        value = struct.unpack_from("<f", data, 4)[0]
        positive = not flags & _CTDA_USE_GLOBAL and ((operator == 0 and value != 0) or (operator == 1 and value == 0))
        on_speaker = _u32(data, 20) == 0 and not flags & _CTDA_SWAP_SUBJECT_TARGET
        param1 = _u32(data, 12)
        out.append(
            _Condition(
                function=struct.unpack_from("<H", data, 8)[0],
                param1=param1,
                target=record.plugin.key(param1),
                applies=positive and on_speaker,
                or_next=bool(flags & _CTDA_OR),
            )
        )
    return out


def _dialogue_conditions(quest: _Record) -> list[bytes]:
    # Quest CTDAs before NEXT are dialogue conditions applied to every line in the quest.
    conditions = []
    for sig, data in quest.subrecords:
        if sig in _QUEST_DIALOGUE_CONDITIONS_END:
            break
        if sig == "CTDA":
            conditions.append(data)
    return conditions


def _union_or_none(voices: list[set[str] | None]) -> set[str] | None:
    return None if any(item is None for item in voices) else set().union(*voices)


def _intersect(groups: list[set[str] | None]) -> set[str]:
    constrained = [group for group in groups if group is not None]
    return set.intersection(*constrained) if constrained else set()


@dataclass(slots=True)
class _Alias:
    forced: FormKey | None = None
    unique: FormKey | None = None
    external_quest: FormKey | None = None
    external_alias: int | None = None
    conditions: list[bytes] = field(default_factory=list)
    voice_types: FormKey | None = None


def _aliases(quest: _Record) -> dict[int, _Alias]:
    aliases: dict[int, _Alias] = {}
    current: _Alias | None = None
    for sig, data in quest.subrecords:
        if sig in ("ALST", "ALLS") and len(data) >= 4:
            current = aliases.setdefault(_u32(data), _Alias())
        elif current is None:
            continue
        elif sig == "ALED":
            current = None
        elif sig == "CTDA":
            current.conditions.append(data)
        elif sig == "ALEA" and len(data) >= 4:
            current.external_alias = _i32(data)
        elif sig in ("ALFR", "ALUA", "ALEQ", "VTCK") and len(data) >= 4:
            key = quest.plugin.key(_u32(data))
            if sig == "ALFR":
                current.forced = key
            elif sig == "ALUA":
                current.unique = key
            elif sig == "ALEQ":
                current.external_quest = key
            else:
                current.voice_types = key
    return aliases


@dataclass(slots=True)
class _Topic:
    key: FormKey
    quest: FormKey | None
    greeting: bool
    text: str


@dataclass(slots=True)
class _Scenes:
    player_topics: set[FormKey] = field(default_factory=set)
    spoken_by: dict[FormKey, list[tuple[FormKey, int]]] = field(default_factory=lambda: defaultdict(list))


@dataclass(slots=True)
class _SceneAction:
    kind: int
    alias: int | None = None
    player_topics: list[FormKey | None] = field(default_factory=list)
    actor_topics: list[FormKey | None] = field(default_factory=list)


def _scenes(order: _LoadOrder) -> _Scenes:
    scenes = _Scenes()
    target = order.target
    for _raw, subrecords in order.records_of(target, "SCEN"):
        quest: FormKey | None = None
        actions: list[_SceneAction] = []
        action: _SceneAction | None = None
        for sig, data in subrecords:
            if sig == "ANAM":
                if len(data) >= 2:
                    action = _SceneAction(struct.unpack_from("<H", data)[0])
                elif action is not None:
                    actions.append(action)
                    action = None
            elif action is not None:
                if sig == "ALID" and len(data) >= 4:
                    action.alias = _i32(data)
                elif sig in _PLAYER_TOPICS and len(data) >= 4:
                    action.player_topics.append(target.key(_u32(data)))
                elif sig in _ACTOR_TOPICS and len(data) >= 4:
                    action.actor_topics.append(target.key(_u32(data)))
            elif sig == "PNAM" and len(data) >= 4:
                quest = target.key(_u32(data))
        for action in actions:
            actor_topics = list(action.actor_topics)
            # NPC Response Dialogue lets an NPC take the player's lines, so they use the actor's voice.
            if action.kind == _NPC_RESPONSE_DIALOGUE:
                actor_topics += action.player_topics
            else:
                scenes.player_topics.update(topic for topic in action.player_topics if topic)
            if quest is None or action.alias is None:
                continue
            for topic in actor_topics:
                if topic:
                    scenes.spoken_by[topic].append((quest, action.alias))
    return scenes


class _VoiceResolver:
    def __init__(self, order: _LoadOrder, scenes: _Scenes) -> None:
        self._order = order
        self._scenes = scenes
        self._aliases: dict[FormKey, dict[int, _Alias]] = {}
        self._members: dict[FormKey, set[FormKey]] | None = None
        self.topic_of_info: dict[FormKey, _Topic] = {}
        self.greetings: dict[FormKey, list[tuple[_Record, _Topic]]] = defaultdict(list)

    def of_info(self, info: _Record, topic: _Topic | None) -> tuple[set[str], str]:
        return self._of_info(info, topic, True, set())

    def _of_info(self, info: _Record, topic: _Topic | None, fallbacks: bool, seen: set[FormKey]) -> tuple[set[str], str]:
        seen.add(info.key)
        if topic is not None and topic.key in self._scenes.player_topics:
            return set(PLAYER_VOICE_TYPES), "player"
        voices = self.of_record(info.form("ANAM"))
        if voices:
            return voices, "speaker"
        quest = self._order.record(topic.quest) if topic else None
        groups = self._groups(_conditions(info), topic.quest if topic else None)
        if quest is not None and quest.signature == "QUST":
            groups += self._groups(_conditions(quest, _dialogue_conditions(quest)), quest.key)
        voices = _intersect(groups)
        if voices:
            return voices, "condition"
        if topic is not None:
            voices = set().union(*(self.of_alias(quest, alias) for quest, alias in self._scenes.spoken_by.get(topic.key, [])))
            if voices:
                return voices, "scene"
        previous = self._order.record(info.form("PNAM"))
        if previous is not None and previous.signature == "INFO" and previous.key not in seen:
            voices, _ = self._of_info(previous, self.topic_of_info.get(previous.key), False, seen)
            if voices:
                return voices, "previous"
        if not fallbacks:
            return set(), ""
        if topic is not None and topic.quest is not None:
            for greeting, greeting_topic in self.greetings.get(topic.quest, []):
                if greeting.key == info.key:
                    continue
                voices, _ = self._of_info(greeting, greeting_topic, False, set())
                if voices:
                    return voices, "greeting"
        voices = self.of_record(_DEFAULT_NPC_VOICE_TYPES)
        if voices:
            return voices, "default"
        return set(), ""

    def _groups(self, conditions: list[_Condition], quest: FormKey | None) -> list[set[str] | None]:
        # Consecutive OR-flagged conditions form a group; groups are ANDed.
        # None marks a group that places no constraint on the speaker.
        groups: list[set[str] | None] = []
        current: list[set[str] | None] = []
        for condition in conditions:
            current.append(self._of_condition(condition, quest))
            if not condition.or_next:
                groups.append(_union_or_none(current))
                current = []
        if current:
            groups.append(_union_or_none(current))
        return groups

    def _of_condition(self, condition: _Condition, quest: FormKey | None) -> set[str] | None:
        if not condition.applies:
            return None
        if condition.function == _GET_IS_ALIAS_REF:
            return (self.of_alias(quest, condition.param1) if quest else set()) or None
        if condition.function in _FORM_CONDITIONS:
            return self.of_record(condition.target) or None
        return None

    def of_alias(self, quest_key: FormKey, alias_id: int, seen: set[tuple[FormKey, int]] | None = None) -> set[str]:
        seen = set() if seen is None else seen
        if (quest_key, alias_id) in seen:
            return set()
        seen.add((quest_key, alias_id))
        quest = self._order.record(quest_key)
        if quest is None or quest.signature != "QUST":
            return set()
        if quest_key not in self._aliases:
            self._aliases[quest_key] = _aliases(quest)
        alias = self._aliases[quest_key].get(alias_id)
        if alias is None:
            return set()
        if alias.forced:
            voices = self.of_record(alias.forced)
        elif alias.unique:
            voices = self.of_record(alias.unique)
        elif alias.external_quest and alias.external_alias is not None:
            voices = self.of_alias(alias.external_quest, alias.external_alias, seen)
        else:
            voices = _intersect(self._groups(_conditions(quest, alias.conditions), quest_key))
        limit = self.of_record(alias.voice_types)
        if limit:
            voices = (voices & limit) or limit
        return voices

    def of_record(self, key: FormKey | None, seen: set[FormKey] | None = None) -> set[str]:
        if key is None:
            return set()
        if key[0] == _BASE_MASTER and key[1] in _PLAYER_OBJECT_IDS:
            return set(PLAYER_VOICE_TYPES)
        seen = set() if seen is None else seen
        if key in seen:
            return set()
        seen.add(key)
        record = self._order.record(key)
        if record is None:
            return set()
        signature = record.signature
        if signature == "VTYP":
            return {record.editor_id} if record.editor_id else set()
        if signature in ("ACHR", "REFR"):
            return self.of_record(record.form("NAME"), seen)
        if signature == "NPC_":
            return self.of_record(self._npc_voice(record), seen)
        if signature == "TACT":
            return self.of_record(record.form("VNAM"), seen)
        if signature == "LVLN":
            entries = [record.plugin.key(_u32(data, 4)) for data in record.all("LVLO") if len(data) >= 8]
            return set().union(*(self.of_record(entry, seen) for entry in entries))
        if signature == "FLST":
            entries = [record.plugin.key(_u32(data)) for data in record.all("LNAM") if len(data) >= 4]
            return set().union(*(self.of_record(entry, seen) for entry in entries))
        if signature in ("FACT", "CLAS"):
            return set().union(*(self.of_record(npc, seen) for npc in self._members_of(key)))
        return set()

    @staticmethod
    def _npc_voice(npc: _Record) -> FormKey | None:
        acbs = npc.first("ACBS")
        template_flags = struct.unpack_from("<H", acbs, 14)[0] if acbs and len(acbs) >= 16 else 0
        if template_flags & _TEMPLATE_USE_TRAITS:
            traits = npc.first("TPTA")
            traits_template = npc.plugin.key(_u32(traits)) if traits and len(traits) >= 4 else None
            return traits_template or npc.form("TPLT") or npc.form("VTCK")
        return npc.form("VTCK")

    def _members_of(self, group: FormKey) -> set[FormKey]:
        if self._members is None:
            memberships: dict[FormKey, list[FormKey]] = {}
            for plugin in self._order.plugins:
                for raw, subrecords in self._order.records_of(plugin, "NPC_"):
                    npc = plugin.key(raw)
                    if npc is None:
                        continue
                    memberships[npc] = [
                        key
                        for sig, data in subrecords
                        if sig in ("SNAM", "CNAM") and len(data) >= 4
                        for key in [plugin.key(_u32(data))]
                        if key is not None
                    ]
            self._members = defaultdict(set)
            for npc, groups in memberships.items():
                for member_of in groups:
                    self._members[member_of].add(npc)
        return self._members.get(group, set())


def _response_numbers(info: _Record) -> list[int]:
    numbers = []
    pending: int | None = None
    for sig, data in info.subrecords:
        if sig == "TRDA" and len(data) >= 5:
            pending = data[4]
        elif sig == "NAM1":
            numbers.append(pending or len(numbers) + 1)
            pending = None
    return numbers


def _export(order: _LoadOrder) -> list[DialogueLine]:
    target = order.target
    resolver = _VoiceResolver(order, _scenes(order))
    infos: list[tuple[_Record, int, _Topic, list[str]]] = []
    for topic_payload in json.loads(native_runtime.plugin_handle_extract_dialogue_text(target.handle, "json")):
        dial_key = target.key(int(topic_payload["dial_form_id"], 16))
        dial = order.record(dial_key)
        if dial_key is None or dial is None:
            continue
        topic = _Topic(
            key=dial_key,
            quest=dial.form("QNAM"),
            greeting=(dial.first("SNAM") or b"")[:4] == b"GREE",
            text=((topic_payload.get("topic") or {}).get("text") or ""),
        )
        for info_payload in topic_payload.get("infos", []):
            raw = int(info_payload["form_id"], 16)
            info = order.record(target.key(raw))
            if info is None:
                continue
            resolver.topic_of_info[info.key] = topic
            if topic.greeting and topic.quest is not None:
                resolver.greetings[topic.quest].append((info, topic))
            if info.key[0] != target.name.lower():
                continue
            texts = [(response or {}).get("text") or "" for response in info_payload.get("responses", [])]
            infos.append((info, raw, topic, texts))

    quest_names: dict[FormKey | None, str] = {}
    lines: list[DialogueLine] = []
    for info, raw, topic, texts in infos:
        voices, source = resolver.of_info(info, topic)
        if topic.quest not in quest_names:
            quest = order.record(topic.quest)
            quest_names[topic.quest] = quest.editor_id if quest is not None else ""
        for ordinal, number in enumerate(_response_numbers(info)):
            for voice in sorted(voices) or [""]:
                lines.append(
                    DialogueLine(
                        plugin=target.name,
                        quest=quest_names[topic.quest],
                        topic_text=topic.text,
                        info_form_id=f"{raw:08X}",
                        response_number=number,
                        response_text=texts[ordinal] if ordinal < len(texts) else "",
                        voice_type=voice,
                        voice_source=source,
                    )
                )
    return lines
