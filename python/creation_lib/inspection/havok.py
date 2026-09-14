from __future__ import annotations

import re
import xml.etree.ElementTree as ET

from creation_lib.inspection.records import field_differences


def graph_snapshot(xml):
    try:
        root = ET.fromstring(xml)
    except ET.ParseError as error:
        raise ValueError(f"Invalid behavior XML: {error}") from error
    objects = {obj.get("name"): obj for obj in root.iter("hkobject") if obj.get("name")}
    if not objects:
        raise ValueError("No named HKX objects found")
    if len(objects) != sum(1 for obj in root.iter("hkobject") if obj.get("name")):
        raise ValueError("Duplicate HKX object names")
    order, identifiers = [], {}

    def reference(name):
        if name not in objects:
            raise ValueError(f"Unresolved HKX object reference: {name}")
        if name not in identifiers:
            identifiers[name] = f"#{len(identifiers)}"
            order.append(name)
        return identifiers[name]

    def element_value(element):
        if element.tag == "hkcstring":
            return element.text or ""
        children = list(element)
        if children:
            if all(child.tag == "hkparam" for child in children):
                if len({child.get("name") for child in children}) != len(children):
                    raise ValueError("Duplicate HKX parameter names")
                return {child.get("name"): element_value(child) for child in sorted(children, key=lambda c: c.get("name", ""))}
            return [element_value(child) for child in children]
        text = (element.text or "").strip()
        tokens = text.split()
        if tokens and all(re.fullmatch(r"#[\w]+|null", token) for token in tokens):
            return " ".join(reference(token) if token != "null" else token for token in tokens)
        return text

    reference(root.get("toplevelobject") or next(iter(objects)))
    result = {}
    position = 0
    while position < len(objects):
        if position == len(order):
            remaining = [name for name in objects if name not in identifiers]
            remaining.sort(key=lambda name: (objects[name].get("class", ""), ET.tostring(objects[name], encoding="unicode")))
            reference(remaining[0])
        name = order[position]
        obj = objects[name]
        result[identifiers[name]] = {"class": obj.get("class"), "fields": element_value(obj)}
        position += 1
    return result


def graph_differences(before_xml, after_xml):
    return field_differences(graph_snapshot(before_xml), graph_snapshot(after_xml))
