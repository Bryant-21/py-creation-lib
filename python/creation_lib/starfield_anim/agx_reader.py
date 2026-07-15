"""Parse Starfield .agx behavior graph files (XML format).

.agx files are plain XML containing a node graph with typed nodes,
connections, variables, and events. Parsed into a BehaviorData-compatible
interface for indexing alongside Havok behaviors.

Reference: refs/CALUMI.Motion/CALUMI.Motion/AgxNodes/AgxNode.h (node types)
"""
from __future__ import annotations

import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class AgxNode:
    """A single node in an .agx behavior graph."""
    node_type: str = ""
    name: str = ""
    guid: str = ""


@dataclass
class AgxData:
    """Parsed .agx behavior graph — BehaviorData-compatible interface."""
    name: str = ""
    category: str = ""
    node_count: int = 0
    node_classes: list[str] = field(default_factory=list)  # Unique node types
    nodes: list[AgxNode] = field(default_factory=list)
    variables: list[tuple[str, str]] = field(default_factory=list)  # (name, type)
    events: list[str] = field(default_factory=list)
    sequences: list[str] = field(default_factory=list)
    transitions: list[tuple[str, str]] = field(default_factory=list)  # (name, duration)


def parse_agx(agx_path: Path) -> AgxData:
    """Parse an .agx XML file and return behavior graph data."""
    result = AgxData()
    try:
        tree = ET.parse(str(agx_path))
    except (ET.ParseError, OSError):
        return result

    root = tree.getroot()

    # Top-level metadata
    name_el = root.find("Name")
    if name_el is not None and name_el.text:
        result.name = name_el.text.strip()

    cat_el = root.find("Category")
    if cat_el is not None and cat_el.text:
        result.category = cat_el.text.strip()

    # Parse nodes
    seen_types: set[str] = set()
    for node_el in root.findall("node"):
        node = AgxNode()

        type_el = node_el.find("node_type")
        if type_el is not None and type_el.text:
            node.node_type = type_el.text.strip()

        name_el = node_el.find("name")
        if name_el is not None and name_el.text:
            node.name = name_el.text.strip()

        guid_el = node_el.find("guid")
        if guid_el is not None and guid_el.text:
            node.guid = guid_el.text.strip()

        result.nodes.append(node)

        if node.node_type and node.node_type not in seen_types:
            seen_types.add(node.node_type)
            result.node_classes.append(node.node_type)

        # Extract animation clip names as sequences
        if node.node_type == "NT_ANIMATION_NODE":
            anim_el = node_el.find("animation_name")
            if anim_el is not None and anim_el.text:
                result.sequences.append(anim_el.text.strip())

        # Extract events from event controller nodes
        if node.node_type == "NT_EVENT_CONTROLLER":
            event_el = node_el.find("event_name")
            if event_el is not None and event_el.text:
                result.events.append(event_el.text.strip())

    result.node_count = len(result.nodes)

    # Extract variables from variable-related nodes
    _VAR_TYPES = {"NT_ASSIGN_VARIABLE", "NT_EVALUATE_CONDITION_VARIABLE",
                  "NT_DAMPEN_VARIABLE", "NT_LINEAR_VARIABLE",
                  "NT_MASS_SPRING_DAMPEN_VARIABLE", "NT_VARIABLE_COMBINER",
                  "NT_STATE_VARIABLE_CONTROL"}
    seen_vars: set[str] = set()
    for node_el in root.findall("node"):
        type_el = node_el.find("node_type")
        ntype = type_el.text.strip() if type_el is not None and type_el.text else ""
        if ntype in _VAR_TYPES:
            var_el = node_el.find("variable_name")
            if var_el is not None and var_el.text:
                vname = var_el.text.strip()
                if vname and vname not in seen_vars:
                    seen_vars.add(vname)
                    result.variables.append((vname, "unknown"))

    return result
