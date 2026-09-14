from __future__ import annotations

import re
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path


def read_behavior_xml(path):
    path = Path(path)
    if path.suffix.casefold() == ".xml":
        xml = path.read_text(encoding="utf-8-sig")
    else:
        from creation_lib.havok.native_runtime import load_native_module
        native = load_native_module()
        if native is None:
            raise RuntimeError("Havok native backend unavailable")
        xml = native.unpack_hkx_to_xml(str(path))
    return xml


def behavior_report(path):
    path = Path(path)
    xml = read_behavior_xml(path)
    try:
        root = ET.fromstring(xml)
    except ET.ParseError as error:
        raise ValueError(f"Invalid behavior XML: {error}") from error
    objects = {obj.get("name"): obj for obj in root.iter("hkobject") if obj.get("name")}
    if not objects:
        raise ValueError("No named HKX objects found")

    def param(obj, name):
        return obj.find(f"./hkparam[@name='{name}']") if obj is not None else None

    def value(obj, name, default=None):
        element = param(obj, name)
        return element.text.strip() if element is not None and element.text else default

    def number(obj, name):
        raw = value(obj, name)
        return int(raw) if raw is not None else None

    def refs(obj, name):
        return re.findall(r"#[\w]+", value(obj, name, ""))

    strings = next((obj for obj in objects.values() if obj.get("class") == "hkbBehaviorGraphStringData"), None)

    def names(name):
        element = param(strings, name)
        return [(item.text or "") for item in element.findall("hkcstring")] if element is not None else []

    events, variables, properties = names("eventNames"), names("variableNames"), names("characterPropertyNames")
    transitions, machines, clips, links, bindings = [], [], [], [], []
    for identifier, obj in objects.items():
        for element in obj.iter("hkparam"):
            if element.get("name") not in {"name", "animationName", "eventNames", "variableNames"}:
                links.extend({"from": identifier, "to": target, "field": element.get("name")}
                             for target in re.findall(r"#[\w]+", element.text or ""))
        if obj.get("class") == "hkbClipGenerator":
            clips.append({"id": identifier, "name": value(obj, "name"), "animation": value(obj, "animationName"),
                          "mode": value(obj, "mode"), "triggers": value(obj, "triggers")})
        if obj.get("class") == "hkbVariableBindingSet":
            array = param(obj, "bindings")
            owners = [key for key, owner in objects.items() if identifier in refs(owner, "variableBindingSet")]
            for index, binding in enumerate(array.findall("hkobject") if array is not None else []):
                binding_type = value(binding, "bindingType", "0")
                variable_index = number(binding, "variableIndex")
                source = properties if binding_type in {"1", "BINDING_TYPE_CHARACTER_PROPERTY"} else variables
                bindings.append({"set": identifier, "index": index, "owners": owners,
                                 "member_path": value(binding, "memberPath"), "variable_index": variable_index,
                                 "variable": source[variable_index] if variable_index is not None and 0 <= variable_index < len(source) else None,
                                 "binding_type": binding_type, "binding_type_explicit": param(binding, "bindingType") is not None,
                                 "bit_index": number(binding, "bitIndex"), "enable_binding_index": number(obj, "indexOfBindingToEnable")})
        if obj.get("class") != "hkbStateMachine":
            continue
        states = []
        for state_ref in refs(obj, "states"):
            state = objects.get(state_ref)
            if state is not None:
                states.append({"id": state_ref, "state_id": number(state, "stateId"), "name": value(state, "name"),
                               "generator": value(state, "generator"), "transitions": value(state, "transitions")})
        state_names = {state["state_id"]: state["name"] for state in states}
        sources = [(state["state_id"], state["transitions"]) for state in states]
        sources.append((None, value(obj, "wildcardTransitions")))
        for source, transition_ref in sources:
            array = param(objects.get(transition_ref), "transitions")
            if array is None:
                continue
            for transition in array.findall("hkobject"):
                event, target = number(transition, "eventId"), number(transition, "toStateId")
                transitions.append({"machine": identifier, "from_state": source, "to_state": target,
                                    "to_name": state_names.get(target), "event_id": event,
                                    "event": events[event] if event is not None and 0 <= event < len(events) else None,
                                    "condition": value(transition, "condition"), "effect": value(transition, "transition"),
                                    "flags": value(transition, "flags")})
        machines.append({"id": identifier, "name": value(obj, "name"), "start_state": number(obj, "startStateId"), "states": states})
    unresolved = [link for link in links if link["to"] not in objects]
    return {"path": str(path.resolve()), "node_count": len(objects),
            "classes": dict(sorted(Counter(obj.get("class") for obj in objects.values()).items())),
            "events": [{"id": i, "name": name} for i, name in enumerate(events)],
            "variables": [{"id": i, "name": name} for i, name in enumerate(variables)],
            "character_properties": [{"id": i, "name": name} for i, name in enumerate(properties)],
            "bindings": bindings,
            "state_machines": machines, "transitions": transitions, "clips": clips, "links": links,
            "unresolved_references": unresolved}
